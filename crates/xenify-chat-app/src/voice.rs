//! Voice messages and meeting transcripts: speech turned into text on this
//! computer (xenify-voice). The model is downloaded the first time it's
//! wanted; Settings › Voice and transcripts says how far along it is.

use base64::Engine;
use serde::Serialize;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, State};
use xenify_voice::{Model, Speech};

#[derive(Default)]
pub struct Voice {
    /// The model loaded last (its id), kept for the next one.
    loaded: Mutex<Option<(&'static str, Arc<Speech>)>>,
    /// A download going on: its model's id and how far (bytes).
    downloading: Mutex<Option<(&'static str, u64, u64)>>,
    stop: Arc<AtomicBool>,
}

#[derive(Serialize)]
pub struct ModelOut {
    id: &'static str,
    bytes: u64,
    have: bool,
}

#[derive(Serialize)]
pub struct VoiceState {
    /// This build can turn speech into text.
    available: bool,
    models: Vec<ModelOut>,
    /// Downloading: (model id, done, of).
    downloading: Option<(&'static str, u64, u64)>,
    source: &'static str,
}

#[tauri::command]
pub fn voice_state(voice: State<'_, Voice>) -> VoiceState {
    VoiceState {
        available: true,
        models: xenify_voice::MODELS
            .iter()
            .map(|m| ModelOut { id: m.id, bytes: m.bytes, have: xenify_voice::have(*m) })
            .collect(),
        downloading: *voice.downloading.lock().unwrap(),
        source: xenify_voice::SOURCE,
    }
}

#[derive(Clone, Serialize)]
struct Progress {
    id: &'static str,
    done: u64,
    of: u64,
    /// Finished: "" when it worked, else what went wrong.
    finished: Option<String>,
}

/// Starts downloading `id`'s model (says how it goes in `voice_download`).
#[tauri::command]
pub fn voice_download(app: AppHandle, voice: State<'_, Voice>, id: String) {
    let m = xenify_voice::model(&id);
    {
        let mut d = voice.downloading.lock().unwrap();
        if d.is_some() {
            return;
        }
        *d = Some((m.id, 0, m.bytes));
    }
    voice.stop.store(false, Ordering::Relaxed);
    let stop = voice.stop.clone();
    std::thread::spawn(move || {
        let mut last = 0;
        let done = xenify_voice::download(m, &stop, |done, of| {
            // About every megabyte.
            if done - last >= 1 << 20 {
                last = done;
                use tauri::Manager;
                *app.state::<Voice>().downloading.lock().unwrap() = Some((m.id, done, of));
                let _ = app.emit("voice_download", Progress { id: m.id, done, of, finished: None });
            }
        });
        use tauri::Manager;
        *app.state::<Voice>().downloading.lock().unwrap() = None;
        let finished = Some(done.err().unwrap_or_default());
        let _ = app.emit("voice_download", Progress { id: m.id, done: 0, of: m.bytes, finished });
    });
}

#[tauri::command]
pub fn voice_stop_download(voice: State<'_, Voice>) {
    voice.stop.store(true, Ordering::Relaxed);
}

/// Throws a downloaded model away (Settings, to free the space).
#[tauri::command]
pub fn voice_remove(voice: State<'_, Voice>, id: String) -> Result<(), String> {
    let m = xenify_voice::model(&id);
    let mut loaded = voice.loaded.lock().unwrap();
    if loaded.as_ref().is_some_and(|(l, _)| *l == m.id) {
        *loaded = None;
    }
    std::fs::remove_file(xenify_voice::path(m)).map_err(|e| e.to_string())
}

/// What's said in `audio` (base64 of 16 kHz mono 16-bit samples), with
/// `id`'s model. Errors "no_model" when it isn't downloaded yet.
#[tauri::command]
pub async fn voice_text(
    voice: State<'_, Voice>,
    id: String,
    audio: String,
    language: String,
) -> Result<String, String> {
    let m = xenify_voice::model(&id);
    let speech = speech(&voice, m)?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(audio).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let t = std::time::Instant::now();
        let out = speech.text(&xenify_voice::from_i16_le(&bytes), &language);
        if crate::net::tracing() {
            eprintln!("voice: {} s of sound into text in {:?}", bytes.len() / 32000, t.elapsed());
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The same, with when each piece was said (meeting transcripts).
#[tauri::command]
pub async fn voice_pieces(
    voice: State<'_, Voice>,
    id: String,
    audio: String,
    language: String,
) -> Result<Vec<xenify_voice::Piece>, String> {
    let m = xenify_voice::model(&id);
    let speech = speech(&voice, m)?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(audio).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let t = std::time::Instant::now();
        let out = speech.pieces(&xenify_voice::from_i16_le(&bytes), &language);
        if crate::net::tracing() {
            eprintln!("voice: {} s of sound into text in {:?}", bytes.len() / 32000, t.elapsed());
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?
}

fn speech(voice: &Voice, m: Model) -> Result<Arc<Speech>, String> {
    if !xenify_voice::have(m) {
        return Err("no_model".into());
    }
    let mut loaded = voice.loaded.lock().unwrap();
    if let Some((id, s)) = loaded.as_ref()
        && *id == m.id
    {
        return Ok(s.clone());
    }
    let s = Arc::new(Speech::load(m)?);
    *loaded = Some((m.id, s.clone()));
    Ok(s)
}

/// For tests (`XENIFY_VOICE_TEST_WAV`, 16 kHz mono 16-bit): what the
/// microphone "hears" when recording, as base64 samples.
#[tauri::command]
pub fn voice_test_audio() -> Option<String> {
    let wav = std::fs::read(std::env::var_os("XENIFY_VOICE_TEST_WAV")?).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(wav.get(44..)?))
}
