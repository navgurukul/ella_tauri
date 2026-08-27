pub mod application;
pub mod domain;
pub mod error;
pub mod infrastructure;
mod ipc;
mod telemetry;

use std::sync::Arc;

use application::AppService;
use infrastructure::{database::Database, engines::engine_from_environment};
use ipc::AppState;
use tauri::Manager;

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
            app.manage(AppState(Arc::new(AppService::new(
                database,
                engine_from_environment(packaged_engine_root),
            ))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::bootstrap,
            ipc::save_learner,
            ipc::start_session,
            ipc::start_chore,
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
