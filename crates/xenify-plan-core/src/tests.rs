use super::*;
use serde_json::json;

const H: u64 = 60; // minutes
const D: u64 = 450; // a 7.5-hour day

/// A website relaunch, as in the prototype: today Mon 2 Nov 2026, no
/// public holidays. Lena works 6 hours a day and is away 9–11 Nov; Tom is
/// away 23–24 Nov.
fn website() -> (Session, HashMap<&'static str, String>) {
    let mut s = Session::new();
    s.call("set_clock", &json!({ "today": "2026-11-02", "now": 1_000, "seed": 7 })).unwrap();
    s.call("set_me", &json!({ "name": "Sam Carter" })).unwrap();
    s.fresh("Website relaunch");
    s.call("set_item", &json!({ "list": "projects", "id": "web", "value": { "name": "Website", "prefix": "WEB", "color": "#C2410C" } })).unwrap();
    // People come from the server's register.
    s.call("set_world", &json!({ "resources": [
        { "id": "sam", "name": "Sam Carter", "kind": "person" },
        { "id": "priya", "name": "Priya Shah", "kind": "person" },
        { "id": "tom", "name": "Tom Becker", "kind": "person", "away": [["2026-11-23", "2026-11-24"]] },
        { "id": "lena", "name": "Lena Ortiz", "kind": "person", "day_minutes": 360, "away": [["2026-11-09", "2026-11-11"]] },
    ] })).unwrap();
    let mut ids = HashMap::new();
    let add = |s: &mut Session, ids: &mut HashMap<&'static str, String>, k: &'static str, args: Json| {
        let id = s.call("add_task", &args).unwrap()["id"].as_str().unwrap().to_string();
        ids.insert(k, id);
    };
    add(&mut s, &mut ids, "content", json!({ "title": "Content", "kind": "epic" }));
    add(&mut s, &mut ids, "design", json!({ "title": "Design", "kind": "epic" }));
    add(&mut s, &mut ids, "build", json!({ "title": "Build", "kind": "epic" }));
    let (c, d, b) = (ids["content"].clone(), ids["design"].clone(), ids["build"].clone());
    add(
        &mut s,
        &mut ids,
        "copy",
        json!({ "title": "Write homepage copy", "kind": "story", "parent": c, "who": "lena", "work": 3 * D }),
    );
    add(
        &mut s,
        &mut ids,
        "pcopy",
        json!({ "title": "Write product page copy", "kind": "story", "parent": c, "who": "lena", "work": 4 * D }),
    );
    add(
        &mut s,
        &mut ids,
        "hd",
        json!({ "title": "Homepage design", "kind": "story", "parent": d, "who": "priya", "work": 4 * D }),
    );
    let (copy, hd) = (ids["copy"].clone(), ids["hd"].clone());
    add(&mut s, &mut ids, "setup", json!({ "title": "Set up hosting", "parent": b, "who": "tom", "work": 2 * D }));
    let setup = ids["setup"].clone();
    add(
        &mut s,
        &mut ids,
        "build_home",
        json!({ "title": "Build the homepage", "kind": "story", "parent": b, "who": "tom", "work": 5 * D, "waits_for": [hd, copy, setup] }),
    );
    add(&mut s, &mut ids, "redirects", json!({ "title": "Redirects", "parent": b, "who": "tom", "work": null }));
    let (bh, rd) = (ids["build_home"].clone(), ids["redirects"].clone());
    add(
        &mut s,
        &mut ids,
        "launch",
        json!({ "title": "Launch", "kind": "milestone", "waits_for": [bh, rd], "due": "2026-11-20" }),
    );
    (s, ids)
}

fn dates(s: &Session, id: &str) -> (String, String) {
    let d = &s.schedule().tasks[id];
    (d.start.to_string(), d.finish.to_string())
}

#[test]
fn estimates_and_links_set_the_dates() {
    let (s, ids) = website();
    let sch = s.schedule();
    // Lena: 22.5 hours at 6 a day: Mon to Thu.
    assert_eq!(dates(&s, &ids["copy"]), ("2026-11-02".into(), "2026-11-05".into()));
    // Then her product page copy: the rest of Thu, Fri, then away 9–11 Nov: two pieces.
    let pc = &sch.tasks[&ids["pcopy"]];
    assert_eq!(pc.pieces.len(), 2, "{:?}", pc.pieces);
    assert_eq!(pc.pieces[0].0.to_string(), "2026-11-05");
    assert_eq!(pc.pieces[1].0.to_string(), "2026-11-12");
    // Tom builds the homepage once the design, the copy and hosting are done.
    assert_eq!(dates(&s, &ids["hd"]).1, "2026-11-05");
    assert_eq!(dates(&s, &ids["build_home"]).0, "2026-11-06");
    // Redirects have no estimate: not on the Timeline, and Launch says it waits for them.
    assert!(!sch.tasks[&ids["redirects"]].placed);
    assert_eq!(sch.tasks[&ids["launch"]].waits, vec![ids["redirects"].clone()]);
    // Launch the day after the homepage is built; due 20 Nov, so it's late.
    let l = &sch.tasks[&ids["launch"]];
    assert!(l.milestone && l.placed);
    assert_eq!(l.start.to_string(), "2026-11-13");
    assert!(!l.late);
    // Epics span their issues.
    let b = &sch.tasks[&ids["build"]];
    assert!(b.summary);
    assert_eq!(b.start.to_string(), "2026-11-02");
    assert_eq!(b.work, 7 * D as u32);
}

