//! Xenify Write file formats. Everything works on bytes (no file system),
//! so the same code runs in the CLI, the app and a browser build.

pub mod docx;
pub mod html;
pub mod html_read;
mod igdoc;
pub mod markdown;
pub mod merge_data;
pub mod pdf;
pub mod xml;
pub mod zip;

use xenify_wp_doc::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// Markdown with front matter (the default).
    Markdown,
    /// Xenify Write's own format: a zip with `document.json` and media.
    Igdoc,
    /// Plain text: one paragraph per line.
    Text,
    /// PDF (export only).
    Pdf,
    /// A web page (export only).
    Html,
    /// Microsoft Word.
    Docx,
}

impl Format {
    pub const ALL: [Format; 6] =
        [Format::Markdown, Format::Igdoc, Format::Docx, Format::Text, Format::Pdf, Format::Html];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Markdown => "md",
            Format::Igdoc => "xwrite",
            Format::Text => "txt",
            Format::Pdf => "pdf",
            Format::Html => "html",
            Format::Docx => "docx",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Markdown => "Markdown (.md)",
            Format::Igdoc => "Xenify Write document (.xwrite)",
            Format::Text => "Plain text (.txt)",
            Format::Pdf => "PDF (.pdf)",
            Format::Html => "Web page (.html)",
            Format::Docx => "Document (.docx)",
        }
    }

    /// The format a file name's extension means.
    pub fn from_name(name: &str) -> Option<Format> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        if ext == "htm" {
            return Some(Format::Html);
        }
        if ext == "markdown" {
            return Some(Format::Markdown);
        }
        // Write documents were .igdoc before 2026-10-24.
        if ext == "igdoc" {
            return Some(Format::Igdoc);
        }
        Format::ALL.into_iter().find(|f| f.extension() == ext)
    }

    pub fn can_open(self) -> bool {
        !matches!(self, Format::Pdf)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error(transparent)]
    Zip(#[from] zip::ZipError),
    #[error("this document is damaged: {0}")]
    Damaged(String),
    #[error("this document was made by a newer Xenify Write (format {0}); please update")]
    TooNew(u32),
    #[error("{0}")]
    Unsupported(String),
}

/// A loaded document plus anything the user should know about the import.
pub struct Loaded {
    pub doc: Document,
    pub warnings: Vec<String>,
}

pub fn load(bytes: &[u8], format: Format) -> Result<Loaded, IoError> {
    let mut loaded = match format {
        Format::Markdown => markdown::read(bytes),
        Format::Docx => docx::read(bytes)?,
        Format::Igdoc => igdoc::read(bytes)?,
        Format::Html => Loaded { doc: html_read::read(&String::from_utf8_lossy(bytes)), warnings: Vec::new() },
        Format::Pdf => {
            return Err(IoError::Unsupported(format!("Xenify Write can't open {} files", format.name())));
        }
        Format::Text => {
            Loaded { doc: Document::from_plain_text(&String::from_utf8_lossy(bytes)), warnings: Vec::new() }
        }
    };
    loaded.doc.repair();
    Ok(loaded)
}

/// What saving `doc` in `format` would lose (empty: nothing).
pub fn losses(doc: &Document, format: Format) -> Vec<String> {
    match format {
        Format::Markdown => markdown::losses(doc),
        Format::Igdoc => Vec::new(),
        Format::Text => vec!["all formatting".into()],
        Format::Pdf | Format::Html => Vec::new(),
        Format::Docx => docx_losses(doc),
    }
}

/// Saves or exports `doc`; layout-based formats (PDF) lay it out with
/// `engine` and read pictures with `image` (asset key or path → bytes).
pub fn save_with(
    doc: &Document,
    format: Format,
    engine: &mut xenify_wp_layout::Engine,
    image: &dyn Fn(&str) -> Option<std::sync::Arc<Vec<u8>>>,
) -> Result<Vec<u8>, IoError> {
    let doc = &*if format == Format::Igdoc {
        std::borrow::Cow::Borrowed(doc)
    } else {
        xenify_wp_doc::controls::without_prompts(doc)
    };
    match format {
        Format::Pdf => {
            let mut doc = doc.clone();
            // Print layout even when the document is shown pageless.
            doc.meta.pageless = false;
            let layout = engine.layout(&doc);
            Ok(pdf::write(&doc, &layout, image))
        }
        Format::Docx => {
            // Page numbers for contents tables, from the print layout.
            let mut paged = doc.clone();
            paged.meta.pageless = false;
            engine.layout(&paged);
            Ok(docx::write_with_pages(doc, engine.heading_pages()))
        }
        _ => save(doc, format),
    }
}

