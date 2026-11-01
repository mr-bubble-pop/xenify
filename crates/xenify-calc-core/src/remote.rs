//! A book kept on a Xenify server: where it is, and editing it together. As in Draw
//! (xenify-draw-core/src/remote.rs): the app's server.js moves the bytes,
//! this reads and changes them. The server's copy (the "shadow", every
//! change in the server's order) is kept beside the one on screen, which
//! takes it if adjusting someone's change ever went wrong.

use crate::book::Book;
use crate::collab::Change;
use crate::ops::{self, Op};
use crate::{Session, file};
use base64::Engine;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use xenify_collab::{Batch, Client};

pub struct Remote {
    pub server: String,
    pub node: i64,
    pub name: String,
    pub version: i64,
    pub upto: i64,
    live: Option<Live>,
}

struct Live {
    client: Client<Change>,
    shadow: Book,
    shadow_seq: i64,
    /// Server changes not yet applied to the shadow (in their order).
    queue: BTreeMap<i64, Change>,
    /// Batches sent, by id (they join the queue once accepted).
    sent: HashMap<String, Vec<Change>>,
    /// A batch made while answering the server, to go with the next live_out.
    waiting: Option<Batch<Change>>,
}

impl Remote {
    fn encode(&self, c: &Change) -> String {
        serde_json::to_string(c).expect("a change serialises")
    }

    /// A change the server took but this version can't read came from a
    /// newer Calc (a kind of change added since): say so, not serde's words.
    fn decode(&self, data: &str) -> Result<Change, String> {
        serde_json::from_str(data).map_err(|e| {
            if e.is_data() {
                "Someone is working on this with a newer version of Xenify Calc. Update Calc to keep working on it together."
                    .to_string()
            } else {
                e.to_string()
            }
        })
    }

    fn decode_all(&self, ops: &[Json]) -> Result<Vec<(i64, Change)>, String> {
        ops.iter()
            .map(|o| Ok((o["seq"].as_i64().unwrap_or(0), self.decode(o["data"].as_str().unwrap_or_default())?)))
            .collect()
    }

    fn wire(&self, b: Batch<Change>) -> Json {
        json!({ "node": self.node, "client": b.client, "base": b.base, "ops": b.ops.iter().map(|c| self.encode(c)).collect::<Vec<_>>() })
    }
}

/// A copy of a book (through its file: Book holds IronCalc's model, which
/// doesn't copy).
fn copy(book: &Book) -> Result<Book, String> {
    file::open(&file::save(book)?)
}

/// What a book holds, to compare two (a cell with nothing in it and no
/// look of its own counts as no cell).
fn content(book: &Book) -> String {
    let mut v = serde_json::to_value(file::FileBook::from_book(book)).unwrap_or_default();
    if let Some(sheets) = v.get_mut("sheets").and_then(Json::as_array_mut) {
        for sh in sheets {
            if let Some(cells) = sh.get_mut("cells").and_then(Json::as_array_mut) {
                cells.retain(|c| c.as_object().is_some_and(|o| o.keys().any(|k| k != "r" && k != "c")));
            }
        }
    }
    v.to_string()
}

/// Applies someone else's change; one that no longer fits is left out (the
/// shadow sets it right).
fn apply_to(book: &mut Book, c: &Change) {
    let _ = ops::apply_all(book, std::slice::from_ref(&c.op));
}

impl Session {
    /// An op as it goes to the others (with its sheet's name now).
    fn change(&self, op: Op) -> Change {
        let name = crate::collab::sheet_of(&op).and_then(|s| self.book.sheet_name(s).ok()).unwrap_or_default();
        Change { op, name }
    }

