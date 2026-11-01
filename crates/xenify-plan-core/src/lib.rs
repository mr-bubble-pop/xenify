//! Xenify Plan's engine. One [`Session`] is one open plan: its tasks, edit
//! history and saved state. The screens talk to it with [`Session::call`]
//! (a name and JSON arguments, JSON back), the same on the desktop and in
//! the browser; so do the built-in AI and programs over MCP.
//!
//! Every change is a [`model::Step`] of [`ops::Op`]s, kept in the plan's
//! log (History), undone exactly, and sent as one unit when editing
//! together. Dates are never stored: [`schedule`] works them out.

pub mod access;
pub mod collab;
pub mod fields;
pub mod file;
pub mod history;
pub mod load;
pub mod model;
pub mod ops;
pub mod remote;
pub mod schedule;

#[cfg(test)]
mod tests;

pub use model::{Comment, Dep, DepKind, Id, Outside, Plan, Resource, ResourceKind, Step, Task, is_outside};
pub use ops::Op;
pub use schedule::{Booking, Waits, World, waits_of};
pub use xenify_holidays::Date;

use serde_json::{Value as Json, json};
use std::collections::HashMap;

pub struct Session {
    pub plan: Plan,
    pub world: World,
    /// Steps that undo what was done (newest last), and their redo.
    undo: Vec<Step>,
    redo: Vec<Step>,
    pub changes: u64,
    pub saved: u64,
    /// Who is making the changes (History), and the time now (Unix ms).
    pub me: String,
    /// The signed-in person's resource id (My work).
    pub me_resource: String,
    pub now: i64,
    seed: u64,
    pub(crate) remote: Option<remote::Remote>,
    /// Steps made here, to send while editing together.
    pub(crate) outbox: Vec<Step>,
    pub preview: Option<Vec<u8>>,
    /// While several edits are made as one (group_start … group_end): where
    /// the undo list and the history stood.
    group: Option<(usize, usize)>,
    /// What the server lets me change (access.rs); None: everything.
    pub may: Option<Vec<String>>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

fn arg_s(args: &Json, k: &str) -> String {
    args.get(k).and_then(Json::as_str).unwrap_or_default().to_string()
}

fn arg_date(args: &Json, k: &str) -> Option<Date> {
    args.get(k).and_then(Json::as_str).and_then(Date::parse)
}

fn arg_strings(args: &Json, k: &str) -> Vec<String> {
    args.get(k)
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(Json::as_str).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// The words of a description or comment (its tags taken out).
pub fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                out.push(' ');
            }
            c if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

impl Session {
    pub fn new() -> Session {
        let mut s = Session {
            plan: Plan::default(),
            world: World::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            changes: 0,
            saved: 0,
            me: String::new(),
            me_resource: String::new(),
            now: 0,
            seed: 0x9E37_79B9_7F4A_7C15,
            remote: None,
            outbox: Vec::new(),
            preview: None,
            group: None,
            may: None,
        };
        s.world.today = Date::new(2026, 1, 1).unwrap();
        s
    }

    /// A new plan with the usual statuses and types, starting `today`.
    pub fn fresh(&mut self, title: &str) {
        let mut p = Plan::starter(title);
        p.settings.start = Some(self.world.today);
        let step = history::first_step(&p, self.now, &self.me, "Made the plan");
        p.log = vec![step];
        self.plan = p;
        self.undo.clear();
        self.redo.clear();
        self.changes += 1;
        self.saved = self.changes;
    }

    pub fn open_file(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut p = file::open(bytes)?;
        if p.log.is_empty() {
            let step = history::first_step(&p, self.now, &self.me, "Opened the plan");
            p.log.push(step);
        }
        self.plan = p;
        self.preview = file::preview(bytes);
        self.undo.clear();
        self.redo.clear();
        self.changes += 1;
        self.saved = self.changes;
        Ok(())
    }

    /// The .xplan file. Its list of changes (History, who did what) only
    /// goes with plans on a server: a plan kept as a file has no audit.
    pub fn save_file(&self) -> Result<Vec<u8>, String> {
        if self.remote.is_some() {
            return file::save(&self.plan, self.preview.as_deref());
        }
        let mut p = self.plan.clone();
        p.log.clear();
        file::save(&p, self.preview.as_deref())
    }

    /// A new id no other copy will make (the app seeds it with something random).
    pub fn new_id(&mut self, prefix: &str) -> String {
        loop {
            // xorshift64*
            let mut x = self.seed;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.seed = x;
            let n = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
            let id = format!("{prefix}{}", to36(n));
            let used = self.plan.tasks.contains_key(&id)
                || self.plan.resources.iter().any(|r| r.id == id)
                || self.plan.projects.iter().any(|r| r.id == id);
            if !used {
                return id;
            }
        }
    }

    /// Whether the server lets me make these changes.
    fn allowed(&self, ops: &[Op]) -> Result<(), String> {
        match self.may.as_deref().and_then(|m| access::refused(ops, m)) {
            Some(p) => Err(access::message(p)),
            None => Ok(()),
        }
    }

    /// Does `ops` as one step called `what` (History, Undo).
    pub fn step(&mut self, what: &str, ops: Vec<Op>) -> Result<(), String> {
        if ops.is_empty() {
            return Ok(());
        }
        self.allowed(&ops)?;
        let undo = ops::apply_all(&mut self.plan, &ops)?;
        let step = Step { at: self.now, who: self.me.clone(), what: what.to_string(), ops };
        self.plan.log.push(step.clone());
        self.made(step);
        self.undo.push(Step { at: 0, who: String::new(), what: what.to_string(), ops: undo });
        self.redo.clear();
        self.changes += 1;
        Ok(())
    }

    fn undo_redo(&mut self, undo: bool) -> Result<(), String> {
        let from = if undo { &mut self.undo } else { &mut self.redo };
        let Some(s) = from.last() else { return Ok(()) };
        if let Some(p) = self.may.as_deref().and_then(|m| access::refused(&s.ops, m)) {
            return Err(access::message(p));
        }
        let Some(s) = from.pop() else { return Ok(()) };
        let back = ops::apply_all(&mut self.plan, &s.ops)?;
        let what = if undo { format!("Undid: {}", s.what) } else { s.what.clone() };
        let step = Step { at: self.now, who: self.me.clone(), what, ops: s.ops.clone() };
        self.plan.log.push(step.clone());
        self.made(step);
        let to = if undo { &mut self.redo } else { &mut self.undo };
        to.push(Step { at: 0, who: String::new(), what: s.what, ops: back });
        self.changes += 1;
        Ok(())
    }

    pub fn schedule(&self) -> schedule::Schedule {
        schedule::schedule(&self.plan, &self.world)
    }

    /// The task's position for "add after": after everything inside it.
    fn before_after(&self, after: &str) -> Option<String> {
        let i = self.plan.order.iter().position(|x| x == after)?;
        let mut j = i + 1;
        while j < self.plan.order.len() && self.plan.within(&self.plan.order[j], after) {
            j += 1;
        }
        self.plan.order.get(j).cloned()
    }