/// Write's own file (`Format::Igdoc`) alone: no layout, so nothing that
/// looks for the system's fonts is pulled into a program that only needs
/// this (the server).
pub fn save_igdoc(doc: &Document) -> Vec<u8> {
    igdoc::write(doc)
}

pub fn save(doc: &Document, format: Format) -> Result<Vec<u8>, IoError> {
    // Fields still showing their prompt go out empty (Write's own file keeps them).
    let doc = &*if format == Format::Igdoc {
        std::borrow::Cow::Borrowed(doc)
    } else {
        xenify_wp_doc::controls::without_prompts(doc)
    };
    match format {
        Format::Pdf => {
            let mut engine = xenify_wp_layout::Engine::new(xenify_wp_layout::Fonts::new());
            let assets = doc.assets.clone();
            save_with(doc, format, &mut engine, &move |src| assets.get(src).cloned())
        }
        Format::Markdown => Ok(markdown::write(doc).into_bytes()),
        Format::Html => Ok(html::write(doc).into_bytes()),
        Format::Docx => {
            let mut engine = xenify_wp_layout::Engine::new(xenify_wp_layout::Fonts::new());
            let assets = doc.assets.clone();
            save_with(doc, format, &mut engine, &move |src| assets.get(src).cloned())
        }
        Format::Igdoc => Ok(igdoc::write(doc)),
        Format::Text => Ok(doc.plain_text().into_bytes()),
    }
}

/// What .docx can't keep (almost everything maps).
fn docx_losses(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    if doc.meta.pageless {
        out.push("the pageless view setting".into());
    }
    let frames: Vec<&xenify_wp_doc::Frame> = doc.sections.iter().flat_map(|s| &s.frames).collect();
    if frames.iter().any(|f| f.page > 0) {
        out.push("text boxes on later pages (Word frames sit on the page where their section starts)".into());
    }
    if frames.iter().any(|f| f.next.is_some()) {
        out.push("linked text boxes (the text stays in the first box)".into());
    }
    if frames
        .iter()
        .any(|f| f.blocks.iter().any(|b| matches!(b, xenify_wp_doc::Block::Table(_) | xenify_wp_doc::Block::Toc(_))))
    {
        out.push("tables inside text boxes".into());
    }
    if !doc.meta.audience.is_empty() || !doc.meta.tone.is_empty() {
        out.push("the audience and tone settings".into());
    }
    out
}

/// Relative picture paths used in `doc` (pictures kept as files next to it).
pub fn image_sources(doc: &Document) -> Vec<String> {
    fn go(blocks: &[xenify_wp_doc::Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                xenify_wp_doc::Block::Image(i) if !i.src.contains("://") && !i.src.starts_with('/') => {
                    out.push(i.src.clone())
                }
                xenify_wp_doc::Block::Table(t) => {
                    for row in &t.rows {
                        for c in &row.cells {
                            go(&c.blocks, out);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        go(&s.blocks, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// Reads pictures a document refers to (e.g. `media/x.png` next to a
/// Markdown file) into its assets, so they travel with it.
pub fn attach_images(doc: &mut Document, read: &dyn Fn(&str) -> Option<Vec<u8>>) {
    for src in image_sources(doc) {
        if doc.assets.contains_key(&src) {
            continue;
        }
        if let Some(bytes) = read(&src) {
            doc.assets.insert(src, std::sync::Arc::new(bytes));
        }
    }
}

/// Whether pasted text looks like Markdown (headings, lists, tables), so a
/// paste keeps its structure instead of arriving as plain lines.
pub fn looks_like_markdown(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim_start).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    let marked = lines
        .iter()
        .filter(|l| {
            let heading = l.starts_with('#') && l.trim_start_matches('#').starts_with(' ');
            let bullet = ["* ", "- ", "+ "].iter().any(|b| l.starts_with(b));
            let number =
                l.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            let table = l.starts_with('|') && l.ends_with('|');
            let quote = l.starts_with("> ");
            let fence = l.starts_with("```");
            heading || bullet || number || table || quote || fence
        })
        .count();
    marked * 3 >= lines.len() || text.contains("**") && marked > 0
}