#[test]
fn expected_dates_are_kept_once_and_shown_beside_the_dates_now() {
    let (mut s, ids) = website();
    let task = |s: &Session, id: &str| s.view()["tasks"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap().clone();
    assert!(s.view()["expected"].is_null());
    assert!(task(&s, &ids["launch"])["expected"].is_null());
    s.call("save_baseline", &json!({})).unwrap();
    assert_eq!(task(&s, &ids["launch"])["expected"], json!(["2026-11-13", "2026-11-13"]));
    // More work moves Launch on; what was expected stays.
    s.call("set_task", &json!({ "id": ids["setup"], "field": "work", "value": 6 * D })).unwrap();
    let l = task(&s, &ids["launch"]);
    assert_eq!(l["expected"][1], "2026-11-13");
    assert!(l["finish"].as_str().unwrap() > "2026-11-13");
    // Keeping them again replaces the old ones (one set, not a pile).
    s.call("save_baseline", &json!({})).unwrap();
    assert_eq!(s.plan.baselines.len(), 1);
    assert_eq!(task(&s, &ids["launch"])["expected"][1], l["finish"]);
    assert_eq!(task(&s, &ids["build"])["expected"][0], "2026-11-02", "groups have them too");
    s.call("clear_baseline", &json!({})).unwrap();
    assert!(s.view()["expected"].is_null());
    // Someone who may only change the schedule may keep them.
    assert_eq!(crate::access::need(&crate::ops::Op::SetItem { list: "baselines".into(), id: "b1".into(), value: None, before: None }), "plan.schedule");
}

#[test]
fn other_work_never_splits_an_issue() {
    let (mut s, ids) = website();
    // Tom: hosting Mon–Tue, homepage from Fri 6 Nov; Wed–Thu are free.
    let small =
        s.call("add_task", &json!({ "title": "Fix the contact form", "who": "tom", "work": 2 * D })).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(dates(&s, &small), ("2026-11-04".into(), "2026-11-05".into()), "fits the gap whole");
    // Three days don't fit the two-day gap: after the homepage, not around it.
    s.call("set_task", &json!({ "id": small, "field": "work", "value": 3 * D })).unwrap();
    let sch = s.schedule();
    let x = &sch.tasks[&small];
    assert_eq!(x.pieces.len(), 1);
    assert!(x.start > sch.tasks[&ids["build_home"]].finish);
}

#[test]
fn estimates_move_launch_and_due_dates_show_late() {
    let (mut s, ids) = website();
    s.call("set_task", &json!({ "id": ids["redirects"], "field": "work", "value": 6 * D })).unwrap();
    let sch = s.schedule();
    assert!(sch.tasks[&ids["redirects"]].placed);
    assert!(sch.tasks[&ids["launch"]].waits.is_empty());
    let l = &sch.tasks[&ids["launch"]];
    assert!(l.start > sch.tasks[&ids["redirects"]].finish);
    assert!(l.late && l.late_by > 0, "{l:?}");
}

#[test]
fn holding_back_and_unassigned_work() {
    let (mut s, ids) = website();
    s.call("hold", &json!({ "id": ids["hd"], "start": "2026-11-16" })).unwrap();
    assert_eq!(dates(&s, &ids["hd"]).0, "2026-11-16");
    assert!(s.plan.log.last().unwrap().what.starts_with("Homepage design starts no sooner than Mon 16 Nov"));
    s.call("hold", &json!({ "id": ids["hd"] })).unwrap();
    assert_eq!(dates(&s, &ids["hd"]).0, "2026-11-02");
    // No one yet: working days at the plan's hours, not anyone's time.
    let t = s.call("add_task", &json!({ "title": "Accessibility check", "work": 2 * D })).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(dates(&s, &t), ("2026-11-02".into(), "2026-11-03".into()));
}

#[test]
fn what_an_epic_waits_for_everything_in_it_waits_for() {
    let (mut s, ids) = website();
    // Content waits for the homepage design (done Thu 5 Nov): its copy starts after.
    s.call("link", &json!({ "from": ids["hd"], "to": ids["content"] })).unwrap();
    assert_eq!(dates(&s, &ids["copy"]).0, "2026-11-06");
    // An epic waiting for its own issue holds nothing up (no loop).
    s.call("link", &json!({ "from": ids["copy"], "to": ids["content"] })).unwrap();
    let d = &s.schedule().tasks[&ids["copy"]];
    assert!(!d.warnings.iter().any(|w| w.contains("loop")), "{:?}", d.warnings);
    assert_eq!(d.start.to_string(), "2026-11-06");
}

#[test]
fn a_full_day_is_booked_in_full_whatever_its_length() {
    // Lena works 6 hours a day: her homepage copy (22.5 hours) fills Mon to
    // Wed and three quarters of Thu: 93% of her days, not 75% of 7.5-hour
    // ones (another plan would have given her the rest of Mon to Wed).
    let (mut s, ids) = website();
    let bk: Vec<Booking> = serde_json::from_value(s.call("bookings", &json!({})).unwrap()).unwrap();
    let copy = bk.iter().find(|b| b.task == ids["copy"] && b.from.to_string() == "2026-11-02").unwrap();
    assert_eq!((copy.resource.as_str(), copy.percent), ("lena", 93));
}

#[test]
fn a_week_with_no_working_days_gives_up_at_once() {
    let (mut s, ids) = website();
    s.call(
        "set_plan",
        &json!({ "field": "settings.work_days", "value": [false, false, false, false, false, false, false] }),
    )
    .unwrap();
    let t = std::time::Instant::now();
    let d = &s.schedule().tasks[&ids["copy"]];
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "took {:?}", t.elapsed());
    assert!(!d.placed);
    assert!(d.warnings.iter().any(|w| w == "Nobody has time for this work"));
}