    /// The task and everything inside it, in order.
    fn subtree(&self, id: &str) -> Vec<String> {
        self.plan.order.iter().filter(|x| self.plan.within(x, id)).cloned().collect()
    }

    /// Everyone a person field can name.
    fn people_ids(&self) -> Vec<&str> {
        self.plan.resources.iter().chain(self.world.resources.iter()).map(|r| r.id.as_str()).collect()
    }

    fn names(&self) -> HashMap<String, (String, ResourceKind)> {
        let cal = schedule::Calendar::new(&self.plan, &self.world);
        let mut out = HashMap::new();
        for r in self.plan.resources.iter().chain(self.world.resources.iter()) {
            let r = cal.resource(&r.id).unwrap_or(r);
            out.insert(r.id.clone(), (r.name.clone(), r.kind));
        }
        out
    }

    fn task_title(&self, id: &str) -> String {
        self.plan.task(id).map(|t| t.title.clone()).unwrap_or_default()
    }

    /// Everything the screens draw from.
    pub fn view(&self) -> Json {
        let s = self.schedule();
        let names = self.names();
        // When each issue was made and last changed (from History).
        let mut made: HashMap<&str, (i64, i64, &str)> = HashMap::new();
        for st in &self.plan.log {
            for op in &st.ops {
                if let Some(id) = op.task() {
                    let e = made.entry(id).or_insert((st.at, st.at, st.who.as_str()));
                    e.1 = st.at;
                }
            }
        }
        let mut tasks = Vec::new();
        let (mut lo, mut hi): (Option<Date>, Option<Date>) = (None, None);
        for (rank, id) in self.plan.order.iter().enumerate() {
            let Some(t) = self.plan.tasks.get(id) else { continue };
            let d = s.tasks.get(id).cloned().unwrap_or_default();
            if d.placed {
                lo = Some(lo.map_or(d.start, |x: Date| x.min(d.start)));
                hi = Some(hi.map_or(d.finish, |x: Date| x.max(d.finish)));
            }
            let after: Vec<&Dep> =
                t.after.iter().filter(|d| self.plan.tasks.contains_key(&d.task) || is_outside(&d.task)).collect();
            let blocks: Vec<&String> = self
                .plan
                .order
                .iter()
                .filter(|x| self.plan.tasks.get(*x).is_some_and(|o| o.after.iter().any(|d| d.task == *id)))
                .collect();
            let (created, updated, by) = made.get(id.as_str()).copied().unwrap_or((0, 0, ""));
            // Its dates when they were last kept as the expected ones.
            let expected = self.plan.baselines.last().and_then(|b| b.tasks.get(id));
            tasks.push(json!({
                "id": id, "key": t.key, "title": t.title, "kind": t.kind, "status": t.status, "projects": t.projects,
                "parent": t.parent, "depth": self.plan.depth(id), "rank": rank, "epic": self.plan.is_epic(t), "summary": d.summary,
                "milestone": d.milestone, "placed": d.placed, "start": d.start, "finish": d.finish, "pieces": d.pieces, "days": d.days,
                "late": d.late, "late_by": d.late_by, "waits": d.waits, "warnings": d.warnings, "clashes": d.clashes, "overbook": t.overbook, "release": t.release, "mirror": t.mirror, "done": d.done, "work": if t.mirror.is_some() { None } else { t.work }, "total": d.work,
            }));
            let more = json!({
                "held": t.start, "due": t.due, "who": t.who, "who_name": t.who.as_ref().and_then(|w| names.get(w)).map(|x| x.0.clone()),
                "who_group": t.who.as_ref().and_then(|w| names.get(w)).is_some_and(|x| x.1 == ResourceKind::Group),
                "by": d.by, "by_name": d.by.as_ref().filter(|b| Some(*b) != t.who.as_ref()).and_then(|w| names.get(w)).map(|x| x.0.clone()),
                "priority": if t.priority.is_empty() { "medium" } else { t.priority.as_str() },
                "started": t.started, "finished": t.finished, "after": after, "blocks": blocks, "relates": t.relates, "duplicates": t.duplicates,
                "fields": t.fields, "labels": t.labels, "missing": fields::missing(&self.plan, t), "description": t.description, "comments": t.comments,
                "created": created, "updated": updated, "created_by": by, "expected": expected,
                "is_done": self.plan.is_done(t), "is_started": self.plan.is_started(t),
            });
            if let (Some(o), Json::Object(m)) = (tasks.last_mut().and_then(Json::as_object_mut), more) {
                o.extend(m);
            }
        }
        let today = self.world.today;
        let from = lo.unwrap_or(today).min(today).add(-7);
        let to = hi.unwrap_or(today).max(today).add(28);
        let cal = schedule::Calendar::new(&self.plan, &self.world);
        let mut off = Vec::new();
        let mut d = from;
        while d <= to {
            if let Some(n) = cal.day_off(d) {
                off.push(json!({ "date": d, "name": n }));
            }
            d = d.add(1);
        }
        let mut resources: Vec<Json> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in self.world.resources.iter().chain(self.plan.resources.iter()) {
            if r.kind == ResourceKind::Person && seen.insert(r.id.clone()) {
                resources.push(serde_json::to_value(r).unwrap_or_default());
            }
        }
        // Groups: an issue given to one goes to whichever member is free first.
        let mut groups: Vec<Json> = Vec::new();
        for r in self.world.resources.iter().chain(self.plan.resources.iter()) {
            if r.kind == ResourceKind::Group && seen.insert(r.id.clone()) {
                let members: Vec<Json> = r.members.iter().map(|m| json!({ "person": m.person, "name": names.get(&m.person).map(|x| x.0.clone()), "from": m.from, "to": m.to })).collect();
                groups.push(json!({ "id": r.id, "name": r.name, "members": members }));
            }
        }
        let mut labels: Vec<(String, usize)> = Vec::new();
        for t in self.plan.tasks.values() {
            for l in &t.labels {
                match labels.iter_mut().find(|x| x.0.eq_ignore_ascii_case(l)) {
                    Some(x) => x.1 += 1,
                    None => labels.push((l.clone(), 1)),
                }
            }
        }
        labels.sort_by_key(|x| x.0.to_lowercase());
        // Issues in other plans that issues here wait for, as the server (or an exported copy) has them.
        let mut outside = serde_json::Map::new();
        for t in self.plan.tasks.values() {
            for d in t.after.iter().filter(|d| is_outside(&d.task)) {
                let o = self.world.outside.get(&d.task).or_else(|| self.plan.frozen.get(&d.task)).cloned();
                outside.insert(
                    d.task.clone(),
                    o.map_or(json!({ "known": false }), |o| {
                        let mut v = json!(o);
                        v["known"] = json!(true);
                        v
                    }),
                );
            }
        }
        // What mirrors here show.
        let whole: Vec<String> = self
            .plan
            .tasks
            .values()
            .filter_map(|t| t.mirror.clone())
            .collect();
        for r in whole {
            let o = self.world.outside.get(&r).or_else(|| self.plan.frozen.get(&r)).cloned();
            outside.insert(r, o.map_or(json!({ "known": false }), |o| {
                let mut v = json!(o);
                v["known"] = json!(true);
                v
            }));
        }
        json!({
            "audit": self.remote.is_some(),
            // What I may change (null: everything): screens hide the rest.
            "may": self.may.clone().unwrap_or_else(access::all),
            "outside": outside,
            "on_server": self.remote.is_some(),
            "labels": labels.iter().map(|(n, c)| json!({ "name": n, "count": c })).collect::<Vec<_>>(),
            "field_kinds": fields::KINDS.iter().map(|(k, n)| json!({ "id": k, "name": n })).collect::<Vec<_>>(),
            // When the expected dates were kept (Timeline › Expected dates), and by whom.
            "expected": self.plan.baselines.last().map(|b| json!({ "saved": b.saved, "by": b.by })),
            "title": self.plan.title, "settings": self.plan.settings, "projects": self.plan.projects, "types": self.plan.types,
            "statuses": self.plan.statuses, "releases": self.plan.releases, "fields": self.plan.fields, "resources": resources, "groups": groups, "tasks": tasks,
            "today": today, "from": from, "to": to, "days_off": off,
            "undo": self.undo.last().map(|s| s.what.clone()), "redo": self.redo.last().map(|s| s.what.clone()),
            "changed": self.changes != self.saved, "steps": self.plan.log.len(),
            // Who I am in this plan: as chosen, else the person with my name.
            "me": self.me, "me_resource": if self.me_resource.is_empty() {
                resources.iter().find(|r| r["name"].as_str() == Some(self.me.as_str()) && !self.me.is_empty()).map(|r| r["id"].clone()).unwrap_or(json!(""))
            } else { json!(self.me_resource) },
            "holidays_from": if self.plan.settings.country.is_empty() && !self.world.default_country.is_empty() {
                json!({ "country": self.world.default_country, "region": self.world.default_region })
            } else { Json::Null },
        })
    }

