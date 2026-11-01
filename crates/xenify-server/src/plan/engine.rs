//! The server's own copy of every plan: the latest saved version and the
//! changes made since (editing together), put through Plan's engine. From
//! it the server works out each plan's issues and dates (the index other
//! plans wait for and programs add up) and its bookings (who is busy when),
//! instead of taking the apps' word for it. A plan is worked out again when
//! it changes, when a plan it waits for has, and now and then (people's time
//! away and the holidays move dates too).

use crate::Server;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use xenify_plan_core::{Session, Step, ops};

/// Whether the Plan steps `sent` (to go after change `base`) change someone
/// else's comments, add one in another's name, or delete another's without
/// `editor` (may edit the plan): comments are only ever their author's
/// (`me`: "u<account>"). Only read from the database when a step touches
/// comments.
pub async fn comments_refused(s: &Server, node: i64, base: i64, sent: &[String], me: &str, editor: bool) -> bool {
    let steps: Vec<Step> = sent.iter().filter_map(|d| serde_json::from_str(d).ok()).collect();
    let touches = |st: &Step| st.ops.iter().any(|o| matches!(o, ops::Op::SetTask { field, .. } if field == "comments"));
    if !steps.iter().any(touches) {
        return false;
    }
    let Ok((mut plan, mut known)) = plan_at(s, node, base).await else { return true };
    for st in &steps {
        for op in &st.ops {
            if let ops::Op::SetTask { id, field, value } = op
                && field == "comments"
            {
                let before = plan.tasks.get(id).map(|t| t.comments.clone()).unwrap_or_default();
                let Ok(after) = serde_json::from_value::<Vec<xenify_plan_core::model::Comment>>(value.clone()) else {
                    return true;
                };
                if !xenify_plan_core::access::comments_ok(&before, &after, me, editor, &known) {
                    return true;
                }
                known.extend(after);
            }
            ops::apply_lenient(&mut plan, std::slice::from_ref(op));
        }
    }
    false
}

/// A plan as it was after change `upto`: the version in use, then the
/// changes since it up to `upto`; and every comment it has had on the way
/// (a deleted one may be put back as it was).
async fn plan_at(
    s: &Server,
    node: i64,
    upto: i64,
) -> Result<(xenify_plan_core::model::Plan, Vec<xenify_plan_core::model::Comment>), String> {
    let e = |e: tokio_postgres::Error| e.to_string();
    let c = s.pool().get().await.map_err(|e| e.to_string())?;
    // As rebuild does: the version in use (head), then the changes after it.
    let version = c
        .query_opt(
            "SELECT v.blob, v.upto FROM nodes n JOIN versions v ON v.node = n.id AND v.n = n.head WHERE n.id = $1",
            &[&node],
        )
        .await
        .map_err(e)?;
    let mut p = Session::new();
    let from = match version {
        Some(v) => {
            let bytes = s.blobs.get(v.get::<_, &str>(0)).await.map_err(|e| e.to_string())?;
            p.open_file(&bytes)?;
            v.get::<_, i64>(1)
        }
        None => {
            p.fresh("");
            0
        }
    };
    let rows = c
        .query("SELECT data FROM ops WHERE node = $1 AND seq > $2 AND seq <= $3 ORDER BY seq", &[&node, &from, &upto])
        .await
        .map_err(e)?;
    let mut known: Vec<xenify_plan_core::model::Comment> =
        p.plan.tasks.values().flat_map(|t| t.comments.iter().cloned()).collect();
    for r in rows {
        if let Ok(step) = serde_json::from_str::<Step>(r.get::<_, &str>(0)) {
            for op in &step.ops {
                if let ops::Op::SetTask { field, value, .. } = op
                    && field == "comments"
                    && let Ok(list) = serde_json::from_value::<Vec<xenify_plan_core::model::Comment>>(value.clone())
                {
                    known.extend(list);
                }
            }
            ops::apply_lenient(&mut p.plan, &step.ops);
        }
    }
    Ok((p.plan, known))
}

/// Worked out again at least this often (time away, holidays, a new day).
const STALE_SECS: i64 = 600;

