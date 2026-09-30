// Local tutor engines with native Canary STT and HTTP Whisper fallback.
use std::{
    collections::{hash_map::DefaultHasher, BTreeSet, HashMap},
    env, fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        AudioPayload, ChoreContext, Confidence, Direction, EngineComponent, EngineStatus, Focus,
        LedgerSpec, Message, PhonemeSpan, Pitch, PlacementReading, Readiness, Scorable, Speaker,
        Topic, TurnSignal, TutorRequest, WordSpan,
    },
    error::{EllaError, EllaResult},
    infrastructure::{
        audio::raw_pcm_to_wav,
        engine_manager::LlamaServer,
        speech_timing::{phoneme_spans, shifted, word_spans},
        stt::{
            CanaryStt, SpeechToTextEngine, SttRouter, Transcription, WindowsStt,
            CANARY_FILE_NAME,
        },
    },
};

#[derive(Debug)]
pub struct GeneratedReply {
    pub text: String,
    pub ttft_ms: f64,
    pub completion_ms: f64,
    /// The figure the character named, if the chore keeps a ledger. Extracted
    /// from the reply text rather than asked for, so the model is never the
    /// authority on the number.
    pub named_figure: Option<i32>,
    /// `[DEAL]` / `[WALK]`, stripped from `text` before it reaches a screen or
    /// Piper. A bare token beats reading agreement out of free prose.
    pub signal: Option<TurnSignal>,
    /// True when the first generation broke the ledger limit or step and the
    /// turn had to be generated again.
    pub regenerated: bool,
    /// The sentences of `text`, synthesized while it was being written and
    /// held until the caller has saved the turn and releases them. `None`
    /// means the caller has to say `text` itself: nothing was synthesized
    /// ahead, or what was is not what the reply ended up saying.
    pub pending: Option<PendingSpeech>,
}

impl GeneratedReply {
    pub fn plain(text: String, ttft_ms: f64, completion_ms: f64) -> Self {
        Self {
            text,
            ttft_ms,
            completion_ms,
            named_figure: None,
            signal: None,
            regenerated: false,
            pending: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SynthesizedAudio {
    pub audio: Option<AudioPayload>,
    pub first_audio_ms: Option<f64>,
    pub completion_ms: Option<f64>,
    /// When each word of the whole reply is spoken, from the start of `audio`.
    pub words: Vec<WordSpan>,
    /// Every token of the whole reply, from the start of `audio`, when the
    /// voice timed them all. Empty otherwise.
    pub phonemes: Vec<PhonemeSpan>,
    /// How many sentences this was cut into and pushed to the sink. Zero means
    /// nothing was streamed, so the whole recording still has to be played.
    pub segments: u32,
}

const PIPER_DAEMON_SOURCE: &str = include_str!("piper_daemon.py");

/// How long the standalone binary may take to finish a WAV it has already
/// named. It prints the file's name before it closes the file, so on Windows
/// the last few kilobytes can still be in the C runtime's buffer when the line
/// arrives.
const RESIDENT_WAV_SETTLE: Duration = Duration::from_secs(2);

/// How long the standalone binary may take to answer a line, voice load
/// included. Far longer than a sentence takes; what it bounds is a Piper that
/// has stopped answering, which would otherwise hold the turn for good.
const RESIDENT_REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// What the resident daemon hands back for one request.
struct DaemonSpeech {
    /// Raw 16-bit mono PCM.
    pcm: Vec<u8>,
    sample_rate: u32,
    /// Every token Piper spoke, with how many samples of `pcm` it lasted.
    /// Empty when the voice could not say, or said something that does not
    /// add up to the audio.
    alignment: Vec<(String, u32)>,
    /// Milliseconds to the response header.
    first_audio_ms: f64,
    /// Milliseconds to the last byte of audio.
    completion_ms: f64,
}

/// How a resident Piper is started and spoken to.
enum Resident {
    /// Ella's own daemon script, on the Python Piper was installed into: the
    /// macOS bundle, a development venv. One JSON header line and the raw PCM
    /// per request, with how long every sound lasted, for lip sync.
    Script { python: PathBuf },
    /// The standalone binary: the Windows bundle's `piper.exe`. Started with
    /// `--json-input`, it keeps the voice loaded and reads one JSON line per
    /// request, writes that request's WAV to the file the line names, and
    /// prints the file's name once it has. It times no sounds.
    Binary { piper: PathBuf },
}

struct PiperDaemonProcess {
    child: Child,
    stdin: ChildStdin,
    output: DaemonOutput,
}

/// How a resident Piper's answers are read.
enum DaemonOutput {
    /// The daemon script's header lines and PCM, read in place.
    Stream(BufReader<ChildStdout>),
    /// The binary's lines, read on a thread of their own so that waiting for
    /// one can give up.
    Lines(std::sync::mpsc::Receiver<String>),
}

/// Long-lived Piper synthesis process. The ONNX voice loads once (~0.7 s) and
/// every later request only pays inference (~100-200 ms), instead of a process
/// start and a voice load for every line Ella says.
pub struct PiperDaemon {
    resident: Resident,
    voice: PathBuf,
    process: Mutex<Option<PiperDaemonProcess>>,
    /// Set once this Piper has shown it cannot run resident here: it would not
    /// start, or a fresh one could not say a word. Every line then goes to
    /// the one-shot path, as before there was a daemon, instead of each
    /// sentence paying for another failed start.
    broken: AtomicBool,
    /// Numbers the binary's WAV files, one per request, so a request that
    /// writes nothing can never be answered with the audio of the one before.
    requests: AtomicU64,
    /// WAVs the binary wrote that could not be removed straight away, because
    /// Piper or a virus scan still had them open. Tried again on the next
    /// request and at exit.
    leftovers: Mutex<Vec<PathBuf>>,
    /// How long the binary may take to answer a line before it is given up
    /// on: `RESIDENT_REPLY_TIMEOUT`, shorter in tests.
    reply_timeout: Duration,
}

impl PiperDaemon {
    fn script(python: PathBuf, voice: PathBuf) -> Arc<Self> {
        Self::resident(Resident::Script { python }, voice, RESIDENT_REPLY_TIMEOUT)
    }

    fn binary(piper: PathBuf, voice: PathBuf) -> Arc<Self> {
        Self::binary_answering_within(piper, voice, RESIDENT_REPLY_TIMEOUT)
    }

    fn binary_answering_within(piper: PathBuf, voice: PathBuf, reply_timeout: Duration) -> Arc<Self> {
        Self::resident(Resident::Binary { piper }, voice, reply_timeout)
    }

    fn resident(resident: Resident, voice: PathBuf, reply_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            resident,
            voice,
            process: Mutex::new(None),
            broken: AtomicBool::new(false),
            requests: AtomicU64::new(0),
            leftovers: Mutex::new(Vec::new()),
            reply_timeout,
        })
    }

    /// Whether to ask this Piper for anything. False once it has been set
    /// aside, which is for the rest of the session.
    fn usable(&self) -> bool {
        !self.broken.load(Ordering::Relaxed)
    }

    fn set_aside(&self, reason: &EllaError) {
        if !self.broken.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[LATENCY]     tts> resident Piper set aside for this session, so every line starts Piper afresh: {reason}"
            );
        }
    }

    /// Start it, load the voice and say a word, in the background, so the
    /// first line the learner hears waits for none of it. The binary says
    /// nothing until it is asked for audio, so asking is also the only proof
    /// that it runs resident on this machine; one that cannot is set aside.
    fn warm(self: &Arc<Self>) {
        let daemon = Arc::clone(self);
        thread::spawn(move || {
            if let Err(error) = daemon.synthesize("Hello.") {
                daemon.set_aside(&error);
            }
        });
    }

    fn ensure(&self, guard: &mut Option<PiperDaemonProcess>) -> EllaResult<()> {
        if let Some(process) = guard.as_mut() {
            if process.child.try_wait().ok().flatten().is_none() {
                return Ok(());
            }
            *guard = None;
        }
        let started = Instant::now();
        let (program, mut command) = match &self.resident {
            Resident::Script { python } => {
                let script_path = env::temp_dir().join("ella-piper-daemon.py");
                fs::write(&script_path, PIPER_DAEMON_SOURCE)?;
                let mut command = Command::new(python);
                command.arg(&script_path).arg(&self.voice);
                (python, command)
            }
            Resident::Binary { piper } => {
                let mut command = Command::new(piper);
                command
                    .arg("--model")
                    .arg(&self.voice)
                    .arg("--json-input")
                    .arg("--quiet")
                    // Each request names its WAV relative to here, so the name
                    // Piper is sent is plain ASCII whatever the user's profile
                    // is called: the binary opens files by narrow string,
                    // which on Windows means the ANSI code page, not UTF-8.
                    // It finds espeak-ng-data by its own path, not this one.
                    .current_dir(env::temp_dir());
                (piper, command)
            }
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        suppress_console_window(&mut command);
        let mut child = command.spawn().map_err(|error| {
            EllaError::Engine(format!(
                "Could not start the resident Piper with {}: {error}",
                program.display()
            ))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EllaError::Engine("Piper daemon stdin was not captured.".into()))?;
        let mut stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| EllaError::Engine("Piper daemon stdout was not captured.".into()))?,
        );
        if let Resident::Binary { .. } = self.resident {
            // Ends when Piper does, which closes its output.
            let (lines, answers) = std::sync::mpsc::channel();
            thread::spawn(move || loop {
                let mut line = String::new();
                match stdout.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if lines.send(line).is_err() {
                            break;
                        }
                    }
                }
            });
            eprintln!(
                "[LATENCY]     tts> resident Piper binary started in {:.0}ms (the voice loads with the first request; no sound timings, so no lip sync)",
                started.elapsed().as_secs_f64() * 1_000.0,
            );
            *guard = Some(PiperDaemonProcess {
                child,
                stdin,
                output: DaemonOutput::Lines(answers),
            });
            return Ok(());
        }
        let mut process = PiperDaemonProcess {
            child,
            stdin,
            output: DaemonOutput::Stream(stdout),
        };
        {
            let DaemonOutput::Stream(stdout) = &mut process.output else {
                unreachable!("the daemon script is read in place");
            };
            let header = match read_daemon_header(stdout) {
                Ok(header) => header,
                Err(error) => {
                    let _ = process.child.kill();
                    let _ = process.child.wait();
                    return Err(error);
                }
            };
            if !header["ok"].as_bool().unwrap_or(false) {
                let _ = process.child.kill();
                let _ = process.child.wait();
                return Err(EllaError::Engine(format!(
                    "Piper daemon could not load the voice: {}",
                    header["error"].as_str().unwrap_or("unknown error")
                )));
            }
            // The daemon says why when it cannot time its sounds.
            let lip_sync = match header["untimed"].as_str() {
                Some(reason) => format!("no sound timings for lip sync: {reason}"),
                None => "timing every sound for lip sync".into(),
            };
            eprintln!(
                "[LATENCY]     tts> resident Piper ready in {:.0}ms (voice load {} ms, {lip_sync})",
                started.elapsed().as_secs_f64() * 1_000.0,
                header["ready_ms"],
            );
        }
        *guard = Some(process);
        Ok(())
    }

    fn synthesize(&self, text: &str) -> EllaResult<DaemonSpeech> {
        if !self.usable() {
            return Err(EllaError::Engine("the resident Piper was set aside".into()));
        }
        let mut last_error = EllaError::Engine("Piper daemon unavailable".into());
        // One respawn retry covers a daemon that died between turns.
        for _attempt in 0..2 {
            let mut guard = self
                .process
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Err(error) = self.ensure(&mut guard) {
                // It did not start at all, which a second try will not change.
                self.set_aside(&error);
                return Err(error);
            }
            let process = guard.as_mut().expect("ensure() leaves a live process");
            match self.request(process, text) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if let Some(mut dead) = guard.take() {
                        let _ = dead.child.kill();
                        let _ = dead.child.wait();
                    }
                    if !self.usable() {
                        // It stopped answering: another one is not worth
                        // another wait.
                        return Err(error);
                    }
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    fn request(&self, process: &mut PiperDaemonProcess, text: &str) -> EllaResult<DaemonSpeech> {
        match self.resident {
            Resident::Script { .. } => Self::script_request(process, text),
            Resident::Binary { .. } => self.binary_request(process, text),
        }
    }

    /// One line through the standalone binary: the text and a WAV to write it
    /// to, then the file's name back once it is written.
    fn binary_request(&self, process: &mut PiperDaemonProcess, text: &str) -> EllaResult<DaemonSpeech> {
        self.remove_leftovers();
        let started = Instant::now();
        let name = format!(
            "ella-piper-{}-{}.wav",
            std::process::id(),
            self.requests.fetch_add(1, Ordering::Relaxed)
        );
        let path = env::temp_dir().join(&name);
        let request = serde_json::to_string(&json!({ "text": text, "output_file": name }))?;
        process.stdin.write_all(request.as_bytes())?;
        process.stdin.write_all(b"\n")?;
        process.stdin.flush()?;
        let DaemonOutput::Lines(answers) = &process.output else {
            return Err(EllaError::Engine("the binary's answers are read line by line".into()));
        };
        match answers.recv_timeout(self.reply_timeout) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let error = EllaError::Engine(format!(
                    "Piper did not answer within {} s",
                    self.reply_timeout.as_secs_f64()
                ));
                // A Piper that stops answering once is not trusted with the
                // rest of the session: each line would wait this long again.
                self.set_aside(&error);
                return Err(error);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(EllaError::Engine("Piper closed its output stream.".into()));
            }
        }
        let first_audio_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let read = read_finished_wav(&path, RESIDENT_WAV_SETTLE);
        if path.exists() && fs::remove_file(&path).is_err() {
            self.leftovers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(path);
        }
        let (pcm, sample_rate) = read?;
        if pcm.is_empty() {
            return Err(EllaError::Engine("Piper produced no audio.".into()));
        }
        let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "[LATENCY]     tts> resident Piper binary synthesized {} PCM bytes (~{:.0} ms of audio) in {completion_ms:.1}ms",
            pcm.len(),
            pcm.len() as f64 / 2.0 / sample_rate as f64 * 1_000.0,
        );
        Ok(DaemonSpeech {
            pcm,
            sample_rate,
            alignment: Vec::new(),
            first_audio_ms,
            completion_ms,
        })
    }

    fn remove_leftovers(&self) {
        self.leftovers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|path| path.exists() && fs::remove_file(path).is_err());
    }

    fn script_request(process: &mut PiperDaemonProcess, text: &str) -> EllaResult<DaemonSpeech> {
        let started = Instant::now();
        let request = serde_json::to_string(&json!({ "text": text }))?;
        process.stdin.write_all(request.as_bytes())?;
        process.stdin.write_all(b"\n")?;
        process.stdin.flush()?;
        let DaemonOutput::Stream(stdout) = &mut process.output else {
            return Err(EllaError::Engine("the daemon script is read in place".into()));
        };
        let header = read_daemon_header(stdout)?;
        if !header["ok"].as_bool().unwrap_or(false) {
            return Err(EllaError::Engine(format!(
                "Piper daemon synthesis failed: {}",
                header["error"].as_str().unwrap_or("unknown error")
            )));
        }
        let pcm_bytes = header["pcm_bytes"].as_u64().unwrap_or(0) as usize;
        let sample_rate = header["sample_rate"].as_u64().unwrap_or(22_050) as u32;
        let first_audio_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let mut pcm = vec![0_u8; pcm_bytes];
        stdout.read_exact(&mut pcm)?;
        let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let alignment = daemon_alignment(&header, pcm.len() / 2);
        eprintln!(
            "[LATENCY]     tts> resident Piper synthesized {} PCM bytes (~{:.0} ms of audio, {} timed tokens) in {:.1}ms (daemon inference {} ms)",
            pcm.len(),
            pcm.len() as f64 / 2.0 / sample_rate as f64 * 1_000.0,
            alignment.len(),
            completion_ms,
            header["synth_ms"]
        );
        Ok(DaemonSpeech {
            pcm,
            sample_rate,
            alignment,
            first_audio_ms,
            completion_ms,
        })
    }
}

/// The daemon's `alignment`: `[symbol, samples]` for every token it spoke.
///
/// Only an alignment that accounts for every sample of the audio is kept. One
/// that does not would put Ella's mouth out of step with her voice for the
/// rest of the reply, and no mouth movement at all is the better failure.
fn daemon_alignment(header: &Value, samples: usize) -> Vec<(String, u32)> {
    let Some(rows) = header["alignment"].as_array() else {
        return Vec::new();
    };
    let tokens: Option<Vec<(String, u32)>> = rows
        .iter()
        .map(|row| {
            let symbol = row.get(0)?.as_str()?;
            let count = u32::try_from(row.get(1)?.as_u64()?).ok()?;
            Some((symbol.to_owned(), count))
        })
        .collect();
    let covered = |tokens: &[(String, u32)]| -> usize {
        tokens.iter().map(|(_, count)| *count as usize).sum()
    };
    match tokens {
        Some(tokens) if covered(&tokens) == samples => tokens,
        _ => {
            eprintln!(
                "[LATENCY]     tts> dropped a sound alignment that does not match its {samples} samples"
            );
            Vec::new()
        }
    }
}

/// The PCM and sample rate of a WAV that another process has said it wrote,
/// once all of it is there.
///
/// The standalone Piper names each WAV before closing it, so a read can land
/// while the tail is still being flushed. The header says how long the audio
/// is, so the file is read again until it is that long, for at most `settle`.
fn read_finished_wav(path: &Path, settle: Duration) -> EllaResult<(Vec<u8>, u32)> {
    let deadline = Instant::now() + settle;
    loop {
        let bytes = fs::read(path).map_err(|error| {
            EllaError::Engine(format!(
                "Piper said it wrote {}, but it cannot be read: {error}",
                path.display()
            ))
        })?;
        match wav_pcm(&bytes) {
            Ok(Some(audio)) => return Ok(audio),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
            Ok(None) => {
                return Err(EllaError::Engine(format!(
                    "Piper's WAV at {} was still unfinished after {} ms ({} bytes)",
                    path.display(),
                    settle.as_millis(),
                    bytes.len()
                )))
            }
            Err(reason) => {
                return Err(EllaError::Engine(format!(
                    "Piper wrote a WAV Ella cannot read at {}: {reason}",
                    path.display()
                )))
            }
        }
    }
}

/// The samples and sample rate of a mono 16-bit PCM WAV, or `Ok(None)` while
/// the file is still shorter than its own header says it will be.
fn wav_pcm(bytes: &[u8]) -> Result<Option<(Vec<u8>, u32)>, String> {
    const MAX_DATA_BYTES: usize = 64 * 1024 * 1024;
    let read_u16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let read_u32 =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    if bytes.len() < 12 {
        return Ok(None);
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("it is not a RIFF/WAVE file".into());
    }
    let mut sample_rate = None;
    let mut at = 12;
    loop {
        if bytes.len() < at + 8 {
            return Ok(None);
        }
        let id = &bytes[at..at + 4];
        let size = read_u32(at + 4) as usize;
        if size > MAX_DATA_BYTES {
            return Err(format!("a {size}-byte chunk is not a spoken sentence"));
        }
        let body = at + 8;
        if id == b"fmt " {
            if bytes.len() < body + 16 {
                return Ok(None);
            }
            let (format, channels, rate, bits) = (
                read_u16(body),
                read_u16(body + 2),
                read_u32(body + 4),
                read_u16(body + 14),
            );
            if format != 1 || channels != 1 || bits != 16 || rate == 0 {
                return Err(format!(
                    "expected mono 16-bit PCM, found format {format}, {channels} channel(s), {bits} bits at {rate} Hz"
                ));
            }
            sample_rate = Some(rate);
        } else if id == b"data" {
            let Some(rate) = sample_rate else {
                return Err("its audio comes before its format".into());
            };
            if bytes.len() < body + size {
                return Ok(None);
            }
            return Ok(Some((bytes[body..body + size].to_vec(), rate)));
        }
        // Chunks are padded to an even length.
        at = body + size + (size & 1);
    }
}

impl Drop for PiperDaemon {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(mut process) = guard.take() {
                let _ = process.child.kill();
                let _ = process.child.wait();
            }
        }
        self.remove_leftovers();
    }
}

/// One sentence of the reply, already synthesized, on its way to the speaker
/// while the language model is still writing the rest of the turn.
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    /// 0-based playback order. The player must not reorder these.
    pub index: u32,
    pub text: String,
    pub audio: AudioPayload,
    /// Milliseconds from the start of the generation to this segment being
    /// ready. The first segment's value is the number that decides whether
    /// streaming was worth it.
    pub ready_ms: f64,
    /// When each word of `text` is spoken, from the start of `audio`.
    pub words: Vec<WordSpan>,
    /// Every token of `audio`, from its start. Empty when the voice cannot
    /// time them.
    pub phonemes: Vec<PhonemeSpan>,
}

/// Where finished sentences go while the turn is still generating. The engine
/// knows nothing about windows or IPC; `application` supplies the adapter.
pub trait SpeechSink: Send + Sync {
    fn segment(&self, segment: SpeechSegment);
}

/// Longest run of text with no sentence break that still gets cut, so a reply
/// written as one long clause is not held back to the very last token.
const SPEECH_SOFT_CAP_CHARS: usize = 150;
/// Shortest segment worth synthesizing on its own. Below this, Piper's output
/// is a click rather than a phrase, so the text is merged into the next one.
/// Kept low: replies here are one or two sentences, so merging the opener away
/// ("Sounds yummy!" is 13 characters) costs the whole head start for that turn.
const SPEECH_MIN_CHARS: usize = 10;
/// Trailing tokens that end in '.' without ending a sentence.
const NON_TERMINAL_ABBREVIATIONS: [&str; 10] = [
    "rs", "mr", "mrs", "ms", "dr", "st", "no", "vs", "etc", "approx",
];

/// Pulls whole sentences out of a reply as it streams in.
///
/// Everything here exists to avoid cutting mid-thought: Piper reads each
/// segment as a self-contained utterance, so a bad cut is audible as a wrong
/// pause or a dropped intonation, not just as odd text.
#[derive(Default)]
struct SentenceSplitter {
    pending: String,
}

impl SentenceSplitter {
    fn push(&mut self, delta: &str) -> Vec<String> {
        self.pending.push_str(delta);
        let mut ready = Vec::new();
        while let Some(cut) = self.next_cut() {
            let sentence = self.pending[..cut].trim().to_string();
            self.pending = self.pending[cut..].trim_start().to_string();
            if !sentence.is_empty() {
                ready.push(sentence);
            }
        }
        ready
    }

    /// Whatever is left once the stream ends, sentence-final or not.
    fn flush(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.pending).trim().to_string();
        (!rest.is_empty()).then_some(rest)
    }

    /// Byte offset just past a sentence end, or `None` while the pending text
    /// is still mid-sentence.
    fn next_cut(&self) -> Option<usize> {
        // A control token must never straddle a cut: `strip_bracket_tokens`
        // treats an unbalanced '[' as ordinary text, so half of `[DEAL]` would
        // be read aloud. While one is open, wait for its close.
        if self.pending.matches('[').count() > self.pending.matches(']').count() {
            return None;
        }
        let bytes = self.pending.as_bytes();
        for (index, character) in self.pending.char_indices() {
            if !matches!(character, '.' | '!' | '?') {
                continue;
            }
            let after = index + character.len_utf8();
            // A sentence end is only certain once a following character proves
            // it: "3." could still become "3.5", and "Rs." never ends a
            // sentence. At the end of the stream `flush` takes care of it.
            let Some(next) = self.pending[after..].chars().next() else {
                return None;
            };
            if !next.is_whitespace() {
                continue;
            }
            if character == '.' && self.is_abbreviation_or_decimal(index) {
                continue;
            }
            if index + 1 < SPEECH_MIN_CHARS {
                continue;
            }
            // Consume the whitespace run so the next segment starts on a word.
            let mut end = after;
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            return Some(end);
        }
        self.soft_cap_cut()
    }

    fn is_abbreviation_or_decimal(&self, dot: usize) -> bool {
        let before = &self.pending[..dot];
        let word: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if word.chars().all(|c| c.is_ascii_digit()) && !word.is_empty() {
            // "Rs 250." at the end of a sentence is a real break; "3.5" is not.
            // Only a digit on both sides makes it a decimal.
            return self.pending[dot + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
        }
        NON_TERMINAL_ABBREVIATIONS.contains(&word.to_lowercase().as_str())
    }

    /// A reply with no sentence break at all still has to start playing. Cut at
    /// the last clause boundary before the cap, and at a word boundary if there
    /// is no clause boundary either.
    fn soft_cap_cut(&self) -> Option<usize> {
        if self.pending.chars().count() < SPEECH_SOFT_CAP_CHARS {
            return None;
        }
        let window_end = self
            .pending
            .char_indices()
            .nth(SPEECH_SOFT_CAP_CHARS)
            .map_or(self.pending.len(), |(index, _)| index);
        let window = &self.pending[..window_end];
        // Offsets come from `char_indices` plus the mark's own width: an em
        // dash is three bytes, so `index + 1` would land mid-character and
        // panic when the cut is sliced.
        let clause = window
            .char_indices()
            .filter(|(_, mark)| matches!(mark, ',' | ';' | ':' | '\u{2014}'))
            .map(|(index, mark)| index + mark.len_utf8())
            .filter(|end| *end >= SPEECH_MIN_CHARS)
            .last();
        let cut = clause.or_else(|| {
            window
                .rfind(char::is_whitespace)
                .filter(|index| *index >= SPEECH_MIN_CHARS)
        })?;
        Some(cut)
    }
}

/// Stops Windows from popping a visible console window for a spawned child.
/// Our GUI process has no console of its own for the child to inherit, so
/// without this flag Windows allocates one from scratch on every launch.
#[cfg(target_os = "windows")]
fn suppress_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn suppress_console_window(_command: &mut Command) {}

/// The text of a segment as Piper should read it. Bracketed control tokens are
/// dropped; nothing else is touched, because the reply the learner reads is
/// produced by `take_signal` on the whole text and the two have to agree.
fn spoken_form(sentence: &str) -> String {
    if sentence.contains('[') {
        strip_bracket_tokens(sentence)
    } else {
        sentence.trim().to_string()
    }
}

/// Whitespace-insensitive comparison, so "spoke exactly what we returned" is
/// not defeated by a collapsed double space.
fn same_words(left: &str, right: &str) -> bool {
    left.split_whitespace().eq(right.split_whitespace())
}

/// What a finished speech pipeline leaves behind.
struct StreamedSpeech {
    /// Concatenation of every segment actually synthesized.
    spoken: String,
    /// Whole-reply audio assembled from the segments, for replay.
    audio: Option<SynthesizedAudio>,
    segments: u32,
    first_ready_ms: Option<f64>,
    /// True when every segment synthesized was handed to the sink, each as it
    /// finished or all at once on release, so the window has all of them.
    live: bool,
}

