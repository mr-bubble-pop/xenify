//! A canvas kept on a Xenify server: where it is, and editing it together. As in Write
//! (xenify-write-host/src/remote.rs): the screens move the bytes, this
//! reads and changes them, and the server's copy is kept beside the one on
//! screen, which takes it if adjusting someone's change ever went wrong.

use crate::Session;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use xenify_collab::{Batch, Client};
use xenify_doc::Project;
use xenify_ops::collab::{Change, same_content};

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
    shadow: Project,
    shadow_seq: i64,
    queue: BTreeMap<i64, Change>,
    sent: HashMap<String, Vec<Change>>,
    /// The end of the id range leased to this window.
    ids_to: u64,
}

impl Remote {
    /// A change for the server: pictures and clips in it by name (their
    /// bytes go up beside it, see xenify_doc::blobs).
    fn encode(&self, c: &Change) -> String {
        xenify_doc::blobs::by_reference(|| serde_json::to_string(c).expect("a change serialises"))
    }

    /// A change the server took but this version can't read came from a
    /// newer Draw (a kind of change added since): say so, not serde's words.
    fn decode(&self, data: &str) -> Result<Change, String> {
        serde_json::from_str(data).map_err(|e| {
            if e.is_data() {
                "Someone is working on this with a newer version of Xenify Draw. Update Draw to keep working on it together."
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

fn apply_to(p: &mut Project, c: &Change) {
    let cmd = c.command();
    let _ = xenify_ops::apply(p, cmd);
}

impl Session {
    /// The server's copy catches up; with nothing of mine outstanding the
    /// canvas must match it, or takes it.
    fn settle(&mut self) {
        let Some(r) = self.remote.as_mut() else { return };
        let Some(live) = r.live.as_mut() else { return };
        live.queue.retain(|s, _| *s > live.shadow_seq);
        while let Some(c) = live.queue.remove(&(live.shadow_seq + 1)) {
            apply_to(&mut live.shadow, &c);
            live.shadow_seq += 1;
        }
        if live.client.settled() && live.queue.is_empty() && !same_content(self.editor.project(), &live.shadow) {
            self.editor.reset_to(live.shadow.clone());
        }
    }

    fn apply_all(&mut self, changes: Vec<Change>) {
        for c in changes {
            let _ = self.editor.apply_remote(&c);
        }
        let p = self.editor.project();
        self.selection.retain(|id| p.layer(*id).is_ok());
    }

    /// My changes since last time, as a batch to send if one can go now.
    pub fn live_out(&mut self) -> Option<Json> {
        let made = self.editor.take_changes();
        let r = self.remote.as_mut()?;
        let live = r.live.as_mut()?;
        let b = if made.is_empty() { None } else { live.client.local(made) }?;
        live.sent.insert(b.client.clone(), b.ops.clone());
        Some(r.wire(b))
    }

    /// Calls for canvases on a Xenify server (`remote_…`, `live_…`). None:
    /// not one of them.
    pub(crate) fn remote_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        use base64::Engine;
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
                        let json = b64s.decode(data).map_err(|e| e.to_string())?;
                        let p = Project::from_json(&String::from_utf8(json).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                        self.replace(Session::new(p));
                    }
                    self.remote = Some(remote);
                    self.editor.mark_saved();
                    Ok(self.state())
                }
                "remote_save" => {
                    let r = self.remote.as_ref().ok_or("this canvas isn't on a server")?;
                    // `mine`: with the changes made here not on the server yet
                    // (kept to send later), not the server's own copy.
                    let mine = args.get("mine").and_then(Json::as_bool).unwrap_or(false);
                    let p = if mine { self.editor.project() } else { r.live.as_ref().map_or(self.editor.project(), |l| &l.shadow) };
                    let upto = r.live.as_ref().map_or(r.upto, |l| l.client.seq());
                    Ok(json!({ "node": r.node, "base": r.version, "upto": upto, "data": b64s.encode(xenify_doc::blobs::by_reference(|| p.to_json()).as_bytes()), "text": words(p) }))
                }
                "remote_saved" => {
                    let live = {
                        let r = self.remote.as_mut().ok_or("this canvas isn't on a server")?;
                        r.version = i("version");
                        r.live.is_some()
                    };
                    if !live {
                        self.editor.mark_saved();
                    }
                    Ok(Json::Null)
                }
                // Pictures of documents on a server (a Write page): an image
                // named `xen-<server>-<node>.png`, kept in the canvas (so it
                // shows offline) and refreshed when the document changes.
                "link_insert" => {
                    let name = link_name(&s("server"), i("node"));
                    self.call("add_picture", &json!({ "base64": s("data"), "name": name, "mime": "image/png", "width": args["width"], "height": args["height"] }))
                }
                "links" => {
                    let mut out: Vec<Json> = Vec::new();
                    for a in &self.project().assets {
                        if let Some((server, node)) = parse_link_name(&a.name)
                            && !out.iter().any(|v| v["server"] == server && v["node"] == node)
                        {
                            out.push(json!({ "server": server, "node": node }));
                        }
                    }
                    Ok(json!(out))
                }
                "link_update" => {
                    let name = link_name(&s("server"), i("node"));
                    let data = b64s.decode(s("data")).map_err(|e| e.to_string())?;
                    let (w, h) = (args["width"].as_u64().unwrap_or(0) as u32, args["height"].as_u64().unwrap_or(0) as u32);
                    let old = self.project().assets.iter().find(|a| a.name == name).cloned().ok_or("that picture isn't in this canvas")?;
                    if *old.data == *data {
                        return Ok(Json::Null);
                    }
                    let asset = xenify_doc::ImageAsset::new(old.id, name, "image/png", w.max(1), h.max(1), data);
                    self.exec("Update picture", xenify_ops::Command::SetAsset { asset })?;
                    Ok(json!(true))
                }
                "link_keep" => {
                    // Stop updating: the picture becomes the canvas's own.
                    let layer = xenify_doc::Id(args["id"].as_u64().ok_or("missing “id”")?);
                    let asset_id = match &self.project().layer(layer).map_err(|e| e.to_string())?.kind {
                        xenify_doc::LayerKind::Image { asset, .. } => *asset,
                        _ => return Err("that isn't a picture".into()),
                    };
                    let mut asset = self.project().asset(asset_id).cloned().ok_or("that picture is gone")?;
                    asset.name = "Picture.png".into();
                    self.exec("Keep this picture", xenify_ops::Command::SetAsset { asset })?;
                    Ok(Json::Null)
                }
                // Which layers I have selected, for the others editing this
                // canvas (a `say` message); and theirs.
                "live_say" => {
                    let r = self.remote.as_ref().ok_or("this canvas isn't on a server")?;
                    let at = json!({ "sel": self.selection });
                    Ok(json!({ "node": r.node, "data": at.to_string() }))
                }
                "live_heard" => {
                    self.remote.as_ref().ok_or("this canvas isn't on a server")?;
                    let at: Json = serde_json::from_str(&s("data")).map_err(|e| e.to_string())?;
                    Ok(json!({ "ids": at["sel"] }))
                }
                "remote_info" => Ok(match &self.remote {
                    Some(r) => json!({ "server": r.server, "node": r.node, "name": r.name, "version": r.version, "live": r.live.is_some() }),
                    None => Json::Null,
                }),
                "live_start" => {
                    let ops = args.get("ops").and_then(Json::as_array).cloned().unwrap_or_default();
                    let r = self.remote.as_ref().ok_or("this canvas isn't on a server")?;
                    let caught = r.decode_all(&ops)?;
                    let mut shadow = self.editor.project().clone();
                    for (_, c) in &caught {
                        apply_to(&mut shadow, c);
                    }
                    self.apply_all(caught.into_iter().map(|(_, c)| c).collect());
                    self.editor.set_live(true);
                    self.editor.lease_ids(args.get("from").and_then(Json::as_u64).unwrap_or(1));
                    let seq = i("seq");
                    let r = self.remote.as_mut().unwrap();
                    r.live = Some(Live {
                        client: Client::new(seq, &s("conn")),
                        shadow,
                        shadow_seq: seq,
                        queue: BTreeMap::new(),
                        sent: HashMap::new(),
                        ids_to: args.get("to").and_then(Json::as_u64).unwrap_or(u64::MAX),
                    });
                    Ok(self.state())
                }
                // Changes made here the server hasn't got yet (kept if Draw
                // closes before they're sent).
                "remote_unsent" => {
                    Ok(json!({ "unsent": self.remote.as_ref().and_then(|r| r.live.as_ref()).is_some_and(|l| !l.client.settled()) }))
                }
                // How many leased ids are left; a new range when they run low.
                "live_ids" => {
                    let live = self.remote.as_ref().and_then(|r| r.live.as_ref()).ok_or("not editing together")?;
                    let next = self.editor.project().next_id;
                    Ok(json!({ "left": live.ids_to.saturating_sub(next), "next": next }))
                }
                "live_lease" => {
                    let to = args.get("to").and_then(Json::as_u64).ok_or("missing “to”")?;
                    let live = self.remote.as_mut().and_then(|r| r.live.as_mut()).ok_or("not editing together")?;
                    live.ids_to = to;
                    self.editor.lease_ids(args.get("from").and_then(Json::as_u64).unwrap_or(1));
                    Ok(Json::Null)
                }
                "live_ops" => {
                    let r = self.remote.as_mut().ok_or("this canvas isn't on a server")?;
                    let incoming = r.decode_all(args.get("ops").and_then(Json::as_array).map_or(&[][..], |v| v))?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let mut apply = Vec::new();
                    for (seq, c) in incoming {
                        live.queue.insert(seq, c.clone());
                        apply.extend(live.client.remote(seq, c));
                    }
                    self.apply_all(apply);
                    self.settle();
                    Ok(self.state())
                }
                "live_accepted" | "live_behind" => {
                    let client = s("client");
                    let r = self.remote.as_mut().ok_or("this canvas isn't on a server")?;
                    let missed = r.decode_all(args.get("ops").and_then(Json::as_array).map_or(&[][..], |v| v))?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let (apply, next) = if name == "live_accepted" {
                        if let Some(ops) = live.sent.remove(&client) {
                            for (k, c) in ops.into_iter().enumerate() {
                                live.queue.insert(i("from") + k as i64, c);
                            }
                        }
                        live.client.accepted(&client, i("from"), i("to")).map_err(|_| "a change went missing: open the canvas again")?
                    } else {
                        live.sent.remove(&client);
                        for (seq, c) in &missed {
                            live.queue.insert(*seq, c.clone());
                        }
                        live.client.behind(&client, missed)
                    };
                    if let Some(b) = &next {
                        live.sent.insert(b.client.clone(), b.ops.clone());
                    }
                    let next = next.map(|b| r.wire(b));
                    self.apply_all(apply);
                    self.settle();
                    Ok(json!({ "state": self.state(), "batch": next }))
                }
                "live_resend" => {
                    let r = self.remote.as_mut().ok_or("this canvas isn't on a server")?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let b = live.client.resend();
                    if let Some(b) = &b {
                        live.sent.insert(b.client.clone(), b.ops.clone());
                    }
                    Ok(json!({ "batch": b.map(|b| r.wire(b)) }))
                }
                "live_stop" => {
                    self.editor.set_live(false);
                    if let Some(r) = self.remote.as_mut() {
                        r.live = None;
                    }
                    Ok(Json::Null)
                }
                _ => Err(String::new()),
            }
        })())
        .filter(|r| !matches!(r, Err(e) if e.is_empty()))
    }
}

