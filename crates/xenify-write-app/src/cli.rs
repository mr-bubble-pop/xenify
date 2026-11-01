//! The commands that work without the editor window: `xenify-write
//! convert|check|profiles|merge …` (`xenify-write --help` lists them).

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use xenify_wp_io::Format;

/// The words that start a command rather than name a file to open.
pub const COMMANDS: &[&str] = &["convert", "check", "profiles", "merge", "help", "--help", "-h", "--version", "-V"];

#[derive(Parser)]
#[command(
    name = "xenify-write",
    version,
    about = "Xenify Write word processor: `xenify-write [FILE…]` opens the editor; these commands work without it"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum ProfilesAction {
    /// Lists every profile (the default).
    List,
    /// Writes a profile to a .toml file (to share or edit).
    Export { id: String, file: PathBuf },
    /// Adds a profile from a .toml file to your custom profiles.
    Import { file: PathBuf },
}

#[derive(Subcommand)]
enum Command {
    /// Converts a document to another format (by the output's extension, or --to).
    Convert {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Output format (md, docx, xwrite, txt, pdf, html); defaults to the output's extension.
        #[arg(long)]
        to: Option<String>,
    },
    /// Spelling, grammar and readability report (exit code 1 when issues are found).
    Check {
        input: PathBuf,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
        /// English variant: us, uk, ca, au, in, or auto (the computer's language).
        #[arg(long, default_value = "auto")]
        dialect: String,
        /// Writing profile (id, e.g. academic, workplace-email, social; or a
        /// .toml file). Default: the document's own choice, else standard.
        #[arg(long)]
        profile: Option<String>,
        /// Mistake spotter (offline model): auto (use it if installed in
        /// Xenify's data folder, e.g. ~/.local/share/xenify/spotter), off, or a model folder.
        #[arg(long, default_value = "auto")]
        spotter: String,
    },
    /// Writing profiles: list them, or export/import one as a .toml file.
    Profiles {
        #[command(subcommand)]
        action: Option<ProfilesAction>,
    },
    /// Mail merge: fills a template with every record of a data file (CSV
    /// or JSON) and writes one file per record, or one combined file.
    Merge {
        /// The template (Markdown, .xwrite or .docx) with {{field}} placeholders.
        template: PathBuf,
        /// Data file: CSV with a header row, or JSON (a list of objects).
        /// Defaults to the template's merge source.
        #[arg(short, long)]
        data: Option<PathBuf>,
        /// Output format: pdf, docx, md, html, xwrite, txt.
        #[arg(long, default_value = "pdf")]
        to: String,
        /// Folder to write into (created if needed).
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        /// One file with every record (each starting on a new page).
        #[arg(long)]
        combined: bool,
        /// File names, with fields, e.g. "{{last}}-{{first}}".
        #[arg(long)]
        name: Option<String>,
        /// Records to use, e.g. "1-10,15".
        #[arg(long, default_value = "")]
        range: String,
        /// Make one record of the rows sharing this column's value (their rows go in `rows`).
        #[arg(long)]
        group_by: Option<String>,
    },
}

fn format_of(path: &std::path::Path, explicit: Option<&str>) -> Result<Format, String> {
    let name = match explicit {
        Some(ext) => format!("x.{ext}"),
        None => path.to_string_lossy().into_owned(),
    };
    Format::from_name(&name).ok_or_else(|| {
        let known: Vec<_> = Format::ALL.iter().map(|f| f.extension()).collect();
        format!("unknown format for {} (known: {})", path.display(), known.join(", "))
    })
}

