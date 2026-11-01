//! A document kept on a Xenify server: where it is, and editing it
//! together.
//!
//! The screens move the bytes (fetch, WebSocket); everything that reads or
//! changes them happens here. Editing together keeps two copies: the one on
//! screen, with my changes the server hasn't taken yet, and the server's
//! (every change in the server's order). Once all my changes are in, the
//! two must be the same; if adjusting someone's change ever went wrong, the
//! screen takes the server's copy.

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use xenify_collab::{Batch, Client};
use xenify_wp_doc::collab::{Change, same_content};
use xenify_wp_doc::{Document, Editor};

pub struct Remote {
    pub server: String,
    pub node: i64,
    /// The file's name on the server (without .xwrite).
    pub name: String,
    /// The saved version this copy started from, and the changes it holds.
    pub version: i64,
    pub upto: i64,
    pub live: Option<Live>,
    /// The pictures (`media/…`) the server's latest version holds. Changes
    /// sent while editing together don't carry pictures, so a picture
    /// added here goes up in a version of its own ([`Remote::needs`]).
    pub on_server: HashSet<String>,
    /// The pictures in the version being saved now.
    pub saving: HashSet<String>,
    /// What this person may do here (their share of the file).
    pub access: Access,
    /// May only read or comment: the document after the last change that
    /// went out or came in, to go back to when a change isn't allowed.
    pub last: Option<Document>,
    /// Why the last change was put back, to show once.
    pub refused: Option<String>,
}

/// What someone may do with a document on a server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Edit,
    /// Add comments, reply and resolve; change or delete only their own.
    Comment,
    Read,
}

impl Access {
    /// From the server's role ("viewer", "commenter", "editor", "manager").
    pub fn of(role: &str) -> Access {
        match role {
            "viewer" => Access::Read,
            "commenter" => Access::Comment,
            _ => Access::Edit,
        }
    }
}

pub struct Live {
    client: Client<Change>,
    /// The server's copy, and changes waiting to go into it in order.
    shadow: Document,
    /// The server's changes in the server's copy.
    shadow_seq: i64,
    queue: BTreeMap<i64, Change>,
    /// Batches sent, as the server will keep them.
    sent: HashMap<String, Vec<Change>>,
    /// Ids for new paragraphs come from here (leased from the server).
    pub ids: (u64, u64),
}

impl Live {
    /// Everything made here has reached the server.
    pub fn settled(&self) -> bool {
        self.client.settled()
    }
}

impl Remote {
    fn encode(&self, c: &Change) -> String {
        serde_json::to_string(c).expect("a change serialises")
    }

    fn decode(&self, data: &str) -> Result<Change, String> {
        serde_json::from_str(data).map_err(|e| e.to_string())
    }

    /// Starts editing together: `seq` is the server's latest change, and
    /// `ops` the ones after this copy's version (to catch up on).
    pub fn start_live(
        &mut self,
        ed: &mut Editor,
        seq: i64,
        ops: &[Value],
        ids: (u64, u64),
        name: &str,
    ) -> Result<(), String> {
        let mut shadow = ed.doc.clone();
        for op in ops {
            let c = self.decode(op["data"].as_str().unwrap_or_default())?;
            let _ = c.step.clone().apply(&mut shadow);
            let _ = ed.apply_remote(&c);
        }
        ed.set_live(true);
        ed.doc.next_id = ed.doc.next_id.max(ids.0);
        if self.access != Access::Edit {
            self.last = Some(ed.doc.clone());
        }
        self.live = Some(Live {
            client: Client::new(seq, name),
            shadow,
            shadow_seq: seq,
            queue: BTreeMap::new(),
            sent: HashMap::new(),
            ids,
        });
        Ok(())
    }

    fn wire(&self, b: Batch<Change>) -> Value {
        json!({ "node": self.node, "client": b.client, "base": b.base, "ops": b.ops.iter().map(|c| self.encode(c)).collect::<Vec<_>>() })
    }