/// The canvas's words (its text layers), for the server's search.
fn words(p: &Project) -> String {
    let mut out = Vec::new();
    for c in &p.compositions {
        for l in &c.layers {
            if let xenify_doc::LayerKind::Text(t) = &l.kind {
                out.push(t.text.clone());
            }
        }
    }
    out.join("\n")
}

/// The name of a picture of server document `node` in a canvas.
fn link_name(server: &str, node: i64) -> String {
    let server: String = server.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    format!("xen-{server}-{node}.png")
}

/// The server and document a linked picture shows.
fn parse_link_name(name: &str) -> Option<(String, i64)> {
    let rest = name.strip_prefix("xen-")?.strip_suffix(".png")?;
    let (server, node) = rest.rsplit_once('-')?;
    Some((server.to_string(), node.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn two_draws_edit_one_server_canvas() {
        let mut a = Session::new(crate::empty_project());
        a.call("remote_open", &json!({ "server": "s", "node": 3, "name": "Poster", "data": "" })).unwrap();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let mut b = Session::new(crate::empty_project());
        b.call(
            "remote_open",
            &json!({ "server": "s", "node": 3, "name": "Poster", "data": saved["data"], "version": 1 }),
        )
        .unwrap();
        a.call("live_start", &json!({ "seq": 0, "conn": "a", "from": 1000 })).unwrap();
        b.call("live_start", &json!({ "seq": 0, "conn": "b", "from": 2000 })).unwrap();
        let mut srv = Pretend::default();
        a.call("add_shape", &json!({ "x": 10, "y": 10, "w": 50, "h": 50 })).unwrap();
        b.call("add_shape", &json!({ "x": 200, "y": 10, "w": 30, "h": 30 })).unwrap();
        let (oa, ob) = (a.live_out(), b.live_out());
        srv.take(0, oa);
        srv.take(1, ob);
        for _ in 0..20 {
            let [qa, qb] = std::mem::take(&mut srv.to);
            if qa.is_empty() && qb.is_empty() {
                break;
            }
            for (name, m) in qa {
                let out = a.call(&name, &m).unwrap();
                srv.take(0, out.get("batch").cloned());
                srv.take(0, a.live_out());
            }
            for (name, m) in qb {
                let out = b.call(&name, &m).unwrap();
                srv.take(1, out.get("batch").cloned());
                srv.take(1, b.live_out());
            }
        }
        let show = |s: &Session| {
            s.editor
                .project()
                .compositions
                .iter()
                .map(|c| (c.id.0, c.layers.iter().map(|l| (l.id.0, l.meta.parent.map(|p| p.0))).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        };
        assert!(
            same_content(a.editor.project(), b.editor.project()),
            "{:?}\n{:?}\nlog {}",
            show(&a),
            show(&b),
            srv.log.len()
        );
        let ids: Vec<u64> = a.editor.project().compositions[0].layers.iter().map(|l| l.id.0).collect();
        assert!(
            ids.iter().any(|i| (1000..2000).contains(i)) && ids.iter().any(|i| (2000..3000).contains(i)),
            "{ids:?}"
        );
    }

    /// Passes everything waiting between the two windows and the pretend server until it's quiet.
    fn pump(a: &mut Session, b: &mut Session, srv: &mut Pretend) {
        let (oa, ob) = (a.live_out(), b.live_out());
        srv.take(0, oa);
        srv.take(1, ob);
        for _ in 0..40 {
            let [qa, qb] = std::mem::take(&mut srv.to);
            if qa.is_empty() && qb.is_empty() {
                break;
            }
            for (name, m) in qa {
                let out = a.call(&name, &m).unwrap();
                srv.take(0, out.get("batch").cloned());
                srv.take(0, a.live_out());
            }
            for (name, m) in qb {
                let out = b.call(&name, &m).unwrap();
                srv.take(1, out.get("batch").cloned());
                srv.take(1, b.live_out());
            }
        }
    }

    /// The colour of pixel (x, y) of picture layer `id` as `s` has it saved.
    fn pixel(s: &Session, id: u64, x: u32, y: u32) -> [u8; 4] {
        let l = s.editor.project().layer(xenify_doc::Id(id)).unwrap();
        let xenify_doc::LayerKind::Image { asset, .. } = l.kind else { panic!("not a picture") };
        let a = s.editor.project().asset(asset).unwrap();
        image::load_from_memory(&a.data).unwrap().to_rgba8().get_pixel(x, y).0
    }

    #[test]
    fn two_draws_paint_one_raster_layer() {
        let mut a = Session::new(crate::empty_project());
        a.call("remote_open", &json!({ "server": "s", "node": 4, "name": "Paint", "data": "" })).unwrap();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let mut b = Session::new(crate::empty_project());
        b.call(
            "remote_open",
            &json!({ "server": "s", "node": 4, "name": "Paint", "data": saved["data"], "version": 1 }),
        )
        .unwrap();
        a.call("live_start", &json!({ "seq": 0, "conn": "a", "from": 1000 })).unwrap();
        b.call("live_start", &json!({ "seq": 0, "conn": "b", "from": 2000 })).unwrap();
        let mut srv = Pretend::default();
        let stroke = |s: &mut Session, id: u64, color: &str, y: f64| {
            s.call("raster_begin", &json!({ "id": id, "color": color, "size": 20, "hardness": 1, "pressure": false }))
                .unwrap();
            s.call("raster_paint", &json!({ "points": [[100, y], [300, y]] })).unwrap();
            s.call("raster_end", &json!({})).unwrap();
        };
        // A makes a raster layer and paints; B sees it and paints on the same layer; both end up with both strokes.
        let id = a.call("add_raster_layer", &json!({})).unwrap().as_u64().unwrap();
        stroke(&mut a, id, "#ff0000", 100.0);
        pump(&mut a, &mut b, &mut srv);
        assert_eq!(pixel(&b, id, 200, 100), [255, 0, 0, 255], "B sees A's stroke");
        stroke(&mut b, id, "#0000ff", 200.0);
        pump(&mut a, &mut b, &mut srv);
        for s in [&a, &b] {
            assert_eq!(pixel(s, id, 200, 100), [255, 0, 0, 255]);
            assert_eq!(pixel(s, id, 200, 200), [0, 0, 255, 255]);
        }
        // A's next stroke starts from B's pixels (its working copy follows the change).
        stroke(&mut a, id, "#00ff00", 300.0);
        pump(&mut a, &mut b, &mut srv);
        assert_eq!(pixel(&b, id, 200, 200), [0, 0, 255, 255], "B's stroke is still there after A's");
        assert_eq!(pixel(&b, id, 200, 300), [0, 255, 0, 255]);
        assert!(same_content(a.editor.project(), b.editor.project()));
        // Undo on A takes back only A's own last stroke.
        a.call("undo", &json!({})).unwrap();
        pump(&mut a, &mut b, &mut srv);
        assert_eq!(pixel(&b, id, 200, 300)[3], 0);
        assert_eq!(pixel(&b, id, 200, 200), [0, 0, 255, 255]);
        // Both at the same moment, before either hears of the other: both windows end up the same.
        stroke(&mut a, id, "#ffff00", 400.0);
        stroke(&mut b, id, "#00ffff", 500.0);
        pump(&mut a, &mut b, &mut srv);
        assert!(same_content(a.editor.project(), b.editor.project()));
        // The later one is merged over the earlier: both strokes are there, in both windows.
        for s in [&a, &b] {
            assert_eq!(pixel(s, id, 200, 400), [255, 255, 0, 255], "A's stroke");
            assert_eq!(pixel(s, id, 200, 500), [0, 255, 255, 255], "B's stroke");
            assert_eq!(pixel(s, id, 200, 200), [0, 0, 255, 255]);
        }
    }
}