/// The plans on the server (files Plan made), with where each one is now.
async fn plan_nodes(
    c: &deadpool_postgres::Client,
    only: Option<&[i64]>,
) -> Result<Vec<(i64, i64, i64)>, tokio_postgres::Error> {
    let rows = c
        .query(
            "SELECT id, head, seq FROM nodes WHERE kind = 'file' AND trashed IS NULL AND meta LIKE '%\"app\":\"plan\"%'
             AND ($1::BIGINT[] IS NULL OR id = ANY($1))",
            &[&only.map(|x| x.to_vec())],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

/// Which of `nodes` need working out again: changed since, a plan they wait for was
/// worked out after them, or not for a while.
async fn stale(c: &deadpool_postgres::Client, nodes: &[(i64, i64, i64)]) -> Result<Vec<i64>, tokio_postgres::Error> {
    let mut out = Vec::new();
    for &(node, head, seq) in nodes {
        let row = c
            .query_opt(
                "SELECT head, seq, extract(epoch FROM now() - built)::BIGINT, built FROM plan_state WHERE node = $1",
                &[&node],
            )
            .await?;
        let Some(row) = row else {
            out.push(node);
            continue;
        };
        let (h, s, age): (i64, i64, i64) = (row.get(0), row.get(1), row.get(2));
        let built: std::time::SystemTime = row.get(3);
        if h != head || s != seq || age > STALE_SECS {
            out.push(node);
            continue;
        }
        // A plan it waits for (or mirrors) came out differently since.
        let newer = c
            .query_opt(
                "SELECT 1 FROM plan_state p WHERE p.changed > $2 AND p.node <> $1 AND (p.node IN (
                   SELECT DISTINCT substring(w FROM '^n([0-9]+):')::BIGINT FROM plan_issues i, jsonb_array_elements_text(i.waits::jsonb) w
                   WHERE i.node = $1 AND w ~ '^n[0-9]+:')) LIMIT 1",
                &[&node, &built],
            )
            .await?;
        if newer.is_some() {
            out.push(node);
        }
    }
    Ok(out)
}

/// Today on the server (UTC).
fn today() -> String {
    let ms =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    super::ms_day(ms)
}