#[test]
fn status_records_when_work_started_and_finished() {
    let (mut s, ids) = website();
    s.call("set_task", &json!({ "id": ids["copy"], "field": "status", "value": "doing" })).unwrap();
    assert_eq!(s.plan.tasks[&ids["copy"]].started.map(|d| d.to_string()).as_deref(), Some("2026-11-02"));
    s.call("set_clock", &json!({ "today": "2026-11-04" })).unwrap();
    s.call("set_task", &json!({ "id": ids["copy"], "field": "status", "value": "done" })).unwrap();
    assert_eq!(dates(&s, &ids["copy"]), ("2026-11-02".into(), "2026-11-04".into()));
    s.call("undo", &json!({})).unwrap();
    assert!(s.plan.tasks[&ids["copy"]].finished.is_none(), "one undo takes both back");
}

#[test]
fn comments_and_history_per_issue() {
    let (mut s, ids) = website();
    s.call("add_comment", &json!({ "id": ids["hd"], "html": "<p>Ready for review</p>" })).unwrap();
    let c = s.plan.tasks[&ids["hd"]].comments[0].clone();
    assert_eq!(c.who, "Sam Carter");
    s.call("set_comment", &json!({ "id": ids["hd"], "comment": c.id, "html": "<p>Reviewed</p>" })).unwrap();
    assert_eq!(s.plan.tasks[&ids["hd"]].comments[0].html, "<p>Reviewed</p>");
    let e = s.call("history", &json!({ "task": ids["hd"] })).unwrap();
    assert_eq!(e[0]["what"], json!("Changed a comment on Homepage design"));
    let v = s.view();
    let hd = v["tasks"].as_array().unwrap().iter().find(|t| t["id"] == json!(ids["hd"])).unwrap();
    assert_eq!(hd["created_by"], json!("Sam Carter"));
    assert_eq!(hd["priority"], json!("medium"));
}

#[test]
fn load_is_hours_booked_against_hours_free() {
    let (mut s, _) = website();
    let rows = s.call("load", &json!({ "from": "2026-11-02", "weeks": 2 })).unwrap();
    let lena = rows.as_array().unwrap().iter().find(|r| r["name"] == "Lena Ortiz").unwrap();
    assert_eq!(lena["cells"][0]["have"], json!(5 * 360));
    assert_eq!(lena["cells"][0]["booked"], json!(5 * 360), "full in her first week");
    assert_eq!(lena["cells"][1]["away"], json!(3));
    let w = s.call("week_detail", &json!({ "resource": "lena", "week": "2026-11-02" })).unwrap();
    assert_eq!(w.as_array().unwrap().len(), 2);
    let _ = H;
}

#[test]
fn links_cannot_loop() {
    let (mut s, ids) = website();
    assert!(s.call("link", &json!({ "from": ids["launch"], "to": ids["hd"] })).is_err());
    // Through epics too: Build's homepage waits for the homepage design, so
    // Design can't wait for anything in Build, nor Build's issues for Launch.
    assert!(s.call("link", &json!({ "from": ids["build"], "to": ids["design"] })).is_err());
    assert!(s.call("link", &json!({ "from": ids["launch"], "to": ids["build"] })).is_err());
    s.call("link", &json!({ "from": ids["copy"], "to": ids["design"] })).unwrap();
}

#[test]
fn delete_undo_redo() {
    let (mut s, ids) = website();
    let before = s.plan.content();
    s.call("delete_tasks", &json!({ "ids": [ids["content"]] })).unwrap();
    assert!(!s.plan.tasks.contains_key(&ids["copy"]), "deletes what's inside");
    s.call("undo", &json!({})).unwrap();
    assert_eq!(s.plan.content(), before);
    s.call("redo", &json!({})).unwrap();
    assert!(!s.plan.tasks.contains_key(&ids["content"]));
}

#[test]
fn delete_epic_keeping_its_issues() {
    let (mut s, ids) = website();
    let up = s.plan.task(&ids["content"]).unwrap().parent.clone();
    let before = s.plan.content();
    s.call("delete_tasks", &json!({ "ids": [ids["content"]], "keep": true })).unwrap();
    assert!(!s.plan.tasks.contains_key(&ids["content"]));
    assert_eq!(s.plan.task(&ids["copy"]).unwrap().parent, up, "moved to where the epic was");
    s.call("undo", &json!({})).unwrap();
    assert_eq!(s.plan.content(), before, "one Undo brings the epic and its issues back");
}

