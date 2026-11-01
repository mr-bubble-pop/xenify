//! Write documents as web pages, read on the server itself: the Files
//! page's read view and pages opened with a public link work on every
//! server, whether or not it offers the apps in the browser. Also the words
//! of Write documents, for search.
//!
//! People editing together send changes, not versions: a page (and its
//! words) is the latest saved version with the changes since replayed on
//! it, so what's read and found is what's in the document now.

use crate::Server;
use crate::files::{Me, node_role};
use deadpool_postgres::GenericClient;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use xenify_server_proto::Role;
use xenify_wp_doc::collab::Change;
use xenify_wp_doc::{Block, Document};
use xenify_wp_io::Format;

/// The fonts Write documents use by default, for pages read in a browser
/// (they're in this program already, for laying documents out): Inter and
/// CMU Serif's four faces. Open Font License, see assets/SOURCES.toml.
pub fn routes() -> axum::Router<Server> {
    axum::Router::new().route("/v1/fonts/{name}", axum::routing::get(font))
}

async fn font(axum::extract::Path(name): axum::extract::Path<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let [rm, ti, bx, bi] = xenify_wp_layout::SERIF_FONTS;
    let (bytes, kind): (&'static [u8], &str) = match name.as_str() {
        "inter.ttf" => (xenify_wp_layout::INTER_FONT, "font/ttf"),
        "cmu-serif.otf" => (rm, "font/otf"),
        "cmu-serif-italic.otf" => (ti, "font/otf"),
        "cmu-serif-bold.otf" => (bx, "font/otf"),
        "cmu-serif-bold-italic.otf" => (bi, "font/otf"),
        _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    ([(axum::http::header::CONTENT_TYPE, kind), (axum::http::header::CACHE_CONTROL, "public, max-age=604800")], bytes)
        .into_response()
}

/// Whether a file's details (`meta`) say it's a Write document.
pub(crate) fn is_write(meta: &str) -> bool {
    let d: Value = serde_json::from_str(meta).unwrap_or_default();
    let ext = d["ext"].as_str().unwrap_or("");
    d["app"] == "write"
        || matches!(ext, "xwrite" | "igdoc")
        || d["name"].as_str().is_some_and(|n| n.ends_with(".xwrite") || n.ends_with(".igdoc"))
}

/// Tables and charts from Calc books on this server, as the books are now
/// (what Calc put up when they were last saved), for someone who may see
/// the book; the document keeps what was shown when Write last had it open.
async fn calc_now(s: &Server, c: &impl GenericClient, me: &Me, doc: Document) -> Document {
    let here = here(c).await;
    // Tables: their link is `server|node|item`.
    let mut tables: Vec<(xenify_wp_doc::NodeId, i64, String)> = Vec::new();
    fn walk(blocks: &[Block], out: &mut Vec<(xenify_wp_doc::NodeId, String)>) {
        for b in blocks {
            if let Block::Table(t) = b {
                if let Some(l) = &t.linked {
                    out.push((t.id, l.clone()));
                }
                t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.blocks, out));
            }
        }
    }
    let mut linked = Vec::new();
    for sec in &doc.sections {
        walk(&sec.blocks, &mut linked);
        sec.frames.iter().for_each(|f| walk(&f.blocks, &mut linked));
    }
    for (id, l) in linked {
        let mut parts = l.splitn(3, '|');
        if let (Some(server), Some(Ok(node)), Some(item)) =
            (parts.next(), parts.next().map(str::parse::<i64>), parts.next())
            && server == here
        {
            tables.push((id, node, item.to_string()));
        }
    }
    // Charts: pictures kept as `media/xenc-<server>-<node>-<item>.png`.
    let mut charts: Vec<(String, i64, String)> = Vec::new();
    for key in pictures(&doc) {
        let Some(rest) = key.strip_prefix("media/xenc-").and_then(|r| r.strip_suffix(".png")) else { continue };
        let mut parts = rest.splitn(3, '-');
        if let (Some(server), Some(Ok(node)), Some(item)) =
            (parts.next(), parts.next().map(str::parse::<i64>), parts.next())
            && server == here
        {
            charts.push((key.clone(), node, item.to_string()));
        }
    }
    let mut nodes: Vec<i64> = tables.iter().map(|t| t.1).chain(charts.iter().map(|c| c.1)).collect();
    nodes.sort();
    nodes.dedup();
    let mut ed = xenify_wp_doc::Editor::new(doc);
    for node in nodes {
        if !matches!(node_role(c, me, node).await, Ok(Some((_, Some(r)))) if r >= Role::Viewer) {
            continue;
        }
        let Ok(Some(row)) = c.query_opt("SELECT preview FROM nodes WHERE id = $1 AND trashed IS NULL", &[&node]).await
        else {
            continue;
        };
        let Some(hash) = row.get::<_, Option<String>>(0) else { continue };
        let Ok(bytes) = s.blobs.get(&hash).await else { continue };
        let Ok(snap) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let Some(items) = snap["items"].as_array().filter(|_| snap["format"] == "xenify-calc-link") else { continue };
        let clean = |x: &str| x.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>();
        for (id, _, item) in tables.iter().filter(|t| t.1 == node) {
            let Some(it) = items.iter().find(|x| x["id"] == item.as_str()) else { continue };
            let rows: Vec<Vec<String>> = serde_json::from_value(it["rows"].clone()).unwrap_or_default();
            let numbers: Vec<bool> = serde_json::from_value(it["numbers"].clone()).unwrap_or_default();
            let (header, footer) = (it["header"].as_bool().unwrap_or(false), it["footer"].as_bool().unwrap_or(false));
            if !rows.is_empty() {
                let _ = xenify_wp_doc::tables::fill_table(&mut ed, *id, &rows, header, footer, &numbers);
            }
        }
        for (key, _, item) in charts.iter().filter(|c| c.1 == node) {
            use base64::Engine;
            let png = items
                .iter()
                .find(|x| x["kind"] == "chart" && x["id"].as_str().is_some_and(|i| clean(i) == *item))
                .and_then(|x| x["png"].as_str())
                .and_then(|p| base64::engine::general_purpose::STANDARD.decode(p.trim()).ok());
            if let Some(png) = png {
                ed.doc.assets.insert(key.clone(), Arc::new(png));
            }
        }
    }
    ed.doc
}

