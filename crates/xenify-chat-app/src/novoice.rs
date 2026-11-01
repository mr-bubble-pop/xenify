//! Built without speech to text (Windows builds made on Linux, where
//! whisper.cpp doesn't link): the same commands as voice.rs, saying so.
//! Voice messages still record and play; they just have no text.

use serde::Serialize;

#[derive(Default)]
pub struct Voice;

#[derive(Serialize)]
pub struct VoiceState {
    available: bool,
    models: Vec<()>,
    downloading: Option<()>,
    source: &'static str,
}

#[tauri::command]
pub fn voice_state() -> VoiceState {
    VoiceState { available: false, models: Vec::new(), downloading: None, source: "" }
}

#[tauri::command]
pub fn voice_download() {}

#[tauri::command]
pub fn voice_stop_download() {}

#[tauri::command]
pub fn voice_remove() -> Result<(), String> {
    Err("This build can't turn speech into text.".into())
}

#[tauri::command]
pub async fn voice_text() -> Result<String, String> {
    Err("no_model".into())
}

#[tauri::command]
pub async fn voice_pieces() -> Result<Vec<()>, String> {
    Err("no_model".into())
}

#[tauri::command]
pub fn voice_test_audio() -> Option<String> {
    None
}
