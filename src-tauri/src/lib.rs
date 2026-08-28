pub mod application;
pub mod domain;
pub mod error;
pub mod infrastructure;
mod ipc;
mod telemetry;

use std::sync::Arc;
use std::thread;

use application::{AppService, SpeechBroadcast};
use domain::SpeechStreamEvent;
use infrastructure::{
    database::Database,
    engine_manager::{DeferredEngine, EngineSlot},
    engines::{engine_from_environment, resolved_mode, EnginePaths},
    models,
};
use ipc::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// The event name the window listens on for mid-turn speech.
pub const SPEECH_STREAM_EVENT: &str = "ella://speech-segment";

/// Setup progress: model downloads on a first run, then the model load.
pub const SETUP_EVENT: &str = "ella://setup";

#[derive(Clone, Serialize)]
struct SetupProgress {
    /// `downloading`, `loading`, `ready` or `failed`.
    stage: String,
    message: String,
    downloaded_bytes: u64,
    total_bytes: u64,
    /// Which file of how many, for a first run that fetches more than one.
    index: usize,
    of: usize,
    /// 1 on the first try; above that the transfer is being retried.
    attempt: u32,
}

/// Fetch whatever weights this build needs, then bring the local engine up and
/// hand it to the service.
///
/// Everything here is reported to the window and nothing here panics: a failed
/// download or a model that will not load leaves the app open and explaining
/// itself, which is what a tester on another machine can act on.
fn prepare_engine(
    window: AppHandle,
    slot: EngineSlot,
    paths: EnginePaths,
    models_root: std::path::PathBuf,
) {
    let announce = |progress: SetupProgress| {
        let _ = window.emit(SETUP_EVENT, &progress);
    };

    match models::outstanding(&models_root) {
        Ok(work) if !work.is_empty() => {
            let total: u64 = work.iter().map(|spec| spec.approximate_bytes).sum();
            slot.waiting_on(format!(
                "Ella is downloading her voice and language models ({} MB). This happens once.",
                total / (1024 * 1024)
            ));
            let mut report = |progress: models::ModelProgress| {
                announce(SetupProgress {
                    stage: "downloading".into(),
                    message: format!("Downloading {} ({} of {})", progress.key, progress.index, progress.of),
                    downloaded_bytes: progress.downloaded_bytes,
                    total_bytes: progress.total_bytes,
                    index: progress.index,
                    of: progress.of,
                    attempt: progress.attempt,
                });
            };
            if let Err(reason) = models::ensure(&models_root, &mut report) {
                // An interrupted download resumes on the next launch, so this
                // is a setback rather than a dead install.
                slot.waiting_on(format!("Ella could not finish downloading her models: {reason}"));
                announce(SetupProgress {
                    stage: "failed".into(),
                    message: reason.to_string(),
                    downloaded_bytes: 0,
                    total_bytes: 0,
                    index: 0,
                    of: 0,
                    attempt: 1,
                });
                return;
            }
        }
        Ok(_) => {}
        Err(reason) => eprintln!("[setup] model manifest unreadable: {reason}"),
    }

    slot.waiting_on("Ella is loading her language model.");
    announce(SetupProgress {
        stage: "loading".into(),
        message: "Loading the language model".into(),
        downloaded_bytes: 0,
        total_bytes: 0,
        index: 0,
        of: 0,
        attempt: 1,
    });

    let engine = engine_from_environment(paths);
    let status = engine.status();
    slot.fill(engine);
    announce(SetupProgress {
        stage: if status.ready { "ready".into() } else { "failed".into() },
        message: status.label,
        downloaded_bytes: 0,
        total_bytes: 0,
        index: 0,
        of: 0,
        attempt: 1,
    });
}

/// Pushes each synthesized sentence to the window the moment Piper finishes it,
/// so playback starts while the model is still writing the rest of the reply.
struct WindowSpeech(AppHandle);

impl SpeechBroadcast for WindowSpeech {
    fn speak(&self, event: SpeechStreamEvent) {
        if let Err(error) = self.0.emit(SPEECH_STREAM_EVENT, &event) {
            // Losing a segment costs this sentence's early playback, nothing
            // else: the whole reply still arrives with the turn result.
            eprintln!("[LATENCY]     tts> could not push speech segment: {error}");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Both plugins exist for one flow: check for a signed update, install
        // it, restart into it.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let packaged_engine_root = app
                .path()
                .resource_dir()
                .ok()
                .map(|directory| directory.join("engines"));
            std::fs::create_dir_all(&data_dir)?;
            // Latency/error events and Canary failure audio outlive the
            // session so improvements can be reviewed over time.
            telemetry::persist_to(data_dir.join("telemetry"));
            if std::env::var_os("ELLA_STT_DEBUG_DIR").is_none() {
                std::env::set_var("ELLA_STT_DEBUG_DIR", data_dir.join("stt-failures"));
            }
            let database = Database::open(&data_dir.join("ella.sqlite3"))?;
            // Weights are downloaded rather than bundled, so they live beside
            // the database in app data — the one directory an installed build
            // may write to.
            let models_root = data_dir.join("models");
            let paths = EnginePaths {
                engine_root: packaged_engine_root,
                models_root: Some(models_root.clone()),
            };

            let service = if resolved_mode(&paths) == "local" {
                // First run has 2.3 GB to fetch and a 2 GB model to load. The
                // window opens now and the engine arrives underneath it.
                let deferred = DeferredEngine::new("Ella is getting set up.");
                let slot = deferred.slot();
                let service = Arc::new(AppService::new(database, Box::new(deferred)));
                let window = app.handle().clone();
                thread::spawn(move || prepare_engine(window, slot, paths, models_root));
                service
            } else {
                Arc::new(AppService::new(database, engine_from_environment(paths)))
            };
            service.set_speech_broadcast(Arc::new(WindowSpeech(app.handle().clone())));
            app.manage(AppState(service));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::bootstrap,
            ipc::save_learner,
            ipc::start_session,
            ipc::start_chore,
            ipc::speak_opening,
            ipc::get_session,
            ipc::send_text_turn,
            ipc::send_voice_turn,
            ipc::begin_voice_stream,
            ipc::push_voice_stream,
            ipc::cancel_voice_stream,
            ipc::finish_voice_stream_turn,
            ipc::complete_session,
            ipc::reset_demo_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Ella");
}
