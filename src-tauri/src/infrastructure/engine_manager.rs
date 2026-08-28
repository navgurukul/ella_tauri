//! Supervision for the one engine Ella does not run in-process.
//!
//! Canary is linked into the binary and Piper is spawned per turn, so
//! `llama-server` is the last process that a development shell script used to
//! start by hand. A packaged app has no shell script, so it starts the server
//! itself: it picks a free loopback port, waits for the model to load, keeps
//! the server's own stderr where a bug report can reach it, and kills the
//! process when Ella exits rather than leaving 2 GB of model resident.
//!
//! Failure here is reported, never fatal. If the server does not come up the
//! app still opens and says why, which is the difference between a machine we
//! can debug remotely and one that shows a blank window.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{EllaError, EllaResult};

/// How long the model is allowed to load before we call it a failure. Windows
/// Defender scans a 2 GB file the first time it is read, which is minutes on a
/// classroom laptop; the tooling notes call this out explicitly. Waiting is
/// cheap and a false failure is expensive.
const READY_TIMEOUT: Duration = Duration::from_secs(240);

/// The last few stderr lines, kept so a failed start can explain itself. The
/// server prints its real complaint (missing model, bad GGUF, port in use) and
/// nothing else in the app has that text.
const TAIL_LINES: usize = 40;

pub struct LlamaServer {
    child: Child,
    base_url: String,
    tail: Arc<Mutex<Vec<String>>>,
}

