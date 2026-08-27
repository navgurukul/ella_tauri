use std::sync::Arc;

use tauri::State;

use crate::{
    application::AppService,
    domain::{AppSnapshot, Learner, Session, SessionSummary, TurnResult},
    error::EllaResult,
};

pub struct AppState(pub Arc<AppService>);

// Every command hops to a blocking thread: Tauri runs command bodies on the
// main thread, and the STT/LLM/TTS pipeline (and even the 2s engine health
// probes in bootstrap) would otherwise freeze the window and show the macOS
// spinner until the turn finishes.
async fn off_main_thread<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> EllaResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("background task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let service = state.0.clone();
    off_main_thread(move || service.bootstrap()).await
}

#[tauri::command]
pub async fn save_learner(
    state: State<'_, AppState>,
    name: String,
    age: Option<u8>,
) -> Result<Learner, String> {
    let service = state.0.clone();
    off_main_thread(move || service.save_learner(&name, age)).await
}

#[tauri::command]
pub async fn start_session(
    state: State<'_, AppState>,
    topic_id: String,
) -> Result<Session, String> {
    let service = state.0.clone();
    off_main_thread(move || service.start_session(&topic_id)).await
}

#[tauri::command]
pub async fn start_chore(
    state: State<'_, AppState>,
    chore_id: String,
) -> Result<Session, String> {
    let service = state.0.clone();
    off_main_thread(move || service.start_chore(&chore_id)).await
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Session, String> {
    let service = state.0.clone();
    off_main_thread(move || service.get_session(&session_id)).await
}

#[tauri::command]
pub async fn send_text_turn(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<TurnResult, String> {
    let service = state.0.clone();
    off_main_thread(move || service.send_text_turn(&session_id, &text)).await
}

#[tauri::command]
pub async fn send_voice_turn(
    state: State<'_, AppState>,
    session_id: String,
    samples: Vec<i16>,
    sample_rate: u32,
    browser_transcript: Option<String>,
) -> Result<TurnResult, String> {
    let service = state.0.clone();
    off_main_thread(move || {
        service.send_voice_turn(&session_id, samples, sample_rate, browser_transcript)
    })
    .await
}

#[tauri::command]
pub async fn begin_voice_stream(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let service = state.0.clone();
    off_main_thread(move || service.begin_voice_stream(&session_id)).await
}

#[tauri::command]
pub async fn push_voice_stream(
    state: State<'_, AppState>,
    stream_id: String,
    samples: Vec<i16>,
    sample_rate: u32,
) -> Result<(), String> {
    let service = state.0.clone();
    off_main_thread(move || service.push_voice_stream(&stream_id, samples, sample_rate)).await
}

#[tauri::command]
pub async fn cancel_voice_stream(
    state: State<'_, AppState>,
    stream_id: String,
) -> Result<(), String> {
    let service = state.0.clone();
    off_main_thread(move || {
        service.cancel_voice_stream(&stream_id);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn finish_voice_stream_turn(
    state: State<'_, AppState>,
    stream_id: String,
    tail_samples: Vec<i16>,
    sample_rate: u32,
    browser_transcript: Option<String>,
) -> Result<TurnResult, String> {
    let service = state.0.clone();
    off_main_thread(move || {
        service.finish_voice_stream_turn(&stream_id, tail_samples, sample_rate, browser_transcript)
    })
    .await
}

#[tauri::command]
pub async fn complete_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionSummary, String> {
    let service = state.0.clone();
    off_main_thread(move || service.complete_session(&session_id)).await
}

#[tauri::command]
pub async fn reset_demo_data(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let service = state.0.clone();
    off_main_thread(move || service.reset_demo_data()).await
}
