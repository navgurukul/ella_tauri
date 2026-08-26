use std::sync::Arc;

use tauri::State;

use crate::{
    application::AppService,
    domain::{AppSnapshot, Learner, Session, SessionSummary, TurnResult},
};

pub struct AppState(pub Arc<AppService>);

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    state.0.bootstrap().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_learner(state: State<'_, AppState>, name: String) -> Result<Learner, String> {
    state
        .0
        .save_learner(&name)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn start_session(state: State<'_, AppState>, topic_id: String) -> Result<Session, String> {
    state
        .0
        .start_session(&topic_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_session(state: State<'_, AppState>, session_id: String) -> Result<Session, String> {
    state
        .0
        .get_session(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn send_text_turn(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<TurnResult, String> {
    state
        .0
        .send_text_turn(&session_id, &text)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn send_voice_turn(
    state: State<'_, AppState>,
    session_id: String,
    samples: Vec<i16>,
    sample_rate: u32,
    browser_transcript: Option<String>,
) -> Result<TurnResult, String> {
    state
        .0
        .send_voice_turn(&session_id, samples, sample_rate, browser_transcript)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn complete_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionSummary, String> {
    state
        .0
        .complete_session(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn reset_demo_data(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    state.0.reset_demo_data().map_err(|error| error.to_string())
}
