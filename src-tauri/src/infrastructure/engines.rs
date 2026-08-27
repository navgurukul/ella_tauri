// Local tutor engines with native Canary STT and HTTP Whisper fallback.
use std::{
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::{
    domain::{
        AudioPayload, ChoreContext, Direction, EngineComponent, EngineStatus, LedgerSpec,
        Speaker, Topic, TurnSignal, TutorRequest, WordSpan,
    },
    error::{EllaError, EllaResult},
    infrastructure::{
        audio::raw_pcm_to_wav,
        speech_timing::{shifted, word_spans},
        stt::{
            CanaryStt, SpeechToTextEngine, SttRouter, Transcription, WhisperHttpStt,
            CANARY_FILE_NAME,
        },
    },
};

#[derive(Debug, Clone)]
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
    /// Whole-reply audio assembled from the sentences that were synthesized
    /// while this reply was still generating. `None` means the caller has to
    /// synthesize `text` itself, exactly as before sentence streaming existed.
    pub speech: Option<SynthesizedAudio>,
    /// How many sentences were streamed to the speaker mid-generation. Zero
    /// means the learner heard nothing until the turn returned.
    pub streamed_segments: u32,
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
            speech: None,
            streamed_segments: 0,
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
    /// How many sentences this was cut into and pushed to the sink. Zero means
    /// nothing was streamed, so the whole recording still has to be played.
    pub segments: u32,
}

const PIPER_DAEMON_SOURCE: &str = include_str!("piper_daemon.py");

struct PiperDaemonProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// Long-lived Piper synthesis process. The ONNX voice loads once (~1.4 s) and
/// every later request only pays inference (~200 ms), instead of a full
/// interpreter + voice load per turn.
pub struct PiperDaemon {
    python: PathBuf,
    voice: PathBuf,
    process: Mutex<Option<PiperDaemonProcess>>,
}