    /// One screen call: a name and JSON arguments, JSON back.
    pub fn call(&mut self, name: &str, args: &Json) -> Result<Json, String> {
        if let Some(r) = self.remote_call(name, args) {
            return r;
        }
        match name {
            "view" => Ok(self.view()),
            "set_clock" => {
                if let Some(d) = arg_date(args, "today") {
                    self.world.today = d;
                }
                if let Some(n) = args.get("now").and_then(Json::as_i64) {
                    self.now = n;
                }
                if let Some(seed) = args.get("seed").and_then(Json::as_u64) {
                    self.seed = seed | 1;
                }
                Ok(Json::Null)
            }
            // What the server lets me change here (null: everything).
            "set_access" => {
                self.may = match args.get("may") {
                    Some(Json::Array(a)) => Some(a.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
                    _ => None,
                };
                Ok(Json::Null)
            }
            "set_me" => {
                self.me = arg_s(args, "name");
                self.me_resource = arg_s(args, "resource");
                Ok(Json::Null)
            }
            "set_world" => {
                if let Some(v) = args.get("resources") {
                    self.world.resources = serde_json::from_value(v.clone()).map_err(|e| format!("resources: {e}"))?;
                }
                if let Some(v) = args.get("bookings") {
                    self.world.bookings = serde_json::from_value(v.clone()).map_err(|e| format!("bookings: {e}"))?;
                }
                if let Some(v) = args.get("holidays") {
                    self.world.default_country = v["country"].as_str().unwrap_or_default().to_string();
                    self.world.default_region = v["region"].as_str().unwrap_or_default().to_string();
                }
                if let Some(v) = args.get("company_days") {
                    self.world.company_days =
                        serde_json::from_value(v.clone()).map_err(|e| format!("company days: {e}"))?;
                }
                if let Some(v) = args.get("outside") {
                    self.world.outside = serde_json::from_value(v.clone()).map_err(|e| format!("outside: {e}"))?;
                }
                Ok(Json::Null)
            }
            "new" => {
                self.fresh(&arg_s(args, "title"));
                Ok(Json::Null)
            }
            "save_file" => {
                use base64::Engine;
                Ok(json!({ "data": base64::engine::general_purpose::STANDARD.encode(self.save_file()?) }))
            }
            "open_file" => {
                use base64::Engine;
                let bytes =
                    base64::engine::general_purpose::STANDARD.decode(arg_s(args, "data")).map_err(|e| e.to_string())?;
                self.open_file(&bytes)?;
                Ok(Json::Null)
            }
            "set_preview" => {
                use base64::Engine;
                let d = arg_s(args, "data");
                self.preview = if d.is_empty() {
                    None
                } else {
                    Some(base64::engine::general_purpose::STANDARD.decode(d).map_err(|e| e.to_string())?)
                };
                Ok(Json::Null)
            }
            "saved" => {
                self.saved = self.changes;
                Ok(Json::Null)
            }
            // Not saved anywhere yet (a plan recovered after a crash).
            "changed" => {
                self.changes += 1;
                Ok(Json::Null)
            }
            // Several edits as one step (AI's Apply, a program's changes): one Undo, one line in History.
            "group_start" => {
                self.group = Some((self.undo.len(), self.plan.log.len()));
                Ok(Json::Null)
            }
            "group_end" => {
                let Some((u, l)) = self.group.take() else { return Ok(Json::Null) };
                let what = arg_s(args, "what");
                if self.undo.len() > u + 1 {
                    let mut ops = Vec::new();
                    for s in self.undo.drain(u..).rev() {
                        ops.extend(s.ops);
                    }
                    self.undo.push(Step { at: 0, who: String::new(), what: what.clone(), ops });
                }
                if self.plan.log.len() > l + 1 {
                    let mut ops = Vec::new();
                    for s in self.plan.log.drain(l..) {
                        ops.extend(s.ops);
                    }
                    self.plan.log.push(Step { at: self.now, who: self.me.clone(), what: what.clone(), ops });
                } else if let Some(s) = self.plan.log.get_mut(l).filter(|_| !what.is_empty()) {
                    s.what = what;
                }
                Ok(Json::Null)
            }
            "undo" => self.undo_redo(true).map(|_| Json::Null),
            "redo" => self.undo_redo(false).map(|_| Json::Null),
            "add_task" => {
                let id = self.new_id("t");
                let project = args
                    .get("project")
                    .and_then(Json::as_str)
                    .map(String::from)
                    .or_else(|| self.plan.projects.first().map(|p| p.id.clone()));
                let prefix = project
                    .as_ref()
                    .and_then(|p| self.plan.projects.iter().find(|x| &x.id == p))
                    .map_or("T".to_string(), |p| p.prefix.clone());
                let parent = args
                    .get("parent")
                    .and_then(Json::as_str)
                    .filter(|p| self.plan.tasks.contains_key(*p))
                    .map(String::from);
                let before = match args.get("after").and_then(Json::as_str) {
                    Some(a) => self.before_after(a),
                    None => match args.get("before").and_then(Json::as_str) {
                        Some(b) => Some(b.to_string()),
                        // In an epic: after the epic's last issue (so its place in the backlog is with them).
                        None => parent.as_ref().and_then(|p| self.before_after(p)),
                    },
                };
                let kind = args.get("kind").and_then(Json::as_str).map(String::from).unwrap_or_else(|| "task".into());
                let milestone = kind == "milestone" || args.get("milestone").and_then(Json::as_bool).unwrap_or(false);
                let status = args
                    .get("status")
                    .and_then(Json::as_str)
                    .map(String::from)
                    .or_else(|| self.plan.statuses.first().map(|t| t.id.clone()))
                    .unwrap_or_default();
                let after: Vec<Dep> = args
                    .get("waits_for")
                    .and_then(Json::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Json::as_str)
                            .filter(|x| self.plan.tasks.contains_key(*x))
                            .map(|x| Dep { task: x.to_string(), ..Dep::default() })
                            .collect()
                    })
                    .unwrap_or_default();
                let kind_id = if milestone { "milestone".to_string() } else { kind.clone() };
                let mut values = std::collections::BTreeMap::new();
                for f in &self.plan.fields {
                    if !f.types.is_empty() && !f.types.contains(&kind_id) {
                        continue;
                    }
                    let given =
                        args.get("fields").and_then(|x| x.get(&f.id)).cloned().unwrap_or_else(|| f.default.clone());
                    let v = fields::check(f, &given, self.world.today, &self.people_ids())?;
                    if !v.is_null() {
                        values.insert(f.id.clone(), v);
                    }
                }
                let started =
                    self.plan.status(&status).is_some_and(|s| s.started || s.done).then_some(self.world.today);
                let finished = self.plan.status(&status).is_some_and(|s| s.done).then_some(self.world.today);
                let task = Task {
                    id: id.clone(),
                    key: self.plan.next_key(&prefix),
                    title: arg_s(args, "title"),
                    kind: if milestone { "milestone".into() } else { kind },
                    status,
                    projects: project.into_iter().collect(),
                    parent,
                    description: arg_s(args, "description"),
                    milestone,
                    // No estimate given: the plan's usual one (Plan settings), so
                    // every issue is on the Timeline from the start.
                    work: if milestone || kind_id == "epic" {
                        None
                    } else {
                        args.get("work").map_or_else(
                            || {
                                let d = self.plan.settings.default_days;
                                (d > 0.0).then(|| (d * self.plan.settings.day_minutes as f64).round() as u32)
                            },
                            |w| w.as_u64().map(|n| n as u32),
                        )
                    },
                    who: args.get("who").and_then(Json::as_str).filter(|x| !x.is_empty()).map(String::from),
                    priority: args.get("priority").and_then(Json::as_str).unwrap_or("medium").to_string(),
                    due: arg_date(args, "due"),
                    labels: tidy_labels(arg_strings(args, "labels"), &self.plan),
                    fields: values,
                    after,
                    started,
                    finished,
                    ..Task::default()
                };
                let what = format!(
                    "Added {}",
                    if task.title.is_empty() { "an issue".to_string() } else { format!("{} {}", task.key, task.title) }
                );
                let key = task.key.clone();
                self.step(&what, vec![Op::AddTask { task: Box::new(task), before }])?;
                Ok(json!({ "id": id, "key": key }))
            }
            "set_task" => {
                let id = arg_s(args, "id");
                let field = arg_s(args, "field");
                let value = args.get("value").cloned().unwrap_or(Json::Null);
                let title = self.task_title(&id);
                let what = args
                    .get("what")
                    .and_then(Json::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| self.describe(&title, &field, &value));
                let mut ops = vec![Op::SetTask { id: id.clone(), field: field.clone(), value: value.clone() }];
                // A status says when work started and finished (done tasks keep their dates on the Timeline).
                if field == "status"
                    && let (Some(t), Some(st)) = (self.plan.task(&id), value.as_str().and_then(|v| self.plan.status(v)))
                {
                    let today = json!(self.world.today);
                    if (st.started || st.done) && t.started.is_none() {
                        ops.push(Op::SetTask { id: id.clone(), field: "started".into(), value: today.clone() });
                    }
                    if !st.started && !st.done && t.started.is_some() {
                        ops.push(Op::SetTask { id: id.clone(), field: "started".into(), value: Json::Null });
                    }
                    if st.done && t.finished.is_none() {
                        ops.push(Op::SetTask { id: id.clone(), field: "finished".into(), value: today });
                    }
                    if !st.done && t.finished.is_some() {
                        ops.push(Op::SetTask { id: id.clone(), field: "finished".into(), value: Json::Null });
                    }
                    // Not done after all: back from its release onto the Board.
                    if !st.done && t.release.is_some() {
                        ops.push(Op::SetTask { id: id.clone(), field: "release".into(), value: Json::Null });
                    }
                }
                self.step(&what, ops)?;
                Ok(Json::Null)
            }
            // Every issue in another plan that this one waits for (to ask the server about).
            "outside_refs" => {
                let mut refs: Vec<String> = self
                    .plan
                    .tasks
                    .values()
                    .flat_map(|t| t.after.iter())
                    .map(|d| d.task.clone())
                    .filter(|x| is_outside(x))
                    .chain(self.plan.tasks.values().filter_map(|t| t.mirror.clone()))
                    .collect();
                refs.sort();
                refs.dedup();
                Ok(json!(refs))
            }
            // A copy to keep or send: the plan as it is, with the names of its
            // people and the issues it waits for in other plans as they are now.
            "export_copy" => {
                let mut p = self.plan.clone();
                p.log.clear();
                let used: std::collections::HashSet<&str> = p.tasks.values().filter_map(|t| t.who.as_deref()).collect();
                for r in &self.world.resources {
                    if used.contains(r.id.as_str()) && !p.resources.iter().any(|x| x.id == r.id) {
                        p.resources.push(r.clone());
                    }
                }
                for (k, o) in &self.world.outside {
                    if p.tasks.values().any(|t| t.mirror.as_ref() == Some(k) || t.after.iter().any(|d| &d.task == k)) {
                        p.frozen.insert(k.clone(), o.clone());
                    }
                }
                // The server's holidays and the time its people are busy elsewhere, as today.
                if p.settings.country.is_empty() {
                    p.settings.country = self.world.default_country.clone();
                    p.settings.region = self.world.default_region.clone();
                }
                p.frozen_days.extend(self.world.company_days.iter().cloned());
                let mut groups_used: Vec<&str> = Vec::new();
                for r in &self.world.resources {
                    if used.contains(r.id.as_str()) && r.kind == ResourceKind::Group {
                        groups_used.extend(r.members.iter().map(|m| m.person.as_str()));
                    }
                }
                for r in &self.world.resources {
                    if groups_used.contains(&r.id.as_str()) && !p.resources.iter().any(|x| x.id == r.id) {
                        p.resources.push(r.clone());
                    }
                }
                let people: std::collections::HashSet<String> = p.resources.iter().map(|r| r.id.clone()).collect();
                p.frozen_bookings
                    .extend(self.world.bookings.iter().filter(|b| b.first && people.contains(&b.resource)).cloned());
                let bytes = file::save(&p, self.preview.as_deref())?;
                Ok(json!(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)))
            }
            // One of the plan's own fields on an issue, checked against the field's rules.
            "set_field" => {
                let id = arg_s(args, "id");
                let fid = arg_s(args, "field");
                let f = self.plan.fields.iter().find(|f| f.id == fid).ok_or("no such field")?.clone();
                let t = self.plan.task(&id).ok_or("no such issue")?;
                let v =
                    fields::check(&f, args.get("value").unwrap_or(&Json::Null), self.world.today, &self.people_ids())?;
                let mut map = t.fields.clone();
                if v.is_null() {
                    map.remove(&fid)
                } else {
                    map.insert(fid, v.clone())
                };
                let shown = match &v {
                    Json::Null => "nothing".to_string(),
                    Json::String(s) => s.clone(),
                    Json::Array(a) => a.iter().filter_map(Json::as_str).collect::<Vec<_>>().join(", "),
                    x => x.to_string(),
                };
                let what = format!("{}: {} {shown}", t.title, f.name);
                self.step(&what, vec![Op::SetTask { id, field: "fields".into(), value: json!(map) }])?;
                Ok(Json::Null)
            }
            "set_labels" => {
                let id = arg_s(args, "id");
                let t = self.plan.task(&id).ok_or("no such issue")?;
                let labels = tidy_labels(arg_strings(args, "labels"), &self.plan);
                let what = if labels.is_empty() {
                    format!("{}: no labels", t.title)
                } else {
                    format!("{}: labels {}", t.title, labels.join(", "))
                };
                self.step(&what, vec![Op::SetTask { id, field: "labels".into(), value: json!(labels) }])?;
                Ok(Json::Null)
            }
            // A label renamed (or, with no new name, taken off) on every issue.
            "rename_label" => {
                let from = arg_s(args, "from");
                let to = arg_s(args, "to").trim().to_string();
                let mut ops = Vec::new();
                for t in self.plan.tasks.values() {
                    if !t.labels.iter().any(|l| l.eq_ignore_ascii_case(&from)) {
                        continue;
                    }
                    let mut next: Vec<String> =
                        t.labels.iter().filter(|l| !l.eq_ignore_ascii_case(&from)).cloned().collect();
                    if !to.is_empty() && !next.iter().any(|l| l.eq_ignore_ascii_case(&to)) {
                        next.push(to.clone());
                    }
                    ops.push(Op::SetTask { id: t.id.clone(), field: "labels".into(), value: json!(next) });
                }
                let what = if to.is_empty() {
                    format!("Took the label {from} off everything")
                } else {
                    format!("Renamed the label {from} to {to}")
                };
                self.step(&what, ops)?;
                Ok(Json::Null)
            }
            // Held back: it starts no earlier than `start` (None lets it start as soon as it can).
            "hold" => {
                let id = arg_s(args, "id");
                // A day off (a weekend, a holiday) moves on to the next working day.
                let to =
                    arg_date(args, "start").map(|d| schedule::Calendar::new(&self.plan, &self.world).on_or_after(d));
                let title = self.task_title(&id);
                let what = match to {
                    Some(d) => format!("{title} starts no sooner than {}", schedule::day_name(d)),
                    None => format!("{title} starts as soon as it can"),
                };
                self.step(&what, vec![Op::SetTask { id, field: "start".into(), value: json!(to) }])?;
                Ok(Json::Null)
            }
            "add_comment" | "set_comment" | "delete_comment" => {
                let id = arg_s(args, "id");
                let t = self.plan.task(&id).ok_or("no such issue")?;
                let mut list = t.comments.clone();
                let title = t.title.clone();
                let what = match name {
                    "add_comment" => {
                        let cid = self.new_id("c");
                        list.push(model::Comment {
                            id: cid,
                            at: self.now,
                            who: self.me.clone(),
                            by: self.me_resource.clone(),
                            html: arg_s(args, "html"),
                        });
                        format!("Commented on {title}")
                    }
                    "set_comment" => {
                        let c = list.iter_mut().find(|c| c.id == arg_s(args, "comment")).ok_or("no such comment")?;
                        c.html = arg_s(args, "html");
                        format!("Changed a comment on {title}")
                    }
                    _ => {
                        list.retain(|c| c.id != arg_s(args, "comment"));
                        format!("Deleted a comment on {title}")
                    }
                };
                self.step(
                    &what,
                    vec![Op::SetTask { id, field: "comments".into(), value: serde_json::to_value(list).unwrap() }],
                )?;
                Ok(Json::Null)
            }
            "set_tasks" => {
                let changes = args.get("changes").and_then(Json::as_array).cloned().unwrap_or_default();
                let ops: Vec<Op> = changes
                    .iter()
                    .map(|c| Op::SetTask {
                        id: arg_s(c, "id"),
                        field: arg_s(c, "field"),
                        value: c.get("value").cloned().unwrap_or(Json::Null),
                    })
                    .collect();
                let what = args
                    .get("what")
                    .and_then(Json::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| format!("Changed {} tasks", ops.len()));
                self.step(&what, ops)?;
                Ok(Json::Null)
            }
            "delete_tasks" => {
                let ids: Vec<String> = args
                    .get("ids")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                // keep: an epic goes but what's in it stays, moved up to
                // where the epic was (no epic, or the epic around it).
                let keep = args.get("keep").and_then(Json::as_bool).unwrap_or(false);
                let mut all: Vec<String> = Vec::new();
                let mut ops = Vec::new();
                if keep {
                    all = ids.iter().filter(|id| self.plan.tasks.contains_key(*id)).cloned().collect();
                    for t in self.plan.order.iter().filter_map(|x| self.plan.tasks.get(x)) {
                        let Some(p) = &t.parent else { continue };
                        if all.contains(&t.id) || !all.contains(p) {
                            continue;
                        }
                        // The nearest epic around it that isn't going.
                        let mut up = self.plan.tasks.get(p).and_then(|x| x.parent.clone());
                        while let Some(u) = up.clone().filter(|u| all.contains(u)) {
                            up = self.plan.tasks.get(&u).and_then(|x| x.parent.clone());
                        }
                        ops.push(Op::SetTask { id: t.id.clone(), field: "parent".into(), value: json!(up) });
                    }
                } else {
                    for id in &ids {
                        for x in self.subtree(id) {
                            if !all.contains(&x) {
                                all.push(x);
                            }
                        }
                    }
                }
                if all.is_empty() {
                    return Ok(Json::Null);
                }
                let what = match (all.len(), keep && !ops.is_empty()) {
                    (1, true) => format!("Deleted {}, keeping what was in it", self.task_title(&all[0])),
                    (1, false) => format!("Deleted {}", self.task_title(&all[0])),
                    (n, _) => format!("Deleted {n} tasks"),
                };
                ops.extend(all.iter().rev().map(|id| Op::DeleteTask { id: id.clone() }));
                self.step(&what, ops)?;
                Ok(Json::Null)
            }
            "indent" | "outdent" => {
                let id = arg_s(args, "id");
                let t = self.plan.task(&id).ok_or("no such task")?.clone();
                let i = self.plan.order.iter().position(|x| *x == id).ok_or("no such task")?;
                if name == "indent" {
                    let prev = self.plan.order[..i]
                        .iter()
                        .rev()
                        .find(|x| self.plan.tasks.get(*x).is_some_and(|p| p.parent == t.parent))
                        .cloned();
                    let Some(p) = prev else { return Err("Nothing above it to go inside".into()) };
                    self.step(
                        &format!("Put {} inside {}", t.title, self.task_title(&p)),
                        vec![Op::SetTask { id, field: "parent".into(), value: json!(p) }],
                    )?;
                } else {
                    let Some(p) = t.parent.clone() else { return Ok(Json::Null) };
                    let grand = self.plan.task(&p).and_then(|x| x.parent.clone());
                    let mine = self.subtree(&id);
                    // After the rest of the old group.
                    let rest: Vec<String> = self.subtree(&p).into_iter().filter(|x| !mine.contains(x)).collect();
                    let last = rest.last().cloned().unwrap_or(p.clone());
                    let anchor = {
                        let k = self.plan.order.iter().position(|x| *x == last).unwrap_or(0);
                        self.plan.order[k + 1..].iter().find(|x| !mine.contains(x)).cloned()
                    };
                    let mut ops = vec![Op::SetTask { id: id.clone(), field: "parent".into(), value: json!(grand) }];
                    for x in &mine {
                        ops.push(Op::Place { id: x.clone(), before: anchor.clone() });
                    }
                    self.step(&format!("Took {} out of {}", t.title, self.task_title(&p)), ops)?;
                }
                Ok(Json::Null)
            }
            "place" => {
                let id = arg_s(args, "id");
                let before = args.get("before").and_then(Json::as_str).map(String::from);
                let parent = args.get("parent").map(|p| p.as_str().map(String::from));
                let mut ops: Vec<Op> =
                    self.subtree(&id).into_iter().map(|x| Op::Place { id: x, before: before.clone() }).collect();
                if let Some(p) = parent
                    && p.as_deref().is_none_or(|p| !self.plan.within(p, &id))
                {
                    ops.insert(0, Op::SetTask { id: id.clone(), field: "parent".into(), value: json!(p) });
                }
                let key = self.plan.task(&id).map(|t| t.key.clone()).unwrap_or_default();
                self.step(&format!("Moved {key} “{}” in the list", self.task_title(&id)), ops)?;
                Ok(Json::Null)
            }
            // Into an epic (or out of any): its rank stays, unless it was above
            // the epic (an epic's issues come after it), then just below it.
            "set_epic" => {
                let id = arg_s(args, "id");
                let parent = args.get("parent").and_then(Json::as_str).map(String::from);
                let t = self.plan.task(&id).ok_or("no such issue")?;
                if t.parent == parent {
                    return Ok(Json::Null);
                }
                if let Some(p) = &parent
                    && (self.plan.within(p, &id) || !self.plan.tasks.contains_key(p))
                {
                    return Err("An issue can’t go into itself".into());
                }
                let key = t.key.clone();
                let mut ops = vec![Op::SetTask { id: id.clone(), field: "parent".into(), value: json!(parent) }];
                if let Some(p) = &parent {
                    let at = |x: &str| self.plan.order.iter().position(|y| y == x);
                    if let (Some(i), Some(pi)) = (at(&id), at(p))
                        && i < pi
                    {
                        // Just after the epic and everything already in it.
                        let mut end = pi + 1;
                        while end < self.plan.order.len() && self.plan.within(&self.plan.order[end], p) {
                            end += 1;
                        }
                        let before = self.plan.order.get(end).cloned();
                        ops.extend(self.subtree(&id).into_iter().map(|x| Op::Place { id: x, before: before.clone() }));
                    }
                }
                let what = match &parent {
                    Some(p) => format!("Moved {key} into {}", self.task_title(p)),
                    None => format!("Took {key} out of its epic"),
                };
                self.step(&what, ops)?;
                Ok(Json::Null)
            }
            // A mirror of something in another plan ("n12:*" or "n12:t3"),
            // made in one step: a task named after it that shows its dates.
            "add_mirror" => {
                let r = arg_s(args, "ref");
                if !is_outside(&r) {
                    return Err("Pick a plan, an epic or an issue on the server".into());
                }
                let title = match arg_s(args, "title") {
                    t if t.is_empty() => "Mirror".to_string(),
                    t => t,
                };
                // One step (one Undo): made, its estimate taken off, pointed at the other.
                self.call("group_start", &json!({}))?;
                let made = self.call("add_task", &json!({ "title": title, "parent": args.get("parent") }));
                let id = match made {
                    Ok(v) => v["id"].as_str().unwrap_or_default().to_string(),
                    Err(e) => {
                        let _ = self.call("group_end", &json!({ "what": "" }));
                        return Err(e);
                    }
                };
                self.call("set_task", &json!({ "id": id, "field": "work", "value": null }))?;
                self.call("set_task", &json!({ "id": id, "field": "mirror", "value": r }))?;
                self.call("group_end", &json!({ "what": format!("Mirrored {title}") }))?;
                Ok(json!({ "id": id }))
            }
            "link" => {
                let (from, to) = (arg_s(args, "from"), arg_s(args, "to"));
                // `from` can be an issue in another plan on the server ("n12:t3").
                if from == to
                    || !(self.plan.tasks.contains_key(&from) || is_outside(&from))
                    || !self.plan.tasks.contains_key(&to)
                {
                    return Err("Pick two different tasks".into());
                }
                if self.plan.tasks[&to].mirror.is_some() {
                    return Err("A mirror only shows when the other one is done: it can’t start after anything".into());
                }
                if self.comes_after(&from, &to) {
                    return Err("That would make a loop: it already comes after it".into());
                }
                let kind: DepKind =
                    serde_json::from_value(args.get("kind").cloned().unwrap_or(json!("FS"))).unwrap_or_default();
                let lag = args.get("lag").and_then(Json::as_i64).unwrap_or(0) as i32;
                let mut after = self.plan.tasks[&to].after.clone();
                after.retain(|d| d.task != from);
                after.push(Dep { task: from.clone(), kind, lag });
                let other = if is_outside(&from) {
                    self.world
                        .outside
                        .get(&from)
                        .map(|o| {
                            if from.ends_with(":*") {
                                // A whole plan: its name (shown even without access).
                                format!("the plan “{}”", if o.plan.is_empty() { &o.title } else { &o.plan })
                            } else if o.readable && !o.title.is_empty() {
                                format!("{} {}", o.key, o.title)
                            } else {
                                o.key.clone()
                            }
                        })
                        .unwrap_or_else(|| "an issue in another plan".into())
                } else {
                    self.task_title(&from)
                };
                let what = format!("{} now starts after {other}", self.task_title(&to));
                self.step(
                    &what,
                    vec![Op::SetTask { id: to, field: "after".into(), value: serde_json::to_value(after).unwrap() }],
                )?;
                Ok(Json::Null)
            }
            "unlink" => {
                let (from, to) = (arg_s(args, "from"), arg_s(args, "to"));
                let mut after = self.plan.task(&to).ok_or("no such task")?.after.clone();
                after.retain(|d| d.task != from);
                let what = format!("{} no longer starts after {}", self.task_title(&to), self.task_title(&from));
                self.step(
                    &what,
                    vec![Op::SetTask { id: to, field: "after".into(), value: serde_json::to_value(after).unwrap() }],
                )?;
                Ok(Json::Null)
            }
            "relate" => {
                let (a, b) = (arg_s(args, "a"), arg_s(args, "b"));
                let field = if arg_s(args, "kind") == "duplicates" { "duplicates" } else { "relates" };
                let t = self.plan.task(&a).ok_or("no such task")?;
                let mut list = if field == "relates" { t.relates.clone() } else { t.duplicates.clone() };
                let on = args.get("on").and_then(Json::as_bool).unwrap_or(true);
                list.retain(|x| *x != b);
                if on {
                    list.push(b.clone());
                }
                let what = format!(
                    "{} {} to {}",
                    if on { "Related" } else { "Unrelated" },
                    self.task_title(&a),
                    self.task_title(&b)
                );
                self.step(&what, vec![Op::SetTask { id: a, field: field.into(), value: json!(list) }])?;
                Ok(Json::Null)
            }
            "set_item" => {
                let list = arg_s(args, "list");
                let mut id = arg_s(args, "id");
                if id.is_empty() {
                    id = self.new_id(match list.as_str() {
                        "resources" => "r",
                        "projects" => "p",
                        _ => "i",
                    });
                }
                let value = args.get("value").cloned().filter(|v| !v.is_null());
                if list == "fields"
                    && let Some(v) = &value
                {
                    let mut f: model::Field = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
                    f.id = id.clone();
                    fields::check_field(&f)?;
                    if !f.default.is_null() {
                        fields::check(&f, &f.default, self.world.today, &self.people_ids())
                            .map_err(|e| format!("The value new issues start with: {e}"))?;
                    }
                }
                let before = args.get("before").and_then(Json::as_str).map(String::from);
                let label = value.as_ref().and_then(|v| v["name"].as_str()).unwrap_or("").to_string();
                let what = args.get("what").and_then(Json::as_str).map(String::from).unwrap_or_else(|| {
                    match (&value, list.as_str()) {
                        (None, l) => format!("Removed from {l}"),
                        (Some(_), l) => format!(
                            "Set {} in {l}",
                            if label.is_empty() { "an item".to_string() } else { label.clone() }
                        ),
                    }
                });
                self.step(&what, vec![Op::SetItem { list, id: id.clone(), value, before }])?;
                Ok(json!({ "id": id }))
            }
            "set_plan" => {
                let field = arg_s(args, "field");
                let value = args.get("value").cloned().unwrap_or(Json::Null);
                let what =
                    args.get("what").and_then(Json::as_str).map(String::from).unwrap_or_else(|| match field.as_str() {
                        "title" => "Renamed the plan".to_string(),
                        _ => "Changed the plan's settings".to_string(),
                    });
                self.step(&what, vec![Op::SetPlan { field, value }])?;
                Ok(Json::Null)
            }
            "save_baseline" => {
                let s = self.schedule();
                let id = self.new_id("b");
                let tasks: std::collections::BTreeMap<String, (Date, Date)> =
                    s.tasks.iter().map(|(k, d)| (k.clone(), (d.start, d.finish))).collect();
                let b = model::Baseline { id: id.clone(), saved: self.now, by: self.me.clone(), tasks };
                // One set of expected dates: these replace the ones before.
                let mut ops: Vec<Op> = self
                    .plan
                    .baselines
                    .iter()
                    .map(|b| Op::SetItem { list: "baselines".into(), id: b.id.clone(), value: None, before: None })
                    .collect();
                ops.push(Op::SetItem { list: "baselines".into(), id, value: Some(serde_json::to_value(b).unwrap()), before: None });
                self.step("Kept these dates as the expected dates", ops)?;
                Ok(Json::Null)
            }
            // Release: every done issue not yet in a release goes out under
            // `name` (off the Board and the Backlog); `id` instead renames one.
            "release" => {
                let name = arg_s(args, "name").trim().to_string();
                if name.is_empty() {
                    return Err("A release needs a name".into());
                }
                if let Some(id) = args.get("id").and_then(Json::as_str) {
                    let Some(r) = self.plan.releases.iter().find(|r| r.id == id) else {
                        return Err("No such release".into());
                    };
                    let r = model::Release { name: name.clone(), ..r.clone() };
                    self.step(
                        &format!("Renamed a release to {name}"),
                        vec![Op::SetItem { list: "releases".into(), id: r.id.clone(), value: Some(serde_json::to_value(r).unwrap()), before: None }],
                    )?;
                    return Ok(Json::Null);
                }
                let out: Vec<Id> = self
                    .plan
                    .order
                    .iter()
                    .filter(|k| self.plan.tasks.get(*k).is_some_and(|t| t.release.is_none() && self.plan.is_done(t) && !self.plan.is_epic(t)))
                    .cloned()
                    .collect();
                if out.is_empty() {
                    return Err("Nothing is done yet to release".into());
                }
                let id = self.new_id("v");
                let r = model::Release { id: id.clone(), name: name.clone(), date: Some(self.world.today) };
                let mut ops =
                    vec![Op::SetItem { list: "releases".into(), id: id.clone(), value: Some(serde_json::to_value(r).unwrap()), before: None }];
                ops.extend(out.iter().map(|k| Op::SetTask { id: k.clone(), field: "release".into(), value: json!(id) }));
                let n = out.len();
                self.step(&format!("Released {name} ({n} issue{})", if n == 1 { "" } else { "s" }), ops)?;
                Ok(json!({ "id": id, "issues": out.len() }))
            }
            "clear_baseline" => {
                let ops = self
                    .plan
                    .baselines
                    .iter()
                    .map(|b| Op::SetItem { list: "baselines".into(), id: b.id.clone(), value: None, before: None })
                    .collect();
                self.step("Forgot the expected dates", ops)?;
                Ok(Json::Null)
            }
            "history" => {
                let task = args.get("task").and_then(Json::as_str);
                Ok(serde_json::to_value(history::entries(&self.plan, task)).unwrap())
            }
            "state_at" => {
                let old = match args.get("index").and_then(Json::as_u64) {
                    Some(n) => history::state_after(&self.plan, n as usize),
                    None => history::state_at(&self.plan, args.get("at").and_then(Json::as_i64).unwrap_or(i64::MAX)),
                };
                let s = schedule::schedule(&old, &self.world);
                let tasks: Vec<Json> = s
                    .tasks
                    .iter()
                    .filter(|(_, d)| d.placed)
                    .map(|(k, d)| json!({ "id": k, "start": d.start, "finish": d.finish, "summary": d.summary }))
                    .collect();
                Ok(json!({ "tasks": tasks }))
            }
            "slips" => Ok(serde_json::to_value(history::slips(&self.plan, &self.world)).unwrap()),
            "restore" => {
                let index = args.get("index").and_then(Json::as_u64).ok_or("missing index")? as usize;
                let what = self
                    .plan
                    .log
                    .get(index)
                    .map(|s| format!("Went back to before “{}”", s.what))
                    .unwrap_or_else(|| "Went back".into());
                let ops = history::restore_ops(&self.plan, index);
                self.step(&what, ops)?;
                Ok(Json::Null)
            }
            "bookings" => {
                let s = self.schedule();
                Ok(serde_json::to_value(schedule::bookings(&self.plan, &self.world, &s)).unwrap())
            }
            "load" => {
                let s = self.schedule();
                let from = arg_date(args, "from").unwrap_or(self.world.today);
                let weeks = args.get("weeks").and_then(Json::as_u64).unwrap_or(8) as u32;
                Ok(serde_json::to_value(load::rows(&self.plan, &self.world, &s, from, weeks)).unwrap())
            }
            "week_detail" => {
                let s = self.schedule();
                let week = arg_date(args, "week").ok_or("missing week")?;
                Ok(serde_json::to_value(load::week_detail(&self.plan, &self.world, &s, &arg_s(args, "resource"), week))
                    .unwrap())
            }
            // Working days from `from` to `to` (both counted), and the day `n` working days after `from`.
            "working_days" => {
                let cal = schedule::Calendar::new(&self.plan, &self.world);
                let from = arg_date(args, "from").ok_or("missing from")?;
                let to = arg_date(args, "to").unwrap_or(from);
                let n = args.get("n").and_then(Json::as_i64).unwrap_or(0) as i32;
                Ok(
                    json!({ "count": cal.count(from, to), "after": cal.add(from, n), "on_or_after": cal.on_or_after(from) }),
                )
            }
            // Countries and regions with built-in holidays (pickers), and a year's holidays.
            "countries" => Ok(serde_json::to_value(xenify_holidays::countries()).unwrap()),
            "holidays" => {
                let y = args.get("year").and_then(Json::as_i64).unwrap_or(self.world.today.year() as i64) as i32;
                Ok(serde_json::to_value(xenify_holidays::holidays(&arg_s(args, "country"), &arg_s(args, "region"), y))
                    .unwrap())
            }
            "find" => Ok(self.find(&arg_s(args, "query"))),
            _ => Err(format!("unknown call: {name}")),
        }
    }

    /// Whether `a` (or anything in it) already comes after `b` (or anything
    /// in it) through links, its epics' links included, so b-after-a would loop.
    fn comes_after(&self, a: &str, b: &str) -> bool {
        let p = &self.plan;
        let mut todo: Vec<String> = p.order.iter().filter(|k| p.within(k, a)).cloned().collect();
        if todo.is_empty() {
            todo.push(a.to_string());
        }
        let mut seen = std::collections::HashSet::new();
        let tree = schedule::Tree::new(p);
        while let Some(x) = todo.pop() {
            if !seen.insert(x.clone()) {
                continue;
            }
            let Some(t) = p.task(&x) else { continue };
            for (d, _) in tree.waits_for(t) {
                if d == b || p.within(&d, b) {
                    return true;
                }
                todo.push(d);
            }
        }
        false
    }

    /// Tasks whose key, title, description or people match (AI, quick find).
    pub fn find(&self, query: &str) -> Json {
        let q = query.to_lowercase();
        let names = self.names();
        let s = self.schedule();
        let hits: Vec<Json> = self
            .plan
            .order
            .iter()
            .filter_map(|id| self.plan.tasks.get(id))
            .filter(|t| {
                q.is_empty()
                    || t.key.to_lowercase().contains(&q)
                    || t.title.to_lowercase().contains(&q)
                    || t.description.to_lowercase().contains(&q)
                    || t.labels.iter().any(|l| l.to_lowercase().contains(&q))
                    || t.who.as_ref().and_then(|w| names.get(w)).is_some_and(|n| n.0.to_lowercase().contains(&q))
            })
            .take(200)
            .map(|t| {
                let d = s.tasks.get(&t.id).cloned().unwrap_or_default();
                json!({ "id": t.id, "key": t.key, "title": t.title, "status": self.plan.status(&t.status).map(|s| s.name.clone()), "placed": d.placed, "start": d.start, "finish": d.finish, "late": d.late })
            })
            .collect();
        Json::Array(hits)
    }
}

