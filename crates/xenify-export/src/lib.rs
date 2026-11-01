//! Export pipeline: evaluate each frame, render it offscreen on the GPU and
//! stream it to a [`VideoSink`].

pub mod animated_emoji;
pub mod emoji;
pub mod giphy;
pub mod image;
pub mod lottie;
pub mod lottie_import;
pub mod model3d;
pub mod pdf;
pub mod sources;
pub mod svg;
mod svg_export;
pub mod target;
pub mod trace;

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
use std::path::Path;
use xenify_doc::Composition;
pub use xenify_platform::VideoFormat as Format;
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
use xenify_platform::native::FfmpegSink;
use xenify_platform::{PlatformError, VideoFormat};
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
use xenify_platform::{VideoSettings, VideoSink};
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub use xenify_render::Offscreen;
use xenify_render::RenderError;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error("export cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: VideoFormat,
    /// Output scale relative to the composition size (1.0 = full size).
    pub scale: f64,
    /// Leave the background out (formats with alpha: WebM, GIF, PNG).
    pub transparent: bool,
    /// Add the sound (MP4 and WebM), when there is any.
    pub with_sound: bool,
    /// Exact output size in pixels (instead of `scale`); another shape than
    /// the canvas is letterboxed.
    pub size: Option<(u32, u32)>,
    /// The bars' colour when letterboxing.
    pub letterbox: xenify_doc::Color,
    /// Presentations: only slides `first..=last` (counting from 0).
    pub slides: Option<(usize, usize)>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: VideoFormat::Mp4,
            scale: 1.0,
            transparent: false,
            with_sound: true,
            size: None,
            letterbox: xenify_doc::Color::BLACK,
            slides: None,
        }
    }
}