/// This server's id as documents keep it in their links.
async fn here(c: &impl GenericClient) -> String {
    crate::db::setting(c, "server_id")
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// The pictures a document keeps inside it (`media/…`), wherever they are.
fn pictures(doc: &Document) -> Vec<String> {
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Image(i) if i.src.starts_with("media/") => out.push(i.src.clone()),
                Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).for_each(|c| walk(&c.blocks, out)),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        walk(&s.blocks, &mut out);
        s.frames.iter().for_each(|f| walk(&f.blocks, &mut out));
        s.notes.iter().for_each(|n| walk(&n.blocks, &mut out));
        s.headers.iter().for_each(|h| walk(&h.blocks, &mut out));
    }
    out
}

fn words_of(doc: &Document) -> String {
    format!("{}\n{}", doc.meta.title, doc.plain_text())
}

/// Write document `node` as it is now: its latest version with the changes
/// made together since. None when it isn't a Write document, or nothing has
/// been saved in it yet.
pub(crate) async fn latest(s: &Server, c: &impl GenericClient, node: i64) -> Result<Option<Document>, String> {
    let row = c
        .query_opt(
            "SELECT v.blob, v.upto, n.meta FROM versions v JOIN nodes n ON n.id = v.node WHERE v.node = $1 AND v.n = n.head",
            &[&node],
        )
        .await
        .map_err(|e| e.to_string())?;
    let Some(row) = row else { return Ok(None) };
    if !is_write(row.get(2)) {
        return Ok(None);
    }
    let bytes = s.blobs.get(row.get(0)).await.map_err(|e| e.to_string())?;
    let upto: i64 = row.get(1);
    // Pictures added since (they don't travel with the changes).
    let mut added = Vec::new();
    for r in c
        .query("SELECT name, blob FROM node_pictures WHERE node = $1", &[&node])
        .await
        .map_err(|e| e.to_string())?
    {
        if let Ok(b) = s.blobs.get(r.get(1)).await {
            added.push((r.get::<_, String>(0), b));
        }
    }
    let ops: Vec<String> = c
        .query("SELECT data FROM ops WHERE node = $1 AND seq > $2 ORDER BY seq", &[&node, &upto])
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|r| r.get(0))
        .collect();
    tokio::task::spawn_blocking(move || {
        let mut doc = xenify_wp_io::load(&bytes, Format::Igdoc).map_err(|e| e.to_string())?.doc;
        for (name, b) in added {
            doc.assets.entry(name).or_insert_with(|| Arc::new(b));
        }
        for op in ops {
            // As the apps do when they catch up: a change that no longer
            // fits is skipped.
            if let Ok(change) = serde_json::from_str::<Change>(&op) {
                let _ = change.step.apply(&mut doc);
            }
        }
        Ok(Some(doc))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Someone who may only comment: what's wrong with `ops` (Write changes
/// made on the document as it is now), if anything. `who`: the names they
/// go by (a comment's author must be one of them).
pub(crate) async fn comments_refused(
    s: &Server,
    c: &impl GenericClient,
    node: i64,
    ops: Vec<String>,
    who: Vec<String>,
) -> Option<&'static str> {
    use xenify_wp_doc::review::{NotAComment, comment_only};
    const CONTENT: &str = "you may only comment on this document";
    const NOT_YOURS: &str = "you can only change or delete your own comments";
    let Ok(Some(mut doc)) = latest(s, c, node).await else { return Some(CONTENT) };
    tokio::task::spawn_blocking(move || {
        let who: Vec<&str> = who.iter().map(String::as_str).collect();
        for op in ops {
            let Ok(change) = serde_json::from_str::<Change>(&op) else { return Some(CONTENT) };
            match comment_only(&doc, &change.step, &who) {
                Err(NotAComment::Content) => return Some(CONTENT),
                Err(NotAComment::NotYours) => return Some(NOT_YOURS),
                Ok(()) => {}
            }
            if change.step.apply(&mut doc).is_err() {
                return Some(CONTENT);
            }
        }
        None
    })
    .await
    .unwrap_or(Some(CONTENT))
}

/// A Write document's title and words (for search), or None when the bytes
/// aren't one.
pub(crate) async fn words(bytes: Vec<u8>) -> Option<String> {
    tokio::task::spawn_blocking(move || Some(words_of(&xenify_wp_io::load(&bytes, Format::Igdoc).ok()?.doc)))
        .await
        .ok()
        .flatten()
}

/// Documents whose words are due to be read again (changes came in).
/// Files waiting to be read again, by server (its data folder: tests run
/// several servers in one program, whose file numbers overlap) and file.
static DUE: Mutex<Option<HashSet<(std::path::PathBuf, i64)>>> = Mutex::new(None);

/// Changes to `node` came in while people edit it together: its words for
/// search are read again a little later (once for a burst of typing).
pub(crate) fn changed(s: &Server, node: i64) {
    {
        let mut due = DUE.lock().unwrap_or_else(|e| e.into_inner());
        if !due.get_or_insert_with(HashSet::new).insert((s.data.clone(), node)) {
            return;
        }
    }
    let s = s.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        if let Some(due) = DUE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            due.remove(&(s.data.clone(), node));
        }
        let Ok(c) = s.pool.get().await else { return };
        if let Ok(Some(doc)) = latest(&s, &c, node).await {
            let _ = c.execute("UPDATE nodes SET search_text = $2 WHERE id = $1", &[&node, &words_of(&doc)]).await;
        }
    });
}

