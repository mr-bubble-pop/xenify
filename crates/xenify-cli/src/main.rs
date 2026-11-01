//! Headless Xenify Draw tools.
//!
//! ```text
//! xenify-cli render project.xenify -o out.mp4 [--scale 0.5] [--format gif]   (or out.svg / .json / .html)
//! xenify-cli demo demo.xenify
//! ```

use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write;
use xenify_doc::Project;
use xenify_export::{ExportOptions, export_to_file};
use xenify_platform::native::NativeFs;
use xenify_platform::{FileStore, VideoFormat};
use xenify_render::Offscreen;

mod ai;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(version, about = "Headless Xenify Draw tools")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a project's first composition to a video, GIF or PNG sequence.
    Render {
        project: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Output format (default: from the output file extension).
        #[arg(short, long)]
        format: Option<Format>,
        /// Output size relative to the composition (e.g. 0.5 for half size).
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
        /// Leave the background out (WebM, GIF and PNG only).
        #[arg(long)]
        transparent: bool,
        /// Exact output size: 720p, 1080p, 1440p, 4k or WxH (another shape
        /// than the canvas is letterboxed). Overrides --scale.
        #[arg(long)]
        size: Option<String>,
        /// Render the whole presentation (all slides with their transitions).
        #[arg(long)]
        deck: bool,
        /// With --deck: only these slides, e.g. 2-5 (counting from 1).
        #[arg(long)]
        slides: Option<String>,
        /// Leave the sound out.
        #[arg(long)]
        no_sound: bool,
    },
    /// Write the built-in demo project to a file.
    Demo { output: PathBuf },
    /// Build the showcase presentation (all media made by the program).
    Showcase {
        output: PathBuf,
        /// The product name shown in it.
        #[arg(long, default_value = "Spark")]
        name: String,
        /// Use still pictures instead of rendering the movie slide's clips.
        #[arg(long)]
        no_clips: bool,
    },
    /// Ask the AI assistant to create/edit a project (uses OPENROUTER_API_KEY).
    Ai {
        prompt: String,
        /// Project to edit (default: a new empty 1080p project).
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Where to save the resulting project.
        #[arg(short, long)]
        output: PathBuf,
        /// Also render the result to this video/GIF.
        #[arg(long)]
        render: Option<PathBuf>,
        /// Brain model (default from settings).
        #[arg(long)]
        model: Option<String>,
        /// Reviewer model; pass "" to disable frame reviews.
        #[arg(long)]
        reviewer: Option<String>,
        /// Build a whole slide deck from the prompt (as AI ▸ New From a Description does).
        #[arg(long)]
        deck: bool,
        /// Answer to give if the assistant asks a question.
        #[arg(long, default_value = "Use your best judgement.")]
        answer: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Mp4,
    Webm,
    Gif,
    Png,
}

impl From<Format> for VideoFormat {
    fn from(f: Format) -> Self {
        match f {
            Format::Mp4 => VideoFormat::Mp4,
            Format::Webm => VideoFormat::WebM,
            Format::Gif => VideoFormat::Gif,
            Format::Png => VideoFormat::PngSequence,
        }
    }
}