    /// My changes since last time: a batch to send, if one can go now.
    /// Someone who may only read (or comment) gets back what they may not
    /// change, with [`Remote::refused`] saying why.
    pub fn outgoing(&mut self, ed: &mut Editor) -> Option<Value> {
        let mut made = ed.take_changes();
        if self.access != Access::Edit && !made.is_empty() {
            let last = self.last.clone().unwrap_or_else(|| ed.doc.clone());
            let why = match self.access {
                Access::Read => Some("You can only read this document."),
                _ => {
                    let who = [ed.doc.user.clone()];
                    let who: Vec<&str> = who.iter().map(String::as_str).collect();
                    let mut doc = last.clone();
                    made.iter().find_map(|c| {
                        let r = xenify_wp_doc::review::comment_only(&doc, &c.step, &who);
                        let _ = c.step.clone().apply(&mut doc);
                        match r {
                            Ok(()) => None,
                            Err(xenify_wp_doc::review::NotAComment::Content) => {
                                Some("You can comment on this document, but not change it.")
                            }
                            Err(xenify_wp_doc::review::NotAComment::NotYours) => {
                                Some("You can only change or delete your own comments.")
                            }
                        }
                    })
                }
            };
            if let Some(why) = why {
                let next = ed.doc.next_id;
                ed.reset_to(last);
                ed.doc.next_id = next;
                made.clear();
                self.refused = Some(why.into());
            }
            self.last = Some(ed.doc.clone());
        }
        let live = self.live.as_mut()?;
        let batch = if made.is_empty() { None } else { live.client.local(made) }?;
        live.sent.insert(batch.client.clone(), batch.ops.clone());
        Some(self.wire(batch))
    }

    /// Changes from others (`[{seq, data}]`).
    pub fn incoming(&mut self, ed: &mut Editor, ops: &[Value]) -> Result<(), String> {
        let decoded: Vec<(i64, Change)> = ops
            .iter()
            .map(|o| Ok((o["seq"].as_i64().unwrap_or(0), self.decode(o["data"].as_str().unwrap_or_default())?)))
            .collect::<Result<_, String>>()?;
        let live = self.live.as_mut().ok_or("not editing together")?;
        for (seq, c) in decoded {
            live.queue.insert(seq, c.clone());
            for apply in live.client.remote(seq, c) {
                let _ = ed.apply_remote(&apply);
            }
        }
        self.settle(ed);
        Ok(())
    }

    /// The server took batch `client` as changes `from..=to`. Returns the
    /// next batch to send, if any.
    pub fn accepted(&mut self, ed: &mut Editor, client: &str, from: i64, to: i64) -> Result<Option<Value>, String> {
        let live = self.live.as_mut().ok_or("not editing together")?;
        if let Some(ops) = live.sent.remove(client) {
            for (i, c) in ops.into_iter().enumerate() {
                live.queue.insert(from + i as i64, c);
            }
        }
        let (apply, next) =
            live.client.accepted(client, from, to).map_err(|_| "a change went missing: open the document again")?;
        for c in apply {
            let _ = ed.apply_remote(&c);
        }
        if let Some(b) = &next {
            live.sent.insert(b.client.clone(), b.ops.clone());
        }
        self.settle(ed);
        Ok(next.map(|b| self.wire(b)))
    }

    /// Batch `client` came too late; `ops` are what it missed. Returns the
    /// batch to send again.
    pub fn behind(&mut self, ed: &mut Editor, client: &str, ops: &[Value]) -> Result<Option<Value>, String> {
        let missed: Vec<(i64, Change)> = ops
            .iter()
            .map(|o| Ok((o["seq"].as_i64().unwrap_or(0), self.decode(o["data"].as_str().unwrap_or_default())?)))
            .collect::<Result<_, String>>()?;
        let live = self.live.as_mut().ok_or("not editing together")?;
        live.sent.remove(client);
        for (seq, c) in &missed {
            live.queue.insert(*seq, c.clone());
        }
        let (apply, next) = live.client.behind(client, missed);
        for c in apply {
            let _ = ed.apply_remote(&c);
        }
        if let Some(b) = &next {
            live.sent.insert(b.client.clone(), b.ops.clone());
        }
        self.settle(ed);
        Ok(next.map(|b| self.wire(b)))
    }

    /// After reconnecting: what to send again.
    pub fn resend(&mut self) -> Option<Value> {
        let live = self.live.as_mut()?;
        let b = live.client.resend()?;
        live.sent.insert(b.client.clone(), b.ops.clone());
        Some(self.wire(b))
    }

