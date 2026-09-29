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
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{EllaError, EllaResult};

/// How long the server may go without answering at all before we call it a
/// failure. It answers, with a 503, as soon as it is listening, and keeps
/// answering while the model loads.
const READY_TIMEOUT: Duration = Duration::from_secs(240);

/// How long a server that is answering "still loading" may take. Windows
/// Defender scans a 2 GB file the first time it is read, which is minutes on a
/// classroom laptop; the tooling notes call this out explicitly. The learner
/// is on the setup screen for all of it, and a false failure there costs them
/// the whole load again, so this is generous.
const LOADING_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// The last few stderr lines, kept so a failed start can explain itself. The
/// server prints its real complaint (missing model, bad GGUF, port in use) and
/// nothing else in the app has that text.
const TAIL_LINES: usize = 40;

pub struct LlamaServer {
    child: Arc<Mutex<Child>>,
    base_url: String,
    tail: Arc<Mutex<Vec<String>>>,
}

/// Every server this process has started and not yet stopped. Until `start`
/// returns, a server belongs to the thread that is waiting on it, where
/// nothing that runs at exit can reach it; a learner who closes Ella during a
/// long first load would otherwise leave 2 GB resident, holding the files an
/// update needs to replace, and the next launch would start a second one.
static SERVERS: Mutex<Vec<Weak<Mutex<Child>>>> = Mutex::new(Vec::new());