    pub fn editing_together(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.live.is_some())
    }

    /// Edits just made here (and applied): kept to send while editing together.
    pub(crate) fn made(&mut self, ops: &[Op]) {
        if self.remote.as_ref().is_some_and(|r| r.live.is_some()) {
            self.outbox.extend(ops.iter().cloned());
        }
    }

    /// Someone else's changes, here: applied, and the undo steps moved to
    /// still undo what they did.
    fn apply_remote(&mut self, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        for c in &changes {
            apply_to(&mut self.book, c);
        }
        // With their sheets' names: formulas in them follow rows and columns moved there.
        let book = &self.book;
        let named = |op: &Op| Change {
            op: op.clone(),
            name: crate::collab::sheet_of(op).and_then(|s| book.sheet_name(s).ok()).unwrap_or_default(),
        };
        for stack in [&mut self.undo, &mut self.redo] {
            for step in stack.iter_mut() {
                let mine: Vec<Change> = step.iter().map(named).collect();
                let (moved, _) = xenify_collab::transform(&mine, &changes);
                *step = moved.into_iter().map(|c| c.op).collect();
            }
            stack.retain(|s| !s.is_empty());
        }
        self.changes += 1;
    }

    /// The server's copy catches up; with nothing of mine outstanding the
    /// book must match it, or takes it.
    fn settle(&mut self) -> Result<(), String> {
        let Some(r) = self.remote.as_mut() else { return Ok(()) };
        let Some(live) = r.live.as_mut() else { return Ok(()) };
        live.queue.retain(|s, _| *s > live.shadow_seq);
        let mut moved = false;
        while let Some(c) = live.queue.remove(&(live.shadow_seq + 1)) {
            apply_to(&mut live.shadow, &c);
            live.shadow_seq += 1;
            moved = true;
        }
        if moved
            && live.client.settled()
            && live.queue.is_empty()
            && self.outbox.is_empty()
            && content(&self.book) != content(&live.shadow)
        {
            let lease = self.book.lease;
            self.book = copy(&live.shadow)?;
            self.book.lease = lease;
            self.undo.clear();
            self.redo.clear();
            self.changes += 1;
        }
        Ok(())
    }

    /// My changes since last time, as a batch to send if one can go now.
    pub fn live_out(&mut self) -> Option<Json> {
        let made: Vec<Op> = std::mem::take(&mut self.outbox);
        let changes: Vec<Change> = made.into_iter().map(|op| self.change(op)).collect();
        let r = self.remote.as_mut()?;
        let live = r.live.as_mut()?;
        let b = match live.waiting.take() {
            Some(b) => {
                if !changes.is_empty() {
                    // Goes once the waiting one is answered.
                    let _ = live.client.local(changes);
                }
                Some(b)
            }
            None if changes.is_empty() => None,
            None => live.client.local(changes),
        }?;
        live.sent.insert(b.client.clone(), b.ops.clone());
        Some(r.wire(b))
    }

    /// Calls for books on a Xenify server (`remote_…`, `live_…`). None: not one of them.
    pub(crate) fn remote_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        if !(name.starts_with("remote_") || name.starts_with("live_")) {
            return None;
        }
        let b64s = base64::engine::general_purpose::STANDARD;
        let s = |k: &str| args.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
        let i = |k: &str| args.get(k).and_then(Json::as_i64).unwrap_or(0);
        Some((|| -> Result<Json, String> {
            match name {
                "remote_open" => {
                    let remote = Remote {
                        server: s("server"),
                        node: i("node"),
                        name: s("name"),
                        version: i("version"),
                        upto: i("upto"),
                        live: None,
                    };
                    let data = s("data");
                    if !data.is_empty() {
                        let bytes = b64s.decode(data).map_err(|e| e.to_string())?;
                        self.book = file::open(&bytes)?;
                        self.format = crate::Format::Xcalc;
                        self.undo.clear();
                        self.redo.clear();
                    }
                    self.remote = Some(remote);
                    self.saved = self.changes;
                    Ok(json!({ "node": i("node") }))
                }
                "remote_save" => {
                    let r = self.remote.as_ref().ok_or("this book isn't on a server")?;
                    // `mine`: with the changes made here not on the server yet
                    // (kept to send later), not the server's own copy.
                    let mine = args.get("mine").and_then(Json::as_bool).unwrap_or(false);
                    let book = if mine { &self.book } else { r.live.as_ref().map_or(&self.book, |l| &l.shadow) };
                    let upto = r.live.as_ref().map_or(r.upto, |l| l.client.seq());
                    Ok(json!({
                        "node": r.node, "base": r.version, "upto": upto, "data": b64s.encode(file::save(book)?), "text": words(book),
                    }))
                }
                "remote_saved" => {
                    let r = self.remote.as_mut().ok_or("this book isn't on a server")?;
                    r.version = i("version");
                    // Editing together, everything is on the server already.
                    self.saved = self.changes;
                    Ok(Json::Null)
                }
                "remote_info" => Ok(match &self.remote {
                    Some(r) => {
                        json!({ "server": r.server, "node": r.node, "name": r.name, "version": r.version, "live": r.live.is_some() })
                    }
                    None => Json::Null,
                }),
                "remote_unsent" => Ok(
                    json!({ "unsent": self.remote.as_ref().and_then(|r| r.live.as_ref()).is_some_and(|l| !l.client.settled()) || !self.outbox.is_empty() }),
                ),
                "live_start" => {
                    let ops = args.get("ops").and_then(Json::as_array).cloned().unwrap_or_default();
                    let r = self.remote.as_ref().ok_or("this book isn't on a server")?;
                    let caught = r.decode_all(&ops)?;
                    let mut shadow = copy(&self.book)?;
                    for (_, c) in &caught {
                        apply_to(&mut shadow, c);
                    }
                    for (_, c) in &caught {
                        apply_to(&mut self.book, c);
                    }
                    let from = args.get("from").and_then(Json::as_u64).unwrap_or(1) as u32;
                    let to = args.get("to").and_then(Json::as_u64).map_or(u32::MAX, |t| t.min(u32::MAX as u64) as u32);
                    self.book.lease = Some((from, to));
                    let seq = i("seq");
                    let r = self.remote.as_mut().unwrap();
                    r.live = Some(Live {
                        client: Client::new(seq, &s("conn")),
                        shadow,
                        shadow_seq: seq,
                        queue: BTreeMap::new(),
                        sent: HashMap::new(),
                        waiting: None,
                    });
                    self.outbox.clear();
                    self.changes += 1;
                    self.saved = self.changes;
                    Ok(json!({ "live": true }))
                }
                // How many leased ids are left; a new range when they run low.
                "live_ids" => {
                    let (from, to) = self.book.lease.ok_or("not editing together")?;
                    let next = self.book.next_id(from);
                    Ok(json!({ "left": to.saturating_sub(next), "next": next }))
                }
                "live_lease" => {
                    let to = args.get("to").and_then(Json::as_u64).ok_or("missing “to”")?;
                    let from = args.get("from").and_then(Json::as_u64).unwrap_or(1);
                    self.book.lease = Some((from as u32, to.min(u32::MAX as u64) as u32));
                    Ok(Json::Null)
                }
                "live_ops" => {
                    self.fold_outbox();
                    let r = self.remote.as_mut().ok_or("this book isn't on a server")?;
                    let incoming = r.decode_all(args.get("ops").and_then(Json::as_array).map_or(&[][..], |v| v))?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let mut apply = Vec::new();
                    for (seq, c) in incoming {
                        live.queue.insert(seq, c.clone());
                        apply.extend(live.client.remote(seq, c));
                    }
                    self.apply_remote(apply);
                    self.settle()?;
                    Ok(json!({ "changed": true }))
                }
                "live_accepted" | "live_behind" => {
                    self.fold_outbox();
                    let client = s("client");
                    let r = self.remote.as_mut().ok_or("this book isn't on a server")?;
                    let missed = r.decode_all(args.get("ops").and_then(Json::as_array).map_or(&[][..], |v| v))?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let (apply, next) = if name == "live_accepted" {
                        if let Some(ops) = live.sent.remove(&client) {
                            for (k, c) in ops.into_iter().enumerate() {
                                live.queue.insert(i("from") + k as i64, c);
                            }
                        }
                        live.client
                            .accepted(&client, i("from"), i("to"))
                            .map_err(|_| "a change went missing: open the book again")?
                    } else {
                        live.sent.remove(&client);
                        for (seq, c) in &missed {
                            live.queue.insert(*seq, c.clone());
                        }
                        live.client.behind(&client, missed)
                    };
                    let next = next.or_else(|| live.waiting.take());
                    if let Some(b) = &next {
                        live.sent.insert(b.client.clone(), b.ops.clone());
                    }
                    let next = next.map(|b| r.wire(b));
                    self.apply_remote(apply);
                    self.settle()?;
                    Ok(json!({ "batch": next }))
                }
                "live_resend" => {
                    let r = self.remote.as_mut().ok_or("this book isn't on a server")?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let b = live.client.resend();
                    if let Some(b) = &b {
                        live.sent.insert(b.client.clone(), b.ops.clone());
                    }
                    Ok(json!({ "batch": b.map(|b| r.wire(b)) }))
                }
                "live_stop" => {
                    if let Some(r) = self.remote.as_mut() {
                        r.live = None;
                    }
                    self.book.lease = None;
                    Ok(Json::Null)
                }
                // Where my cursor is, for the others (a `say`); and theirs.
                "live_say" => {
                    let r = self.remote.as_ref().ok_or("this book isn't on a server")?;
                    let at = json!({ "sheet": args["sheet"], "row": args["row"], "col": args["col"], "bottom": args["bottom"], "right": args["right"] });
                    Ok(json!({ "node": r.node, "data": at.to_string() }))
                }
                "live_heard" => {
                    self.remote.as_ref().ok_or("this book isn't on a server")?;
                    let at: Json = serde_json::from_str(&s("data")).map_err(|e| e.to_string())?;
                    Ok(at)
                }
                _ => Err(format!("unknown call: {name}")),
            }
        })())
    }

    /// Edits made here but not yet handed to the client go in first, so
    /// incoming changes are adjusted against them too.
    fn fold_outbox(&mut self) {
        if self.outbox.is_empty() {
            return;
        }
        let made: Vec<Op> = std::mem::take(&mut self.outbox);
        let changes: Vec<Change> = made.into_iter().map(|op| self.change(op)).collect();
        if let Some(live) = self.remote.as_mut().and_then(|r| r.live.as_mut())
            && let Some(b) = live.client.local(changes)
        {
            live.waiting = Some(b);
        }
    }
}

