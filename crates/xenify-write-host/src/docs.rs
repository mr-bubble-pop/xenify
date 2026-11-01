//! The open documents and the calls the screens make on them.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use xenify_wp_check::{Category, Issue};
use xenify_wp_doc::{CharFmt, Document, NodeId, Rgba};
use xenify_wp_engine::{FontTable, Mods, Session, Toggle, templates};
use xenify_wp_io::Format;
use xenify_wp_layout::Fonts;

struct Find {
    query: String,
    case: bool,
    whole: bool,
    hits: Vec<(NodeId, std::ops::Range<usize>)>,
    current: Option<usize>,
}

/// One document tab.
struct Open {
    id: u32,
    session: Session,
    /// What the checker last found, and the text of each paragraph it read
    /// (an issue in a paragraph that has changed since is not shown).
    issues: Vec<Issue>,
    checked: HashMap<NodeId, u64>,
    /// Find: what's searched for and where it was found.
    find: Option<Find>,
    /// Unsaved work kept every minute, apart from the file.
    autosave: crate::recovery::Autosave,
    /// Work left by a run that closed unexpectedly, newer than the file.
    left: Option<(crate::recovery::Info, Document)>,
    path: Option<PathBuf>,
    /// Kept on a Xenify server instead of in a file here.
    remote: Option<crate::remote::Remote>,
    /// Numbers "Untitled 2", "Untitled 3"…
    untitled: u32,
}

impl Open {
    fn title(&self) -> String {
        if let Some(r) = &self.remote {
            return r.name.clone();
        }
        if let Some(p) = &self.path {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            // A template being edited says so on its tab.
            return if p.parent() == Some(templates_dir().as_path()) { format!("{stem} (template)") } else { stem };
        }
        let t = self.session.ed.doc.meta.title.trim();
        if !t.is_empty() {
            return t.to_string();
        }
        if self.untitled > 1 { format!("Untitled {}", self.untitled) } else { "Untitled".into() }
    }
}

pub struct Docs {
    pub prefs: crate::prefs::Prefs,
    /// Mail merge: the records loaded (CSV rows or JSON objects) and their file's name.
    merge: Option<(String, Vec<Value>)>,
    autosave_base: crate::recovery::Autosave,
    fonts: Fonts,
    glyphs: FontTable,
    open: Vec<Open>,
    front: usize,
    next_id: u32,
    untitled: u32,
    /// Reading view is on.
    reading: bool,
    /// Your words on the server you're signed in to (spelling): they follow
    /// you there; the ones on this computer are the checker's own.
    server_words: Vec<String>,
    /// Before a call to a document made from a template: (its tab, the
    /// template's parts' fingerprint, where its history was). A call that
    /// changes those parts is taken back in [`Docs::after`].
    guard: Option<(u32, u64, xenify_wp_doc::Checkpoint)>,
    /// The same for tables kept up to date from a plan or a Calc book: (its
    /// tab, each such table's fingerprint, where its history was). They're
    /// the plan's or book's: a call that changes one is taken back.
    link_guard: Option<(u32, Vec<(NodeId, u64)>, xenify_wp_doc::Checkpoint)>,
    /// Said once with the next answer: a document was made from a template
    /// that was opened ({name, from}).
    made_from: Option<Value>,
}

/// The key to keep picture `bytes` under in a document: `media/<name>`,
/// numbered when the document already has a different picture by that name.
fn free_key(assets: &std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>, name: &str, bytes: &[u8]) -> String {
    let mut key = format!("media/{name}");
    let mut n = 2;
    while assets.get(&key).is_some_and(|b| **b != bytes) {
        key = format!("media/{n}-{name}");
        n += 1;
    }
    key
}