/// Kills every server still running, including one that is still loading.
/// For exit only: whatever was waiting on one gets an error back.
pub fn stop_all_servers() {
    let servers: Vec<_> = SERVERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .filter_map(|server| server.upgrade())
        .collect();
    for server in servers {
        let mut child = server.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
    }
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

        // Without this, Windows pops a visible console window for
        // llama-server every time it starts, since our own GUI process has
        // none of its own for the child to inherit.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
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

        let child = Arc::new(Mutex::new(child));
        {
            let mut servers = SERVERS.lock().unwrap_or_else(PoisonError::into_inner);
            servers.retain(|server| server.strong_count() > 0);
            servers.push(Arc::downgrade(&child));
        }
        // From here a failure drops `server`, which kills the process.
        let server = Self { child, base_url, tail };
        server.wait_until_ready()?;
        Ok(server)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Polls `/health` until the model is loaded. A server that exits early —
    /// a missing DLL, a CPU it cannot run on, a corrupt GGUF — is caught on
    /// the same loop, so a dead process fails in a moment rather than at the
    /// timeout, with the server's own last words.
    fn wait_until_ready(&self) -> EllaResult<()> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|reason| EllaError::Engine(format!("HTTP client setup failed: {reason}")))?;
        let health = format!("{}/health", self.base_url.trim_end_matches("/v1"));
        let started = Instant::now();
        // Any answer, even "503 loading", means the server is alive and
        // working through the model, which earns it the longer wait.
        let mut answering = false;

        loop {
            match client.get(&health).send() {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(_) => answering = true,
                Err(_) => {}
            }
            let exited = self
                .child
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_wait()
                .ok()
                .flatten();
            if let Some(status) = exited {
                // Its stderr reader may still be draining the last lines.
                thread::sleep(Duration::from_millis(100));
                return Err(EllaError::Engine(format!(
                    "llama-server stopped before it was ready ({status}). Its last output was:\n{}",
                    self.recent_output()
                )));
            }
            let limit = if answering { LOADING_TIMEOUT } else { READY_TIMEOUT };
            if started.elapsed() > limit {
                return Err(EllaError::Engine(format!(
                    "llama-server did not become ready within {} seconds. Its last output was:\n{}",
                    limit.as_secs(),
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
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
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

    /// `stop_all_servers` reaches every server in the process, so the tests
    /// that start one take turns.
    static SERVER_TESTS: Mutex<()> = Mutex::new(());

    /// A stand-in llama-server: a shell script at the path `start` looks
    /// for, beside a dummy model.
    #[cfg(unix)]
    fn fake_server(name: &str, script: &str) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::Builder::new().prefix(name).tempdir().unwrap();
        let binary = llama_binary(root.path());
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(root.path().join("llm")).unwrap();
        std::fs::write(root.path().join("llm").join("model.gguf"), b"not a model").unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn a_server_that_dies_fails_at_once_with_its_own_words() {
        let _turn = SERVER_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
        let root = fake_server("ella-dead-server", "echo 'cannot load: bad magic' >&2; exit 1");
        let started = Instant::now();
        let Err(error) = LlamaServer::start(root.path(), root.path(), 1) else {
            panic!("a server that exits must not count as started");
        };
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
        let text = error.to_string();
        assert!(text.contains("stopped before it was ready"), "unexpected error: {text}");
        assert!(text.contains("bad magic"), "the server's own output is kept: {text}");
    }

    #[cfg(unix)]
    #[test]
    fn exit_stops_a_server_that_is_still_loading() {
        let _turn = SERVER_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
        // Never listens, never exits: a model load that has not finished.
        let root = fake_server("ella-slow-server", "exec sleep 60");
        let path = root.path().to_path_buf();
        let loading = thread::spawn(move || LlamaServer::start(&path, &path, 1).map(|_| ()));
        thread::sleep(Duration::from_millis(600));

        let started = Instant::now();
        stop_all_servers();
        let outcome = loading.join().unwrap();

        assert!(outcome.is_err(), "the thread waiting on it hears that it stopped");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn an_engine_that_arrives_after_shutdown_is_dropped_not_kept() {
        struct Counted(Arc<std::sync::atomic::AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        impl TutorEngine for Counted {
            fn status(&self) -> EngineStatus {
                EngineStatus { mode: "test".into(), label: "test".into(), ready: true, components: Vec::new() }
            }
            fn opening(&self, _: &Topic, _: &str) -> EllaResult<String> {
                Ok(String::new())
            }
            fn reply(&self, _: &TutorRequest) -> EllaResult<GeneratedReply> {
                unreachable!()
            }
            fn uses_native_stt(&self) -> bool {
                true
            }
            fn transcribe(&self, _: &[i16], _: u32) -> EllaResult<Transcription> {
                unreachable!()
            }
            fn synthesize(&self, _: &str) -> EllaResult<SynthesizedAudio> {
                unreachable!()
            }
        }

        // `shutdown` stops every server in the process.
        let _turn = SERVER_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let deferred = DeferredEngine::new("waiting");
        let slot = deferred.slot();
        deferred.shutdown();

        assert!(!slot.fill(Box::new(Counted(Arc::clone(&dropped)))));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert_eq!(deferred.status().mode, "starting", "nothing was put in the slot");
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

use std::sync::atomic::{AtomicBool, Ordering};
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
    closed: Arc<AtomicBool>,
}

/// The write end, held by the thread doing the work.
#[derive(Clone)]
pub struct EngineSlot {
    inner: Arc<RwLock<Option<Box<dyn TutorEngine>>>>,
    waiting_on: Arc<Mutex<String>>,
    /// Set by `shutdown`. An engine that finishes loading after it has
    /// nothing left to drain it, so it is dropped as it arrives.
    closed: Arc<AtomicBool>,
}

impl DeferredEngine {
    pub fn new(waiting_on: &str) -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            waiting_on: Arc::new(Mutex::new(waiting_on.to_string())),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn slot(&self) -> EngineSlot {
        EngineSlot {
            inner: Arc::clone(&self.inner),
            waiting_on: Arc::clone(&self.waiting_on),
            closed: Arc::clone(&self.closed),
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

    /// Hands the engine over. False when Ella is already closing, in which
    /// case the engine is dropped here, and its llama-server with it.
    pub fn fill(&self, engine: Box<dyn TutorEngine>) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        let replaced = self
            .inner
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(engine);
        // Dropped outside the lock, so an engine that panics on its way out
        // cannot poison the slot for the one that replaced it.
        drop(replaced);
        true
    }

    /// Ella is closing: an engine still being built is dropped by its builder
    /// as soon as it can, rather than handed over.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Take out an engine that never became ready, before setup tries again,
    /// so the retry's model is not loaded beside the failed one's.
    pub fn clear(&self, waiting_on: impl Into<String>) {
        self.waiting_on(waiting_on);
        let taken = self.inner.write().unwrap_or_else(PoisonError::into_inner).take();
        drop(taken);
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

    /// Drops the real engine while the process can still clean up after it.
    ///
    /// Tauri exits without dropping managed state, so without this the engine
    /// outlives every destructor that matters: `LlamaServer`'s never runs and
    /// llama-server stays resident with its 2 GB model, and Canary still holds
    /// Metal buffers when ggml's static device teardown asserts they are all
    /// gone — which aborts, and macOS reports every quit as a crash.
    ///
    /// Bounded, because a turn in flight holds the read lock and a quit must
    /// not hang on a slow reply; if the lock never frees, exit goes ahead
    /// exactly as it did before.
    ///
    /// A server still loading belongs to the setup thread rather than the
    /// slot, so it is stopped separately. That thread then gets an error back
    /// and, seeing the slot closed, drops what it built; exit gives it the
    /// moment that takes (`Setup::wait_while_loading`).
    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(mut slot) = self.inner.try_write() {
                drop(slot.take());
                break;
            }
            if Instant::now() >= deadline {
                eprintln!("[engines] shutdown: engine still busy, exiting without releasing it");
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        stop_all_servers();
    }
}