impl PiperDaemon {
    fn new(python: PathBuf, voice: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            python,
            voice,
            process: Mutex::new(None),
        })
    }

    /// Spawn and load the voice in the background so the first turn is warm.
    fn warm(self: &Arc<Self>) {
        let daemon = Arc::clone(self);
        thread::spawn(move || {
            let mut guard = daemon
                .process
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Err(error) = daemon.ensure(&mut guard) {
                eprintln!("[LATENCY]     tts> resident Piper warmup failed: {error}");
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
        let script_path = env::temp_dir().join("ella-piper-daemon.py");
        fs::write(&script_path, PIPER_DAEMON_SOURCE)?;
        let mut child = Command::new(&self.python)
            .arg(&script_path)
            .arg(&self.voice)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                EllaError::Engine(format!(
                    "Could not start the resident Piper daemon with {}: {error}",
                    self.python.display()
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EllaError::Engine("Piper daemon stdin was not captured.".into()))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| EllaError::Engine("Piper daemon stdout was not captured.".into()))?,
        );
        let mut process = PiperDaemonProcess {
            child,
            stdin,
            stdout,
        };
        let header = read_daemon_header(&mut process.stdout)?;
        if !header["ok"].as_bool().unwrap_or(false) {
            let _ = process.child.kill();
            return Err(EllaError::Engine(format!(
                "Piper daemon could not load the voice: {}",
                header["error"].as_str().unwrap_or("unknown error")
            )));
        }
        eprintln!(
            "[LATENCY]     tts> resident Piper ready in {:.0}ms (voice load {} ms)",
            started.elapsed().as_secs_f64() * 1_000.0,
            header["ready_ms"]
        );
        *guard = Some(process);
        Ok(())
    }

    /// Returns (raw PCM, sample rate, ms to response header, total ms).
    fn synthesize(&self, text: &str) -> EllaResult<(Vec<u8>, u32, f64, f64)> {
        let mut last_error = EllaError::Engine("Piper daemon unavailable".into());
        // One respawn retry covers a daemon that died between turns.
        for _attempt in 0..2 {
            let mut guard = self
                .process
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Err(error) = self.ensure(&mut guard) {
                last_error = error;
                continue;
            }
            let process = guard.as_mut().expect("ensure() leaves a live process");
            match Self::request(process, text) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if let Some(mut dead) = guard.take() {
                        let _ = dead.child.kill();
                    }
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    fn request(
        process: &mut PiperDaemonProcess,
        text: &str,
    ) -> EllaResult<(Vec<u8>, u32, f64, f64)> {
        let started = Instant::now();
        let request = serde_json::to_string(&json!({ "text": text }))?;
        process.stdin.write_all(request.as_bytes())?;
        process.stdin.write_all(b"\n")?;
        process.stdin.flush()?;
        let header = read_daemon_header(&mut process.stdout)?;
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
        process.stdout.read_exact(&mut pcm)?;
        let completion_ms = started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "[LATENCY]     tts> resident Piper synthesized {} PCM bytes (~{:.0} ms of audio) in {:.1}ms (daemon inference {} ms)",
            pcm.len(),
            pcm.len() as f64 / 2.0 / sample_rate as f64 * 1_000.0,
            completion_ms,
            header["synth_ms"]
        );
        Ok((pcm, sample_rate, first_audio_ms, completion_ms))
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
    /// True when segments were handed to the sink as they finished, so the
    /// learner has already heard them.
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

/// Synthesizes sentences on a worker thread while the reply is still being
/// generated, so Piper's ~200 ms per sentence overlaps the model's seconds of
/// decode instead of following them.
struct SpeechPipeline {
    sentences: Option<std::sync::mpsc::Sender<String>>,
    worker: Option<thread::JoinHandle<StreamedSpeech>>,
    splitter: SentenceSplitter,
}

impl SpeechPipeline {
    /// `sink` present means the learner hears each sentence as it lands. Pass
    /// `None` to synthesize ahead but hold the audio back — the right choice
    /// when the turn might still be regenerated.
    fn start(daemon: Arc<PiperDaemon>, sink: Option<Arc<dyn SpeechSink>>, started: Instant) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let live = sink.is_some();
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
            for sentence in rx {
                let text = spoken_form(&sentence);
                if !text.chars().any(char::is_alphanumeric) {
                    continue;
                }
                let (chunk, rate, _, synth_ms) = match daemon.synthesize(&text) {
                    Ok(result) => result,
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
                    "[LATENCY]     tts> segment {segments} ready at +{ready_ms:.1}ms (synth {synth_ms:.1}ms, {} chars): {text:?}",
                    text.chars().count()
                );
                if !spoken.is_empty() {
                    spoken.push(' ');
                }
                spoken.push_str(&text);
                sample_rate = Some(rate);
                let words = word_spans(&text, &chunk, rate);
                let offset_ms = pcm.len() as f64 / 2.0 * 1_000.0 / rate as f64;
                reply_words.extend(shifted(&words, offset_ms));
                if let Some(sink) = sink.as_ref() {
                    sink.segment(SpeechSegment {
                        index: segments,
                        text: text.clone(),
                        audio: AudioPayload {
                            mime_type: "audio/wav".into(),
                            base64: STANDARD.encode(raw_pcm_to_wav(&chunk, rate, 1)),
                        },
                        ready_ms,
                        words,
                    });
                }
                pcm.extend_from_slice(&chunk);
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
                segments,
            });
            StreamedSpeech {
                spoken,
                audio,
                segments,
                first_ready_ms,
                live,
            }
        });
        Self {
            sentences: Some(tx),
            worker: Some(worker),
            splitter: SentenceSplitter::default(),
        }
    }

    /// Feed one streamed token delta. Complete sentences leave for Piper now.
    fn push(&mut self, delta: &str) {
        let Some(tx) = self.sentences.as_ref() else {
            return;
        };
        for sentence in self.splitter.push(delta) {
            if tx.send(sentence).is_err() {
                // The worker gave up; stop feeding it.
                self.sentences = None;
                return;
            }
        }
    }

    /// Close the stream and wait for the last sentence to finish synthesizing.
    fn finish(mut self) -> StreamedSpeech {
        if let (Some(tx), Some(rest)) = (self.sentences.as_ref(), self.splitter.flush()) {
            let _ = tx.send(rest);
        }
        self.sentences = None;
        match self.worker.take().map(|worker| worker.join()) {
            Some(Ok(result)) => result,
            _ => StreamedSpeech {
                spoken: String::new(),
                audio: None,
                segments: 0,
                first_ready_ms: None,
                live: false,
            },
        }
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
    /// What the learner came to do. Mirrors the `Topic::prompt` they read on
    /// the topic card, so both sides of the screen want the same thing.
    goal: &'static str,
}

fn scene_for(topic_id: &str) -> Scene {
    match topic_id {
        "restaurant-order" => Scene {
            role: "the waiter at the restaurant they have just sat down in",
            goal: "order a meal, ask about the menu, and settle the bill",
        },
        "booking-a-cab" => Scene {
            role: "the cab driver they have just flagged down",
            goal: "give an address, agree a fare, and ask how long the trip takes",
        },
        "job-interview" => Scene {
            role: "the interviewer meeting them for a first interview",
            goal: "introduce themselves and answer questions about their work",
        },
        "doctor-clinic" => Scene {
            role: "the doctor at the clinic they have walked into",
            goal: "explain how they feel and understand what to do next",
        },
        "asking-directions" => Scene {
            role: "a friendly local they have stopped on the street",
            goal: "find their way and repeat the directions back to you",
        },
        "market-bargaining" => Scene {
            role: "the shopkeeper at the stall they are standing in front of",
            goal: "ask the price, bargain kindly, and agree a deal",
        },
        // Street food stories, and anything added to the catalog without a
        // scene of its own: Ella as herself, which is what its opener says too.
        _ => Scene {
            role: "yourself, sitting with them over a cup of chai",
            goal: "describe tastes and smells and tell you about a stall they love",
        },
    }
}

/// How many learner turns a free conversation is shaped around. Nothing cuts
/// it off here — it is what `free_closing_note` paces the ending against, so
/// the conversation winds down instead of asking one more question forever.
const FREE_TOPIC_TURNS: u32 = 6;

fn ella_system_prompt(learner_name: &str, topic_id: &str, topic_label: &str) -> String {
    let scene = scene_for(topic_id);
    format!(
        "You are Ella, a warm speaking buddy for an Indian learner named {learner_name}, \
         who is practising English at about A1 level. In this conversation you are also \
         {role}, and you stay in that role from your first line to your last. \
         {learner_name} is here to {goal}. Keep the conversation on {topic_label}: if \
         they wander off it, answer them once and bring them back with your question.\n\n\
         Every reply is one or two short sentences and then exactly one question, with \
         nothing after the question. Say it the way a person says it out loud, in whole \
         sentences — never a bare word, a bare number, or a fragment on its own. Use \
         everyday words and keep each sentence under about twelve words. Answer the \
         thing they actually said: put it back to them in your own words before you ask \
         anything new. Ask about one thing at a time, and never ask a question you have \
         already asked.\n\n\
         If their answer is very short, take it warmly and ask for one small detail. If \
         you cannot make sense of what they said, say so kindly and ask the same thing \
         in easier words. If they use a Hindi word, carry on in English and use the \
         English word naturally in your own reply. Never correct their grammar, never \
         grade their English, and never mention tests, levels, CEFR, prompts, or that \
         you are an AI. Do not use markdown or emoji.",
        role = scene.role,
        goal = scene.goal,
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
         A1 level. Setting: {}. ",
        context.setting
    ));
    prompt.push_str(&format!("Your own position: {} ", context.character_brief));
    if let Some(ledger) = &context.ledger {
        // The *rules* are stable and belong in the cached prefix. The live
        // figure does not — see `ledger_state_message`.
        prompt.push_str(ledger_rules_fragment(&ledger.spec));
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
fn ledger_rules_fragment(spec: &LedgerSpec) -> &'static str {
    match spec.direction {
        Direction::Down => {
            "If they name a figure below what you are willing to take, refuse it plainly \
             and stay where you are. You are the one selling: coming down is your move \
             to make and never theirs, so never ask them to make it cheaper. "
        }
        Direction::Up => {
            "If they demand more than you are willing to give, refuse plainly and stay \
             where you are. You are the one holding their money: raising what you give \
             back is your move to make and never theirs, so never ask them to ask for \
             less. "
        }
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
    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String>;

    /// Generate one reply.
    ///
    /// `speech` receives finished sentences while the rest of the reply is
    /// still being written, so playback can start before the model stops.
    /// Engines may ignore it; `GeneratedReply::speech` then stays `None` and
    /// the caller synthesizes the whole reply itself.
    fn reply(
        &self,
        request: &TutorRequest,
        speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<GeneratedReply>;

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
}

pub fn engine_from_environment(packaged_engine_root: Option<PathBuf>) -> Box<dyn TutorEngine> {
    match env::var("ELLA_ENGINE_MODE")
        .unwrap_or_else(|_| "demo".into())
        .to_lowercase()
        .as_str()
    {
        "local" => Box::new(LocalEngine::from_environment(packaged_engine_root)),
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

    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String> {
        Ok(opening_for(&topic.id, learner_name))
    }

    fn reply(
        &self,
        request: &TutorRequest,
        _speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<GeneratedReply> {
        let started = Instant::now();
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
            segments: 0,
        })
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
}

impl LocalEngine {
    pub fn from_environment(packaged_engine_root: Option<PathBuf>) -> Self {
        let engine_root = resolve_engine_root(packaged_engine_root);
        let stt_base_url =
            env::var("ELLA_STT_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:39092".into());
        let stt_root = stt_base_url.trim_end_matches('/').trim_end_matches("/v1");
        let stt_transcribe_url =
            env::var("ELLA_STT_TRANSCRIBE_URL").unwrap_or_else(|_| format!("{stt_root}/inference"));
        let canary_path = env::var("ELLA_CANARY_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|_| engine_root.join("models/stt").join(CANARY_FILE_NAME));
        let canary_threads = env_i32("ELLA_STT_THREADS", 0);
        let verify_checksum = env::var("ELLA_CANARY_VERIFY_SHA256")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let canary: Box<dyn SpeechToTextEngine> =
            Box::new(CanaryStt::new(canary_path, canary_threads, verify_checksum));
        let whisper = || {
            Box::new(WhisperHttpStt::new(
                stt_base_url.clone(),
                stt_transcribe_url.clone(),
            )) as Box<dyn SpeechToTextEngine>
        };
        let stt = if env::var("ELLA_STT_ENGINE")
            .unwrap_or_else(|_| "canary".into())
            .eq_ignore_ascii_case("whisper")
        {
            SttRouter::new(whisper(), None)
        } else {
            let fallback = env::var("ELLA_STT_FALLBACK")
                .unwrap_or_else(|_| "whisper".into())
                .eq_ignore_ascii_case("whisper")
                .then(whisper);
            SttRouter::new(canary, fallback)
        };

        let piper_binary = env::var("ELLA_PIPER_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_piper_binary(&engine_root));
        let piper_voice = env::var("ELLA_PIPER_VOICE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| engine_root.join("models/tts/en_US-lessac-medium.onnx"));
        let daemon_enabled = env::var("ELLA_PIPER_DAEMON")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let piper_daemon = daemon_enabled
            .then(|| piper_python_interpreter(&piper_binary))
            .flatten()
            .map(|python| PiperDaemon::new(python, piper_voice.clone()));
        if let Some(daemon) = &piper_daemon {
            daemon.warm();
        }

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client configuration is valid"),
            llm_base_url: env::var("ELLA_LLM_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:39091/v1".into()),
            llm_slot: env_i32("ELLA_LLM_SLOT", 0),
            stt,
            piper_binary,
            piper_voice,
            piper_daemon,
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

    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String> {
        let text = opening_for(&topic.id, learner_name);
        self.warm_prompt_cache(
            ella_system_prompt(learner_name, &topic.id, &topic.label),
            text.clone(),
        );
        Ok(text)
    }

    fn opening_in_chore(&self, context: &ChoreContext, learner_name: &str) -> EllaResult<String> {
        let text = chore_opening_for(&context.chore_id)
            .map(str::to_owned)
            .unwrap_or_else(|| fallback_opening(context));
        self.warm_prompt_cache(chore_system_prompt(learner_name, context), text.clone());
        Ok(text)
    }

    fn reply(
        &self,
        request: &TutorRequest,
        speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<GeneratedReply> {
        let system = match &request.chore {
            Some(context) => chore_system_prompt(&request.learner_name, context),
            None => ella_system_prompt(
                &request.learner_name,
                &request.topic_id,
                &request.topic_label,
            ),
        };
        let mut history = self.history_messages(request);
        // Everything that moves between turns goes here, after the whole
        // cached conversation, so the prefix llama.cpp has already evaluated
        // stays byte-identical from the first turn to the last.
        let turn_note = match &request.chore {
            Some(context) => chore_turn_message(context, request.turn),
            None => free_closing_note(request.turn),
        };
        if let Some(note) = turn_note {
            history.insert(
                history.len().saturating_sub(1),
                json!({"role": "system", "content": note}),
            );
        }

        // A ledger turn can be thrown away and generated again, so nothing from
        // it may reach the speaker before the figure has been checked. Every
        // other turn is final the moment it is written, so its sentences can be
        // spoken as they arrive. Sentence streaming needs the resident Piper:
        // one-shot Piper reloads the voice per call, which would cost more per
        // sentence than the whole reply saves.
        let retractable = request
            .chore
            .as_ref()
            .and_then(|context| context.ledger.as_ref())
            .is_some();
        let generation_started = Instant::now();
        let mut pipeline = self.piper_daemon.as_ref().map(|daemon| {
            SpeechPipeline::start(
                Arc::clone(daemon),
                if retractable { None } else { speech },
                generation_started,
            )
        });

        let first = self.stream_once(&system, &history, None, pipeline.as_mut())?;
        // Only the first generation feeds the speaker; a regenerated reply
        // synthesizes from scratch below.
        let streamed = pipeline.take().map(SpeechPipeline::finish);
        if let Some(speech) = streamed.as_ref() {
            if let Some(first_ready_ms) = speech.first_ready_ms {
                eprintln!(
                    "[LATENCY]     tts> first sentence audible {:.1}ms into a {:.1}ms generation ({} segments)",
                    first_ready_ms, first.completion_ms, speech.segments
                );
            }
        }
        let (text, signal) = take_signal(&first.text);

        // Free conversation: nothing to enforce, nothing to extract.
        let Some(ledger) = request.chore.as_ref().and_then(|c| c.ledger.as_ref()) else {
            let (speech, streamed_segments) =
                streamed.map_or((None, 0), |speech| speech.resolve(&text));
            return Ok(GeneratedReply {
                text,
                ttft_ms: first.ttft_ms,
                completion_ms: first.completion_ms,
                named_figure: None,
                signal,
                regenerated: false,
                speech,
                streamed_segments,
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
            // authored line for a wordless turn, which `resolve` catches. A
            // ledger turn never streams, so the count is always zero.
            let (speech, _) = streamed.map_or((None, 0), |speech| speech.resolve(&final_text));
            return Ok(GeneratedReply {
                speech,
                text: final_text,
                named_figure: figure,
                signal,
                regenerated: false,
                ttft_ms: first.ttft_ms,
                completion_ms: first.completion_ms,
                streamed_segments: 0,
            });
        }
        drop(streamed);

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
                speech: None,
                streamed_segments: 0,
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
            speech: None,
            streamed_segments: 0,
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
                segments: 0,
            });
        }
        if let Some(daemon) = &self.piper_daemon {
            eprintln!(
                "[LATENCY]     tts> asking resident Piper for {} chars of text",
                text.chars().count()
            );
            match daemon.synthesize(text) {
                Ok((pcm, sample_rate, first_audio_ms, completion_ms)) => {
                    let words = word_spans(text, &pcm, sample_rate);
                    let encode_started = Instant::now();
                    let base64 = STANDARD.encode(raw_pcm_to_wav(&pcm, sample_rate, 1));
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
                        first_audio_ms: Some(first_audio_ms),
                        completion_ms: Some(completion_ms),
                        words,
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
        let Some(daemon) = self.piper_daemon.as_ref() else {
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
    fn warm_prompt_cache(&self, system: String, opening: String) {
        let client = self.client.clone();
        let url = format!(
            "{}/chat/completions",
            self.llm_base_url.trim_end_matches('/')
        );
        let slot = self.llm_slot;
        thread::spawn(move || {
            let started = Instant::now();
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
                Ok(response) => eprintln!(
                    "[LATENCY]     llm> session prompt-cache warmup took {:.0}ms (status {})",
                    started.elapsed().as_secs_f64() * 1_000.0,
                    response.status()
                ),
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
        let mut messages = Vec::new();
        for message in &request.messages {
            messages.push(json!({
                "role": if message.speaker == Speaker::Ella { "assistant" } else { "user" },
                "content": message.content,
            }));
        }
        messages.push(json!({"role": "user", "content": request.learner_text}));
        messages
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
        let started = Instant::now();
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
                "max_tokens": 90,
                // The deposit bench run said "<echo them>, then. I'll give you
                // Rs N back." four turns running. "Do not reuse a sentence you
                // have already said" is in the prompt and does nothing; the
                // penalties reach the repetition the prompt cannot.
                "presence_penalty": 0.4,
                "frequency_penalty": 0.3,
                "stream": true,
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
        let mut child = Command::new(&self.piper_binary)
            .arg("--model")
            .arg(&self.piper_voice)
            .arg("--output_raw")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
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
            segments: 0,
        })
    }
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

/// The resident daemon needs the Piper venv's Python. Returns None for the
/// standalone C++ piper binary, which keeps the one-shot path.
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
            }, None)
            .unwrap();
        assert!(reply.text.ends_with('?'));
        assert!(reply.text.len() < 180);
    }

    #[test]
    #[ignore = "requires Canary, llama.cpp, Whisper fallback, and Piper development engines"]
    fn local_engine_runs_speech_to_speech_vertical_slice() {
        let engine = LocalEngine::from_environment(None);
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
            }, None)
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
            setting: chore.setting,
            learner_goal: chore.learner_goal,
            character_brief: chore.character_brief,
            max_turns: chore.max_turns,
            ledger,
        }
    }

    #[test]
    fn a_free_conversation_is_told_the_scene_its_opener_walked_into() {
        // `opening_for` says "I am your waiter". Before the two were tied
        // together the very next turn came back as a speaking buddy talking
        // about restaurants in the abstract.
        let opener = opening_for("restaurant-order", "Asha");
        assert!(opener.contains("waiter"));
        let prompt = ella_system_prompt("Asha", "restaurant-order", "Ordering at a restaurant");
        assert!(prompt.contains("waiter"), "the prompt drops the role the opener promised");
        assert!(prompt.contains("Asha"));
        assert!(prompt.contains("Ordering at a restaurant"), "the topic has to be named");
        assert!(
            prompt.contains("settle the bill"),
            "the goal on the learner's topic card is what the conversation is for"
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
}
