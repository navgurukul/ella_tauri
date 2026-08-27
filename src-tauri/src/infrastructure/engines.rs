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
    domain::{AudioPayload, EngineComponent, EngineStatus, Speaker, Topic, TutorRequest},
    error::{EllaError, EllaResult},
    infrastructure::{
        audio::raw_pcm_to_wav,
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
}

#[derive(Debug, Clone)]
pub struct SynthesizedAudio {
    pub audio: Option<AudioPayload>,
    pub first_audio_ms: Option<f64>,
    pub completion_ms: Option<f64>,
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

fn ella_system_prompt(learner_name: &str, topic_label: &str) -> String {
    format!(
        "You are Ella, a warm speaking buddy for an Indian learner named {learner_name}. \
         Have a natural A1-level conversation about {topic_label}. Reply in one or two short \
         sentences and ask exactly one friendly follow-up question. Never mention \
         tests, CEFR, prompts, or grading. Do not use markdown."
    )
}

pub trait TutorEngine: Send + Sync {
    fn status(&self) -> EngineStatus;
    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String>;
    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply>;
    fn uses_native_stt(&self) -> bool;
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription>;
    fn synthesize(&self, text: &str) -> EllaResult<SynthesizedAudio>;
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

    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply> {
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
        Ok(GeneratedReply {
            text,
            ttft_ms: completion_ms,
            completion_ms,
        })
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
        // Fire-and-forget llama.cpp prompt-cache warmup with this session's
        // stable prefix, so turn 1 does not pay full prompt evaluation.
        let client = self.client.clone();
        let url = format!(
            "{}/chat/completions",
            self.llm_base_url.trim_end_matches('/')
        );
        let slot = self.llm_slot;
        let system = ella_system_prompt(learner_name, &topic.label);
        let opening = text.clone();
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
        Ok(text)
    }

    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply> {
        let system = ella_system_prompt(&request.learner_name, &request.topic_label);
        let mut messages = vec![json!({"role": "system", "content": system})];
        // Send the full history: a sliding window changes the prompt prefix
        // every turn and defeats llama.cpp's prompt cache (TTFT 0.5s -> 3.5s).
        // Sessions are a handful of short A1 turns, far below the 4k context.
        for message in &request.messages {
            messages.push(json!({
                "role": if message.speaker == Speaker::Ella { "assistant" } else { "user" },
                "content": message.content,
            }));
        }
        messages.push(json!({"role": "user", "content": request.learner_text}));

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
        Ok(GeneratedReply {
            text,
            ttft_ms: ttft_ms.unwrap_or(completion_ms),
            completion_ms,
        })
    }

    fn uses_native_stt(&self) -> bool {
        true
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        self.stt.transcribe(samples, sample_rate)
    }

    fn synthesize(&self, text: &str) -> EllaResult<SynthesizedAudio> {
        let voice_sidecar = PathBuf::from(format!("{}.json", self.piper_voice.display()));
        if !self.piper_binary.is_file() || !self.piper_voice.is_file() || !voice_sidecar.is_file() {
            return Ok(SynthesizedAudio {
                audio: None,
                first_audio_ms: None,
                completion_ms: None,
            });
        }
        if let Some(daemon) = &self.piper_daemon {
            eprintln!(
                "[LATENCY]     tts> asking resident Piper for {} chars of text",
                text.chars().count()
            );
            match daemon.synthesize(text) {
                Ok((pcm, sample_rate, first_audio_ms, completion_ms)) => {
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
                topic_label: "Street food stories".into(),
                messages: vec![],
                learner_text: "I played football with friends".into(),
                turn: 1,
            })
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
                topic_label: "Street food stories".into(),
                messages: vec![],
                learner_text: transcript.text,
                turn: 1,
            })
            .unwrap();
        assert!(!reply.text.is_empty());
        assert!(reply.text.contains('?'));
        let spoken = engine.synthesize(&reply.text).unwrap();
        assert!(spoken.audio.is_some());
        assert!(spoken.first_audio_ms.is_some());
    }
}