fn convert(input: &PathBuf, output: &PathBuf, to: Option<&str>) -> Result<(), String> {
    let from = format_of(input, None)?;
    let to = format_of(output, to)?;
    let bytes = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let mut loaded = xenify_wp_io::load(&bytes, from).map_err(|e| format!("{}: {e}", input.display()))?;
    for w in &loaded.warnings {
        eprintln!("warning: {w}");
    }
    // Pictures next to the input travel with the document…
    let in_dir = input.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    xenify_wp_io::attach_images(&mut loaded.doc, &|src| std::fs::read(in_dir.join(src)).ok());
    let out = xenify_wp_io::save(&loaded.doc, to).map_err(|e| e.to_string())?;
    std::fs::write(output, out).map_err(|e| format!("{}: {e}", output.display()))?;
    // …and Markdown output keeps them as files next to it.
    if to == Format::Markdown {
        let out_dir = output.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        for src in xenify_wp_io::image_sources(&loaded.doc) {
            if let Some(b) = loaded.doc.assets.get(&src) {
                let t = out_dir.join(&src);
                if let Some(p) = t.parent() {
                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
                std::fs::write(&t, b.as_slice()).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn merge(
    template: &PathBuf,
    data: Option<&PathBuf>,
    to: &str,
    out: &PathBuf,
    combined: bool,
    name: Option<&str>,
    range: &str,
    group_by: Option<&str>,
) -> Result<(), String> {
    let from = format_of(template, None)?;
    let format = format_of(out, Some(to))?;
    let bytes = std::fs::read(template).map_err(|e| format!("{}: {e}", template.display()))?;
    let mut doc = xenify_wp_io::load(&bytes, from).map_err(|e| format!("{}: {e}", template.display()))?.doc;
    let dir = template.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    xenify_wp_io::attach_images(&mut doc, &|src| std::fs::read(dir.join(src)).ok());
    let config = doc.meta.merge.clone().unwrap_or_default();
    let data_path = match data {
        Some(d) => d.clone(),
        None if !config.source.is_empty() => dir.join(&config.source),
        None => return Err("no data file: pass --data (CSV or JSON)".into()),
    };
    let data_bytes = std::fs::read(&data_path).map_err(|e| format!("{}: {e}", data_path.display()))?;
    let mut records = xenify_wp_io::merge_data::records(&data_bytes, &data_path.to_string_lossy())?;
    let group = group_by.map(str::to_string).unwrap_or(config.group_by.clone());
    if !group.is_empty() {
        records = xenify_wp_io::merge_data::group(records, &group);
    }
    if records.is_empty() {
        return Err(format!("{} has no records", data_path.display()));
    }
    let job = xenify_wp_io::merge_data::MergeJob {
        format,
        combined,
        name_template: name.unwrap_or("").to_string(),
        range: range.to_string(),
    };
    let mut engine = xenify_wp_layout::Engine::new(xenify_wp_layout::Fonts::new());
    let assets = doc.assets.clone();
    let img_dir = dir.clone();
    let image = move |src: &str| {
        assets.get(src).cloned().or_else(|| std::fs::read(img_dir.join(src)).ok().map(std::sync::Arc::new))
    };
    let files = xenify_wp_io::merge_data::run(&doc, &records, &job, &mut engine, &image, &mut |done, total| {
        eprint!("\rmerged {done}/{total}");
    })
    .map_err(|e| e.to_string())?;
    eprintln!();
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    for (name, bytes) in &files {
        let path = out.join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("{}", path.display());
    }
    Ok(())
}

fn profiles(action: Option<ProfilesAction>) -> Result<(), String> {
    use xenify_wp_check::{Feature, Profiles};
    let mut ps = Profiles::load(Some(xenify_write_host::proof::profiles_dir()));
    match action.unwrap_or(ProfilesAction::List) {
        ProfilesAction::List => {
            for p in ps.all() {
                let kind = if p.built_in { "built-in" } else { "custom" };
                println!("{:22} {:28} {kind:8} {}", p.id, p.name, p.description);
                let levels: Vec<String> =
                    Feature::ALL.iter().map(|f| format!("{}={}", f.name(), p.level(*f).name())).collect();
                println!("{:22} {}", "", levels.join(", "));
            }
            println!("custom profiles: {}", xenify_write_host::proof::profiles_dir().display());
            Ok(())
        }
        ProfilesAction::Export { id, file } => {
            if !ps.ids().contains(&id) {
                return Err(format!("no profile {id:?}; see `xenify-write profiles`"));
            }
            xenify_wp_check::write_profile(&file, &ps.get(&id))
        }
        ProfilesAction::Import { file } => {
            let p = ps.import(&file)?;
            println!("imported {} ({})", p.name, p.id);
            Ok(())
        }
    }
}

fn check(input: &PathBuf, json: bool, dialect: &str, profile: Option<&str>, spotter: &str) -> Result<bool, String> {
    use xenify_wp_check::{Checker, Dialect, Settings};
    let from = format_of(input, None)?;
    let bytes = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let doc = xenify_wp_io::load(&bytes, from).map_err(|e| format!("{}: {e}", input.display()))?.doc;
    let dialect = if dialect.eq_ignore_ascii_case("auto") {
        Dialect::from_locale()
    } else {
        Dialect::parse(dialect)
            .ok_or_else(|| format!("unknown English variant {dialect:?} (us, uk, ca, au, in or auto)"))?
    };
    let words = xenify_write_host::proof::read_words();
    let profiles = xenify_wp_check::Profiles::load(Some(xenify_write_host::proof::profiles_dir()));
    let profile = match profile {
        Some(p) if p.ends_with(".toml") => xenify_wp_check::read_profile(std::path::Path::new(p))?,
        Some(p) if !profiles.ids().iter().any(|id| id == p) => {
            return Err(format!("no profile {p:?}; see `xenify-write profiles`"));
        }
        Some(p) => profiles.get(p),
        None => profiles.get(doc.meta.proofing_profile.as_deref().unwrap_or("standard")),
    };
    let (optional_commas, sentence_sense) = (profile.optional_commas, profile.sentence_sense);
    let mut c = Checker::new(Settings { dialect, profile, ..Default::default() }, &words);
    let mut issues = c.check_document(&doc);
    let model = match spotter {
        "off" => None,
        "auto" => {
            set_bundled_spotter_dir();
            let dir = xenify_wp_check::SpotterPrefs::default().model_dir();
            dir.join("config.json").exists().then(|| xenify_wp_check::Spotter::load(&dir)).transpose()?
        }
        dir => Some(xenify_wp_check::Spotter::load(std::path::Path::new(dir)).map_err(|e| format!("{dir}: {e}"))?),
    };
    if let Some(m) = &model {
        let mut run = xenify_wp_check::SpotterRun::default();
        let ignores = xenify_wp_check::Ignores::of(&doc);
        let opts = xenify_wp_check::SpotOptions {
            threshold: xenify_wp_check::DEFAULT_THRESHOLD,
            optional_commas,
            sentence_sense,
        };
        for p in doc.paragraphs() {
            let found = run.paragraph(m, opts, p.id, &p.text, &issues, &ignores);
            issues.extend(found);
        }
        issues.sort_by_key(|i| (doc.paragraphs().position(|p| p.id == i.para), i.range.start));
    }
    let index: std::collections::HashMap<_, _> =
        doc.paragraphs().enumerate().map(|(i, p)| (p.id, (i + 1, p.text.clone()))).collect();
    let texts: Vec<&str> = doc.paragraphs().map(|p| p.text.as_str()).collect();
    let r = xenify_wp_check::readability(&texts);
    if json {
        let list: Vec<_> = issues
            .iter()
            .map(|i| {
                let (n, t) = &index[&i.para];
                serde_json::json!({
                    "paragraph": n, "start": i.range.start, "end": i.range.end,
                    "text": t.get(i.range.clone()).unwrap_or(""), "category": i.category.name(),
                    "rule": i.rule, "message": i.message, "suggestions": i.suggestions,
                    "feature": i.feature, "strong": i.strong,
                })
            })
            .collect();
        let out = serde_json::json!({"issues": list, "readability": {"words": r.words, "sentences": r.sentences, "grade": r.grade, "ease": r.ease}});
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    } else {
        for i in &issues {
            let (n, t) = &index[&i.para];
            let sug = if i.suggestions.is_empty() {
                String::new()
            } else {
                format!(
                    " → {}",
                    i.suggestions
                        .iter()
                        .map(|s| if s.is_empty() { "(remove)" } else { s })
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
            };
            println!(
                "{}:{n}: [{}] \"{}\" {}{sug}",
                input.display(),
                i.category.name(),
                t.get(i.range.clone()).unwrap_or(""),
                i.message
            );
        }
        println!("{} issues · {} words · grade {:.1} · reading ease {:.0}", issues.len(), r.words, r.grade, r.ease);
    }
    Ok(issues.is_empty())
}

/// The model installed with the app, found from where this program is:
/// next to it (Windows, or the unpacked Linux .tar.gz), in the app's
/// Resources (macOS), or in `/usr/lib/Xenify Write` (Linux packages).
pub fn set_bundled_spotter_dir() {
    let Some(bin) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) else { return };
    for dir in [bin.join("spotter"), bin.join("../Resources/spotter"), bin.join("../lib/Xenify Write/spotter")] {
        if dir.join("config.json").exists() {
            xenify_wp_check::set_bundled_spotter_dir(dir);
            return;
        }
    }
}

/// Runs the command line (`args` starts with a word in [`COMMANDS`]).
pub fn run() -> ExitCode {
    // A release build on Windows has no console of its own: print to the one
    // it was started from.
    #[cfg(windows)]
    // SAFETY: no preconditions; failing (no parent console) is harmless.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Convert { input, output, to } => convert(&input, &output, to.as_deref()),
        Command::Profiles { action } => profiles(action),
        Command::Check { input, json, dialect, profile, spotter } => {
            match check(&input, json, &dialect, profile.as_deref(), &spotter) {
                Ok(true) => return ExitCode::SUCCESS,
                Ok(false) => return ExitCode::from(1),
                Err(e) => Err(e),
            }
        }
        Command::Merge { template, data, to, out, combined, name, range, group_by } => {
            merge(&template, data.as_ref(), &to, &out, combined, name.as_deref(), &range, group_by.as_deref())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xenify-write: {e}");
            ExitCode::FAILURE
        }
    }
}