fn main() -> ExitCode {
    xenify_platform::media::install();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Cmd::Ai { prompt, input, output, render, model, reviewer, deck, answer } => {
            let project = match input {
                Some(p) => Project::from_json(&NativeFs.read_to_string(&p)?)?,
                None => Project::new(),
            };
            let mut settings = xenify_ai::AiSettings::load();
            if let Some(m) = model {
                settings.brain_model = m;
            }
            if let Some(r) = reviewer {
                settings.reviewer_model = r;
            }
            let (prompt, steps) = if deck {
                let s = &project.main_comp().ok_or("no composition")?.settings;
                (xenify_ai::requests::new_deck(&prompt, s.width, s.height), Some(xenify_ai::requests::DECK_STEPS))
            } else {
                (prompt, None)
            };
            let project = ai::run(project, &prompt, settings, &answer, steps)?;
            NativeFs.write(&output, project.to_json().as_bytes())?;
            eprintln!("wrote {}", output.display());
            if let Some(video) = render {
                let format = VideoFormat::from_path(&video).ok_or("unknown video format")?;
                let options = ExportOptions { format, scale: 1.0, transparent: false, ..Default::default() };
                let mut gpu = Offscreen::new()?;
                if project.slides.is_empty() {
                    let comp = project.main_comp().ok_or("no composition")?;
                    export_to_file(&project, comp.id, &video, &options, &mut gpu, &mut |_, _| true)?;
                } else {
                    // A presentation: every slide, played through.
                    xenify_export::export_deck_to_file(&project, &video, &options, &mut gpu, &mut |_, _| true)?;
                }
                eprintln!("rendered {}", video.display());
            }
        }
        Cmd::Showcase { output, name, no_clips } => {
            let (project, _) = xenify_showcase::build_with(&xenify_showcase::Options { name, clips: !no_clips });
            NativeFs.write(&output, project.to_json().as_bytes())?;
            eprintln!("wrote {} ({} slides)", output.display(), project.slides.len());
        }
        Cmd::Demo { output } => {
            NativeFs.write(&output, xenify_doc::demo_project().to_json().as_bytes())?;
            eprintln!("wrote {}", output.display());
        }
        Cmd::Render { project, output, format, scale, transparent, size, deck, slides, no_sound } => {
            let project = Project::from_json(&NativeFs.read_to_string(&project)?)?;
            let comp = project.main_comp().ok_or("project has no compositions")?;
            // Animated SVG, Lottie JSON / HTML player: vector exports, no rendering.
            let ext = output.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
            if format.is_none() && ext.as_deref() == Some("svg") {
                let options = xenify_export::svg::AnimatedSvgOptions::default();
                let mut text = xenify_render::TextEngine::new();
                let svg = xenify_export::svg::export_animated_svg(&project, comp.id, &mut text, options)?;
                NativeFs.write(&output, svg.as_bytes())?;
                eprintln!("wrote {}", output.display());
                return Ok(());
            }
            if format.is_none() && matches!(ext.as_deref(), Some("json" | "html")) {
                let json =
                    xenify_export::lottie::export_lottie(&project, comp.id, &mut xenify_render::TextEngine::new())?;
                let text = if ext.as_deref() == Some("html") {
                    xenify_export::lottie::export_html(&json, &comp.settings.name, comp.settings.background)
                } else {
                    json.to_string()
                };
                NativeFs.write(&output, text.as_bytes())?;
                eprintln!("wrote {}", output.display());
                return Ok(());
            }
            if format.is_none() && ext.as_deref() == Some("pdf") {
                // A page per slide (or the first frame), with any speaker notes.
                let notes = project.slides.iter().any(|s| !s.notes.trim().is_empty());
                let options = xenify_export::pdf::PdfOptions { scale: scale.clamp(0.1, 8.0), notes };
                let pdf = xenify_export::pdf::export_pdf(&project, comp.id, 0.0, options, &mut Offscreen::new()?)?;
                NativeFs.write(&output, &pdf)?;
                eprintln!("wrote {}", output.display());
                return Ok(());
            }
            let format = match format {
                Some(f) => f.into(),
                None => VideoFormat::from_path(&output)
                    .ok_or("cannot tell the format from the output name; pass --format")?,
            };
            if !(scale > 0.0 && scale <= 8.0) {
                return Err("--scale must be in (0, 8]".into());
            }
            let portrait = comp.settings.height > comp.settings.width;
            let size = match size {
                Some(s) => {
                    Some(xenify_export::parse_size(&s, portrait).ok_or("--size: use 720p, 1080p, 1440p, 4k or WxH")?)
                }
                None => None,
            };
            let slides = match slides {
                Some(s) => {
                    let (a, b) = s.split_once('-').unwrap_or((&s, &s));
                    let (a, b): (usize, usize) = (a.trim().parse()?, b.trim().parse()?);
                    if a == 0 || b < a {
                        return Err("--slides: use FIRST-LAST, counting from 1".into());
                    }
                    Some((a - 1, b - 1))
                }
                None => None,
            };
            let options =
                ExportOptions { format, scale, transparent, size, slides, with_sound: !no_sound, ..Default::default() };
            let mut gpu = Offscreen::new()?;
            eprintln!("rendering {:?} on {} …", comp.settings.name, gpu.adapter_name());
            let start = std::time::Instant::now();
            let mut report = |done: u64, total: u64| {
                eprint!("\r  frame {done}/{total}");
                let _ = std::io::stderr().flush();
                true
            };
            if deck && !project.slides.is_empty() {
                xenify_export::export_deck_to_file(&project, &output, &options, &mut gpu, &mut report)?;
            } else {
                export_to_file(&project, comp.id, &output, &options, &mut gpu, &mut report)?;
            }
            eprintln!("\nwrote {} in {:.1}s", output.display(), start.elapsed().as_secs_f64());
        }
    }
    Ok(())
}