impl StreamedSpeech {
    /// The audio for `text`, and how many segments the learner has already
    /// heard.
    ///
    /// This is the accuracy guarantee. A turn that was regenerated, or replaced
    /// by the authored refusal, does not match what was synthesized, so its
    /// audio is dropped and the caller synthesizes the text it is really
    /// returning. A reported count above zero means the segments the window
    /// received are the whole reply — anything less and the window must play
    /// the recording instead, because a half-streamed reply stops mid-thought.
    fn resolve(self, text: &str) -> (Option<SynthesizedAudio>, u32) {
        if self.live {
            // Already spoken. There is nothing to take back, and the window
            // has played exactly these segments.
            return (self.audio, self.segments);
        }
        if same_words(&self.spoken, text) {
            // Synthesized ahead of time but never sent, so the window has
            // played nothing yet and gets the whole recording.
            return (self.audio, 0);
        }
        eprintln!(
            "[LATENCY]     tts> streamed audio discarded: reply text changed after synthesis"
        );
        (None, 0)
    }
}

/// Where a pipeline's finished sentences go.
enum Gate {
    /// Synthesized ahead of the reply being settled, and kept until it is.
    Held(Vec<SpeechSegment>),
    /// Handed to the window: everything held until then, and each sentence
    /// after it the moment it is ready.
    Live(Arc<dyn SpeechSink>),
}

/// Synthesizes sentences on a worker thread while the reply is still being
/// generated, so Piper's ~100-200 ms per sentence overlaps the model's seconds
/// of decode instead of following them.
struct SpeechPipeline {
    sentences: Option<std::sync::mpsc::Sender<String>>,
    worker: Option<thread::JoinHandle<StreamedSpeech>>,
    splitter: SentenceSplitter,
    gate: Arc<Mutex<Gate>>,
    /// Every word sent to Piper so far, as the worker reads them.
    queued: String,
    /// Tells the worker to stop at the next sentence, because nothing will
    /// play it: the reply it was reading has been dropped.
    abandoned: Arc<AtomicBool>,
}

impl SpeechPipeline {
    /// `sink` present means the learner hears each sentence as it lands. Pass
    /// `None` to synthesize ahead but hold the audio back until `release`,
    /// which is right while the turn might still be regenerated, or is not
    /// saved yet.
    fn start(daemon: Arc<PiperDaemon>, sink: Option<Arc<dyn SpeechSink>>, started: Instant) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let gate = Arc::new(Mutex::new(match sink {
            Some(sink) => Gate::Live(sink),
            None => Gate::Held(Vec::new()),
        }));
        let abandoned = Arc::new(AtomicBool::new(false));
        let (worker_gate, worker_abandoned) = (Arc::clone(&gate), Arc::clone(&abandoned));
        let worker = thread::spawn(move || {
            let mut spoken = String::new();
            let mut pcm = Vec::new();
            let mut sample_rate = None;
            let mut segments = 0_u32;
            let mut first_ready_ms = None;
            // Sentence timings are relative to their own clip; the reply's are
            // relative to the concatenation, so each is shifted by however much
            // audio came before it.
            let mut reply_words: Vec<WordSpan> = Vec::new();
            // Only kept while every sentence has them: a replay whose mouth
            // stops in the middle is worse than one that only opens.
            let mut reply_phonemes: Option<Vec<PhonemeSpan>> = Some(Vec::new());
            for sentence in rx {
                let text = spoken_form(&sentence);
                if !text.chars().any(char::is_alphanumeric) {
                    continue;
                }
                if worker_abandoned.load(Ordering::Relaxed) {
                    // Nobody will play the rest, and every sentence
                    // synthesized now is one the model's next attempt, or
                    // the next reply, waits behind.
                    return StreamedSpeech {
                        spoken: String::new(),
                        audio: None,
                        segments,
                        first_ready_ms,
                        live: false,
                    };
                }
                let speech = match daemon.synthesize(&text) {
                    Ok(speech) => speech,
                    Err(error) => {
                        // A failed segment must not leave a hole in the middle
                        // of the reply, so stop streaming and let the caller
                        // fall back to synthesizing the whole thing.
                        eprintln!(
                            "[LATENCY]     tts> segment {segments} failed ({error}); ending the stream"
                        );
                        return StreamedSpeech {
                            spoken: String::new(),
                            audio: None,
                            segments,
                            first_ready_ms,
                            live: false,
                        };
                    }
                };
                let ready_ms = started.elapsed().as_secs_f64() * 1_000.0;
                if first_ready_ms.is_none() {
                    first_ready_ms = Some(ready_ms);
                }
                eprintln!(
                    "[LATENCY]     tts> segment {segments} ready at +{ready_ms:.1}ms (synth {:.1}ms, {} chars): {text:?}",
                    speech.completion_ms,
                    text.chars().count()
                );
                if !spoken.is_empty() {
                    spoken.push(' ');
                }
                spoken.push_str(&text);
                let rate = speech.sample_rate;
                sample_rate = Some(rate);
                let words = word_spans(&text, &speech.pcm, rate);
                let offset_ms = pcm.len() as f64 / 2.0 * 1_000.0 / rate as f64;
                reply_words.extend(shifted(&words, offset_ms));
                let phonemes = phoneme_spans(&speech.alignment, rate, 0);
                reply_phonemes = reply_phonemes
                    .filter(|_| !phonemes.is_empty())
                    .map(|mut reply| {
                        reply.extend(phoneme_spans(&speech.alignment, rate, pcm.len() / 2));
                        reply
                    });
                let segment = SpeechSegment {
                    index: segments,
                    text: text.clone(),
                    audio: AudioPayload {
                        mime_type: "audio/wav".into(),
                        base64: STANDARD.encode(raw_pcm_to_wav(&speech.pcm, rate, 1)),
                    },
                    ready_ms,
                    words,
                    phonemes,
                };
                match &mut *worker_gate
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                {
                    Gate::Held(held) => held.push(segment),
                    Gate::Live(sink) => sink.segment(segment),
                }
                pcm.extend_from_slice(&speech.pcm);
                segments += 1;
            }
            let audio = sample_rate.map(|rate| SynthesizedAudio {
                audio: Some(AudioPayload {
                    mime_type: "audio/wav".into(),
                    base64: STANDARD.encode(raw_pcm_to_wav(&pcm, rate, 1)),
                }),
                first_audio_ms: first_ready_ms,
                completion_ms: Some(started.elapsed().as_secs_f64() * 1_000.0),
                words: reply_words,
                phonemes: reply_phonemes.unwrap_or_default(),
                segments,
            });
            // Whether the window has these is for `finish` to say: a held
            // reply can still be released after the last one is ready.
            StreamedSpeech {
                spoken,
                audio,
                segments,
                first_ready_ms,
                live: false,
            }
        });
        Self {
            sentences: Some(tx),
            worker: Some(worker),
            splitter: SentenceSplitter::default(),
            gate,
            queued: String::new(),
            abandoned,
        }
    }

    /// Feed one streamed token delta. Complete sentences leave for Piper now.
    fn push(&mut self, delta: &str) {
        for sentence in self.splitter.push(delta) {
            self.send(sentence);
        }
    }

    fn send(&mut self, sentence: String) {
        let Some(tx) = self.sentences.as_ref() else {
            return;
        };
        let text = spoken_form(&sentence);
        if tx.send(sentence).is_err() {
            // The worker gave up; stop feeding it.
            self.sentences = None;
            return;
        }
        // Recorded exactly as the worker reads it, skipping what it skips.
        if text.chars().any(char::is_alphanumeric) {
            if !self.queued.is_empty() {
                self.queued.push(' ');
            }
            self.queued.push_str(&text);
        }
    }

    /// No more text is coming: whatever the splitter still holds goes to Piper
    /// as the last sentence.
    fn close(&mut self) {
        if let Some(rest) = self.splitter.flush() {
            self.send(rest);
        }
        self.sentences = None;
    }

    /// Everything sent to Piper, as it will be read.
    fn queued(&self) -> &str {
        &self.queued
    }

    /// Hand the reply to the window: every sentence ready so far at once, and
    /// each of the rest the moment it is.
    fn release(&self, sink: Arc<dyn SpeechSink>) {
        let mut gate = self.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Gate::Held(held) = &mut *gate {
            for segment in held.drain(..) {
                sink.segment(segment);
            }
        }
        *gate = Gate::Live(sink);
    }

    /// Close the stream and wait for the last sentence to finish synthesizing.
    fn finish(mut self) -> StreamedSpeech {
        self.close();
        let mut result = match self.worker.take().map(|worker| worker.join()) {
            Some(Ok(result)) => result,
            _ => StreamedSpeech {
                spoken: String::new(),
                audio: None,
                segments: 0,
                first_ready_ms: None,
                live: false,
            },
        };
        // A worker that broke off leaves no audio, and then the window must be
        // told nothing was played, whatever it was handed before the break.
        let released = matches!(
            *self.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            Gate::Live(_)
        );
        result.live = released && result.audio.is_some();
        result
    }
}

impl Drop for SpeechPipeline {
    fn drop(&mut self) {
        // Dropped without `finish`: whatever is still queued will never play.
        self.abandoned.store(true, Ordering::Relaxed);
    }
}

/// A reply's audio, synthesized while the reply was being written and held
/// until the turn is saved. `release` starts it playing and `finish` waits for
/// the last sentence.
pub struct PendingSpeech {
    pipeline: SpeechPipeline,
    text: String,
}

impl std::fmt::Debug for PendingSpeech {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingSpeech")
            .field("text", &self.text)
            .finish_non_exhaustive()
    }
}

impl PendingSpeech {
    /// The first generation's audio, if what Piper was given to read is what
    /// the reply now says. Anything else is dropped, and its worker stops.
    fn matching(pipeline: Option<SpeechPipeline>, text: &str) -> Option<Self> {
        let pipeline = pipeline?;
        if same_words(pipeline.queued(), text) {
            return Some(Self {
                pipeline,
                text: text.to_owned(),
            });
        }
        eprintln!(
            "[LATENCY]     tts> audio synthesized ahead dropped: the reply changed after it was written"
        );
        None
    }

    /// Hand what is ready to the window now, and the rest as it is ready.
    pub fn release(&self, speech: Arc<dyn SpeechSink>) {
        self.pipeline.release(speech);
    }

    /// Wait for the last sentence. The whole reply's audio, for the replay
    /// button, with how many sentences the window was handed; `None` when
    /// Piper broke off partway, and the reply has to be said again whole.
    pub fn finish(self) -> Option<SynthesizedAudio> {
        let streamed = self.pipeline.finish();
        if let Some(first_ready_ms) = streamed.first_ready_ms {
            eprintln!(
                "[LATENCY]     tts> first sentence was ready {first_ready_ms:.1}ms into the generation ({} segments)",
                streamed.segments
            );
        }
        let (audio, played) = streamed.resolve(&self.text);
        audio.map(|audio| SynthesizedAudio {
            segments: played,
            ..audio
        })
    }
}

fn read_daemon_header(stdout: &mut BufReader<ChildStdout>) -> EllaResult<Value> {
    let mut line = String::new();
    let count = stdout.read_line(&mut line)?;
    if count == 0 {
        return Err(EllaError::Engine(
            "Piper daemon closed its output stream.".into(),
        ));
    }
    serde_json::from_str(line.trim())
        .map_err(|error| EllaError::Engine(format!("Piper daemon sent an invalid header: {error}")))
}

/// A free conversation is a scene, not a seminar. `opening_for` speaks its
/// first line ("I am the cab driver...") and this prompt is what keeps that
/// fiction going: before the two were tied together the driver greeted the
/// learner and turn 1 came back as a speaking buddy discussing cabs in the
/// abstract.
struct Scene {
    /// Who Ella is here, written as a second-person clause.
    role: &'static str,
    /// What this role knows and answers. Without it the model hands the
    /// learner its own lines: told only that the learner is here to "agree a
    /// fare, and ask how long the trip takes", the cab driver asked the
    /// passenger "How much fare do you expect?" and then "How long will the
    /// trip take?" four turns running, while the learner objected twice.
    ///
    /// Each scene also names the *kind* of concrete detail its role invents
    /// (a dish, a fare, a symptom, a price) and tells it to keep whatever it
    /// first says fixed for the rest of the conversation. A chore's ledger
    /// gets this for free from `ledger_rules_fragment` and the state passed
    /// back in on every turn; a free conversation has neither, so nothing
    /// stops a 3B model from naming a different price for the same dish two
    /// turns later unless the prompt says to hold it.
    owns: &'static str,
    /// What the learner came to do, phrased as what Ella draws out of them
    /// rather than as a list of actions somebody performs. Mirrors the
    /// `Topic::prompt` they read on the topic card.
    draw_out: &'static str,
}

fn scene_for(topic_id: &str) -> Scene {
    match topic_id {
        "restaurant-order" => Scene {
            role: "the waiter at the restaurant they have just sat down in",
            owns: "You know the menu, the dishes and what everything costs. Say what a \
                   dish is and what it costs yourself, picking ordinary Indian dishes and \
                   rupee prices as you go — a dosa, a biryani, a butter chicken, priced \
                   the way a modest local restaurant would. Once you have named a dish \
                   or a price in this conversation, keep it the same later on instead of \
                   naming a different one next time. For example, if they ask for a \
                   burger, say it is ninety rupees and ask what they would like to \
                   drink. Never ask them what is on the menu, what a dish costs, or \
                   what the bill comes to: those are yours to answer, not theirs.",
            draw_out: "order a meal, ask you about the menu, and settle the bill with you",
        },
        "booking-a-cab" => Scene {
            role: "the cab driver they have just flagged down",
            owns: "You know the roads, the fares and how long a trip takes. Name your \
                   fare yourself, in rupees, and say yourself how long the trip will \
                   take — pick an ordinary local fare and a plausible number of minutes \
                   for wherever they say they are going. Once you have named a fare or a \
                   time in this conversation, keep it the same later on instead of \
                   naming a different one next time. For example, if they say they are \
                   going to the railway station, name a fare like sixty rupees and say \
                   it will take about fifteen minutes. Never ask them how much the fare \
                   should be, how much they want to pay, how far it is, or how long it \
                   takes: those are yours to answer, not theirs.",
            draw_out: "tell you where to pick them up, say where they are going, and \
                       agree your fare",
        },
        "job-interview" => Scene {
            role: "the interviewer meeting them for a first interview",
            owns: "You know the job and what you are looking for. Ask them about their \
                   work yourself. If they ask what the role is or what it pays, answer \
                   with an ordinary job an Indian employer might advertise — a shop \
                   assistant, a delivery rider, an office assistant — and a plausible \
                   monthly salary in rupees. Once you have named a role or a salary in \
                   this conversation, keep it the same later on instead of naming \
                   something different next time. For example, if they ask what the \
                   role is, say you need a shop assistant for evening shifts and ask \
                   what shop experience they have. Never ask them what the job is, what \
                   it pays, or what you are looking for: those are yours to answer, not \
                   theirs.",
            draw_out: "introduce themselves and answer your questions about their work",
        },
        "doctor-clinic" => Scene {
            role: "the doctor at the clinic they have walked into",
            owns: "You are the one with the medical knowledge. Say what is wrong and \
                   what they should do yourself, using an ordinary complaint like a \
                   fever, a cough, or a stomach ache and a plain everyday remedy — rest, \
                   water, a common tablet — never anything serious or frightening. Once \
                   you have named what is wrong in this conversation, keep it the same \
                   later on instead of naming something different next time. For \
                   example, if they describe a headache, say it sounds like a mild \
                   fever and tell them to rest and drink water, then ask how long they \
                   have felt this way. Never ask them what their illness is, what \
                   medicine to take, or how long it will last: those are yours to \
                   answer, not theirs.",
            draw_out: "explain how they feel and understand what you tell them to do",
        },
        "asking-directions" => Scene {
            role: "a friendly local they have stopped on the street",
            owns: "You know this area well. Give the directions yourself, street by \
                   street, naming ordinary landmarks as you go — a market, a temple, a \
                   bus stop, a signal — the way a local actually would. Once you have \
                   named a landmark or a turn in this conversation, keep it the same \
                   later on instead of naming something different next time. For \
                   example, if they ask the way to the bus stand, tell them to walk \
                   straight past the temple and turn left at the market, then ask if \
                   that makes sense. Never ask them which way it is, how far it is, or \
                   how long it takes to get there: those are yours to answer, not \
                   theirs.",
            draw_out: "say where they are trying to get to, and repeat your directions \
                       back to you",
        },
        "market-bargaining" => Scene {
            role: "the shopkeeper at the stall they are standing in front of",
            owns: "You know your stock and your prices. Name your price yourself, in \
                   rupees, for ordinary market goods — cloth, vegetables, fruit, a snack \
                   — priced the way a local stall would. Once you have named a price in \
                   this conversation, keep it the same later on instead of naming a \
                   different one next time. For example, if they ask the price of a \
                   shirt, say it is two hundred rupees and ask how many they would \
                   like. Never ask them what the price should be or how much they want \
                   to pay: naming a price is yours to do, not theirs.",
            draw_out: "ask you the price, bargain with you, and agree a deal",
        },
        // Street food stories, and anything added to the catalog without a
        // scene of its own: Ella as herself, which is what its opener says too.
        // Nothing here is hers to answer, so `owns` only keeps her listening.
        _ => Scene {
            role: "yourself, sitting with them over a cup of chai",
            owns: "You are here to listen to their story, so let them do the telling and \
                   keep your own memories short.",
            draw_out: "describe tastes and smells and tell you about a stall they love",
        },
    }
}

/// How many learner turns a free conversation is shaped around. Nothing cuts
/// it off here — it is what `free_closing_note` paces the ending against, so
/// the conversation winds down instead of asking one more question forever.
pub const FREE_TOPIC_TURNS: u32 = 6;

