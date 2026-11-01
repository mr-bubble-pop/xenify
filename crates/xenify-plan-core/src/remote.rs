//! A plan kept on a Xenify server: where it is, and editing it together. As in Calc
//! (xenify-calc-core/src/remote.rs): the app's server.js moves the bytes,
//! this reads and changes them. The server's copy (the "shadow", every step
//! in the server's order) is kept beside the one on screen, which takes it
//! if adjusting someone's step ever went wrong.

use crate::model::{Plan, Step};
use crate::{Session, file, ops};
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
    /// The plan as the server has it (the opened or last saved version),
    /// while not editing together yet.
    base: Option<Plan>,
}

struct Live {
    client: Client<Step>,
    shadow: Plan,
    shadow_seq: i64,
    queue: BTreeMap<i64, Step>,
    sent: HashMap<String, Vec<Step>>,
    waiting: Option<Batch<Step>>,
}

impl Remote {
    fn encode(&self, c: &Step) -> String {
        serde_json::to_string(c).expect("a step serialises")
    }

    /// A change the server took but this version can't read came from a
    /// newer Plan (a kind of change added since): say so, not serde's words.
    fn decode(&self, data: &str) -> Result<Step, String> {
        serde_json::from_str(data).map_err(|e| {
            if e.is_data() {
                "Someone is working on this with a newer version of Xenify Plan. Update Plan to keep working on it together."
                    .to_string()
            } else {
                e.to_string()
            }
        })
    }

    fn decode_all(&self, ops: &[Json]) -> Result<Vec<(i64, Step)>, String> {
        ops.iter()
            .map(|o| Ok((o["seq"].as_i64().unwrap_or(0), self.decode(o["data"].as_str().unwrap_or_default())?)))
            .collect()
    }

    fn wire(&self, b: Batch<Step>) -> Json {
        json!({ "node": self.node, "client": b.client, "base": b.base, "ops": b.ops.iter().map(|c| self.encode(c)).collect::<Vec<_>>() })
    }
}

/// Someone else's step: applied (what still fits) and kept in History.
fn apply_to(plan: &mut Plan, s: &Step) {
    ops::apply_lenient(plan, &s.ops);
    plan.log.push(s.clone());
}

