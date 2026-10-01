pub mod application;
pub mod curriculum;
pub mod domain;
pub mod error;
pub mod infrastructure;
mod ipc;
pub mod notes;
pub mod progress;
mod setup;
mod telemetry;
mod window_fit;

use std::sync::Arc;

use application::{AppService, SpeechBroadcast};
use domain::SpeechStreamEvent;
use infrastructure::{
    database::Database,
    engine_manager::DeferredEngine,
    engines::{engine_from_environment, resolved_mode, EnginePaths},
};
use ipc::{AppState, SetupState};
use setup::{Setup, SetupProgress, SetupSink};
use tauri::{AppHandle, Emitter, Manager};

/// The event name the window listens on for mid-turn speech.
pub const SPEECH_STREAM_EVENT: &str = "ella://speech-segment";

/// Setup progress: model downloads on a first run, then the model load. See
/// `setup` for why the window also asks for it.
pub const SETUP_EVENT: &str = "ella://setup";

/// Sends each setup announcement to the window as it happens. The window
/// also asks for the latest one when it opens, so a lost event costs nothing.
struct WindowSetup(AppHandle);

impl SetupSink for WindowSetup {
    fn announce(&self, progress: &SetupProgress) {
        let _ = self.0.emit(SETUP_EVENT, progress);
    }
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

/// Shuts the engine down, and folds the database's log into `ella.sqlite3`,
/// when Tauri clears the app's resource table.
///
/// On Windows the updater runs the installer and then calls `process::exit`
/// itself, which skips `RunEvent::Exit`; the only thing it does first is
/// `cleanup_before_exit`, which drops every entry in this table. Without the
/// guard, llama-server would outlive the app and still hold the files the
/// installer is trying to replace.
struct EngineShutdownGuard(Arc<AppService>);

impl tauri::Resource for EngineShutdownGuard {}

impl Drop for EngineShutdownGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Both plugins exist for one flow: download a signed update, install
        // it, restart into it.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // The running build's version in the title bar, where a tester can
            // read it off a screenshot. Taken from the bundle rather than
            // written into tauri.conf.json, so it follows an update.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&format!(
                    "Ella {} — Speak English every day",
                    app.package_info().version
                ));
                if let Err(error) = window_fit::fit_to_screen(&window) {
                    eprintln!("[window] could not fit the window to the screen: {error}");
                }
                window_fit::zoom_to_window(&window);
            }
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

            let sink = Box::new(WindowSetup(app.handle().clone()));
            let (service, setup) = if resolved_mode(&paths) == "local" {
                // First run has 2.3 GB to fetch and a 2 GB model to load. The
                // window opens now, on the setup screen, and the engine
                // arrives underneath it.
                let deferred = DeferredEngine::new("Ella is getting set up.");
                let setup = Setup::deferred(sink, deferred.slot(), paths, models_root);
                (Arc::new(AppService::new(database, Box::new(deferred))), setup)
            } else {
                (
                    Arc::new(AppService::new(database, engine_from_environment(paths))),
                    Setup::ready(sink),
                )
            };
            setup.start();
            app.manage(SetupState(setup));
            service.set_speech_broadcast(Arc::new(WindowSpeech(app.handle().clone())));
            app.manage(AppState(Arc::clone(&service)));
            app.resources_table().add(EngineShutdownGuard(service));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::bootstrap,
            ipc::setup_state,
            ipc::retry_setup,
            ipc::save_learner,
            ipc::log_in,
            ipc::log_out,
            ipc::save_avatar_color,
            ipc::start_session,
            ipc::start_placement,
            ipc::start_chore,
            ipc::speak_opening,
            ipc::speak_retry_prompt,
            ipc::speak_fix,
            ipc::get_session,
            ipc::send_text_turn,
            ipc::send_voice_turn,
            ipc::begin_voice_stream,
            ipc::push_voice_stream,
            ipc::cancel_voice_stream,
            ipc::finish_voice_stream_turn,
            ipc::complete_session,
            ipc::assess_session,
            ipc::levels,
            ipc::fit_page_zoom,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Ella")
        .run(|app, event| {
            // Tauri exits the process without dropping managed state, so this
            // is the last chance to stop llama-server, free Canary's Metal
            // buffers and fold the database's write-ahead log into
            // ella.sqlite3. See `DeferredEngine::shutdown` and
            // `Database::checkpoint`.
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    state.0.shutdown();
                }
                // A model load cut short by the quit drops its half-built
                // engine on the setup thread; give it the moment that takes.
                if let Some(setup) = app.try_state::<SetupState>() {
                    setup.0.wait_while_loading(std::time::Duration::from_secs(3));
                }
            }
        });
}