impl LlamaServer {
    /// Starts `llama-server` against the packaged model and blocks until it
    /// answers `/health`. `engine_root` holds the binaries; `models_root` holds
    /// the weights, which on a packaged install live in app data because they
    /// are downloaded rather than bundled.
    pub fn start(engine_root: &Path, models_root: &Path, threads: i32) -> EllaResult<Self> {
        let binary = llama_binary(engine_root);
        if !binary.exists() {
            return Err(EllaError::Engine(format!(
                "llama-server is missing from the installation: {}",
                binary.display()
            )));
        }
        let model = models_root.join("llm").join("model.gguf");
        if !model.exists() {
            return Err(EllaError::Engine(format!(
                "The language model has not been downloaded yet: {}",
                model.display()
            )));
        }

        let port = free_loopback_port()?;
        let base_url = format!("http://127.0.0.1:{port}/v1");

        let mut command = Command::new(&binary);
        command
            .arg("--model")
            .arg(&model)
            .args(["--host", "127.0.0.1"])
            .args(["--port", &port.to_string()])
            .args(["--ctx-size", "4096"])
            .args(["--threads", &threads.max(1).to_string()])
            // The WebView never talks to llama-server directly — Rust does —
            // but the server refuses unknown origins, and the Tauri origin is
            // what a proxied request would carry.
            .args(["--cors-origins", "tauri://localhost,http://tauri.localhost"])
            .args(["--parallel", "1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        // llama.cpp ships its backends beside the executable. Windows resolves
        // those from the binary's own directory; macOS and Linux need to be
        // told, and `current_dir` covers relative lookups on every platform.
        if let Some(directory) = binary.parent() {
            command.current_dir(directory);
            let existing = std::env::var(library_path_variable()).unwrap_or_default();
            let joined = if existing.is_empty() {
                directory.display().to_string()
            } else {
                format!("{}{}{}", directory.display(), path_separator(), existing)
            };
            command.env(library_path_variable(), joined);
        }

        let mut child = command.spawn().map_err(|reason| {
            EllaError::Engine(format!(
                "Could not start llama-server at {}: {reason}",
                binary.display()
            ))
        })?;

        let tail = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let sink = Arc::clone(&tail);
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut kept = sink.lock().expect("llama-server log mutex is not poisoned");
                    if kept.len() == TAIL_LINES {
                        kept.remove(0);
                    }
                    kept.push(line);
                }
            });
        }

        let server = Self { child, base_url, tail };
        server.wait_until_ready()?;
        Ok(server)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Polls `/health` until the model is loaded. A server that exits early —
    /// a corrupt GGUF is the usual cause — is caught on the same loop, so a
    /// dead process fails in a second rather than at the timeout.
    fn wait_until_ready(&self) -> EllaResult<()> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|reason| EllaError::Engine(format!("HTTP client setup failed: {reason}")))?;
        let health = format!("{}/health", self.base_url.trim_end_matches("/v1"));
        let started = Instant::now();

        loop {
            // `try_wait` needs &mut, and the caller holds this by value until
            // it is handed to the engine, so a short-lived raw check is enough:
            // an exited process stops answering, which the timeout below turns
            // into an error carrying the server's own last words.
            if client
                .get(&health)
                .send()
                .map(|response| response.status().is_success())
                .unwrap_or(false)
            {
                return Ok(());
            }
            if started.elapsed() > READY_TIMEOUT {
                return Err(EllaError::Engine(format!(
                    "llama-server did not become ready within {} seconds. Its last output was:\n{}",
                    READY_TIMEOUT.as_secs(),
                    self.recent_output()
                )));
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    fn recent_output(&self) -> String {
        self.tail
            .lock()
            .map(|lines| lines.join("\n"))
            .unwrap_or_else(|_| "(log unavailable)".into())
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        // A leaked llama-server holds the model in memory with no window
        // attached, and the next launch cannot bind its port.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn llama_binary(engine_root: &Path) -> PathBuf {
    let directory = engine_root.join("bin").join("llama");
    if cfg!(windows) {
        directory.join("llama-server.exe")
    } else {
        directory.join("llama-server")
    }
}

fn library_path_variable() -> &'static str {
    if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else if cfg!(windows) {
        "PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

fn path_separator() -> char {
    if cfg!(windows) {
        ';'
    } else {
        ':'
    }
}

/// Asks the OS for an unused loopback port and immediately gives it back. The
/// gap between here and llama-server binding is a race in theory; in practice
/// nothing else on a learner's machine is hunting for ephemeral ports, and a
/// fixed port is the worse bet because a stale server survives a crash.
fn free_loopback_port() -> EllaResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_is_reported_rather_than_panicking() {
        let root = std::env::temp_dir().join("ella-engine-manager-absent");
        let Err(error) = LlamaServer::start(&root, &root, 4) else {
            panic!("a missing binary must not yield a running server");
        };
        assert!(
            error.to_string().contains("llama-server is missing"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn free_ports_are_actually_free() {
        let port = free_loopback_port().unwrap();
        assert!(port > 0);
        // Binding again proves the probe released it.
        TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Deferring the engine so the window is never held hostage to a download.
// ---------------------------------------------------------------------------

use std::sync::RwLock;

use crate::domain::{ChoreContext, EngineComponent, EngineStatus, Topic, TutorRequest};
use crate::infrastructure::engines::{
    GeneratedReply, SpeechSink, SynthesizedAudio, TutorEngine,
};
use crate::infrastructure::stt::Transcription;

/// A `TutorEngine` that is not there yet.
///
/// The first launch after an install downloads ~2.3 GB of weights and then
/// waits for a 2 GB model to load. None of that can happen before the window
/// exists — a learner staring at a dead dock icon assumes the app is broken —
/// so the service starts with this, and a background thread swaps the real
/// engine in underneath it. Every call before that returns the same plain
/// sentence, and `status()` reports what the app is waiting for.
pub struct DeferredEngine {
    inner: Arc<RwLock<Option<Box<dyn TutorEngine>>>>,
    waiting_on: Arc<Mutex<String>>,
}

/// The write end, held by the thread doing the work.
#[derive(Clone)]
pub struct EngineSlot {
    inner: Arc<RwLock<Option<Box<dyn TutorEngine>>>>,
    waiting_on: Arc<Mutex<String>>,
}

impl DeferredEngine {
    pub fn new(waiting_on: &str) -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            waiting_on: Arc::new(Mutex::new(waiting_on.to_string())),
        }
    }

    pub fn slot(&self) -> EngineSlot {
        EngineSlot {
            inner: Arc::clone(&self.inner),
            waiting_on: Arc::clone(&self.waiting_on),
        }
    }

    fn pending(&self) -> EllaError {
        EllaError::Engine(self.message())
    }

    fn message(&self) -> String {
        self.waiting_on
            .lock()
            .map(|held| held.clone())
            .unwrap_or_else(|_| "Ella is still starting up.".into())
    }
}

impl EngineSlot {
    /// Report what the learner is waiting for, in words a bug report can quote.
    pub fn waiting_on(&self, message: impl Into<String>) {
        if let Ok(mut held) = self.waiting_on.lock() {
            *held = message.into();
        }
    }

    pub fn fill(&self, engine: Box<dyn TutorEngine>) {
        if let Ok(mut slot) = self.inner.write() {
            *slot = Some(engine);
        }
    }
}

impl TutorEngine for DeferredEngine {
    fn status(&self) -> EngineStatus {
        match self.inner.read() {
            Ok(slot) => match slot.as_ref() {
                Some(engine) => engine.status(),
                None => EngineStatus {
                    mode: "starting".into(),
                    label: "Getting Ella ready".into(),
                    ready: false,
                    components: vec![EngineComponent {
                        name: "Setup".into(),
                        ready: false,
                        detail: self.message(),
                    }],
                },
            },
            Err(_) => EngineStatus {
                mode: "error".into(),
                label: "Ella could not start".into(),
                ready: false,
                components: Vec::new(),
            },
        }
    }

    fn opening(&self, topic: &Topic, learner_name: &str) -> EllaResult<String> {
        match self.inner.read().ok().and_then(|slot| {
            slot.as_ref().map(|engine| engine.opening(topic, learner_name))
        }) {
            Some(result) => result,
            None => Err(self.pending()),
        }
    }

    fn opening_in_chore(&self, context: &ChoreContext, learner_name: &str) -> EllaResult<String> {
        match self.inner.read().ok().and_then(|slot| {
            slot.as_ref()
                .map(|engine| engine.opening_in_chore(context, learner_name))
        }) {
            Some(result) => result,
            None => Err(self.pending()),
        }
    }

    fn reply(&self, request: &TutorRequest) -> EllaResult<GeneratedReply> {
        match self
            .inner
            .read()
            .ok()
            .and_then(|slot| slot.as_ref().map(|engine| engine.reply(request)))
        {
            Some(result) => result,
            None => Err(self.pending()),
        }
    }

    fn uses_native_stt(&self) -> bool {
        // Claiming the browser recognizer while the native one is still
        // loading would send the WebView down a path the desktop cannot
        // serve, so an unfinished engine answers the same as a finished one.
        self.inner
            .read()
            .ok()
            .and_then(|slot| slot.as_ref().map(|engine| engine.uses_native_stt()))
            .unwrap_or(true)
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> EllaResult<Transcription> {
        match self.inner.read().ok().and_then(|slot| {
            slot.as_ref()
                .map(|engine| engine.transcribe(samples, sample_rate))
        }) {
            Some(result) => result,
            None => Err(self.pending()),
        }
    }

    fn synthesize(&self, text: &str) -> EllaResult<SynthesizedAudio> {
        match self
            .inner
            .read()
            .ok()
            .and_then(|slot| slot.as_ref().map(|engine| engine.synthesize(text)))
        {
            Some(result) => result,
            None => Err(self.pending()),
        }
    }

    fn speak(
        &self,
        text: &str,
        speech: Option<Arc<dyn SpeechSink>>,
    ) -> EllaResult<SynthesizedAudio> {
        match self.inner.read().ok().and_then(|slot| {
            slot.as_ref()
                .map(|engine| engine.speak(text, speech.clone()))
        }) {
            Some(result) => result,
            None => Err(self.pending()),
        }
    }
}