/// Works out plan `node` from its saved version and changes, and keeps its
/// index and bookings. Ok(false) when it isn't a plan (or can't be read).
pub async fn rebuild(s: &Server, node: i64) -> Result<bool, String> {
    let e = |e: tokio_postgres::Error| e.to_string();
    let c = s.pool().get().await.map_err(|e| e.to_string())?;
    let Some(n) =
        c.query_opt("SELECT head, seq FROM nodes WHERE id = $1 AND trashed IS NULL", &[&node]).await.map_err(e)?
    else {
        return Ok(false);
    };
    let (head, seq): (i64, i64) = (n.get(0), n.get(1));
    let version =
        c.query_opt("SELECT blob, upto FROM versions WHERE node = $1 AND n = $2", &[&node, &head]).await.map_err(e)?;
    let (bytes, upto) = match version {
        Some(v) => (Some(s.blobs.get(v.get::<_, &str>(0)).await.map_err(|e| e.to_string())?), v.get::<_, i64>(1)),
        None => (None, 0),
    };
    let changes: Vec<String> = c
        .query("SELECT data FROM ops WHERE node = $1 AND seq > $2 ORDER BY seq", &[&node, &upto])
        .await
        .map_err(e)?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let (people, groups, things) = super::register_world(&c).await.map_err(e)?;
    let name = c
        .query_opt("SELECT meta FROM nodes WHERE id = $1", &[&node])
        .await
        .map_err(e)?
        .map(|r| super::plan_name(&r.get::<_, String>(0)))
        .unwrap_or_default();
    let holidays = super::holidays(&c).await;
    // What it waits for in other plans, and other plans' bookings (people's time taken).
    let outside: HashMap<String, Value> = c
        .query(&format!("SELECT {} FROM plan_issues WHERE node <> $1", super::ISSUE_COLS), &[&node])
        .await
        .map_err(e)?
        .iter()
        .map(|r| (format!("n{}:{}", r.get::<_, i64>("node"), r.get::<_, String>("task")), super::issue_json(r, true)))
        .collect();
    let bookings: Vec<Value> = c
        .query(
            "SELECT b.resource, b.task, b.from_day, b.to_day, b.percent, b.node, b.over, b.title, n.meta
             FROM plan_bookings b JOIN nodes n ON n.id = b.node WHERE b.node <> $1",
            &[&node],
        )
        .await
        .map_err(e)?
        .iter()
        .map(|r| {
            json!({
                "resource": r.get::<_, String>(0), "task": format!("{}:{}", r.get::<_, i64>(5), r.get::<_, String>(1)),
                "from": r.get::<_, String>(2), "to": r.get::<_, String>(3), "percent": r.get::<_, i32>(4),
                "first": r.get::<_, i64>(5) < node, "over": r.get::<_, bool>(6), "title": r.get::<_, String>(7),
                "plan": super::plan_name(&r.get::<_, String>(8)),
            })
        })
        .collect();

    // Links that come back round to this plan (a loop through other plans):
    // left out of the dates, with a warning, so they can't push each other
    // later every time the plans are worked out.
    // Reading and working out a plan takes a while for a big one: done on a
    // thread for such work, not on the server's own threads.
    let fail = |e: tokio::task::JoinError| e.to_string();
    let (wanted, bytes, changes) = tokio::task::spawn_blocking(move || {
        let mut p = Session::new();
        let ok = match &bytes {
            Some(b) => p.open_file(b).is_ok(),
            None => {
                p.fresh("");
                true
            }
        };
        if ok {
            for d in &changes {
                if let Ok(step) = serde_json::from_str::<Step>(d) {
                    ops::apply_lenient(&mut p.plan, &step.ops);
                }
            }
        }
        let wanted = p
            .call("outside_refs", &json!({}))
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect::<HashSet<String>>();
        (wanted, bytes, changes)
    })
    .await
    .map_err(fail)?;
    let mut looped: HashSet<String> = HashSet::new();
    for r in &wanted {
        if super::loops_back(&c, r, node).await {
            looped.insert(r.clone());
        }
    }

    // The engine's part (no waiting in here).
    let worked_out = tokio::task::spawn_blocking(move || -> Result<Option<(Vec<Value>, Value)>, String> {
        let mut p = Session::new();
        p.call("set_clock", &json!({ "today": today(), "now": 0 }))?;
        match &bytes {
            Some(b) => {
                if p.open_file(b).is_err() {
                    return Ok(None);
                }
            }
            None => p.fresh(""),
        }
        for d in &changes {
            if let Ok(step) = serde_json::from_str::<Step>(d) {
                ops::apply_lenient(&mut p.plan, &step.ops);
            }
        }
        let resources: Vec<Value> = people.iter().chain(groups.iter()).chain(things.iter()).cloned().collect();
        let company = holidays["company_days"].clone();
        let refs: HashSet<String> = p.call("outside_refs", &json!({}))?
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        let outside: serde_json::Map<String, Value> = refs
            .iter()
            .filter_map(|r| outside.get(r).map(|v| (r.clone(), v.clone())))
            .map(|(r, mut v)| {
                if looped.contains(&r) {
                    v["looped"] = json!(true);
                }
                (r, v)
            })
            .collect();
        p.call("set_world", &json!({ "resources": resources, "holidays": holidays, "company_days": company, "outside": outside, "bookings": bookings }))?;
        let v = p.view();
        let statuses = v["statuses"].as_array().cloned().unwrap_or_default();
        let tasks = v["tasks"].as_array().cloned().unwrap_or_default();
        let titles: HashMap<&str, &str> =
            tasks.iter().filter_map(|x| Some((x["id"].as_str()?, x["title"].as_str().unwrap_or("")))).collect();
        let title = |id: &Value| id.as_str().and_then(|id| titles.get(id)).copied().unwrap_or("").to_string();
        let waits_of = xenify_plan_core::Waits::new(&p.plan);
        let mut issues: Vec<Value> = tasks
            .iter()
            .map(|t| {
                let placed = t["placed"].as_bool().unwrap_or(false);
                let mirror = t["mirror"].as_str().map(String::from);
                // Its epics' links count too (everything in an epic waits for what it waits for).
                let mut waits: Vec<String> = waits_of.of(t["id"].as_str().unwrap_or(""))
                    .into_iter()
                    .map(|x| if xenify_plan_core::is_outside(&x) { x } else { format!("n{node}:{x}") })
                    .collect();
                // A mirror follows what it mirrors (worked out again when that changes).
                if let Some(r) = mirror {
                    waits.push(r);
                }
                json!({
                    "task": t["id"], "key": t["key"], "title": t["title"],
                    "epic": if t["epic"].as_bool().unwrap_or(false) { "(epic)".to_string() } else if t["parent"].is_string() { title(&t["parent"]) } else { String::new() },
                    "placed": placed, "start": if placed { t["start"].clone() } else { json!("") }, "finish": if placed { t["finish"].clone() } else { json!("") },
                    "milestone": t["milestone"], "done": t["is_done"], "work": if t["epic"].as_bool().unwrap_or(false) { t["total"].clone() } else { t["work"].clone() },
                    "status": statuses.iter().find(|s| s["id"] == t["status"]).and_then(|s| s["name"].as_str()).unwrap_or(""),
                    "labels": t["labels"], "who": t["by"].as_str().or(t["who"].as_str()).unwrap_or(""), "waits": waits,
                })
            })
            .collect();
        // The plan as a whole ("n<plan>:*"): first start to last finish of its
        // issues, done when they all are, waiting for whatever they wait for
        // outside it.
        let real: Vec<&Value> = issues.iter().filter(|x| x["epic"] != json!("(epic)")).collect();
        let placed: Vec<&&Value> = real.iter().filter(|x| x["placed"] == json!(true)).collect();
        let day = |x: &Value, k: &str| x[k].as_str().filter(|d| !d.is_empty()).map(String::from);
        let start = placed.iter().filter_map(|x| day(x, "start")).min();
        let finish = placed.iter().filter_map(|x| day(x, "finish")).max();
        let mut waits: Vec<String> = Vec::new();
        for x in &real {
            for w in x["waits"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if !w.starts_with(&format!("n{node}:")) && !waits.iter().any(|y| y == w) {
                    waits.push(w.to_string());
                }
            }
        }
        issues.push(json!({
            "task": "*", "key": "", "title": name, "epic": "(plan)", "placed": start.is_some() && finish.is_some(),
            "start": start.unwrap_or_default(), "finish": finish.unwrap_or_default(), "milestone": false,
            "done": !real.is_empty() && real.iter().all(|x| x["done"] == json!(true)),
            "work": real.iter().filter_map(|x| x["work"].as_i64()).sum::<i64>(), "status": "", "labels": [], "who": "", "waits": waits,
        }));
        let mine = p.call("bookings", &json!({}))?;
        Ok(Some((issues, mine)))
    })
    .await
    .map_err(fail)??;
    let Some((issues, mine)) = worked_out else { return Ok(false) };

    let mut c = c;
    let tx = c.transaction().await.map_err(e)?;
    tx.execute("DELETE FROM plan_issues WHERE node = $1", &[&node]).await.map_err(e)?;
    for (rank, x) in issues.iter().enumerate() {
        let s = |k: &str| x[k].as_str().unwrap_or("").to_string();
        tx.execute(
            "INSERT INTO plan_issues (node, task, key, title, epic, placed, start_day, finish_day, milestone, done, work, status, labels, who, rank, waits)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) ON CONFLICT (node, task) DO NOTHING",
            &[&node, &s("task"), &s("key"), &s("title"), &s("epic"), &x["placed"].as_bool().unwrap_or(false), &s("start"), &s("finish"),
              &x["milestone"].as_bool().unwrap_or(false), &x["done"].as_bool().unwrap_or(false), &x["work"].as_i64().map(|w| w as i32), &s("status"),
              &x["labels"].to_string(), &s("who"), &(rank as i32), &x["waits"].to_string()],
        )
        .await
        .map_err(e)?;
    }
    tx.execute("DELETE FROM plan_bookings WHERE node = $1", &[&node]).await.map_err(e)?;
    for b in mine.as_array().into_iter().flatten() {
        let s = |k: &str| b[k].as_str().unwrap_or("").to_string();
        tx.execute(
            "INSERT INTO plan_bookings (node, resource, task, from_day, to_day, percent, title, over) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (node, resource, task, from_day) DO UPDATE SET to_day = $5, percent = $6, title = $7, over = $8",
            &[&node, &s("resource"), &s("task"), &s("from"), &s("to"), &(b["percent"].as_i64().unwrap_or(0) as i32), &s("title"),
              &b["over"].as_bool().unwrap_or(false)],
        )
        .await
        .map_err(e)?;
    }
    // Plans waiting for this one go again only when what they read changed.
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&issues).unwrap_or_default().hash(&mut h);
        format!("{:016x}", h.finish())
    };
    tx.execute(
        "INSERT INTO plan_state (node, head, seq, built, hash, changed) VALUES ($1, $2, $3, now(), $4, now())
         ON CONFLICT (node) DO UPDATE SET head = $2, seq = $3, built = now(), hash = $4,
           changed = CASE WHEN plan_state.hash = $4 THEN plan_state.changed ELSE now() END",
        &[&node, &head, &seq, &hash],
    )
    .await
    .map_err(e)?;
    tx.commit().await.map_err(e)?;
    Ok(true)
}