/// Document `node` as a page, `{ html, title }`. `me`: someone signed in,
/// whose pictures of Draw canvases on this server are swapped for the
/// canvas's current picture when they may see it (the copy in the document
/// is from when someone last had it open in Write). Public links keep the
/// copy. Err(None): not a Write document, or nothing saved yet.
pub(crate) async fn page(
    s: &Server,
    c: &impl GenericClient,
    me: Option<&Me>,
    node: i64,
) -> Result<Value, Option<String>> {
    page_at(s, c, me, node, None).await
}

/// Write document `node` as it was in version `n` (the latest one, with the
/// changes made together since, when `n` is its newest).
pub(crate) async fn at_version(s: &Server, c: &impl GenericClient, node: i64, n: i64) -> Result<Option<Document>, String> {
    let row = c
        .query_opt(
            "SELECT v.blob, n.meta, n.head FROM versions v JOIN nodes n ON n.id = v.node WHERE v.node = $1 AND v.n = $2",
            &[&node, &n],
        )
        .await
        .map_err(|e| e.to_string())?;
    let Some(row) = row else { return Ok(None) };
    if !is_write(row.get(1)) {
        return Ok(None);
    }
    if row.get::<_, i64>(2) == n {
        return latest(s, c, node).await;
    }
    let bytes = s.blobs.get(row.get(0)).await.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || xenify_wp_io::load(&bytes, Format::Igdoc).map(|l| Some(l.doc)).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// A Write document's paragraphs as plain text (comparing versions).
pub(crate) fn paragraphs(doc: &Document) -> Vec<String> {
    doc.plain_text().lines().map(|l| l.trim_end().to_string()).filter(|l| !l.trim().is_empty()).collect()
}

/// [`page`], or (`version`) the page as it was in that version.
pub(crate) async fn page_at(
    s: &Server,
    c: &impl GenericClient,
    me: Option<&Me>,
    node: i64,
    version: Option<i64>,
) -> Result<Value, Option<String>> {
    let mut doc = match version {
        None => latest(s, c, node).await.map_err(Some)?.ok_or(None)?,
        Some(n) => at_version(s, c, node, n).await.map_err(Some)?.ok_or(None)?,
    };
    if let Some(me) = me {
        let here = here(c).await;
        let linked: Vec<(String, i64)> = pictures(&doc)
            .iter()
            .filter_map(|k| {
                let rest = k.strip_prefix("media/xen-")?.strip_suffix(".png")?;
                let (server, node) = rest.rsplit_once('-')?;
                if server != here {
                    return None;
                }
                Some((k.clone(), node.parse().ok()?))
            })
            .collect();
        for (key, node) in linked {
            if !matches!(node_role(c, me, node).await, Ok(Some((_, Some(r)))) if r >= Role::Viewer) {
                continue;
            }
            let Ok(Some(row)) =
                c.query_opt("SELECT preview FROM nodes WHERE id = $1 AND trashed IS NULL", &[&node]).await
            else {
                continue;
            };
            let Some(hash) = row.get::<_, Option<String>>(0) else { continue };
            if let Ok(png) = s.blobs.get(&hash).await
                && png.starts_with(b"\x89PNG")
            {
                doc.assets.insert(key, Arc::new(png));
            }
        }
    }
    if let Some(me) = me {
        doc = calc_now(s, c, me, doc).await;
    }
    tokio::task::spawn_blocking(move || json!({ "html": xenify_wp_io::html::write(&xenify_wp_doc::controls::without_prompts(&doc)), "title": doc.meta.title }))
        .await
        .map_err(|e| Some(e.to_string()))
}