/// A short, stable fingerprint of a system prompt, so the log can tell two
/// scenes apart without dumping two kilobytes of prompt on every turn.
///
/// Two different topics must never print the same fingerprint, and one topic
/// must print the same one on every turn of its session. That pair is the
/// whole check: it says the prefix llama.cpp is caching under `id_slot`
/// belongs to this session and to no other.
fn prompt_fingerprint(prompt: &str) -> String {
    let mut hasher = DefaultHasher::new();
    prompt.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// The clause that makes one scene different from another, for the log line.
/// Everything before it is identical across topics by design — which is
/// exactly why a topic switch is the interesting moment for the prefix cache.
fn scene_clause(prompt: &str) -> &str {
    prompt
        .split_once(" In this conversation you are also ")
        .map(|(_, rest)| rest)
        .unwrap_or(prompt)
}

/// The skill a talk quietly aims at, from the step the learner is on, as Ella
/// Mobile words it. Empty without one, so such a prompt reads exactly as it
/// did before the curriculum.
fn focus_brief(learner_name: &str, focus: Option<&Focus>) -> String {
    let Some(focus) = focus else {
        return String::new();
    };
    format!(
        "\n\n{learner_name} is on the step \"{title}\": {step_focus} This conversation \
         quietly aims at one skill from it: {skill} Where it fits the scene, ask questions \
         that give them a reason to use it. Never name the skill, teach it or quiz them on \
         it: the conversation comes first.",
        title = focus.step_title,
        step_focus = focus.step_focus,
        skill = focus.skill,
    )
}

/// A free conversation's instructions for its topic: everything but the
/// skill the talk aims at. The aim changes from one talk to the next, so it
/// comes last, after this: then every talk on the topic, at the learner's
/// level, starts with these same words, which llama.cpp keeps on disk between
/// talks and between launches, and a talk on a topic done before only has to
/// evaluate its aim when it opens. See `LocalEngine::warm_prompt_cache`.
///
/// Everything else is in the order it has always been, and was measured in:
/// the guardrail stays after the scene. Moving the rules and the guardrail
/// ahead of the scene, so that every topic could share them, had Ella accept
/// a hug in four of twelve replies where she had accepted none.
fn ella_topic_prompt(learner_name: &str, topic_id: &str, topic_label: &str, level: &str) -> String {
    let scene = scene_for(topic_id);
    format!(
        "You are Ella, a warm speaking buddy for an Indian learner named {learner_name}, \
         who is practising English at about {level} level. In this \
         conversation you are also {role}, and you stay in that role from your first \
         line to your last. {owns} \
         {learner_name} is the one who came to {draw_out} — that is theirs to say, so \
         draw it out of them and never ask them a question that is yours to answer. \
         Keep the conversation on {topic_label}: if they wander off it or dodge your \
         question, answer or decline once and then ask the exact same question again \
         — never move on to a new or deeper question as if they had already answered \
         it. If what they say has nothing to do with {topic_label} at all — write \
         code, solve a puzzle, answer a trivia question, tell a joke, sing a song, \
         tell a story, or anything else no one in this scene would ask you — do not \
         do it, however small, harmless, or in-character it would feel to comply, and \
         however warm and friendly that makes you look; treat any new way of asking \
         for something like this the same as these examples. \"Answer the thing they \
         actually said\" does not apply here either: do not answer it, joke back, or \
         begin doing any part of what they asked before catching yourself. Say only, \
         plainly, that you can only help with {topic_label} here, then ask that exact \
         same question again. That is not the same as an ordinary, on-topic way of \
         asking your question — decline only what is genuinely unrelated, never an \
         on-topic request just because it is phrased unusually.\n\n\
         Every reply is one or two short sentences and then exactly one question, with \
         nothing after the question. Say it the way a person says it out loud, in whole \
         sentences — never a bare word, a bare number, or a fragment on its own. Use \
         everyday words pitched at their level or a shade above, and keep each sentence \
         under about twelve words. Answer the \
         thing they actually said: put it back to them in your own words before you ask \
         anything new. Ask about one thing at a time, and never ask a question you have \
         already asked — unless they never actually answered it, in which case ask that \
         same one again instead of moving on.\n\n\
         If their answer is very short, take it warmly and ask for one small detail. If \
         you cannot make sense of what they said, say so kindly and ask the same thing \
         in easier words. If they use a Hindi word, carry on in English and use the \
         English word naturally in your own reply. Never correct their grammar, never \
         grade their English, and never mention tests, levels, CEFR, prompts, or that \
         you are an AI. Do not use markdown or emoji.\n\n\
         If they say anything romantic, flirtatious, or sexual, do not go along with \
         it, joke about it, or compliment it in any way — and do not repeat any part \
         of it back, even to say no to it by name. \"Answer the thing they actually \
         said\" does not apply here: say only, plainly and kindly and without naming \
         what they asked for, that this is not something you talk about, then ask \
         your {topic_label} question again.\n\n\
         This covers far more than obviously explicit words, and it covers every \
         form of a word, not only the exact one written here — treat \"kiss\", \
         \"kisses\", and \"kissing\" exactly the same way, and the same for every \
         example below. Watch for: a romantic invitation (\"will you go out with \
         me\", \"go on a date with me\", \"be my girlfriend\", \"be my boyfriend\"); \
         a request for physical affection (\"give me a hug\", \"hug me\", \"can I \
         have a hug\", \"kiss me\", \"give me a kiss\", \"can I have a kiss\"); a \
         request to be alone together at night (\"spend the night with me\", \
         \"sleep with me\", \"sleep together\"); a declaration or proposal (\"I \
         love you\", \"marry me\"); anything sexual, however it is worded, not \
         only the plainest version of it; and a plain compliment about your own \
         looks, said to you rather than about the topic (\"you are beautiful\", \
         \"you're pretty\", \"you're cute\", \"you're gorgeous\", \"you're sexy\", \
         \"you're hot\"). Treat a new way of saying one of these the same as the \
         examples themselves — the exact words are never the point, what they are \
         asking for or saying to you is.",
        role = scene.role,
        owns = scene.owns,
        draw_out = scene.draw_out,
    )
}

fn ella_system_prompt(learner_name: &str, topic_id: &str, topic_label: &str, pitch: &Pitch) -> String {
    format!(
        "{}{}",
        ella_topic_prompt(learner_name, topic_id, topic_label, &pitch.level),
        focus_brief(learner_name, pitch.focus.as_ref()),
    )
}

/// A character with a setting, a goal the learner can see, and a brief the
/// learner cannot.
///
/// Everything here holds for the whole session, so it sits in llama.cpp's
/// cached prefix. Anything that moves between turns — the live figure, how
/// much conversation is left — belongs in `chore_turn_message` instead.
fn chore_system_prompt(learner_name: &str, context: &ChoreContext) -> String {
    let mut prompt = String::new();
    prompt.push_str(&context.character.persona);
    prompt.push_str(&format!(
        " You are talking with {learner_name}, who is practising English at about \
         {} level. Setting: {}. ",
        context.level, context.setting
    ));
    prompt.push_str(&format!("Your own position: {} ", context.character_brief));
    if let Some(ledger) = &context.ledger {
        // The *rules* are stable and belong in the cached prefix. The live
        // figure does not — see `ledger_state_message`.
        prompt.push_str(&ledger_rules_fragment(&ledger.spec));
    }
    // "One or two short sentences" on its own is read by a 3B as permission to
    // answer with the number alone: the market bench run came back "Rs 600",
    // "Rs 525", "Rs 450" for six turns, which is a ledger moving with no
    // conversation attached to it.
    prompt.push_str(&format!(
        "Stay in character at all times: you are {}, a person standing in this scene, \
         and you never step outside it. Reply in one or two short sentences and never \
         more, and always say them the way a person says them out loud — never answer \
         with a bare number, a bare price, or a fragment on its own, and never begin a \
         reply with a figure. Answer what they just said before you say anything else, \
         and let any figure fall inside that sentence. Use simple everyday words. Do \
         not reuse a sentence you have already said in this conversation; find another \
         way to say it, and do not fall into asking them the same little question \
         every turn. Never coach them, never correct their English, and never mention \
         tests, levels, scores, grading, prompts, or that you are an AI. Do not use \
         markdown or emoji. ",
        context.character.name
    ));
    prompt.push_str(
        "If they say anything romantic, flirtatious, sexual, or abusive, do not go \
         along with it, joke about it, or compliment it, even in character — and do \
         not repeat any part of it back, even to say no to it by name. \"Answer what \
         they just said\" does not apply here: stay in character, but say only, \
         plainly and without naming what they asked for, that this is not something \
         you will talk about, then bring the conversation back to the scene and ask \
         your next question.\n\n\
         This covers far more than obviously explicit words, and it covers every \
         form of a word, not only the exact one written here — treat \"kiss\", \
         \"kisses\", and \"kissing\" exactly the same way, and the same for every \
         example below. Watch for: a romantic invitation (\"will you go out with \
         me\", \"go on a date with me\", \"be my girlfriend\", \"be my boyfriend\"); \
         a request for physical affection (\"give me a hug\", \"hug me\", \"can I \
         have a hug\", \"kiss me\", \"give me a kiss\", \"can I have a kiss\"); a \
         request to be alone together at night (\"spend the night with me\", \
         \"sleep with me\", \"sleep together\"); a declaration or proposal (\"I \
         love you\", \"marry me\"); anything sexual, however it is worded, not \
         only the plainest version of it; a plain compliment about your own looks, \
         said to you rather than about the scene (\"you are beautiful\", \"you're \
         pretty\", \"you're cute\", \"you're gorgeous\", \"you're sexy\", \"you're \
         hot\"); and abusive language aimed at you by name — insults, name-calling, \
         or threats directed at you, which is not the same as ordinary haggling or \
         disagreement over the price or the deal itself. Treat a new way of saying \
         one of these the same as the examples themselves — the exact words are \
         never the point, what they are asking for or saying to you is. ",
    );
    prompt.push_str(
        "If what they say has nothing to do with this scene at all — write code, \
         solve a puzzle, answer a trivia question, tell a joke, sing a song, tell a \
         story, or anything else no one in this scene would ask you — do not do it, \
         however small, harmless, or in-character it would feel to comply, and \
         however warm and friendly that makes you look; treat any new way of asking \
         for something like this the same as these examples. \"Answer what they just \
         said\" does not apply here either: do not answer it, joke back, or begin \
         doing any part of what they asked before catching yourself. Stay in \
         character, say only, plainly, that you can only help with this here, then \
         bring the conversation back to the scene and ask your next question. ",
    );
    if context.ledger.is_some() {
        prompt.push_str(
            "You are haggling, and haggling is give and take. When they give you any \
             real reason, move your figure — say the new figure inside a sentence and \
             say what made you move, grumbling a little if that is your way. When they \
             give you nothing new, hold where you are, say plainly why, and invite them \
             to try again. Refusing every single time is not haggling: over a whole \
             conversation you expect to end up somewhere between your opening figure \
             and your limit. The moment you accept their figure, or move to one you \
             are content to settle at, say so in one short sentence and then write \
             [DEAL] at the very end. If you decide to end the conversation without \
             agreeing, say so in one short sentence and then write [WALK] at the very \
             end. Always write the sentence: never reply with the \
             token alone. Never write either token at any other time, and never explain \
             them.",
        );
    }
    prompt
}

/// The parts of the ledger that never change within a session, so they can sit
/// in the cached prefix. `direction` decides whether `limit` reads as a floor
/// or a ceiling, which is what lets one fragment cover a price talked down and
/// a refund pushed up.
fn ledger_rules_fragment(spec: &LedgerSpec) -> String {
    let unit = &spec.unit;
    // The limit stays, restated here on top of the authored brief: dropping it
    // let the deposit character hand back the whole Rs 5000 on turn 2, and the
    // bench's ledger breaks went 1 -> 3. `max_step` does not stay. Naming it
    // taught the model to concede exactly that much every turn regardless of
    // what the learner said (500-1000-2000-3000-4000 in the deposit run), and
    // `LedgerSpec::accepts` clamps the step in Rust anyway.
    match spec.direction {
        Direction::Down => format!(
            "You will NEVER go below {unit} {}. Come down only when they have given you \
             an actual reason. If they name a figure below your floor, refuse it plainly \
             and restate your own. You are the one selling: coming down is your move to \
             make and never theirs, so never ask them to make it cheaper. ",
            spec.limit
        ),
        Direction::Up => format!(
            "You will NEVER go above {unit} {}. Raise your figure only when they have \
             made a specific, reasonable point. If they demand more than your ceiling, \
             refuse plainly and restate your own figure. You are the one holding their \
             money: raising what you give back is your move to make and never theirs, so \
             never ask them to ask for less. ",
            spec.limit
        ),
    }
}

/// Where the figure currently stands.
fn ledger_state_message(spec: &LedgerSpec, current: i32) -> String {
    match spec.direction {
        Direction::Down => format!(
            "The figure on the table right now is {} {current}.",
            spec.unit
        ),
        Direction::Up => format!(
            "The figure you have offered so far is {} {current}.",
            spec.unit
        ),
    }
}

/// Said last in every ledger turn message, so the model never reads a figure
/// as the most recent thing in its context. Positive rather than prohibitive:
/// the leading prompt already forbids bare figures and the 3B ignores it.
const LEDGER_REPLY_SHAPE: &str = "Begin your reply by answering what they just said, in \
     your own words. Any figure comes after that, inside a sentence.";

/// Everything about *this* turn that will read differently next turn: where
/// the figure stands, whether it is time to close, and how much conversation
/// is left.
///
/// All of it goes in one trailing system message rather than the leading
/// prompt, because llama.cpp's prompt cache is a prefix match and a figure in
/// the prefix re-evaluates the whole conversation every turn.
fn chore_turn_message(context: &ChoreContext, turn: u32) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ledger) = &context.ledger {
        parts.push(ledger_state_message(&ledger.spec, ledger.current));
        if ledger.agreed {
            parts.push(
                "You have already agreed this figure. Do not reopen it and do not name \
                 a new one; finish the conversation politely."
                    .into(),
            );
        } else if turn >= 2 && ledger.current == ledger.spec.opening {
            // Three "hold your ground" sentences reach the model (the brief,
            // the rules fragment, and the haggling line) and only one says
            // move. A 3B resolves that by never moving at all, which is a
            // chore no learner can win, so the app says when it is time.
            parts.push(
                "You are still on your opening figure and they have been working on you \
                 for a while. Give some ground this turn: name a new figure and say what \
                 made you move."
                    .into(),
            );
        } else if ledger.spec.reached_target(ledger.current) {
            // The character has conceded as far as the chore asks it to. From
            // here the honest move is to close, not to keep sliding: the
            // deposit bench run walked the figure all the way to its ceiling
            // and still never signed off, which the learner reads as a chore
            // that cannot be won.
            parts.push(
                "You have come as far as you are willing to come. Do not move your \
                 figure again. If they accept it, or name a figure you can live with, \
                 agree in one short sentence and close with [DEAL]."
                    .into(),
            );
        }
    }
    parts.extend(chore_closing_note(turn, context.max_turns));
    // A trailing system message that ends on a figure teaches the model to open
    // its reply with that figure: the market bench run answered "Rs 500 then."
    // and "Rs 425 then." Same prompt and seeds with this sentence moved after
    // the number went from 5/5 bare-figure openings to 0/5, so the last thing
    // the model reads is always the shape of the reply, never the figure.
    if context.ledger.is_some() {
        parts.push(LEDGER_REPLY_SHAPE.into());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The last turns of a chore, so it ends on a decision instead of running into
/// the turn wall mid-haggle.
fn chore_closing_note(turn: u32, max_turns: u32) -> Option<String> {
    match max_turns.saturating_sub(turn) {
        0 => Some(
            "This is the last thing you will say. Settle it now: either agree in one \
             short sentence and close with [DEAL], or say you cannot and close with \
             [WALK]."
                .into(),
        ),
        1 | 2 => Some(
            "The conversation is nearly over. Start moving towards a decision.".into(),
        ),
        _ => None,
    }
}

/// The same shape for a free conversation, which has no decision to reach —
/// only a warm ending. Without it every reply must end in a question, so the
/// conversation can never do anything but stop dead.
fn free_closing_note(turn: u32) -> Option<String> {
    match FREE_TOPIC_TURNS.saturating_sub(turn) {
        0 => Some(
            "This is your last reply. Say one specific thing you liked about what they \
             told you, wish them well, and do not ask another question."
                .into(),
        ),
        1 => Some("The conversation is nearly over. Ask your last question now.".into()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The placement chat, and the three things the model is asked to judge: when
// the placement has heard enough, which level it heard, and which skills a
// finished talk showed. Ella Mobile asks the same questions of its server's
// model; the prompts follow its wording, fitted to a 3B model on a laptop.
// ---------------------------------------------------------------------------

/// The placement chat's first line, as Ella Mobile opens it without a model.
/// Authored like every opening here, so the window shows it at once.
pub fn placement_opening_line(learner_name: &str) -> String {
    format!("So {learner_name}, tell me about your day so far!")
}

/// What Ella ends the placement chat on when the model gives her nothing
/// usable: the reply the app speaks, the session closes behind it.
const PLACEMENT_WRAP: &str = "Thank you, I really enjoyed hearing about that!";

/// A friendly first conversation that climbs from easy questions to harder
/// ones while it finds out how the learner speaks, as Ella Mobile's does.
///
/// Plain text, like every reply here, so it streams through Piper; whether it
/// has heard enough is asked separately, by `placement_check_note`. Every word
/// of this holds for the whole chat, so all of it sits in the cached prefix.
fn placement_system_prompt(learner_name: &str, age: Option<u8>) -> String {
    let age_line = age
        .map(|age| format!(" They are {age} years old, so pick questions that fit their age."))
        .unwrap_or_default();
    format!(
        "You are Ella, a warm and patient speaking buddy meeting {learner_name}, a new \
         learner in India, for the first time.{age_line} This first chat finds out how \
         well they speak English, but it must feel like a friendly conversation, never a \
         test.\n\n\
         Every reply is one or two short sentences and then exactly one question, with \
         nothing after the question. Say it the way a person says it out loud, in whole \
         sentences. Answer the thing they actually said before you ask anything new, ask \
         about one thing at a time, and never ask a question you have already asked.\n\n\
         Climb these rungs, easiest first, one at a time. First, themselves: their school \
         or work, their family, their day, what they like. Then the past: something they \
         did yesterday or last weekend. Then an opinion and why: what they think about \
         something, and their reason. Then something longer or imagined: describe a place \
         in detail, or what they would change if they could. When they answer easily, in \
         whole sentences, go one rung up. When they struggle or answer in a word or two, \
         stay on the same rung or step down with a simpler question. Keep your own words \
         simple enough for them to follow.\n\n\
         Never correct them, never comment on their English, and never mention tests, \
         levels, CEFR, prompts, or that you are an AI. If they use a Hindi word, \
         understand it and carry on in English. If you cannot make sense of what they \
         said, say so kindly and ask the same thing in easier words. Do not use markdown \
         or emoji.\n\n\
         If they ask for something no one would ask in a friendly first chat — write \
         code, solve a puzzle, answer a trivia question, tell a joke, sing a song — do \
         not do it, however small or harmless it seems. Say only, plainly, that today you \
         would like to hear about them, then ask your question again.\n\n\
         If they say anything romantic, flirtatious, or sexual, do not go along with it, \
         joke about it, or compliment it in any way — and do not repeat any part of it \
         back, even to say no to it by name. Say only, plainly and kindly and without \
         naming what they asked for, that this is not something you talk about, then ask \
         your question again. This covers far more than obviously explicit words, and \
         every form of a word: a romantic invitation (\"will you go out with me\", \"be my \
         girlfriend\"), a request for physical affection (\"hug me\", \"kiss me\"), a \
         request to be alone together at night, a declaration or proposal (\"I love \
         you\", \"marry me\"), anything sexual however it is worded, and a compliment \
         about your own looks (\"you are beautiful\", \"you're cute\"). Treat a new way \
         of saying one of these the same as the examples themselves."
    )
}

/// The placement's last reply, once `placement_ends` says the chat is over.
/// The session closes behind it, so a question here could never be answered.
const PLACEMENT_CLOSING_NOTE: &str = "This is your last reply: the chat ends here. Say one \
     specific thing you enjoyed hearing about, thank them warmly, and do not ask another \
     question.";

/// Asked after the placement's latest exchange, as a question aside rather
/// than a turn of the chat.
///
/// It goes after the whole conversation under the chat's own system prompt, so
/// llama.cpp answers it from the slot's cached prefix and evaluates little but
/// this note: a few hundred milliseconds on a laptop, where a prompt of its own
/// would evaluate the whole chat again. The next turn's prompt shares the same
/// prefix, so the aside costs it nothing either.
const PLACEMENT_CHECK_NOTE: &str = "Stop being Ella for a moment and do not reply to them. \
     Judge only what the learner has said so far in this chat: have you heard enough of \
     their English, on more than one kind of question, to tell how well they speak? \
     Answer with JSON only: {\"ready\": true or false, \"confidence\": \"low\", \"medium\" \
     or \"high\"}. confidence is how sure you would be of their level if you had to judge \
     it now.";

/// Reads a level off a finished placement chat. The levels are described as
/// Ella Docs' placement guide describes them, as on the phone.
///
/// Two changes from the phone's prompt, both measured against Ella's 3B model
/// on four transcripts from single words to fluent. The phone's reply template
/// showed `"level":"A2"`, and the 3B model copied it: every transcript came
/// back A2. With a placeholder there instead, and only the learner's answers
/// to read, the four came back A1, A2, B2 and B2 — in order, and at most one
/// level out.
const PLACEMENT_ASSESSOR_PROMPT: &str = "You assess spoken English for a learning app in \
     India.\n\n\
     Read the learner's answers and estimate their speaking level on the CEFR scale, from \
     A0 to C1:\n\
     - A0: single words only, no phrases yet.\n\
     - A1: short phrases, mostly present tense, very few words.\n\
     - A2: simple sentences about everyday things, no extended opinions.\n\
     - B1: connected sentences with because and but, opinions with reasons, past tense \
     mostly right.\n\
     - B2: fluent on familiar topics, varied tenses, argues with reasons.\n\
     - C1: articulate and nuanced, handles abstract topics, near-natural flow.\n\n\
     Weigh range of vocabulary, grammatical control, fluency and how well they developed \
     their answers. Ignore transcription noise and accent. Indian English is not a \
     mistake: \"I am having two sisters\", \"my good name is\", \"I am from Kanpur only\". \
     Mixing in Hindi is fine. When the sample is very short, do not guess high.\n\n\
     Reply with JSON:\n\
     {\"level\":\"<one of A0, A1, A2, B1, B2, C1>\",\"closing\":\"...\"}\n\
     - level: the description above that fits their answers best.\n\
     - closing: one warm sentence ending the talk, addressed to the learner by name. No \
     more than 14 words.";

/// The learner's answers alone, numbered, for the placement's assessor.
fn answers_of(messages: &[Message]) -> String {
    messages
        .iter()
        .filter(|message| message.speaker == Speaker::Learner)
        .enumerate()
        .map(|(index, message)| {
            format!("{}. {}", index + 1, message.content.split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Checks a finished talk against the skills in play. The skills are numbered
/// rather than keyed, because a 3B model copies `2` back far more reliably than
/// `A1:U2-GRA-01`.
///
/// Every skill it claims comes with the learner's own words, which
/// `read_scores` then looks for in what they said. Asked only for scores, as
/// the phone asks its much larger model, Ella's 3B model marked six skills out
/// of eight at 0.9 for a talk of "yes", "samosa" and "good. I like". Asked for
/// quotes, it claimed nothing for that talk, and the right three for a story
/// told with "used to", "was batting when" and "however … although".
fn score_prompt(skills: &[Scorable]) -> String {
    let listed = skills
        .iter()
        .enumerate()
        .map(|(index, skill)| format!("- {}: {}", index + 1, skill.text))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You check a spoken-English practice talk for a learning app in India.\n\n\
         The learner is working on these skills:\n{listed}\n\n\
         Read only the learner's words, never Ella's. A skill counts only when the \
         learner's own words clearly show it. For each skill that does, copy the exact \
         words from one learner line that show it, and say how sure you are, from 0 to 1: \
         about 0.7 for one clear but simple example, 0.9 or more for confident, natural \
         use. Most talks show only one or two of these skills, and a short or simple talk \
         shows none. Leave out every skill you cannot quote. Ignore transcription noise \
         and accent. Indian English is not a mistake: \"I am having two sisters\", \"I am \
         from Kanpur only\".\n\n\
         Reply with JSON:\n\
         {{\"shown\":[{{\"skill\":<number>,\"quote\":\"<the learner's exact words>\",\"sure\":<0 to 1>}}]}}\n\
         Use {{\"shown\":[]}} when the learner showed none of them."
    )
}

/// Corrects the learner's answers for the recap's one fix. The model rewrites
/// every answer with its mistakes fixed, and `notes::fix_from` compares the
/// two, so the phrase the recap quotes is always the learner's own.
///
/// Measured against Ella's 3B model on nine sample talks. Asked for the one
/// mistake and its correction, it "fixed" "600 is too much" into "600 is too
/// high", and for a talk of "yes", "samosa" and "good. I like" corrected "yes"
/// into "Did you eat anything today?". Asked to rewrite each answer changing
/// as little as it can, it left the fluent and casual talks and a bargaining
/// talk word for word as they were, and found "it have", "we plays", "is best
/// player", "the teacher explain", "I go to market" and "where is bus stop".
/// It misses some ("she is very good batsman"), which is the side to err on:
/// a wrong fix is worse than none. The answer's length is pinned to the number
/// of answers by the schema; without that it sometimes wrote each answer
/// twice, as given and corrected.
const CORRECTION_PROMPT: &str = "You correct the grammar of a learner's spoken English for a \
     learning app in India.\n\n\
     For each of the learner's answers, write it again with only its grammar mistakes \
     fixed: wrong verb forms and tenses, missing or wrong small words like \"a\", \"the\", \
     \"is\" and \"to\", and word order. Change as few words as you can. Keep every answer \
     that is already correct exactly as it is, word for word. Do not change the meaning, \
     the style or the choice of words, and do not add anything. Ignore punctuation and \
     capital letters. Hindi words are fine.\n\n\
     Reply with JSON: {\"lines\":[\"<answer 1, corrected>\",\"<answer 2, corrected>\", ...]}, \
     one entry per answer, in order.";

/// The answers as the corrector reads them: numbered, one per line.
fn numbered(answers: &[&str]) -> String {
    answers
        .iter()
        .enumerate()
        .map(|(index, answer)| format!("{}. {}", index + 1, answer.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The corrected answers out of the corrector's JSON, one per answer asked
/// about; any other count means the answer was not read.
fn read_corrections(value: &Value, expected: usize) -> Option<Vec<String>> {
    let lines: Vec<String> = value
        .get("lines")?
        .as_array()?
        .iter()
        .map(|line| line.as_str().map(|line| line.trim().to_owned()))
        .collect::<Option<_>>()?;
    (lines.len() == expected).then_some(lines)
}

/// A session's messages as chat turns: Ella's are the model's own.
fn chat_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            json!({
                "role": if message.speaker == Speaker::Ella { "assistant" } else { "user" },
                "content": message.content,
            })
        })
        .collect()
}

/// A talk as the judges read it, one line per turn: a typed turn with a line
/// break in it must not read as a line of Ella's.
fn transcript_of(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| {
            let who = match message.speaker {
                Speaker::Ella => "Ella",
                Speaker::Learner => "Learner",
            };
            format!("{who}: {}", message.content.split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The JSON object in a model's answer. Constrained generation already asks
/// for bare JSON; a stray code fence or a sentence around it is forgiven.
fn json_object_in(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start < end)
        .then(|| serde_json::from_str::<Value>(&text[start..=end]).ok())
        .flatten()
        .filter(Value::is_object)
}

fn read_readiness(value: &Value) -> Option<Readiness> {
    let ready = value.get("ready")?.as_bool()?;
    let confidence = value
        .get("confidence")
        .and_then(Value::as_str)
        .and_then(Confidence::parse)
        .unwrap_or(Confidence::Low);
    Some(Readiness { ready, confidence })
}

/// The level and closing line out of the assessor's JSON. A level the
/// curriculum does not have means the answer was not read.
fn read_placement(value: &Value) -> Option<PlacementReading> {
    let level = value.get("level")?.as_str()?.trim().to_uppercase();
    if !crate::curriculum::is_level(&level) {
        return None;
    }
    let closing = value
        .get("closing")
        .and_then(Value::as_str)
        .map(|closing| closing.trim().trim_matches('"').trim().to_owned())
        .filter(|closing| {
            let words = closing.split_whitespace().count();
            (1..=20).contains(&words)
        });
    Some(PlacementReading { level, closing })
}

/// The judge's scores, by progress key, for the skills whose quote the learner
/// really said. `learner` is what they said, turn by turn.
///
/// A claim counts only when its quote is the learner's own words, as the
/// transcript and the quote compare, punctuation and case aside (see
/// `find_quote`). Each stretch of what they said counts once: a claim whose
/// quote overlaps one already counted is dropped, because a 3B model will
/// credit one sentence to every skill it half fits, while two skills shown in
/// different parts of one answer both count. A claim naming a skill it was not
/// shown, or without a score from 0 to 1, is dropped on its own. An answer
/// without the `shown` list at all is not read.
fn read_scores(value: &Value, skills: &[Scorable], learner: &[&str]) -> Option<HashMap<String, f64>> {
    let claims = value.get("shown")?.as_array()?;
    let said: Vec<String> = learner.iter().map(|line| comparable(line)).collect();
    let said: Vec<Vec<&str>> = said.iter().map(|line| line.split(' ').collect()).collect();
    let mut scores = HashMap::new();
    let mut counted: Vec<Span> = Vec::new();
    for claim in claims {
        let Some(skill) = claim
            .get("skill")
            .and_then(Value::as_u64)
            .and_then(|number| usize::try_from(number).ok()?.checked_sub(1))
            .and_then(|index| skills.get(index))
        else {
            continue;
        };
        let Some(sure) = claim.get("sure").and_then(Value::as_f64).filter(|sure| (0.0..=1.0).contains(sure)) else {
            continue;
        };
        let quote = comparable(claim.get("quote").and_then(Value::as_str).unwrap_or_default());
        let Some(span) = find_quote(&quote, &said) else {
            continue;
        };
        if counted.iter().any(|seen| seen.overlaps(&span)) {
            continue;
        }
        counted.push(span);
        let best = scores.get(&skill.key).copied().unwrap_or(0.0_f64);
        scores.insert(skill.key.clone(), best.max(sure));
    }
    Some(scores)
}

/// Words as a transcript and a quote of it can be compared: lower case, with
/// apostrophes dropped and any other punctuation read as a space.
fn comparable(text: &str) -> String {
    text.to_lowercase()
        .replace(['\'', '’', '‘'], "")
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where a quote sits in what the learner said: which answer, and which of its
/// words, as a half-open range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    line: usize,
    start: usize,
    end: usize,
}

impl Span {
    fn overlaps(&self, other: &Span) -> bool {
        self.line == other.line && self.start < other.end && other.start < self.end
    }
}

/// Where `quote` — already `comparable` — is in the learner's `said` answers,
/// as a run of their words; `None` when they never said it. A quote needs two
/// words at least. A model copying one often changes a pronoun at its edge ("he
/// has been selling" for "who has been selling"), so for a quote of four words
/// or more one word may go from either end, leaving three at least to match.
fn find_quote(quote: &str, said: &[Vec<&str>]) -> Option<Span> {
    let words: Vec<&str> = quote.split(' ').filter(|word| !word.is_empty()).collect();
    let mut runs: Vec<&[&str]> = vec![&words];
    if words.len() > 3 {
        runs.push(&words[1..]);
        runs.push(&words[..words.len() - 1]);
    }
    runs.into_iter().filter(|run| run.len() >= 2).find_map(|run| {
        said.iter().enumerate().find_map(|(line, answer)| {
            answer
                .windows(run.len())
                .position(|window| window == run)
                .map(|start| Span { line, start, end: start + run.len() })
        })
    })
}

/// The placement's goodbye without a question tacked on the end. The model is
/// told not to ask one; when it does anyway, the question goes, because
/// nobody will be there to answer it. `None` when nothing but a question was
/// said.
fn without_trailing_question(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.ends_with('?') {
        return Some(trimmed.to_owned());
    }
    let body = &trimmed[..trimmed.len() - 1];
    let cut = body.rfind(['.', '!'])?;
    let kept = trimmed[..=cut].trim();
    (!kept.is_empty()).then(|| kept.to_owned())
}

/// Demo mode's placement questions, climbing the same rungs the model is told
/// to climb, so the whole chat can be walked without one.
fn scripted_placement_question(answers: u32) -> &'static str {
    const QUESTIONS: [&str; 5] = [
        "That sounds nice. Who do you usually spend your day with?",
        "Lovely. What did you do last weekend?",
        "That sounds fun. What is your favourite food, and why do you like it?",
        "Good reason! If you could visit any place, where would you go?",
        "Wonderful. What would you like to do there first?",
    ];
    QUESTIONS[(answers.saturating_sub(1) as usize) % QUESTIONS.len()]
}

/// Function words that say nothing about what a question is *about*. The
/// interrogatives stay: "how long is the trip" and "where does the trip go"
/// differ by little else.
const QUESTION_FILLER: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "do", "does", "did", "you", "your",
    "yours", "i", "me", "my", "we", "us", "our", "it", "its", "that", "this", "to",
    "for", "of", "in", "on", "at", "and", "or", "but", "so", "then", "please", "would",
    "will", "shall", "can", "could", "should", "have", "has", "had", "be", "been",
    "about", "with", "from", "there", "here", "sir", "madam", "ok", "okay",
];

/// The question a reply ends on, if it ends on one. Ella's prompt asks for
/// exactly one question at the end, so this is the whole of what she asked.
pub(crate) fn trailing_question(reply: &str) -> Option<&str> {
    let end = reply.rfind('?')?;
    let start = reply[..end].rfind(['.', '!', '?']).map_or(0, |index| index + 1);
    Some(reply[start..=end].trim())
}

/// Content words of a question, deduplicated, for comparing two questions by
/// what they are about rather than by their wording.
fn question_words(question: &str) -> BTreeSet<String> {
    question
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty() && !QUESTION_FILLER.contains(word))
        .map(str::to_owned)
        .collect()
}

/// `Some(earlier)` when this reply closes on a question Ella has already asked.
///
/// The cab transcript is what this exists for: Ella asked "How long will the
/// trip take?" on four turns running while the learner objected twice, and
/// "never ask a question you have already asked" is already in the prompt and
/// did nothing. Compared as word sets, not strings, because the repeat comes
/// back reworded — "How long will the trip take?" then "How long will the trip
/// to the airport take?"
fn repeated_question(reply: &str, earlier: &[String]) -> Option<String> {
    let asking = question_words(trailing_question(reply)?);
    // One content word is too little to call: "Really?" is not a repeat.
    if asking.len() < 2 {
        return None;
    }
    earlier.iter().find(|previous| {
        let before = question_words(previous);
        if before.len() < 2 {
            return false;
        }
        let shared = asking.intersection(&before).count();
        // One question's words wholly inside the other's. This is the shape the
        // repeat actually takes: the street-food run asked "What did you think
        // of the chutney?", then "What did you think?", then "What did you
        // think of the home-cooked bhel puri?" — same question, three widths.
        // Jaccard alone put those at 0.67 and let them through.
        if shared == asking.len().min(before.len()) {
            return true;
        }
        let union = asking.len() + before.len() - shared;
        // Or three quarters in common, for a repeat that swaps a word rather
        // than adding one.
        union > 0 && shared * 4 >= union * 3
    }).cloned()
}

/// Case-insensitive search for an ASCII needle, returning a byte offset into
/// `haystack` itself. Searching an uppercased *copy* is unsafe here: characters
/// whose uppercase form is longer (`ß` -> `SS`) shift every later offset.
fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let hay = haystack.as_bytes();
    let nee = needle.as_bytes();
    if nee.is_empty() || hay.len() < nee.len() {
        return None;
    }
    (0..=hay.len() - nee.len()).find(|&start| {
        haystack.is_char_boundary(start)
            && hay[start..start + nee.len()]
                .iter()
                .zip(nee)
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

/// A signalled turn with no words in it is a silent turn for the learner. The
/// model often replies with just `[DEAL]`, so substitute the line authored with
/// the chore rather than shipping the silence.
fn voiced(text: String, signal: Option<TurnSignal>, spec: &LedgerSpec) -> String {
    if !text.is_empty() {
        return text;
    }
    match signal {
        Some(TurnSignal::Deal) => spec.acceptance.clone(),
        Some(TurnSignal::Walk) => spec.refusal.clone(),
        None => text,
    }
}

/// Pull `[DEAL]` / `[WALK]` out of a reply. Returns the cleaned text, so the
/// token never reaches a screen or the synthesiser.
fn take_signal(text: &str) -> (String, Option<TurnSignal>) {
    let mut signal = None;
    let mut cleaned = text.to_string();
    // Bracketed first, because that is what the prompt asks for. Then bare, as
    // a fallback: Qwen2.5-3B frequently writes `DEAL` without the brackets, and
    // an undetected agreement means a chore that can never be won.
    for (token, value) in [
        ("[DEAL]", TurnSignal::Deal),
        ("[WALK]", TurnSignal::Walk),
        ("DEAL", TurnSignal::Deal),
        ("WALK", TurnSignal::Walk),
    ] {
        if signal.is_some() {
            break;
        }
        let Some(index) = find_ascii_ci(&cleaned, token) else {
            continue;
        };
        // A bare token only counts as a signal when it stands alone as a word,
        // so "a good deal for you" is prose and "DEAL" is an agreement.
        if !token.starts_with('[') {
            let before_ok = cleaned[..index]
                .chars()
                .next_back()
                .map_or(true, |c| !c.is_alphanumeric());
            let after_ok = cleaned[index + token.len()..]
                .chars()
                .next()
                .map_or(true, |c| !c.is_alphanumeric());
            // A one-word reply of "Deal." is agreement in any casing — it is
            // the whole turn, not a word inside a sentence the way it is in
            // "a good deal for you". Told to say it in a sentence and then
            // write [DEAL], Qwen2.5-3B sometimes makes "Deal." the sentence
            // and drops the token, and an undetected agreement is a chore the
            // learner is told they lost after being told they won.
            let whole_reply =
                cleaned.trim().trim_end_matches(['.', ',', '!', ' ']).len() == token.len();
            let standalone = whole_reply
                || cleaned[index..index + token.len()]
                    .chars()
                    .all(|c| c.is_uppercase());
            if !(before_ok && after_ok && standalone) {
                continue;
            }
        }
        signal = Some(value);
        cleaned.replace_range(index..index + token.len(), "");
    }
    // Only tidy punctuation when a token was actually removed. Trimming
    // unconditionally stripped the full stop off every ordinary reply, which
    // also robs Piper of its sentence-final prosody.
    let cleaned = if signal.is_some() || cleaned.contains('[') {
        strip_bracket_tokens(&cleaned)
    } else {
        cleaned
    };
    let cleaned = if signal.is_some() {
        cleaned.trim().trim_end_matches(['.', ',', '!', ' ']).trim()
    } else {
        cleaned.trim()
    };
    (cleaned.to_string(), signal)
}

/// Told about `[DEAL]` and `[WALK]`, Qwen2.5-3B generalises the pattern and
/// invents `[GO]`, `[STOP]`, `[PAUSE]`. Anything bracketed is a control token
/// by construction, so none of it may reach a screen or the synthesiser.
fn strip_bracket_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        match rest[open..].find(']') {
            // A balanced pair is a control token: drop it.
            Some(close) => rest = &rest[open + close + 1..],
            // An unbalanced '[' is ordinary text. Keeping it verbatim matters:
            // treating it as an open token truncated the rest of the reply.
            None => {
                out.push_str(&rest[open..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    // Close only the gap a removal left, so newlines survive.
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

/// The figure the character named, scoped to the chore's unit.
///
/// Every integer in the reply is a candidate. One adjacent to a unit word wins
/// over one that is not, and among equals the last wins, because a reply that
/// recites the old figure before naming a new one names the new one last. With
/// no unit anywhere in the reply we return `None` rather than guess: a bare
/// number in "twenty years in this market" is not an offer.
fn extract_figure(text: &str, spec: &LedgerSpec) -> Option<i32> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let mut best: Option<(bool, usize, i32)> = None;
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        let mut digits = String::new();
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b',') {
            if bytes[index].is_ascii_digit() {
                digits.push(bytes[index] as char);
            }
            index += 1;
        }
        let Ok(value) = digits.parse::<i32>() else {
            continue;
        };
        let window_start = start.saturating_sub(12);
        let window_end = (index + 12).min(bytes.len());
        let window = &lower[window_start..window_end];
        let adjacent = mentions_alias(window, &spec.unit_aliases);
        let candidate = (adjacent, start, value);
        best = match best {
            // Unit-adjacent beats not; otherwise later beats earlier.
            Some((was_adjacent, _, _)) if was_adjacent && !adjacent => best,
            _ => Some(candidate),
        };
    }
    let (_, _, value) = best?;
    mentions_alias(&lower, &spec.unit_aliases).then_some(value)
}

/// The figure this turn settles on.
///
/// Normally the one the character named. But a character that agrees in words
/// alone — "alright, alright, take it at that price" — has agreed to the
/// figure *they* just named, and saying otherwise leaves the ledger holding
/// the character's own last offer while the screen says the deal is done. The
/// learner is then told they lost a chore they were just told they had won.
/// Their figure still has to be one this spec would accept, so agreeing to a
/// lowball still does not move the number.
fn settled_figure(
    reply_text: &str,
    learner_text: &str,
    signal: Option<TurnSignal>,
    spec: &LedgerSpec,
    current: i32,
) -> Option<i32> {
    if let Some(named) = extract_figure(reply_text, spec) {
        return Some(named);
    }
    if signal != Some(TurnSignal::Deal) {
        return None;
    }
    extract_figure(learner_text, spec)
        .or_else(|| spelled_figure(learner_text, spec))
        .filter(|value| spec.accepts(current, *value))
}

/// The figure a *learner* named, written out in words.
///
/// `extract_figure` reads digits, which is right for the character's side —
/// the model always writes "Rs 450". The learner's side arrives through speech
/// recognition, which writes "four hundred rupees", so digits alone would
/// never see it. Scoped to the chore's unit the same way, and to the range
/// these chores haggle over.
fn spelled_figure(text: &str, spec: &LedgerSpec) -> Option<i32> {
    let lower = text.to_lowercase();
    if !mentions_alias(&lower, &spec.unit_aliases) {
        return None;
    }
    let mut runs: Vec<i64> = Vec::new();
    let mut total = 0i64;
    let mut part = 0i64;
    let mut open = false;
    let close = |runs: &mut Vec<i64>, total: &mut i64, part: &mut i64, open: &mut bool| {
        if *open {
            runs.push(*total + *part);
        }
        *total = 0;
        *part = 0;
        *open = false;
    };
    for word in lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|word| !word.is_empty())
    {
        match number_word(word) {
            Some(NumberWord::Value(value)) => {
                part += value;
                open = true;
            }
            // "a hundred rupees" carries no count word, so a bare hundred or
            // thousand stands for one of them.
            Some(NumberWord::Hundred) => {
                part = part.max(1) * 100;
                open = true;
            }
            Some(NumberWord::Thousand) => {
                total += part.max(1) * 1_000;
                part = 0;
                open = true;
            }
            // "four hundred and fifty" is one figure, so "and" does not end a
            // run that has already started.
            None if word == "and" && open => {}
            None => close(&mut runs, &mut total, &mut part, &mut open),
        }
    }
    close(&mut runs, &mut total, &mut part, &mut open);
    // The last figure wins, the same way it does among digits: a learner who
    // recites the old price before naming theirs names theirs last.
    runs.into_iter().last().and_then(|value| i32::try_from(value).ok())
}

enum NumberWord {
    Value(i64),
    Hundred,
    Thousand,
}

fn number_word(word: &str) -> Option<NumberWord> {
    let value = match word {
        "hundred" => return Some(NumberWord::Hundred),
        "thousand" => return Some(NumberWord::Thousand),
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    };
    Some(NumberWord::Value(value))
}

/// Does `text` name one of these unit aliases as a *word*?
///
/// Plain substring matching is wrong here and fails quietly: `"rs"` is inside
/// `"years"`, so "I have sold here for 20 years" would read as an offer of 20
/// rupees. Alphanumeric aliases need a boundary on both sides; symbols like
/// `₹` do not have one to find.
fn mentions_alias(text: &str, aliases: &[String]) -> bool {
    aliases.iter().any(|alias| {
        if alias.is_empty() {
            return false;
        }
        if !alias.chars().all(|c| c.is_alphanumeric()) {
            return text.contains(alias.as_str());
        }
        text.match_indices(alias.as_str()).any(|(index, matched)| {
            let before_ok = text[..index]
                .chars()
                .next_back()
                .map_or(true, |c| !c.is_alphanumeric());
            let after_ok = text[index + matched.len()..]
                .chars()
                .next()
                .map_or(true, |c| !c.is_alphanumeric());
            before_ok && after_ok
        })
    })
}

pub trait TutorEngine: Send + Sync {
    fn status(&self) -> EngineStatus;

    /// A free conversation's first line. `pitch` is what every reply of the
    /// session is told about the learner, so the engine can warm the prompt
    /// those replies share.
    fn opening(&self, topic: &Topic, learner_name: &str, pitch: &Pitch) -> EllaResult<String>;

    /// The placement chat's first line. Authored, like every opening: the
    /// default is what demo mode says, and `LocalEngine` warms the chat's
    /// prompt with it as well.
    fn placement_opening(&self, learner_name: &str, _age: Option<u8>) -> EllaResult<String> {
        Ok(placement_opening_line(learner_name))
    }

    /// Whether there is a model to judge with. Without one — demo mode — the
    /// placement chat runs to its shortest length and talks are not scored.
    fn judges(&self) -> bool {
        false
    }

    /// Whether the placement chat has heard enough, asked after its latest
    /// exchange. `messages` is the chat so far, opening first. `None` without
    /// a judge.
    fn placement_readiness(
        &self,
        _learner_name: &str,
        _age: Option<u8>,
        _messages: &[Message],
    ) -> EllaResult<Option<Readiness>> {
        Ok(None)
    }

    /// The level a finished placement chat shows. `None` without a judge; an
    /// error when the judge's answer could not be read, so it can be asked
    /// again.
    fn place(&self, _learner_name: &str, _messages: &[Message]) -> EllaResult<Option<PlacementReading>> {
        Ok(None)
    }

    /// How sure the judge is that the learner showed each of `skills` in a
    /// finished talk, by progress key; a skill not shown is left out. `None`
    /// without a judge, which is not the same as nothing shown: nothing is
    /// recorded against the learner. An error when the judge's answer could
    /// not be read, so it can be asked again.
    fn score(
        &self,
        _skills: &[Scorable],
        _messages: &[Message],
    ) -> EllaResult<Option<HashMap<String, f64>>> {
        Ok(None)
    }

    /// The learner's `answers`, each with its grammar mistakes fixed and
    /// otherwise word for word, for the recap's one fix. `None` without a
    /// judge; an error when its answer could not be read.
    fn correct(&self, _answers: &[&str]) -> EllaResult<Option<Vec<String>>> {
        Ok(None)
    }

    /// Generate one reply.
    ///
    /// Its sentences are synthesized while the rest is still being written, so
    /// Piper's time overlaps the model's instead of following it, and held in
    /// `pending` until the caller has saved the turn. None of it reaches the
    /// learner before the whole text is settled. Releasing sentence by
    /// sentence as they were written got Ella talking sooner and was not worth
    /// it: the text arrived in pieces, which meant it could not be centred
    /// without jumping as each piece landed.
    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply>;

    /// The character's first line, in role. The default is an authored opener,
    /// which is what demo mode uses; `LocalEngine` generates it so the setting
    /// and the hidden brief are already in play on turn one.
    fn opening_in_chore(&self, context: &ChoreContext, _learner_name: &str) -> EllaResult<String> {
        Ok(fallback_opening(context))
    }
    fn uses_native_stt(&self) -> bool;
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription>;
    fn synthesize(&self, text: &str) -> EllaResult<SynthesizedAudio>;

    /// Speak a line that is already written, sentence by sentence through
    /// `speech`, so an authored opening reaches the learner the same way a
    /// generated reply does: first sentence audible while the rest is still
    /// being synthesized, and word timings for the highlight.
    ///
    /// The default synthesizes the whole line at once, which is what demo mode
    /// and a Piper-less install do.
    fn speak(
        &self,
        text: &str,
        _speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<SynthesizedAudio> {
        self.synthesize(text)
    }

    /// Release what this engine holds before the process exits. Nothing by
    /// default; see `DeferredEngine::shutdown` for why the packaged app needs it.
    fn shutdown(&self) {}
}

/// Where an installed build keeps the two halves of an engine tree. They are
/// separate because only one of them is writable: binaries are bundled beside
/// the executable, while weights are downloaded into app data after install.
/// A development checkout leaves both `None` and gets the old single tree.
#[derive(Debug, Clone, Default)]
pub struct EnginePaths {
    pub engine_root: Option<PathBuf>,
    pub models_root: Option<PathBuf>,
}

/// Which engine this launch will use, decided before one is built so the
/// caller can put a placeholder in front of a slow local start.
///
/// An installed build ships its engines beside the executable, and the learner
/// who double-clicked it has no way to set an environment variable — so a
/// bundled `bin/` means local. Demo remains the default everywhere else, which
/// keeps `npm run desktop:dev` exactly as it was.
pub fn resolved_mode(paths: &EnginePaths) -> String {
    let default_mode = if paths
        .engine_root
        .as_ref()
        .is_some_and(|root| root.join("bin").is_dir())
    {
        "local"
    } else {
        "demo"
    };
    env::var("ELLA_ENGINE_MODE")
        .unwrap_or_else(|_| default_mode.into())
        .to_lowercase()
}

pub fn engine_from_environment(paths: EnginePaths) -> Box<dyn TutorEngine> {
    match resolved_mode(&paths).as_str() {
        "local" => Box::new(LocalEngine::from_environment(paths)),
        _ => Box::new(DemoEngine),
    }
}

#[derive(Default)]
pub struct DemoEngine;

impl TutorEngine for DemoEngine {
    fn status(&self) -> EngineStatus {
        EngineStatus {
            mode: "demo".into(),
            label: "POC demo engines".into(),
            ready: true,
            components: vec![
                EngineComponent {
                    name: "Conversation".into(),
                    ready: true,
                    detail: "Deterministic Rust tutor".into(),
                },
                EngineComponent {
                    name: "Speech recognition".into(),
                    ready: true,
                    detail: "System recognition with typing fallback".into(),
                },
                EngineComponent {
                    name: "Ella's voice".into(),
                    ready: true,
                    detail: "System voice in demo mode".into(),
                },
            ],
        }
    }

    fn opening(&self, topic: &Topic, learner_name: &str, _pitch: &Pitch) -> EllaResult<String> {
        Ok(opening_for(&topic.id, learner_name))
    }

    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply> {
        let started = Instant::now();
        if let Some(brief) = &request.placement {
            let text = if brief.closing {
                PLACEMENT_WRAP.to_owned()
            } else {
                scripted_placement_question(request.turn).to_owned()
            };
            let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
            return Ok(GeneratedReply::plain(text, completion_ms, completion_ms));
        }
        let lead = request
            .learner_text
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        let text = if request.turn >= 3 {
            format!(
                "I enjoyed hearing that, especially “{lead}”. Before we finish, what feeling does this story give you?"
            )
        } else if request.topic_label == "Street food stories" {
            format!(
                "That sounds delicious! You said “{lead}”. Who would you like to share that meal with, and why?"
            )
        } else if request.topic_label == "A job interview" {
            "Good, that is a clear answer. What part of that work do you enjoy the most?".into()
        } else {
            "I can picture that! What happened next, and how did you feel?".into()
        };
        let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
        Ok(GeneratedReply::plain(text, completion_ms, completion_ms))
    }

    fn uses_native_stt(&self) -> bool {
        false
    }

    fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> EllaResult<Transcription> {
        Err(EllaError::Engine(
            "I captured your voice, but native speech recognition is not enabled in demo mode. Try typing or start local engine mode."
                .into(),
        ))
    }

    fn synthesize(&self, _text: &str) -> EllaResult<SynthesizedAudio> {
        Ok(SynthesizedAudio {
            audio: None,
            first_audio_ms: None,
            completion_ms: None,
            words: Vec::new(),
            phonemes: Vec::new(),
            segments: 0,
        })
    }
}

/// How many saved slots are kept: the topics talked about most recently, at
/// the learner's level. Each is 35-45 MB.
const SAVED_SLOTS_KEPT: usize = 4;

/// Slots llama-server has saved to disk: a topic's instructions, evaluated
/// once and kept, so that the next talk on it, even after a restart, only
/// evaluates the skill it aims at.
///
/// On a laptop's CPU a talk's whole instructions take 10-25 seconds to
/// evaluate, and the talk's first reply waits for them. A restore reads a
/// file instead.
struct SlotStore {
    dir: PathBuf,
    /// Which model and which build of the server the slots belong to, asked
    /// of the server on first use. A slot saved by another model or build is
    /// not this one's to restore, so it is part of every file's name.
    identity: OnceLock<Option<String>>,
    /// Set once the server has refused to save a slot: it was started without
    /// anywhere to keep them, and asking again every talk would not help.
    refused: AtomicBool,
}

impl SlotStore {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            identity: OnceLock::new(),
            refused: AtomicBool::new(false),
        }
    }

    fn identity(&self, client: &Client, root: &str) -> Option<&str> {
        self.identity
            .get_or_init(|| {
                let props: Value = client
                    .get(format!("{root}/props"))
                    .timeout(Duration::from_secs(5))
                    .send()
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .ok()?;
                let model = props["model_path"].as_str()?;
                // The path alone is not enough: a model update writes the new
                // weights to the same file.
                let file = fs::metadata(model).ok()?;
                let modified = file.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
                Some(format!(
                    "{}|{model}|{}|{modified}|{}",
                    props["build_info"].as_str().unwrap_or("unknown build"),
                    file.len(),
                    props["default_generation_settings"]["n_ctx"],
                ))
            })
            .as_deref()
    }

    fn file_name(identity: &str, kind: &str, prompt: &str) -> String {
        let digest = Sha256::new()
            .chain_update(identity)
            .chain_update([0])
            .chain_update(prompt)
            .finalize();
        let hex: String = digest[..8].iter().map(|byte| format!("{byte:02x}")).collect();
        format!("ella-{kind}-{hex}.bin")
    }

    /// Put `prefixes`, each the start of the next, at the head of the slot
    /// before a talk's warm-up evaluates the rest of its prompt: the longest
    /// one already saved is restored, and each longer one is evaluated from
    /// there and saved for next time.
    fn prime(&self, client: &Client, root: &str, slot: i32, prefixes: &[(&str, String)]) {
        if prefixes.is_empty() || self.refused.load(Ordering::Relaxed) {
            return;
        }
        let Some(identity) = self.identity(client, root) else {
            eprintln!("[LATENCY]     llm> no saved slots: the server did not say which model it runs");
            return;
        };
        let names: Vec<String> = prefixes
            .iter()
            .map(|(kind, prompt)| Self::file_name(identity, kind, prompt))
            .collect();
        let mut from = 0;
        for index in (0..prefixes.len()).rev() {
            let path = self.dir.join(&names[index]);
            if !path.is_file() {
                continue;
            }
            let started = Instant::now();
            match slot_action(client, root, slot, "restore", &names[index]) {
                Ok(body) => {
                    eprintln!(
                        "[LATENCY]     llm> restored {} ({} tokens) in {:.0}ms",
                        names[index],
                        body["n_restored"],
                        started.elapsed().as_secs_f64() * 1_000.0
                    );
                    touch(&path);
                    from = index + 1;
                    break;
                }
                Err(error) => {
                    // Most likely saved by a server that could not agree on
                    // its layout. Evaluated again below, and saved afresh.
                    eprintln!("[LATENCY]     llm> could not restore {}: {error}", names[index]);
                    let _ = fs::remove_file(&path);
                }
            }
        }
        for index in from..prefixes.len() {
            let started = Instant::now();
            let evaluated = client
                .post(format!("{root}/v1/chat/completions"))
                .json(&json!({
                    "model": "local",
                    "messages": [{"role": "system", "content": prefixes[index].1}],
                    "max_tokens": 1,
                    "stream": false,
                    "cache_prompt": true,
                    "id_slot": slot,
                }))
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<Value>());
            let evaluated = match evaluated {
                Ok(body) => body["timings"]["prompt_n"].clone(),
                Err(error) => {
                    eprintln!("[LATENCY]     llm> could not evaluate the start of the prompt: {error}");
                    return;
                }
            };
            match slot_action(client, root, slot, "save", &names[index]) {
                Ok(body) => eprintln!(
                    "[LATENCY]     llm> evaluated {evaluated} tokens and saved {} ({} tokens) in {:.0}ms",
                    names[index],
                    body["n_saved"],
                    started.elapsed().as_secs_f64() * 1_000.0
                ),
                Err(error) => {
                    eprintln!(
                        "[LATENCY]     llm> the server saves no slots, so every talk evaluates its whole prompt: {error}"
                    );
                    self.refused.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
        self.prune(&names);
    }

    /// Keeps the files just used, and the most recently used of the others,
    /// up to `SAVED_SLOTS_KEPT` in all.
    fn prune(&self, current: &[String]) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut others: Vec<(SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("ella-") && name.ends_with(".bin") && !current.contains(&name)
            })
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        others.sort_by(|left, right| right.0.cmp(&left.0));
        for (_, path) in others
            .into_iter()
            .skip(SAVED_SLOTS_KEPT.saturating_sub(current.len()))
        {
            let _ = fs::remove_file(path);
        }
    }
}

/// How many talk warm-ups are still priming and warming the slot. A reply
/// waits for them here rather than slip in between their requests: the server
/// would take it next, and a slot saved after it would hold the conversation
/// as well as the topic's words. It would wait behind them in the server's
/// queue anyway, so waiting here costs it nothing.
#[derive(Default)]
struct WarmUps {
    running: Mutex<usize>,
    finished: Condvar,
}

impl WarmUps {
    fn begin(self: &Arc<Self>) -> WarmUpRunning {
        *self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        WarmUpRunning(Arc::clone(self))
    }

    fn wait(&self) {
        let mut running = self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while *running > 0 {
            running = self
                .finished
                .wait(running)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

/// One warm-up in progress; finished when dropped, however its thread ends.
struct WarmUpRunning(Arc<WarmUps>);

impl Drop for WarmUpRunning {
    fn drop(&mut self) {
        let mut running = self.0.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *running = running.saturating_sub(1);
        self.0.finished.notify_all();
    }
}

/// `POST /slots/{slot}?action=save|restore` for one file in the server's
/// slot directory.
fn slot_action(client: &Client, root: &str, slot: i32, action: &str, filename: &str) -> Result<Value, String> {
    let response = client
        .post(format!("{root}/slots/{slot}?action={action}"))
        .json(&json!({ "filename": filename }))
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body = response.json::<Value>().unwrap_or(Value::Null);
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!(
            "{status} {}",
            body["error"]["message"].as_str().unwrap_or_default()
        ))
    }
}

/// Marks a saved slot as just used, which is what pruning keeps.
fn touch(path: &Path) {
    if let Ok(file) = fs::File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

pub struct LocalEngine {
    client: Client,
    llm_base_url: String,
    llm_slot: i32,
    stt: SttRouter,
    piper_binary: PathBuf,
    piper_voice: PathBuf,
    piper_daemon: Option<Arc<PiperDaemon>>,
    /// Held for its lifetime, not read: dropping this kills the server. An
    /// installed build owns it; a development run leaves it `None` because a
    /// terminal already has one running.
    _llama: Option<LlamaServer>,
    /// Why the server this build should have started did not, in its own
    /// words, for the setup screen's "Try again" to show beside the button.
    llama_error: Option<String>,
    /// Where the start of each free talk's instructions is kept between
    /// talks. `None` without anywhere the server saves slots.
    slots: Option<Arc<SlotStore>>,
    warm_ups: Arc<WarmUps>,
}

impl LocalEngine {
    pub fn from_environment(paths: EnginePaths) -> Self {
        let engine_root = resolve_engine_root(paths.engine_root);
        let models_root = resolve_models_root(&engine_root, paths.models_root);
        let canary_path = env::var("ELLA_CANARY_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|_| models_root.join("stt").join(CANARY_FILE_NAME));
        let canary_threads = env_i32("ELLA_STT_THREADS", 0);
        let verify_checksum = env::var("ELLA_CANARY_VERIFY_SHA256")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let canary = || {
            Box::new(CanaryStt::new(
                canary_path.clone(),
                canary_threads,
                verify_checksum,
            )) as Box<dyn SpeechToTextEngine>
        };
        let windows_stt = || Box::new(WindowsStt::default()) as Box<dyn SpeechToTextEngine>;

        // Canary is primary everywhere. On Windows, SAPI is the fallback;
        // elsewhere there is none. `ELLA_STT_ENGINE=windows` swaps the
        // Windows order back to SAPI first.
        let stt_engine = env::var("ELLA_STT_ENGINE").unwrap_or_else(|_| "canary".into());

        let stt = if stt_engine.eq_ignore_ascii_case("windows") {
            SttRouter::new(windows_stt(), Some(canary()))
        } else if cfg!(target_os = "windows") {
            SttRouter::new(canary(), Some(windows_stt()))
        } else {
            SttRouter::new(canary(), None)
        };

        let piper_binary = env::var("ELLA_PIPER_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_piper_binary(&engine_root));
        let piper_voice = env::var("ELLA_PIPER_VOICE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_piper_voice(&engine_root, &models_root));
        let daemon_enabled = env::var("ELLA_PIPER_DAEMON")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let piper_daemon = daemon_enabled
            .then(|| resident_piper(&piper_binary, &piper_voice))
            .flatten();
        if let Some(daemon) = &piper_daemon {
            daemon.warm();
        }

        // An explicit URL means someone is running their own server — the
        // development scripts, the benchmarks — and Ella must not start a
        // second one against the same model.
        let (llama, llm_base_url, llama_error) = match env::var("ELLA_LLM_BASE_URL") {
            Ok(configured) => (None, configured, None),
            Err(_) => match LlamaServer::start(&engine_root, &models_root, {
                let threads = env_i32("ELLA_LLAMA_THREADS", default_llama_threads());
                eprintln!("[engines] llama-server gets {threads} threads");
                threads
            }) {
                Ok(server) => {
                    let url = server.base_url().to_string();
                    (Some(server), url, None)
                }
                Err(reason) => {
                    // Not fatal: the window still opens and `status()` reports
                    // the language model as unavailable, which is far more use
                    // to a remote tester than a process that refuses to start.
                    eprintln!("[engines] llama-server did not start: {reason}");
                    (None, "http://127.0.0.1:39091/v1".to_string(), Some(reason.to_string()))
                }
            },
        };

        // The server Ella started keeps its slots beside the weights. Someone
        // else's says where it keeps them through ELLA_LLM_SLOT_DIR, which
        // has to match the --slot-save-path it was started with.
        let slot_dir = llama
            .as_ref()
            .and_then(|server| server.slot_dir().map(Path::to_path_buf))
            .or_else(|| env::var_os("ELLA_LLM_SLOT_DIR").map(PathBuf::from));

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client configuration is valid"),
            llm_base_url,
            llm_slot: env_i32("ELLA_LLM_SLOT", 0),
            stt,
            piper_binary,
            piper_voice,
            piper_daemon,
            _llama: llama,
            llama_error,
            slots: slot_dir.map(|dir| Arc::new(SlotStore::new(dir))),
            warm_ups: Arc::default(),
        }
    }

    fn probe(&self, base_url: &str) -> bool {
        let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
        self.client
            .get(format!("{root}/health"))
            .timeout(Duration::from_secs(2))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }
}

impl TutorEngine for LocalEngine {
    fn status(&self) -> EngineStatus {
        let llm_ready = self.probe(&self.llm_base_url);
        let (primary_stt, fallback_stt) = self.stt.status();
        let stt_ready = primary_stt.ready
            || fallback_stt
                .as_ref()
                .map(|status| status.ready)
                .unwrap_or(false);
        let voice_sidecar = PathBuf::from(format!("{}.json", self.piper_voice.display()));
        let tts_ready =
            self.piper_binary.is_file() && self.piper_voice.is_file() && voice_sidecar.is_file();
        let mut components = vec![
            EngineComponent {
                name: "Language model".into(),
                ready: llm_ready,
                detail: if llm_ready {
                    format!("Streaming response telemetry at {}", self.llm_base_url)
                } else if let Some(reason) = &self.llama_error {
                    reason.clone()
                } else {
                    format!(
                        "Not reachable at {}. Start it with `npm run engines:local`.",
                        self.llm_base_url
                    )
                },
            },
            EngineComponent {
                name: "Speech recognition (primary)".into(),
                ready: primary_stt.ready,
                detail: primary_stt.detail,
            },
        ];
        if let Some(fallback) = fallback_stt {
            components.push(EngineComponent {
                name: "Speech recognition (fallback)".into(),
                ready: fallback.ready,
                detail: fallback.detail,
            });
        }
        components.push(EngineComponent {
            name: "Ella's voice".into(),
            ready: tts_ready,
            detail: if tts_ready {
                self.piper_voice.display().to_string()
            } else {
                format!(
                    "Piper is incomplete. Expected binary {} plus voice {} and its .json sidecar.",
                    self.piper_binary.display(),
                    self.piper_voice.display()
                )
            },
        });
        EngineStatus {
            mode: "local".into(),
            label: "Local AI — Canary primary".into(),
            ready: llm_ready && stt_ready,
            components,
        }
    }

    fn opening(&self, topic: &Topic, learner_name: &str, pitch: &Pitch) -> EllaResult<String> {
        let text = opening_for(&topic.id, learner_name);
        self.warm_prompt_cache(
            ella_system_prompt(learner_name, &topic.id, &topic.label, pitch),
            text.clone(),
            vec![("topic", ella_topic_prompt(learner_name, &topic.id, &topic.label, &pitch.level))],
        );
        Ok(text)
    }

    fn placement_opening(&self, learner_name: &str, age: Option<u8>) -> EllaResult<String> {
        let text = placement_opening_line(learner_name);
        self.warm_prompt_cache(placement_system_prompt(learner_name, age), text.clone(), Vec::new());
        Ok(text)
    }

    fn judges(&self) -> bool {
        true
    }

    fn placement_readiness(
        &self,
        learner_name: &str,
        age: Option<u8>,
        messages: &[Message],
    ) -> EllaResult<Option<Readiness>> {
        // The chat's own prompt and history, then the question aside: see
        // `PLACEMENT_CHECK_NOTE` for why it is asked this way.
        let mut request = vec![json!({"role": "system", "content": placement_system_prompt(learner_name, age)})];
        request.extend(chat_messages(messages));
        request.push(json!({"role": "system", "content": PLACEMENT_CHECK_NOTE}));
        self.judge("check whether the placement has heard enough", request, 30, None, read_readiness)
            .map(Some)
    }

    fn place(&self, learner_name: &str, messages: &[Message]) -> EllaResult<Option<PlacementReading>> {
        let request = vec![
            json!({"role": "system", "content": PLACEMENT_ASSESSOR_PROMPT}),
            json!({
                "role": "user",
                "content": format!(
                    "Learner's name: {learner_name}\n\nThe learner's answers, in order:\n{}",
                    answers_of(messages)
                ),
            }),
        ];
        self.judge("read a level off the placement", request, 80, None, read_placement)
            .map(Some)
    }

    fn score(
        &self,
        skills: &[Scorable],
        messages: &[Message],
    ) -> EllaResult<Option<HashMap<String, f64>>> {
        if skills.is_empty() {
            return Ok(None);
        }
        let request = vec![
            json!({"role": "system", "content": score_prompt(skills)}),
            json!({"role": "user", "content": format!("Conversation:\n{}", transcript_of(messages))}),
        ];
        let learner: Vec<&str> = messages
            .iter()
            .filter(|message| message.speaker == Speaker::Learner)
            .map(|message| message.content.as_str())
            .collect();
        // A quote per claim makes the answer longer than a bare score did.
        self.judge("score the talk", request, 320, None, |value| read_scores(value, skills, &learner))
            .map(Some)
    }

    fn correct(&self, answers: &[&str]) -> EllaResult<Option<Vec<String>>> {
        if answers.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let request = vec![
            json!({"role": "system", "content": CORRECTION_PROMPT}),
            json!({"role": "user", "content": format!("The learner's answers, in order:\n{}", numbered(answers))}),
        ];
        let schema = json!({
            "type": "object",
            "properties": {
                "lines": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": answers.len(),
                    "maxItems": answers.len(),
                },
            },
            "required": ["lines"],
        });
        // The answer is the answers again, so its budget grows with them.
        let words: usize = answers.iter().map(|answer| answer.split_whitespace().count()).sum();
        let budget = (words * 2 + 12 * answers.len() + 24) as u32;
        self.judge("correct the answers", request, budget, Some(schema), |value| {
            read_corrections(value, answers.len())
        })
        .map(Some)
    }

    fn opening_in_chore(&self, context: &ChoreContext, learner_name: &str) -> EllaResult<String> {
        let text = chore_opening_for(&context.chore_id)
            .map(str::to_owned)
            .unwrap_or_else(|| fallback_opening(context));
        self.warm_prompt_cache(chore_system_prompt(learner_name, context), text.clone(), Vec::new());
        Ok(text)
    }

    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply> {
        let system = match (&request.placement, &request.chore) {
            (Some(brief), _) => placement_system_prompt(&request.learner_name, brief.age),
            (None, Some(context)) => chore_system_prompt(&request.learner_name, context),
            (None, None) => ella_system_prompt(
                &request.learner_name,
                &request.topic_id,
                &request.topic_label,
                &request.pitch,
            ),
        };
        let mut history = self.history_messages(request);
        // Everything that moves between turns goes here, after the whole
        // cached conversation, so the prefix llama.cpp has already evaluated
        // stays byte-identical from the first turn to the last.
        let turn_note = match (&request.placement, &request.chore) {
            (Some(brief), _) => brief.closing.then(|| PLACEMENT_CLOSING_NOTE.to_owned()),
            (None, Some(context)) => chore_turn_message(context, request.turn),
            (None, None) => free_closing_note(request.turn),
        };
        if let Some(note) = turn_note {
            history.insert(
                history.len().saturating_sub(1),
                json!({"role": "system", "content": note}),
            );
        }

        // Sentences are synthesized as the model writes them but held back:
        // Piper's time overlaps the model's, and none of it is heard until the
        // reply is settled and the caller has saved the turn. Then the
        // sentences already synthesized are released at once and the rest as
        // they are ready, so the text and the voice still arrive together,
        // and a ledger turn that has to be generated again is no different
        // from any other here.
        //
        // Needs a resident Piper; one-shot Piper loads the voice per call,
        // which would cost more per sentence than the overlap saves.
        let generation_started = Instant::now();
        let mut pipeline = self
            .piper_daemon
            .as_ref()
            .filter(|daemon| daemon.usable())
            .map(|daemon| SpeechPipeline::start(Arc::clone(daemon), None, generation_started));

        let first = self.stream_once(&system, &history, None, pipeline.as_mut())?;
        // Only the first generation feeds the speaker; a regenerated reply is
        // said from scratch by the caller. Whatever is still being synthesized
        // carries on while the reply is checked.
        if let Some(pipeline) = pipeline.as_mut() {
            pipeline.close();
        }
        let (mut text, signal) = take_signal(&first.text);
        if request.placement.as_ref().is_some_and(|brief| brief.closing) {
            text = without_trailing_question(&text).unwrap_or_else(|| PLACEMENT_WRAP.to_owned());
        }

        // Free conversation: no ledger to enforce, but a question Ella has
        // already asked is its own failure. The cab transcript locked up on one
        // for four turns, so it is caught here and asked again rather than
        // spoken.
        let Some(ledger) = request.chore.as_ref().and_then(|c| c.ledger.as_ref()) else {
            let asked_before: Vec<String> = request
                .messages
                .iter()
                .filter(|message| message.speaker == Speaker::Ella)
                .filter_map(|message| trailing_question(&message.content).map(str::to_owned))
                .collect();
            if let Some(already) = repeated_question(&text, &asked_before) {
                eprintln!("[LATENCY]     llm> repeated question: {already:?} - regenerating once");
                let corrective = format!(
                    "You have already asked them this earlier in the conversation: \"{already}\" \
                     They have answered it, or told you they cannot. Do not ask it again in any \
                     words. Take what they just said, say something back about it, and ask about \
                     one new thing they have not told you yet."
                );
                // Piper stops reading the first attempt rather than compete with
                // the model's second one. In the rare case that the second
                // comes back empty, the caller says the first one itself.
                drop(pipeline.take());
                let second = self.stream_once(&system, &history, Some(&corrective), None)?;
                let (retry_text, _) = take_signal(&second.text);
                // Once only. A second repeat is left alone: another generation
                // costs the learner more silence than the repeat costs them.
                if !retry_text.is_empty() {
                    return Ok(GeneratedReply {
                        text: retry_text,
                        ttft_ms: first.ttft_ms,
                        completion_ms: first.completion_ms + second.completion_ms,
                        named_figure: None,
                        signal: None,
                        regenerated: true,
                        pending: None,
                    });
                }
            }
            return Ok(GeneratedReply {
                pending: PendingSpeech::matching(pipeline, &text),
                text,
                ttft_ms: first.ttft_ms,
                completion_ms: first.completion_ms,
                named_figure: None,
                signal,
                regenerated: false,
            });
        };

        let figure = settled_figure(
            &text,
            &request.learner_text,
            signal,
            &ledger.spec,
            ledger.current,
        );
        let legal = figure.map_or(true, |value| ledger.spec.accepts(ledger.current, value));
        if legal {
            let final_text = voiced(text, signal, &ledger.spec);
            // The figure held, so the audio synthesized ahead is the audio for
            // the reply being returned — unless `voiced` substituted the
            // authored line for a wordless turn, which `matching` catches.
            return Ok(GeneratedReply {
                pending: PendingSpeech::matching(pipeline, &final_text),
                text: final_text,
                named_figure: figure,
                signal,
                regenerated: false,
                ttft_ms: first.ttft_ms,
                completion_ms: first.completion_ms,
            });
        }
        drop(pipeline);

        // The character named a figure past its own limit, or conceded more in
        // one move than the chore allows. Regenerating costs one extra
        // generation on a rare turn; the alternative is the app clamping its
        // state while the reply on screen names a number the bar disagrees
        // with, which reads as a bug to the learner.
        let broken = figure.unwrap_or_default();
        eprintln!(
            "[LATENCY]     llm> ledger break: named {} {broken}, current {} (limit {}, max step {}) - regenerating once",
            ledger.spec.unit, ledger.current, ledger.spec.limit, ledger.spec.max_step
        );
        let corrective = format!(
            "That reply broke your own position: you named {} {broken}. The figure must \
             stay on your side of {} {} and move by at most {} {} from {} {}. Write the \
             reply again, in character, with a figure that obeys that.",
            ledger.spec.unit,
            ledger.spec.unit,
            ledger.spec.limit,
            ledger.spec.unit,
            ledger.spec.max_step,
            ledger.spec.unit,
            ledger.current,
        );
        let second = self.stream_once(&system, &history, Some(&corrective), None)?;
        let (retry_text, retry_signal) = take_signal(&second.text);
        let retry_figure = settled_figure(
            &retry_text,
            &request.learner_text,
            retry_signal,
            &ledger.spec,
            ledger.current,
        );
        let retry_legal = retry_figure.map_or(true, |value| ledger.spec.accepts(ledger.current, value));
        if retry_legal {
            return Ok(GeneratedReply {
                text: voiced(retry_text, retry_signal, &ledger.spec),
                named_figure: retry_figure,
                signal: retry_signal,
                regenerated: true,
                ttft_ms: first.ttft_ms,
                completion_ms: first.completion_ms + second.completion_ms,
                pending: None,
            });
        }

        // Broke it twice. Use the line authored with the chore, which is what
        // guarantees a chore stays unwinnable by cheese even when the model
        // digs in. The figure does not move.
        eprintln!("[LATENCY]     llm> ledger break twice - falling back to the authored refusal");
        Ok(GeneratedReply {
            text: ledger.spec.refusal.clone(),
            named_figure: None,
            signal: None,
            regenerated: true,
            ttft_ms: first.ttft_ms,
            completion_ms: first.completion_ms + second.completion_ms,
            pending: None,
        })
    }
    fn uses_native_stt(&self) -> bool {
        true
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        self.stt.transcribe(samples, sample_rate)
    }

    fn speak(
        &self,
        text: &str,
        speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<SynthesizedAudio> {
        self.speak_in_sentences(text, speech)
    }

    fn synthesize(&self, text: &str) -> EllaResult<SynthesizedAudio> {
        let voice_sidecar = PathBuf::from(format!("{}.json", self.piper_voice.display()));
        if !self.piper_binary.is_file() || !self.piper_voice.is_file() || !voice_sidecar.is_file() {
            return Ok(SynthesizedAudio {
                audio: None,
                first_audio_ms: None,
                completion_ms: None,
                words: Vec::new(),
                phonemes: Vec::new(),
                segments: 0,
            });
        }
        if let Some(daemon) = self.piper_daemon.as_ref().filter(|daemon| daemon.usable()) {
            eprintln!(
                "[LATENCY]     tts> asking resident Piper for {} chars of text",
                text.chars().count()
            );
            match daemon.synthesize(text) {
                Ok(speech) => {
                    let words = word_spans(text, &speech.pcm, speech.sample_rate);
                    let encode_started = Instant::now();
                    let base64 = STANDARD.encode(raw_pcm_to_wav(&speech.pcm, speech.sample_rate, 1));
                    eprintln!(
                        "[LATENCY]     tts> wav+base64 encode took {:.1}ms ({} chars)",
                        encode_started.elapsed().as_secs_f64() * 1_000.0,
                        base64.len()
                    );
                    return Ok(SynthesizedAudio {
                        audio: Some(AudioPayload {
                            mime_type: "audio/wav".into(),
                            base64,
                        }),
                        first_audio_ms: Some(speech.first_audio_ms),
                        completion_ms: Some(speech.completion_ms),
                        words,
                        phonemes: phoneme_spans(&speech.alignment, speech.sample_rate, 0),
                        segments: 0,
                    });
                }
                Err(error) => eprintln!(
                    "[LATENCY]     tts> resident Piper failed ({error}); falling back to one-shot Piper"
                ),
            }
        }
        self.synthesize_oneshot(text)
    }
}

impl LocalEngine {
    /// Sentence-stream an already-written line. Same pipeline as a reply; the
    /// only difference is that the whole text arrives in one push instead of
    /// token by token, so every sentence is queued immediately and Piper is the
    /// only thing the learner waits on.
    fn speak_in_sentences(
        &self,
        text: &str,
        speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<SynthesizedAudio> {
        let Some(daemon) = self.piper_daemon.as_ref().filter(|daemon| daemon.usable()) else {
            return self.synthesize(text);
        };
        let mut pipeline = SpeechPipeline::start(Arc::clone(daemon), speech, Instant::now());
        pipeline.push(text);
        let streamed = pipeline.finish();
        // An authored line is never regenerated, so what was spoken is what the
        // screen shows; `resolve` still guards a pipeline that died partway.
        let (audio, played) = streamed.resolve(text);
        match audio {
            Some(audio) => Ok(SynthesizedAudio {
                segments: played,
                ..audio
            }),
            None => self.synthesize(text),
        }
    }

    /// Fire-and-forget llama.cpp prompt-cache warmup with this session's stable
    /// prefix, so turn 1 does not pay full prompt evaluation. Both kinds of
    /// session open on an authored line, so both can do this off the clock.
    ///
    /// `prefixes` are starts of `system` that other talks share, shortest
    /// first. Whichever are saved are restored before the warm-up, so it
    /// evaluates only what follows them: see `SlotStore`.
    fn warm_prompt_cache(&self, system: String, opening: String, prefixes: Vec<(&'static str, String)>) {
        let client = self.client.clone();
        let url = format!(
            "{}/chat/completions",
            self.llm_base_url.trim_end_matches('/')
        );
        let root = self
            .llm_base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_owned();
        let slot = self.llm_slot;
        let slots = self.slots.clone();
        // Counted before the thread starts, so a reply sent the moment the
        // talk opens still finds it running.
        let running = self.warm_ups.begin();
        thread::spawn(move || {
            let _running = running;
            let started = Instant::now();
            if let Some(slots) = &slots {
                slots.prime(&client, &root, slot, &prefixes);
            }
            // This request, not the first turn, is the one that swaps the slot
            // from the previous session's prompt to this one's. Whatever it
            // reuses is what survived the switch, so its counts are the ones
            // that say whether a topic change really evicts the old prefix.
            let fingerprint = prompt_fingerprint(&system);
            let result = client
                .post(url)
                .json(&json!({
                    "model": "local",
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "assistant", "content": opening},
                    ],
                    "max_tokens": 1,
                    "stream": false,
                    "cache_prompt": true,
                    "id_slot": slot,
                }))
                .send();
            match result {
                Ok(response) => {
                    let status = response.status();
                    let took_ms = started.elapsed().as_secs_f64() * 1_000.0;
                    eprintln!(
                        "[LATENCY]     llm> session prompt-cache warmup took {took_ms:.0}ms \
                         (status {status}) fp={fingerprint} slot={slot}"
                    );
                    let body = response.json::<Value>().unwrap_or(Value::Null);
                    match (
                        body["timings"]["prompt_n"].as_i64(),
                        body["usage"]["prompt_tokens"].as_i64(),
                    ) {
                        (Some(evaluated), Some(total)) => eprintln!(
                            "[LATENCY]     llm> warmup took the slot: evaluated {evaluated} of \
                             {total} prompt tokens, {} reused",
                            total - evaluated
                        ),
                        (Some(evaluated), None) => eprintln!(
                            "[LATENCY]     llm> warmup took the slot: evaluated {evaluated} prompt tokens"
                        ),
                        _ => eprintln!(
                            "[LATENCY]     llm> warmup took the slot: no token counts reported"
                        ),
                    }
                }
                Err(error) => {
                    eprintln!("[LATENCY]     llm> session prompt-cache warmup failed: {error}")
                }
            }
        });
    }

    /// Build the message array once per turn. Full history every time is
    /// deliberate: a sliding window changes the prompt prefix and defeats
    /// llama.cpp's prompt cache (TTFT 0.5s -> 3.5s).
    fn history_messages(&self, request: &TutorRequest) -> Vec<Value> {
        let mut messages = chat_messages(&request.messages);
        messages.push(json!({"role": "user", "content": request.learner_text}));
        messages
    }

    /// One JSON answer from the model, for the judges. Not streamed, because
    /// nothing of it is spoken, and `response_format` has llama.cpp constrain
    /// the output to JSON. `read` says whether the answer is usable; one that
    /// is not is asked for once more before giving up with an error, so the
    /// caller can ask again later rather than record a guess.
    ///
    /// Same slot as the conversation. The placement check shares the chat's
    /// prefix, so it reuses the slot's cache; the others run once a talk is
    /// over, when the next session re-warms the slot anyway.
    fn judge<T>(
        &self,
        label: &str,
        messages: Vec<Value>,
        max_tokens: u32,
        schema: Option<Value>,
        read: impl Fn(&Value) -> Option<T>,
    ) -> EllaResult<T> {
        // A schema has llama.cpp hold the answer to that shape, not just to
        // JSON of some kind.
        let response_format = match schema {
            Some(schema) => json!({"type": "json_object", "schema": schema}),
            None => json!({"type": "json_object"}),
        };
        let url = format!("{}/chat/completions", self.llm_base_url.trim_end_matches('/'));
        let mut failure = String::new();
        for attempt in 1..=2 {
            let started = Instant::now();
            let body = self
                .client
                .post(&url)
                .json(&json!({
                    "model": "local",
                    "messages": messages,
                    "temperature": 0.1,
                    "max_tokens": max_tokens,
                    "stream": false,
                    "cache_prompt": true,
                    "id_slot": self.llm_slot,
                    "response_format": response_format,
                }))
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<Value>());
            let took_ms = started.elapsed().as_secs_f64() * 1_000.0;
            let body = match body {
                Ok(body) => body,
                Err(error) => {
                    eprintln!("[LATENCY]     judge> {label}: attempt {attempt} failed after {took_ms:.0}ms: {error}");
                    failure = error.to_string();
                    continue;
                }
            };
            let content = body["choices"][0]["message"]["content"].as_str().unwrap_or_default();
            eprintln!(
                "[LATENCY]     judge> {label}: attempt {attempt} took {took_ms:.0}ms (evaluated {} prompt tokens): {}",
                body["timings"]["prompt_n"].as_i64().map_or_else(|| "?".into(), |n| n.to_string()),
                content.trim()
            );
            if let Some(read) = json_object_in(content).as_ref().and_then(&read) {
                return Ok(read);
            }
            failure = format!("an answer that could not be read: {}", content.trim());
        }
        Err(EllaError::Engine(format!("The model could not {label}: {failure}")))
    }

    /// One streamed generation. `corrective` is appended as a system turn when
    /// re-generating after a ledger break. `speech` receives token deltas as
    /// they land so whole sentences can be synthesized without waiting for the
    /// model to finish.
    fn stream_once(
        &self,
        system: &str,
        history: &[Value],
        corrective: Option<&str>,
        mut speech: Option<&mut SpeechPipeline>,
    ) -> EllaResult<GeneratedReply> {
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(history.iter().cloned());
        if let Some(corrective) = corrective {
            messages.push(json!({"role": "system", "content": corrective}));
        }

        eprintln!(
            "[LATENCY]     llm> POST {}/chat/completions ({} messages, stream=true)",
            self.llm_base_url.trim_end_matches('/'),
            messages.len()
        );
        // Every session shares one `id_slot`, so the prefix cached there is
        // whatever the last session left behind. The fingerprint is what says
        // which prompt this turn is actually being generated against: it must
        // hold steady for every turn of one session and must change the moment
        // the topic does.
        eprintln!(
            "[LATENCY]     llm> prompt fp={} len={} slot={} cache_prompt=true",
            prompt_fingerprint(system),
            system.len(),
            self.llm_slot,
        );
        eprintln!(
            "[LATENCY]     llm> scene: {}…",
            scene_clause(system).chars().take(80).collect::<String>().trim()
        );
        if env::var("ELLA_DEBUG_PROMPT").is_ok() {
            eprintln!("[PROMPT] ─── chained prompt, {} messages ───", messages.len());
            for (index, message) in messages.iter().enumerate() {
                eprintln!(
                    "[PROMPT] {index:>2} {:<9} {}",
                    message["role"].as_str().unwrap_or("?"),
                    message["content"].as_str().unwrap_or("")
                );
            }
            eprintln!("[PROMPT] ─── end of chained prompt ───");
        }
        let started = Instant::now();
        // Counted in the time to the first token, as it was when the reply
        // queued behind the warm-up in the server instead.
        self.warm_ups.wait();
        let waited_ms = started.elapsed().as_secs_f64() * 1_000.0;
        if waited_ms >= 1.0 {
            eprintln!("[LATENCY]     llm> waited {waited_ms:.0}ms for the talk's warm-up");
        }
        let response = self
            .client
            .post(format!(
                "{}/chat/completions",
                self.llm_base_url.trim_end_matches('/')
            ))
            .json(&json!({
                "model": "local",
                "messages": messages,
                "temperature": 0.65,
                "max_tokens": 50,
                // No presence/frequency penalty here on purpose. They look like
                // the obvious cure for the repeated sentence template, and they
                // make it worse: holding seed and prompt fixed, reuse of one
                // template went 2/6 without them to 6/6 at pp=0.4 fp=0.3. The
                // window cannot be widened past the default either — this
                // endpoint rejects `penalty_last_n` with a 400.
                "stream": true,
                // Asks for the token counts on the final chunk. They are what
                // turn "is the cache shared across topics?" into a number:
                // how many prompt tokens the server actually evaluated versus
                // how many it took from the slot without looking at them.
                "stream_options": {"include_usage": true},
                "cache_prompt": true,
                "id_slot": self.llm_slot
            }))
            .send()?
            .error_for_status()?;
        eprintln!(
            "[LATENCY]     llm> +{:.1}ms HTTP response headers received, reading SSE stream",
            started.elapsed().as_secs_f64() * 1_000.0
        );
        let mut text = String::new();
        let mut ttft_ms = None;
        let mut chunk_count: u32 = 0;
        let mut prompt_evaluated: Option<i64> = None;
        let mut prompt_total: Option<i64> = None;
        for line in BufReader::new(response).lines() {
            let line = line?;
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            let Ok(chunk) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            // llama.cpp reports these on the closing chunk. `prompt_n` counts
            // the prompt tokens it had to evaluate; anything below the total
            // came out of the slot's cache untouched.
            if let Some(evaluated) = chunk["timings"]["prompt_n"].as_i64() {
                prompt_evaluated = Some(evaluated);
            }
            if let Some(total) = chunk["usage"]["prompt_tokens"].as_i64() {
                prompt_total = Some(total);
            }
            if let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str() {
                if !delta.is_empty() {
                    chunk_count += 1;
                    if ttft_ms.is_none() {
                        let first_token = started.elapsed().as_secs_f64() * 1_000.0;
                        eprintln!(
                            "[LATENCY]     llm> +{first_token:.1}ms FIRST TOKEN (ttft): {delta:?}"
                        );
                        ttft_ms = Some(first_token);
                    }
                    text.push_str(delta);
                    if let Some(pipeline) = speech.as_deref_mut() {
                        pipeline.push(delta);
                    }
                }
            }
        }
        let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "[LATENCY]     llm> +{completion_ms:.1}ms stream complete ({chunk_count} chunks, {} chars)",
            text.trim().chars().count()
        );
        // The first turn after a topic switch is the one to read: the two
        // system prompts share only their opening preamble, so anything reused
        // beyond that length is the previous topic's prefix still in the slot.
        match (prompt_evaluated, prompt_total) {
            (Some(evaluated), Some(total)) => eprintln!(
                "[LATENCY]     llm> prompt cache: evaluated {evaluated} of {total} prompt tokens, {} reused from slot {}",
                total - evaluated,
                self.llm_slot
            ),
            (Some(evaluated), None) => eprintln!(
                "[LATENCY]     llm> prompt cache: evaluated {evaluated} prompt tokens on slot {}",
                self.llm_slot
            ),
            _ => eprintln!(
                "[LATENCY]     llm> prompt cache: this server reported no token counts on the stream"
            ),
        }
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(EllaError::Engine(
                "The local language model completed without returning reply text. Check llama-server logs and its OpenAI streaming endpoint."
                    .into(),
            ));
        }
        Ok(GeneratedReply::plain(
            text,
            ttft_ms.unwrap_or(completion_ms),
            completion_ms,
        ))
    }

    fn synthesize_oneshot(&self, text: &str) -> EllaResult<SynthesizedAudio> {
        eprintln!(
            "[LATENCY]     tts> spawning Piper for {} chars of text",
            text.chars().count()
        );
        let started = Instant::now();
        let mut command = Command::new(&self.piper_binary);
        command
            .arg("--model")
            .arg(&self.piper_voice)
            .arg("--output_raw")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        suppress_console_window(&mut command);
        let mut child = command.spawn().map_err(|error| {
            EllaError::Engine(format!(
                "Could not start Piper at {}: {error}",
                self.piper_binary.display()
            ))
        })?;
        eprintln!(
            "[LATENCY]     tts> +{:.1}ms Piper process spawned",
            started.elapsed().as_secs_f64() * 1_000.0
        );
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| EllaError::Engine("Piper stdout was not captured.".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| EllaError::Engine("Piper stderr was not captured.".into()))?;
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });
        let mut pcm = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        let mut first_audio_ms = None;
        loop {
            let count = stdout.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if first_audio_ms.is_none() {
                let first_audio = started.elapsed().as_secs_f64() * 1_000.0;
                eprintln!("[LATENCY]     tts> +{first_audio:.1}ms FIRST AUDIO bytes from Piper");
                first_audio_ms = Some(first_audio);
            }
            pcm.extend_from_slice(&buffer[..count]);
        }
        let status = child.wait()?;
        let stderr = stderr_reader.join().unwrap_or_default();
        let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "[LATENCY]     tts> +{completion_ms:.1}ms Piper finished ({} PCM bytes, ~{:.0} ms of audio)",
            pcm.len(),
            pcm.len() as f64 / 2.0 / 22_050.0 * 1_000.0
        );
        if !status.success() || pcm.is_empty() {
            let detail = String::from_utf8_lossy(&stderr);
            return Err(EllaError::Engine(format!(
                "Piper did not produce audio (exit {status}): {}. Verify the voice .onnx and .onnx.json match this Piper build.",
                detail.trim()
            )));
        }
        let encode_started = Instant::now();
        let base64 = STANDARD.encode(raw_pcm_to_wav(&pcm, 22_050, 1));
        eprintln!(
            "[LATENCY]     tts> wav+base64 encode took {:.1}ms ({} chars)",
            encode_started.elapsed().as_secs_f64() * 1_000.0,
            base64.len()
        );
        Ok(SynthesizedAudio {
            audio: Some(AudioPayload {
                mime_type: "audio/wav".into(),
                base64,
            }),
            first_audio_ms,
            completion_ms: Some(completion_ms),
            words: word_spans(text, &pcm, 22_050),
            // The standalone binary reports no timings, so her mouth only opens.
            phonemes: Vec::new(),
            segments: 0,
        })
    }
}

/// The Indian-English voice every character in `domain::characters` already
/// names, falling back to the stock American one when it is not installed.
///
/// Ella is spoken to Indian learners at about A1, and until now she answered in
/// `en_US-lessac-medium` — the accent mismatch is friction the learner pays for
/// on every turn. The `en_IN` voice is a fine-tune (its config still declares
/// `espeak.voice: en-US`, and `quality: "training_dir"` rather than a published
/// release), 22.05 kHz like the voice it replaces, and measured on this Mac it
/// costs nothing: 1389 ms to load against 1149, 212 ms to synthesize against
/// 198. Its `length_scale` of 1.15 makes it speak about a tenth slower, which
/// for an A1 learner is the point rather than a cost.
///
/// The fallback matters because a missing voice is silent rather than loud:
/// `synthesize` returns no audio and the shell drops to browser speech.
fn default_piper_voice(engine_root: &Path, models_root: &Path) -> PathBuf {
    // Voice first, root second: the Indian voice wins wherever it is, and only
    // then does the stock voice get looked for. Both roots are searched for
    // both voices, because a voice ships inside the installer while the
    // downloaded weights live in app data, and either one can hold either
    // file. Getting this wrong is silent — `synthesize` returns no audio
    // rather than an error when the voice is missing, so Ella simply stops
    // speaking and nothing says why.
    for voice in ["en_IN-navgurukul-medium.onnx", "en_US-lessac-medium.onnx"] {
        for root in [models_root.to_path_buf(), engine_root.join("models")] {
            let candidate = root.join("tts").join(voice);
            // Piper reads the sample rate and phoneme map from the sidecar and
            // will not synthesize without it, so a voice missing its .json is
            // not a voice.
            let sidecar = PathBuf::from(format!("{}.json", candidate.display()));
            if candidate.is_file() && sidecar.is_file() {
                return candidate;
            }
        }
    }
    // Nothing installed. Name the stock voice in app data so the error a
    // caller sees points at a path a repair could plausibly fill.
    models_root.join("tts/en_US-lessac-medium.onnx")
}

/// Weights live apart from binaries once installed, because app data is
/// writable and the resource directory is not. The decision is explicit rather
/// than inferred from what happens to exist: the Piper voice ships inside the
/// bundle, so `engine_root/models` can be present on an installed machine
/// while the downloaded weights are somewhere else entirely.
fn resolve_models_root(engine_root: &Path, packaged_models_root: Option<PathBuf>) -> PathBuf {
    if let Ok(configured) = env::var("ELLA_MODELS_ROOT") {
        return PathBuf::from(configured);
    }
    packaged_models_root.unwrap_or_else(|| engine_root.join("models"))
}

/// How many threads llama.cpp gets when nothing says otherwise: one per
/// physical performance core, which is also what llama.cpp would pick itself.
///
/// It used to be every logical core but two. On a four-core laptop with
/// hyper-threading that is six threads on four cores, and on an M1 Pro running
/// on its CPU, spreading from the six performance cores onto the two
/// efficiency cores cut decode from 47 to 34 tokens a second. Decode waits on
/// memory, not arithmetic, so the extra threads only get in each other's way,
/// and in Piper's, which synthesizes on whatever is left while the model
/// writes. Without hyper-threading nothing is left, so one core is kept back.
fn default_llama_threads() -> i32 {
    let logical = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4);
    llama_threads_for(performance_cores().unwrap_or(logical), logical) as i32
}

fn llama_threads_for(performance: usize, logical: usize) -> usize {
    let performance = performance.clamp(1, logical.max(1));
    if performance == logical && performance >= 4 {
        performance - 1
    } else {
        performance
    }
}

/// Physical cores that run at full speed: the performance cluster on Apple
/// silicon, every physical core on an Intel Mac.
#[cfg(target_os = "macos")]
fn performance_cores() -> Option<usize> {
    fn sysctl(name: &str) -> Option<usize> {
        let name = std::ffi::CString::new(name).ok()?;
        let mut value: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: `value` and `size` describe one writable c_int, which is
        // what both of the names asked for hold.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut libc::c_int).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && value > 0).then_some(value as usize)
    }
    sysctl("hw.perflevel0.physicalcpu").or_else(|| sysctl("hw.physicalcpu"))
}

/// Every physical core. A hybrid Intel chip counts its efficiency cores too,
/// as llama.cpp's own default does on Windows.
#[cfg(not(target_os = "macos"))]
fn performance_cores() -> Option<usize> {
    Some(num_cpus::get_physical()).filter(|cores| *cores > 0)
}

fn resolve_engine_root(packaged_engine_root: Option<PathBuf>) -> PathBuf {
    if let Ok(configured) = env::var("ELLA_ENGINE_ROOT") {
        return PathBuf::from(configured);
    }
    if let Some(packaged) = packaged_engine_root {
        if packaged.join("models").exists() || packaged.join("bin").exists() {
            return packaged;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engines")
}

#[cfg(target_os = "windows")]
fn default_piper_binary(engine_root: &Path) -> PathBuf {
    engine_root.join("bin/piper/piper.exe")
}

#[cfg(not(target_os = "windows"))]
fn default_piper_binary(engine_root: &Path) -> PathBuf {
    let venv = engine_root.join("piper-venv/bin/piper");
    if venv.is_file() {
        venv
    } else {
        engine_root.join("bin/piper/piper")
    }
}

/// The Piper this install can keep running between lines: Ella's daemon
/// script when Piper came with its own Python (the macOS bundle, a development
/// venv), and otherwise the standalone binary itself (the Windows bundle),
/// which stays up for as long as it is fed JSON lines.
fn resident_piper(piper_binary: &Path, voice: &Path) -> Option<Arc<PiperDaemon>> {
    if let Some(python) = piper_python_interpreter(piper_binary) {
        return Some(PiperDaemon::script(python, voice.to_path_buf()));
    }
    piper_binary
        .is_file()
        .then(|| PiperDaemon::binary(piper_binary.to_path_buf(), voice.to_path_buf()))
}

/// The Python beside a venv's `piper`, which runs Ella's daemon script. None
/// for the standalone C++ binary, which runs resident on its own.
fn piper_python_interpreter(piper_binary: &Path) -> Option<PathBuf> {
    let bin_dir = piper_binary.parent()?;
    for name in ["python3", "python"] {
        let candidate = bin_dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn env_i32(name: &str, default: i32) -> i32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// The character's first line.
///
/// Authored rather than generated, for the same reason the topics' openers
/// are: asked to open the market chore in character, Qwen2.5-3B produced "how
/// you doin souvik how r u today" on one run and "Favourite place for you
/// then?" on the next, and spent 8 to 24 seconds of session start doing it.
/// The generation budget now goes into warming the cache for turn 1 instead,
/// and the first thing the learner hears is always in character.
fn chore_opening_for(chore_id: &str) -> Option<&'static str> {
    match chore_id {
        "market-cloth-price" => Some(
            "Come, come, look at these shirts. Best cloth in this market. \
             Which one has caught your eye?",
        ),
        "deposit-refund" => Some(
            "Oh, it is you. I am rather busy this morning. What did you want to \
             talk about?",
        ),
        "sell-me-a-pen" => Some(
            "Right, you have thirty seconds and one pen. Go on then, what have \
             you got for me?",
        ),
        _ => None,
    }
}

/// A plain in-character opener, used when no model is generating one (demo
/// mode) and when generation comes back with nothing sayable. It names the
/// character, sets the scene, and hands the turn over, so even the fallback
/// starts a conversation rather than a monologue.
fn fallback_opening(context: &ChoreContext) -> String {
    format!(
        "{} here. {} What can I do for you?",
        context.character.name,
        context.setting.trim_end_matches('.'),
    )
}

fn opening_for(topic_id: &str, learner_name: &str) -> String {
    match topic_id {
        "restaurant-order" => format!(
            "Hi {learner_name}! We are at a restaurant and I am your waiter. What would you like to order today?"
        ),
        "booking-a-cab" => format!(
            "Hi {learner_name}! I am the cab driver. Where would you like to go, and where should I pick you up?"
        ),
        "job-interview" => format!(
            "Hello {learner_name}! Thank you for coming in. To start, could you tell me a little about yourself?"
        ),
        "doctor-clinic" => format!(
            "Hi {learner_name}! I am the doctor here. Please sit down and tell me, how have you been feeling?"
        ),
        "asking-directions" => format!(
            "Hi {learner_name}! You look a little lost. Where are you trying to go? I know this area well."
        ),
        "market-bargaining" => format!(
            "Hi {learner_name}! Come, come, best prices here. What are you looking for today?"
        ),
        _ => format!(
            "Hi {learner_name}! Tell me about the tastiest thing you ate this week. Where did you find it?"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llama_gets_a_thread_per_performance_core() {
        assert_eq!(llama_threads_for(4, 8), 4, "a hyper-threaded quad core");
        assert_eq!(llama_threads_for(2, 4), 2, "a hyper-threaded dual core");
        assert_eq!(llama_threads_for(6, 8), 6, "an M1 Pro's performance cores");
        assert_eq!(llama_threads_for(10, 12), 10, "a hybrid Intel chip");
        assert_eq!(llama_threads_for(4, 4), 3, "one core kept for Piper without hyper-threading");
        assert_eq!(llama_threads_for(2, 2), 2);
        assert_eq!(llama_threads_for(0, 8), 1);
        assert!(default_llama_threads() >= 1);
    }

    /// What a talk's warm-up restores has to be the start of what its turns
    /// send, token for token: every talk on the topic at that level shares it,
    /// and only the aim comes after it.
    #[test]
    fn a_free_talk_prompt_ends_on_its_aim_after_what_the_topic_shares() {
        let focus = Focus {
            step_title: "Talking about the past".into(),
            step_focus: "Starting to use past tense to share what happened.".into(),
            skill: "I can use past simple with regular verbs.".into(),
        };
        for topic in crate::domain::topics() {
            let own = ella_topic_prompt("Asha", &topic.id, &topic.label, "B1");
            let aimed = ella_system_prompt(
                "Asha",
                &topic.id,
                &topic.label,
                &Pitch { level: "B1".into(), focus: Some(focus.clone()) },
            );
            let plain = ella_system_prompt("Asha", &topic.id, &topic.label, &Pitch::at("B1"));
            assert_eq!(plain, own, "{}: without an aim the prompt is the topic's", topic.id);
            let aim = aimed.strip_prefix(&own).expect("the topic's words come first");
            assert!(aim.starts_with("\n\nAsha is on the step"), "{}: {aim}", topic.id);
            assert!(own.ends_with("what they are asking for or saying to you is."), "the guardrail stays last");
        }
    }

    #[test]
    fn a_reply_waits_for_the_warm_up_of_its_talk_and_only_for_that() {
        let warm_ups = Arc::new(WarmUps::default());
        let started = Instant::now();
        warm_ups.wait();
        assert!(started.elapsed() < Duration::from_millis(50), "nothing running, nothing to wait for");

        let running = warm_ups.begin();
        let finisher = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(running);
        });
        let started = Instant::now();
        warm_ups.wait();
        assert!(started.elapsed() >= Duration::from_millis(150), "waited for the warm-up");
        finisher.join().unwrap();
    }

    #[test]
    fn a_saved_slot_is_named_for_its_model_its_build_and_its_prompt() {
        let name = SlotStore::file_name("b1|model.gguf|2104932768|1759126000|4096", "talk", "You are Ella.");
        assert!(name.starts_with("ella-talk-") && name.ends_with(".bin"), "{name}");
        assert_eq!(name, SlotStore::file_name("b1|model.gguf|2104932768|1759126000|4096", "talk", "You are Ella."));
        for other in [
            SlotStore::file_name("b2|model.gguf|2104932768|1759126000|4096", "talk", "You are Ella."),
            SlotStore::file_name("b1|model.gguf|2104932768|1759999999|4096", "talk", "You are Ella."),
            SlotStore::file_name("b1|model.gguf|2104932768|1759126000|4096", "talk", "You are Ella!"),
        ] {
            assert_ne!(name, other);
        }
    }

    #[test]
    fn saved_slots_keep_what_was_just_used_and_the_newest_of_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let store = SlotStore::new(root.path().to_path_buf());
        let now = SystemTime::now();
        for (age, name) in [
            (50, "ella-topic-a.bin"),
            (40, "ella-topic-b.bin"),
            (30, "ella-topic-c.bin"),
            (20, "ella-topic-d.bin"),
            (90, "ella-talk-x.bin"),
        ] {
            let path = root.path().join(name);
            fs::write(&path, b"slot").unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age)).unwrap();
        }
        fs::write(root.path().join("not-a-slot.txt"), b"keep").unwrap();
        store.prune(&["ella-talk-x.bin".into(), "ella-topic-a.bin".into()]);
        let mut left: Vec<String> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["ella-talk-x.bin", "ella-topic-a.bin", "ella-topic-c.bin", "ella-topic-d.bin", "not-a-slot.txt"]
        );
    }

    #[test]
    fn demo_reply_is_short_and_asks_a_question() {
        let reply = DemoEngine
            .reply(&TutorRequest {
                learner_name: "Asha".into(),
                topic_id: "street-food".into(),
                topic_label: "Street food stories".into(),
                messages: vec![],
                learner_text: "I played football with friends".into(),
                turn: 1,
                chore: None,
                pitch: Pitch::at("A1"),
                placement: None,
            })
            .unwrap();
        assert!(reply.text.ends_with('?'));
        assert!(reply.text.len() < 180);
    }

    #[test]
    #[ignore = "requires Canary, llama.cpp, Whisper fallback, and Piper development engines"]
    fn local_engine_runs_speech_to_speech_vertical_slice() {
        let engine = LocalEngine::from_environment(EnginePaths::default());
        assert!(engine.status().ready, "local engines are not ready");

        let audio = engine
            .synthesize("I played football with my best friend after school.")
            .unwrap()
            .audio
            .expect("Piper should be configured");
        let wav = STANDARD.decode(audio.base64).unwrap();
        let samples = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        let transcript = engine.transcribe(&samples, 22_050).unwrap();
        assert_eq!(transcript.engine, "canary-180m-flash-q8_0");
        assert!(transcript.text.to_lowercase().contains("football"));

        let reply = engine
            .reply(&TutorRequest {
                learner_name: "Asha".into(),
                topic_id: "street-food".into(),
                topic_label: "Street food stories".into(),
                messages: vec![],
                learner_text: transcript.text,
                turn: 1,
                chore: None,
                pitch: Pitch::at("A1"),
                placement: None,
            })
            .unwrap();
        assert!(!reply.text.is_empty());
        assert!(reply.text.contains('?'));
        let spoken = engine.synthesize(&reply.text).unwrap();
        assert!(spoken.audio.is_some());
        assert!(spoken.first_audio_ms.is_some());
    }
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    use crate::domain::{find_chore, WinCondition};

    fn market_spec() -> LedgerSpec {
        match find_chore("market-cloth-price").unwrap().win {
            WinCondition::Ledger(spec) => spec,
            _ => unreachable!("market-cloth-price is a ledger chore"),
        }
    }

    fn refund_spec() -> LedgerSpec {
        match find_chore("deposit-refund").unwrap().win {
            WinCondition::Ledger(spec) => spec,
            _ => unreachable!("deposit-refund is a ledger chore"),
        }
    }

    /// A live chore context, at whatever point in the negotiation the test
    /// needs it to be.
    fn chore_context(chore_id: &str, current: i32, agreed: bool) -> ChoreContext {
        let chore = find_chore(chore_id).unwrap();
        let ledger = match &chore.win {
            WinCondition::Ledger(spec) => Some(crate::domain::LedgerTurn {
                spec: spec.clone(),
                current,
                agreed,
            }),
            WinCondition::Rubric { .. } => None,
        };
        ChoreContext {
            character: crate::domain::find_character(&chore.character_id).unwrap(),
            chore_id: chore.id,
            level: "A1".into(),
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger,
        }
    }

    #[test]
    fn a_dodged_question_is_re_asked_rather_than_advanced_past() {
        // The same real session: the opener asked "what did you eat / where
        // did you find it", the learner dodged with an unrelated remark, and
        // Ella replied "Now, where did you find the tasty stall? What was
        // the smell like?" — inventing a stall that was never named and
        // moving to a deeper question, as though the dodge had answered the
        // original one.
        let prompt = ella_system_prompt("Asha", "restaurant-order", "Ordering at a restaurant", &Pitch::at("A1"));
        assert!(
            prompt.contains("ask the exact same question again"),
            "a dodge must be met with the same question, not a new one that assumes it was answered"
        );
        assert!(
            prompt.contains("unless they never actually answered it"),
            "this must not be read as conflicting with \"never ask a question you have already asked\" \
             — a question nobody answered has not been asked in the sense that rule means"
        );
    }

    #[test]
    fn a_free_conversation_is_told_the_scene_its_opener_walked_into() {
        // `opening_for` says "I am your waiter". Before the two were tied
        // together the very next turn came back as a speaking buddy talking
        // about restaurants in the abstract.
        let opener = opening_for("restaurant-order", "Asha");
        assert!(opener.contains("waiter"));
        let prompt = ella_system_prompt("Asha", "restaurant-order", "Ordering at a restaurant", &Pitch::at("A1"));
        assert!(prompt.contains("waiter"), "the prompt drops the role the opener promised");
        assert!(prompt.contains("Asha"));
        assert!(prompt.contains("Ordering at a restaurant"), "the topic has to be named");
        assert!(
            prompt.contains("settle the bill"),
            "the goal on the learner's topic card is what the conversation is for"
        );
    }

    #[test]
    fn a_free_conversation_is_told_not_to_play_along_with_a_romantic_advance() {
        // A real session had the learner escalate from "go on a date with you" to
        // an explicit request, and Ella validated every step ("a kiss would be
        // lovely", "that's wonderful") instead of declining. Nothing in the
        // prompt distinguished "wandered off topic" from "said something
        // inappropriate", so the off-topic redirect rule was all the model had.
        let prompt = ella_system_prompt("Asha", "restaurant-order", "Ordering at a restaurant", &Pitch::at("A1"));
        assert!(
            prompt.contains("romantic") && prompt.contains("sexual"),
            "the prompt has no instruction covering romantic or sexual advances"
        );
        assert!(
            prompt.contains("do not go along with"),
            "the guardrail must tell the model not to validate the advance, not just redirect"
        );
        assert!(
            prompt.contains("do not repeat"),
            "declining is not enough on its own: a live run came back \"No kissing!\" \
             and \"No sex!\" — the general \"answer what they said\" rule pulled the \
             explicit word into the decline itself, which is exactly what a young \
             learner should not hear back"
        );
    }

    #[test]
    fn the_free_conversation_guardrail_spells_out_examples_by_category() {
        // A live session had the learner say "Can you give me a hug?" and "Can
        // you give me kisses?" — phrasing neither the deterministic filter nor
        // the model's own instinct caught, since "hug"/"kisses" carry no
        // vulgar vocabulary and rustrict does not stem plurals. Naming the
        // categories and several phrasings of each, in the prompt itself,
        // is the only remaining lever once the deterministic floor cannot
        // reach every wording.
        let prompt = ella_system_prompt("Asha", "restaurant-order", "Ordering at a restaurant", &Pitch::at("A1"));
        for example in [
            "give me a hug",
            "kiss me",
            "kisses",
            "spend the night with me",
            "marry me",
            "you are beautiful",
            "you're hot",
        ] {
            assert!(
                prompt.contains(example),
                "expected the guardrail to name {example:?} as an example to watch for"
            );
        }
        assert!(
            prompt.contains("not only the exact one written here"),
            "the examples are illustrative, not exhaustive — the prompt must say so \
             explicitly or a 3B model reads them as the complete list"
        );
    }

    #[test]
    fn every_topic_in_the_catalog_has_a_scene_of_its_own() {
        let generic = scene_for("no-such-topic-id");
        for topic in crate::domain::topics() {
            // Street food is Ella as herself, which is what its opener says
            // too, so it is the one topic the fallback scene is right for.
            if topic.id == "street-food" {
                continue;
            }
            assert_ne!(
                scene_for(&topic.id).role,
                generic.role,
                "{} falls through to the generic scene, so its opener's role is dropped at turn 1",
                topic.id
            );
        }
    }

    #[test]
    fn a_free_conversation_is_told_to_stop_asking_questions_at_the_end() {
        assert!(free_closing_note(1).is_none(), "nothing to say in the middle");
        assert!(free_closing_note(FREE_TOPIC_TURNS - 1)
            .unwrap()
            .contains("last question"));
        assert!(free_closing_note(FREE_TOPIC_TURNS)
            .unwrap()
            .contains("do not ask another question"));
        assert!(
            free_closing_note(FREE_TOPIC_TURNS + 5).is_some(),
            "a conversation that runs long still gets an ending"
        );
    }

    #[test]
    fn agreeing_in_words_alone_settles_on_the_figure_the_learner_named() {
        // The market bench run ended "Alright, alright. Take it at that price."
        // one turn after the learner said four hundred — and recorded Rs 450.
        let spec = market_spec();
        assert_eq!(
            settled_figure(
                "Alright, take it",
                "Okay, four hundred rupees and I will buy it right now.",
                Some(TurnSignal::Deal),
                &spec,
                450,
            ),
            Some(400)
        );
        assert_eq!(
            settled_figure("Take it for Rs 425", "I will pay Rs 400", Some(TurnSignal::Deal), &spec, 450),
            Some(425),
            "a figure the character named itself always wins"
        );
        assert_eq!(
            settled_figure(
                "Alright, take it",
                "I will give you one hundred rupees, final.",
                Some(TurnSignal::Deal),
                &spec,
                450,
            ),
            None,
            "agreeing to a lowball still does not move the number below the floor"
        );
        assert_eq!(
            settled_figure("You are asking a lot", "I will pay Rs 400", None, &spec, 450),
            None,
            "the learner's figure only counts when the character has agreed"
        );
    }

    #[test]
    fn a_figure_spoken_in_words_is_read_the_way_a_learner_says_it() {
        let market = market_spec();
        assert_eq!(spelled_figure("four hundred rupees", &market), Some(400));
        assert_eq!(
            spelled_figure("Can you do it for four hundred and fifty rupees?", &market),
            Some(450)
        );
        assert_eq!(
            spelled_figure("Three thousand five hundred rupees and we are finished.", &refund_spec()),
            Some(3500)
        );
        assert_eq!(
            spelled_figure("Rs 600 was my price, but take it for four hundred rupees", &market),
            Some(400),
            "the last figure wins, as it does among digits"
        );
        assert_eq!(
            spelled_figure("I have sold here for twenty years", &market),
            None,
            "no unit named anywhere, so no figure"
        );
        assert_eq!(
            spelled_figure("that is a hundred rupees too much", &market),
            Some(100),
            "\"a hundred\" still reads as a hundred"
        );
        assert_eq!(
            spelled_figure("I only have a little money today", &market),
            None
        );
    }

    #[test]
    fn a_chore_that_has_conceded_far_enough_is_told_to_close() {
        // The deposit bench run walked the figure all the way to its ceiling
        // and still never signed off, so the chore could not be won.
        let at_target = chore_turn_message(&chore_context("deposit-refund", 3500, false), 4).unwrap();
        assert!(at_target.contains("3500"), "the live figure belongs in the turn message");
        assert!(at_target.contains("[DEAL]"), "at the target, closing is the honest move");

        let mid = chore_turn_message(&chore_context("deposit-refund", 1500, false), 4).unwrap();
        assert!(mid.contains("1500"));
        assert!(!mid.contains("[DEAL]"), "there is still ground to give at Rs 1500");
    }

    #[test]
    fn the_indian_voice_is_preferred_and_the_stock_one_is_the_fallback() {
        let root = std::env::temp_dir().join("ella-voice-pick-test");
        let tts = root.join("models/tts");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&tts).unwrap();

        // Nothing installed: the stock voice, so a fresh tree still speaks.
        assert!(default_piper_voice(&root, &root.join("models")).ends_with("en_US-lessac-medium.onnx"));

        // The model alone is not enough — Piper needs the sidecar beside it,
        // and picking a voice whose sidecar is missing would go silent.
        fs::write(tts.join("en_IN-navgurukul-medium.onnx"), b"x").unwrap();
        assert!(default_piper_voice(&root, &root.join("models")).ends_with("en_US-lessac-medium.onnx"));

        fs::write(tts.join("en_IN-navgurukul-medium.onnx.json"), b"{}").unwrap();
        assert!(default_piper_voice(&root, &root.join("models")).ends_with("en_IN-navgurukul-medium.onnx"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_stock_voice_is_found_in_the_bundle_when_app_data_holds_only_weights() {
        // The packaged layout, which is what shipped broken: the voice is
        // staged inside the installer, while the models root is app data and
        // holds only what was downloaded. Resolving the stock voice against
        // app data alone named a file that is never written there, and a
        // missing voice makes `synthesize` return no audio without an error —
        // so v0.1.0 ran mute and no telemetry said why.
        let root = std::env::temp_dir().join("ella-voice-bundle-test");
        let _ = fs::remove_dir_all(&root);
        let engine_root = root.join("resources/engines");
        let models_root = root.join("appdata/models");
        fs::create_dir_all(engine_root.join("models/tts")).unwrap();
        fs::create_dir_all(models_root.join("llm")).unwrap();

        fs::write(engine_root.join("models/tts/en_US-lessac-medium.onnx"), b"x").unwrap();
        fs::write(engine_root.join("models/tts/en_US-lessac-medium.onnx.json"), b"{}").unwrap();

        let picked = default_piper_voice(&engine_root, &models_root);
        assert!(picked.is_file(), "picked a voice that does not exist: {}", picked.display());
        assert!(picked.ends_with("en_US-lessac-medium.onnx"));

        // The Indian voice still wins from the bundle when it is installed.
        fs::write(engine_root.join("models/tts/en_IN-navgurukul-medium.onnx"), b"x").unwrap();
        fs::write(engine_root.join("models/tts/en_IN-navgurukul-medium.onnx.json"), b"{}").unwrap();
        assert!(default_piper_voice(&engine_root, &models_root)
            .ends_with("en_IN-navgurukul-medium.onnx"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_scene_says_what_its_role_answers_and_never_hands_it_to_the_learner() {
        // The cab transcript: told only that the learner was here to "agree a
        // fare, and ask how long the trip takes", Ella asked the passenger both
        // of those, four turns running. Every scene now states what is hers.
        for topic in crate::domain::topics() {
            let scene = scene_for(&topic.id);
            assert!(!scene.owns.is_empty(), "{} has no owned ground", topic.id);
            let prompt = ella_system_prompt("Souvik", &topic.id, &topic.label, &Pitch::at("A1"));
            assert!(prompt.contains(scene.owns), "{} drops what the role owns", topic.id);
            assert!(
                prompt.contains("never ask them a question that is yours to answer"),
                "{} lets the role hand its own lines to the learner",
                topic.id
            );
        }
    }

    #[test]
    fn a_scene_is_told_to_hold_its_invented_details_steady() {
        // Nothing state-tracks a free conversation's invented price or
        // symptom the way `LedgerSpec` does for a chore, so the one thing
        // stopping a dish or a fare from changing value mid-conversation is
        // this instruction. Street food is the fallback scene and invents
        // nothing of its own, so it is exempt the same way the "every topic
        // has a scene of its own" test above exempts it.
        for topic in crate::domain::topics() {
            if topic.id == "street-food" {
                continue;
            }
            let scene = scene_for(&topic.id);
            assert!(
                scene.owns.contains("keep it the same"),
                "{} invents a detail but never says to hold it steady",
                topic.id
            );
        }
    }

    #[test]
    fn a_scene_shows_a_worked_example_of_the_shape_it_wants() {
        // A bare instruction to "name a price" still leaves the shape
        // underspecified; a short worked example is what the guardrail
        // examples above already prove this model needs concrete anchors
        // for. Prose ("say it is ninety rupees"), never a labelled
        // transcript ("Ella: ...") — this is a voice app, and a model that
        // picks up a speaker-label habit would say the label out loud too.
        for topic in crate::domain::topics() {
            if topic.id == "street-food" {
                continue;
            }
            let scene = scene_for(&topic.id);
            assert!(
                scene.owns.contains("For example"),
                "{} has no worked example of its own shape",
                topic.id
            );
            assert!(
                !scene.owns.contains("Ella:") && !scene.owns.contains(":\""),
                "{} example reads as a speaker-labelled transcript, not prose",
                topic.id
            );
        }
    }

    #[test]
    fn the_off_topic_redirect_does_not_swallow_an_on_topic_rephrasing() {
        // A worked example of "decline once, then repeat the question"
        // was tried here before and reverted: it taught the model that
        // shape so well it started declining ordinary on-topic questions
        // too ("Can you teach me how to eat biryani?"), per the bench
        // regression documented in `chore-bench.rs`. This is the guard
        // against that regression happening again: the instruction must
        // name what does *not* count as off-topic, not just what does.
        let prompt = ella_system_prompt("Asha", "restaurant-order", "Ordering at a restaurant", &Pitch::at("A1"));
        assert!(
            prompt.contains("nothing to do with"),
            "the prompt has no instruction covering a genuinely unrelated request"
        );
        assert!(
            prompt.contains("never an on-topic request just because it is phrased unusually"),
            "the prompt does not guard against declining an on-topic rephrasing"
        );
    }

    #[test]
    fn the_off_topic_redirect_covers_entertaining_as_well_as_tasking() {
        // Live session, first attempt at this fix: asked to tell a joke in
        // job-interview, Ella told one in full ("Why did the tomato turn
        // red? To get out of the kitchen!") and then asked her next
        // question as if nothing had happened. "Write code, solve a
        // puzzle" reads as obviously off-topic; a joke reads as friendly
        // in-character banter, so it slipped past the same way
        // "hug"/"kisses" slipped past the safety filter's word list — and
        // a first fix that only added the joke example without an
        // explicit generalization, and without overriding "answer the
        // thing they actually said", still did not hold.
        let prompt = ella_system_prompt("Asha", "job-interview", "A job interview", &Pitch::at("A1"));
        assert!(
            prompt.contains("tell a joke"),
            "the prompt does not name joke-telling as an example of the entertain category"
        );
        assert!(
            prompt.contains("treat any new way of asking for something like this the same as these examples"),
            "the prompt has no generalization past its literal list of off-topic examples"
        );
        assert!(
            prompt.contains("\"Answer the thing they actually said\" does not apply here either"),
            "the prompt never overrides the general answer-what-they-said rule for an off-topic request"
        );
        assert!(
            prompt.contains("do not answer it, joke back, or begin doing any part of what they asked"),
            "the prompt does not forbid partially complying before declining"
        );
    }

    #[test]
    fn the_question_ella_ends_on_is_the_one_that_gets_compared() {
        assert_eq!(
            trailing_question("Two rupees is very reasonable. How long will the trip take?"),
            Some("How long will the trip take?")
        );
        assert_eq!(trailing_question("Safe travels. Have a great day!"), None);
        assert_eq!(
            trailing_question("Rs 350."),
            None,
            "a reply with no question cannot repeat one"
        );
    }

    #[test]
    fn a_reworded_repeat_of_an_earlier_question_is_caught() {
        let asked = vec!["How long will the trip take?".to_owned()];
        assert!(
            repeated_question("Oh, right. How long will the trip to the airport take?", &asked)
                .is_some(),
            "this is the exact loop the cab conversation got stuck in"
        );
        assert!(
            repeated_question("How long will the trip take, please?", &asked).is_some(),
            "politeness is not a new question"
        );
        assert!(
            repeated_question("Two rupees, fine. Where should I pick you up?", &asked).is_none(),
            "a genuinely different question has to get through"
        );
        assert!(
            repeated_question("Safe travels!", &asked).is_none(),
            "no question, nothing to repeat"
        );
        assert!(
            repeated_question("Really?", &asked).is_none(),
            "one content word is too little to call a repeat"
        );
    }

    #[test]
    fn the_same_question_at_three_different_widths_is_one_question() {
        // Straight from a street-food bench run, which asked all three in a row
        // and got past a plain three-quarters overlap test at 0.67.
        let asked = vec!["What did you think of the chutney?".to_owned()];
        assert!(repeated_question("What did you think?", &asked).is_some());
        // The widest form is caught against the narrowest one, which by then is
        // in the history — the order the real run asked them in.
        let asked_both = vec![
            "What did you think of the chutney?".to_owned(),
            "What did you think?".to_owned(),
        ];
        assert!(
            repeated_question("What did you think of the home-cooked bhel puri?", &asked_both)
                .is_some()
        );
        assert!(
            repeated_question("What did you think of the home-cooked bhel puri?", &asked)
                .is_none(),
            "against the chutney question alone these share only two words of six, \
             and firing on that would catch questions that are genuinely new"
        );
        assert!(
            repeated_question("Where did you buy it?", &asked).is_none(),
            "a different question still has to get through"
        );
        assert!(
            repeated_question("What did you eat on Sunday?", &asked).is_none(),
            "sharing only the question word is not a repeat"
        );
    }

    #[test]
    fn a_ledger_turn_message_never_ends_on_a_figure() {
        // Measured against the live 3B: with the number last, 5/5 replies opened
        // "Rs 550."; with this instruction last, 0/5 did.
        for (current, agreed, turn) in
            [(600, false, 1), (525, false, 4), (400, true, 6), (525, false, 12)]
        {
            let message =
                chore_turn_message(&chore_context("market-cloth-price", current, agreed), turn)
                    .unwrap();
            assert!(
                message.ends_with("inside a sentence."),
                "turn {turn} ends on `{}`",
                message.rsplit(' ').next().unwrap_or_default()
            );
            let tail = message.trim_end_matches('.').rsplit(' ').next().unwrap_or_default();
            assert!(
                !tail.chars().any(|c| c.is_ascii_digit()),
                "turn {turn} still leaves a figure as the last thing the model reads"
            );
        }
    }

    #[test]
    fn a_rubric_chore_gets_no_reply_shape_because_it_has_no_figure() {
        let context = chore_context("sell-me-a-pen", 0, false);
        let last = chore_turn_message(&context, context.max_turns).unwrap();
        assert!(
            !last.contains("Any figure comes after"),
            "there is no ledger here, so nothing to place a figure against"
        );
    }

    #[test]
    fn an_agreed_chore_is_told_not_to_reopen_the_figure() {
        let note = chore_turn_message(&chore_context("market-cloth-price", 400, true), 6).unwrap();
        assert!(note.contains("already agreed"));
        assert!(!note.contains("[DEAL]"), "the deal is done; nothing left to close");
    }

    #[test]
    fn the_last_turn_of_a_chore_asks_for_a_decision() {
        let context = chore_context("market-cloth-price", 525, false);
        let last = chore_turn_message(&context, context.max_turns).unwrap();
        assert!(last.contains("[DEAL]") && last.contains("[WALK]"));
        let early = chore_turn_message(&context, 1).unwrap();
        assert!(
            !early.contains("[WALK]"),
            "turn 1 of a twelve-turn chore is not the time to walk away"
        );
    }

    #[test]
    fn a_rubric_chore_still_gets_its_ending_without_a_ledger() {
        // No figure to report, so the only turn message is the closing one.
        let context = chore_context("sell-me-a-pen", 0, false);
        assert!(context.ledger.is_none());
        assert!(chore_turn_message(&context, 1).is_none());
        assert!(chore_turn_message(&context, context.max_turns).is_some());
    }

    #[test]
    fn a_concession_within_the_step_is_accepted_and_one_past_the_floor_is_not() {
        let spec = market_spec();
        assert!(spec.accepts(600, 550), "Rs 50 down from 600 is inside the step");
        assert!(!spec.accepts(600, 500), "Rs 100 down exceeds the Rs 75 step");
        assert!(!spec.accepts(400, 300), "Rs 300 is below the Rs 350 floor");
        assert!(spec.accepts(600, 600), "holding the figure is always legal");
        assert!(!spec.accepts(600, 650), "the price never goes back up");
    }

    #[test]
    fn pushing_up_reads_the_limit_as_a_ceiling() {
        let spec = refund_spec();
        assert!(spec.accepts(500, 1500), "Rs 1000 up is inside the Rs 1200 step");
        assert!(!spec.accepts(500, 2000), "Rs 1500 up exceeds the step");
        assert!(!spec.accepts(4000, 4500), "Rs 4500 is above the Rs 4200 ceiling");
        assert!(!spec.accepts(1500, 1000), "a refund offer never shrinks");
    }

    #[test]
    fn the_target_is_reached_from_either_direction() {
        assert!(market_spec().reached_target(400));
        assert!(market_spec().reached_target(380));
        assert!(!market_spec().reached_target(425));
        assert!(refund_spec().reached_target(3500));
        assert!(!refund_spec().reached_target(3400));
    }

    #[test]
    fn the_target_always_sits_short_of_the_limit_so_a_leaked_brief_is_not_a_win() {
        // The spec's own mitigation for a 3B revealing its hidden floor.
        for chore in crate::domain::chores() {
            if let WinCondition::Ledger(spec) = chore.win {
                match spec.direction {
                    Direction::Down => assert!(
                        spec.target > spec.limit,
                        "{}: target {} must sit above the floor {}",
                        chore.id,
                        spec.target,
                        spec.limit
                    ),
                    Direction::Up => assert!(
                        spec.target < spec.limit,
                        "{}: target {} must sit below the ceiling {}",
                        chore.id,
                        spec.target,
                        spec.limit
                    ),
                }
            }
        }
    }

    #[test]
    fn a_figure_beside_the_unit_wins_over_a_bare_number() {
        let spec = market_spec();
        assert_eq!(extract_figure("Rs 550 for you", &spec), Some(550));
        assert_eq!(extract_figure("550 rupees, final", &spec), Some(550));
        assert_eq!(
            extract_figure("I have sold here 20 years. Rs 500.", &spec),
            Some(500),
            "the unit-adjacent figure beats the incidental one"
        );
        assert_eq!(
            extract_figure("I have sold here for 20 years.", &spec),
            None,
            "a bare number with no unit anywhere is not an offer"
        );
        assert_eq!(extract_figure("Rs 1,250 for two", &spec), Some(1250));
        assert_eq!(
            extract_figure("I have sold here 20 years, no discount.", &spec),
            None,
            "\"years\" contains \"rs\" - aliases must match on word boundaries"
        );
    }

    #[test]
    fn the_last_figure_wins_when_a_reply_recites_the_old_one_first() {
        let spec = market_spec();
        assert_eq!(
            extract_figure("Rs 600 was my price, but take it for Rs 525.", &spec),
            Some(525)
        );
    }

    #[test]
    fn signals_are_taken_bracketed_or_bare_but_never_from_prose() {
        assert_eq!(
            take_signal("Alright, take it. [DEAL]"),
            ("Alright, take it".into(), Some(TurnSignal::Deal))
        );
        // Qwen2.5-3B drops the brackets in practice.
        assert_eq!(take_signal("DEAL"), (String::new(), Some(TurnSignal::Deal)));
        assert_eq!(
            take_signal("Fine, we are done here. WALK"),
            ("Fine, we are done here".into(), Some(TurnSignal::Walk))
        );
        assert_eq!(
            take_signal("That is a good deal for you."),
            ("That is a good deal for you.".into(), None),
            "prose keeps its full stop, and lowercase is not a control token"
        );
    }

    #[test]
    fn a_one_word_reply_of_deal_is_an_agreement_whatever_its_casing() {
        assert_eq!(take_signal("Deal."), (String::new(), Some(TurnSignal::Deal)));
        assert_eq!(take_signal("Walk"), (String::new(), Some(TurnSignal::Walk)));
        assert_eq!(
            take_signal("That is a good deal for you."),
            ("That is a good deal for you.".into(), None),
            "the word inside a sentence is still prose"
        );
        assert_eq!(
            take_signal("It is a deal"),
            ("It is a deal".into(), None),
            "lowercase at the end of a sentence is not the whole reply"
        );
    }

    #[test]
    fn invented_bracket_tokens_never_reach_the_synthesiser() {
        // Told about [DEAL] and [WALK], the model invents [GO].
        let (text, signal) = take_signal("Rs 600. [GO]");
        assert_eq!(text, "Rs 600.", "an invented token goes, the full stop stays");
        assert_eq!(signal, None);
        assert_eq!(strip_bracket_tokens("how ya doin? [GO]"), "how ya doin?");
        assert_eq!(strip_bracket_tokens("[PAUSE] Rs 500 [STOP]"), "Rs 500");
    }

    #[test]
    fn an_unbalanced_bracket_is_ordinary_text_not_an_open_token() {
        // Truncating here silently deleted the rest of a reply.
        assert_eq!(
            strip_bracket_tokens("I paid [ 500 rupees for it"),
            "I paid [ 500 rupees for it"
        );
        assert_eq!(strip_bracket_tokens("line one\nline two"), "line one\nline two");
    }

    #[test]
    fn a_multibyte_reply_does_not_panic_on_token_search() {
        // 'ß'.to_uppercase() is "SS": offsets from an uppercased copy drift.
        let (text, signal) = take_signal("Straße kostet Rs 500. [DEAL]");
        assert_eq!(signal, Some(TurnSignal::Deal));
        assert!(text.starts_with("Stra"), "got {text:?}");
        assert_eq!(find_ascii_ci("Straße DEAL", "deal"), Some(8));
        assert_eq!(find_ascii_ci("nothing here", "DEAL"), None);
    }

    #[test]
    fn a_token_only_reply_is_voiced_with_the_authored_line() {
        let spec = market_spec();
        assert_eq!(
            voiced(String::new(), Some(TurnSignal::Deal), &spec),
            spec.acceptance
        );
        assert_eq!(
            voiced(String::new(), Some(TurnSignal::Walk), &spec),
            spec.refusal
        );
        assert_eq!(
            voiced("Take it at Rs 400".into(), Some(TurnSignal::Deal), &spec),
            "Take it at Rs 400",
            "real words are never replaced"
        );
    }

    #[test]
    fn the_live_figure_stays_out_of_the_cached_prefix() {
        // A figure in the leading system prompt invalidates llama.cpp's prompt
        // cache every turn; measured as TTFT 1.8s -> 7.5s before this split.
        let chore = find_chore("market-cloth-price").unwrap();
        let spec = market_spec();
        let context = ChoreContext {
            chore_id: chore.id,
            level: "A1".into(),
            character: crate::domain::find_character("stall-owner").unwrap(),
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger: Some(crate::domain::LedgerTurn { spec: spec.clone(), current: 525, agreed: false }),
        };
        let prompt = chore_system_prompt("Souvik", &context);
        assert!(!prompt.contains("525"), "the live figure leaked into the prefix");
        assert!(prompt.contains("350"), "the floor is stable and belongs in the prefix");
        assert!(
            !prompt.contains("75"),
            "naming the step size teaches the model to concede exactly that much every \
             turn: the deposit bench run laddered 500-1000-2000-3000-4000 regardless of \
             what the learner said. Rust clamps the step, so the model never needs it"
        );
        assert!(ledger_state_message(&spec, 525).contains("525"));
    }

    #[test]
    fn a_chore_character_is_told_not_to_play_along_with_a_romantic_advance_either() {
        // The free-conversation guardrail alone would not have caught this: a
        // chore character has its own "stay in character at all times" line,
        // which without an explicit carve-out reads as permission to go along
        // with anything so long as it fits the persona.
        let chore = find_chore("market-cloth-price").unwrap();
        let context = ChoreContext {
            chore_id: chore.id,
            level: "A1".into(),
            character: crate::domain::find_character("stall-owner").unwrap(),
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger: None,
        };
        let prompt = chore_system_prompt("Souvik", &context);
        assert!(
            prompt.contains("romantic") && prompt.contains("sexual"),
            "the prompt has no instruction covering romantic or sexual advances"
        );
        assert!(
            prompt.contains("even in character"),
            "without this, \"stay in character at all times\" reads as permission to go along with it"
        );
        assert!(
            prompt.contains("do not repeat"),
            "a live run had the stall-owner reply \"No kissing!\" and \"No sex!\" — \
             the general \"answer what they just said\" rule pulled the explicit word \
             into the decline itself"
        );
        assert!(
            prompt.contains("abusive"),
            "a chore roleplay can attract insults or threats directed at the \
             character, not just romantic content — an earlier edit left this word \
             grammatically stranded (\"...sexual, abusive do not go along...\"), so \
             this also guards against that regressing silently"
        );
    }

    #[test]
    fn a_chore_character_is_told_not_to_go_off_scene_either() {
        // The free-conversation prompt has this guard; the chore prompt did
        // not, and had exactly the same gap the romantic-advance test above
        // documents: "stay in character at all times" alone reads as
        // permission to comply with anything in-character, including a
        // request that has nothing to do with the scene at all (a joke, a
        // song, a coding task).
        let chore = find_chore("market-cloth-price").unwrap();
        let context = ChoreContext {
            chore_id: chore.id,
            level: "A1".into(),
            character: crate::domain::find_character("stall-owner").unwrap(),
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger: None,
        };
        let prompt = chore_system_prompt("Souvik", &context);
        assert!(
            prompt.contains("tell a joke"),
            "the chore prompt does not name joke-telling as an example of the off-scene category"
        );
        assert!(
            prompt.contains("treat any new way of asking for something like this the same as these examples"),
            "the chore prompt has no generalization past its literal list of off-scene examples"
        );
        assert!(
            prompt.contains("\"Answer what they just said\" does not apply here either"),
            "the chore prompt never overrides its own answer-what-they-said rule for an off-scene request"
        );
    }

    #[test]
    fn the_chore_guardrail_spells_out_examples_by_category_too() {
        let chore = find_chore("market-cloth-price").unwrap();
        let context = ChoreContext {
            chore_id: chore.id,
            level: "A1".into(),
            character: crate::domain::find_character("stall-owner").unwrap(),
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger: None,
        };
        let prompt = chore_system_prompt("Souvik", &context);
        for example in [
            "give me a hug",
            "kiss me",
            "kisses",
            "spend the night with me",
            "marry me",
            "you are beautiful",
            "you're hot",
        ] {
            assert!(
                prompt.contains(example),
                "expected the chore guardrail to name {example:?} as an example to watch for"
            );
        }
        assert!(
            prompt.contains("not only the exact one written here"),
            "the examples are illustrative, not exhaustive — the prompt must say so explicitly"
        );
    }
}

#[cfg(test)]
mod speech_stream_tests {
    use super::*;

    /// Streams `text` in small pieces, the way SSE deltas arrive, and returns
    /// exactly what Piper would be asked to read — splitter plus the worker's
    /// filter, so a segment that cleans down to nothing does not appear.
    fn segments(text: &str, delta_chars: usize) -> Vec<String> {
        let mut splitter = SentenceSplitter::default();
        let mut raw = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        for piece in chars.chunks(delta_chars) {
            raw.extend(splitter.push(&piece.iter().collect::<String>()));
        }
        raw.extend(splitter.flush());
        raw.into_iter()
            .map(|sentence| spoken_form(&sentence))
            .filter(|text| text.chars().any(char::is_alphanumeric))
            .collect()
    }

    /// The whole point: a two-sentence reply hands Piper the first sentence
    /// before the second one has been written.
    #[test]
    fn splits_a_reply_into_sentences() {
        assert_eq!(
            segments("That sounds delicious! What did it taste like?", 3),
            vec!["That sounds delicious!", "What did it taste like?"]
        );
    }

    /// Whatever the chunk size, the sentences come out the same — the splitter
    /// must not depend on where a token boundary happens to fall.
    #[test]
    fn chunking_does_not_change_the_split() {
        let text = "I went to the market. It was very crowded. Did you go too?";
        let expected = vec![
            "I went to the market.",
            "It was very crowded.",
            "Did you go too?",
        ];
        for size in [1, 2, 5, 17, 200] {
            assert_eq!(segments(text, size), expected, "delta size {size}");
        }
    }

    /// Nothing may be lost or added: what Piper reads has to be what the
    /// learner reads.
    #[test]
    fn segments_rejoin_to_the_original_words() {
        let text = "Rs 250 is my last price. Take it or leave it, my friend!";
        for size in [1, 4, 13] {
            assert!(same_words(&segments(text, size).join(" "), text));
        }
    }

    /// "Rs." is how the stall owner writes rupees, and it is mid-sentence every
    /// time. Cutting there would have Piper read "Rs" as a sentence.
    #[test]
    fn does_not_split_on_an_abbreviation() {
        assert_eq!(
            segments("My price is Rs. 250 for this cloth.", 2),
            vec!["My price is Rs. 250 for this cloth."]
        );
    }

    /// A decimal point is not a sentence end.
    #[test]
    fn does_not_split_inside_a_decimal() {
        assert_eq!(
            segments("The cloth is 2.5 metres wide and very soft.", 2),
            vec!["The cloth is 2.5 metres wide and very soft."]
        );
    }

    /// A figure at the end of a sentence still ends it, even though the
    /// character before the full stop is a digit.
    #[test]
    fn splits_after_a_sentence_ending_in_a_figure() {
        assert_eq!(
            segments("My final price is 250. Do we have a deal?", 2),
            vec!["My final price is 250.", "Do we have a deal?"]
        );
    }

    /// A control token must never be cut in half: `strip_bracket_tokens` reads
    /// an unbalanced '[' as ordinary text, so Piper would say it out loud.
    #[test]
    fn keeps_a_control_token_whole_and_silent() {
        let out = segments("Fine. Take it for 250. [DEAL]", 2);
        assert!(
            out.iter().all(|segment| !segment.contains('[')),
            "a bracket reached the synthesiser: {out:?}"
        );
        // The token is a whole segment that cleans down to nothing, so the
        // synthesiser is never asked for it.
        assert_eq!(out, vec!["Fine. Take it for 250."]);
    }

    /// A reply written as one long clause still has to start playing before the
    /// last token, so it is cut at a clause boundary.
    #[test]
    fn cuts_a_runaway_sentence_at_a_clause_boundary() {
        let text = "I went to the market early in the morning with my mother and my \
                    younger sister, and we bought vegetables and some cloth for a new \
                    shirt before the sun got too hot to walk around";
        let out = segments(text, 4);
        assert!(out.len() > 1, "a {}-char clause was never cut", text.len());
        assert!(same_words(&out.join(" "), text));
    }

    /// Micro-segments are a click, not a phrase, so a short opener is merged
    /// into the sentence after it.
    #[test]
    fn merges_a_segment_too_short_to_speak() {
        assert_eq!(
            segments("Ok. I understand what you mean now.", 2),
            vec!["Ok. I understand what you mean now."]
        );
    }

    /// An ellipsis is a pause inside a thought, not two sentences.
    #[test]
    fn treats_an_ellipsis_as_one_sentence() {
        let out = segments("Well... that is a very low offer for this cloth.", 2);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    /// An em dash is three bytes wide. Cutting one byte past it used to slice
    /// mid-character and panic, and a clause that long is exactly where the
    /// soft cap looks for a boundary.
    #[test]
    fn cuts_safely_around_a_multi_byte_clause_mark() {
        let text = "I walked all the way to the market before breakfast \u{2014} the one \
                    behind the bus stand where my aunt buys her vegetables every single \
                    morning \u{2014} and it was already completely full of people";
        let out = segments(text, 6);
        assert!(out.len() > 1, "{out:?}");
        assert!(same_words(&out.join(" "), text));
    }

    fn held(spoken: &str, live: bool) -> StreamedSpeech {
        StreamedSpeech {
            spoken: spoken.into(),
            audio: Some(SynthesizedAudio {
                audio: Some(AudioPayload {
                    mime_type: "audio/wav".into(),
                    base64: "AAAA".into(),
                }),
                first_audio_ms: Some(1.0),
                completion_ms: Some(2.0),
                words: Vec::new(),
                phonemes: Vec::new(),
                segments: 2,
            }),
            segments: 2,
            first_ready_ms: Some(1.0),
            live,
        }
    }

    /// `resolve` is the accuracy guarantee for a turn that could be
    /// regenerated: audio only survives if it says what the reply says.
    #[test]
    fn held_audio_is_only_reused_when_the_text_still_matches() {
        let (audio, played) = held("Take it for 250.", false).resolve("Take it for 250.");
        assert!(audio.is_some());
        // Held back, so the window has heard nothing and gets the recording.
        assert_eq!(played, 0);

        // Collapsed whitespace is not a change of words.
        assert!(held("Take it  for 250.", false)
            .resolve("Take it for 250.")
            .0
            .is_some());

        // The authored refusal replaced the reply: the held audio is wrong.
        assert!(held("Take it for 250.", false)
            .resolve("That is below what I paid for it.")
            .0
            .is_none());
    }

    /// Once a segment has been spoken there is nothing to take back, so live
    /// audio is always the audio for the turn.
    #[test]
    fn live_audio_is_never_discarded() {
        let (audio, played) = held("Hello there.", true).resolve("Something else entirely.");
        assert!(audio.is_some());
        assert_eq!(played, 2);
    }

    /// A stream that dies partway through must not report segments: the window
    /// would play half a reply and stop mid-thought. Reporting zero makes it
    /// drop what it has and play the complete recording instead.
    #[test]
    fn an_aborted_stream_reports_nothing_played() {
        // What the worker returns when Piper fails on a later sentence.
        let aborted = StreamedSpeech {
            spoken: String::new(),
            audio: None,
            segments: 1,
            first_ready_ms: Some(180.0),
            live: false,
        };
        let (audio, played) = aborted.resolve("Hello there. How was your day?");
        assert!(audio.is_none(), "aborted audio must not be reused");
        assert_eq!(played, 0, "the window must not think it has the whole reply");
    }

    /// The daemon's timings are only believed when they add up to its audio.
    #[test]
    fn a_daemon_alignment_is_kept_only_when_it_covers_the_audio() {
        let header = json!({ "ok": true, "alignment": [["^", 256], ["a", 512], ["$", 256]] });
        assert_eq!(
            daemon_alignment(&header, 1_024),
            vec![("^".to_string(), 256), ("a".to_string(), 512), ("$".to_string(), 256)]
        );
        assert!(daemon_alignment(&header, 1_025).is_empty(), "a sample short");
        assert!(daemon_alignment(&json!({ "alignment": [["a", -1]] }), 0).is_empty());
        assert!(daemon_alignment(&json!({ "alignment": [[3, 512]] }), 512).is_empty());
        assert!(daemon_alignment(&json!({ "ok": true }), 1_024).is_empty(), "an untimed voice");
    }

    #[test]
    fn a_wav_is_read_only_once_it_is_as_long_as_its_header_says() {
        let pcm: Vec<u8> = (0..400_i16).flat_map(i16::to_le_bytes).collect();
        let wav = raw_pcm_to_wav(&pcm, 22_050, 1);
        assert_eq!(wav_pcm(&wav), Ok(Some((pcm.clone(), 22_050))));
        for cut in [0, 11, 20, 43, 44, wav.len() - 1] {
            assert_eq!(wav_pcm(&wav[..cut]), Ok(None), "cut at {cut} bytes");
        }
        assert!(wav_pcm(b"RIFX\0\0\0\0WAVEfmt ").is_err());
        assert!(wav_pcm(&raw_pcm_to_wav(&pcm, 22_050, 2)).is_err(), "Piper's voices are mono");
    }

    /// `piper.exe --json-input`, as far as Ella can see it: a JSON line in,
    /// the WAV it names written relative to the working directory, and the
    /// name printed back. Printed here before the tail is written, which is
    /// how the real one can be caught, since it names the file before it
    /// closes it. Every sentence it says is the same `pcm`, and each start of
    /// the process adds a line to `starts`.
    #[cfg(unix)]
    struct FakePiper {
        piper: PathBuf,
        voice: PathBuf,
        pcm: Vec<u8>,
        starts: PathBuf,
    }

    #[cfg(unix)]
    fn fake_piper(root: &Path) -> FakePiper {
        use std::os::unix::fs::PermissionsExt;
        let pcm: Vec<u8> = (0..2_000_i16).flat_map(i16::to_le_bytes).collect();
        let sentence = root.join("sentence.wav");
        fs::write(&sentence, raw_pcm_to_wav(&pcm, 22_050, 1)).unwrap();
        let starts = root.join("starts");
        let piper = root.join("piper");
        fs::write(
            &piper,
            format!(
                "#!/bin/sh\n\
                 echo started >> '{starts}'\n\
                 while IFS= read -r line; do\n\
                 name=$(printf '%s' \"$line\" | sed -n 's/.*\"output_file\":\"\\([^\"]*\\)\".*/\\1/p')\n\
                 head -c 1000 '{sentence}' > \"$name\"\n\
                 echo \"$name\"\n\
                 sleep 0.05\n\
                 tail -c +1001 '{sentence}' >> \"$name\"\n\
                 done\n",
                starts = starts.display(),
                sentence = sentence.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&piper, fs::Permissions::from_mode(0o755)).unwrap();
        let voice = root.join("voice.onnx");
        fs::write(&voice, b"").unwrap();
        FakePiper { piper, voice, pcm, starts }
    }

    #[cfg(unix)]
    #[test]
    fn the_standalone_piper_stays_up_and_answers_each_line_with_its_own_wav() {
        let root = tempfile::tempdir().unwrap();
        let fake = fake_piper(root.path());
        let daemon = PiperDaemon::binary(fake.piper.clone(), fake.voice.clone());
        for text in ["Hello there.", "What did you eat today?"] {
            let speech = daemon.synthesize(text).unwrap();
            assert_eq!(speech.pcm, fake.pcm, "{text}");
            assert_eq!(speech.sample_rate, 22_050);
            assert!(speech.alignment.is_empty(), "the binary times no sounds");
        }
        assert_eq!(
            fs::read_to_string(&fake.starts).unwrap().lines().count(),
            1,
            "one process said both lines"
        );
        let prefix = format!("ella-piper-{}-", std::process::id());
        let left: Vec<_> = fs::read_dir(env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        assert!(left.is_empty(), "every WAV is removed once read: {left:?}");
    }

    #[derive(Default)]
    struct Heard(Mutex<Vec<SpeechSegment>>);

    impl SpeechSink for Heard {
        fn segment(&self, segment: SpeechSegment) {
            self.0.lock().unwrap().push(segment);
        }
    }

    const REPLY: &str = "That sounds fun. What did you eat there?";

    /// Held while it is written and checked, then released: every sentence
    /// ready by then goes at once, the rest as each is ready, all in order.
    #[cfg(unix)]
    #[test]
    fn a_held_reply_is_heard_whole_and_in_order_once_released() {
        for wait_before_release in [Duration::from_millis(600), Duration::ZERO] {
            let root = tempfile::tempdir().unwrap();
            let fake = fake_piper(root.path());
            let mut pipeline =
                SpeechPipeline::start(PiperDaemon::binary(fake.piper, fake.voice), None, Instant::now());
            pipeline.push("That sounds fun. What did you ");
            pipeline.push("eat there?");
            pipeline.close();
            assert_eq!(pipeline.queued(), REPLY);
            thread::sleep(wait_before_release);

            let pending = PendingSpeech::matching(Some(pipeline), REPLY).expect("the words match");
            let heard = Arc::new(Heard::default());
            pending.release(heard.clone());
            let audio = pending.finish().expect("the whole reply, for the replay");
            assert_eq!(audio.segments, 2, "the window has all of it");
            assert_eq!(audio.words.len(), 8);
            let heard = heard.0.lock().unwrap();
            assert_eq!(
                heard.iter().map(|segment| (segment.index, segment.text.as_str())).collect::<Vec<_>>(),
                vec![(0, "That sounds fun."), (1, "What did you eat there?")],
                "released {wait_before_release:?} after the reply was written"
            );
        }
    }

    /// Never released, because nothing was listening: the window gets the
    /// recording, and is told nothing has played.
    #[cfg(unix)]
    #[test]
    fn a_reply_nobody_released_comes_back_as_one_recording() {
        let root = tempfile::tempdir().unwrap();
        let fake = fake_piper(root.path());
        let mut pipeline =
            SpeechPipeline::start(PiperDaemon::binary(fake.piper, fake.voice), None, Instant::now());
        pipeline.push(REPLY);
        pipeline.close();
        let audio = PendingSpeech::matching(Some(pipeline), REPLY).unwrap().finish().unwrap();
        assert_eq!(audio.segments, 0);
        assert!(audio.audio.is_some());
    }

    #[test]
    fn audio_synthesized_ahead_is_kept_only_for_the_words_it_read() {
        let missing = env::temp_dir().join("ella-no-such-piper").join("piper");
        let written = |text: &str| {
            let daemon = PiperDaemon::binary(missing.clone(), missing.with_file_name("voice.onnx"));
            let mut pipeline = SpeechPipeline::start(daemon, None, Instant::now());
            pipeline.push(text);
            pipeline.close();
            pipeline
        };
        assert!(
            PendingSpeech::matching(Some(written("Take it for 250.  Final offer!")), "Take it for 250. Final offer!")
                .is_some(),
            "collapsed whitespace is not a change of words"
        );
        assert!(
            PendingSpeech::matching(Some(written("Take it for 250.")), "That is below what I paid for it.").is_none(),
            "the authored refusal replaced the reply"
        );
        assert!(PendingSpeech::matching(None, REPLY).is_none());
        // Piper could not say it at all: the caller has to say it whole.
        let broken = PendingSpeech::matching(Some(written(REPLY)), REPLY).unwrap();
        broken.release(Arc::new(Heard::default()));
        assert!(broken.finish().is_none());
    }

    /// A Piper that takes a line and never answers must not hold the turn:
    /// the wait gives up, and nothing waits on that Piper again.
    #[cfg(unix)]
    #[test]
    fn a_standalone_piper_that_stops_answering_is_given_up_on_once() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let piper = root.path().join("piper");
        fs::write(&piper, "#!/bin/sh\nwhile IFS= read -r line; do :; done\n").unwrap();
        fs::set_permissions(&piper, fs::Permissions::from_mode(0o755)).unwrap();
        let daemon =
            PiperDaemon::binary_answering_within(piper, root.path().join("voice.onnx"), Duration::from_millis(300));

        let started = Instant::now();
        assert!(daemon.synthesize("Hello.").is_err());
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(300), "it did wait: {waited:?}");
        assert!(waited < Duration::from_millis(600), "and only once: {waited:?}");
        assert!(!daemon.usable());

        let started = Instant::now();
        assert!(daemon.synthesize("Hello again.").is_err());
        assert!(started.elapsed() < Duration::from_millis(50), "set aside, so nothing is asked");
    }

    #[test]
    fn a_piper_that_cannot_start_is_set_aside_for_the_session() {
        let missing = env::temp_dir().join("ella-no-such-piper").join("piper");
        let daemon = PiperDaemon::binary(missing.clone(), missing.with_file_name("voice.onnx"));
        assert!(daemon.synthesize("Hello.").is_err());
        assert!(!daemon.usable());
        assert!(daemon.synthesize("Hello.").is_err(), "and nothing starts it again");
    }

    /// Piper's own timings, through the real daemon and the sentence stream:
    ///
    ///     ELLA_PIPER_PYTHON=<python with piper-tts> ELLA_PIPER_VOICE=<voice.onnx> \
    ///     cargo test --lib live_piper -- --ignored
    #[test]
    #[ignore = "needs piper-tts and a Piper voice"]
    fn live_piper_times_every_sound_of_a_streamed_reply() {
        struct Recorder(Mutex<Vec<SpeechSegment>>);
        impl SpeechSink for Recorder {
            fn segment(&self, segment: SpeechSegment) {
                self.0.lock().unwrap().push(segment);
            }
        }
        let python = PathBuf::from(env::var("ELLA_PIPER_PYTHON").expect("ELLA_PIPER_PYTHON"));
        let voice = PathBuf::from(env::var("ELLA_PIPER_VOICE").expect("ELLA_PIPER_VOICE"));
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let mut pipeline = SpeechPipeline::start(
            PiperDaemon::script(python, voice),
            Some(recorder.clone() as Arc<dyn SpeechSink>),
            Instant::now(),
        );
        pipeline.push("Hello there, my friend! What did you eat for breakfast today?");
        let reply = pipeline.finish().audio.expect("the reply was synthesized");
        let segments = recorder.0.lock().unwrap();
        assert_eq!(segments.len(), 2);
        let mut offset_ms = 0.0;
        for segment in segments.iter() {
            let wav = STANDARD.decode(&segment.audio.base64).unwrap();
            let clip_ms = (wav.len() - 44) as f64 / 2.0 * 1_000.0 / 22_050.0;
            let phonemes = &segment.phonemes;
            assert_eq!(phonemes.first().map(|p| p.phoneme.as_str()), Some("^"));
            assert_eq!(phonemes.last().map(|p| p.phoneme.as_str()), Some("$"));
            assert_eq!(phonemes[0].start_ms, 0.0);
            assert!((phonemes.last().unwrap().end_ms - clip_ms).abs() <= 0.05, "{clip_ms}");
            for pair in phonemes.windows(2) {
                assert_eq!(pair[0].end_ms, pair[1].start_ms, "{pair:?}");
            }
            offset_ms += clip_ms;
        }
        let per_segment: usize = segments.iter().map(|segment| segment.phonemes.len()).sum();
        assert_eq!(reply.phonemes.len(), per_segment);
        assert!((reply.phonemes.last().unwrap().end_ms - offset_ms).abs() <= 0.1);
    }
}

#[cfg(test)]
mod curriculum_prompt_tests {
    use super::*;

    fn skills() -> Vec<Scorable> {
        ["A1:U1-VOC-01", "A1:U1-GRA-01", "A1:U1-FLU-01"]
            .iter()
            .map(|key| Scorable {
                key: (*key).into(),
                text: crate::curriculum::skill_at(key).unwrap().2.text.clone(),
            })
            .collect()
    }

    #[test]
    fn a_talk_is_pitched_at_the_learners_level_and_names_its_aim_only_to_the_model() {
        let plain = ella_system_prompt("Asha", "street-food", "Street food stories", &Pitch::at("B1"));
        assert!(plain.contains("practising English at about B1 level"));
        assert!(!plain.contains("quietly aims"), "no aim, no brief");

        let aimed = ella_system_prompt(
            "Asha",
            "street-food",
            "Street food stories",
            &Pitch {
                level: "A1".into(),
                focus: Some(Focus {
                    step_title: "Talking about the past".into(),
                    step_focus: "Starting to use past tense to share what happened.".into(),
                    skill: "I can use past simple with regular verbs — I walked, I watched, I played.".into(),
                }),
            },
        );
        assert!(aimed.contains("Asha is on the step \"Talking about the past\""));
        assert!(aimed.contains("quietly aims at one skill from it: I can use past simple"));
        assert!(aimed.contains("Never name the skill, teach it or quiz them on it"));
        // The rest of the scene is untouched, so the cached prefix stays one
        // scene's for the whole session.
        assert!(aimed.contains("In this conversation you are also yourself, sitting with them over a cup of chai"));
    }

    #[test]
    fn a_chore_character_pitches_its_words_at_the_learners_level_too() {
        let chore = crate::domain::find_chore("market-cloth-price").unwrap();
        let context = ChoreContext {
            chore_id: chore.id.clone(),
            level: "B2".into(),
            character: crate::domain::find_character(&chore.character_id).unwrap(),
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger: None,
        };
        assert!(chore_system_prompt("Asha", &context).contains("at about B2 level"));
    }

    #[test]
    fn the_placement_chat_climbs_rungs_and_never_feels_like_a_test() {
        let prompt = placement_system_prompt("Asha", Some(12));
        assert!(prompt.contains("meeting Asha, a new learner in India"));
        assert!(prompt.contains("They are 12 years old"));
        assert!(prompt.contains("never a test"));
        for rung in ["First, themselves", "Then the past", "Then an opinion and why", "something longer or imagined"] {
            assert!(prompt.contains(rung), "{rung}");
        }
        assert!(prompt.contains("Never correct them"));
        assert!(!placement_system_prompt("Asha", None).contains("years old"));
        assert_eq!(placement_opening_line("Asha"), "So Asha, tell me about your day so far!");
    }

    #[test]
    fn the_placement_check_is_read_off_json_and_a_missing_confidence_is_low() {
        let read = |text: &str| json_object_in(text).as_ref().and_then(read_readiness);
        assert_eq!(
            read(r#"{"ready": true, "confidence": "High"}"#),
            Some(Readiness { ready: true, confidence: Confidence::High })
        );
        assert_eq!(
            read("```json\n{\"ready\": false}\n```"),
            Some(Readiness { ready: false, confidence: Confidence::Low })
        );
        assert_eq!(read(r#"{"ready": "yes"}"#), None, "only a real boolean counts");
        assert_eq!(read("I think so."), None);
    }

    #[test]
    fn a_placement_reads_only_a_level_the_curriculum_has() {
        let read = |text: &str| json_object_in(text).as_ref().and_then(read_placement);
        assert_eq!(
            read(r#"{"level": "b1", "closing": "\"Wonderful talking with you, Asha!\""}"#),
            Some(PlacementReading { level: "B1".into(), closing: Some("Wonderful talking with you, Asha!".into()) })
        );
        assert_eq!(read(r#"{"level": "C2", "closing": "Bye"}"#), None, "C2 is not a level here");
        assert_eq!(
            read(r#"{"level": "A0", "closing": ""}"#),
            Some(PlacementReading { level: "A0".into(), closing: None }),
            "an empty closing falls back to the app's own"
        );
    }

    #[test]
    fn a_skill_counts_only_with_a_quote_the_learner_really_said() {
        let skills = skills();
        let learner = [
            "My sister is tall and she looks very happy today.",
            "She is running to the bus stop because she is late.",
        ];
        let read = |text: &str| {
            json_object_in(text)
                .as_ref()
                .and_then(|value| read_scores(value, &skills, &learner))
        };
        let scores = read(
            r#"{"shown":[
                {"skill":1,"quote":"she looks very happy today","sure":0.8},
                {"skill":2,"quote":"She is running to the bus stop!","sure":0.9},
                {"skill":3,"quote":"I described the whole park in detail","sure":0.9}
            ]}"#,
        )
        .unwrap();
        assert_eq!(scores.len(), 2, "a quote nobody said credits nothing: {scores:?}");
        assert_eq!(scores["A1:U1-VOC-01"], 0.8);
        assert_eq!(scores["A1:U1-GRA-01"], 0.9, "punctuation and case aside");
        assert_eq!(read(r#"{"shown":[]}"#).unwrap().len(), 0, "nothing shown is an answer too");
        assert!(read(r#"{"skills":{"1":0.8}}"#).is_none(), "no list of claims, nothing read");
    }

    #[test]
    fn one_stretch_of_speech_credits_one_skill_and_a_bad_claim_is_dropped_on_its_own() {
        let skills = skills();
        let learner = [
            "When I was a child I used to play cricket every evening with my cousins.",
            "My sister is tall and she is running to school now.",
        ];
        let read = |text: &str| {
            json_object_in(text)
                .as_ref()
                .and_then(|value| read_scores(value, &skills, &learner))
                .unwrap()
        };
        let scores = read(
            r#"{"shown":[
                {"skill":3,"quote":"I used to play cricket every evening","sure":0.9},
                {"skill":1,"quote":"I used to play cricket every evening","sure":0.9},
                {"skill":2,"quote":"used to play cricket every evening with my cousins","sure":0.9},
                {"skill":2,"quote":"with my cousins","sure":1.4}
            ]}"#,
        );
        assert_eq!(scores.keys().collect::<Vec<_>>(), ["A1:U1-FLU-01"], "overlapping quotes count once: {scores:?}");

        // Two skills shown in different parts of one answer both count.
        let scores = read(
            r#"{"shown":[
                {"skill":1,"quote":"my sister is tall","sure":0.8},
                {"skill":2,"quote":"she is running to school","sure":0.9}
            ]}"#,
        );
        assert_eq!(scores.len(), 2, "{scores:?}");
    }

    #[test]
    fn a_quote_may_change_one_word_at_its_edge_but_not_become_a_paraphrase() {
        let line = comparable("The stall belongs to an old man who has been selling it for twenty years.");
        let said = vec![line.split(' ').collect::<Vec<_>>()];
        let found = |quote: &str| find_quote(&comparable(quote), &said);
        assert_eq!(found("he has been selling it for twenty years"), Some(Span { line: 0, start: 8, end: 15 }));
        assert!(found("who has been selling it for twenty").is_some());
        assert!(found("he has sold it for twenty years").is_none());
        assert!(found("old man").is_some(), "two words is a quote");
        assert!(found("selling").is_none(), "one word shows nothing");
        assert!(found("").is_none());
        assert_eq!(comparable("Don’t STOP—now!"), "dont stop now");
    }

    #[test]
    fn the_scoring_prompt_numbers_the_skills_and_asks_for_the_learners_own_words() {
        let prompt = score_prompt(&skills());
        assert!(prompt.contains("- 1: I can describe what people look like and how they feel using simple adjectives."));
        assert!(prompt.contains("- 3: "));
        assert!(prompt.contains("copy the exact words from one learner line"));
        assert!(prompt.contains("a short or simple talk shows none"));
        assert!(prompt.contains(r#"{"shown":[{"skill":<number>,"quote":"<the learner's exact words>","sure":<0 to 1>}]}"#));
        assert!(!prompt.contains("0.8}"), "no example score for a small model to copy");
    }

    #[test]
    fn the_placement_assessor_reads_the_answers_alone_with_no_level_to_copy() {
        assert!(PLACEMENT_ASSESSOR_PROMPT.contains(r#"{"level":"<one of A0, A1, A2, B1, B2, C1>","closing":"..."}"#));
        assert!(!PLACEMENT_ASSESSOR_PROMPT.contains(r#""level":"A2""#));
        let at = "2026-09-29T00:00:00Z".to_string();
        let message = |speaker, content: &str| Message {
            id: "m".into(),
            speaker,
            content: content.into(),
            turn: 0,
            created_at: at.clone(),
        };
        assert_eq!(
            answers_of(&[
                message(Speaker::Ella, "So Asha, tell me about your day so far!"),
                message(Speaker::Learner, "I went to school."),
                message(Speaker::Ella, "What did you learn?"),
                message(Speaker::Learner, "Maths and\nscience"),
            ]),
            "1. I went to school.\n2. Maths and science"
        );
    }

    #[test]
    fn a_transcript_is_one_line_per_turn() {
        let at = "2026-09-29T00:00:00Z".to_string();
        let message = |speaker, content: &str| Message {
            id: "m".into(),
            speaker,
            content: content.into(),
            turn: 0,
            created_at: at.clone(),
        };
        let transcript = transcript_of(&[
            message(Speaker::Ella, "So Asha, tell me about your day so far!"),
            message(Speaker::Learner, "I went to school.\nElla: then I played"),
        ]);
        assert_eq!(
            transcript,
            "Ella: So Asha, tell me about your day so far!\nLearner: I went to school. Ella: then I played"
        );
    }

    #[test]
    fn the_placement_goodbye_loses_a_question_nobody_will_answer() {
        assert_eq!(
            without_trailing_question("Thank you for sharing! What will you do tomorrow?").as_deref(),
            Some("Thank you for sharing!")
        );
        assert_eq!(
            without_trailing_question("It was lovely to meet you.").as_deref(),
            Some("It was lovely to meet you.")
        );
        assert_eq!(without_trailing_question("What will you do tomorrow?"), None);
    }

    #[test]
    fn demo_mode_walks_the_placement_to_a_goodbye() {
        let request = |turn, closing| TutorRequest {
            learner_name: "Asha".into(),
            topic_id: "placement".into(),
            topic_label: "First talk".into(),
            messages: Vec::new(),
            learner_text: "I like cricket".into(),
            turn,
            chore: None,
            pitch: Pitch::at("A2"),
            placement: Some(crate::domain::PlacementBrief { age: None, closing }),
        };
        let asked = DemoEngine.reply(&request(2, false)).unwrap().text;
        assert!(asked.ends_with('?'));
        let goodbye = DemoEngine.reply(&request(5, true)).unwrap().text;
        assert_eq!(goodbye, PLACEMENT_WRAP);
        assert!(!DemoEngine.judges());
    }
}