#[test]
fn history_replays_and_goes_back() {
    let (mut s, ids) = website();
    let mark = s.plan.log.len();
    s.now = 2 * 86_400_000;
    s.call("set_task", &json!({ "id": ids["hd"], "field": "work", "value": 8 * D })).unwrap();
    assert_eq!(history::state_after(&s.plan, s.plan.log.len()).content(), s.plan.content());
    let then = s.call("state_at", &json!({ "index": mark })).unwrap();
    let hd = then["tasks"].as_array().unwrap().iter().find(|t| t["id"] == json!(ids["hd"])).unwrap();
    assert_eq!(hd["finish"], json!("2026-11-05"));
    let e = s.call("history", &json!({ "task": ids["hd"] })).unwrap();
    assert_eq!(e[0]["what"], json!("Homepage design: estimate 60h"));
    let idx = s.plan.log.len() - 1;
    s.call("restore", &json!({ "index": idx })).unwrap();
    assert_eq!(dates(&s, &ids["hd"]).1, "2026-11-05");
}

#[test]
fn several_edits_as_one_step() {
    let (mut s, ids) = website();
    let before = s.plan.content();
    let steps = s.plan.log.len();
    s.call("group_start", &json!({})).unwrap();
    let t = s.call("add_task", &json!({ "title": "Basket" })).unwrap()["id"].as_str().unwrap().to_string();
    s.call("link", &json!({ "from": ids["setup"], "to": t })).unwrap();
    s.call("set_task", &json!({ "id": t, "field": "who", "value": "tom" })).unwrap();
    s.call("group_end", &json!({ "what": "Ask AI: broke down the build" })).unwrap();
    assert_eq!(s.plan.log.len(), steps + 1);
    assert_eq!(history::state_after(&s.plan, s.plan.log.len()).content(), s.plan.content(), "History still replays");
    s.call("undo", &json!({})).unwrap();
    assert_eq!(s.plan.content(), before, "one undo takes it all back");
}

#[test]
fn file_round_trip_keeps_the_plan() {
    let (s, _) = website();
    let bytes = s.save_file().unwrap();
    let mut t = Session::new();
    t.open_file(&bytes).unwrap();
    assert_eq!(t.plan.content(), s.plan.content());
}

#[test]
fn diff_turns_one_plan_into_another() {
    let (a, _) = website();
    let (mut b, ids) = website();
    b.call("set_task", &json!({ "id": ids["setup"], "field": "title", "value": "Hosting" })).unwrap();
    b.call(
        "set_item",
        &json!({ "list": "statuses", "id": "blocked", "value": { "name": "Blocked" }, "before": "done" }),
    )
    .unwrap();
    let mut x = a.plan.clone();
    ops::apply_all(&mut x, &ops::diff(&a.plan, &b.plan)).unwrap();
    x.log = b.plan.log.clone();
    assert_eq!(x, b.plan);
}

mod together {
    use super::*;

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

    #[test]
    fn two_people_edit_one_plan() {
        let (mut a, ids) = website();
        a.call("remote_open", &json!({ "server": "s", "node": 9, "name": "Website relaunch", "data": "" })).unwrap();
        let saved = a.call("remote_save", &json!({})).unwrap();
        let mut b = Session::new();
        b.call("set_clock", &json!({ "today": "2026-11-02", "seed": 99 })).unwrap();
        b.call("set_me", &json!({ "name": "Priya Shah" })).unwrap();
        b.call(
            "remote_open",
            &json!({ "server": "s", "node": 9, "name": "Website relaunch", "data": saved["data"], "version": 1 }),
        )
        .unwrap();
        assert_eq!(b.plan.content(), a.plan.content());
        // A step made while joining goes once editing together starts.
        a.call(
            "set_task",
            &json!({ "id": ids["pcopy"], "field": "title", "value": "Write product page copy (all pages)" }),
        )
        .unwrap();
        a.call("live_start", &json!({ "seq": 0, "conn": "a" })).unwrap();
        b.call("live_start", &json!({ "seq": 0, "conn": "b" })).unwrap();
        let mut srv = Pretend::default();

        // At once: A renames the homepage design and holds it back; B renames
        // it too, gives hosting to Priya, adds an issue, and deletes
        // Redirects while A estimates it.
        a.call("set_task", &json!({ "id": ids["hd"], "field": "title", "value": "Look and feel" })).unwrap();
        a.call("hold", &json!({ "id": ids["hd"], "start": "2026-11-16" })).unwrap();
        a.call("set_task", &json!({ "id": ids["redirects"], "field": "work", "value": 900 })).unwrap();
        let na = a.call("add_task", &json!({ "title": "Accessibility check", "project": "web" })).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        b.call("set_task", &json!({ "id": ids["hd"], "field": "title", "value": "Homepage design v2" })).unwrap();
        b.call("set_task", &json!({ "id": ids["setup"], "field": "who", "value": "priya" })).unwrap();
        let nb = b.call("add_task", &json!({ "title": "Analytics", "project": "web" })).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        b.call("delete_tasks", &json!({ "ids": [ids["redirects"]] })).unwrap();
        assert_eq!(a.plan.tasks[&na].key, b.plan.tasks[&nb].key, "both picked the next key");
        let (oa, ob) = (a.live_out(), b.live_out());
        srv.take(0, oa);
        srv.take(1, ob);
        srv.run(&mut a, &mut b);

        assert_eq!(a.plan.content(), b.plan.content(), "both hold the same plan");
        assert_eq!(a.plan.tasks[&ids["hd"]].title, "Homepage design v2", "the later rename wins");
        assert_eq!(
            b.plan.tasks[&ids["pcopy"]].title, "Write product page copy (all pages)",
            "made while joining, still sent"
        );
        assert_eq!(a.plan.tasks[&ids["setup"]].who.as_deref(), Some("priya"));
        assert!(!a.plan.tasks.contains_key(&ids["redirects"]), "deleted; A's edit to it dropped");
        assert_ne!(a.plan.tasks[&na].key, a.plan.tasks[&nb].key, "the later new task got the next number");
        assert_eq!(dates(&b, &ids["hd"]).0, "2026-11-16");
        // Both copies' History has everyone's steps.
        assert!(b.plan.log.iter().any(|s| s.who == "Sam Carter" && s.what.starts_with("Look and feel starts no sooner than")));

        // Undo undoes my own step, where things are now.
        b.call("undo", &json!({})).unwrap();
        srv.take(1, b.live_out());
        srv.run(&mut a, &mut b);
        assert!(a.plan.tasks.contains_key(&ids["redirects"]), "B's delete undone everywhere");
        assert_eq!(a.plan.content(), b.plan.content());
    }
}