impl Session {
    pub fn editing_together(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.live.is_some())
    }

    /// A step just made here: kept to send while editing together (or
    /// joining: steps made before it starts go once it has).
    pub(crate) fn made(&mut self, step: Step) {
        if self.remote.is_some() {
            self.outbox.push(step);
        }
    }

    fn apply_remote(&mut self, steps: Vec<Step>) {
        if steps.is_empty() {
            return;
        }
        for s in &steps {
            apply_to(&mut self.plan, s);
        }
        for stack in [&mut self.undo, &mut self.redo] {
            for step in stack.iter_mut() {
                let (moved, _) = xenify_collab::transform(std::slice::from_ref(step), &steps);
                *step = moved.into_iter().next().unwrap_or(Step { ops: Vec::new(), ..step.clone() });
            }
            stack.retain(|s| !s.ops.is_empty());
        }
        self.changes += 1;
    }

    fn settle(&mut self) {
        let Some(r) = self.remote.as_mut() else { return };
        let Some(live) = r.live.as_mut() else { return };
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
            && self.plan.content() != live.shadow.content()
        {
            #[cfg(test)]
            eprintln!("took the server's copy: {:?}", crate::ops::diff(&self.plan, &live.shadow));
            self.plan = live.shadow.clone();
            self.undo.clear();
            self.redo.clear();
            self.changes += 1;
        }
    }

    /// My steps since last time, as a batch to send if one can go now.
    pub fn live_out(&mut self) -> Option<Json> {
        let made: Vec<Step> = std::mem::take(&mut self.outbox);
        let r = self.remote.as_mut()?;
        let live = r.live.as_mut()?;
        let b = match live.waiting.take() {
            Some(b) => {
                if !made.is_empty() {
                    let _ = live.client.local(made);
                }
                Some(b)
            }
            None if made.is_empty() => None,
            None => live.client.local(made),
        }?;
        live.sent.insert(b.client.clone(), b.ops.clone());
        Some(r.wire(b))
    }

    fn fold_outbox(&mut self) {
        if self.outbox.is_empty() {
            return;
        }
        let made: Vec<Step> = std::mem::take(&mut self.outbox);
        if let Some(live) = self.remote.as_mut().and_then(|r| r.live.as_mut())
            && let Some(b) = live.client.local(made)
        {
            live.waiting = Some(b);
        }
    }

    /// Calls for plans on a Xenify server (`remote_…`, `live_…`). None: not one of them.
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
                    let mut remote = Remote {
                        server: s("server"),
                        node: i("node"),
                        name: s("name"),
                        version: i("version"),
                        upto: i("upto"),
                        live: None,
                        base: None,
                    };
                    let data = s("data");
                    if !data.is_empty() {
                        let bytes = b64s.decode(data).map_err(|e| e.to_string())?;
                        self.open_file(&bytes)?;
                    }
                    remote.base = Some(self.plan.clone());
                    self.outbox.clear();
                    self.remote = Some(remote);
                    self.saved = self.changes;
                    Ok(json!({ "node": i("node") }))
                }
                "remote_save" => {
                    let r = self.remote.as_ref().ok_or("this plan isn't on a server")?;
                    let mine = args.get("mine").and_then(Json::as_bool).unwrap_or(false);
                    let plan = if mine { &self.plan } else { r.live.as_ref().map_or(&self.plan, |l| &l.shadow) };
                    let upto = r.live.as_ref().map_or(r.upto, |l| l.client.seq());
                    Ok(
                        json!({ "node": r.node, "base": r.version, "upto": upto, "data": b64s.encode(file::save(plan, self.preview.as_deref())?), "text": words(plan) }),
                    )
                }
                "remote_saved" => {
                    let r = self.remote.as_mut().ok_or("this plan isn't on a server")?;
                    r.version = i("version");
                    if r.live.is_none() {
                        // Saved as it is here: nothing waits to be sent.
                        r.base = Some(self.plan.clone());
                        self.outbox.clear();
                    }
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
                    let r = self.remote.as_ref().ok_or("this plan isn't on a server")?;
                    let caught = r.decode_all(&ops)?;
                    // The server's copy: as opened or saved, plus what came since.
                    let mut shadow = r.base.clone().unwrap_or_else(|| self.plan.clone());
                    for (_, c) in &caught {
                        apply_to(&mut shadow, c);
                    }
                    // Steps made here while joining go after the ones caught up.
                    let mine: Vec<Step> = std::mem::take(&mut self.outbox);
                    let theirs: Vec<Step> = caught.iter().map(|(_, c)| c.clone()).collect();
                    let (mine, theirs) = xenify_collab::transform(&mine, &theirs);
                    if !theirs.is_empty() {
                        for c in &theirs {
                            apply_to(&mut self.plan, c);
                        }
                    }
                    let seq = i("seq");
                    let r = self.remote.as_mut().unwrap();
                    r.base = None;
                    r.live = Some(Live {
                        client: Client::new(seq, &s("conn")),
                        shadow,
                        shadow_seq: seq,
                        queue: BTreeMap::new(),
                        sent: HashMap::new(),
                        waiting: None,
                    });
                    self.outbox = mine;
                    self.changes += 1;
                    self.saved = self.changes;
                    Ok(json!({ "live": true }))
                }
                "live_ops" => {
                    self.fold_outbox();
                    let r = self.remote.as_mut().ok_or("this plan isn't on a server")?;
                    let incoming = r.decode_all(args.get("ops").and_then(Json::as_array).map_or(&[][..], |v| v))?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let mut apply = Vec::new();
                    for (seq, c) in incoming {
                        live.queue.insert(seq, c.clone());
                        apply.extend(live.client.remote(seq, c));
                    }
                    self.apply_remote(apply);
                    self.settle();
                    Ok(json!({ "changed": true }))
                }
                "live_accepted" | "live_behind" => {
                    self.fold_outbox();
                    let client = s("client");
                    let r = self.remote.as_mut().ok_or("this plan isn't on a server")?;
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
                            .map_err(|_| "a change went missing: open the plan again")?
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
                    self.settle();
                    Ok(json!({ "batch": next }))
                }
                "live_resend" => {
                    let r = self.remote.as_mut().ok_or("this plan isn't on a server")?;
                    let live = r.live.as_mut().ok_or("not editing together")?;
                    let b = live.client.resend();
                    if let Some(b) = &b {
                        live.sent.insert(b.client.clone(), b.ops.clone());
                    }
                    Ok(json!({ "batch": b.map(|b| r.wire(b)) }))
                }
                // Plan's ids are random: no ranges to lease.
                "live_ids" => Ok(json!({ "left": i64::MAX, "next": 1 })),
                "live_lease" => Ok(Json::Null),
                "live_stop" => {
                    if let Some(r) = self.remote.as_mut() {
                        r.live = None;
                    }
                    Ok(Json::Null)
                }
                // Which task I'm on, for the others; and theirs.
                "live_say" => {
                    let r = self.remote.as_ref().ok_or("this plan isn't on a server")?;
                    let at = json!({ "task": args["task"], "view": args["view"] });
                    Ok(json!({ "node": r.node, "data": at.to_string() }))
                }
                "live_heard" => {
                    self.remote.as_ref().ok_or("this plan isn't on a server")?;
                    let at: Json = serde_json::from_str(&s("data")).map_err(|e| e.to_string())?;
                    Ok(at)
                }
                _ => Err(format!("unknown call: {name}")),
            }
        })())
    }
}

/// The plan's words: its title and each task's title and description, for
/// the server's search.
fn words(plan: &Plan) -> String {
    let mut out = vec![plan.title.clone()];
    for t in plan.tasks.values() {
        out.push(t.title.clone());
        out.push(crate::text_of(&t.description));
    }
    out.join("\n")
}