/// The book's words: every sheet's name and cells as shown, for the
/// server's search.
fn words(book: &Book) -> String {
    let mut out = String::new();
    for w in &book.model.workbook.worksheets {
        out.push_str(&w.name);
        out.push('\n');
        out.push_str(&crate::csv::save(book, w.sheet_id, '\t').unwrap_or_default());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A pretend Xenify server: keeps the order of changes and says who got in first.
    #[derive(Default)]
    struct Pretend {
        log: Vec<Json>,
        to: [Vec<(String, Json)>; 2],
    }

    impl Pretend {
        fn take(&mut self, who: usize, batch: Option<Json>) {
            let Some(batch) = batch.filter(|b| !b.is_null()) else { return };
            let base = batch["base"].as_i64().unwrap();
            if base != self.log.len() as i64 {
                let missed = self.log[base as usize..].to_vec();
                self.to[who].push(("live_behind".into(), json!({ "client": batch["client"], "ops": missed })));
                return;
            }
            let from = self.log.len() as i64 + 1;
            let mut ops = Vec::new();
            for d in batch["ops"].as_array().unwrap() {
                let op = json!({ "seq": self.log.len() as i64 + 1, "data": d });
                self.log.push(op.clone());
                ops.push(op);
            }
            self.to[who].push((
                "live_accepted".into(),
                json!({ "client": batch["client"], "from": from, "to": self.log.len() }),
            ));
            self.to[1 - who].push(("live_ops".into(), json!({ "ops": ops })));
        }

        /// Delivers everything until nothing's left.
        fn run(&mut self, a: &mut Session, b: &mut Session) {
            for _ in 0..50 {
                let [qa, qb] = std::mem::take(&mut self.to);
                if qa.is_empty() && qb.is_empty() {
                    break;
                }
                for (name, m) in qa {
                    let out = a.call(&name, &m).unwrap();
                    self.take(0, out.get("batch").cloned());
                    self.take(0, a.live_out());
                }
                for (name, m) in qb {
                    let out = b.call(&name, &m).unwrap();
                    self.take(1, out.get("batch").cloned());
                    self.take(1, b.live_out());
                }
            }
        }
    }

    fn text(s: &Session, sheet: u32, row: i32, col: i32) -> String {
        let i = s.book.index(sheet).unwrap();
        s.book.model.get_formatted_cell_value(i, row, col).unwrap()
    }

    #[test]
    fn two_calcs_edit_one_server_book() {
        let mut a = Session::new();
        let sheet = a.book.id_at(0).unwrap();
        for (r, v) in [(1, "Item"), (2, "Tea"), (3, "Jam"), (4, "Cocoa")] {
            a.type_in(sheet, r, 1, v).unwrap();
        }
        a.type_in(sheet, 5, 2, "=COUNTA(A2:A4)").unwrap();
        a.call("remote_open", &json!({ "server": "s", "node": 3, "name": "Shop", "data": "" })).unwrap();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let mut b = Session::new();
        b.call(
            "remote_open",
            &json!({ "server": "s", "node": 3, "name": "Shop", "data": saved["data"], "version": 1 }),
        )
        .unwrap();
        assert_eq!(text(&b, sheet, 3, 1), "Jam");
        a.call("live_start", &json!({ "seq": 0, "conn": "a", "from": 1000, "to": 2000 })).unwrap();
        b.call("live_start", &json!({ "seq": 0, "conn": "b", "from": 2000, "to": 3000 })).unwrap();
        let mut srv = Pretend::default();

        // At the same time: A inserts a row above Jam; B types a price beside
        // Jam and a formula about it; both add a sheet called Notes.
        a.call("insert_rows", &json!({ "sheet": sheet, "at": 3, "count": 1 })).unwrap();
        a.type_in(sheet, 3, 1, "Coffee").unwrap();
        b.type_in(sheet, 3, 2, "2.80").unwrap();
        b.type_in(sheet, 6, 3, "=B3*2").unwrap();
        let na = a.call("add_sheet", &json!({ "name": "Notes" })).unwrap().as_u64().unwrap() as u32;
        let nb = b.call("add_sheet", &json!({ "name": "Notes" })).unwrap().as_u64().unwrap() as u32;
        assert!((1000..2000).contains(&na) && (2000..3000).contains(&nb), "{na} {nb}: ids from each window's lease");
        let (oa, ob) = (a.live_out(), b.live_out());
        srv.take(0, oa);
        srv.take(1, ob);
        srv.run(&mut a, &mut b);

        assert_eq!(content(&a.book), content(&b.book), "both windows hold the same book");
        assert_eq!(text(&a, sheet, 3, 1), "Coffee");
        assert_eq!(text(&a, sheet, 4, 1), "Jam");
        assert_eq!(text(&a, sheet, 4, 2), "2.8", "B's price followed Jam down a row");
        let i = a.book.index(sheet).unwrap();
        assert_eq!(a.book.model.get_localized_cell_content(i, 7, 3).unwrap(), "=B4*2", "B's formula moved with it");
        assert_eq!(a.book.model.get_localized_cell_content(i, 6, 2).unwrap(), "=COUNTA(A2:A5)");
        let names: Vec<String> = a.book.model.workbook.worksheets.iter().map(|w| w.get_name()).collect();
        assert!(names.contains(&"Notes".to_string()) && names.contains(&"Notes 2".to_string()), "{names:?}");

        // Undo undoes my own edits, where they are now: the sheet, then the formula (moved down a row).
        b.call("undo", &json!({})).unwrap();
        b.call("undo", &json!({})).unwrap();
        srv.take(1, b.live_out());
        srv.run(&mut a, &mut b);
        assert_eq!(text(&a, sheet, 7, 3), "");
        assert_eq!(text(&a, sheet, 4, 2), "2.8", "the price stays");
        assert_eq!(a.book.model.workbook.worksheets.len(), 2);
        assert_eq!(content(&a.book), content(&b.book));

        // Deletes meeting: both delete Coffee's row; one row goes.
        a.call("delete_rows", &json!({ "sheet": sheet, "at": 3, "count": 1 })).unwrap();
        b.call("delete_rows", &json!({ "sheet": sheet, "at": 3, "count": 1 })).unwrap();
        let (oa, ob) = (a.live_out(), b.live_out());
        srv.take(0, oa);
        srv.take(1, ob);
        srv.run(&mut a, &mut b);
        assert_eq!(text(&a, sheet, 3, 1), "Jam");
        assert_eq!(text(&b, sheet, 4, 1), "Cocoa");
        assert_eq!(content(&a.book), content(&b.book));

        // Saving a version takes the server's copy.
        let v = a.call("remote_save", &json!({})).unwrap();
        let back = Session::open_file(&b64s().decode(v["data"].as_str().unwrap()).unwrap(), "x.xcalc").unwrap();
        assert_eq!(content(&back.book), content(&a.book));
    }

    #[test]
    fn undo_puts_formulas_back_where_rows_moved_them() {
        let mut a = Session::new();
        let sheet = a.book.id_at(0).unwrap();
        a.type_in(sheet, 1, 3, "=A4").unwrap();
        a.call("remote_open", &json!({ "server": "s", "node": 3, "name": "Shop", "data": "" })).unwrap();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let mut b = Session::new();
        b.call(
            "remote_open",
            &json!({ "server": "s", "node": 3, "name": "Shop", "data": saved["data"], "version": 1 }),
        )
        .unwrap();
        a.call("live_start", &json!({ "seq": 0, "conn": "a", "from": 1000, "to": 2000 })).unwrap();
        b.call("live_start", &json!({ "seq": 0, "conn": "b", "from": 2000, "to": 3000 })).unwrap();
        let mut srv = Pretend::default();
        // A types over its formula; B inserts a row above the cell it names.
        a.type_in(sheet, 1, 3, "x").unwrap();
        srv.take(0, a.live_out());
        srv.run(&mut a, &mut b);
        b.call("insert_rows", &json!({ "sheet": sheet, "at": 2, "count": 1 })).unwrap();
        srv.take(1, b.live_out());
        srv.run(&mut a, &mut b);
        // A's undo brings the formula back, naming the same cell (now A5).
        a.undo().unwrap();
        srv.take(0, a.live_out());
        srv.run(&mut a, &mut b);
        let i = a.book.index(sheet).unwrap();
        assert_eq!(a.book.model.get_localized_cell_content(i, 1, 3).unwrap(), "=A5");
        assert_eq!(content(&a.book), content(&b.book));
    }

    fn b64s() -> base64::engine::GeneralPurpose {
        base64::engine::general_purpose::STANDARD
    }
}