#[test]
fn custom_fields_keep_their_rules() {
    let (mut s, ids) = website();
    let field =
        |s: &mut Session, id: &str, v: Json| s.call("set_item", &json!({ "list": "fields", "id": id, "value": v }));
    field(
        &mut s,
        "pts",
        json!({ "name": "Points", "kind": "number", "min": 0, "max": 13, "decimals": 0, "required": true }),
    )
    .unwrap();
    field(
        &mut s,
        "ref",
        json!({ "name": "Ticket", "kind": "text", "pattern": "[A-Z]+-\\d+", "pattern_says": "Like OPS-12" }),
    )
    .unwrap();
    field(&mut s, "area", json!({ "name": "Area", "kind": "choices", "choices": ["Front", "Back"], "max_picks": 1 }))
        .unwrap();
    assert!(field(&mut s, "bad", json!({ "name": "Bad", "kind": "number", "min": 5, "max": 1 })).is_err());
    assert!(field(&mut s, "bad", json!({ "name": "Bad", "kind": "choice" })).is_err(), "a choice field needs choices");
    let t = ids["copy"].clone();
    let set = |s: &mut Session, f: &str, v: Json| s.call("set_field", &json!({ "id": t, "field": f, "value": v }));
    assert_eq!(set(&mut s, "pts", json!(20)).unwrap_err(), "Points is at most 13");
    assert_eq!(set(&mut s, "pts", json!(2.5)).unwrap_err(), "Points is a whole number");
    set(&mut s, "pts", json!("5")).unwrap();
    assert_eq!(s.plan.tasks[&t].fields["pts"], json!(5.0));
    assert_eq!(set(&mut s, "ref", json!("ops 12")).unwrap_err(), "Like OPS-12");
    set(&mut s, "ref", json!(" OPS-12 ")).unwrap();
    assert_eq!(s.plan.tasks[&t].fields["ref"], json!("OPS-12"));
    assert!(set(&mut s, "area", json!(["Front", "Back"])).is_err());
    set(&mut s, "area", json!(["front"])).unwrap();
    assert_eq!(s.plan.tasks[&t].fields["area"], json!(["Front"]));
    // Required and empty: the issue needs details.
    let v = s.view();
    let other = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(ids["pcopy"])).unwrap();
    assert!(other["missing"].as_array().unwrap().contains(&json!("Points")));
    set(&mut s, "pts", Json::Null).unwrap();
    assert!(!s.plan.tasks[&t].fields.contains_key("pts"));
}

#[test]
fn labels_are_kept_once_and_renamed_everywhere() {
    let (mut s, ids) = website();
    s.call("set_labels", &json!({ "id": ids["copy"], "labels": ["Content", " content ", "SEO"] })).unwrap();
    assert_eq!(s.plan.tasks[&ids["copy"]].labels, vec!["Content", "SEO"]);
    s.call("set_labels", &json!({ "id": ids["pcopy"], "labels": ["seo"] })).unwrap();
    assert_eq!(s.plan.tasks[&ids["pcopy"]].labels, vec!["SEO"], "spelt as the plan has it");
    s.call("rename_label", &json!({ "from": "seo", "to": "Search" })).unwrap();
    assert_eq!(s.plan.tasks[&ids["copy"]].labels, vec!["Content", "Search"]);
    assert_eq!(s.view()["labels"], json!([{ "name": "Content", "count": 1 }, { "name": "Search", "count": 2 }]));
    s.call("undo", &json!({})).unwrap();
    assert_eq!(s.plan.tasks[&ids["pcopy"]].labels, vec!["SEO"]);
}

#[test]
fn a_plan_kept_as_a_file_has_no_audit() {
    let (s, _) = website();
    assert!(!s.plan.log.is_empty());
    let mut t = Session::new();
    t.open_file(&s.save_file().unwrap()).unwrap();
    assert_eq!(t.plan.log.len(), 1, "only the opening line: no changes, no names");
    assert_eq!(s.view()["audit"], json!(false));
}

