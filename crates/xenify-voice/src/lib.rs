//! Speech to text on this computer, for voice messages and meeting
//! transcripts: OpenAI's Whisper models run with whisper.cpp. Nothing said
//! is sent anywhere. The model is downloaded once, on first use, from
//! whisper.cpp's own copies on Hugging Face, and checked against the
//! SHA-256 written here before it's used.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Audio the models take: 16 kHz, mono.
pub const RATE: u32 = 16_000;

/// The sizes on offer (quantised copies: much smaller, nearly as good).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    /// "quick", "better" or "best" (the setting).
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const MODELS: [Model; 3] = [
    Model {
        id: "quick",
        file: "ggml-tiny-q8_0.bin",
        bytes: 43_537_433,
        sha256: "c2085835d3f50733e2ff6e4b41ae8a2b8d8110461e18821b09a15c40c42d1cca",
    },
    Model {
        id: "better",
        file: "ggml-base-q5_1.bin",
        bytes: 59_707_625,
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
    },
    Model {
        id: "best",
        file: "ggml-small-q5_1.bin",
        bytes: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
];

/// Where they come from (whisper.cpp's conversions of OpenAI's Whisper, MIT).
pub const SOURCE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// `id`'s model ("better" when it's not one of them).
pub fn model(id: &str) -> Model {
    MODELS.iter().copied().find(|m| m.id == id).unwrap_or(MODELS[1])
}

/// Where models are kept: Xenify's data folder (`~/.local/share/xenify` on
/// Linux), `models/whisper`. `XENIFY_VOICE_DIR` overrides it (tests).
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XENIFY_VOICE_DIR") {
        return d.into();
    }
    dirs::data_local_dir().unwrap_or_else(|| ".".into()).join("xenify").join("models").join("whisper")
}

pub fn path(m: Model) -> PathBuf {
    dir().join(m.file)
}

/// Downloaded (and checked when it was).
pub fn have(m: Model) -> bool {
    std::fs::metadata(path(m)).is_ok_and(|f| f.len() == m.bytes)
}

/// Downloads `m`, telling `progress` (done, of) as it goes; `stop` cancels.
/// Blocks: call it off the screens' thread. The file only takes its name
/// once its SHA-256 matches.
pub fn download(m: Model, stop: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    if have(m) {
        return Ok(());
    }
    let url = format!("{SOURCE}{}", m.file);
    let failed = |e: &dyn std::fmt::Display| format!("Couldn't download the speech model: {e}");
    std::fs::create_dir_all(dir()).map_err(|e| failed(&e))?;
    let part = path(m).with_extension("part");
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_connect(Some(std::time::Duration::from_secs(30))).build().into();
    let mut resp = agent.get(&url).call().map_err(|e| failed(&e))?;
    let mut body = resp.body_mut().as_reader();
    let mut out = std::fs::File::create(&part).map_err(|e| failed(&e))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err("Stopped.".into());
        }
        let n = body.read(&mut buf).map_err(|e| failed(&e))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| failed(&e))?;
        done += n as u64;
        if done > m.bytes {
            break;
        }
        progress(done, m.bytes);
    }
    drop(out);
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if done != m.bytes || got != m.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("The speech model that came wasn't the right one, so it wasn't kept. Try again later.".into());
    }
    std::fs::rename(&part, path(m)).map_err(|e| failed(&e))?;
    log(&url, m);
    Ok(())
}

/// Notes the download beside the models (when, from where, how big).
fn log(url: &str, m: Model) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("downloads.log")) {
        let _ = writeln!(f, "{secs}\t{url}\t{}\t{} bytes\tsha256 {}", m.file, m.bytes, m.sha256);
    }
}

/// A loaded model.
pub struct Speech {
    ctx: whisper_rs::WhisperContext,
}

impl Speech {
    /// Loads `m` (downloaded already).
    pub fn load(m: Model) -> Result<Speech, String> {
        // whisper.cpp's own messages go nowhere (they'd go to standard error).
        whisper_rs::install_logging_hooks();
        let p = path(m);
        let ctx = whisper_rs::WhisperContext::new_with_params(
            p.to_str().ok_or("The model's folder has an odd name.")?,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| format!("The speech model didn't load: {e}"))?;
        Ok(Speech { ctx })
    }

    /// What's said in `audio` (16 kHz mono, -1..1). `language`: a code
    /// like "en", or "" to work it out.
    pub fn text(&self, audio: &[f32], language: &str) -> Result<String, String> {
        Ok(self.pieces(audio, language)?.into_iter().map(|p| p.text).collect::<Vec<_>>().join(" "))
    }

    /// The same, piece by piece, with when each was said.
    pub fn pieces(&self, audio: &[f32], language: &str) -> Result<Vec<Piece>, String> {
        if audio.len() < RATE as usize / 2 {
            return Ok(Vec::new());
        }
        let mut state = self.ctx.create_state().map_err(|e| e.to_string())?;
        let mut p = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        p.set_language(Some(if language.is_empty() { "auto" } else { language }));
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
        p.set_n_threads(threads as i32);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        p.set_suppress_nst(true);
        // Whisper hears 30 s at a time; a shorter piece needn't cost as much
        // (1500 = 30 s, with a little room).
        let ctx = (audio.len() as u64 * 1500).div_ceil(30 * RATE as u64) + 128;
        p.set_audio_ctx(ctx.clamp(384, 1500) as i32);
        state.full(p, audio).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for s in state.as_iter() {
            // Silence comes out as nothing (not "[BLANK_AUDIO]" and the like).
            if s.no_speech_probability() > 0.8 {
                continue;
            }
            let t = s.to_str_lossy().map_err(|e| e.to_string())?;
            let t = t.trim();
            if t.is_empty() || (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')')) {
                continue;
            }
            // (Centiseconds.)
            out.push(Piece { from_ms: s.start_timestamp() * 10, to_ms: s.end_timestamp() * 10, text: t.to_string() });
        }
        Ok(out)
    }
}

/// Something said, and when (from the start of the audio).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Piece {
    pub from_ms: i64,
    pub to_ms: i64,
    pub text: String,
}

/// 16-bit little-endian samples (as the screens send them) → -1..1.
pub fn from_i16_le(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_sizes_mean_better() {
        assert_eq!(model("best").file, "ggml-small-q5_1.bin");
        assert_eq!(model("huge"), MODELS[1]);
    }

    #[test]
    fn samples_convert() {
        let bytes = [0x00, 0x80, 0x00, 0x00, 0xff, 0x7f];
        let got = from_i16_le(&bytes);
        assert_eq!(got[0], -1.0);
        assert_eq!(got[1], 0.0);
        assert!((got[2] - 1.0).abs() < 0.001);
    }

    /// With `XENIFY_VOICE_TEST_WAV` (16 kHz mono 16-bit) and the Quick
    /// model downloaded: prints what it hears.
    #[test]
    #[ignore]
    fn hears_a_recording() {
        let wav = std::fs::read(std::env::var("XENIFY_VOICE_TEST_WAV").unwrap()).unwrap();
        let m = model(&std::env::var("XENIFY_VOICE_TEST_MODEL").unwrap_or("quick".into()));
        download(m, &AtomicBool::new(false), |_, _| {}).unwrap();
        let s = Speech::load(m).unwrap();
        let t = std::time::Instant::now();
        let text = s.text(&from_i16_le(&wav[44..]), "en").unwrap();
        eprintln!("{:?} in {:?}", text, t.elapsed());
        assert!(!text.is_empty());
    }
}