    /// The server's copy catches up; with nothing of mine outstanding the
    /// screen must match it, or takes it.
    fn settle(&mut self, ed: &mut Editor) {
        let Some(live) = self.live.as_mut() else { return };
        live.queue.retain(|s, _| *s > live.shadow_seq);
        while let Some(c) = live.queue.remove(&(live.shadow_seq + 1)) {
            let _ = c.step.apply(&mut live.shadow);
            live.shadow_seq += 1;
        }
        if live.client.settled() && live.queue.is_empty() && !same_content(&ed.doc, &live.shadow) {
            // New paragraph ids stay in this window's range.
            let next = ed.doc.next_id;
            ed.reset_to(live.shadow.clone());
            ed.doc.next_id = next;
        }
        if self.access != Access::Edit {
            self.last = Some(ed.doc.clone());
        }
    }

    /// The server's changes this copy holds (for saving a version).
    pub fn seq(&self) -> i64 {
        self.live.as_ref().map_or(self.upto, |l| l.client.seq())
    }

    /// The document as the server has it, for saving a version while
    /// editing together (my changes not yet taken aren't in it).
    pub fn server_copy<'a>(&'a self, ed: &'a Editor) -> &'a Document {
        self.live.as_ref().map_or(&ed.doc, |l| &l.shadow)
    }

    /// What the document's pictures need: `upload` the ones here but not on
    /// the server (they go up on their own, without a version), and the ones
    /// used but not here (`missing`: someone else added them; fetch them).
    pub fn needs(&self, ed: &Editor) -> (Vec<String>, Vec<String>) {
        let used = pictures(&ed.doc);
        let upload =
            used.iter().filter(|k| ed.doc.assets.contains_key(*k) && !self.on_server.contains(*k)).cloned().collect();
        let missing = used.into_iter().filter(|k| !ed.doc.assets.contains_key(k)).collect();
        (upload, missing)
    }

    /// Takes one picture someone else put in (`name`, its bytes).
    pub fn take_picture(&mut self, ed: &mut Editor, name: &str, bytes: Vec<u8>) {
        let bytes = std::sync::Arc::new(bytes);
        ed.doc.assets.insert(name.to_string(), bytes.clone());
        if let Some(live) = self.live.as_mut() {
            live.shadow.assets.insert(name.to_string(), bytes);
        }
        self.on_server.insert(name.to_string());
    }

    /// Takes the pictures missing here from a version of the document
    /// (`doc`, as loaded). Returns the ones taken.
    pub fn take_pictures(&mut self, ed: &mut Editor, doc: &Document) -> Vec<String> {
        self.on_server.extend(doc.assets.keys().cloned());
        let (_, missing) = self.needs(ed);
        let mut taken = Vec::new();
        for k in missing {
            if let Some(bytes) = doc.assets.get(&k) {
                ed.doc.assets.insert(k.clone(), bytes.clone());
                if let Some(live) = self.live.as_mut() {
                    live.shadow.assets.insert(k.clone(), bytes.clone());
                }
                taken.push(k);
            }
        }
        taken
    }
}

/// The pictures a document shows that are kept inside it (`media/…`):
/// in its text, tables, boxes, notes, headers and footers.
pub fn pictures(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, src) in every_picture(doc) {
        if src.starts_with("media/") && !out.contains(&src) {
            out.push(src);
        }
    }
    out
}

/// Every picture in a document (its id and source), wherever it is.
pub fn every_picture(doc: &Document) -> Vec<(xenify_wp_doc::NodeId, String)> {
    fn walk(blocks: &[xenify_wp_doc::Block], out: &mut Vec<(xenify_wp_doc::NodeId, String)>) {
        for b in blocks {
            match b {
                xenify_wp_doc::Block::Image(i) => out.push((i.id, i.src.clone())),
                xenify_wp_doc::Block::Table(t) => {
                    for cell in t.rows.iter().flat_map(|r| &r.cells) {
                        walk(&cell.blocks, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        walk(&s.blocks, &mut out);
        for f in &s.frames {
            walk(&f.blocks, &mut out);
        }
        for n in &s.notes {
            walk(&n.blocks, &mut out);
        }
        for h in &s.headers {
            walk(&h.blocks, &mut out);
        }
    }
    out
}