#[test]
fn mirrors_show_when_something_elsewhere_is_done() {
    let (mut s, ids) = website();
    let m = s.call("add_mirror", &json!({ "ref": "n7:*", "title": "Brand refresh" })).unwrap()["id"].as_str().unwrap().to_string();
    let e = s.call("add_mirror", &json!({ "ref": "n7:t3", "title": "Logo" })).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(s.call("outside_refs", &json!({})).unwrap(), json!(["n7:*", "n7:t3"]));
    s.call("set_world", &json!({ "outside": {
        "n7:*": { "key": "", "title": "Brand", "plan": "Brand", "readable": true, "placed": true, "start": "2026-11-02", "finish": "2026-11-20" },
        "n7:t3": { "key": "BRA-3", "title": "Logo", "readable": true, "placed": true, "start": "2026-11-09", "finish": "2026-11-11", "done": true },
    } })).unwrap();
    let v = s.view();
    let get = |id: &str| v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(id)).unwrap().clone();
    assert_eq!((get(&m)["start"].clone(), get(&m)["finish"].clone()), (json!("2026-11-02"), json!("2026-11-20")), "the other's dates");
    assert_eq!((get(&m)["work"].clone(), get(&m)["total"].clone()), (json!(null), json!(0)), "no work of its own");
    assert_eq!(get(&e)["done"], json!(100));
    // Nothing else moves because of it; it can't start after anything, but others can start after it.
    assert_eq!(get(&ids["hd"])["start"], json!("2026-11-02"));
    assert!(s.call("link", &json!({ "from": ids["hd"], "to": m })).is_err());
    s.call("link", &json!({ "from": m, "to": ids["hd"] })).unwrap();
    let v = s.view();
    let hd = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(ids["hd"])).unwrap();
    assert_eq!(hd["start"], json!("2026-11-23"), "starts after what the mirror shows");
    // One Undo each.
    s.call("undo", &json!({})).unwrap();
    s.call("undo", &json!({})).unwrap();
    s.call("undo", &json!({})).unwrap();
    assert!(!s.plan.tasks.contains_key(&m));
}

#[test]
fn issues_wait_for_issues_in_other_plans() {
    let (mut s, ids) = website();
    let hd = ids["hd"].clone();
    s.call("set_world", &json!({ "outside": { "n7:t1": { "key": "MKT-4", "title": "", "readable": false, "placed": true, "start": "2026-11-16", "finish": "2026-11-20" } } })).unwrap();
    s.call("link", &json!({ "from": "n7:t1", "to": hd })).unwrap();
    assert_eq!(
        s.plan.log.last().unwrap().what,
        "Homepage design now starts after MKT-4",
        "no title from a plan you can't open"
    );
    let v = s.view();
    let t = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(hd)).unwrap();
    assert_eq!(t["start"], json!("2026-11-23"), "starts the working day after the other plan's issue finishes");
    assert_eq!(v["outside"]["n7:t1"]["key"], json!("MKT-4"));
    // Not known yet (the server hasn't said): it waits, rather than guessing.
    s.call("set_world", &json!({ "outside": {} })).unwrap();
    let v = s.view();
    let t = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(hd)).unwrap();
    assert_eq!(t["waits"], json!(["n7:t1"]));
    assert_eq!(s.call("outside_refs", &json!({})).unwrap(), json!(["n7:t1"]));
    // An exported copy keeps it as it was.
    s.call("set_world", &json!({ "outside": { "n7:t1": { "key": "MKT-4", "placed": true, "start": "2026-11-16", "finish": "2026-11-20" } } })).unwrap();
    let b64 = s.call("export_copy", &json!({})).unwrap();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.as_str().unwrap()).unwrap();
    let mut c = Session::new();
    c.call("set_clock", &json!({ "today": "2026-11-02" })).unwrap();
    c.open_file(&bytes).unwrap();
    let v = c.view();
    let t = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(hd)).unwrap();
    assert_eq!(t["start"], json!("2026-11-23"));
}

#[test]
fn an_exported_copy_reads_without_the_server() {
    let (mut s, ids) = website();
    assert!(s.plan.resources.is_empty(), "a plan keeps only ids");
    let b64 = s.call("export_copy", &json!({})).unwrap();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.as_str().unwrap()).unwrap();
    let mut c = Session::new();
    c.open_file(&bytes).unwrap();
    let v = c.view();
    let t = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(ids["copy"])).unwrap();
    assert_eq!(t["who_name"], json!("Lena Ortiz"), "names written into the copy");
}

#[test]
fn an_exported_copy_shows_other_plans_bookings_in_people() {
    let (mut s, _) = website();
    let other = json!([{ "resource": "tom", "plan": "Shop", "task": "x", "title": "Basket", "from": "2026-11-02", "to": "2026-11-06", "percent": 50 }]);
    s.call("set_world", &json!({ "bookings": other })).unwrap();
    let busy = |s: &mut Session| {
        let rows = s.call("load", &json!({ "from": "2026-11-02", "weeks": 1 })).unwrap();
        rows.as_array().unwrap().iter().find(|r| r["id"] == json!("tom")).unwrap()["cells"][0]["booked"].clone()
    };
    let here = busy(&mut s);
    let b64 = s.call("export_copy", &json!({})).unwrap();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.as_str().unwrap()).unwrap();
    let mut c = Session::new();
    c.call("set_clock", &json!({ "today": "2026-11-02" })).unwrap();
    c.open_file(&bytes).unwrap();
    assert_eq!(busy(&mut c), here);
    let week = c.call("week_detail", &json!({ "resource": "tom", "week": "2026-11-02" })).unwrap();
    assert!(week.as_array().unwrap().iter().any(|w| w["title"] == json!("Basket")));
}