/// A copy of the picture shown from file `src` (relative to `dir`, the
/// document's folder) to keep inside the document (holding `assets`): its
/// key and bytes.
fn picture_copy(
    src: &str,
    dir: Option<&std::path::Path>,
    assets: &std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
) -> Result<(String, Vec<u8>), String> {
    let p = PathBuf::from(src);
    let p = if p.is_absolute() { p } else { dir.ok_or("save the document first")?.join(p) };
    let bytes = read_picture(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("picture".into());
    let mut name = name.replace(['#', ':'], "-");
    if !name.ends_with(".png") && (name.ends_with(".xdraw") || name.ends_with(".xenify") || src.contains('#')) {
        name.push_str(".png");
    }
    Ok((free_key(assets, &name, &bytes), bytes))
}

/// Pictures shown from files on this computer, copied into document `ed`
/// (`dir`: its folder): a document on a server is read by people who don't
/// have those files. Returns how many were copied, and the ones that
/// couldn't be read.
fn copy_in_linked(ed: &mut xenify_wp_doc::Editor, dir: Option<&std::path::Path>) -> (usize, Vec<String>) {
    let mut copied = 0;
    let mut failed = Vec::new();
    for (id, src) in crate::remote::every_picture(&ed.doc) {
        if src.starts_with("media/") || ed.doc.assets.contains_key(&src) || src.contains("://") {
            continue;
        }
        let Some(xenify_wp_doc::Block::Image(mut img)) = ed.doc.block(id).cloned() else { continue };
        match picture_copy(&src, dir, &ed.doc.assets) {
            Ok((key, bytes)) => {
                ed.doc.assets.insert(key.clone(), std::sync::Arc::new(bytes));
                img.src = key;
                ed.seal();
                if xenify_wp_doc::tables::replace_block(ed, id, xenify_wp_doc::Block::Image(img), "Picture").is_ok() {
                    copied += 1;
                }
            }
            Err(e) => failed.push(e),
        }
    }
    (copied, failed)
}

/// Whether unsaved work in `doc` is worth offering back: anything but an
/// empty document (words, or pictures without any words).
fn worth_keeping(doc: &Document) -> bool {
    doc.word_count() > 0 || !crate::remote::every_picture(doc).is_empty()
}

/// A Draw canvas file.
pub fn is_canvas(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xdraw") || e.eq_ignore_ascii_case("xenify"))
}

/// A part of a Draw canvas file (a zip: `preview.png`, `info.json`, …).
fn canvas_part(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut f = z.by_name(name).ok()?;
    let mut out = Vec::new();
    f.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Whether Draw marked a canvas as moving when it saved it.
fn canvas_animated(path: &std::path::Path) -> bool {
    #[derive(serde::Deserialize)]
    struct Info {
        #[serde(default)]
        animated: bool,
    }
    canvas_part(path, "info.json").and_then(|b| serde_json::from_slice::<Info>(&b).ok()).is_some_and(|c| c.animated)
}

/// A picture file's bytes. A Draw canvas (`.xdraw`, earlier `.xenify`) shows the picture Draw
/// keeps in it each time it saves (its `preview`), so a canvas can be shown
/// in a document and follow it as it changes.
pub fn read_picture(path: &std::path::Path) -> Result<Vec<u8>, String> {
    // A chart from a Calc book on this computer: `book.xcalc#<item>`.
    if let Some((book, item)) =
        path.to_str().and_then(|p| p.rsplit_once('#')).filter(|(b, _)| is_calc_book(std::path::Path::new(b)))
    {
        let snap = calc_book_snapshot(std::path::Path::new(book))?;
        let png = snap["items"]
            .as_array()
            .and_then(|items| items.iter().find(|x| x["id"] == item))
            .and_then(|x| x["png"].as_str())
            .ok_or("that chart isn't in the book any more")?;
        use base64::Engine;
        return base64::engine::general_purpose::STANDARD.decode(png.trim()).map_err(|e| e.to_string());
    }
    if !is_canvas(path) {
        return std::fs::read(path).map_err(|e| e.to_string());
    }
    std::fs::metadata(path).map_err(|e| e.to_string())?;
    canvas_part(path, "preview.png")
        .ok_or_else(|| "that canvas has no picture yet: open it in Draw and save it once".into())
}

/// A Calc book (`.xcalc`).
pub fn is_calc_book(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xcalc"))
}

/// What Write can show of a Calc book on this computer: the `link.json`
/// Calc keeps in it each time it saves (the same snapshot it puts on a
/// server for books there), with the file's name and when it changed.
pub fn calc_book_snapshot(path: &std::path::Path) -> Result<Value, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't open that book: {e}"))?;
    let changed = file
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "that file isn't a Calc book".to_string())?;
    let mut text = String::new();
    zip.by_name("link.json")
        .map_err(|_| "that book has nothing to show yet: open it in Calc and save it once".to_string())?
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let mut snap: Value = serde_json::from_str(&text).map_err(|_| "that book's contents can't be read".to_string())?;
    if snap["format"] != "xenify-calc-link" {
        return Err("that book's contents can't be read".into());
    }
    snap["version"] = json!(changed.to_string());
    snap["name"] = json!(path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    Ok(snap)
}

/// A linked table's link: `server|node|item` for a book on a server,
/// `file|path|item` for one on this computer.
fn calc_link_of(args: &Value) -> Result<String, String> {
    let item = arg_str(args, "item")?;
    if let Some(file) = args.get("file").and_then(Value::as_str) {
        return Ok(format!("file|{file}|{item}"));
    }
    Ok(format!(
        "{}|{}|{}",
        arg_str(args, "server")?,
        args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?,
        item
    ))
}

/// A new document from `template` (`name`, found at `from`): the template's
/// parts with its `{{fields}}` to fill in, the body empty, linked to it.
fn from_template(template: Document, name: &str, from: &str) -> Document {
    let mut doc = xenify_wp_doc::linked::new_document(&template, name, from);
    templates::fill_ins(&mut doc);
    doc.meta.title.clear();
    doc
}

/// A template of the person's own, by name (.xwrite, or an older .igdoc).
fn template_file(name: &str) -> PathBuf {
    let new = templates_dir().join(format!("{}.xwrite", safe_name(name)));
    let old = templates_dir().join(format!("{}.igdoc", safe_name(name)));
    if !new.exists() && old.exists() { old } else { new }
}

/// Where a picture of server document `node` is kept in a document.
fn link_key(server: &str, node: i64) -> String {
    let server: String = server.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    format!("media/xen-{server}-{node}.png")
}

/// Tables showing something from a Calc book (Insert › From Calc): their
/// ids and links (`server|node|item`), tables inside cells too.
fn linked_tables(doc: &xenify_wp_doc::Document) -> Vec<(NodeId, String)> {
    fn walk(blocks: &[xenify_wp_doc::Block], out: &mut Vec<(NodeId, String)>) {
        for b in blocks {
            if let xenify_wp_doc::Block::Table(t) = b {
                if let Some(l) = &t.linked {
                    out.push((t.id, l.clone()));
                }
                for r in &t.rows {
                    for c in &r.cells {
                        walk(&c.blocks, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        walk(&s.blocks, &mut out);
    }
    out
}

/// The paragraphs inside linked tables (from a plan or a Calc book).
fn linked_paragraphs(doc: &xenify_wp_doc::Document) -> std::collections::HashSet<NodeId> {
    fn walk(blocks: &[xenify_wp_doc::Block], out: &mut std::collections::HashSet<NodeId>) {
        for b in blocks {
            match b {
                xenify_wp_doc::Block::Paragraph(p) => {
                    out.insert(p.id);
                }
                xenify_wp_doc::Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.blocks, out)),
                _ => {}
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    for (id, _) in linked_tables(doc) {
        if let Some(t) = doc.table(id) {
            t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.blocks, &mut out));
        }
    }
    out
}

/// Each linked table's fingerprint (everything about it: its cells, their
/// text and formatting, and its look).
fn linked_prints(doc: &xenify_wp_doc::Document) -> Vec<(NodeId, u64)> {
    use std::hash::{Hash, Hasher};
    linked_tables(doc)
        .into_iter()
        .filter_map(|(id, _)| {
            let t = doc.table(id)?;
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(t).unwrap_or_default().hash(&mut h);
            Some((id, h.finish()))
        })
        .collect()
}

/// A chart from a Calc book, kept as a picture in the document.
fn calc_picture_key(server: &str, node: i64, item: &str) -> String {
    let clean = |x: &str| x.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>();
    format!("media/xenc-{}-{node}-{}.png", clean(server), clean(item))
}

/// The server and document a linked picture shows.
pub fn parse_link_key(key: &str) -> Option<(String, i64)> {
    let rest = key.strip_prefix("media/xen-")?.strip_suffix(".png")?;
    let (server, node) = rest.rsplit_once('-')?;
    Some((server.to_string(), node.parse().ok()?))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing “{key}”"))
}

/// Where the person's own templates are kept (one .xwrite each, named after it;
/// older ones .igdoc).
fn templates_dir() -> PathBuf {
    xenify_wp_check::config_dir().join("write-templates")
}

/// The person's own templates: (name, document), by name.
fn my_templates() -> Vec<(String, Document)> {
    let Ok(dir) = std::fs::read_dir(templates_dir()) else { return Vec::new() };
    let mut out: Vec<(String, Document)> = dir
        .filter_map(|e| {
            let p = e.ok()?.path();
            if p.extension()? != "xwrite" && p.extension()? != "igdoc" {
                return None;
            }
            let bytes = std::fs::read(&p).ok()?;
            let doc = xenify_wp_io::load(&bytes, Format::Igdoc).ok()?.doc;
            Some((p.file_stem()?.to_string_lossy().into_owned(), doc))
        })
        .collect();
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

/// "This is a template" with starting parts: `title` (a title page of
/// fields), `contents` (a contents page), `numbered` (headings numbered 1,
/// 1.1, 1.1.1), `headers` (the title at the top of each page, page numbers
/// at the bottom). With a title page or contents, an empty paragraph after
/// them is where each document's text goes (unless one is marked already).
fn template_starter(ed: &mut xenify_wp_doc::Editor, parts: &[String]) -> Result<(), xenify_wp_doc::EditError> {
    use xenify_wp_doc::{Block, Container, ParaProps, Paragraph, Step, StyleSheet};
    let has = |p: &str| parts.iter().any(|x| x == p);
    let (title, contents) = (has("title"), has("contents"));
    if has("numbered") {
        xenify_wp_doc::ops::number_headings(ed, true)?;
    }
    if title || contents {
        // An empty document's one paragraph is the body; else a new one goes
        // before what's there (which stays, after the body, as the template's).
        let empty = ed.doc.sections.len() == 1
            && matches!(ed.doc.sections[0].blocks.as_slice(), [Block::Paragraph(p)] if p.text.is_empty());
        let mark_body = ed.doc.meta.body_slot.is_none();
        let sel = ed.sel;
        let mut body = None;
        ed.transact("Make a template", None, |tx| {
            let mut at = 0;
            let put = |tx: &mut xenify_wp_doc::Tx, at: &mut usize, block: Block| {
                tx.step(Step::InsertBlock { container: Container::Section(0), index: *at, block })?;
                *at += 1;
                Ok::<_, xenify_wp_doc::EditError>(())
            };
            let centred = |id, style: &str, text: &str| {
                let mut p = Paragraph::new(id, style.to_string(), text);
                p.props.align = Some(xenify_wp_doc::Align::Center);
                Block::Paragraph(p)
            };
            if title {
                for (style, text) in [
                    (StyleSheet::TITLE, "{{Title}}"),
                    (StyleSheet::SUBTITLE, "{{Subtitle}}"),
                    (StyleSheet::NORMAL, "{{Author}}"),
                    (StyleSheet::NORMAL, "{{Date}}"),
                ] {
                    let id = tx.doc.alloc_id();
                    put(tx, &mut at, centred(id, style, text))?;
                }
            }
            if contents {
                let id = tx.doc.alloc_id();
                let p = Paragraph::new(id, StyleSheet::SUBTITLE.to_string(), "Contents");
                put(tx, &mut at, Block::Paragraph(p))?;
                let id = tx.doc.alloc_id();
                put(tx, &mut at, Block::Toc(xenify_wp_doc::Toc { id, levels: 3, page_numbers: true, ..Default::default() }))?;
            }
            // Where the text goes: on a new page after the contents.
            let body_id = if empty {
                match &tx.doc.sections[0].blocks[at] {
                    Block::Paragraph(p) => p.id,
                    _ => unreachable!("the empty document's paragraph"),
                }
            } else {
                let id = tx.doc.alloc_id();
                put(tx, &mut at, Block::Paragraph(Paragraph::new(id, StyleSheet::NORMAL.to_string(), "")))?;
                id
            };
            if contents {
                let p = tx.doc.para(body_id).ok_or(xenify_wp_doc::EditError::NoParagraph(body_id))?;
                let props = p.props.with(&ParaProps { page_break_before: Some(true), ..Default::default() });
                tx.step(Step::SetProps { para: body_id, props })?;
            }
            if title {
                // The title page is a section of its own, its lines centred top to bottom.
                let page = tx.doc.sections[0].page.clone();
                let start = tx.doc.sections[0].start;
                let sid = tx.doc.alloc_id();
                tx.step(Step::SplitSection {
                    section: 0,
                    at: 4,
                    id: sid,
                    start: xenify_wp_doc::SectionStart::NewPage,
                    page: page.clone(),
                    frames: Vec::new(),
                })?;
                let centred = xenify_wp_doc::PageSetup { v_align: xenify_wp_doc::PageVAlign::Center, ..page };
                tx.step(Step::SetSection { section: 0, start, page: centred })?;
            }
            body = Some(body_id);
            Ok(sel)
        })?;
        if let Some(id) = body.filter(|_| mark_body) {
            let mut meta = ed.doc.meta.clone();
            meta.body_slot = Some(id);
            xenify_wp_doc::ops::set_meta(ed, meta, "Mark where the text goes")?;
        }
        if let Some(id) = body {
            ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(id, 0));
        }
    }
    if has("headers") {
        // Every section but a title page: the title top left, "Page 2 of 5" at the bottom.
        let skip = usize::from(title);
        for i in skip..ed.doc.sections.len() {
            let s = &ed.doc.sections[i];
            let mut page = s.page.clone();
            page.header = xenify_wp_doc::HfLine { left: "{title}".into(), ..Default::default() };
            page.footer = xenify_wp_doc::HfLine { center: "Page {page} of {pages}".into(), ..Default::default() };
            page.link_to_previous = false;
            xenify_wp_doc::ops::set_section(ed, i, s.start, page)?;
        }
    }
    let mut meta = ed.doc.meta.clone();
    meta.is_template = true;
    xenify_wp_doc::ops::set_meta(ed, meta, "Make a template")
}

/// A new document from a template id: a built-in example, or "mine:NAME".
fn template_document(id: &str) -> Option<Document> {
    match id.strip_prefix("mine:") {
        Some(name) => my_templates().into_iter().find(|(n, _)| n == name).map(|(_, d)| d),
        None => templates::document(id),
    }
}

/// The first words of a document (to tell untitled ones apart).
fn opening_words(d: &Document) -> String {
    // Pictures and other inline objects are U+FFFC in the text.
    let clean =
        |t: &str| t.chars().filter(|c| *c != '\u{FFFC}' && !c.is_control()).collect::<String>().trim().to_string();
    let text = d.paragraphs().map(|p| clean(&p.text)).find(|t| !t.is_empty()).unwrap_or_default();
    let mut out: String = text.chars().take(90).collect();
    if out.len() < text.len() {
        out = format!("{}…", out.trim_end());
    }
    out
}

fn arg_f64(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key).and_then(Value::as_f64).ok_or_else(|| format!("missing “{key}”"))
}

fn mods(args: &Value) -> Mods {
    args.get("mods").and_then(|m| serde_json::from_value(m.clone()).ok()).unwrap_or_default()
}

impl Docs {
    pub fn new(files: &[String]) -> Self {
        let prefs = crate::prefs::Prefs::load();
        let reopen: Vec<String> =
            if files.is_empty() && prefs.start == "reopen" { prefs.open_files.clone() } else { Vec::new() };
        let mut docs = Self {
            merge: None,
            // This run's number names autosaves of documents not saved yet.
            autosave_base: {
                let mut a = crate::recovery::Autosave::default();
                a.session = crate::recovery::now_ms();
                a
            },
            prefs,
            fonts: Fonts::new(),
            glyphs: FontTable::default(),
            open: Vec::new(),
            front: 0,
            next_id: 1,
            untitled: 0,
            reading: false,
            server_words: Vec::new(),
            guard: None,
            link_guard: None,
            made_from: None,
        };
        for f in files.iter().chain(&reopen) {
            if let Err(e) = docs.open_file(PathBuf::from(f)) {
                eprintln!("xenify-write-app: {f}: {e}");
            }
        }
        if docs.open.is_empty() {
            docs.add(Document::default(), None);
        }
        docs
    }

    fn add(&mut self, doc: Document, path: Option<PathBuf>) {
        let untitled = if path.is_none() {
            self.untitled += 1;
            self.untitled
        } else {
            0
        };
        let id = self.next_id;
        self.next_id += 1;
        let mut doc = doc;
        doc.user = self.user_name();
        let mut session = Session::new(doc, self.fonts.clone());
        session.set_reading(self.reading);
        session.autoformat = self.prefs.autoformat.clone();
        session.autocorrect = self.prefs.autocorrect.clone();
        let mut autosave = self.autosave_base.sibling(id as u64);
        autosave.bind(path.as_deref());
        let left = path.as_deref().and_then(|p| autosave.left_for(p));
        self.open.push(Open {
            id,
            session,
            path,
            remote: None,
            untitled,
            issues: Vec::new(),
            checked: HashMap::new(),
            find: None,
            autosave,
            left,
        });
        self.front = self.open.len() - 1;
    }

    fn open_file(&mut self, path: PathBuf) -> Result<(), String> {
        self.open_file_as(path, false)
    }

    /// Opens `path`; a template (unless `edit`) gives a new document made from it.
    fn open_file_as(&mut self, path: PathBuf, edit: bool) -> Result<(), String> {
        if let Some(i) = self.open.iter().position(|o| o.path.as_ref() == Some(&path)) {
            self.front = i;
            return Ok(());
        }
        let format = Format::from_name(&path.to_string_lossy()).ok_or("Xenify Write doesn't know this kind of file")?;
        let bytes = std::fs::read(&path).map_err(|e| format!("couldn't read the file: {e}"))?;
        let loaded = xenify_wp_io::load(&bytes, format).map_err(|e| e.to_string())?;
        let mut doc = loaded.doc;
        let dir = path.parent().map(|p| p.to_path_buf());
        xenify_wp_io::attach_images(&mut doc, &|src| dir.as_ref().and_then(|d| read_picture(&d.join(src)).ok()));
        size_pictures(&mut doc);
        // A blank, untouched tab is replaced rather than kept.
        if self.open.len() == 1
            && self.open[0].path.is_none()
            && !self.open[0].session.ed.is_dirty()
            && self.open[0].session.ed.doc.word_count() == 0
        {
            self.open.clear();
        }
        if doc.meta.is_template && !edit {
            let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let from = format!("file:{}", path.display());
            self.add(from_template(doc, &name, &from), None);
            self.made_from = Some(json!({ "name": name, "from": from }));
            return Ok(());
        }
        self.prefs.remember(&path.display().to_string());
        self.prefs.save();
        self.add(doc, Some(path));
        Ok(())
    }

    /// The front document's template, when Write can read it itself (one of
    /// the person's own, or a file on this computer).
    fn template_here(&self) -> Option<Document> {
        let link = self.open[self.front].session.ed.doc.meta.template.as_ref()?;
        if let Some(path) = link.from.strip_prefix("file:") {
            let bytes = std::fs::read(path).ok()?;
            return xenify_wp_io::load(&bytes, Format::Igdoc).ok().map(|l| l.doc);
        }
        let name = link.from.strip_prefix("mine:").unwrap_or(if link.from.is_empty() { &link.name } else { "" });
        my_templates().into_iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }

    /// Brings `template` (as it is now) into the front document: its parts,
    /// styles and page setup; fields keep what was typed in them. One undo step.
    fn refresh_from(&mut self, template: Document) -> Result<bool, String> {
        let ed = &mut self.open[self.front].session.ed;
        let Some(link) = ed.doc.meta.template.clone() else { return Ok(false) };
        if link.version == xenify_wp_doc::linked::version(&template) {
            return Ok(false);
        }
        if xenify_wp_doc::linked::slot(&template).is_none() {
            // Only its look (a template without a place for the text marked).
            xenify_wp_doc::linked::apply(ed, &template, &link.name).map_err(|e| e.to_string())?;
            return Ok(true);
        }
        let mut t = template;
        t.renumber(ed.doc.next_id);
        templates::fill_ins(&mut t);
        let next = t.next_id.max(ed.doc.next_id);
        let new = xenify_wp_doc::linked::refreshed(&ed.doc, &t).ok_or("the template has no place for the text")?;
        // The template's pictures (its logo, say) come with its parts.
        let pictures = t.assets.clone();
        ed.doc.next_id = next;
        let content =
            xenify_wp_doc::Content { sections: new.sections, styles: new.styles, lists: new.lists, meta: new.meta };
        ed.seal();
        ed.transact("Update from template", None, |tx| {
            tx.step(xenify_wp_doc::Step::SetContent { content: Box::new(content) })?;
            Ok(xenify_wp_doc::Selection::caret(tx.doc.start()))
        })
        .map_err(|e| e.to_string())?;
        ed.seal();
        for (name, bytes) in pictures {
            ed.doc.assets.entry(name).or_insert(bytes);
        }
        Ok(true)
    }

    /// A change to a template's part of a document (not one of its fields)
    /// is taken back, with why.
    fn keep_template_parts(&mut self) -> Option<String> {
        let (id, print, cp) = self.guard.take()?;
        let o = self.open.iter_mut().find(|o| o.id == id)?;
        if xenify_wp_doc::linked::fixed_print(&o.session.ed.doc) == Some(print) {
            return None;
        }
        o.session.ed.roll_back(cp);
        let name = o.session.ed.doc.meta.template.as_ref().map(|t| t.name.clone()).unwrap_or_default();
        Some(format!(
            "This part comes from the template “{name}”: only its fields can be typed in. File › Detach from template makes the document your own."
        ))
    }

    /// Puts back a linked table (from a plan or a Calc book) that the last
    /// call changed: what's in it comes from there, and the next update
    /// would undo the change anyway. Deleting the whole table is fine.
    fn keep_linked_tables(&mut self) -> Option<String> {
        let (id, before, cp) = self.link_guard.take()?;
        let o = self.open.iter_mut().find(|o| o.id == id)?;
        let now = linked_prints(&o.session.ed.doc);
        let changed = before.iter().any(|(t, h)| now.iter().any(|(u, g)| u == t && g != h));
        if !changed {
            return None;
        }
        o.session.ed.roll_back(cp);
        Some("This table comes from a plan or a Calc book and is kept up to date from it: change it there (its ⋯ › Open it). To change it here instead, ⋯ › Keep this copy (stop updating).".to_string())
    }

    /// Your name on comments and tracked changes (Settings › General).
    fn user_name(&self) -> String {
        let n = self.prefs.name.trim();
        if n.is_empty() { "You".into() } else { n.to_string() }
    }

    fn merge_info(&self) -> Value {
        let fields = xenify_wp_doc::merge::fields(&self.open[self.front].session.ed.doc);
        match &self.merge {
            Some((name, records)) => {
                let mut columns: Vec<String> = Vec::new();
                for r in records.iter().take(50) {
                    if let Some(o) = r.as_object() {
                        for k in o.keys() {
                            if !columns.contains(k) {
                                columns.push(k.clone());
                            }
                        }
                    }
                }
                json!({ "file": name, "records": records.len(), "columns": columns, "used": fields })
            }
            None => json!({ "file": null, "records": 0, "columns": [], "used": fields }),
        }
    }

    /// Puts a picture (its file's bytes) in at the caret, kept in the document.
    fn put_picture(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .ok_or("that isn't a picture Write can show (PNG, JPEG, WebP, GIF or BMP)")?;
        let o = self.doc();
        let doc = &mut o.session.ed.doc;
        let key = free_key(&doc.assets, name, &bytes);
        doc.assets.insert(key.clone(), std::sync::Arc::new(bytes));
        // 96 dpi pictures at their size, shrunk to fit the text width by the layout.
        xenify_wp_doc::tables::insert_image(&mut o.session.ed, &key, w as f32 * 0.75, h as f32 * 0.75)
            .map_err(|e| e.to_string())
    }

    /// The front document's editor (for the AI, which reads but doesn't change).
    pub fn front_editor(&self) -> &xenify_wp_doc::Editor {
        &self.open[self.front].session.ed
    }

    pub fn any_dirty(&self) -> bool {
        self.open.iter().any(|o| o.session.ed.is_dirty())
    }

    fn doc(&mut self) -> &mut Open {
        &mut self.open[self.front]
    }

    fn tabs(&self) -> Value {
        json!({
            "front": self.open[self.front].id,
            "tabs": self.open.iter().map(|o| json!({
                "id": o.id,
                "title": o.title(),
                "dirty": o.session.ed.is_dirty() && o.remote.as_ref().is_none_or(|r| r.live.is_none()),
                "path": o.path.as_ref().map(|p| p.display().to_string()),
                "remote": o.remote.as_ref().map(|r| r.live.is_some()),
            })).collect::<Vec<_>>(),
        })
    }

    /// Keeps "the files open now" for reopening next time.
    fn note_open_files(&mut self) {
        let files: Vec<String> =
            self.open.iter().filter_map(|o| o.path.as_ref()).map(|p| p.display().to_string()).collect();
        if files != self.prefs.open_files {
            self.prefs.open_files = files;
            self.prefs.save();
        }
    }

    /// The view after a change, with the tab strip (titles and unsaved dots change too).
    fn after(&mut self) -> Value {
        let kept = self.keep_template_parts().or_else(|| self.keep_linked_tables());
        self.note_open_files();
        let view = self.doc().session.view();
        let find = self.refresh_find();
        // Editing together: my changes go out as soon as they're made.
        let live: Vec<Value> = self
            .open
            .iter_mut()
            .filter_map(|o| {
                let r = o.remote.as_mut()?;
                r.outgoing(&mut o.session.ed).map(|b| json!({ "id": o.id, "batch": b }))
            })
            .collect();
        let mut out = json!({ "view": view, "tabs": self.tabs(), "details": self.details_summary(), "marks": self.marks(), "find": find });
        if !live.is_empty() {
            out["live"] = json!(live);
        }
        // A change put back (may only read or comment): say why.
        if let Some(why) = self.open.iter_mut().find_map(|o| o.remote.as_mut()?.refused.take()) {
            out["refused"] = json!(why);
        }
        if let Some(why) = kept {
            out["refused"] = json!(why);
        }
        if let Some(m) = self.made_from.take() {
            out["made_from_template"] = m;
        }
        out
    }

    fn by_id(&mut self, args: &Value) -> Result<usize, String> {
        let id = args.get("id").and_then(Value::as_u64).ok_or("missing “id”")? as u32;
        self.open.iter().position(|o| o.id == id).ok_or_else(|| "that document isn't open".to_string())
    }

    /// The calls for documents on a Xenify server (`remote_…`, `live_…`).
    /// None: not one of them.
    fn remote_call(&mut self, name: &str, args: &Value) -> Option<Result<Value, String>> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        Some((|| -> Result<Value, String> {
            match name {
                "remote_open" => {
                    let remote = crate::remote::Remote {
                        server: arg_str(args, "server")?.to_string(),
                        node: args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?,
                        name: arg_str(args, "name")?.to_string(),
                        version: args.get("version").and_then(Value::as_i64).unwrap_or(0),
                        upto: args.get("upto").and_then(Value::as_i64).unwrap_or(0),
                        live: None,
                        on_server: Default::default(),
                        saving: Default::default(),
                        access: Default::default(),
                        last: None,
                        refused: None,
                    };
                    // Comments here carry the name the server knows you by.
                    let author = args.get("user").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty());
                    // Nothing saved in it yet (made on the Files page): a new, blank document.
                    let blank = args.get("blank").and_then(Value::as_bool).unwrap_or(false);
                    let doc = match args.get("data").and_then(Value::as_str) {
                        Some(d) if !d.is_empty() => {
                            let bytes = b64.decode(d).map_err(|e| e.to_string())?;
                            let mut doc = xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc;
                            size_pictures(&mut doc);
                            doc
                        }
                        _ if blank => template_document("blank").ok_or("that template is missing")?,
                        // A new file on the server: start from the front document.
                        _ => self.open[self.front].session.ed.doc.clone(),
                    };
                    if blank || args.get("data").and_then(Value::as_str).is_some_and(|d| !d.is_empty()) {
                        self.add(doc, None);
                    }
                    // Going onto the server: pictures from files here are copied in.
                    let mut copied = (0, Vec::new());
                    if !args.get("data").and_then(Value::as_str).is_some_and(|d| !d.is_empty()) {
                        let dir = self.open[self.front].path.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                        copied = copy_in_linked(&mut self.open[self.front].session.ed, dir.as_deref());
                    }
                    let o = &mut self.open[self.front];
                    if let Some(a) = author {
                        o.session.ed.doc.user = a.to_string();
                    }
                    let mut remote = remote;
                    remote.on_server = o.session.ed.doc.assets.keys().filter(|_| args.get("data").and_then(Value::as_str).is_some_and(|d| !d.is_empty())).cloned().collect();
                    o.remote = Some(remote);
                    o.session.ed.mark_saved();
                    let mut out = self.after();
                    out["id"] = json!(self.open[self.front].id);
                    out["copied"] = json!(copied.0);
                    out["not_copied"] = json!(copied.1);
                    Ok(out)
                }
                "remote_copy_in" => {
                    // A picture from a file here went into a server document: copied in.
                    let dir = self.open[self.front].path.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                    let (n, failed) = copy_in_linked(&mut self.open[self.front].session.ed, dir.as_deref());
                    let mut out = self.after();
                    out["copied"] = json!(n);
                    out["not_copied"] = json!(failed);
                    Ok(out)
                }
                "remote_save" => {
                    // The front (or `id`) document as the server keeps it.
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let o = &mut self.open[i];
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    // `mine`: with the changes made here not on the server yet
                    // (kept to send later), not the server's own copy.
                    let mine = args.get("mine").and_then(Value::as_bool).unwrap_or(false);
                    let mut doc = if mine { o.session.ed.doc.clone() } else { r.server_copy(&o.session.ed).clone() };
                    // Pictures don't travel with the changes: the ones here
                    // (added, or fetched afresh) go in with the version.
                    for (k, v) in &o.session.ed.doc.assets {
                        doc.assets.insert(k.clone(), v.clone());
                    }
                    r.saving = doc.assets.keys().cloned().collect();
                    let bytes = xenify_wp_io::save(&doc, Format::Igdoc).map_err(|e| e.to_string())?;
                    // The words, for the server's search.
                    let text = format!("{}\n{}", doc.meta.title, doc.plain_text());
                    Ok(json!({
                        "id": o.id, "node": r.node, "base": r.version, "upto": r.seq(),
                        "data": b64.encode(&bytes), "text": text,
                    }))
                }
                "remote_saved" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    r.version = args.get("version").and_then(Value::as_i64).unwrap_or(r.version);
                    let saved = std::mem::take(&mut r.saving);
                    r.on_server.extend(saved);
                    // Editing together, everything goes to the server as it's
                    // made; a saved version is only a snapshot of it.
                    if r.live.is_none() {
                        o.session.ed.mark_saved();
                    }
                    o.autosave.clear();
                    Ok(self.after())
                }
                "remote_needs" => {
                    // Pictures and editing together (Remote::needs).
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let o = &self.open[i];
                    let r = o.remote.as_ref().ok_or("this document isn't on a server")?;
                    let (upload, missing) = r.needs(&o.session.ed);
                    Ok(json!({ "upload": upload, "missing": missing }))
                }
                "remote_picture" => {
                    // One picture's bytes, to put up on the server.
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let name = arg_str(args, "name")?;
                    let bytes = self.open[i].session.ed.doc.assets.get(name).ok_or("no such picture")?;
                    Ok(json!({ "data": b64.encode(bytes.as_slice()) }))
                }
                "remote_uploaded" => {
                    // Pictures now on the server.
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let r = self.open[i].remote.as_mut().ok_or("this document isn't on a server")?;
                    for n in args.get("names").and_then(Value::as_array).into_iter().flatten() {
                        if let Some(n) = n.as_str() {
                            r.on_server.insert(n.to_string());
                        }
                    }
                    Ok(json!({}))
                }
                "remote_take_picture" => {
                    // A picture someone else put in, fetched by itself.
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let name = arg_str(args, "name")?.to_string();
                    let bytes = b64.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?;
                    let o = &mut self.open[i];
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    r.take_picture(&mut o.session.ed, &name, bytes);
                    let mut out = self.after();
                    out["taken"] = json!([name]);
                    Ok(out)
                }
                "remote_take_pictures" => {
                    // The pictures missing here, from the server's latest version.
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let bytes = b64.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?;
                    let doc = xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc;
                    let o = &mut self.open[i];
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    let taken = r.take_pictures(&mut o.session.ed, &doc);
                    let mut out = self.after();
                    out["taken"] = json!(taken);
                    Ok(out)
                }
                // Pictures of documents on a server (a Draw canvas in a Write
                // document): kept as `media/xen-<server>-<node>.png`, so the
                // last picture fetched is saved with the document and shows
                // offline; refreshed when the original changes.
                "link_insert" => {
                    let key = link_key(arg_str(args, "server")?, args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?);
                    let bytes = b64.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?;
                    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
                        .with_guessed_format()
                        .ok()
                        .and_then(|r| r.into_dimensions().ok())
                        .ok_or("that picture can't be shown")?;
                    let o = self.doc();
                    o.session.ed.doc.assets.insert(key.clone(), std::sync::Arc::new(bytes));
                    xenify_wp_doc::tables::insert_image(&mut o.session.ed, &key, w as f32 * 0.75, h as f32 * 0.75).map_err(|e| e.to_string())?;
                    Ok(self.after())
                }
                "links" => {
                    let mut out: Vec<Value> = Vec::new();
                    for o in &self.open {
                        for key in o.session.ed.doc.assets.keys() {
                            if let Some((server, node)) = parse_link_key(key)
                                && !out.iter().any(|v| v["server"] == server && v["node"] == node)
                            {
                                out.push(json!({ "server": server, "node": node }));
                            }
                        }
                    }
                    Ok(json!(out))
                }
                "link_update" => {
                    let key = link_key(arg_str(args, "server")?, args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?);
                    let bytes = std::sync::Arc::new(b64.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?);
                    // Its shape may differ (the first real picture after the empty box): as wide, as tall as it needs.
                    let shape = image::ImageReader::new(std::io::Cursor::new(&bytes[..]))
                        .with_guessed_format()
                        .ok()
                        .and_then(|r| r.into_dimensions().ok())
                        .filter(|(w, h)| *w > 0 && *h > 0);
                    for o in &mut self.open {
                        let doc = &mut o.session.ed.doc;
                        if doc.assets.contains_key(&key) {
                            if let Some((w, h)) = shape {
                                doc.for_each_image_mut(&mut |i| {
                                    if i.src == key && i.width > 0.0 {
                                        i.height = i.width * h as f32 / w as f32;
                                    }
                                });
                            }
                            doc.assets.insert(key.clone(), bytes.clone());
                            doc.revision += 1;
                            let rev = doc.revision;
                            doc.for_each_para_mut(&mut |p| p.rev = rev);
                        }
                    }
                    let mut out = self.after();
                    out["key"] = json!(key);
                    Ok(out)
                }
                // Tables (and charts, as pictures) from a Calc book on a server,
                // kept up to date: Insert › From Calc.
                "calc_link_insert" => {
                    let link = calc_link_of(args)?;
                    let rows: Vec<Vec<String>> = serde_json::from_value(args.get("rows").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
                    let numbers: Vec<bool> = serde_json::from_value(args.get("numbers").cloned().unwrap_or_default()).unwrap_or_default();
                    let (header, footer) = (args["header"].as_bool().unwrap_or(false), args["footer"].as_bool().unwrap_or(false));
                    let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                    let ed = &mut self.doc().session.ed;
                    // Right after another table (several parts put in at once): a blank
                    // line between them, so they don't run together.
                    let at = ed.sel.focus.para;
                    if ed.doc.para(at).is_some_and(|p| p.text.is_empty())
                        && let Some((c, i)) = ed.doc.locate(at)
                        && i > 0
                        && matches!(ed.doc.blocks(&c).and_then(|b| b.get(i - 1)), Some(xenify_wp_doc::Block::Table(_)))
                    {
                        xenify_wp_doc::ops::split_paragraph(ed).map_err(|e| e.to_string())?;
                    }
                    xenify_wp_doc::tables::insert_table(ed, rows.len().max(1), cols).map_err(|e| e.to_string())?;
                    let (id, _, _) = xenify_wp_doc::tables::cell_at(ed, ed.sel.focus.para).ok_or("the table didn't go in")?;
                    xenify_wp_doc::tables::fill_table(ed, id, &rows, header, footer, &numbers).map_err(|e| e.to_string())?;
                    xenify_wp_doc::tables::style_linked(ed, id).map_err(|e| e.to_string())?;
                    xenify_wp_doc::tables::set_linked(ed, id, Some(link)).map_err(|e| e.to_string())?;
                    // The caret goes after the table: typing in it would be lost at the next update.
                    if let Some((c, i)) = ed.doc.locate(id)
                        && let Some(xenify_wp_doc::Block::Paragraph(p)) = ed.doc.blocks(&c).and_then(|b| b.get(i + 1))
                    {
                        ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(p.id, 0));
                    }
                    Ok(self.after())
                }
                // Task chips from Plan (Insert › From Plan › Tasks): badges linking to
                // xenify-plan://<server>/<node>/<task>, kept up to date like Calc's tables.
                "plan_task_insert" => {
                    let link = format!("xenify-plan://{}/{}/{}", arg_str(args, "server")?, args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?, arg_str(args, "task")?);
                    let field = xenify_wp_doc::Field::Badge { text: arg_str(args, "text")?.to_string(), color: arg_str(args, "color")?.to_string() };
                    xenify_wp_doc::ops::insert_linked_field(&mut self.doc().session.ed, field, &link, "Task from Plan").map_err(|e| e.to_string())?;
                    Ok(self.after())
                }
                "plan_task_links" => {
                    let mut out: Vec<Value> = Vec::new();
                    for o in &mut self.open {
                        o.session.ed.doc.for_each_para_mut(&mut |p| {
                            for r in &p.runs {
                                if let (Some(xenify_wp_doc::Field::Badge { .. }), Some(l)) = (&r.fmt.field, &r.fmt.link)
                                    && let Some(rest) = l.strip_prefix("xenify-plan://")
                                {
                                    let mut parts = rest.splitn(3, '/');
                                    let (server, node, task) = (parts.next().unwrap_or(""), parts.next().and_then(|n| n.parse::<i64>().ok()).unwrap_or(0), parts.next().unwrap_or(""));
                                    let v = json!({ "server": server, "node": node, "task": task });
                                    if !out.contains(&v) {
                                        out.push(v);
                                    }
                                }
                            }
                        });
                    }
                    Ok(json!(out))
                }
                "plan_task_update" => {
                    let link = format!("xenify-plan://{}/{}/{}", arg_str(args, "server")?, args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?, arg_str(args, "task")?);
                    let (text, color) = (arg_str(args, "text")?.to_string(), arg_str(args, "color")?.to_string());
                    for o in &mut self.open {
                        let doc = &mut o.session.ed.doc;
                        let mut changed = false;
                        doc.for_each_para_mut(&mut |p| {
                            for r in &mut p.runs {
                                if r.fmt.link.as_deref() == Some(link.as_str())
                                    && let Some(xenify_wp_doc::Field::Badge { text: t, color: c }) = &mut r.fmt.field
                                    && (*t != text || *c != color)
                                {
                                    *t = text.clone();
                                    *c = color.clone();
                                    changed = true;
                                }
                            }
                        });
                        if changed {
                            doc.revision += 1;
                            let rev = doc.revision;
                            doc.for_each_para_mut(&mut |p| p.rev = rev);
                        }
                    }
                    Ok(self.after())
                }
                "calc_links" => {
                    let mut out: Vec<Value> = Vec::new();
                    for o in &self.open {
                        for (_, l) in linked_tables(&o.session.ed.doc) {
                            // A book on this computer: its path and item.
                            if let Some(rest) = l.strip_prefix("file|") {
                                let (file, item) = rest.rsplit_once('|').unwrap_or((rest, ""));
                                let v = json!({ "file": file, "item": item });
                                if !out.contains(&v) {
                                    out.push(v);
                                }
                                continue;
                            }
                            let mut parts = l.splitn(3, '|');
                            let (server, node, item) = (parts.next().unwrap_or(""), parts.next().and_then(|n| n.parse::<i64>().ok()).unwrap_or(0), parts.next().unwrap_or(""));
                            if !out.iter().any(|v| v["server"] == server && v["node"] == node && v["item"] == item) {
                                out.push(json!({ "server": server, "node": node, "item": item }));
                            }
                        }
                        for key in o.session.ed.doc.assets.keys() {
                            if let Some(rest) = key.strip_prefix("media/xenc-").and_then(|r| r.strip_suffix(".png")) {
                                let mut parts = rest.splitn(3, '-');
                                let (server, node, item) = (parts.next().unwrap_or(""), parts.next().and_then(|n| n.parse::<i64>().ok()).unwrap_or(0), parts.next().unwrap_or(""));
                                if !out.iter().any(|v| v["server"] == server && v["node"] == node && v["picture"] == item) {
                                    out.push(json!({ "server": server, "node": node, "picture": item }));
                                }
                            }
                        }
                    }
                    Ok(json!(out))
                }
                "calc_link_update" => {
                    let link = calc_link_of(args)?;
                    let rows: Vec<Vec<String>> = serde_json::from_value(args.get("rows").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
                    let numbers: Vec<bool> = serde_json::from_value(args.get("numbers").cloned().unwrap_or_default()).unwrap_or_default();
                    let (header, footer) = (args["header"].as_bool().unwrap_or(false), args["footer"].as_bool().unwrap_or(false));
                    for o in &mut self.open {
                        let ids: Vec<NodeId> = linked_tables(&o.session.ed.doc).into_iter().filter(|(_, l)| *l == link).map(|(id, _)| id).collect();
                        for id in ids {
                            let same = o.session.ed.doc.table(id).is_some_and(|t| {
                                t.rows.len() == rows.len().max(1)
                                    && t.rows.iter().zip(&rows).all(|(r, texts)| r.cells.len() == texts.len().max(1) && r.cells.iter().zip(texts).all(|(c, x)| xenify_wp_doc::tables::cell_text(c) == *x))
                            });
                            if !same {
                                xenify_wp_doc::tables::fill_table(&mut o.session.ed, id, &rows, header, footer, &numbers).map_err(|e| e.to_string())?;
                                xenify_wp_doc::tables::set_linked(&mut o.session.ed, id, Some(link.clone())).map_err(|e| e.to_string())?;
                            }
                        }
                    }
                    Ok(self.after())
                }
                // A Calc book on this computer: what Write can show of it.
                // `file` is relative to the document's folder, or absolute.
                "calc_file_snapshot" => {
                    let file = PathBuf::from(arg_str(args, "file")?);
                    let full = if file.is_absolute() { file } else { self.open[self.front].path.as_ref().and_then(|p| p.parent()).ok_or("save the document first")?.join(file) };
                    calc_book_snapshot(&full)
                }
                // How to name a book on this computer in a link: relative to
                // the document's folder when it's in or under it.
                "calc_file_name" => {
                    let file = PathBuf::from(arg_str(args, "file")?);
                    let dir = self.open[self.front].path.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                    Ok(json!(match dir.as_ref().and_then(|d| file.strip_prefix(d).ok()) {
                        Some(rel) => rel.display().to_string(),
                        None => file.display().to_string(),
                    }))
                }
                // A chart from a book on this computer: shown from the file
                // (`book.xcalc#<item>`), so it follows the book like a Draw canvas.
                "calc_file_picture_insert" => {
                    let (file, item) = (arg_str(args, "file")?, arg_str(args, "item")?);
                    let src = format!("{file}#{item}");
                    let full = {
                        let p = PathBuf::from(&src);
                        if p.is_absolute() { p } else { self.open[self.front].path.as_ref().and_then(|d| d.parent()).ok_or("save the document first")?.join(p) }
                    };
                    let bytes = read_picture(&full)?;
                    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
                        .with_guessed_format()
                        .ok()
                        .and_then(|r| r.into_dimensions().ok())
                        .ok_or("that chart can't be shown")?;
                    // Drawn at twice the size: shown at half, crisp.
                    xenify_wp_doc::tables::insert_image(&mut self.doc().session.ed, &src, w as f32 * 0.375, h as f32 * 0.375).map_err(|e| e.to_string())?;
                    Ok(self.after())
                }
                "calc_picture_insert" => {
                    let key = calc_picture_key(arg_str(args, "server")?, args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?, arg_str(args, "item")?);
                    let bytes = b64.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?;
                    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
                        .with_guessed_format()
                        .ok()
                        .and_then(|r| r.into_dimensions().ok())
                        .ok_or("that chart can't be shown")?;
                    let o = self.doc();
                    o.session.ed.doc.assets.insert(key.clone(), std::sync::Arc::new(bytes));
                    // Drawn at twice the size: shown at half, crisp.
                    let ed = &mut o.session.ed;
                    xenify_wp_doc::tables::insert_image(ed, &key, w as f32 * 0.375, h as f32 * 0.375).map_err(|e| e.to_string())?;
                    // The caret goes after the picture (it stays before it when it went in
                    // after a line of text), so what goes in next comes after it.
                    if let Some((c, i)) = ed.doc.locate(ed.sel.focus.para)
                        && let Some(blocks) = ed.doc.blocks(&c)
                        && matches!(blocks.get(i + 1), Some(xenify_wp_doc::Block::Image(img)) if img.src == key)
                        && let Some(xenify_wp_doc::Block::Paragraph(p)) = blocks.get(i + 2)
                    {
                        ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(p.id, 0));
                    }
                    Ok(self.after())
                }
                "calc_picture_update" => {
                    let key = calc_picture_key(arg_str(args, "server")?, args.get("node").and_then(Value::as_i64).ok_or("missing “node”")?, arg_str(args, "item")?);
                    let bytes = std::sync::Arc::new(b64.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?);
                    for o in &mut self.open {
                        let doc = &mut o.session.ed.doc;
                        if doc.assets.get(&key).is_some_and(|b| **b != *bytes) {
                            doc.assets.insert(key.clone(), bytes.clone());
                            doc.revision += 1;
                            let rev = doc.revision;
                            doc.for_each_para_mut(&mut |p| p.rev = rev);
                        }
                    }
                    let mut out = self.after();
                    out["key"] = json!(key);
                    Ok(out)
                }
                // A linked table becomes the document's own (stops updating).
                "calc_link_keep" => {
                    let id = NodeId(arg_f64(args, "id")? as u64);
                    xenify_wp_doc::tables::set_linked(&mut self.doc().session.ed, id, None).map_err(|e| e.to_string())?;
                    Ok(self.after())
                }
                "link_keep" => {
                    // Stop updating: the picture becomes the document's own.
                    let id = NodeId(arg_f64(args, "id")? as u64);
                    let ed = &mut self.doc().session.ed;
                    let Some(xenify_wp_doc::Block::Image(mut img)) = ed.doc.block(id).cloned() else {
                        return Err("that picture is gone".into());
                    };
                    let bytes = ed.doc.assets.get(&img.src).cloned().ok_or("that picture is gone")?;
                    let mut key = format!("media/picture-{}.png", id.0);
                    let mut n = 2;
                    while ed.doc.assets.contains_key(&key) {
                        key = format!("media/picture-{}-{n}.png", id.0);
                        n += 1;
                    }
                    ed.doc.assets.insert(key.clone(), bytes);
                    img.src = key;
                    ed.seal();
                    xenify_wp_doc::tables::replace_block(ed, id, xenify_wp_doc::Block::Image(img), "Picture").map_err(|e| e.to_string())?;
                    Ok(self.after())
                }
                // Where my caret and selection are in server document `id`,
                // for the others editing it (a `say` message).
                "live_say" => {
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let o = &self.open[i];
                    let r = o.remote.as_ref().ok_or("this document isn't on a server")?;
                    let sel = o.session.ed.sel;
                    let at = json!({ "a": [sel.anchor.para.0, sel.anchor.offset], "f": [sel.focus.para.0, sel.focus.offset] });
                    Ok(json!({ "node": r.node, "data": at.to_string() }))
                }
                // Someone else's caret and selection (their `say`), on
                // the pages of the front document: {caret, selection}.
                "live_heard" => {
                    let o = &mut self.open[self.front];
                    o.remote.as_ref().ok_or("this document isn't on a server")?;
                    let at: Value = serde_json::from_str(arg_str(args, "data")?).map_err(|e| e.to_string())?;
                    let pos = |v: &Value| -> Option<xenify_wp_doc::Pos> {
                        Some(xenify_wp_doc::Pos::new(NodeId(v[0].as_u64()?), v[1].as_u64()? as usize))
                    };
                    let (Some(anchor), Some(focus)) = (pos(&at["a"]), pos(&at["f"])) else { return Ok(Value::Null) };
                    let (caret, selection) = o.session.rects_of(xenify_wp_doc::Selection { anchor, focus });
                    Ok(json!({ "caret": caret, "selection": selection }))
                }
                "remote_info" => {
                    let o = &self.open[self.front];
                    Ok(match &o.remote {
                        Some(r) => json!({ "id": o.id, "server": r.server, "node": r.node, "name": r.name, "version": r.version, "live": r.live.is_some() }),
                        None => Value::Null,
                    })
                }
                "live_start" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    r.access = crate::remote::Access::of(args.get("role").and_then(Value::as_str).unwrap_or("editor"));
                    let ops = args.get("ops").and_then(Value::as_array).cloned().unwrap_or_default();
                    let ids = (
                        args.get("from").and_then(Value::as_u64).unwrap_or(0),
                        args.get("to").and_then(Value::as_u64).unwrap_or(u64::MAX),
                    );
                    r.start_live(&mut o.session.ed, args.get("seq").and_then(Value::as_i64).unwrap_or(0), &ops, ids, arg_str(args, "conn")?)?;
                    Ok(self.after())
                }
                // How many leased ids are left; a new range when they run low.
                "live_ids" => {
                    let i = self.by_id(args)?;
                    let o = &self.open[i];
                    let live = o.remote.as_ref().and_then(|r| r.live.as_ref()).ok_or("not editing together")?;
                    Ok(json!({ "left": live.ids.1.saturating_sub(o.session.ed.doc.next_id), "next": o.session.ed.doc.next_id }))
                }
                "live_lease" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    let live = o.remote.as_mut().and_then(|r| r.live.as_mut()).ok_or("not editing together")?;
                    let from = args.get("from").and_then(Value::as_u64).ok_or("missing “from”")?;
                    live.ids = (from, args.get("to").and_then(Value::as_u64).ok_or("missing “to”")?);
                    o.session.ed.doc.next_id = o.session.ed.doc.next_id.max(from);
                    Ok(Value::Null)
                }
                "live_ops" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    r.incoming(&mut o.session.ed, args.get("ops").and_then(Value::as_array).map_or(&[][..], |v| v))?;
                    Ok(self.after())
                }
                "live_accepted" | "live_behind" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    let id = o.id;
                    let r = o.remote.as_mut().ok_or("this document isn't on a server")?;
                    let client = arg_str(args, "client")?;
                    let next = if name == "live_accepted" {
                        r.accepted(
                            &mut o.session.ed,
                            client,
                            args.get("from").and_then(Value::as_i64).unwrap_or(0),
                            args.get("to").and_then(Value::as_i64).unwrap_or(0),
                        )?
                    } else {
                        r.behind(&mut o.session.ed, client, args.get("ops").and_then(Value::as_array).map_or(&[][..], |v| v))?
                    };
                    let mut out = self.after();
                    if let Some(b) = next {
                        let mut list = out.get("live").and_then(Value::as_array).cloned().unwrap_or_default();
                        list.push(json!({ "id": id, "batch": b }));
                        out["live"] = json!(list);
                    }
                    Ok(out)
                }
                // Changes made here the server hasn't got yet (kept if Write
                // closes before they're sent).
                "remote_unsent" => {
                    let i = if args.get("id").is_some() { self.by_id(args)? } else { self.front };
                    let o = &self.open[i];
                    Ok(json!({ "unsent": o.remote.as_ref().and_then(|r| r.live.as_ref()).is_some_and(|l| !l.settled()) }))
                }
                "live_resend" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    Ok(json!({ "batch": o.remote.as_mut().and_then(|r| r.resend()) }))
                }
                "live_stop" => {
                    let i = self.by_id(args)?;
                    let o = &mut self.open[i];
                    o.session.ed.set_live(false);
                    if let Some(r) = o.remote.as_mut() {
                        r.live = None;
                    }
                    Ok(self.after())
                }
                _ => Err(String::new()),
            }
        })())
        .filter(|r| !matches!(r, Err(e) if e.is_empty()))
    }

    // ----- Find and replace -----

    /// Finds again (the text may have changed) and says how many there are
    /// and which one is selected.
    fn refresh_find(&mut self) -> Value {
        let o = &mut self.open[self.front];
        let Some(f) = &mut o.find else { return Value::Null };
        f.hits = xenify_wp_doc::ops::find_all(&o.session.ed.doc, &f.query, f.case, f.whole);
        let (a, b) = o.session.ed.doc.ordered(o.session.ed.sel);
        f.current =
            f.hits.iter().position(|(p, r)| *p == a.para && r.start == a.offset && b.para == *p && r.end == b.offset);
        json!({ "count": f.hits.len(), "current": f.current })
    }

    fn select_hit(&mut self, back: bool) {
        let o = &mut self.open[self.front];
        let Some(f) = &o.find else { return };
        if f.hits.is_empty() {
            return;
        }
        let doc = &o.session.ed.doc;
        let caret = if back { doc.ordered(o.session.ed.sel).0 } else { doc.ordered(o.session.ed.sel).1 };
        let key = |p: NodeId, off: usize| (doc.para_index(p).unwrap_or(0), off);
        let at = key(caret.para, caret.offset);
        let i = if back {
            f.hits
                .iter()
                .rposition(|(p, r)| key(*p, r.end) < at || (key(*p, r.end) == at && f.current.is_none()))
                .unwrap_or(f.hits.len() - 1)
        } else {
            f.hits.iter().position(|(p, r)| key(*p, r.start) >= at).unwrap_or(0)
        };
        let (p, r) = f.hits[i].clone();
        o.session.ed.sel = xenify_wp_doc::Selection {
            anchor: xenify_wp_doc::Pos::new(p, r.start),
            focus: xenify_wp_doc::Pos::new(p, r.end),
        };
    }

    /// The maths, serif and emoji fonts arrived (the browser downloads them
    /// after the first page): every document is laid out again with them.
    pub fn fonts_arrived(&mut self) {
        if self.fonts.add_late_fonts() {
            for o in &mut self.open {
                o.session.fonts_changed();
            }
        }
    }

    // ----- Spelling and grammar -----

    /// What to check in the front document: its paragraphs (not code), its
    /// kind of writing and its ignore lists.
    pub fn proof_request(&self) -> (u32, crate::proof::Request) {
        let o = &self.open[self.front];
        let doc = &o.session.ed.doc;
        // Not the words of tables from a plan or a Calc book: they're the
        // plan's or book's, and can't be changed here anyway.
        let theirs = linked_paragraphs(doc);
        let paras = doc
            .paragraphs()
            .filter(|p| p.style != xenify_wp_doc::StyleSheet::CODE && !theirs.contains(&p.id))
            .map(|p| (p.id, p.text.clone()))
            .collect();
        let profile = doc.meta.proofing_profile.clone().unwrap_or_else(|| self.prefs.proofing.default_profile.clone());
        (
            o.id,
            crate::proof::Request {
                prefs: self.prefs.proofing.clone(),
                profile,
                paras,
                ignore_words: doc.meta.ignore_words.iter().chain(&self.server_words).cloned().collect(),
                ignore_issues: doc.meta.ignore_issues.clone(),
            },
        )
    }

    pub fn set_issues(&mut self, doc: u32, texts: &[(NodeId, String)], mut issues: Vec<Issue>) {
        let Some(o) = self.open.iter_mut().find(|o| o.id == doc) else { return };
        issues.sort_by_key(|i| (o.session.ed.doc.para_index(i.para).unwrap_or(usize::MAX), i.range.start));
        o.checked = texts.iter().map(|(id, t)| (*id, text_hash(t))).collect();
        o.issues = issues;
    }

    /// The issues still true of the text (not edited since they were found).
    fn live_issues(&self) -> Vec<(usize, &Issue)> {
        let o = &self.open[self.front];
        let doc = &o.session.ed.doc;
        o.issues
            .iter()
            .enumerate()
            .filter(|(_, i)| doc.para(i.para).is_some_and(|p| o.checked.get(&i.para) == Some(&text_hash(&p.text))))
            .collect()
    }

    /// Underlines for the live issues, in document space.
    fn marks(&mut self) -> Value {
        let live: Vec<(usize, Issue)> = self.live_issues().into_iter().map(|(n, i)| (n, i.clone())).collect();
        let o = &mut self.open[self.front];
        let layout = o.session.layout().clone();
        let find: Vec<Value> = o.find.as_ref().map_or_else(Vec::new, |f| {
            f.hits
                .iter()
                .enumerate()
                .map(|(k, (p, r))| {
                    let rects: Vec<[f64; 4]> = layout
                        .range_rects(*p, r.clone())
                        .into_iter()
                        .map(|r| [r.x0, r.y0, r.width(), r.height()])
                        .collect();
                    json!({ "kind": if f.current == Some(k) { "find on" } else { "find" }, "rects": rects })
                })
                .collect()
        });
        let mut all: Vec<Value> = find;
        all.extend(live.iter().map(|(n, i)| {
            let rects: Vec<[f64; 4]> = layout
                .range_rects(i.para, i.range.clone())
                .into_iter()
                .map(|r| [r.x0, r.y0, r.width(), r.height()])
                .collect();
            json!({ "i": n, "kind": kind_of(i.category), "rects": rects })
        }));
        json!(all)
    }

    /// The live issues for the Writing panel, each with a little of the text
    /// around it.
    fn issue_list(&self) -> Value {
        let doc = &self.open[self.front].session.ed.doc;
        json!(
            self.live_issues()
                .into_iter()
                .map(|(n, i)| {
                    let text = doc.para(i.para).map(|p| p.text.as_str()).unwrap_or("");
                    let (a, b) = (i.range.start.min(text.len()), i.range.end.min(text.len()));
                    let before =
                        text[..a].chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<String>();
                    let after: String = text[b..].chars().take(40).collect();
                    json!({
                        "i": n, "kind": kind_of(i.category), "label": label_of(i), "message": i.message,
                        "suggestions": i.suggestions.iter().take(4).collect::<Vec<_>>(),
                        "text": &text[a..b], "before": before, "after": after,
                        "spelling": i.category == Category::Spelling,
                    })
                })
                .collect::<Vec<_>>()
        )
    }

    fn issue(&self, args: &Value) -> Result<Issue, String> {
        let n = arg_f64(args, "i")? as usize;
        self.live_issues()
            .into_iter()
            .find(|(k, _)| *k == n)
            .map(|(_, i)| i.clone())
            .ok_or_else(|| "that was already changed".into())
    }

    /// Mail merge: the front document filled in once per row of the data, as
    /// `format` files (one file with every copy when `combined`).
    #[cfg(not(target_arch = "wasm32"))]
    fn merge_files(&self, args: &Value, format: Format, combined: bool) -> Result<Vec<(String, Vec<u8>)>, String> {
        let job = xenify_wp_io::merge_data::MergeJob {
            format,
            combined,
            name_template: args.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            range: args.get("range").and_then(Value::as_str).unwrap_or("").to_string(),
        };
        let (_, records) = self.merge.as_ref().ok_or("choose the data first")?;
        let doc = self.open[self.front].session.ed.doc.clone();
        let mut engine = xenify_wp_layout::Engine::new(self.fonts.clone());
        let assets = doc.assets.clone();
        xenify_wp_io::merge_data::run(
            &doc,
            records,
            &job,
            &mut engine,
            &move |src| assets.get(src).cloned(),
            &mut |_, _| {},
        )
        .map_err(|e| e.to_string())
    }

    /// What the screens show about picture `id` at `rect` (position, caption, box…).
    fn image_json(&self, id: NodeId, rect: kurbo::Rect) -> Value {
        let doc = &self.open[self.front].session.ed.doc;
        let (align, alt, src) = match doc.block(id) {
            Some(xenify_wp_doc::Block::Image(img)) => {
                (format!("{:?}", img.align).to_lowercase(), img.alt.clone(), img.src.clone())
            }
            _ => ("left".into(), String::new(), String::new()),
        };
        // Linked: shown from a file on disk, not kept inside the document.
        let linked = !doc.assets.contains_key(&src);
        // A Draw canvas shown from its file: where it is, and whether it moves
        // (Write then offers to play it).
        let canvas = linked.then(|| PathBuf::from(&src)).filter(|p| is_canvas(p)).and_then(|p| {
            if p.is_absolute() { Some(p) } else { Some(self.open[self.front].path.as_ref()?.parent()?.join(p)) }
        });
        let animated = canvas.as_ref().is_some_and(|p| canvas_animated(p));
        let border = matches!(doc.block(id), Some(xenify_wp_doc::Block::Image(img)) if img.border.is_some());
        // In a box (text around it, in front or behind) or in line with the text.
        let (container, index) = doc.locate(id).unwrap_or((xenify_wp_doc::Container::Section(0), 0));
        let frame = match &container {
            xenify_wp_doc::Container::Frame(f) => doc.frame(*f).cloned(),
            _ => None,
        };
        let blocks = doc.blocks(&container).cloned().unwrap_or_default();
        let is_caption = |k: Option<usize>| {
            k.is_some_and(|k| matches!(blocks.get(k), Some(xenify_wp_doc::Block::Paragraph(p)) if p.style == xenify_wp_doc::StyleSheet::CAPTION))
        };
        let caption = if is_caption(index.checked_sub(1)) {
            Some("top")
        } else if is_caption(Some(index + 1)) {
            Some("bottom")
        } else {
            None
        };
        json!({ "id": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()], "align": align, "alt": alt,
            "linked": linked, "src": src, "border": border, "caption": caption,
            "canvas": canvas.map(|p| p.display().to_string()), "animated": animated,
            "frame": frame.as_ref().map(|f| f.id.0), "wrap": frame.as_ref().map_or("Inline".to_string(), |f| format!("{:?}", f.wrap)),
            "gap": frame.as_ref().map(|f| f.gap) })
    }

    /// What the toolbar's details chip shows.
    fn details_summary(&self) -> Value {
        let m = &self.open[self.front].session.ed.doc.meta;
        let id = m.proofing_profile.clone().unwrap_or_else(|| "standard".into());
        let name = all_kinds().into_iter().find(|p| p.id == id).map(|p| p.name).unwrap_or(id);
        let doc = &self.open[self.front].session.ed.doc;
        let numbered = doc.styles.get(&xenify_wp_doc::StyleSheet::heading(1)).is_some_and(|s| s.list.is_some());
        json!({ "profile_name": name, "reader": m.audience, "tone": m.tone,
            "template": m.template.as_ref().map(|t| t.name.clone()), "is_template": m.is_template, "template_from": m.template.as_ref().map(|t| t.from.clone()), "numbered_headings": numbered })
    }

    pub fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        if name.starts_with("remote_")
            || name.starts_with("live_")
            // Linked documents (link_insert, links, …), not "link" itself
            // (a web link on the selection, an edit like any other).
            || (name.starts_with("link") && name != "link")
            || name.starts_with("calc_")
            || name.starts_with("plan_task_")
        {
            return self.remote_call(name, args).unwrap_or_else(|| Err(format!("unknown call {name}")));
        }
        // A document made from a template: its parts stay the template's.
        self.guard = None;
        if !name.starts_with("template_")
            && !matches!(
                name,
                "undo" | "redo" | "view" | "scenes" | "new" | "open" | "open_bytes" | "close" | "recover"
            )
            && let Some(o) = self.open.get(self.front)
            && let Some(print) = xenify_wp_doc::linked::fixed_print(&o.session.ed.doc)
        {
            self.guard = Some((o.id, print, o.session.ed.checkpoint()));
        }
        // Tables from a plan or a Calc book are theirs (not for reading calls,
        // or Undo and Redo, which may go back past an update).
        self.link_guard = None;
        if !matches!(name, "undo" | "redo" | "view" | "scenes" | "new" | "open" | "open_bytes" | "close" | "recover" | "table_info" | "table_pick")
            && let Some(o) = self.open.get(self.front)
        {
            let prints = linked_prints(&o.session.ed.doc);
            if !prints.is_empty() {
                self.link_guard = Some((o.id, prints, o.session.ed.checkpoint()));
            }
        }
        // Split view: the other pane reads (only reads) a tab that isn't in front.
        if matches!(name, "view" | "scenes")
            && let Some(id) = args.get("doc").and_then(Value::as_u64)
            && let Some(i) = self.open.iter().position(|o| o.id as u64 == id)
            && i != self.front
        {
            let front = self.front;
            self.front = i;
            let out = self.call_with_scenes(name, args);
            self.front = front;
            return out;
        }
        self.call_with_scenes(name, args)
    }

    fn call_with_scenes(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let mut out = self.call_inner(name, args)?;
        // The screen says which pages it shows and what it drew on them;
        // pages that changed come back with the answer (no second round trip).
        if let (Some(have), Some(view)) = (args.get("have").and_then(Value::as_object), out.get("view")) {
            let changed: Vec<usize> = view["pages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    let i = p["index"].as_u64()? as usize;
                    let drawn = have.get(&i.to_string())?;
                    (drawn.as_str() != p["sig"].as_str()).then_some(i)
                })
                .collect();
            if !changed.is_empty() {
                let o = &mut self.open[self.front];
                out["scenes"] = json!(o.session.scenes(&changed, &mut self.glyphs));
            }
        }
        Ok(out)
    }

    fn call_inner(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "view" => Ok(self.after()),
            "issues" => Ok(self.issue_list()),
            "snapshot" => {
                // Unsaved documents as Xenify documents (the browser keeps them
                // in case the tab closes); nothing is marked saved. Empty ones
                // aren't worth offering back, and server documents keep their
                // unsent changes apart (the screens' keepUnsent).
                use base64::Engine;
                let out: Vec<Value> = self
                    .open
                    .iter()
                    .filter(|o| o.session.ed.is_dirty() && o.remote.is_none() && worth_keeping(&o.session.ed.doc))
                    .filter_map(|o| {
                        let bytes = xenify_wp_io::save(&o.session.ed.doc, Format::Igdoc).ok()?;
                        Some(json!({
                            "title": o.title(), "words": o.session.ed.doc.word_count(), "kept": true,
                            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                        }))
                    })
                    .collect();
                Ok(json!(out))
            }
            "frame_at" => {
                // The text box whose edge is at (x, y), for moving it.
                let (x, y) = (arg_f64(args, "x")?, arg_f64(args, "y")?);
                let slop = args.get("slop").and_then(Value::as_f64).unwrap_or(5.0);
                let o = &mut self.open[self.front];
                let hit = o.session.layout().frame_edge_at(kurbo::Point::new(x, y), slop).cloned();
                Ok(match hit {
                    Some(f) => {
                        let fr = xenify_wp_doc::frames::find(&o.session.ed, f.id)
                            .and_then(|(s, i)| o.session.ed.doc.sections.get(s)?.frames.get(i))
                            .cloned()
                            .unwrap_or_default();
                        json!({
                            "id": f.id.0, "rect": [f.rect.x0, f.rect.y0, f.rect.width(), f.rect.height()],
                            "wrap": format!("{:?}", fr.wrap), "border": fr.border.is_some(),
                            "fill": fr.fill.map(|c| c.to_hex()), "padding": fr.padding, "gap": fr.gap,
                        })
                    }
                    None => Value::Null,
                })
            }
            "frame_set" => {
                // Moves (dx, dy), resizes (w, h), or changes how text flows around a box.
                let id = NodeId(arg_f64(args, "id")? as u64);
                let dx = args.get("dx").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let dy = args.get("dy").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let w = args.get("w").and_then(Value::as_f64).map(|v| v.max(24.0) as f32);
                let h = args.get("h").and_then(Value::as_f64).map(|v| v.max(16.0) as f32);
                let wrap = args.get("wrap").and_then(Value::as_str).map(|w| match w {
                    "top_bottom" => xenify_wp_doc::Wrap::TopBottom,
                    "in_front" => xenify_wp_doc::Wrap::InFront,
                    "behind" => xenify_wp_doc::Wrap::Behind,
                    _ => xenify_wp_doc::Wrap::Around,
                });
                let border = args.get("border").and_then(Value::as_bool);
                // Background colour: a hex, or null for none.
                let fill = args.get("fill").map(|v| v.as_str().and_then(Rgba::from_hex));
                let padding = args.get("padding").and_then(Value::as_f64).map(|v| v.clamp(0.0, 72.0) as f32);
                let gap = args.get("gap").and_then(Value::as_f64).map(|v| v.clamp(0.0, 72.0) as f32);
                let ed = &mut self.doc().session.ed;
                xenify_wp_doc::frames::update_frame(ed, id, "Text box", |f| {
                    f.x += dx;
                    f.y += dy;
                    if let Some(w) = w {
                        f.width = w;
                    }
                    if let Some(h) = h {
                        f.height = h;
                    }
                    if let Some(wr) = wrap {
                        f.wrap = wr;
                    }
                    if let Some(b) = border {
                        f.border =
                            b.then_some(xenify_wp_doc::Border { width: 0.75, color: Rgba([0x57, 0x53, 0x4E, 255]) });
                    }
                    if let Some(c) = fill {
                        f.fill = c;
                    }
                    if let Some(p) = padding {
                        f.padding = p;
                    }
                    if let Some(g) = gap {
                        f.gap = g;
                    }
                })
                .map_err(|e| e.to_string())?;
                ed.seal();
                Ok(self.after())
            }
            "frame_delete" => {
                let id = NodeId(arg_f64(args, "id")? as u64);
                xenify_wp_doc::frames::delete_frame(&mut self.doc().session.ed, id).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "frame_insert" => {
                // A text box on the caret's page, near the caret.
                let o = &mut self.open[self.front];
                let caret = o.session.ed.sel.focus;
                let section = o.session.ed.doc.section_of(caret.para).unwrap_or(0);
                let layout = o.session.layout().clone();
                let at = layout.caret(caret, false);
                let page_i = at.and_then(|r| layout.pages.iter().position(|p| r.y0 < p.rect.y1)).unwrap_or(0);
                let first = layout.pages.iter().position(|p| p.section == section).unwrap_or(0);
                let page = layout.pages.get(page_i);
                let (x, y, width) = match (page, at) {
                    (Some(p), Some(r)) => {
                        ((p.body.x0 - p.rect.x0) as f32, (r.y1 - p.rect.y0) as f32 + 8.0, p.body.width() as f32)
                    }
                    _ => (72.0, 72.0, 451.0),
                };
                // Across the column under the caret's line, text stopping above it and
                // carrying on below (flowing beside it is a choice on its right-click menu).
                let frame = xenify_wp_doc::Frame {
                    page: page_i.saturating_sub(first) as u32,
                    x,
                    y,
                    width,
                    height: 72.0,
                    wrap: xenify_wp_doc::Wrap::TopBottom,
                    border: Some(xenify_wp_doc::Border { width: 0.75, color: Rgba([0x57, 0x53, 0x4E, 255]) }),
                    ..Default::default()
                };
                xenify_wp_doc::frames::insert_frame(&mut o.session.ed, section, frame).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "table_info" => {
                use xenify_wp_doc::tables as t;
                let ed = &self.open[self.front].session.ed;
                let Some((id, row, col)) = t::cell_at(ed, ed.sel.focus.para) else { return Ok(Value::Null) };
                let tb = ed.doc.table(id).ok_or("that table is gone")?;
                let cell = tb.rows.get(row).and_then(|r| r.cells.get(col));
                Ok(json!({
                    "id": id.0, "row": row, "col": col, "can_sort": t::can_sort(&ed.doc, id),
                    "styles": t::TableStyle::ALL.iter().map(|s| s.name()).collect::<Vec<_>>(),
                    "header_row": tb.rows.first().is_some_and(|r| r.header), "header_col": tb.header_col,
                    "footer_row": tb.rows.last().is_some_and(|r| r.footer),
                    "repeat_header": tb.repeat_header, "repeat_footer": tb.repeat_footer,
                    "band": tb.band.is_some(), "lines": tb.border.is_some(),
                    "header_color": tb.header_fill.map(|c| c.to_hex()), "band_color": tb.band.map(|c| c.to_hex()),
                    "line_color": table_line_color(tb).map(|c| c.to_hex()), "themed": ed.doc.meta.table_colors.is_some(),
                    "width": tb.width, "align": format!("{:?}", tb.align).to_lowercase(), "padding": tb.padding,
                    "v_align": cell.map_or("top".to_string(), |c| format!("{:?}", c.v_align).to_lowercase()),
                    "linked": tb.linked.is_some(),
                    // Where it comes from: "server|node|item" (or "file|path|item").
                    "link": tb.linked.clone(),
                }))
            }
            "change_suggest" => {
                // A summary of what changed since the file was last saved.
                let o = &self.open[self.front];
                let saved = o
                    .path
                    .as_ref()
                    .and_then(|p| std::fs::read(p).ok().map(|b| (p.clone(), b)))
                    .and_then(|(p, b)| xenify_wp_io::load(&b, Format::from_name(&p.to_string_lossy())?).ok())
                    .map(|l| l.doc)
                    .unwrap_or_default();
                Ok(json!({ "summary": xenify_wp_doc::changelog::summarize(&saved, &o.session.ed.doc) }))
            }
            "record_change" => {
                let summary = arg_str(args, "summary")?.trim().to_string();
                if summary.is_empty() {
                    return Err("say what changed first".into());
                }
                xenify_wp_doc::changelog::record(&mut self.doc().session.ed, &summary).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "kinds" => {
                let features: Vec<Value> = xenify_grammar::Feature::ALL
                    .iter()
                    .map(|f| json!({ "id": f, "name": f.name(), "about": f.describe() }))
                    .collect();
                let kinds: Vec<Value> = all_kinds()
                    .iter()
                    .map(|p| {
                        let mut v = json!(p);
                        v["built_in"] = json!(p.built_in);
                        v
                    })
                    .collect();
                Ok(json!({ "kinds": kinds, "features": features }))
            }
            "save_kind" => {
                let mut p: xenify_grammar::Profile =
                    serde_json::from_value(args.get("kind").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
                if p.name.trim().is_empty() {
                    return Err("give it a name".into());
                }
                // Skipped by serde, so it came from Profile's default (a built-in one).
                p.built_in = false;
                if p.id.trim().is_empty() {
                    p.id = xenify_grammar::slug(&p.name);
                }
                let mut all = xenify_grammar::Profiles::load(Some(crate::proof::profiles_dir()));
                all.save(&p)?;
                crate::proof::PROFILES_CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(json!({ "id": p.id }))
            }
            "delete_kind" => {
                let mut all = xenify_grammar::Profiles::load(Some(crate::proof::profiles_dir()));
                all.delete(arg_str(args, "id")?)?;
                crate::proof::PROFILES_CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Value::Null)
            }
            "merge_load" => {
                // Data for a mail merge: a file's path (desktop) or its bytes (browser).
                use base64::Engine;
                let (name, bytes) = if let Some(p) = args.get("path").and_then(Value::as_str) {
                    let p = PathBuf::from(p);
                    (
                        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        std::fs::read(&p).map_err(|e| format!("couldn't read it: {e}"))?,
                    )
                } else {
                    (
                        arg_str(args, "name")?.to_string(),
                        base64::engine::general_purpose::STANDARD
                            .decode(arg_str(args, "data")?)
                            .map_err(|e| e.to_string())?,
                    )
                };
                let records = xenify_wp_io::merge_data::records(&bytes, &name)?;
                self.merge = Some((name, records));
                Ok(self.merge_info())
            }
            "merge_info" => Ok(self.merge_info()),
            "merge_preview" => {
                // Record `n` (1-based) merged, in a tab of its own.
                let n = args.get("n").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                let (_, records) = self.merge.as_ref().ok_or("choose the data first")?;
                let record = records.get(n - 1).cloned().ok_or("there's no record with that number")?;
                let mut doc = xenify_wp_doc::merge::instantiate(&self.open[self.front].session.ed.doc, &record, n);
                doc.meta.title = format!("Preview of record {n}");
                doc.meta.merge = None;
                self.add(doc, None);
                Ok(self.after())
            }
            #[cfg(not(target_arch = "wasm32"))]
            "merge_run" => {
                let dir = PathBuf::from(arg_str(args, "dir")?);
                let ext = args.get("ext").and_then(Value::as_str).unwrap_or("pdf");
                let format = Format::from_name(&format!("x.{ext}")).ok_or("unknown file type")?;
                let files =
                    self.merge_files(args, format, args.get("combined").and_then(Value::as_bool).unwrap_or(false))?;
                for (name, bytes) in &files {
                    write_file(&dir.join(name), bytes)?;
                }
                Ok(json!({ "written": files.len(), "dir": dir.display().to_string() }))
            }
            #[cfg(not(target_arch = "wasm32"))]
            "merge_print" => {
                // Every copy in one PDF, to the printer (or the PDF viewer when none is chosen).
                let files = self.merge_files(args, Format::Pdf, true)?;
                let (_, pdf) = files.first().ok_or("there's nothing to print")?;
                match args.get("printer").and_then(Value::as_str) {
                    Some(printer) => {
                        let copies = args.get("copies").and_then(Value::as_u64).unwrap_or(1).clamp(1, 999) as u32;
                        crate::print::print_pdf(pdf, printer, copies, None)?;
                    }
                    None => crate::print::open_in_viewer(pdf)?,
                }
                Ok(Value::Null)
            }
            "open_bytes" => {
                // A file picked in a browser: its name and contents (base64).
                use base64::Engine;
                let name = arg_str(args, "name")?.to_string();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(arg_str(args, "data")?)
                    .map_err(|e| e.to_string())?;
                let format = Format::from_name(&name).ok_or("Xenify Write doesn't know this kind of file")?;
                let mut doc = xenify_wp_io::load(&bytes, format).map_err(|e| e.to_string())?.doc;
                size_pictures(&mut doc);
                // `template`: a new document from it (a server template), untitled;
                // so is any template opened, unless `edit` (to change the template).
                let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem).to_string();
                if args.get("template").and_then(Value::as_bool).unwrap_or(false)
                    || doc.meta.is_template && !args.get("edit").and_then(Value::as_bool).unwrap_or(false)
                {
                    let from = args.get("from").and_then(Value::as_str).unwrap_or("").to_string();
                    let shown = args.get("template_name").and_then(Value::as_str).map_or(stem.clone(), str::to_string);
                    if doc.meta.is_template {
                        self.made_from = Some(json!({ "name": shown, "from": from }));
                    }
                    doc = from_template(doc, &shown, &from);
                } else if doc.meta.title.trim().is_empty() {
                    doc.meta.title = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem).to_string();
                }
                if self.open.len() == 1
                    && self.open[0].path.is_none()
                    && !self.open[0].session.ed.is_dirty()
                    && self.open[0].session.ed.doc.word_count() == 0
                {
                    self.open.clear();
                }
                // `recovered`: work kept in the browser, still unsaved (like the
                // desktop's recover: one undoable step into a blank document).
                if args.get("recovered").and_then(Value::as_bool).unwrap_or(false) {
                    self.add(Document::default(), None);
                    let o = self.open.last_mut().expect("just added");
                    let _ = xenify_wp_doc::restore(&mut o.session.ed, doc);
                } else {
                    self.add(doc, None);
                }
                Ok(self.after())
            }
            "save_bytes" => {
                // The front document as a file to download: {name, data (base64)}.
                use base64::Engine;
                let ext = args.get("ext").and_then(Value::as_str).unwrap_or("md");
                let format = Format::from_name(&format!("x.{ext}")).ok_or("unknown file type")?;
                let o = &mut self.open[self.front];
                let bytes = xenify_wp_io::save(&o.session.ed.doc, format).map_err(|e| e.to_string())?;
                if format.can_open() {
                    o.session.ed.mark_saved();
                }
                let name = format!("{}.{ext}", safe_name(&o.title()));
                let mut out = self.after();
                out["file"] = json!({ "name": name, "data": base64::engine::general_purpose::STANDARD.encode(bytes) });
                Ok(out)
            }
            "insert_picture_bytes" => {
                use base64::Engine;
                let name = arg_str(args, "name")?.replace(' ', "-");
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(arg_str(args, "data")?)
                    .map_err(|e| e.to_string())?;
                self.put_picture(&name, bytes)?;
                Ok(self.after())
            }
            "sources" => {
                use xenify_wp_doc::cite;
                let doc = &self.open[self.front].session.ed.doc;
                let cited = cite::cited_order(doc);
                Ok(json!({
                    "style": doc.meta.cite_style,
                    "styles": cite::CiteStyle::ALL.iter().map(|s| json!({ "id": s, "name": s.name() })).collect::<Vec<_>>(),
                    "kinds": cite::SourceKind::ALL.iter().map(|k| json!({ "id": k, "name": k.name() })).collect::<Vec<_>>(),
                    "sources": doc.meta.sources.iter().enumerate().map(|(n, src)| {
                        let entry: String = cite::entry(src, doc.meta.cite_style, n + 1).into_iter().map(|(t, _)| t).collect();
                        json!({ "source": src, "entry": entry, "cited": cited.contains(&src.key) })
                    }).collect::<Vec<_>>(),
                }))
            }
            "save_source" => {
                // Adds a source, or replaces the one with the same key.
                use xenify_wp_doc::cite;
                let mut src: cite::Source = serde_json::from_value(args.get("source").cloned().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                let ed = &mut self.doc().session.ed;
                let mut list = ed.doc.meta.sources.clone();
                if src.key.trim().is_empty() {
                    src.key = cite::new_key(&ed.doc, &src);
                }
                match list.iter_mut().find(|x| x.key == src.key) {
                    Some(x) => *x = src.clone(),
                    None => list.push(src.clone()),
                }
                cite::set_sources(ed, list).map_err(|e| e.to_string())?;
                let mut out = self.after();
                out["key"] = json!(src.key);
                Ok(out)
            }
            "delete_source" => {
                let key = arg_str(args, "key")?.to_string();
                let ed = &mut self.doc().session.ed;
                let list: Vec<_> = ed.doc.meta.sources.iter().filter(|x| x.key != key).cloned().collect();
                xenify_wp_doc::cite::set_sources(ed, list).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "import_sources" => {
                // Pasted BibTeX or RIS (what reference managers export).
                use xenify_wp_doc::cite;
                let text = arg_str(args, "text")?;
                let found =
                    if text.trim_start().starts_with('@') { cite::read_bibtex(text) } else { cite::read_ris(text) };
                if found.is_empty() {
                    return Err("that doesn't look like BibTeX or RIS".into());
                }
                let ed = &mut self.doc().session.ed;
                let mut list = ed.doc.meta.sources.clone();
                let n = found.len();
                for mut src in found {
                    if src.key.is_empty() || list.iter().any(|x| x.key == src.key) {
                        let tmp = xenify_wp_doc::Document {
                            meta: xenify_wp_doc::Meta { sources: list.clone(), ..Default::default() },
                            ..Default::default()
                        };
                        src.key = cite::new_key(&tmp, &src);
                    }
                    list.push(src);
                }
                cite::set_sources(ed, list).map_err(|e| e.to_string())?;
                let mut out = self.after();
                out["added"] = json!(n);
                Ok(out)
            }
            "cite_style" => {
                let style: xenify_wp_doc::cite::CiteStyle =
                    serde_json::from_value(args.get("style").cloned().unwrap_or_default())
                        .map_err(|e| e.to_string())?;
                xenify_wp_doc::cite::set_style(&mut self.doc().session.ed, style).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "cite" => {
                let keys: Vec<String> =
                    serde_json::from_value(args.get("keys").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
                if keys.is_empty() {
                    return Err("choose a source to cite".into());
                }
                let locator = args.get("locator").and_then(Value::as_str).unwrap_or("").trim().to_string();
                let ed = &mut self.doc().session.ed;
                ed.seal();
                xenify_wp_doc::cite::insert_citation(ed, keys, locator).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "bibliography" => {
                let ed = &mut self.doc().session.ed;
                ed.seal();
                xenify_wp_doc::tables::insert_generated(ed, xenify_wp_doc::TocKind::Bibliography)
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "image_at" => {
                let (x, y) = (arg_f64(args, "x")?, arg_f64(args, "y")?);
                let hit = self.open[self.front].session.layout().image_at(kurbo::Point::new(x, y)).cloned();
                Ok(hit.map_or(Value::Null, |i| self.image_json(i.id, i.rect)))
            }
            "image_info" => {
                // The same, found by id (after it moved into or out of a box).
                let id = NodeId(arg_f64(args, "id")? as u64);
                let hit = self.open[self.front].session.layout().images.iter().find(|i| i.id == id).cloned();
                Ok(hit.map_or(Value::Null, |i| self.image_json(i.id, i.rect)))
            }
            "tables_on_screen" => {
                // Every table: where it is, its row and column edges (document space), and
                // which one the caret is in, for the controls drawn on it.
                let o = &mut self.open[self.front];
                let caret_table =
                    xenify_wp_doc::tables::cell_at(&o.session.ed, o.session.ed.sel.focus.para).map(|(t, _, _)| t.0);
                let layout = o.session.layout().clone();
                let mut by: std::collections::BTreeMap<u64, Vec<&xenify_wp_layout::PlacedCell>> = Default::default();
                for c in &layout.cells {
                    by.entry(c.table.0).or_default().push(c);
                }
                let doc = &o.session.ed.doc;
                let out: Vec<Value> = by
                    .into_iter()
                    .filter_map(|(id, cells)| {
                        let t = doc.table(NodeId(id))?;
                        let rect = cells.iter().map(|c| c.rect).reduce(|a, b| a.union(b))?;
                        // Edges from single cells (merged ones span several).
                        let mut cols: Vec<(usize, f64, f64)> =
                            cells.iter().filter(|c| c.col_span == 1).map(|c| (c.col, c.rect.x0, c.rect.x1)).collect();
                        cols.sort_by_key(|c| c.0);
                        cols.dedup_by_key(|c| c.0);
                        let mut rows: Vec<(usize, f64, f64)> =
                            cells.iter().filter(|c| c.row_span == 1).map(|c| (c.row, c.rect.y0, c.rect.y1)).collect();
                        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
                        rows.dedup_by_key(|r| r.0);
                        Some(json!({
                            "id": id, "rect": [rect.x0, rect.y0, rect.width(), rect.height()],
                            "cols": cols.iter().map(|(i, a, b)| json!([i, a, b])).collect::<Vec<_>>(),
                            "rows": rows.iter().map(|(i, a, b)| json!([i, a, b])).collect::<Vec<_>>(),
                            "widths": t.widths, "here": caret_table == Some(id),
                        }))
                    })
                    .collect();
                Ok(json!(out))
            }
            "table_widths" => {
                let id = NodeId(arg_f64(args, "id")? as u64);
                let widths: Vec<f32> = serde_json::from_value(args.get("widths").cloned().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                xenify_wp_doc::tables::set_widths(&mut self.doc().session.ed, id, widths).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "table_add" => {
                // A row (`row`: before that index) or column (`col`) added at an edge.
                let id = NodeId(arg_f64(args, "id")? as u64);
                let ed = &mut self.doc().session.ed;
                if let Some(r) = args.get("row").and_then(Value::as_u64) {
                    xenify_wp_doc::tables::insert_row_at(ed, id, r as usize).map_err(|e| e.to_string())?;
                } else if let Some(c) = args.get("col").and_then(Value::as_u64) {
                    xenify_wp_doc::tables::insert_column_at(ed, id, c as usize).map_err(|e| e.to_string())?;
                }
                Ok(self.after())
            }
            "table_pick" => {
                // Selects a whole row or column (its handle was clicked).
                let id = NodeId(arg_f64(args, "id")? as u64);
                let ed = &mut self.doc().session.ed;
                if let Some(r) = args.get("row").and_then(Value::as_u64) {
                    xenify_wp_doc::tables::select_row(ed, id, r as usize);
                } else if let Some(c) = args.get("col").and_then(Value::as_u64) {
                    xenify_wp_doc::tables::select_column(ed, id, c as usize);
                }
                Ok(self.after())
            }
            "contents_boxes" => {
                // Where each generated list (contents, bibliography…) is, for its outline on screen.
                let doc = self.open[self.front].session.ed.doc.clone();
                let layout = self.open[self.front].session.layout();
                let mut boxes: Vec<(NodeId, kurbo::Rect)> = Vec::new();
                for f in &layout.fragments {
                    let Some(id) = f.para.generated else { continue };
                    let r = f.rect();
                    match boxes.last_mut() {
                        Some((last, b)) if *last == id && r.y0 - b.y1 < 30.0 && r.y0 >= b.y0 => *b = b.union(r),
                        _ => boxes.push((id, r)),
                    }
                }
                Ok(json!(
                    boxes
                        .iter()
                        .filter_map(|(id, r)| {
                            let Some(xenify_wp_doc::Block::Toc(t)) = doc.block(*id) else { return None };
                            let kind = match t.kind {
                                xenify_wp_doc::TocKind::Contents => "contents",
                                xenify_wp_doc::TocKind::Bibliography => "bibliography",
                                xenify_wp_doc::TocKind::ChangeLog => "change_log",
                                xenify_wp_doc::TocKind::Figures => "figures",
                            };
                            Some(json!({ "id": id.0, "kind": kind, "levels": t.levels, "page_numbers": t.page_numbers,
                        "empty": kind == "contents" && xenify_wp_layout::toc_entries(&doc, t.id, t.levels).is_empty(),
                        "rect": [r.x0, r.y0, r.width(), r.height()] }))
                        })
                        .collect::<Vec<_>>()
                ))
            }
            "contents_set" => {
                // A table of contents' settings: how many heading levels, page numbers.
                let id = NodeId(arg_f64(args, "id")? as u64);
                let ed = &mut self.doc().session.ed;
                let Some(xenify_wp_doc::Block::Toc(mut t)) = ed.doc.block(id).cloned() else {
                    return Err("that table of contents is gone".into());
                };
                if let Some(n) = args.get("levels").and_then(Value::as_u64) {
                    t.levels = n.clamp(1, 6) as u8;
                }
                if let Some(b) = args.get("page_numbers").and_then(Value::as_bool) {
                    t.page_numbers = b;
                }
                ed.seal();
                xenify_wp_doc::tables::replace_block(ed, id, xenify_wp_doc::Block::Toc(t), "Contents")
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "image_wrap" => {
                // In line with the text, or in a box with text beside it, above and below, in front or behind.
                use xenify_wp_doc::{Container, Wrap, frames};
                let id = NodeId(arg_f64(args, "id")? as u64);
                let wrap = match arg_str(args, "wrap")? {
                    "inline" => None,
                    "top_bottom" => Some(Wrap::TopBottom),
                    "in_front" => Some(Wrap::InFront),
                    "behind" => Some(Wrap::Behind),
                    _ => Some(Wrap::Around),
                };
                let o = &mut self.open[self.front];
                let layout = o.session.layout().clone();
                let ed = &mut o.session.ed;
                let in_frame = match ed.doc.locate(id) {
                    Some((Container::Frame(f), _)) => Some(f),
                    _ => None,
                };
                match (in_frame, wrap) {
                    (Some(f), Some(w)) => {
                        frames::update_frame(ed, f, "Picture position", |fr| fr.wrap = w).map_err(|e| e.to_string())?
                    }
                    (Some(f), None) => {
                        // Back in the text before the first body paragraph level with or below the box.
                        let top = layout.frames.iter().find(|p| p.id == f).map_or(0.0, |p| p.rect.y0);
                        let before = layout
                            .fragments
                            .iter()
                            .filter(|fr| fr.lines.start == 0 && fr.rect().y0 >= top - 1.0)
                            .map(|fr| fr.para.id)
                            .find(|p| matches!(ed.doc.locate(*p), Some((Container::Section(_), _))));
                        frames::inline_image(ed, f, before).map_err(|e| e.to_string())?;
                    }
                    (None, Some(w)) => {
                        // Floated where it is now.
                        let placed =
                            layout.images.iter().find(|i| i.id == id).ok_or("that picture isn't on the page")?;
                        let r = placed.rect;
                        let pi = layout.pages.iter().position(|p| p.rect.contains(r.center())).unwrap_or(0);
                        let page = &layout.pages[pi];
                        let first = layout.pages.iter().position(|p| p.section == page.section).unwrap_or(0);
                        let at = ((r.x0 - page.rect.x0) as f32, (r.y0 - page.rect.y0) as f32);
                        frames::float_image(ed, id, page.section, (pi - first) as u32, at, w)
                            .map_err(|e| e.to_string())?;
                    }
                    (None, None) => {}
                }
                Ok(self.after())
            }
            "image_caption" => {
                // A caption above or below the picture, or none.
                let id = NodeId(arg_f64(args, "id")? as u64);
                let place = match args.get("place").and_then(Value::as_str) {
                    Some("top") => Some(true),
                    Some(_) => Some(false),
                    None => None,
                };
                let ed = &mut self.doc().session.ed;
                xenify_wp_doc::frames::set_caption(ed, id, place).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "image_link" => {
                // A picture shown from a file (`path`: kept up to date with it), or copied
                // into the document (`embed`: it no longer changes with the file).
                let id = NodeId(arg_f64(args, "id")? as u64);
                let dir = self.open[self.front].path.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                let ed = &mut self.doc().session.ed;
                let Some(xenify_wp_doc::Block::Image(mut img)) = ed.doc.block(id).cloned() else {
                    return Err("that picture is gone".into());
                };
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    // Next to the document: written relative, so the two can move together.
                    let p = PathBuf::from(path);
                    img.src = match dir.as_ref().and_then(|d| p.strip_prefix(d).ok()) {
                        Some(rel) => rel.display().to_string(),
                        None => path.to_string(),
                    };
                } else if args.get("embed").and_then(Value::as_bool).unwrap_or(false) {
                    let (key, bytes) = picture_copy(&img.src, dir.as_deref(), &ed.doc.assets)?;
                    ed.doc.assets.insert(key.clone(), std::sync::Arc::new(bytes));
                    img.src = key;
                }
                ed.seal();
                xenify_wp_doc::tables::replace_block(ed, id, xenify_wp_doc::Block::Image(img), "Picture")
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "image_set" => {
                // A picture's size, alignment or description.
                let id = NodeId(arg_f64(args, "id")? as u64);
                let ed = &mut self.doc().session.ed;
                let Some(xenify_wp_doc::Block::Image(img)) = ed.doc.block(id).cloned() else {
                    return Err("that picture is gone".into());
                };
                let mut img = img;
                if let (Some(w), Some(h)) =
                    (args.get("w").and_then(Value::as_f64), args.get("h").and_then(Value::as_f64))
                {
                    img.width = w.max(8.0) as f32;
                    img.height = h.max(8.0) as f32;
                }
                if let Some(a) = args.get("align").and_then(Value::as_str) {
                    img.align = match a {
                        "center" => xenify_wp_doc::Align::Center,
                        "right" => xenify_wp_doc::Align::Right,
                        _ => xenify_wp_doc::Align::Left,
                    };
                }
                if let Some(t) = args.get("alt").and_then(Value::as_str) {
                    img.alt = t.to_string();
                }
                if let Some(b) = args.get("border").and_then(Value::as_bool) {
                    img.border =
                        b.then_some(xenify_wp_doc::Border { width: 0.75, color: Rgba([0x57, 0x53, 0x4E, 255]) });
                }
                let (w, h) = (img.width, img.height);
                ed.seal();
                xenify_wp_doc::tables::replace_block(ed, id, xenify_wp_doc::Block::Image(img), "Picture")
                    .map_err(|e| e.to_string())?;
                // A picture in a box: the box takes its new size (with room for a caption).
                if args.get("w").is_some()
                    && let Some((xenify_wp_doc::Container::Frame(f), _)) = ed.doc.locate(id)
                {
                    let extra = ed.doc.frame(f).map_or(0, |fr| fr.blocks.len().saturating_sub(1)) as f32 * 22.0;
                    xenify_wp_doc::frames::update_frame(ed, f, "Picture size", |fr| {
                        fr.width = w;
                        fr.height = h + extra;
                    })
                    .map_err(|e| e.to_string())?;
                }
                ed.seal();
                Ok(self.after())
            }
            "delete_block" => {
                let id = NodeId(arg_f64(args, "id")? as u64);
                xenify_wp_doc::tables::delete_block(&mut self.doc().session.ed, id).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "slash" => {
                // A "/" command: the typed "/word" (`typed` bytes before the caret) goes,
                // and `cmd` goes in its place.
                use xenify_wp_doc::{Field, ops};
                let cmd = arg_str(args, "cmd")?.to_string();
                let ed = &mut self.doc().session.ed;
                let at = ed.sel.focus;
                // The "/word" just before the caret (found here, so keys still on their way can't confuse it).
                let typed = ed.doc.para(at.para).map_or(0, |p| {
                    let before = &p.text[..at.offset.min(p.text.len())];
                    let word = before.len() - before.trim_end_matches(|c: char| c.is_alphanumeric()).len();
                    let slash = before.len() - word;
                    let ok = before[..slash].ends_with('/')
                        && before[..slash - 1].chars().next_back().is_none_or(char::is_whitespace);
                    if ok { word + 1 } else { 0 }
                });
                if typed > 0 && at.offset >= typed {
                    ed.sel = xenify_wp_doc::Selection {
                        anchor: xenify_wp_doc::Pos::new(at.para, at.offset - typed),
                        focus: at,
                    };
                    ops::delete_selection(ed).map_err(|e| e.to_string())?;
                }
                // Fixed: the date it was put in (it doesn't change by itself).
                let dt = |format: &str| Field::DateTime {
                    format: format.into(),
                    fixed: Some(xenify_wp_doc::datetime::format(format, &xenify_wp_doc::datetime::now())),
                };
                match cmd.as_str() {
                    "date" => ops::insert_field(ed, dt("d MMMM yyyy"), "Date"),
                    "time" => ops::insert_field(ed, dt("HH:mm"), "Time"),
                    "datetime" => ops::insert_field(ed, dt("d MMMM yyyy, HH:mm"), "Date and time"),
                    "badge" => {
                        ops::insert_field(ed, Field::Badge { text: "Status".into(), color: "blue".into() }, "Badge")
                    }
                    "info" | "note" | "success" | "warning" | "alert" => {
                        ops::set_panel(ed, Some(&format!("panel-{cmd}")))
                    }
                    "page" => ops::insert_field(ed, Field::Page, "Page number"),
                    "footnote" => xenify_wp_doc::notes::insert_note(ed, xenify_wp_doc::NoteKind::Foot).map(|_| ()),
                    "toc" => xenify_wp_doc::tables::insert_toc(ed, 3),
                    "table" => xenify_wp_doc::tables::insert_table(ed, 3, 3),
                    // Only take the typed word away (the screens do the rest).
                    "clear" => Ok(()),
                    _ => return Err(format!("no /{cmd}")),
                }
                .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "badge_at" => {
                // The badge under a point, for its right-click menu.
                let (x, y) = (arg_f64(args, "x")?, arg_f64(args, "y")?);
                let o = self.doc();
                let Some(pos) = o.session.layout().hit(kurbo::Point::new(x, y)) else { return Ok(Value::Null) };
                let doc = &o.session.ed.doc;
                let w = xenify_wp_doc::Field::CHAR.len_utf8();
                Ok([pos.offset, pos.offset.saturating_sub(w)]
                    .into_iter()
                    .find_map(|off| match xenify_wp_doc::ops::field_at(doc, xenify_wp_doc::Pos::new(pos.para, off)) {
                        Some(xenify_wp_doc::Field::Badge { text, color }) => {
                            Some(json!({ "para": pos.para.0, "offset": off, "text": text, "color": color }))
                        }
                        _ => None,
                    })
                    .unwrap_or(Value::Null))
            }
            "badge_set" => {
                let pos =
                    xenify_wp_doc::Pos::new(NodeId(arg_f64(args, "para")? as u64), arg_f64(args, "offset")? as usize);
                let field = xenify_wp_doc::Field::Badge {
                    text: arg_str(args, "text")?.trim().to_string(),
                    color: arg_str(args, "color")?.to_string(),
                };
                xenify_wp_doc::ops::set_field(&mut self.doc().session.ed, pos, field, "Badge")
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "ai_table" => {
                // An AI proposal for a table: new rows, changed cells (rows and columns from 1).
                let id = NodeId(arg_f64(args, "table")? as u64);
                let rows: Vec<Vec<String>> = serde_json::from_value(args.get("rows").cloned().unwrap_or(json!([])))
                    .map_err(|e| e.to_string())?;
                let cells: Vec<(usize, usize, String)> =
                    serde_json::from_value(args.get("cells").cloned().unwrap_or(json!([])))
                        .map_err(|e| e.to_string())?;
                let at = args.get("at").and_then(Value::as_u64).map(|v| (v as usize).saturating_sub(1));
                let ed = &mut self.doc().session.ed;
                if ed.doc.table(id).is_none() {
                    return Err("that table is gone".into());
                }
                ed.seal();
                if !rows.is_empty() {
                    xenify_wp_doc::tables::add_rows_with(ed, id, at, &rows).map_err(|e| e.to_string())?;
                }
                if !cells.is_empty() {
                    let cells: Vec<_> =
                        cells.into_iter().map(|(r, c, t)| (r.saturating_sub(1), c.saturating_sub(1), t)).collect();
                    xenify_wp_doc::tables::set_cell_texts(ed, id, &cells).map_err(|e| e.to_string())?;
                }
                ed.seal();
                Ok(self.after())
            }
            "divider" => {
                xenify_wp_doc::ops::insert_divider(&mut self.doc().session.ed).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "style_list" => {
                // A style that numbers its paragraphs: "numbers", "bullets", or none.
                let id = arg_str(args, "id")?.to_string();
                let ed = &mut self.doc().session.ed;
                let heading = ed.doc.styles.get(&id).and_then(|s| s.outline_level).is_some();
                let list = match args.get("list").and_then(Value::as_str) {
                    Some("bullets") => Some(xenify_wp_doc::ListDef::BULLETS),
                    Some(_) if heading => Some(xenify_wp_doc::ListDef::HEADINGS),
                    Some(_) => Some(xenify_wp_doc::ListDef::NUMBERS),
                    None => None,
                };
                xenify_wp_doc::ops::set_style_list(ed, &id, list).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "number_headings" => {
                let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
                xenify_wp_doc::ops::number_headings(&mut self.doc().session.ed, on).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "list_start" => {
                // Where the caret's numbered item starts counting (`null`: carry on from the one before).
                let at = args.get("at").and_then(Value::as_u64).map(|n| n.clamp(0, 9999) as u32);
                xenify_wp_doc::ops::restart_numbering(&mut self.doc().session.ed, at).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "list_info" => {
                // The caret's list item, for its right-click menu: numbered or not, its number, set start.
                let ed = &self.doc().session.ed;
                let Some(p) = ed.doc.para(ed.sel.focus.para) else { return Ok(Value::Null) };
                let Some(item) = &p.list else { return Ok(Value::Null) };
                let kind = ed.doc.list_def(&item.list).map(|d| d.level(item.level).kind);
                let numbered = kind.is_some_and(|k| {
                    !matches!(k, xenify_wp_doc::NumberKind::Bullet | xenify_wp_doc::NumberKind::Check)
                });
                let label = xenify_wp_doc::lists::labels(&ed.doc).get(&p.id).cloned().unwrap_or_default();
                // Numbered headings count through the document; they aren't started again by hand.
                let headings = item.list == xenify_wp_doc::ListDef::HEADINGS;
                Ok(json!({ "numbered": numbered, "label": label, "restart": item.restart, "follow": item.follow, "headings": headings }))
            }
            "panel" => {
                // The caret's paragraph as a panel (info, warning…), or back to body text.
                let id = args.get("kind").and_then(Value::as_str).map(|k| format!("panel-{k}"));
                xenify_wp_doc::ops::set_panel(&mut self.doc().session.ed, id.as_deref()).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "reading" => {
                // Reading view for every open document (and ones opened later).
                self.reading = args.get("on").and_then(Value::as_bool).unwrap_or(false);
                for o in &mut self.open {
                    o.session.set_reading(self.reading);
                }
                Ok(self.after())
            }
            "datetime_presets" => {
                // Each format offered, written out for now.
                let now = xenify_wp_layout::now();
                Ok(json!(
                    xenify_wp_doc::datetime::PRESETS
                        .iter()
                        .map(|f| json!({ "format": f, "example": xenify_wp_doc::datetime::format(f, &now) }))
                        .collect::<Vec<_>>()
                ))
            }
            "datetime_at" => {
                // The date and time field under a point (or the caret), for its right-click menu.
                let o = self.doc();
                let pos = match (args.get("x").and_then(Value::as_f64), args.get("y").and_then(Value::as_f64)) {
                    (Some(x), Some(y)) => o.session.layout().hit(kurbo::Point::new(x, y)),
                    _ => Some(o.session.ed.sel.focus),
                };
                let Some(pos) = pos else { return Ok(Value::Null) };
                let doc = &o.session.ed.doc;
                let w = xenify_wp_doc::Field::CHAR.len_utf8();
                let found = [pos.offset, pos.offset.saturating_sub(w)].into_iter().find_map(|off| {
                    match xenify_wp_doc::ops::field_at(doc, xenify_wp_doc::Pos::new(pos.para, off)) {
                        Some(xenify_wp_doc::Field::DateTime { format, fixed }) => Some((off, format, fixed)),
                        _ => None,
                    }
                });
                Ok(match found {
                    Some((off, format, fixed)) => {
                        json!({ "para": pos.para.0, "offset": off, "format": format, "auto": fixed.is_none() })
                    }
                    None => Value::Null,
                })
            }
            "datetime_set" => {
                // A date and time field's format, whether it updates by itself, or "update now".
                let pos =
                    xenify_wp_doc::Pos::new(NodeId(arg_f64(args, "para")? as u64), arg_f64(args, "offset")? as usize);
                let format = arg_str(args, "format")?.to_string();
                let auto = args.get("auto").and_then(Value::as_bool).unwrap_or(false);
                let fixed = (!auto).then(|| xenify_wp_doc::datetime::format(&format, &xenify_wp_layout::now()));
                let ed = &mut self.doc().session.ed;
                xenify_wp_doc::ops::set_field(
                    ed,
                    pos,
                    xenify_wp_doc::Field::DateTime { format, fixed },
                    "Date and time",
                )
                .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "math_to_latex" => Ok(json!(xenify_wp_engine::easymath::to_latex(arg_str(args, "text")?))),
            "math_to_easy" => Ok(json!(xenify_wp_engine::easymath::to_easy(arg_str(args, "latex")?))),
            "equation_get" => {
                // The equation at the caret, just before it, or under a point.
                let o = self.doc();
                let pos = match (args.get("x").and_then(Value::as_f64), args.get("y").and_then(Value::as_f64)) {
                    (Some(x), Some(y)) => o.session.layout().hit(kurbo::Point::new(x, y)),
                    _ => Some(o.session.ed.sel.focus),
                };
                let Some(pos) = pos else { return Ok(Value::Null) };
                let doc = &o.session.ed.doc;
                let w = xenify_wp_doc::Field::CHAR.len_utf8();
                let found = [pos.offset, pos.offset.saturating_sub(w)].into_iter().find_map(|off| {
                    xenify_wp_doc::ops::equation_at(doc, xenify_wp_doc::Pos::new(pos.para, off)).map(|e| (off, e))
                });
                Ok(match found {
                    Some((off, (latex, display))) => {
                        json!({ "para": pos.para.0, "offset": off, "latex": latex, "display": display })
                    }
                    None => Value::Null,
                })
            }
            "equation_set" => {
                let latex = arg_str(args, "latex")?.to_string();
                let display = args.get("display").and_then(Value::as_bool).unwrap_or(false);
                let ed = &mut self.doc().session.ed;
                let at = match (args.get("para").and_then(Value::as_u64), args.get("offset").and_then(Value::as_u64)) {
                    (Some(p), Some(o)) => xenify_wp_doc::Pos::new(NodeId(p), o as usize),
                    _ => {
                        // A new one goes in at the caret.
                        let at = ed.doc.ordered(ed.sel).0;
                        xenify_wp_doc::ops::insert_field(
                            ed,
                            xenify_wp_doc::Field::Equation { latex: latex.clone(), display, omml: String::new() },
                            "Equation",
                        )
                        .map_err(|e| e.to_string())?;
                        let mut out = self.after();
                        out["at"] = json!({ "para": at.para.0, "offset": at.offset });
                        return Ok(out);
                    }
                };
                xenify_wp_doc::ops::set_equation(ed, at, &latex, display).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "equation_remove" => {
                let at =
                    xenify_wp_doc::Pos::new(NodeId(arg_f64(args, "para")? as u64), arg_f64(args, "offset")? as usize);
                xenify_wp_doc::ops::remove_equation(&mut self.doc().session.ed, at).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "table" => {
                use xenify_wp_doc::tables as t;
                let ed = &mut self.doc().session.ed;
                ed.seal();
                match arg_str(args, "what")? {
                    "row_above" => t::insert_row(ed, false),
                    "row_below" => t::insert_row(ed, true),
                    "col_left" => t::insert_column(ed, false),
                    "col_right" => t::insert_column(ed, true),
                    "delete_rows" => t::delete_rows(ed),
                    "delete_cols" => t::delete_columns(ed),
                    "delete_table" => t::delete_table(ed),
                    "merge" => t::merge_cells(ed),
                    "split" => t::split_cell(ed),
                    "header_row" => t::toggle_header_rows(ed),
                    "header_col" => t::toggle_header_col(ed),
                    "headers" => {
                        let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                        let on = |k: &str| args.get(k).and_then(Value::as_bool).unwrap_or(false);
                        t::set_headers(ed, id, on("row"), on("col"))
                    }
                    "footer" => {
                        let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                        t::set_footer(ed, id, args.get("on").and_then(Value::as_bool).unwrap_or(true))
                    }
                    "repeat" => {
                        let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                        let get = |k: &str| args.get(k).and_then(Value::as_bool);
                        t::set_repeats(ed, id, get("header"), get("footer"))
                    }
                    "band" => {
                        let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
                        t::set_band(ed, on.then_some(Rgba([0xF4, 0xF7, 0xFB, 255])))
                    }
                    "layout" => {
                        // Width (share of the column), where it sits, and room inside the cells.
                        let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                        let tb = ed.doc.table(id).ok_or("that table is gone")?.clone();
                        let width = args.get("width").and_then(Value::as_f64).map_or(tb.width, |v| v as f32);
                        let align = match args.get("align").and_then(Value::as_str) {
                            Some("center") => xenify_wp_doc::Align::Center,
                            Some("right") => xenify_wp_doc::Align::Right,
                            Some(_) => xenify_wp_doc::Align::Left,
                            None => tb.align,
                        };
                        let padding = args.get("padding").and_then(Value::as_f64).map_or(tb.padding, |v| v as f32);
                        t::set_table_layout(ed, id, tb.widths.clone(), width, align, padding)
                    }
                    "style" => {
                        let n = args.get("n").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let style = *t::TableStyle::ALL.get(n).ok_or("unknown table style")?;
                        let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                        t::apply_style(ed, id, style)
                    }
                    "shade" => t::set_shading(ed, args.get("hex").and_then(Value::as_str).and_then(Rgba::from_hex)),
                    // This table's colours: {header|band: "#hex" or "none", lines: "#hex"}.
                    "colors" => {
                        let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                        let fill = |k: &str| args.get(k).and_then(Value::as_str).map(Rgba::from_hex);
                        let lines = args.get("lines").and_then(Value::as_str).and_then(Rgba::from_hex);
                        t::set_table_colors(ed, id, fill("header"), fill("band"), lines)
                    }
                    // This table's colours for every table in the document (and its looks), or not.
                    "theme" => {
                        let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
                        let theme = if on {
                            let (id, _, _) = t::cell_at(ed, ed.sel.focus.para).ok_or("put the cursor in a table")?;
                            let tb = ed.doc.table(id).ok_or("that table is gone")?;
                            Some(t::TableColors { header: tb.header_fill, band: tb.band, lines: table_line_color(tb) })
                        } else {
                            None
                        };
                        t::set_table_theme(ed, theme)
                    }
                    "lines" => t::set_border(
                        ed,
                        Some(xenify_wp_doc::Border { width: 0.75, color: Rgba([0x80, 0x7A, 0x73, 255]) }),
                    ),
                    "no_lines" => t::set_border(ed, None),
                    "sort" | "sort_desc" => match t::cell_at(ed, ed.sel.focus.para) {
                        Some((id, _, col)) => t::sort_rows(ed, id, col, arg_str(args, "what")? == "sort_desc"),
                        None => return Err("put the cursor in a table".into()),
                    },
                    "top" | "middle" | "bottom" => t::set_v_align(
                        ed,
                        match arg_str(args, "what")? {
                            "middle" => xenify_wp_doc::VAlign::Middle,
                            "bottom" => xenify_wp_doc::VAlign::Bottom,
                            _ => xenify_wp_doc::VAlign::Top,
                        },
                    ),
                    "distribute" => match t::cell_at(ed, ed.sel.focus.para) {
                        Some((id, _, _)) => t::distribute_columns(ed, id),
                        None => return Err("put the cursor in a table".into()),
                    },
                    _ => return Err("unknown table action".into()),
                }
                .map_err(|e| e.to_string())?;
                ed.seal();
                Ok(self.after())
            }
            "style_from_selection" => {
                // Update style `id` to look like the selection, or make a new one named `name`.
                // A document made from a template takes its styles from there.
                if let Some(t) = &self.doc().session.ed.doc.meta.template {
                    return Err(format!(
                        "This document's styles come from the template “{}”. To change them, use File › Edit a template.",
                        t.name
                    ));
                }
                let ed = &mut self.doc().session.ed;
                ed.seal();
                if let Some(name) = args.get("name").and_then(Value::as_str) {
                    let id =
                        xenify_wp_doc::ops::new_style_from_selection(ed, name.trim()).map_err(|e| e.to_string())?;
                    let _ = xenify_wp_doc::ops::set_style(ed, &id);
                } else {
                    xenify_wp_doc::ops::update_style_from_selection(ed, arg_str(args, "id")?)
                        .map_err(|e| e.to_string())?;
                }
                ed.seal();
                Ok(self.after())
            }
            "para" => {
                // The caret paragraph's spacing and indents as they look (style + direct).
                let ed = &self.doc().session.ed;
                let p = ed.doc.para(ed.sel.focus.para).ok_or("put the cursor in a paragraph")?;
                let mut r = ed.doc.styles.resolve(&p.style);
                r.apply(&p.props);
                Ok(json!({
                    "space_before": r.space_before, "space_after": r.space_after, "line_height": r.line_height, "line_exact": r.line_exact,
                    "indent_left": r.indent_left, "indent_right": r.indent_right, "first_line": r.first_line,
                    "keep_with_next": r.keep_with_next, "keep_together": r.keep_together, "page_break_before": r.page_break_before,
                }))
            }
            "set_para" => {
                let patch: xenify_wp_doc::ParaProps =
                    serde_json::from_value(args.get("patch").cloned().unwrap_or_default())
                        .map_err(|e| e.to_string())?;
                let ed = &mut self.doc().session.ed;
                ed.seal();
                xenify_wp_doc::ops::apply_para_props(ed, &patch, "Paragraph").map_err(|e| e.to_string())?;
                ed.seal();
                Ok(self.after())
            }
            "review" => {
                use xenify_wp_doc::review;
                let doc = &self.open[self.front].session.ed.doc;
                let anchors = review::anchors(doc);
                let quote = |a: xenify_wp_doc::Pos, b: xenify_wp_doc::Pos| -> String {
                    if a.para == b.para {
                        doc.para(a.para)
                            .and_then(|p| p.text.get(a.offset..b.offset))
                            .unwrap_or("")
                            .chars()
                            .take(120)
                            .collect()
                    } else {
                        doc.para(a.para).and_then(|p| p.text.get(a.offset..)).unwrap_or("").chars().take(120).collect()
                    }
                };
                let mut out = json!({
                    "tracking": doc.meta.track_changes,
                    "comments": doc.meta.comments.iter().map(|c| {
                        let at = anchors.iter().find(|(id, _, _)| *id == c.id);
                        json!({
                            "id": c.id.0, "author": c.author, "date": crate::recovery::parse_iso(&c.date).map(crate::recovery::when).unwrap_or_default(),
                            "text": c.text, "resolved": c.resolved,
                            "replies": c.replies.iter().map(|r| json!({"author": r.author, "text": r.text, "date": crate::recovery::parse_iso(&r.date).map(crate::recovery::when).unwrap_or_default()})).collect::<Vec<_>>(),
                            "quote": at.map(|(_, a, b)| quote(*a, *b)).unwrap_or_default(),
                            "para": at.map(|(_, a, _)| a.para.0), "start": at.map(|(_, a, _)| a.offset), "end": at.map(|(_, _, b)| b.offset),
                        })
                    }).collect::<Vec<_>>(),
                    "changes": review::changes(doc).iter().map(|c| json!({
                        "kind": if c.change.kind == review::ChangeKind::Inserted { "inserted" } else { "deleted" },
                        "author": c.change.author, "date": crate::recovery::parse_iso(&c.change.date).map(crate::recovery::when).unwrap_or_default(),
                        "text": c.text.chars().take(160).collect::<String>(), "para": c.para.0, "start": c.range.start, "end": c.range.end, "is_break": c.is_break,
                    })).collect::<Vec<_>>(),
                });
                // Where each is on the page (document space), for the notes in the margin.
                let layout = self.open[self.front].session.layout();
                for list in ["comments", "changes"] {
                    for c in out[list].as_array_mut().into_iter().flatten() {
                        let (Some(p), Some(a), Some(b)) = (c["para"].as_u64(), c["start"].as_u64(), c["end"].as_u64())
                        else {
                            continue;
                        };
                        let para = NodeId(p);
                        let rects: Vec<[f64; 4]> = layout
                            .range_rects(para, a as usize..b as usize)
                            .iter()
                            .map(|r| [r.x0, r.y0, r.width(), r.height()])
                            .collect();
                        let y = rects
                            .first()
                            .map(|r| r[1])
                            .or_else(|| layout.caret(xenify_wp_doc::Pos::new(para, a as usize), false).map(|r| r.y0));
                        c["y"] = json!(y);
                        c["rects"] = json!(rects);
                    }
                }
                Ok(out)
            }
            "set_tracking" => {
                let on = args.get("on").and_then(Value::as_bool).unwrap_or(false);
                xenify_wp_doc::review::set_tracking(&mut self.doc().session.ed, on).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "add_comment" => {
                let text = arg_str(args, "text")?.trim().to_string();
                if text.is_empty() {
                    return Err("write the comment first".into());
                }
                xenify_wp_doc::review::add_comment(&mut self.doc().session.ed, &text).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "reply" => {
                let id = NodeId(arg_f64(args, "id")? as u64);
                xenify_wp_doc::review::reply(&mut self.doc().session.ed, id, arg_str(args, "text")?.trim())
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "resolve" => {
                let id = NodeId(arg_f64(args, "id")? as u64);
                let on = args.get("resolved").and_then(Value::as_bool).unwrap_or(true);
                xenify_wp_doc::review::set_resolved(&mut self.doc().session.ed, id, on).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "delete_comment" => {
                let id = NodeId(arg_f64(args, "id")? as u64);
                xenify_wp_doc::review::delete_comment(&mut self.doc().session.ed, id).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "decide" => {
                // Accept or reject: one change (by its place), the selection's, or all.
                let accept = args.get("accept").and_then(Value::as_bool).unwrap_or(true);
                let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
                let ed = &mut self.doc().session.ed;
                if let (Some(p), Some(a), Some(b)) = (
                    args.get("para").and_then(Value::as_u64),
                    args.get("start").and_then(Value::as_u64),
                    args.get("end").and_then(Value::as_u64),
                ) {
                    let para = NodeId(p);
                    ed.sel = xenify_wp_doc::Selection {
                        anchor: xenify_wp_doc::Pos::new(para, a as usize),
                        focus: xenify_wp_doc::Pos::new(para, (b as usize).max(a as usize)),
                    };
                }
                ed.seal();
                let n = xenify_wp_doc::review::decide(ed, accept, all).map_err(|e| e.to_string())?;
                let mut out = self.after();
                out["decided"] = json!(n);
                Ok(out)
            }
            "next_change" => {
                let forward = !args.get("back").and_then(Value::as_bool).unwrap_or(false);
                let ed = &mut self.doc().session.ed;
                let from = if forward { ed.doc.ordered(ed.sel).1 } else { ed.doc.ordered(ed.sel).0 };
                match xenify_wp_doc::review::next_change(&ed.doc, from, forward) {
                    Some(sel) => ed.sel = sel,
                    None => return Err("no more changes".into()),
                }
                Ok(self.after())
            }
            "select_range" => {
                let para = NodeId(arg_f64(args, "para")? as u64);
                let (a, b) = (arg_f64(args, "start")? as usize, arg_f64(args, "end")? as usize);
                let ed = &mut self.doc().session.ed;
                ed.sel = xenify_wp_doc::Selection {
                    anchor: xenify_wp_doc::Pos::new(para, a),
                    focus: xenify_wp_doc::Pos::new(para, b),
                };
                Ok(self.after())
            }
            "symbols" => Ok(json!({
                "symbols": xenify_wp_doc::symbols::groups().into_iter().map(|(g, items)| json!({ "group": g, "items": items })).collect::<Vec<_>>(),
                "emoji": xenify_wp_layout::emoji::library().iter().map(|e| json!({ "glyph": e.glyph, "name": e.name, "group": e.group, "keywords": e.keywords })).collect::<Vec<_>>(),
                "emoticons": xenify_wp_doc::autocorrect::EMOTICONS,
                "credit": xenify_wp_layout::emoji::CREDIT,
            })),
            "emoji_svg" => {
                let glyph = arg_str(args, "glyph")?;
                Ok(xenify_wp_layout::emoji::find(glyph).map_or(Value::Null, |e| json!(e.svg())))
            }
            "outline" => {
                // Headings in order, with their level and page (for navigation).
                let o = &mut self.open[self.front];
                let heads: Vec<(NodeId, u8, String)> = o
                    .session
                    .ed
                    .doc
                    .paragraphs()
                    .filter_map(|p| {
                        let level = o.session.ed.doc.styles.get(&p.style).and_then(|s| s.outline_level)?;
                        let text = p.text.trim();
                        (!text.is_empty()).then(|| (p.id, level, text.chars().take(120).collect()))
                    })
                    .collect();
                let caret = o.session.ed.sel.focus.para;
                let doc_order = |id: NodeId, d: &Document| d.para_index(id).unwrap_or(0);
                let at = doc_order(caret, &o.session.ed.doc);
                let current = heads.iter().rposition(|(id, _, _)| doc_order(*id, &o.session.ed.doc) <= at);
                let layout = o.session.layout();
                let list: Vec<Value> = heads
                    .iter()
                    .map(|(id, level, text)| {
                        let page = layout
                            .by_para
                            .get(id)
                            .and_then(|f| f.first())
                            .map(|&i| layout.fragments[i].origin.y)
                            .and_then(|y| layout.pages.iter().position(|p| y < p.rect.y1))
                            .map(|i| layout.pages[i].number);
                        json!({ "para": id.0, "level": level, "text": text, "page": page })
                    })
                    .collect();
                Ok(json!({ "headings": list, "current": current }))
            }
            "go_to" => {
                let para = NodeId(arg_f64(args, "para")? as u64);
                let o = self.doc();
                o.session.ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(para, 0));
                o.session.ed.seal();
                Ok(self.after())
            }
            "ai_apply" => {
                // Puts an AI suggestion in, if the text it was about is still there.
                let para = NodeId(arg_f64(args, "para")? as u64);
                let (a, b) = (arg_f64(args, "start")? as usize, arg_f64(args, "end")? as usize);
                let old = arg_str(args, "old")?;
                let new = arg_str(args, "new")?;
                let ed = &mut self.doc().session.ed;
                let now = ed.doc.para(para).and_then(|p| p.text.get(a..b)).map(str::to_string);
                if now.as_deref() != Some(old) {
                    return Err("that text has changed since; ask again".into());
                }
                let edits: Vec<(std::ops::Range<usize>, String)> =
                    xenify_wp_ai::edits(old, new).into_iter().map(|(r, t)| (r.start + a..r.end + a, t)).collect();
                ed.seal();
                xenify_wp_doc::ops::apply_edits(ed, para, &edits, "AI edit").map_err(|e| e.to_string())?;
                ed.seal();
                Ok(self.after())
            }
            "ai_apply_quote" => {
                // A chat proposal: replace the quoted text, or insert at the caret.
                let quote = arg_str(args, "quote")?.to_string();
                let new = arg_str(args, "new")?.to_string();
                let o = self.doc();
                if quote.trim().is_empty() {
                    o.session.paste_text(&new);
                } else {
                    let (para, range) = xenify_wp_ai::locate(&o.session.ed.doc, &quote)
                        .ok_or("that text isn't in the document any more")?;
                    o.session.ed.seal();
                    xenify_wp_doc::ops::replace_text(&mut o.session.ed, para, range, &new, "AI edit")
                        .map_err(|e| e.to_string())?;
                }
                Ok(self.after())
            }
            #[cfg(not(target_arch = "wasm32"))]
            "printers" => Ok(json!({ "printers": crate::print::printers(), "why_not": crate::print::unavailable() })),
            #[cfg(target_arch = "wasm32")]
            "printers" => Ok(json!({ "printers": [], "why_not": "print from your browser (Ctrl+P on the PDF)" })),
            #[cfg(not(target_arch = "wasm32"))]
            "print" => {
                let pdf = xenify_wp_io::save(&self.doc().session.ed.doc, Format::Pdf).map_err(|e| e.to_string())?;
                let pages = args.get("pages").and_then(Value::as_str).filter(|p| !p.trim().is_empty());
                match args.get("printer").and_then(Value::as_str) {
                    Some(printer) => {
                        let copies = args.get("copies").and_then(Value::as_u64).unwrap_or(1).clamp(1, 999) as u32;
                        crate::print::print_pdf(&pdf, printer, copies, pages)?;
                    }
                    None => crate::print::open_in_viewer(&pdf)?,
                }
                Ok(Value::Null)
            }
            "page" => {
                // The page setup of the section the caret is in.
                let ed = &self.doc().session.ed;
                let i = ed.doc.section_of(ed.sel.focus.para).unwrap_or(0);
                let sec = &ed.doc.sections[i];
                Ok(json!({ "section": i, "sections": ed.doc.sections.len(), "page": sec.page }))
            }
            "set_page" => {
                // The caret's section, or every section with `all`.
                let o = self.doc();
                let ed = &mut o.session.ed;
                let here = ed.doc.section_of(ed.sel.focus.para).unwrap_or(0);
                let which: Vec<usize> = if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
                    (0..ed.doc.sections.len()).collect()
                } else {
                    vec![here]
                };
                ed.seal();
                for i in which {
                    let start = ed.doc.sections[i].start;
                    // Only the fields sent change.
                    let mut page = serde_json::to_value(&ed.doc.sections[i].page).map_err(|e| e.to_string())?;
                    if let (Some(obj), Some(patch)) =
                        (page.as_object_mut(), args.get("page").and_then(Value::as_object))
                    {
                        for (k, v) in patch {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                    let page: xenify_wp_doc::PageSetup = serde_json::from_value(page).map_err(|e| e.to_string())?;
                    xenify_wp_doc::ops::set_section(ed, i, start, page).map_err(|e| e.to_string())?;
                }
                ed.seal();
                Ok(self.after())
            }
            "section_from_page" => {
                // A new section starting with the first paragraph that starts on the caret's
                // page (no new page), so its headers and footers can differ from there on.
                let o = &mut self.open[self.front];
                let caret = o.session.ed.sel.focus;
                let layout = o.session.layout().clone();
                let at = layout.caret(caret, false).ok_or("put the cursor in the text")?;
                let page = layout.pages.iter().find(|p| at.y0 < p.rect.y1).ok_or("put the cursor in the text")?;
                if page.first_of_section {
                    return Ok(self.after());
                }
                let first = layout
                    .fragments
                    .iter()
                    .filter(|f| f.lines.start == 0 && f.para.id != NodeId(0) && f.para.generated.is_none())
                    .filter(|f| f.rect().y0 >= page.body.y0 - 1.0 && f.rect().y0 < page.rect.y1)
                    .map(|f| f.para.id)
                    .next()
                    .ok_or("no paragraph starts on this page")?;
                let ed = &mut o.session.ed;
                let keep = ed.sel;
                ed.sel = xenify_wp_doc::Selection::caret(xenify_wp_doc::Pos::new(first, 0));
                xenify_wp_doc::ops::insert_section_break(ed, xenify_wp_doc::SectionStart::Continuous)
                    .map_err(|e| e.to_string())?;
                ed.sel = keep;
                Ok(self.after())
            }
            "find" => {
                let query = arg_str(args, "query")?.to_string();
                let case = args.get("case").and_then(Value::as_bool).unwrap_or(false);
                let whole = args.get("whole").and_then(Value::as_bool).unwrap_or(false);
                let o = self.doc();
                o.find = Some(Find { query, case, whole, hits: Vec::new(), current: None });
                self.refresh_find();
                if !matches!(args.get("stay").and_then(Value::as_bool), Some(true)) {
                    // From where the caret is, like a browser.
                    let o = self.doc();
                    let start = o.session.ed.doc.ordered(o.session.ed.sel).0;
                    o.session.ed.sel = xenify_wp_doc::Selection::caret(start);
                    self.select_hit(false);
                }
                Ok(self.after())
            }
            "find_next" => {
                self.refresh_find();
                self.select_hit(args.get("back").and_then(Value::as_bool).unwrap_or(false));
                Ok(self.after())
            }
            "replace" => {
                let with = arg_str(args, "with")?.to_string();
                self.refresh_find();
                let o = self.doc();
                if let Some(f) = &o.find
                    && let Some(i) = f.current
                {
                    let (p, r) = f.hits[i].clone();
                    o.session.ed.seal();
                    xenify_wp_doc::ops::replace_text(&mut o.session.ed, p, r, &with, "Replace")
                        .map_err(|e| e.to_string())?;
                }
                self.refresh_find();
                self.select_hit(false);
                Ok(self.after())
            }
            "replace_all" => {
                let with = arg_str(args, "with")?.to_string();
                self.refresh_find();
                let o = self.doc();
                let hits = o.find.as_ref().map(|f| f.hits.clone()).unwrap_or_default();
                let n = xenify_wp_doc::ops::replace_all(&mut o.session.ed, &hits, &with).map_err(|e| e.to_string())?;
                let mut out = self.after();
                out["replaced"] = json!(n);
                Ok(out)
            }
            "find_close" => {
                self.doc().find = None;
                Ok(self.after())
            }
            "insert" => {
                let what = arg_str(args, "what")?;
                let ed = &mut self.doc().session.ed;
                ed.seal();
                match what {
                    "page_break" => xenify_wp_doc::ops::insert_page_break(ed),
                    "footnote" => xenify_wp_doc::notes::insert_note(ed, xenify_wp_doc::NoteKind::Foot).map(|_| ()),
                    "endnote" => xenify_wp_doc::notes::insert_note(ed, xenify_wp_doc::NoteKind::End).map(|_| ()),
                    "toc" => xenify_wp_doc::tables::insert_toc(
                        ed,
                        args.get("levels").and_then(Value::as_u64).unwrap_or(3).clamp(1, 6) as u8,
                    ),
                    "change_log" => xenify_wp_doc::tables::insert_generated(ed, xenify_wp_doc::TocKind::ChangeLog),
                    "title_page" => xenify_wp_doc::ops::insert_title_page(ed),
                    "page_number" => xenify_wp_doc::ops::insert_field(ed, xenify_wp_doc::Field::Page, "Page number"),
                    "date" => xenify_wp_doc::ops::insert_field(ed, xenify_wp_doc::Field::Date, "Date"),
                    "datetime" => {
                        let format = args
                            .get("format")
                            .and_then(Value::as_str)
                            .unwrap_or(xenify_wp_doc::datetime::PRESETS[0])
                            .to_string();
                        let auto = args.get("auto").and_then(Value::as_bool).unwrap_or(false);
                        let fixed = (!auto).then(|| xenify_wp_doc::datetime::format(&format, &xenify_wp_layout::now()));
                        xenify_wp_doc::ops::insert_field(
                            ed,
                            xenify_wp_doc::Field::DateTime { format, fixed },
                            "Date and time",
                        )
                    }
                    "table" => {
                        let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(3).clamp(1, 60) as usize;
                        let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(3).clamp(1, 20) as usize;
                        xenify_wp_doc::tables::insert_table(ed, rows, cols)
                    }
                    _ => return Err("can't insert that".into()),
                }
                .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "insert_picture" => {
                let path = PathBuf::from(arg_str(args, "path")?);
                // A Draw canvas: shown from the file, so it follows the canvas.
                if is_canvas(&path) {
                    let bytes = read_picture(&path)?;
                    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
                        .with_guessed_format()
                        .ok()
                        .and_then(|r| r.into_dimensions().ok())
                        .ok_or("the canvas's picture can't be shown")?;
                    let dir = self.open[self.front].path.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                    let src = match dir.as_ref().and_then(|d| path.strip_prefix(d).ok()) {
                        Some(rel) => rel.display().to_string(),
                        None => path.display().to_string(),
                    };
                    let ed = &mut self.doc().session.ed;
                    xenify_wp_doc::tables::insert_image(ed, &src, w as f32 * 0.75, h as f32 * 0.75)
                        .map_err(|e| e.to_string())?;
                    return Ok(self.after());
                }
                let bytes = std::fs::read(&path).map_err(|e| format!("couldn't read the picture: {e}"))?;
                let name = path.file_name().map_or("image.png".into(), |n| n.to_string_lossy().replace(' ', "-"));
                self.put_picture(&name, bytes)?;
                Ok(self.after())
            }
            "asset" => {
                // A picture's bytes for the screen, as a data URL.
                use base64::Engine;
                let src = arg_str(args, "src")?;
                let o = &self.open[self.front];
                let bytes = o.session.ed.doc.assets.get(src).cloned().or_else(|| {
                    let p = PathBuf::from(src);
                    let p = if p.is_absolute() { p } else { o.path.as_ref()?.parent()?.join(p) };
                    read_picture(&p).ok().map(std::sync::Arc::new)
                });
                let Some(bytes) = bytes else { return Ok(Value::Null) };
                let mime = match image::guess_format(&bytes) {
                    Ok(image::ImageFormat::Jpeg) => "image/jpeg",
                    Ok(image::ImageFormat::Gif) => "image/gif",
                    Ok(image::ImageFormat::WebP) => "image/webp",
                    Ok(image::ImageFormat::Bmp) => "image/bmp",
                    _ => "image/png",
                };
                Ok(json!(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&*bytes))))
            }
            "link" => {
                let url = arg_str(args, "url")?.trim().to_string();
                let o = self.doc();
                // Nothing selected and words given (a page's name): they go
                // in, linked.
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.is_empty() && o.session.ed.sel.is_collapsed() {
                    o.session.insert_link(text, &url);
                    return Ok(self.after());
                }
                let fmt = CharFmt { link: Some(url), ..Default::default() };
                o.session.format(&fmt, "Link");
                Ok(self.after())
            }
            "lookup" => {
                // The word at a point (or the caret), with its meanings and
                // other words already in its form.
                let o = &mut self.open[self.front];
                let pos = match (args.get("x").and_then(Value::as_f64), args.get("y").and_then(Value::as_f64)) {
                    (Some(x), Some(y)) => o.session.layout().hit(kurbo::Point::new(x, y)),
                    _ => Some(o.session.ed.sel.focus),
                };
                let Some(pos) = pos else { return Ok(Value::Null) };
                let text = o.session.ed.doc.para(pos.para).map(|p| p.text.clone()).unwrap_or_default();
                let Some(range) = xenify_wp_check::lexicon::word_at(&text, pos.offset) else { return Ok(Value::Null) };
                let word = text[range.clone()].to_string();
                let caret = o
                    .session
                    .layout()
                    .range_rects(pos.para, range.clone())
                    .first()
                    .map(|r| [r.x0, r.y0, r.width(), r.height()]);
                #[cfg_attr(not(feature = "checker"), allow(unused_mut))]
                let mut out =
                    json!({ "para": pos.para.0, "start": range.start, "end": range.end, "word": word, "rect": caret });
                // Without the checker built in (the browser's first download)
                // the screens ask the checker's own worker for the meanings.
                #[cfg(feature = "checker")]
                if let (Value::Object(out), Value::Object(entry)) = (&mut out, crate::proof::look_up(&word)) {
                    out.extend(entry);
                }
                Ok(out)
            }
            "swap" => {
                let para = NodeId(arg_f64(args, "para")? as u64);
                let (a, b) = (arg_f64(args, "start")? as usize, arg_f64(args, "end")? as usize);
                let with = arg_str(args, "with")?.to_string();
                let o = self.doc();
                o.session.ed.seal();
                xenify_wp_doc::ops::replace_text(&mut o.session.ed, para, a..b, &with, "Synonym")
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "fix" => {
                let i = self.issue(args)?;
                let with = arg_str(args, "with")?.to_string();
                let o = self.doc();
                o.session.ed.seal();
                xenify_wp_doc::ops::replace_text(&mut o.session.ed, i.para, i.range.clone(), &with, "Correction")
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "server_words" => {
                // Signed in (or out): your words on that server.
                self.server_words = args
                    .get("words")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                Ok(json!({}))
            }
            "doc_word" => {
                // A word that's right in this document only (kept in it).
                let word = arg_str(args, "word")?.trim().to_string();
                if word.is_empty() {
                    return Err("no word".into());
                }
                let o = self.doc();
                let mut meta = o.session.ed.doc.meta.clone();
                if args.get("remove").and_then(Value::as_bool).unwrap_or(false) {
                    meta.ignore_words.retain(|w| *w != word);
                } else if !meta.ignore_words.contains(&word) {
                    meta.ignore_words.push(word.clone());
                }
                let _ = xenify_wp_doc::ops::set_meta(&mut o.session.ed, meta, "Document's words");
                Ok(self.after())
            }
            "ignore" => {
                let i = self.issue(args)?;
                let o = self.doc();
                let text = o.session.ed.doc.para(i.para).map(|p| p.text.clone()).unwrap_or_default();
                let mut meta = o.session.ed.doc.meta.clone();
                meta.ignore_issues.push(i.ignore_key(&text));
                let _ = xenify_wp_doc::ops::set_meta(&mut o.session.ed, meta, "Ignore");
                o.issues.retain(|x| x != &i);
                Ok(self.after())
            }
            "select_issue" => {
                let i = self.issue(args)?;
                let o = self.doc();
                o.session.ed.sel = xenify_wp_doc::Selection {
                    anchor: xenify_wp_doc::Pos::new(i.para, i.range.start),
                    focus: xenify_wp_doc::Pos::new(i.para, i.range.end),
                };
                Ok(self.after())
            }
            "issue_at" => {
                // The issue under document point (x, y), for the right-click menu.
                let (x, y) = (arg_f64(args, "x")?, arg_f64(args, "y")?);
                let live: Vec<(usize, Issue)> = self.live_issues().into_iter().map(|(n, i)| (n, i.clone())).collect();
                let layout = self.open[self.front].session.layout();
                let hit = live.iter().find(|(_, i)| {
                    layout
                        .range_rects(i.para, i.range.clone())
                        .iter()
                        .any(|r| r.inflate(1.0, 2.0).contains(kurbo::Point::new(x, y)))
                });
                Ok(match hit {
                    Some((n, _)) => self
                        .issue_list()
                        .as_array()
                        .and_then(|l| l.iter().find(|v| v["i"] == json!(n)).cloned())
                        .unwrap_or(Value::Null),
                    None => Value::Null,
                })
            }
            "autosave" => {
                for o in &mut self.open {
                    // Server documents aren't kept here: their unsent changes
                    // are kept apart (the screens' keepUnsent).
                    if o.remote.is_some() {
                        o.autosave.clear();
                        continue;
                    }
                    o.autosave.tick(&o.session.ed.doc, o.session.ed.is_dirty());
                }
                Ok(Value::Null)
            }
            "discard_all_autosaves" => {
                // Quitting with "Don't save".
                for o in &mut self.open {
                    o.autosave.clear();
                }
                Ok(Value::Null)
            }
            "recovery" => {
                // Work to offer back: left for open files, and never-saved documents.
                // Empty documents aren't worth offering back.
                let (orphans, empty): (Vec<_>, Vec<_>) =
                    self.autosave_base.orphans().into_iter().partition(|(_, d, _)| worth_keeping(d));
                for (_, _, k) in empty {
                    self.autosave_base.forget(None, Some(k.as_str()));
                }
                Ok(json!({
                    "left": self.open.iter().filter_map(|o| o.left.as_ref().map(|(i, _)| json!({
                        "id": o.id, "title": o.title(), "when": crate::recovery::when(i.time),
                    }))).collect::<Vec<_>>(),
                    "orphans": orphans.iter().map(|(i, d, k)| json!({
                        "key": k, "title": if i.title.is_empty() { d.meta.title.clone() } else { i.title.clone() },
                        "when": crate::recovery::when(i.time), "words": d.word_count(),
                        "start": opening_words(d),
                    })).collect::<Vec<_>>(),
                }))
            }
            "recover" => {
                if let Some(id) = args.get("id").and_then(Value::as_u64) {
                    // The open file's newer unsaved work replaces what was loaded (undoable).
                    let o = self.open.iter_mut().find(|o| o.id == id as u32).ok_or("that document is closed")?;
                    if let Some((_, doc)) = o.left.take() {
                        let _ = xenify_wp_doc::restore(&mut o.session.ed, doc);
                    }
                    self.front = self.open.iter().position(|o| o.id == id as u32).unwrap_or(self.front);
                } else {
                    let key = arg_str(args, "key")?.to_string();
                    let found = self.autosave_base.orphans().into_iter().find(|(_, _, k)| *k == key);
                    if let Some((_, doc, k)) = found {
                        self.add(Document::default(), None);
                        let o = self.open.last_mut().expect("just added");
                        let _ = xenify_wp_doc::restore(&mut o.session.ed, doc);
                        self.autosave_base.forget(None, Some(k.as_str()));
                    }
                }
                Ok(self.after())
            }
            "forget_recovery" => {
                if let Some(id) = args.get("id").and_then(Value::as_u64) {
                    if let Some(o) = self.open.iter_mut().find(|o| o.id == id as u32) {
                        o.left = None;
                        if let Some(p) = o.path.clone() {
                            o.autosave.forget(Some(&p), None);
                        }
                    }
                } else if let Some(k) = args.get("key").and_then(Value::as_str) {
                    self.autosave_base.forget(None, Some(k));
                }
                Ok(Value::Null)
            }
            "prefs" => Ok(json!(self.prefs)),
            "set_prefs" => {
                let p: crate::prefs::Prefs = serde_json::from_value(args.get("prefs").cloned().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                // The lists the screens don't edit stay as they are.
                let (recent, open_files) = (self.prefs.recent.clone(), self.prefs.open_files.clone());
                self.prefs = crate::prefs::Prefs { recent, open_files, ..p };
                self.prefs.save();
                let user = self.user_name();
                for o in &mut self.open {
                    if o.remote.is_none() {
                        o.session.ed.doc.user = user.clone();
                    }
                    o.session.autoformat = self.prefs.autoformat.clone();
                    o.session.autocorrect = self.prefs.autocorrect.clone();
                }
                Ok(json!(self.prefs))
            }
            "clear_recent" => {
                self.prefs.recent.clear();
                self.prefs.save();
                Ok(json!(self.prefs))
            }
            "log" => {
                eprintln!("ui: {}", args.get("text").and_then(Value::as_str).unwrap_or(""));
                Ok(Value::Null)
            }
            "scenes" => {
                let pages: Vec<usize> = serde_json::from_value(args.get("pages").cloned().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                let o = &mut self.open[self.front];
                Ok(json!(o.session.scenes(&pages, &mut self.glyphs)))
            }
            "glyphs" => {
                let font = arg_f64(args, "font")? as u32;
                let ids: Vec<u32> =
                    serde_json::from_value(args.get("ids").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
                Ok(json!(self.glyphs.outlines(font, &ids)))
            }
            "key" => {
                let handled = self.doc().session.key(arg_str(args, "key")?, mods(args));
                let mut out = self.after();
                out["handled"] = json!(handled);
                Ok(out)
            }
            "text" => {
                self.doc().session.type_text(arg_str(args, "text")?);
                Ok(self.after())
            }
            "paste" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                let html = args.get("html").and_then(Value::as_str).unwrap_or("").to_string();
                let plain = args.get("plain").and_then(Value::as_bool).unwrap_or(false);
                self.doc().session.paste(&text, &html, plain);
                Ok(self.after())
            }
            "copy" => {
                self.doc().session.copy();
                Ok(json!({ "text": self.doc().session.selected_text() }))
            }
            "cut" => {
                let text = self.doc().session.cut();
                let mut out = self.after();
                out["text"] = json!(text);
                Ok(out)
            }
            "down" => {
                let (x, y) = (arg_f64(args, "x")?, arg_f64(args, "y")?);
                let clicks = args.get("clicks").and_then(Value::as_u64).unwrap_or(1) as u32;
                self.doc().session.pointer_down(x, y, clicks, mods(args));
                let link = self.doc().session.link_clicked.take();
                let mut out = self.after();
                if let Some(url) = link {
                    out["open_link"] = json!(url);
                }
                Ok(out)
            }
            "move" => {
                let (x, y) = (arg_f64(args, "x")?, arg_f64(args, "y")?);
                self.doc().session.pointer_move(x, y);
                Ok(self.after())
            }
            "up" => {
                self.doc().session.pointer_up();
                Ok(self.after())
            }
            "toggle" => {
                let what = Toggle::parse(arg_str(args, "what")?).ok_or("unknown format")?;
                self.doc().session.toggle(what);
                Ok(self.after())
            }
            "style" => {
                self.doc().session.set_style(arg_str(args, "id")?);
                Ok(self.after())
            }
            "align" => {
                self.doc().session.set_align(arg_str(args, "to")?);
                Ok(self.after())
            }
            "list" => {
                self.doc().session.toggle_list(arg_str(args, "kind")?);
                Ok(self.after())
            }
            "font" => {
                let font = arg_str(args, "name")?.to_string();
                self.doc().session.format(&CharFmt { font: Some(font), ..Default::default() }, "Font");
                Ok(self.after())
            }
            "size" => {
                let size = arg_f64(args, "pt")? as f32;
                self.doc()
                    .session
                    .format(&CharFmt { size: Some(size.clamp(1.0, 1638.0)), ..Default::default() }, "Size");
                Ok(self.after())
            }
            "color" => {
                let c = Rgba::from_hex(arg_str(args, "hex")?).ok_or("not a colour")?;
                self.doc().session.format(&CharFmt { color: Some(c), ..Default::default() }, "Text colour");
                Ok(self.after())
            }
            "highlight" => {
                // A highlight colour, or none (fully transparent).
                let c = match args.get("hex").and_then(Value::as_str) {
                    Some(h) => Rgba::from_hex(h).ok_or("not a colour")?,
                    None => Rgba([0, 0, 0, 0]),
                };
                self.doc().session.format(&CharFmt { highlight: Some(c), ..Default::default() }, "Highlight");
                Ok(self.after())
            }
            "para_look" => {
                // Shading behind the paragraph and a box around it.
                let mut patch = xenify_wp_doc::ParaProps::default();
                match arg_str(args, "what")? {
                    "shade" => patch.shading = Some(Rgba::from_hex(arg_str(args, "hex")?).ok_or("not a colour")?),
                    "no_shade" => patch.shading = Some(Rgba([0, 0, 0, 0])),
                    "box" => {
                        patch.borders = Some(xenify_wp_doc::ParaBorders::all(xenify_wp_doc::Border {
                            width: 0.75,
                            color: Rgba([0x57, 0x53, 0x4E, 255]),
                        }))
                    }
                    "line_above" => {
                        patch.borders = Some(xenify_wp_doc::ParaBorders {
                            top: Some(xenify_wp_doc::Border { width: 1.0, color: Rgba([0x57, 0x53, 0x4E, 255]) }),
                            ..Default::default()
                        })
                    }
                    "no_border" => patch.borders = Some(xenify_wp_doc::ParaBorders::default()),
                    _ => return Err("unknown".into()),
                }
                let ed = &mut self.doc().session.ed;
                ed.seal();
                xenify_wp_doc::ops::apply_para_props(ed, &patch, "Borders and shading").map_err(|e| e.to_string())?;
                ed.seal();
                Ok(self.after())
            }
            "clear_format" => {
                self.doc().session.clear_format();
                Ok(self.after())
            }
            "undo" => {
                self.doc().session.undo();
                Ok(self.after())
            }
            "redo" => {
                self.doc().session.redo();
                Ok(self.after())
            }
            "styles" => Ok(json!(
                self.doc()
                    .session
                    .styles()
                    .into_iter()
                    .map(|(id, name, level)| {
                        let list = self.open[self.front].session.ed.doc.styles.get(&id).and_then(|s| s.list.clone());
                        json!({"id": id, "name": name, "level": level, "list": list})
                    })
                    .collect::<Vec<_>>()
            )),
            "fonts" => Ok(json!(self.doc().session.font_families())),
            "new" => {
                let id = args.get("template").and_then(Value::as_str).unwrap_or("blank");
                let mut doc = template_document(id).ok_or("that template is missing")?;
                if let Some(name) = id.strip_prefix("mine:") {
                    // Linked: the template keeps owning its parts, styles and page setup.
                    doc = from_template(doc, name, id);
                }
                let mut doc = templates::apply_title(doc, args.get("title").and_then(Value::as_str).unwrap_or(""));
                if let Some(r) = args.get("reader").and_then(Value::as_str) {
                    doc.meta.audience = r.to_string();
                }
                if let Some(t) = args.get("tone").and_then(Value::as_str) {
                    doc.meta.tone = t.to_string();
                }
                // A blank, untouched tab is replaced rather than kept.
                if self.open.len() == 1
                    && self.open[0].path.is_none()
                    && !self.open[0].session.ed.is_dirty()
                    && self.open[0].session.ed.doc.word_count() == 0
                {
                    self.open.clear();
                    self.untitled = 0;
                }
                self.add(doc, None);
                Ok(self.after())
            }
            "templates" => {
                let profiles = all_kinds();
                Ok(json!(
                    templates::TEMPLATES
                        .iter()
                        .map(|t| {
                            let p = profiles.iter().find(|p| p.id == t.profile);
                            json!({
                                "id": t.id, "group": t.group, "name": t.name, "about": t.about,
                                "profile": t.profile,
                                "profile_name": p.map(|p| p.name.clone()).unwrap_or_default(),
                                "profile_note": p.map(|p| p.description.clone()).unwrap_or_default(),
                                "reader": t.reader, "tone": t.tone,
                                "fill": templates::fill_in_labels(t.id),
                            })
                        })
                        .chain(my_templates().into_iter().map(|(name, d)| {
                            let id = d.meta.proofing_profile.clone().unwrap_or_else(|| "standard".into());
                            let p = profiles.iter().find(|p| p.id == id);
                            json!({
                                "id": format!("mine:{name}"), "group": "Mine", "name": name, "mine": true,
                                "about": "One of your own templates.",
                                "profile": id,
                                "profile_name": p.map(|p| p.name.clone()).unwrap_or_default(),
                                "profile_note": p.map(|p| p.description.clone()).unwrap_or_default(),
                                "reader": if d.meta.audience.is_empty() { "General readers".into() } else { d.meta.audience.clone() },
                                "tone": if d.meta.tone.is_empty() { "Neutral".into() } else { d.meta.tone.clone() },
                                "fill": Vec::<String>::new(),
                            })
                        }))
                        .collect::<Vec<_>>()
                ))
            }
            "save_template" => {
                // The front document as one of the person's templates (replacing one with that name).
                let raw = arg_str(args, "name")?.trim();
                if raw.is_empty() {
                    return Err("give the template a name".into());
                }
                let name = safe_name(raw);
                let mut doc = self.doc().session.ed.doc.clone();
                doc.meta.title.clear();
                let bytes = xenify_wp_io::save(&doc, Format::Igdoc).map_err(|e| e.to_string())?;
                std::fs::create_dir_all(templates_dir()).map_err(|e| e.to_string())?;
                std::fs::write(templates_dir().join(format!("{name}.xwrite")), bytes).map_err(|e| e.to_string())?;
                Ok(json!({ "name": name }))
            }
            "template_status" => {
                // The front document's template: whether it exists and has changed since applied.
                let Some(link) = self.doc().session.ed.doc.meta.template.clone() else { return Ok(Value::Null) };
                let tpl = my_templates().into_iter().find(|(n, _)| *n == link.name).map(|(_, d)| d);
                Ok(json!({
                    "name": link.name,
                    "exists": tpl.is_some(),
                    "out_of_date": tpl.as_ref().is_some_and(|t| xenify_wp_doc::linked::out_of_date(&self.doc().session.ed.doc, t)),
                }))
            }
            "template_update" => {
                // Brings the template's styles and page setup into the front document.
                let name = match args.get("name").and_then(Value::as_str) {
                    Some(n) => n.to_string(),
                    None => self
                        .doc()
                        .session
                        .ed
                        .doc
                        .meta
                        .template
                        .as_ref()
                        .map(|t| t.name.clone())
                        .ok_or("this document has no template")?,
                };
                let tpl = my_templates()
                    .into_iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, d)| d)
                    .ok_or("that template is gone")?;
                xenify_wp_doc::linked::apply(&mut self.doc().session.ed, &tpl, &name).map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "template_use" => {
                // Puts the front document (its own text) into a template: it
                // takes the template's styles, page setup, headers and footers
                // and any parts around the text, and follows the template from
                // then on. `data`: the template's file, base64 (a server's);
                // else `name` of one of the person's own, or `path` of a file.
                use base64::Engine;
                let (tpl, name, from) = match (args.get("data").and_then(Value::as_str), args.get("path").and_then(Value::as_str)) {
                    (Some(data), _) => {
                        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
                        let doc = xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc;
                        (doc, arg_str(args, "name")?.to_string(), args.get("from").and_then(Value::as_str).unwrap_or("").to_string())
                    }
                    (None, Some(path)) => {
                        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                        let doc = xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc;
                        let name = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        (doc, name, format!("file:{path}"))
                    }
                    (None, None) => {
                        let name = arg_str(args, "name")?.to_string();
                        let doc = my_templates().into_iter().find(|(n, _)| *n == name).map(|(_, d)| d).ok_or("that template is gone")?;
                        (doc, name.clone(), format!("mine:{name}"))
                    }
                };
                {
                    let ed = &mut self.doc().session.ed;
                    if ed.doc.meta.is_template {
                        return Err("This document is a template itself. Use File › New from template to make a document from it.".into());
                    }
                    // Linked with no parts of the template yet: the whole document is its text.
                    let mut meta = ed.doc.meta.clone();
                    meta.template = Some(xenify_wp_doc::linked::TemplateLink { name: name.clone(), from, ..Default::default() });
                    xenify_wp_doc::ops::set_meta(ed, meta, "Use a template").map_err(|e| e.to_string())?;
                }
                self.refresh_from(tpl)?;
                let mut out = self.after();
                out["template"] = json!(name);
                Ok(out)
            }
            "template_pictures_missing" => {
                // Pictures the front document names but hasn't got: a server
                // template's (its copy carries no pictures), to fetch from it.
                let d = &self.doc().session.ed.doc;
                let missing: Vec<String> =
                    crate::remote::pictures(d).into_iter().filter(|k| !d.assets.contains_key(k)).collect();
                Ok(json!(missing))
            }
            "template_picture" => {
                // One of those pictures: kept in the document as if put in
                // here (so it goes up with it when it's on a server).
                use base64::Engine;
                let name = arg_str(args, "name")?.to_string();
                let bytes = base64::engine::general_purpose::STANDARD.decode(arg_str(args, "data")?).map_err(|e| e.to_string())?;
                self.doc().session.ed.doc.assets.insert(name, std::sync::Arc::new(bytes));
                Ok(self.after())
            }
            "template_unlink" => {
                let ed = &mut self.doc().session.ed;
                let mut meta = ed.doc.meta.clone();
                meta.template = None;
                let _ = xenify_wp_doc::ops::set_meta(ed, meta, "Detach from template");
                Ok(self.after())
            }
            "template_edit" => {
                // Opens a template itself to change it (saving it changes the template).
                // `path`: a template file elsewhere on this computer.
                match args.get("path").and_then(Value::as_str) {
                    Some(path) => self.open_file_as(PathBuf::from(path), true)?,
                    None => self.open_file_as(template_file(arg_str(args, "name")?), true)?,
                }
                Ok(self.after())
            }
            "template_info" => {
                let d = &self.doc().session.ed.doc;
                let link = d.meta.template.as_ref();
                Ok(json!({
                    "is_template": d.meta.is_template,
                    "body": d.meta.body_slot.map(|n| n.0),
                    "name": link.map(|t| t.name.clone()),
                    "from": link.map(|t| t.from.clone()),
                    "fixed": link.is_some_and(|t| t.before + t.after > 0),
                }))
            }
            "template_mark_body" => {
                // The caret's paragraph is where each document's text goes; the document becomes a template.
                let ed = &mut self.doc().session.ed;
                let para = ed.sel.focus.para;
                let top =
                    ed.doc.sections.iter().any(|s| {
                        s.blocks.iter().any(|b| matches!(b, xenify_wp_doc::Block::Paragraph(p) if p.id == para))
                    });
                if !top {
                    return Err("Put the cursor in a paragraph of its own (not in a table, a text box or a header) to mark where the text goes.".into());
                }
                let mut meta = ed.doc.meta.clone();
                meta.body_slot = Some(para);
                meta.is_template = true;
                xenify_wp_doc::ops::set_meta(ed, meta, "Mark where the text goes").map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "template_set" => {
                // This document is a template (opening it makes a new document), or not.
                let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
                let ed = &mut self.doc().session.ed;
                let mut meta = ed.doc.meta.clone();
                meta.is_template = on;
                if !on {
                    meta.body_slot = None;
                }
                xenify_wp_doc::ops::set_meta(ed, meta, if on { "Make a template" } else { "Not a template" })
                    .map_err(|e| e.to_string())?;
                Ok(self.after())
            }
            "template_starter" => {
                // The document becomes a template, with the starting parts
                // ticked in "This is a template" (one undo step).
                let parts: Vec<String> = args
                    .get("parts")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let ed = &mut self.doc().session.ed;
                ed.seal();
                ed.group("Make a template", None, |ed| template_starter(ed, &parts)).map_err(|e| e.to_string())?;
                ed.seal();
                Ok(self.after())
            }
            "template_refresh" => {
                // The template as it is now (`data`: its file, base64, from a
                // server; else read here) into the front document.
                let tpl = match args.get("data").and_then(Value::as_str) {
                    Some(data) => {
                        use base64::Engine;
                        let bytes =
                            base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
                        Some(xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc)
                    }
                    None => self.template_here(),
                };
                let Some(tpl) = tpl else { return Ok(json!({ "changed": false, "missing": true })) };
                let changed = self.refresh_from(tpl)?;
                let mut out = self.after();
                out["changed"] = json!(changed);
                Ok(out)
            }
            "delete_template" => {
                let name =
                    arg_str(args, "id")?.strip_prefix("mine:").ok_or("only your own templates can be removed")?;
                std::fs::remove_file(template_file(name)).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            "preview" => {
                // The real first page of an example, as it would be created.
                let id = arg_str(args, "template")?;
                let doc = template_document(id).ok_or("that template is missing")?;
                let doc = templates::apply_title(doc, args.get("title").and_then(Value::as_str).unwrap_or(""));
                let mut s = Session::new(doc, self.fonts.clone());
                let view = s.view();
                let scene = s.scenes(&[0], &mut self.glyphs);
                Ok(json!({ "page": view.pages.first(), "scene": scene.first() }))
            }
            "details" => {
                let m = &self.doc().session.ed.doc.meta;
                let profiles = all_kinds();
                let profile = m.proofing_profile.clone().unwrap_or_else(|| "standard".into());
                Ok(json!({
                    "title": m.title, "reader": m.audience, "tone": m.tone, "profile": profile,
                    "words": m.ignore_words, "colors": m.colors.iter().map(|c| c.to_hex()).collect::<Vec<_>>(),
                    "profiles": profiles.iter().map(|p| json!({"id": p.id, "name": p.name, "about": p.description})).collect::<Vec<_>>(),
                }))
            }
            // The document's own colours in the colour pickers: one added (put
            // first; already there, it moves up) or taken off the list.
            "add_color" | "remove_color" => {
                let c = xenify_wp_doc::Rgba::from_hex(arg_str(args, "hex")?).ok_or("that isn't a colour")?;
                let ed = &mut self.doc().session.ed;
                let mut meta = ed.doc.meta.clone();
                meta.colors.retain(|x| *x != c);
                if name == "add_color" {
                    meta.colors.insert(0, c);
                    meta.colors.truncate(24);
                }
                if meta != ed.doc.meta {
                    let label = if name == "add_color" { "Add a colour" } else { "Remove a colour" };
                    let _ = xenify_wp_doc::ops::set_meta(ed, meta, label);
                }
                Ok(self.after())
            }
            "set_details" => {
                let ed = &mut self.doc().session.ed;
                let mut meta = ed.doc.meta.clone();
                if let Some(v) = args.get("title").and_then(Value::as_str) {
                    meta.title = v.to_string();
                }
                if let Some(v) = args.get("reader").and_then(Value::as_str) {
                    meta.audience = v.to_string();
                }
                if let Some(v) = args.get("tone").and_then(Value::as_str) {
                    meta.tone = v.to_string();
                }
                if let Some(v) = args.get("profile").and_then(Value::as_str) {
                    meta.proofing_profile = Some(v.to_string());
                }
                let _ = xenify_wp_doc::ops::set_meta(ed, meta, "Document details");
                Ok(self.after())
            }
            "open" => {
                self.open_file(PathBuf::from(arg_str(args, "path")?))?;
                Ok(self.after())
            }
            "switch" => {
                let id = arg_f64(args, "id")? as u32;
                self.front = self.open.iter().position(|o| o.id == id).ok_or("that document is closed")?;
                Ok(self.after())
            }
            "close" => {
                let id = arg_f64(args, "id")? as u32;
                if let Some(i) = self.open.iter().position(|o| o.id == id) {
                    // Closed on purpose (saved, or "Don't save"): its autosave goes.
                    self.open[i].autosave.clear();
                    self.open.remove(i);
                    if self.open.is_empty() {
                        self.add(Document::default(), None);
                    }
                    self.front = self.front.min(self.open.len() - 1);
                    if i < self.front {
                        self.front -= 1;
                    }
                }
                Ok(self.after())
            }
            "save" => {
                let o = &mut self.open[self.front];
                let path = match args.get("path").and_then(Value::as_str) {
                    Some(p) => PathBuf::from(p),
                    None => o.path.clone().ok_or("choose where to save it first")?,
                };
                let format = Format::from_name(&path.to_string_lossy()).unwrap_or(Format::Markdown);
                if !format.can_open() {
                    return Err("use Export for PDF".into());
                }
                let bytes = xenify_wp_io::save(&o.session.ed.doc, format).map_err(|e| e.to_string())?;
                write_file(&path, &bytes)?;
                o.session.ed.mark_saved();
                o.autosave.clear();
                o.path = Some(path.clone());
                o.autosave.bind(Some(&path));
                self.prefs.remember(&path.display().to_string());
                self.prefs.save();
                Ok(self.after())
            }
            "html_of" => {
                // A saved document's bytes (base64) as a web page, for reading
                // it without opening it (the server's Files page).
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(arg_str(args, "data")?)
                    .map_err(|e| e.to_string())?;
                let doc = xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc;
                Ok(json!({ "html": xenify_wp_io::html::write(&doc), "title": doc.meta.title }))
            }
            "export" => {
                let path = PathBuf::from(arg_str(args, "path")?);
                let format = Format::from_name(&path.to_string_lossy()).ok_or("choose PDF or a web page")?;
                let bytes = xenify_wp_io::save(&self.doc().session.ed.doc, format).map_err(|e| e.to_string())?;
                write_file(&path, &bytes)?;
                Ok(json!({ "path": path.display().to_string() }))
            }
            "losses" => {
                let format = Format::from_name(&format!("x.{}", arg_str(args, "ext")?)).ok_or("unknown file type")?;
                Ok(json!(xenify_wp_io::losses(&self.doc().session.ed.doc, format)))
            }
            "save_info" => {
                // What Save as suggests: a name from the title, the usual type, the folder.
                let o = &self.open[self.front];
                let ext = o
                    .path
                    .as_ref()
                    .and_then(|p| p.extension())
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.prefs.save_as.clone());
                let dir = o.path.as_ref().and_then(|p| p.parent()).map(|p| p.display().to_string()).or_else(|| {
                    Some(xenify_wp_check::home_dir().join("Documents"))
                        .filter(|d| d.is_dir())
                        .map(|d| d.display().to_string())
                });
                Ok(
                    json!({ "name": format!("{}.{ext}", safe_name(&o.title())), "stem": safe_name(&o.title()), "ext": ext, "dir": dir, "has_path": o.path.is_some() }),
                )
            }
            "dirty_tabs" => Ok(json!(
                self.open
                    .iter()
                    // A server document edited together is saved as it's
                    // typed: unsaved only while changes haven't reached it.
                    .filter(|o| match o.remote.as_ref().and_then(|r| r.live.as_ref()) {
                        Some(l) => !l.settled(),
                        None => o.session.ed.is_dirty(),
                    })
                    .map(|o| json!({"id": o.id, "title": o.title()}))
                    .collect::<Vec<_>>()
            )),
            _ => Err(format!("unknown call “{name}”")),
        }
    }
}

/// A file name from a title: no slashes or other characters file systems refuse.
fn safe_name(title: &str) -> String {
    let s: String = title.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "Untitled".into() } else { s.chars().take(80).collect() }
}

/// Writes beside the file, then moves it into place: a crash part-way never
/// leaves half a document.
fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let tmp =
        path.with_extension(format!("{}.saving", path.extension().map(|e| e.to_string_lossy()).unwrap_or_default()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("couldn't save: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't save: {e}")
    })
}

fn text_hash(t: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn kind_of(c: Category) -> &'static str {
    match c {
        Category::Spelling => "spelling",
        Category::Grammar => "grammar",
        Category::Style => "style",
        Category::Readability => "readability",
        Category::Register => "register",
    }
}

/// The card's heading: what kind of thing it is, in plain words.
fn label_of(i: &Issue) -> &'static str {
    match i.category {
        Category::Spelling => "Spelling",
        Category::Grammar => "Grammar",
        Category::Style => "Clearer wording",
        Category::Readability => "Easier to read",
        Category::Register => "Tone",
    }
}

/// Pictures that came without a size (Markdown `![](x.png)`) get their own
/// shape: their pixel size at 96 dpi (the layout fits them to the column).
fn size_pictures(doc: &mut Document) {
    fn go(blocks: &mut [xenify_wp_doc::Block], assets: &std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>) {
        for b in blocks {
            match b {
                xenify_wp_doc::Block::Image(i) if i.width <= 0.0 || i.height <= 0.0 => {
                    if let Some((w, h)) = assets.get(&i.src).and_then(|bytes| {
                        image::ImageReader::new(std::io::Cursor::new(&***bytes))
                            .with_guessed_format()
                            .ok()?
                            .into_dimensions()
                            .ok()
                    }) {
                        i.width = w as f32 * 0.75;
                        i.height = h as f32 * 0.75;
                    }
                }
                xenify_wp_doc::Block::Table(t) => {
                    for row in &mut t.rows {
                        for c in &mut row.cells {
                            go(&mut c.blocks, assets);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let assets = doc.assets.clone();
    for s in &mut doc.sections {
        go(&mut s.blocks, &assets);
    }
}

/// Every kind of writing: the built-in ones, then the person's own.
fn all_kinds() -> Vec<xenify_grammar::Profile> {
    xenify_grammar::Profiles::load(Some(crate::proof::profiles_dir())).all()
}

/// The colour of a table's lines: around and between its cells, or the
/// first cell line (a look with lines between rows only).
fn table_line_color(t: &xenify_wp_doc::Table) -> Option<xenify_wp_doc::Rgba> {
    t.border.map(|b| b.color).or_else(|| {
        t.rows.iter().flat_map(|r| &r.cells).find_map(|c| {
            let b = c.borders.as_ref()?;
            [b.top, b.bottom, b.left, b.right].into_iter().flatten().next().map(|b| b.color)
        })
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Settings, recovery and dictionary in a folder of the test's own.
    fn docs() -> (Docs, tempdir::Dir) {
        let dir = tempdir::Dir::new();
        // SAFETY: the folder holds a lock until the test ends, so no other test
        // changes these while this one reads them.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.0.join("config"));
            std::env::set_var("XDG_DATA_HOME", dir.0.join("data"));
        }
        (Docs::new(&[]), dir)
    }

    mod tempdir {
        /// The settings folders come from the environment, which every test in
        /// the process shares: tests hold this while their folder is in use.
        static IN_USE: std::sync::Mutex<()> = std::sync::Mutex::new(());

        pub struct Dir(pub std::path::PathBuf, #[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
        impl Dir {
            pub fn new() -> Self {
                let turn = IN_USE.lock().unwrap_or_else(|e| e.into_inner());
                let p = std::env::temp_dir().join(format!("xenify-write-app-test-{}-{}", std::process::id(), rand()));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p, turn)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        fn rand() -> u64 {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
        }
    }

    fn text(d: &mut Docs, t: &str) {
        for c in t.chars() {
            d.call("text", &json!({ "text": c.to_string() })).unwrap();
        }
    }

    /// Link (Ctrl+K) reaches the editor (it isn't one of the linked
    /// documents' calls); a page picked with nothing selected goes in by its
    /// name, linked, and what's typed after it isn't.
    #[test]
    fn links_go_on_the_words() {
        let (mut d, _dir) = docs();
        text(&mut d, "See ");
        d.call("link", &json!({ "url": "https://example.org/files/#/wiki/1/2", "text": "Holidays" })).unwrap();
        text(&mut d, " now");
        let ed = &d.open[d.front].session.ed;
        let p = ed.doc.para(ed.sel.focus.para).unwrap();
        assert_eq!(p.text, "See Holidays now");
        assert_eq!(p.fmt_at(6).link.as_deref(), Some("https://example.org/files/#/wiki/1/2"));
        assert_eq!(p.fmt_at(15).link, None, "typing after the link isn't linked");
    }

    /// Copying a picture shown from a file into the document never changes
    /// another picture kept in it under the same file name.
    #[test]
    fn copied_in_pictures_keep_others_with_the_same_name() {
        let (mut d, dir) = docs();
        let png = |c: [u8; 4]| {
            let mut out = Vec::new();
            image::RgbaImage::from_pixel(4, 4, image::Rgba(c))
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        for (sub, c) in [("a", [255, 0, 0, 255]), ("b", [0, 0, 255, 255])] {
            std::fs::create_dir_all(dir.0.join(sub)).unwrap();
            std::fs::write(dir.0.join(sub).join("photo.png"), png(c)).unwrap();
        }
        let a = dir.0.join("a/photo.png").display().to_string();
        let b = dir.0.join("b/photo.png").display().to_string();
        d.call("insert_picture", &json!({ "path": a })).unwrap();
        d.call("insert_picture", &json!({ "path": b })).unwrap();
        let pics = |d: &Docs| crate::remote::every_picture(&d.open[d.front].session.ed.doc);
        let second = pics(&d)[1].0;
        // The second shown from its file, then copied back in.
        d.call("image_link", &json!({ "id": second.0, "path": b })).unwrap();
        d.call("image_link", &json!({ "id": second.0, "embed": true })).unwrap();
        let doc = &d.open[d.front].session.ed.doc;
        let srcs: Vec<String> = pics(&d).into_iter().map(|(_, s)| s).collect();
        assert_ne!(srcs[0], srcs[1], "{srcs:?}");
        assert_eq!(**doc.assets.get(&srcs[0]).unwrap(), png([255, 0, 0, 255]));
        assert_eq!(**doc.assets.get(&srcs[1]).unwrap(), png([0, 0, 255, 255]));
    }

    /// Unsaved work that's only pictures is offered back after a crash (it
    /// isn't empty); an empty document isn't.
    #[test]
    fn picture_only_work_is_offered_back() {
        let (mut d, dir) = docs();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 128, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.0.join("pic.png"), png).unwrap();
        d.call("insert_picture", &json!({ "path": dir.0.join("pic.png").display().to_string() })).unwrap();
        assert_eq!(d.open[d.front].session.ed.doc.word_count(), 0);
        // The browser keeps it.
        assert_eq!(d.call("snapshot", &json!({})).unwrap().as_array().unwrap().len(), 1);
        // Left by a run that ended (a process that isn't running).
        let root = crate::recovery::root().unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let info = |pid: u32| json!({ "path": null, "title": "", "time": 1, "pid": pid }).to_string();
        for (k, doc) in [("untitled-1", d.open[d.front].session.ed.doc.clone()), ("untitled-2", Document::default())] {
            std::fs::write(root.join(format!("{k}.igdoc")), xenify_wp_io::save(&doc, Format::Igdoc).unwrap()).unwrap();
            std::fs::write(root.join(format!("{k}.json")), info(0x7fff_fff0)).unwrap();
        }
        let r = d.call("recovery", &json!({})).unwrap();
        let keys: Vec<&str> = r["orphans"].as_array().unwrap().iter().map(|o| o["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["untitled-1"]);
    }

    /// A Calc book file with what Write links to (`link.json`): a table
    /// whose January total is `jan`, and a chart.
    fn calc_book(path: &std::path::Path, jan: &str) {
        use std::io::Write;
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([194, 65, 12, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        use base64::Engine;
        let link = json!({ "format": "xenify-calc-link", "items": [
            { "id": "sales", "kind": "table", "name": "Sales", "sheet": "Sheet1", "rows": [["Month", "Total"], ["January", jan]], "header": true, "footer": false, "numbers": [false, true] },
            { "id": "chart1", "kind": "chart", "name": "Sales by month", "png": base64::engine::general_purpose::STANDARD.encode(&png) },
        ]});
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        z.start_file("calc.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"{}").unwrap();
        z.start_file("link.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(link.to_string().as_bytes()).unwrap();
        z.finish().unwrap();
    }

    /// Insert › From Calc with a book on this computer: a linked table that
    /// follows the book when it's saved again, and a chart shown from it.
    #[test]
    fn calc_books_on_this_computer_link_and_update() {
        let (mut d, dir) = docs();
        let book = dir.0.join("sales.xcalc");
        calc_book(&book, "100");
        let file = book.display().to_string();
        let snap = d.call("calc_file_snapshot", &json!({ "file": file })).unwrap();
        assert_eq!(snap["items"].as_array().unwrap().len(), 2);
        assert_eq!(snap["name"], "sales");
        let table = &snap["items"][0];
        d.call("calc_link_insert", &json!({ "file": file, "item": "sales", "rows": table["rows"], "header": true, "footer": false, "numbers": table["numbers"] })).unwrap();
        let links = d.call("calc_links", &json!({})).unwrap();
        assert_eq!(links, json!([{ "file": file, "item": "sales" }]));
        let total = |d: &Docs| {
            let doc = &d.open[d.front].session.ed.doc;
            let (id, _) = linked_tables(doc)[0].clone();
            let t = doc.table(id).unwrap();
            xenify_wp_doc::tables::cell_text(&t.rows[1].cells[1])
        };
        assert_eq!(total(&d), "100");
        // Saved again in Calc: the table follows.
        calc_book(&book, "150");
        let snap = d.call("calc_file_snapshot", &json!({ "file": file })).unwrap();
        let table = &snap["items"][0];
        d.call("calc_link_update", &json!({ "file": file, "item": "sales", "rows": table["rows"], "header": true, "footer": false, "numbers": table["numbers"] })).unwrap();
        assert_eq!(total(&d), "150");
        // It keeps its look (a tinted header, lines between rows) through the update.
        {
            let doc = &d.open[d.front].session.ed.doc;
            let t = doc.table(linked_tables(doc)[0].0).unwrap();
            assert!(t.header_fill.is_some() && t.band.is_some() && t.border.is_none());
            assert!(t.rows[1].cells[0].borders.is_some_and(|b| b.bottom.is_some()));
        }
        // The chart: a picture read from the book.
        d.call("calc_file_picture_insert", &json!({ "file": file, "item": "chart1" })).unwrap();
        let shown = d.call("asset", &json!({ "src": format!("{file}#chart1") })).unwrap();
        assert!(shown.as_str().unwrap().starts_with("data:image/png;base64,"));
        // A book never saved with link.json says what to do.
        let old = dir.0.join("old.xcalc");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&old).unwrap());
            z.start_file("calc.json", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"{}").unwrap();
            z.finish().unwrap();
        }
        let e = d.call("calc_file_snapshot", &json!({ "file": old.display().to_string() })).unwrap_err();
        assert!(e.contains("save it once"), "{e}");
    }

    /// A pretend Xenify server for the test below: keeps the order of
    /// changes and says who got in first.
    #[derive(Default)]
    struct Pretend {
        log: Vec<Value>,
        /// Messages waiting for window 0 and window 1.
        to: [Vec<(String, Value)>; 2],
    }

    impl Pretend {
        fn take(&mut self, who: usize, out: &Value) {
            for l in out["live"].as_array().into_iter().flatten() {
                let batch = &l["batch"];
                let base = batch["base"].as_i64().unwrap();
                if base != self.log.len() as i64 {
                    let missed: Vec<Value> = self.log[base as usize..].to_vec();
                    self.to[who].push(("behind".into(), json!({ "client": batch["client"], "ops": missed })));
                    continue;
                }
                let from = self.log.len() as i64 + 1;
                let mut ops = Vec::new();
                for d in batch["ops"].as_array().unwrap() {
                    let op = json!({ "seq": self.log.len() as i64 + 1, "data": d });
                    self.log.push(op.clone());
                    ops.push(op);
                }
                self.to[who].push((
                    "accepted".into(),
                    json!({ "client": batch["client"], "from": from, "to": self.log.len() }),
                ));
                self.to[1 - who].push(("ops".into(), json!({ "ops": ops })));
            }
        }
    }

    /// A file made on the Files page has nothing saved in it: it opens as a
    /// new, blank document, not a copy of the one in front.
    #[test]
    fn an_empty_server_file_opens_blank() {
        let (mut a, _dir) = docs();
        text(&mut a, "Mine");
        let tabs = a.open.len();
        a.call("remote_open", &json!({ "server": "s", "node": 9, "name": "Notes", "data": "", "blank": true }))
            .unwrap();
        assert_eq!(a.open.len(), tabs + 1);
        let doc = &a.open[a.front].session.ed.doc;
        assert!(doc.paragraphs().all(|p| p.text.is_empty()));
        assert_eq!(a.open[a.front].remote.as_ref().unwrap().node, 9);
        assert_eq!(a.open[0].session.ed.doc.paragraphs().next().unwrap().text, "Mine");
    }

    /// Shared "can comment": comments go out, typing is put back.
    #[test]
    fn someone_who_may_comment_only_sends_comments() {
        let (mut a, _dir) = docs();
        text(&mut a, "Plan");
        a.call("remote_open", &json!({ "server": "s", "node": 7, "name": "Plan", "data": "" })).unwrap();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let mut b = Docs::new(&[]);
        let id = b
            .call(
                "remote_open",
                &json!({ "server": "s", "node": 7, "name": "Plan", "data": saved["data"], "version": 1, "user": "Kim Lee" }),
            )
            .unwrap()["id"]
            .clone();
        let start = json!({ "id": id, "seq": 0, "conn": "b", "from": 3000, "to": 4000, "role": "commenter" });
        b.call("live_start", &start).unwrap();
        let out = b.call("text", &json!({ "text": "x" })).unwrap();
        assert_eq!(out["refused"], "You can comment on this document, but not change it.");
        assert!(out.get("live").is_none());
        assert_eq!(b.open[b.front].session.ed.doc.paragraphs().next().unwrap().text, "Plan");
        let out = b.call("add_comment", &json!({ "text": "Which plan?" })).unwrap();
        assert!(out.get("refused").is_none(), "{out}");
        assert_eq!(out["live"][0]["batch"]["ops"].as_array().unwrap().len(), 2);
        let r = b.call("review", &json!({})).unwrap();
        assert_eq!(r["comments"][0]["author"], "Kim Lee");
    }

    /// Two Writes editing one document on a server.
    #[test]
    fn two_writes_edit_one_server_document() {
        let (mut a, _dir) = docs();
        let mut b = Docs::new(&[]);
        text(&mut a, "Plan");
        let ida =
            a.call("remote_open", &json!({ "server": "s", "node": 7, "name": "Plan", "data": "" })).unwrap()["id"]
                .clone();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let idb = b
            .call(
                "remote_open",
                &json!({ "server": "s", "node": 7, "name": "Plan", "data": saved["data"], "version": 1 }),
            )
            .unwrap()["id"]
            .clone();
        assert_eq!(b.call("remote_info", &json!({})).unwrap()["name"], "Plan");
        a.call("live_start", &json!({ "id": ida, "seq": 0, "conn": "a", "from": 1000, "to": 2000 })).unwrap();
        b.call("live_start", &json!({ "id": idb, "seq": 0, "conn": "b", "from": 3000, "to": 4000 })).unwrap();

        let mut srv = Pretend::default();
        // Both type at once; A's gets in first.
        let oa = a.call("text", &json!({ "text": "?" })).unwrap();
        let ob = b.call("text", &json!({ "text": "<" })).unwrap();
        srv.take(0, &oa);
        srv.take(1, &ob);
        let ob = b.call("text", &json!({ "text": "<" })).unwrap();
        srv.take(1, &ob);
        for _ in 0..20 {
            let [qa, qb] = std::mem::take(&mut srv.to);
            if qa.is_empty() && qb.is_empty() {
                break;
            }
            for (kind, mut m) in qa {
                m["id"] = ida.clone();
                let out = a.call(&format!("live_{kind}"), &m).unwrap();
                srv.take(0, &out);
            }
            for (kind, mut m) in qb {
                m["id"] = idb.clone();
                let out = b.call(&format!("live_{kind}"), &m).unwrap();
                srv.take(1, &out);
            }
        }
        let text_of = |d: &Docs| {
            d.open[d.front].session.ed.doc.paragraphs().map(|p| p.text.clone()).collect::<Vec<_>>().join("\n")
        };
        assert_eq!(text_of(&a), text_of(&b));
        assert_eq!(text_of(&a).matches('<').count(), 2, "{}", text_of(&a));
        assert!(text_of(&a).contains("Plan") && text_of(&a).contains('?'), "{}", text_of(&a));
        assert_eq!(srv.log.len(), 3);
    }

    /// A picture added while editing together doesn't travel with the
    /// changes: it goes up in a version, and the other Write takes it from
    /// there (else it shows as an empty box).
    #[test]
    fn pictures_reach_the_others_editing_together() {
        let (mut a, _dir) = docs();
        let mut b = Docs::new(&[]);
        text(&mut a, "Plan");
        let ida =
            a.call("remote_open", &json!({ "server": "s", "node": 7, "name": "Plan", "data": "" })).unwrap()["id"]
                .clone();
        let saved = a.call("remote_save", &json!({})).unwrap();
        a.call("remote_saved", &json!({ "id": ida, "version": 1 })).unwrap();
        let idb = b
            .call(
                "remote_open",
                &json!({ "server": "s", "node": 7, "name": "Plan", "data": saved["data"], "version": 1 }),
            )
            .unwrap()["id"]
            .clone();
        a.call("live_start", &json!({ "id": ida, "seq": 0, "conn": "a", "from": 1000, "to": 2000 })).unwrap();
        b.call("live_start", &json!({ "id": idb, "seq": 0, "conn": "b", "from": 3000, "to": 4000 })).unwrap();
        assert_eq!(a.call("remote_needs", &json!({})).unwrap(), json!({ "upload": [], "missing": [] }));

        // A puts a canvas from the server in; the change reaches B.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([20, 80, 200, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        use base64::Engine;
        let mut srv = Pretend::default();
        let data = base64::engine::general_purpose::STANDARD.encode(&png);
        let out = a.call("link_insert", &json!({ "server": "s", "node": 9, "data": data })).unwrap();
        srv.take(0, &out);
        for _ in 0..20 {
            let [qa, qb] = std::mem::take(&mut srv.to);
            if qa.is_empty() && qb.is_empty() {
                break;
            }
            for (kind, mut m) in qa {
                m["id"] = ida.clone();
                let out = a.call(&format!("live_{kind}"), &m).unwrap();
                srv.take(0, &out);
            }
            for (kind, mut m) in qb {
                m["id"] = idb.clone();
                let out = b.call(&format!("live_{kind}"), &m).unwrap();
                srv.take(1, &out);
            }
        }
        let key = "media/xen-s-9.png";
        assert_eq!(a.call("remote_needs", &json!({})).unwrap(), json!({ "upload": [key], "missing": [] }));
        assert_eq!(b.call("remote_needs", &json!({})).unwrap(), json!({ "upload": [], "missing": [key] }));
        // A puts the picture up by itself (no version).
        let up = a.call("remote_picture", &json!({ "name": key })).unwrap();
        a.call("remote_uploaded", &json!({ "names": [key] })).unwrap();
        assert_eq!(a.call("remote_needs", &json!({})).unwrap()["upload"], json!([]));
        // B fetches it.
        let took = b.call("remote_take_picture", &json!({ "id": idb, "name": key, "data": up["data"] })).unwrap();
        assert_eq!(took["taken"], json!([key]));
        assert_eq!(b.call("remote_needs", &json!({})).unwrap(), json!({ "upload": [], "missing": [] }));
        assert_eq!(b.open[b.front].session.ed.doc.assets[key].as_slice(), png.as_slice());
        // A version saved later holds it too, and someone opening that
        // version can take it from there.
        let v2 = a.call("remote_save", &json!({})).unwrap();
        a.call("remote_saved", &json!({ "id": ida, "version": 2 })).unwrap();
        let doc = xenify_wp_io::load(
            &base64::engine::general_purpose::STANDARD.decode(v2["data"].as_str().unwrap()).unwrap(),
            Format::Igdoc,
        )
        .unwrap()
        .doc;
        assert!(doc.assets.contains_key(key));
    }

    /// Your words on a server and a document's own words both reach the
    /// checker; a document's words are kept in it.
    #[test]
    fn server_and_document_words_reach_the_checker() {
        let (mut a, _dir) = docs();
        a.call("server_words", &json!({ "words": ["Zorblaxian"] })).unwrap();
        a.call("doc_word", &json!({ "word": "Quixlorp" })).unwrap();
        let (_, req) = a.proof_request();
        assert!(req.ignore_words.contains(&"Zorblaxian".to_string()));
        assert!(req.ignore_words.contains(&"Quixlorp".to_string()));
        assert_eq!(a.call("details", &json!({})).unwrap()["words"], json!(["Quixlorp"]));
        a.call("doc_word", &json!({ "word": "Quixlorp", "remove": true })).unwrap();
        a.call("server_words", &json!({ "words": [] })).unwrap();
        assert!(a.proof_request().1.ignore_words.is_empty());
    }

    /// Others reading a document on a server don't have files from this
    /// computer: pictures shown from them go in as copies.
    #[test]
    fn pictures_from_files_go_in_when_a_document_goes_on_a_server() {
        let (mut a, dir) = docs();
        // A Draw canvas: shown from its file, following it.
        use base64::Engine;
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 80, 20, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let file = dir.0.join("sketch.xdraw");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        z.start_file("preview.png", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut z, &png).unwrap();
        z.finish().unwrap();
        text(&mut a, "Plan");
        a.call("insert_picture", &json!({ "path": file.display().to_string() })).unwrap();
        let src = |d: &Docs| crate::remote::every_picture(&d.open[d.front].session.ed.doc)[0].1.clone();
        assert!(!src(&a).starts_with("media/"), "shown from the file");
        let out = a.call("remote_open", &json!({ "server": "s", "node": 7, "name": "Plan", "data": "" })).unwrap();
        assert_eq!(out["copied"], 1);
        assert_eq!(src(&a), "media/sketch.xdraw.png");
        let saved = a.call("remote_save", &json!({})).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(saved["data"].as_str().unwrap()).unwrap();
        let doc = xenify_wp_io::load(&bytes, Format::Igdoc).unwrap().doc;
        assert_eq!(doc.assets["media/sketch.xdraw.png"].as_slice(), png.as_slice());
    }

    #[test]
    fn plan_parts_go_in_one_after_the_other() {
        // Write's From a server › a plan puts in a picture and then a table: they keep that order,
        // also after a line of text.
        let png = {
            let mut out = Vec::new();
            image::RgbaImage::new(40, 20).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, out)
        };
        let (mut d, _dir) = docs();
        text(&mut d, "Status");
        d.call("calc_picture_insert", &json!({ "server": "s1", "node": 9, "item": "gantt", "data": png })).unwrap();
        let rows = json!([["Key date", "Expected"], ["Launch", "Fri 13 Nov"]]);
        d.call("calc_link_insert", &json!({ "server": "s1", "node": 9, "item": "key-dates", "rows": rows, "header": true, "footer": false, "numbers": [false, false] })).unwrap();
        let kinds: Vec<&str> = d.open[0].session.ed.doc.sections[0].blocks.iter().map(|b| match b {
            xenify_wp_doc::Block::Image(_) => "picture",
            xenify_wp_doc::Block::Table(_) => "table",
            _ => "text",
        }).collect();
        let pic = kinds.iter().position(|k| *k == "picture").unwrap();
        let table = kinds.iter().position(|k| *k == "table").unwrap();
        assert!(pic < table, "{kinds:?}");
    }

    #[test]
    fn plan_task_chips_follow_their_task() {
        let (mut d, dir) = docs();
        text(&mut d, "Next up: ");
        d.call("plan_task_insert", &json!({ "server": "s1", "node": 9, "task": "t42", "text": "WEB-6 Checkout pages · To do", "color": "grey" })).unwrap();
        let links = d.call("plan_task_links", &json!({})).unwrap();
        assert_eq!(links, json!([{ "server": "s1", "node": 9, "task": "t42" }]));
        d.call("plan_task_update", &json!({ "server": "s1", "node": 9, "task": "t42", "text": "WEB-6 Checkout pages · Done", "color": "green" })).unwrap();
        let doc = &d.open[0].session.ed.doc;
        let mut seen = None;
        for b in doc.sections[0].blocks.iter() {
            if let xenify_wp_doc::Block::Paragraph(p) = b {
                for r in &p.runs {
                    if let Some(xenify_wp_doc::Field::Badge { text, color }) = &r.fmt.field {
                        seen = Some((text.clone(), color.clone(), r.fmt.link.clone()));
                    }
                }
            }
        }
        assert_eq!(
            seen,
            Some(("WEB-6 Checkout pages · Done".into(), "green".into(), Some("xenify-plan://s1/9/t42".into())))
        );
        // Saved and opened again, it still knows its task.
        let path = dir.0.join("chips.xwrite");
        d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        let mut e = Docs::new(&[]);
        e.call("open", &json!({ "path": path.display().to_string() })).unwrap();
        assert_eq!(e.call("plan_task_links", &json!({})).unwrap(), links);
    }

    #[test]
    fn type_save_and_open_again() {
        let (mut d, dir) = docs();
        text(&mut d, "Hello there");
        let path = dir.0.join("note.md");
        let out = d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        assert_eq!(out["tabs"]["tabs"][0]["dirty"], json!(false));
        assert!(std::fs::read_to_string(&path).unwrap().contains("Hello there"));
        d.call("new", &json!({})).unwrap();
        d.call("open", &json!({ "path": path.display().to_string() })).unwrap();
        // Opening a file that's already open switches to its tab.
        let tabs = d.call("view", &json!({})).unwrap()["tabs"]["tabs"].as_array().unwrap().len();
        assert_eq!(tabs, 2);
    }

    #[test]
    fn a_new_document_takes_the_title_and_details() {
        let (mut d, _dir) = docs();
        let out = d
            .call(
                "new",
                &json!({ "template": "blank", "title": "Why rivers flood", "reader": "Teacher", "tone": "Formal" }),
            )
            .unwrap();
        assert_eq!(out["details"]["profile_name"], json!("Standard"));
        assert_eq!(out["details"]["reader"], json!("Teacher"));
        // The blank untouched first tab was replaced.
        assert_eq!(out["tabs"]["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(out["tabs"]["tabs"][0]["title"], json!("Why rivers flood"));
    }

    #[test]
    fn your_own_templates() {
        let (mut d, _dir) = docs();
        text(&mut d, "Week ending: ");
        d.call("save_template", &json!({ "name": "Weekly report" })).unwrap();
        let all = d.call("templates", &json!({})).unwrap();
        let mine: Vec<&Value> = all.as_array().unwrap().iter().filter(|t| t["mine"] == json!(true)).collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0]["id"], json!("mine:Weekly report"));
        let out = d.call("new", &json!({ "template": "mine:Weekly report", "title": "Week 3" })).unwrap();
        assert_eq!(out["tabs"]["tabs"].as_array().unwrap().len(), 2);
        assert!(d.doc().session.ed.doc.paragraphs().any(|p| p.text.starts_with("Week ending")));
        d.call("delete_template", &json!({ "id": "mine:Weekly report" })).unwrap();
        let all = d.call("templates", &json!({})).unwrap();
        assert!(!all.as_array().unwrap().iter().any(|t| t["mine"] == json!(true)));
    }

    #[test]
    fn documents_follow_their_template() {
        let (mut d, _dir) = docs();
        text(&mut d, "Name: {{Your name}}");
        d.call("set_page", &json!({ "page": { "footer": { "right": "{page}" } } })).unwrap();
        d.call("save_template", &json!({ "name": "Tech doc" })).unwrap();
        // A document from it: linked, with the fill-in box made.
        d.call("new", &json!({ "template": "mine:Tech doc" })).unwrap();
        assert_eq!(d.doc().session.ed.doc.meta.template.as_ref().map(|t| t.name.as_str()), Some("Tech doc"));
        assert!(!d.doc().session.ed.doc.meta.controls.is_empty());
        assert_eq!(d.call("template_status", &json!({})).unwrap()["out_of_date"], json!(false));
        // Its styles can't be changed from the document.
        assert!(d.call("style_from_selection", &json!({ "id": "normal" })).is_err());
        // The template changes (edited and saved): the document can catch up.
        d.call("template_edit", &json!({ "name": "Tech doc" })).unwrap();
        d.call("set_page", &json!({ "page": { "footer": { "left": "{page}", "right": "" } } })).unwrap();
        let path = templates_dir().join("Tech doc.xwrite");
        d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        let back = d.open.iter().position(|o| o.session.ed.doc.meta.template.is_some()).unwrap();
        d.front = back;
        assert_eq!(d.call("template_status", &json!({})).unwrap()["out_of_date"], json!(true));
        d.call("template_update", &json!({})).unwrap();
        assert_eq!(d.doc().session.ed.doc.sections[0].page.footer.left, "{page}");
        assert_eq!(d.call("template_status", &json!({})).unwrap()["out_of_date"], json!(false));
    }

    #[test]
    fn a_written_document_takes_a_template_and_follows_it() {
        let (mut d, _dir) = docs();
        // The template: a logo, a letterhead in the header, a footer, where the text goes.
        d.call("set_page", &json!({ "page": { "header": { "left": "Riverside" }, "footer": { "center": "riverside.example" } } })).unwrap();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([194, 65, 12, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        use base64::Engine;
        let logo = base64::engine::general_purpose::STANDARD.encode(&png);
        d.call("insert_picture_bytes", &json!({ "name": "logo.png", "data": logo })).unwrap();
        d.call("key", &json!({ "key": "Enter" })).unwrap();
        text(&mut d, "Body");
        d.call("template_mark_body", &json!({})).unwrap();
        d.call("save_template", &json!({ "name": "Letterhead" })).unwrap();
        // A document already written, without a template.
        d.call("new", &json!({})).unwrap();
        text(&mut d, "Quote for Harbour Foods");
        let out = d.call("template_use", &json!({ "name": "Letterhead" })).unwrap();
        assert_eq!(out["template"], json!("Letterhead"));
        let doc = d.doc().session.ed.doc.clone();
        assert_eq!(doc.meta.template.as_ref().map(|t| t.name.as_str()), Some("Letterhead"));
        assert_eq!(doc.sections[0].page.header.left, "Riverside");
        assert_eq!(doc.sections[0].page.footer.center, "riverside.example");
        let texts: Vec<_> = doc.paragraphs().map(|p| p.text.clone()).filter(|t| !t.is_empty()).collect();
        assert_eq!(texts, ["Quote for Harbour Foods"], "the document keeps its own text");
        let used = crate::remote::pictures(&doc);
        assert!(!used.is_empty() && used.iter().all(|k| doc.assets.contains_key(k)), "the logo comes with it: {used:?}");
        // The letterhead changes: the document follows when it's next shown.
        let mine = d.front;
        d.call("template_edit", &json!({ "name": "Letterhead" })).unwrap();
        d.call("set_page", &json!({ "page": { "header": { "left": "Riverside Studio" } } })).unwrap();
        let path = templates_dir().join("Letterhead.xwrite");
        d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        d.front = mine;
        assert_eq!(d.call("template_refresh", &json!({})).unwrap()["changed"], json!(true));
        let doc = d.doc().session.ed.doc.clone();
        assert_eq!(doc.sections[0].page.header.left, "Riverside Studio");
        let texts: Vec<_> = doc.paragraphs().map(|p| p.text.clone()).filter(|t| !t.is_empty()).collect();
        assert_eq!(texts, ["Quote for Harbour Foods"]);
        // A template can't be put in a template.
        d.call("template_edit", &json!({ "name": "Letterhead" })).unwrap();
        assert!(d.call("template_use", &json!({ "name": "Letterhead" })).is_err());
    }

    #[test]
    fn find_replace_and_undo() {
        let (mut d, _dir) = docs();
        text(&mut d, "cat and cat and dog");
        let out = d.call("find", &json!({ "query": "cat" })).unwrap();
        assert_eq!(out["find"]["count"], json!(2));
        let out = d.call("replace_all", &json!({ "with": "fox" })).unwrap();
        assert_eq!(out["replaced"], json!(2));
        assert_eq!(out["find"]["count"], json!(0));
        d.call("undo", &json!({})).unwrap();
        let out = d.call("find", &json!({ "query": "cat", "stay": true })).unwrap();
        assert_eq!(out["find"]["count"], json!(2));
    }

    #[test]
    fn spelling_issues_come_and_fix() {
        let (mut d, _dir) = docs();
        text(&mut d, "This is a mispeled word.");
        let proof = crate::proof::Proof::new();
        let (id, req) = d.proof_request();
        let paras = req.paras.clone();
        let issues = proof.check(req);
        d.set_issues(id, &paras, issues);
        let list = d.call("issues", &json!({})).unwrap();
        let spelling =
            list.as_array().unwrap().iter().find(|i| i["kind"] == "spelling").expect("a spelling mistake").clone();
        assert_eq!(spelling["text"], json!("mispeled"));
        let fix = spelling["suggestions"][0].as_str().unwrap().to_string();
        d.call("fix", &json!({ "i": spelling["i"], "with": fix })).unwrap();
        // The paragraph changed, so the old issue isn't shown any more.
        let list = d.call("issues", &json!({})).unwrap();
        assert!(list.as_array().unwrap().iter().all(|i| i["text"] != "mispeled"));
    }

    #[test]
    fn template_starter_parts_make_a_ready_template() {
        let (mut d, dir) = docs();
        let ed = |d: &Docs| d.open[d.front].session.ed.doc.clone();
        // None ticked: only marked as a template.
        d.call("template_starter", &json!({ "parts": [] })).unwrap();
        let doc = ed(&d);
        assert!(doc.meta.is_template && doc.meta.body_slot.is_none());
        assert_eq!(doc.paragraphs().count(), 1);
        d.call("undo", &json!({})).unwrap();
        assert!(!ed(&d).meta.is_template);
        // Everything: a title page section, contents, numbered headings, headers.
        d.call("template_starter", &json!({ "parts": ["title", "contents", "numbered", "headers"] })).unwrap();
        let doc = ed(&d);
        assert!(doc.meta.is_template);
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].page.v_align, xenify_wp_doc::PageVAlign::Center);
        assert!(doc.sections[0].page.footer.is_empty());
        assert_eq!(doc.sections[1].page.footer.center, "Page {page} of {pages}");
        assert!(doc.sections[1].blocks.iter().any(|b| matches!(b, xenify_wp_doc::Block::Toc(_))));
        let texts: Vec<_> = doc.paragraphs().map(|p| p.text.clone()).collect();
        assert_eq!(texts, ["{{Title}}", "{{Subtitle}}", "{{Author}}", "{{Date}}", "Contents", ""]);
        let body = doc.meta.body_slot.expect("where the text goes");
        assert_eq!(doc.para(body).unwrap().props.page_break_before, Some(true));
        assert!(doc.styles.get(&xenify_wp_doc::StyleSheet::heading(1)).unwrap().list.is_some());
        // One undo takes it all away.
        d.call("undo", &json!({})).unwrap();
        let doc = ed(&d);
        assert!(!doc.meta.is_template && doc.sections.len() == 1 && doc.paragraphs().count() == 1);
        // Redone and saved, then opened: a document from it, its title page fields to fill in.
        d.call("redo", &json!({})).unwrap();
        let path = dir.0.join("Report.xwrite");
        d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        d.open.retain(|o| o.path.as_ref() != Some(&path));
        d.add(Document::default(), None);
        d.front = 0;
        let out = d.call("open", &json!({ "path": path.display().to_string() })).unwrap();
        assert_eq!(out["made_from_template"]["name"], "Report");
        let doc = ed(&d);
        assert!(doc.paragraphs().all(|p| !p.text.contains("{{")));
        assert_eq!(doc.meta.controls.len(), 4);
    }

    #[test]
    fn templates_keep_their_parts_and_fields_follow_changes() {
        let (mut d, dir) = docs();
        let paras = |d: &Docs| d.open[d.front].session.ed.doc.paragraphs().map(|p| p.text.clone()).collect::<Vec<_>>();
        // The template: a heading with a field, where the text goes, a closing line.
        text(&mut d, "Quote for {{Client}}");
        d.call("key", &json!({ "key": "Enter" })).unwrap();
        d.call("key", &json!({ "key": "Enter" })).unwrap();
        text(&mut d, "Valid for 30 days");
        d.call("key", &json!({ "key": "ArrowUp" })).unwrap();
        d.call("template_mark_body", &json!({})).unwrap();
        let info = d.call("template_info", &json!({})).unwrap();
        assert_eq!(info["is_template"], true);
        let path = dir.0.join("Quote.xwrite");
        d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        // Closed, then opened: a new document from it.
        d.open.retain(|o| o.path.as_ref() != Some(&path));
        d.add(Document::default(), None);
        d.front = 0;
        let out = d.call("open", &json!({ "path": path.display().to_string() })).unwrap();
        assert_eq!(out["made_from_template"]["name"], "Quote");
        assert_eq!(paras(&d), ["Quote for Client", "", "Valid for 30 days"]);
        assert_eq!(d.call("template_info", &json!({})).unwrap()["fixed"], true);
        // The body is the document's.
        d.call("key", &json!({ "key": "ArrowDown" })).unwrap();
        text(&mut d, "Two chairs");
        // The field: typing over its prompt.
        d.call("key", &json!({ "key": "ArrowUp" })).unwrap();
        d.call("key", &json!({ "key": "End" })).unwrap();
        d.call("key", &json!({ "key": "ArrowLeft" })).unwrap();
        text(&mut d, "Ada");
        assert_eq!(paras(&d)[..2], ["Quote for Ada".to_string(), "Two chairs".to_string()]);
        // The template's own words: put back, with why.
        d.call("key", &json!({ "key": "Home" })).unwrap();
        let out = d.call("text", &json!({ "text": "X" })).unwrap();
        assert!(out["refused"].as_str().unwrap().contains("template “Quote”"), "{}", out["refused"]);
        assert_eq!(paras(&d)[..2], ["Quote for Ada".to_string(), "Two chairs".to_string()]);
        // Undo still undoes the typing before.
        // The template changes: the document takes it when refreshed (as on opening).
        d.call("template_edit", &json!({ "path": path.display().to_string() })).unwrap();
        d.call("key", &json!({ "key": "End", "mods": { "command": true } })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        d.call("key", &json!({ "key": "Backspace" })).unwrap();
        text(&mut d, "60 days");
        assert_eq!(paras(&d).last().unwrap(), "Valid for 60 days");
        d.call("save", &json!({ "path": path.display().to_string() })).unwrap();
        let mine = d.open.iter().position(|o| o.path.is_none()).unwrap();
        d.front = mine;
        let out = d.call("template_refresh", &json!({})).unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(paras(&d), ["Quote for Ada", "Two chairs", "Valid for 60 days"]);
        // Printed or exported: an unfilled field is empty; Detach makes it the document's own.
        d.call("template_unlink", &json!({})).unwrap();
        d.call("key", &json!({ "key": "Home", "mods": { "command": true } })).unwrap();
        let out = d.call("text", &json!({ "text": "A " })).unwrap();
        assert!(out.get("refused").is_none());
        assert_eq!(paras(&d)[0], "A Quote for Ada");
    }

    #[test]
    fn comments_and_tracked_changes() {
        let (mut d, _dir) = docs();
        text(&mut d, "one two");
        d.call("key", &json!({ "key": "Home" })).unwrap();
        d.call("key", &json!({ "key": "ArrowRight", "mods": { "shift": true, "command": true } })).unwrap();
        d.call("add_comment", &json!({ "text": "really?" })).unwrap();
        d.call("set_tracking", &json!({ "on": true })).unwrap();
        d.call("key", &json!({ "key": "End" })).unwrap();
        text(&mut d, " three");
        let r = d.call("review", &json!({})).unwrap();
        // Ctrl+Shift+Right takes the word and its space, as on Windows.
        assert_eq!(r["comments"][0]["quote"], json!("one "));
        assert_eq!(r["changes"][0]["kind"], json!("inserted"));
        let out = d.call("decide", &json!({ "accept": true, "all": true })).unwrap();
        assert_eq!(out["decided"], json!(1));
    }

    #[test]
    fn import_cite_and_list_sources() {
        let (mut d, _dir) = docs();
        text(&mut d, "As shown before ");
        let out = d
            .call("import_sources", &json!({ "text": "@book{smith2020, author = {Smith, Jane}, title = {Writing Well}, publisher = {Harbour Press}, year = {2020}}" }))
            .unwrap();
        assert_eq!(out["added"], json!(1));
        let src = d.call("sources", &json!({})).unwrap();
        let key = src["sources"][0]["source"]["key"].as_str().unwrap().to_string();
        assert!(src["sources"][0]["entry"].as_str().unwrap().contains("Writing"));
        d.call("cite", &json!({ "keys": [key], "locator": "p. 4" })).unwrap();
        d.call("bibliography", &json!({})).unwrap();
        let src = d.call("sources", &json!({})).unwrap();
        assert_eq!(src["sources"][0]["cited"], json!(true));
        d.call("cite_style", &json!({ "style": "ieee" })).unwrap();
        assert_eq!(d.call("sources", &json!({})).unwrap()["style"], json!("ieee"));
    }

    #[test]
    fn mail_merge_previews_and_writes_one_file_per_row() {
        let (mut d, dir) = docs();
        text(&mut d, "Dear {{name}} in {{city}}");
        let csv = dir.0.join("people.csv");
        std::fs::write(&csv, "name,city\nPriya Shah,Leeds\nSam Lee,York\n").unwrap();
        let info = d.call("merge_load", &json!({ "path": csv.display().to_string() })).unwrap();
        assert_eq!(info["records"], json!(2));
        assert_eq!(info["used"], json!(["name", "city"]));
        let out = d.call("merge_preview", &json!({ "n": 2 })).unwrap();
        assert_eq!(out["tabs"]["tabs"].as_array().unwrap().len(), 2);
        assert!(out["view"]["para_text"].as_str().unwrap().contains("Sam Lee in York"));
        // Back to the letter itself, then make the copies.
        let first = out["tabs"]["tabs"][0]["id"].clone();
        d.call("switch", &json!({ "id": first })).unwrap();
        let outdir = dir.0.join("out");
        std::fs::create_dir_all(&outdir).unwrap();
        let out = d
            .call("merge_run", &json!({ "dir": outdir.display().to_string(), "ext": "md", "name": "{{name}}" }))
            .unwrap();
        assert_eq!(out["written"], json!(2));
        let priya = std::fs::read_to_string(outdir.join("Priya Shah.md")).unwrap();
        assert!(priya.contains("Dear Priya Shah in Leeds"), "{priya}");
    }

    #[test]
    fn a_copied_kind_of_writing_saves() {
        let (mut d, _dir) = docs();
        let kinds = d.call("kinds", &json!({})).unwrap();
        let mut k = kinds["kinds"].as_array().unwrap().iter().find(|k| k["id"] == "school-essay").unwrap().clone();
        k["id"] = json!("");
        k["name"] = json!("School essay (my copy)");
        k["built_in"] = json!(false);
        k["features"]["slang"] = json!("allow");
        let out = d.call("save_kind", &json!({ "kind": k }));
        assert!(out.is_ok(), "{out:?}");
        let kinds = d.call("kinds", &json!({})).unwrap();
        let mine =
            kinds["kinds"].as_array().unwrap().iter().find(|k| k["name"] == "School essay (my copy)").unwrap().clone();
        assert_eq!(mine["features"]["slang"], json!("allow"));
        assert_eq!(mine["built_in"], json!(false));
    }
}