impl Session {
    /// "Wireframes: status" → words for History.
    fn describe(&self, title: &str, field: &str, value: &Json) -> String {
        let status =
            |v: &Json| v.as_str().and_then(|x| self.plan.status(x)).map(|s| s.name.clone()).unwrap_or_default();
        let hours = |v: &Json| {
            v.as_u64()
                .map(|m| format!("{}h", (m as f64 / 60.0 * 10.0).round() / 10.0))
                .unwrap_or_else(|| "no estimate".into())
        };
        match field {
            "title" => format!("Renamed an issue to {}", value.as_str().unwrap_or_default()),
            "status" => format!("{title}: {}", status(value)),
            "who" => match value.as_str().and_then(|w| self.names().get(w).map(|x| x.0.clone())) {
                Some(n) => format!("Gave {title} to {n}"),
                None => format!("{title}: no one yet"),
            },
            "work" => format!("{title}: estimate {}", hours(value)),
            "due" => match value.as_str() {
                Some(d) => format!("{title}: due {d}"),
                None => format!("{title}: no due date"),
            },
            "priority" => format!("{title}: priority {}", value.as_str().unwrap_or("medium")),
            "parent" => format!("Moved {title} to another epic"),
            "after" => format!("{title}: changed links"),
            "description" => format!("{title}: changed the description"),
            "labels" => format!("{title}: changed labels"),
            f => format!("{title}: changed {f}"),
        }
    }
}

fn to36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = Vec::new();
    for _ in 0..8 {
        s.push(D[(n % 36) as usize]);
        n /= 36;
    }
    String::from_utf8(s).unwrap()
}

/// Labels as kept: trimmed, once each, and spelt as the plan already has
/// them ("frontend" joins "Frontend").
fn tidy_labels(given: Vec<String>, plan: &Plan) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in given {
        let l = l.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() || out.iter().any(|x| x.eq_ignore_ascii_case(&l)) {
            continue;
        }
        let known = plan.tasks.values().flat_map(|t| t.labels.iter()).find(|x| x.eq_ignore_ascii_case(&l)).cloned();
        out.push(known.unwrap_or(l));
    }
    out
}