#[test]
fn group_work_goes_to_the_member_free_first() {
    let (mut s, ids) = website();
    // Front end: Priya, Tom and Lena; any of them can do its work.
    s.call("set_world", &json!({ "resources": [
        { "id": "sam", "name": "Sam Carter", "kind": "person" },
        { "id": "priya", "name": "Priya Shah", "kind": "person" },
        { "id": "tom", "name": "Tom Becker", "kind": "person", "away": [["2026-11-23", "2026-11-24"]] },
        { "id": "lena", "name": "Lena Ortiz", "kind": "person", "day_minutes": 360, "away": [["2026-11-09", "2026-11-11"]] },
        { "id": "g1", "name": "Front end", "kind": "group", "members": [{ "person": "priya" }, { "person": "tom" }, { "person": "lena" }] },
    ] })).unwrap();
    let a = s.call("add_task", &json!({ "title": "Cookie banner", "who": "g1", "work": 2 * D })).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let sch = s.schedule();
    // Tom has hosting Mon–Tue then a free Wed–Thu: he finishes first (Thu 5 Nov).
    assert_eq!(sch.tasks[&a].by.as_deref(), Some("tom"));
    assert_eq!(dates(&s, &a), ("2026-11-04".into(), "2026-11-05".into()));
    // A second one goes to whoever is next free, not on top of Tom.
    let b = s.call("add_task", &json!({ "title": "Footer links", "who": "g1", "work": 2 * D })).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let sch = s.schedule();
    let by_b = sch.tasks[&b].by.clone().unwrap();
    assert_ne!(sch.tasks[&b].start, sch.tasks[&a].start, "not both on Tom's gap");
    assert!(["priya", "tom", "lena"].contains(&by_b.as_str()));
    // Booked to the member, so the People view and other plans see it.
    let bk = schedule::bookings(&s.plan, &s.world, &sch);
    assert!(bk.iter().any(|x| x.task == a && x.resource == "tom"));
    assert!(!bk.iter().any(|x| x.resource == "g1"));
    let v = s.view();
    let t = v["tasks"].as_array().unwrap().iter().find(|x| x["id"] == json!(a)).unwrap();
    assert_eq!(
        (t["who_name"].clone(), t["by_name"].clone(), t["who_group"].clone()),
        (json!("Front end"), json!("Tom Becker"), json!(true))
    );
    assert_eq!(v["groups"][0]["members"].as_array().unwrap().len(), 3);
    // Someone who left the group gets none of its new work.
    s.call("set_world", &json!({ "resources": [
        { "id": "priya", "name": "Priya Shah", "kind": "person" },
        { "id": "tom", "name": "Tom Becker", "kind": "person" },
        { "id": "g1", "name": "Front end", "kind": "group", "members": [{ "person": "priya" }, { "person": "tom", "to": "2026-11-01" }] },
    ] })).unwrap();
    let _ = ids;
    assert_eq!(s.schedule().tasks[&a].by.as_deref(), Some("priya"));
}

#[test]
fn moving_into_an_epic_keeps_the_rank() {
    let (mut s, ids) = website();
    let x = s.call("add_task", &json!({ "title": "Old articles", "who": "tom", "work": D })).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let rank = |s: &Session, id: &str| s.plan.order.iter().position(|y| y == id).unwrap();
    let before = rank(&s, &ids["setup"]) < rank(&s, &x);
    s.call("set_epic", &json!({ "id": x, "parent": ids["content"] })).unwrap();
    assert_eq!(s.plan.tasks[&x].parent.as_deref(), Some(ids["content"].as_str()));
    assert_eq!(rank(&s, &ids["setup"]) < rank(&s, &x), before, "still after Set up hosting");
    assert_eq!(s.plan.log.last().unwrap().what, format!("Moved {} into Content", s.plan.tasks[&x].key));
    s.call("set_epic", &json!({ "id": x, "parent": null })).unwrap();
    assert!(s.plan.log.last().unwrap().what.starts_with("Took "));
}

#[test]
fn a_started_issue_is_booked_only_where_it_is_worked() {
    let (mut s, ids) = website();
    // Started last week: its bar reaches back, its booking doesn't.
    let x = ids["hd"].clone();
    s.call("set_clock", &json!({ "today": "2026-11-04" })).unwrap();
    s.plan.tasks.get_mut(&x).unwrap().started = Some(xenify_holidays::Date::new(2026, 10, 26).unwrap());
    let doing = s.plan.statuses.iter().find(|st| st.id != s.plan.tasks[&x].status).unwrap().id.clone();
    s.plan.tasks.get_mut(&x).unwrap().status = doing;
    let sch = s.schedule();
    let bk = schedule::bookings(&s.plan, &s.world, &sch);
    for b in bk.iter().filter(|b| b.task == x) {
        assert!(b.from.to_string().as_str() >= "2026-11-04", "{b:?}");
    }
}

#[test]
fn holding_back_to_a_weekend_means_the_monday() {
    let (mut s, ids) = website();
    s.call("hold", &json!({ "id": ids["hd"], "start": "2026-11-07" })).unwrap();
    assert_eq!(dates(&s, &ids["hd"]).0, "2026-11-09");
    assert!(s.plan.log.last().unwrap().what.ends_with("no sooner than Mon 9 Nov"), "{}", s.plan.log.last().unwrap().what);
}

