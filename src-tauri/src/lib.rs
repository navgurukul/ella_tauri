pub mod application;
pub mod domain;
pub mod error;
pub mod infrastructure;
mod ipc;
mod telemetry;

use std::sync::Arc;

use application::{AppService, SpeechBroadcast};
use domain::SpeechStreamEvent;
use infrastructure::{database::Database, engines::engine_from_environment};
use ipc::AppState;
use tauri::{AppHandle, Emitter, Manager};

/// The event name the window listens on for mid-turn speech.
pub const SPEECH_STREAM_EVENT: &str = "ella://speech-segment";

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
            let service = Arc::new(AppService::new(
                database,
                engine_from_environment(packaged_engine_root),
            ));
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