/// Something every plan depends on changed (holidays, someone's hours,
/// time away or team dates): every plan is worked out again, now.
pub async fn world_changed(s: &Server) {
    if let Ok(c) = s.pool().get().await {
        let _ = c.execute("UPDATE plan_state SET built = 'epoch'", &[]).await;
    }
    let s = s.clone();
    tokio::spawn(async move { refresh(&s, None).await });
}

/// Works out again whatever needs it (of `only`, or every plan), then the
/// plans waiting for those, a few rounds deep.
pub async fn refresh(s: &Server, only: Option<&[i64]>) {
    // The first time since this server started: every plan is worked out
    // again (an updated server may work dates and bookings out differently).
    static FRESH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    if FRESH.swap(false, std::sync::atomic::Ordering::SeqCst)
        && let Ok(c) = s.pool().get().await
    {
        let _ = c.execute("UPDATE plan_state SET built = 'epoch'", &[]).await;
    }
    ensure_worker(s);
    // Plans edited since they were last worked out go first, whichever
    // were asked about: a program's new link changes the plans in it.
    if only.is_some()
        && let Ok(c) = s.pool().get().await
        && let Ok(rows) = c
            .query(
                "SELECT n.id FROM nodes n LEFT JOIN plan_state p ON p.node = n.id
                 WHERE n.kind = 'file' AND n.trashed IS NULL AND n.meta LIKE '%\"app\":\"plan\"%'
                   AND (p.node IS NULL OR p.head <> n.head OR p.seq <> n.seq)",
                &[],
            )
            .await
    {
        drop(c);
        for n in rows.iter().map(|r| r.get::<_, i64>(0)) {
            let _ = rebuild(s, n).await;
        }
    }
    for _ in 0..6 {
        let Ok(c) = s.pool().get().await else { return };
        let Ok(nodes) = plan_nodes(&c, only).await else { return };
        let Ok(todo) = stale(&c, &nodes).await else { return };
        drop(c);
        if todo.is_empty() {
            return;
        }
        for n in todo {
            let _ = rebuild(s, n).await;
        }
        // Then the plans that wait for those (not only the ones asked about).
        if only.is_some() {
            Box::pin(refresh_dependents(s)).await;
            return;
        }
    }
}

async fn refresh_dependents(s: &Server) {
    for _ in 0..6 {
        let Ok(c) = s.pool().get().await else { return };
        let Ok(nodes) = plan_nodes(&c, None).await else { return };
        let Ok(todo) = stale(&c, &nodes).await else { return };
        drop(c);
        if todo.is_empty() {
            return;
        }
        for n in todo {
            let _ = rebuild(s, n).await;
        }
    }
}

/// Keeps every plan worked out in the background (started by the first Plan call).
fn ensure_worker(s: &Server) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        loop {
            refresh(&s, None).await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}