#[test]
fn a_person_booked_in_an_earlier_plan_is_waited_for_unless_over_booked() {
    let (mut s, ids) = website();
    let shop = |first: bool, over: bool| json!([{ "resource": "priya", "plan": "Shop", "task": "x", "title": "Basket", "from": "2026-11-02", "to": "2026-11-06", "percent": 100, "first": first, "over": over }]);
    // The shop (made before) has Priya all week: the homepage design waits, and says why.
    s.call("set_world", &json!({ "bookings": shop(true, false) })).unwrap();
    let d = s.schedule().tasks[&ids["hd"]].clone();
    assert_eq!(d.start.to_string(), "2026-11-09");
    assert_eq!(d.clashes.len(), 1);
    assert!(!d.clashes[0].over && d.clashes[0].plan == "Shop" && d.clashes[0].who == "Priya Shah");
    assert!(!d.warnings.iter().any(|w| w.starts_with("Over-booked")));
    // Over-booked: it goes ahead at once, with a warning, and its bookings say so.
    s.call("set_task", &json!({ "id": ids["hd"], "field": "overbook", "value": true })).unwrap();
    let d = s.schedule().tasks[&ids["hd"]].clone();
    assert_eq!(d.start.to_string(), "2026-11-02");
    assert!(
        d.warnings.iter().any(|w| w.starts_with("Over-booked: Priya Shah is also booked in “Shop” (Basket)")),
        "{:?}",
        d.warnings
    );
    let bk: Vec<Booking> = serde_json::from_value(s.call("bookings", &json!({})).unwrap()).unwrap();
    assert!(bk.iter().filter(|b| b.task == ids["hd"]).all(|b| b.over));
    s.call("undo", &json!({})).unwrap();
    // The other way round: a later plan's work doesn't move this one, and
    // only warns here when it's set to go ahead over it.
    s.call("set_world", &json!({ "bookings": shop(false, false) })).unwrap();
    let d = s.schedule().tasks[&ids["hd"]].clone();
    assert_eq!(d.start.to_string(), "2026-11-02");
    assert!(d.clashes.is_empty() && d.warnings.is_empty(), "{:?}", d.warnings);
    s.call("set_world", &json!({ "bookings": shop(false, true) })).unwrap();
    let d = s.schedule().tasks[&ids["hd"]].clone();
    assert_eq!(d.start.to_string(), "2026-11-02");
    assert!(
        d.warnings.iter().any(|w| w.starts_with("Over-booked: Priya Shah is also booked in “Shop”")),
        "{:?}",
        d.warnings
    );
}

#[test]
fn over_booking_is_a_schedule_change() {
    assert_eq!(
        access::need(&ops::Op::SetTask { id: "t".into(), field: "overbook".into(), value: json!(true) }),
        "plan.schedule"
    );
}

#[test]
fn new_issues_get_the_usual_estimate() {
    let (mut s, _) = website();
    let id = s.call("add_task", &json!({ "title": "Tidy up" })).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(s.plan.tasks[&id].work, Some(D as u32));
    // Epics and milestones have none; 0 days in the settings: none either.
    let e = s.call("add_task", &json!({ "title": "Group", "kind": "epic" })).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(s.plan.tasks[&e].work, None);
    s.call("set_plan", &json!({ "field": "settings.default_days", "value": 0.0 })).unwrap();
    let n = s.call("add_task", &json!({ "title": "Later" })).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(s.plan.tasks[&n].work, None);
}

#[test]
fn releasing_takes_the_done_issues() {
    let (mut s, ids) = website();
    assert!(s.call("release", &json!({ "name": "1.0" })).is_err(), "nothing done yet");
    let done = s.plan.statuses.iter().find(|x| x.done).unwrap().id.clone();
    s.call("set_task", &json!({ "id": ids["copy"], "field": "status", "value": done })).unwrap();
    let r = s.call("release", &json!({ "name": "1.0" })).unwrap();
    assert_eq!(r["issues"], 1);
    let rid = r["id"].as_str().unwrap().to_string();
    assert_eq!(s.plan.tasks[&ids["copy"]].release.as_deref(), Some(rid.as_str()));
    assert_eq!(s.plan.releases[0].name, "1.0");
    s.call("release", &json!({ "id": rid, "name": "Spring" })).unwrap();
    assert_eq!(s.plan.releases[0].name, "Spring");
    s.call("undo", &json!({})).unwrap();
    s.call("undo", &json!({})).unwrap();
    assert!(s.plan.releases.is_empty() && s.plan.tasks[&ids["copy"]].release.is_none());
}

#[test]
fn a_started_issue_waiting_on_unfinished_work_says_so() {
    let (mut s, ids) = website();
    let started = s.plan.statuses.iter().find(|x| x.started && !x.done).unwrap().id.clone();
    s.call("set_task", &json!({ "id": ids["build_home"], "field": "status", "value": started })).unwrap();
    let v = s.view();
    let t = v["tasks"].as_array().unwrap().iter().find(|t| t["id"] == ids["build_home"].as_str()).unwrap();
    // Not drawn from today: it starts after what it waits for.
    assert!(t["start"].as_str().unwrap() > "2026-11-02");
    assert!(t["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("right way round")));
}