/// Preset `name` (`720p`, `1080p`, `1440p`, `4k`; any case) or `WxH`, turned
/// to match a `portrait` canvas for the presets.
pub fn parse_size(text: &str, portrait: bool) -> Option<(u32, u32)> {
    let t = text.trim().to_ascii_lowercase();
    let preset = match t.as_str() {
        "720p" | "hd" => Some((1280, 720)),
        "1080p" | "fullhd" | "full-hd" | "fhd" => Some((1920, 1080)),
        "1440p" | "qhd" => Some((2560, 1440)),
        "4k" | "2160p" | "uhd" => Some((3840, 2160)),
        _ => None,
    };
    if let Some((w, h)) = preset {
        return Some(if portrait { (h, w) } else { (w, h) });
    }
    let (w, h) = t.split_once(['x', '×'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    (w > 0 && h > 0 && w <= 16384 && h <= 16384).then_some((w, h))
}

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
/// What to mix for an export's sound: a composition, or the presentation.
enum SoundOf {
    Comp(xenify_doc::Id, f64),
    Deck(f64, f64),
}

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
/// Mixes the sound and puts it into the exported `video` (MP4 / WebM only).
fn add_sound(
    project: &xenify_doc::Project,
    video: &Path,
    options: &ExportOptions,
    of: SoundOf,
) -> Result<(), ExportError> {
    if !options.with_sound
        || !matches!(options.format, VideoFormat::Mp4 | VideoFormat::WebM)
        || !xenify_audio::has_sound(project)
    {
        return Ok(());
    }
    let pcm = match of {
        SoundOf::Comp(comp, length) => xenify_audio::mix_comp(project, comp, 0.0, length),
        SoundOf::Deck(from, to) => xenify_audio::mix_deck(project, from, to),
    };
    let wav = video.with_extension("sound.wav");
    std::fs::write(&wav, xenify_audio::wav(&pcm)).map_err(PlatformError::from)?;
    let result = xenify_platform::media::mux_audio(video, &wav);
    let _ = std::fs::remove_file(&wav);
    Ok(result?)
}

/// Progress callback: `(frames done, total frames)`. Return `false` to cancel.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
/// Renders every frame of `comp` into `sink`.
pub fn render_frames(
    project: &xenify_doc::Project,
    comp: &Composition,
    gpu: &mut Offscreen,
    size: (u32, u32),
    sink: Box<dyn VideoSink>,
    transparent: bool,
    progress: Progress<'_>,
) -> Result<(), ExportError> {
    let mut sink = sink;
    let total = comp.settings.frame_count();
    for frame in 0..total {
        if !progress(frame, total) {
            return Err(ExportError::Cancelled);
        }
        let image = gpu.render_frame(project, comp.id, comp.settings.frame_time(frame), size.0, size.1, transparent)?;
        sink.write_frame(&image.pixels)?;
    }
    sink.finish()?;
    progress(total, total);
    Ok(())
}

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
/// Exports `comp` to `output` with ffmpeg. For [`VideoFormat::PngSequence`],
/// `output` may be a directory; frames are then written as `frame_00000.png`, ….
pub fn export_to_file(
    project: &xenify_doc::Project,
    comp: xenify_doc::Id,
    output: &Path,
    options: &ExportOptions,
    gpu: &mut Offscreen,
    progress: Progress<'_>,
) -> Result<(), ExportError> {
    let comp = project.comp(comp).map_err(|e| ExportError::Invalid(e.to_string()))?;
    let size = options.size.unwrap_or_else(|| output_size(comp, options.scale));
    gpu.set_letterbox(options.letterbox);
    let settings = VideoSettings { format: options.format, width: size.0, height: size.1, fps: comp.settings.fps };
    let target = if options.format == VideoFormat::PngSequence && (output.is_dir() || output.extension().is_none()) {
        std::fs::create_dir_all(output).map_err(PlatformError::from)?;
        output.join("frame_%05d.png")
    } else {
        output.to_path_buf()
    };
    let sink = FfmpegSink::spawn(&settings, &target)?;
    render_frames(
        project,
        comp,
        gpu,
        size,
        Box::new(sink),
        options.transparent && options.format.supports_alpha(),
        progress,
    )?;
    add_sound(project, &target, options, SoundOf::Comp(comp.id, comp.settings.duration))
}

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
/// Exports the presentation (or `options.slides` of it) as one video: the
/// slides in order with their transitions, without stopping at pauses. The
/// frame rate is the first slide's; the size is the first slide's (times
/// `scale`) unless `options.size` says otherwise.
pub fn export_deck_to_file(
    project: &xenify_doc::Project,
    output: &Path,
    options: &ExportOptions,
    gpu: &mut Offscreen,
    progress: Progress<'_>,
) -> Result<(), ExportError> {
    let deck = xenify_anim::deck::Deck::new(project);
    let first = deck
        .slide_comp(0)
        .and_then(|c| project.comp(c).ok())
        .ok_or_else(|| ExportError::Invalid("the presentation has no slides".into()))?;
    let (w, h) = options.size.unwrap_or_else(|| output_size(first, options.scale));
    gpu.set_letterbox(options.letterbox);
    let fps = first.settings.fps;
    // A range of slides: from the first one's start to the last one's end.
    let (from, to) = match options.slides {
        Some((a, b)) => {
            let last = deck.len().saturating_sub(1);
            let (a, b) = (a.min(last), b.min(last).max(a.min(last)));
            (deck.slide_start(a), deck.slide_end(b))
        }
        None => (0.0, deck.duration()),
    };
    let settings = VideoSettings { format: options.format, width: w, height: h, fps };
    let target = if options.format == VideoFormat::PngSequence && (output.is_dir() || output.extension().is_none()) {
        std::fs::create_dir_all(output).map_err(PlatformError::from)?;
        output.join("frame_%05d.png")
    } else {
        output.to_path_buf()
    };
    let mut sink: Box<dyn VideoSink> = Box::new(FfmpegSink::spawn(&settings, &target)?);
    let total = ((to - from) * fps).round().max(1.0) as u64;
    for frame in 0..total {
        if !progress(frame, total) {
            return Err(ExportError::Cancelled);
        }
        let Some(at) = deck.at(from + frame as f64 / fps) else { break };
        let image = gpu.render_deck(project, &at, w, h)?;
        sink.write_frame(&image.pixels)?;
    }
    sink.finish()?;
    add_sound(project, &target, options, SoundOf::Deck(from, to))?;
    progress(total, total);
    Ok(())
}

pub fn output_size(comp: &Composition, scale: f64) -> (u32, u32) {
    let s = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (s(comp.settings.width), s(comp.settings.height))
}

#[cfg(test)]
mod size_tests {
    use super::*;

    #[test]
    fn sizes_parse_as_presets_or_width_by_height() {
        assert_eq!(parse_size("4k", false), Some((3840, 2160)));
        assert_eq!(parse_size("4K", true), Some((2160, 3840)), "turned for a portrait canvas");
        assert_eq!(parse_size("1080p", false), Some((1920, 1080)));
        assert_eq!(parse_size("1000x500", true), Some((1000, 500)), "exact sizes aren't turned");
        assert_eq!(parse_size("0x5", false), None);
        assert_eq!(parse_size("big", false), None);
    }
}
