//! Xenify Plan's register: people, groups (with join and leave days),
//! things, holidays, time away (Chat's out of office) and every plan's
//! bookings, with titles only for people who may open the plan.

mod common;

use common::*;
use serde_json::{Value, json};

async fn put(addr: &str, path: &str, token: &str, body: Value) -> (u16, Value) {
    call(addr, "PUT", path, Some(token), Some(body)).await
}

fn new_plan(title: &str) -> xenify_plan_core::Session {
    let mut p = xenify_plan_core::Session::new();
    p.call("set_clock", &json!({ "today": "2030-01-01", "seed": 11 })).unwrap();
    p.fresh(title);
    p.call(
        "set_item",
        &json!({ "list": "projects", "id": "p1", "value": { "name": title, "prefix": title[..3].to_uppercase() } }),
    )
    .unwrap();
    p
}

fn add(p: &mut xenify_plan_core::Session, args: Value) -> String {
    let id = p.call("add_task", &args).unwrap()["id"].as_str().unwrap().to_string();
    // Held back (as dragging it on the Timeline does): fixed dates whatever today is.
    if let Some(d) = args.get("start") {
        p.call("hold", &json!({ "id": id, "start": d })).unwrap();
    }
    id
}

/// Saves `plan` as the file's newest version (what Plan's Save does).
async fn save(addr: &str, token: &str, node: i64, plan: &xenify_plan_core::Session) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let bytes = plan.save_file().unwrap();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /v1/nodes/{node}/content HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(&bytes).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn bookings_people_groups_things_holidays_and_time_away() {
    let Some(db) = fresh_db("plan").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design team" })).await;
    let design = g["id"].as_i64().unwrap();
    let mut tokens = Vec::new();
    let mut ids = Vec::new();
    for who in ["priya", "sam"] {
        let (status, p) = post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": who, "display_name": format!("{who} {who}"), "groups": [design] })).await;
        assert_eq!(status, 200, "{p}");
        ids.push(p["id"].as_i64().unwrap());
        let (status, s) = use_link(&addr, p["code"].as_str().unwrap(), who, &format!("{who} password")).await;
        assert_eq!(status, 200, "{s}");
        tokens.push(s["token"].as_str().unwrap().to_string());
    }
    let (priya, sam) = (tokens[0].clone(), tokens[1].clone());
    let (pu, su) = (format!("u{}", ids[0]), format!("u{}", ids[1]));

    // Admin › Holidays; only admins may change it.
    let (status, h) = put(&addr, "/v1/admin/plan/holidays", &admin, json!({ "country": "gb", "region": "ENG", "company_days": [["2026-12-29", "2026-12-31", "Christmas shutdown"]] })).await;
    assert_eq!(status, 200, "{h}");
    assert_eq!(put(&addr, "/v1/admin/plan/holidays", &priya, json!({ "country": "FR" })).await.0, 403);
    // Sam leaves the team on 18 Dec (admins).
    assert_eq!(
        put(&addr, &format!("/v1/plan/groups/g{design}/dates"), &admin, json!({ "person": su, "to": "2026-12-18" }))
            .await
            .0,
        200
    );
    // Priya's own hours and region, and time away; not Sam's.
    assert_eq!(
        put(
            &addr,
            &format!("/v1/plan/people/{pu}"),
            &priya,
            json!({ "day_minutes": 360, "country": "GB", "region": "SCT" })
        )
        .await
        .0,
        200
    );
    assert_eq!(put(&addr, &format!("/v1/plan/people/{su}"), &priya, json!({ "day_minutes": 60 })).await.0, 403);
    let (status, a) =
        put(&addr, &format!("/v1/plan/away/{pu}"), &priya, json!({ "times": [["2026-12-21", "2027-01-01"]] })).await;
    assert_eq!(status, 200, "{a}");
    // A thing.
    let (status, t) = post(&addr, "/v1/plan/things", Some(&sam), json!({ "name": "Test phone A" })).await;
    assert_eq!(status, 200, "{t}");

    // Priya's plan, in her own files: the server works out its bookings.
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").expect("a personal space")["id"]
        .as_i64()
        .unwrap();
    let (status, n) = post(&addr, "/v1/nodes", Some(&priya), json!({ "space": mine, "parent": null, "kind": "file", "meta": json!({ "name": "Website", "app": "plan" }).to_string() })).await;
    assert_eq!(status, 200, "{n}");
    let node = n["id"].as_i64().unwrap();
    let mut plan = new_plan("Website");
    add(&mut plan, json!({ "title": "Page templates", "who": su, "work": 5 * 450, "start": "2030-12-02" }));
    assert_eq!(save(&addr, &priya, node, &plan).await, 200);
    let (status, reg) = get(&addr, "/v1/plan/register", Some(&sam)).await;
    assert_eq!(status, 200, "{reg}");
    let people = reg["people"].as_array().unwrap();
    let p = people.iter().find(|x| x["id"] == json!(pu)).unwrap();
    assert_eq!(p["day_minutes"], 360);
    assert_eq!(p["region"], "SCT");
    assert_eq!(p["away"], json!([["2026-12-21", "2027-01-01"]]), "Chat's out of office, as days");
    let team = reg["groups"].as_array().unwrap().iter().find(|x| x["name"] == "Design team").unwrap();
    let sam_in = team["members"].as_array().unwrap().iter().find(|m| m["person"] == json!(su)).unwrap();
    assert_eq!(sam_in["to"], "2026-12-18");
    assert!(reg["things"].as_array().unwrap().iter().any(|x| x["name"] == "Test phone A"));
    assert_eq!(reg["holidays"]["country"], "GB");
    let (_, days) = get(&addr, "/v1/plan/holidays/2026", Some(&sam)).await;
    let names: Vec<&str> = days.as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Christmas Day") && names.contains(&"Christmas shutdown"), "{names:?}");
    let (_, cs) = get(&addr, "/v1/plan/countries", None).await;
    assert!(cs.as_array().unwrap().len() >= 27);
    let bs = reg["bookings"].as_array().unwrap();
    assert_eq!(bs.len(), 1, "{bs:?}");
    let sams = &bs[0];
    assert_eq!(sams["resource"], json!(su));
    assert_eq!(
        (sams["from"].as_str(), sams["to"].as_str(), sams["percent"].as_i64()),
        (Some("2030-12-02"), Some("2030-12-06"), Some(100)),
        "worked out by the server"
    );
    assert_eq!(sams["title"], "Page templates", "Sam can't open Priya's plan, but it's his own task");
    assert_eq!(sams["readable"], false);
    let (_, reg) = get(&addr, "/v1/plan/register", Some(&priya)).await;
    assert!(reg["bookings"].as_array().unwrap().iter().any(|x| x["title"] == "Page templates"));

    // A second plan wanting Sam that week: it waits for the first (made
    // before it), unless set to go ahead anyway; then it says so, and the
    // first plan is told by name.
    let (_, n2) = post(&addr, "/v1/nodes", Some(&priya), json!({ "space": mine, "parent": null, "kind": "file", "meta": json!({ "name": "Shop", "app": "plan" }).to_string() })).await;
    let shop = n2["id"].as_i64().unwrap();
    let mut plan2 = new_plan("Shop");
    let basket = add(&mut plan2, json!({ "title": "Basket", "who": su, "work": 2 * 450, "start": "2030-12-02" }));
    assert_eq!(save(&addr, &priya, shop, &plan2).await, 200);
    let shop_of = |reg: &Value| -> Vec<Value> {
        reg["bookings"].as_array().unwrap().iter().filter(|b| b["node"] == json!(shop)).cloned().collect()
    };
    let (_, reg) = get(&addr, "/v1/plan/register", Some(&priya)).await;
    let b = shop_of(&reg);
    assert_eq!((b[0]["from"].as_str(), b[0]["over"].as_bool()), (Some("2030-12-09"), Some(false)), "{b:?}");
    let first = reg["bookings"].as_array().unwrap().iter().find(|b| b["node"] == json!(node)).unwrap().clone();
    assert_eq!(first["plan"], "Website", "named for someone who may open it");
    plan2.call("set_task", &json!({ "id": basket, "field": "overbook", "value": true })).unwrap();
    assert_eq!(save(&addr, &priya, shop, &plan2).await, 200);
    let (_, reg) = get(&addr, "/v1/plan/register", Some(&priya)).await;
    let b = shop_of(&reg);
    assert_eq!((b[0]["from"].as_str(), b[0]["over"].as_bool()), (Some("2030-12-02"), Some(true)), "{b:?}");
    let (_, reg) = get(&addr, "/v1/plan/register", Some(&sam)).await;
    assert!(shop_of(&reg).iter().all(|b| b["plan"] == ""), "no plan names for someone who can't open it");

    // Chat's out of office keeps its own reply when Plan changes the days.
    let (c, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    let away: String = c.query_one("SELECT away FROM users WHERE id = $1", &[&ids[0]]).await.unwrap().get(0);
    let v: Value = serde_json::from_str(&away).unwrap();
    assert_eq!(v["times"].as_array().unwrap().len(), 1);
    c.execute(
        "UPDATE users SET away = jsonb_set(away::jsonb, '{text}', '\"Back in January\"')::text WHERE id = $1",
        &[&ids[0]],
    )
    .await
    .unwrap();
    put(&addr, &format!("/v1/plan/away/{pu}"), &priya, json!({ "times": [["2026-12-24", "2026-12-24"]] })).await;
    let away: String = c.query_one("SELECT away FROM users WHERE id = $1", &[&ids[0]]).await.unwrap().get(0);
    assert!(away.contains("Back in January"), "{away}");
}

#[tokio::test]
async fn plans_wait_for_each_other_without_showing_what_you_cant_open() {
    let Some(db) = fresh_db("plan_links").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let mut tok = Vec::new();
    for (who, name) in [("priya", "Priya Shah"), ("sam", "Sam Lee")] {
        let (status, p) = post(
            &addr,
            "/v1/admin/people",
            Some(&admin),
            json!({ "username": who, "display_name": name }),
        )
        .await;
        assert_eq!(status, 200, "{p}");
        let (_, s) = use_link(&addr, p["code"].as_str().unwrap(), who, &format!("{who} password")).await;
        tok.push(s["token"].as_str().unwrap().to_string());
    }
    let (priya, sam) = (tok[0].clone(), tok[1].clone());
    let file_in_own = |token: String, name: &'static str| {
        let addr = addr.clone();
        async move {
            let (_, spaces) = get(&addr, "/v1/spaces", Some(&token)).await;
            let mine =
                spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
            let (_, n) = post(&addr, "/v1/nodes", Some(&token), json!({ "space": mine, "parent": null, "kind": "file", "meta": json!({ "name": name, "app": "plan", "ext": "xplan" }).to_string() })).await;
            n["id"].as_i64().unwrap()
        }
    };
    let marketing = file_in_own(priya.clone(), "Marketing").await;
    let dev = file_in_own(sam.clone(), "Development").await;
    // Marketing: a campaign held back to 18 Nov 2030, three days' work.
    let mut m = new_plan("Marketing");
    let launch = add(
        &mut m,
        json!({ "title": "Launch campaign", "work": 3 * 450, "start": "2030-11-18", "labels": ["Launch"] }),
    );
    assert_eq!(save(&addr, &priya, marketing, &m).await, 200);
    let r = format!("n{marketing}:{launch}");
    // Development's build waits for it (Sam can't open Marketing: someone shared the link with him).
    let mut d = new_plan("Development");
    let build = add(&mut d, json!({ "title": "Build", "work": 2 * 450, "waits_for": [] }));
    d.call("link", &json!({ "from": r, "to": build })).unwrap();
    assert_eq!(save(&addr, &sam, dev, &d).await, 200);

    // Sam can't open Marketing: the issue's key and dates, the plan's name and whom to ask; never its words.
    let (status, o) = post(&addr, "/v1/plan/outside", Some(&sam), json!({ "refs": [r] })).await;
    assert_eq!(status, 200, "{o}");
    assert_eq!(o[&r]["key"], "MAR-1");
    assert_eq!((o[&r]["start"].as_str(), o[&r]["finish"].as_str()), (Some("2030-11-18"), Some("2030-11-20")));
    assert_eq!(o[&r]["readable"], false);
    assert_eq!(o[&r]["plan"], "Marketing", "the name, so Sam knows what to ask for");
    assert_eq!(o[&r]["ask"], "Priya Shah");
    assert!(o[&r].get("title").is_none(), "{o}");
    let (_, o) = post(&addr, "/v1/plan/outside", Some(&priya), json!({ "refs": [r] })).await;
    assert_eq!(o[&r]["title"], "Launch campaign");
    // The server worked Development out after Marketing: Build starts the day after.
    let (_, di) = get(&addr, &format!("/v1/plan/issues/{dev}"), Some(&sam)).await;
    assert_eq!(di["issues"][0]["start"], "2030-11-21", "{di}");
    // Only plans you can open to link to.
    let (_, ps) = get(&addr, "/v1/plan/plans", Some(&sam)).await;
    let names: Vec<&str> = ps.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Development"]);
    assert_eq!(get(&addr, &format!("/v1/plan/issues/{marketing}"), Some(&sam)).await.0, 404);
    // Other files aren't plans: nothing of them, not even their names.
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let (_, n) = post(&addr, "/v1/nodes", Some(&priya), json!({ "space": mine, "parent": null, "kind": "file", "meta": json!({ "name": "Pay rises", "app": "write" }).to_string() })).await;
    let letter = n["id"].as_i64().unwrap();
    let lr = format!("n{letter}:t1");
    let (_, o) = post(&addr, "/v1/plan/outside", Some(&sam), json!({ "refs": [lr] })).await;
    assert_eq!(o[&lr], json!({ "gone": true }));
    // Launch waiting for Build would be a loop: the whole of it comes back.
    let b = format!("n{dev}:{build}");
    let (_, l) = post(&addr, "/v1/plan/loop", Some(&sam), json!({ "from": b, "to": r })).await;
    assert_eq!(l["loop"], true, "{l}");
    let chain: Vec<(&str, &str)> = l["chain"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["key"].as_str().unwrap(), x["plan"].as_str().unwrap()))
        .collect();
    assert_eq!(chain, vec![("DEV-1", "Development"), ("MAR-1", "Marketing")]);
    assert!(l["chain"][1].get("title").is_none(), "Sam still can't read Marketing's words");
    let (_, l) = post(&addr, "/v1/plan/loop", Some(&sam), json!({ "from": r, "to": b })).await;
    assert_eq!(l["loop"], false);
    // Marketing's Launch holds up Development's Build: Priya sees that, but not Build's words.
    let (_, w) = get(&addr, &format!("/v1/plan/waiting/{marketing}"), Some(&priya)).await;
    assert_eq!(w[&launch][0]["key"], "DEV-1", "{w}");
    assert_eq!((w[&launch][0]["readable"].clone(), w[&launch][0]["ask"].clone()), (json!(false), json!("Sam Lee")));
    assert!(w[&launch][0].get("title").is_none());
    assert_eq!(
        get(&addr, &format!("/v1/plan/waiting/{marketing}"), Some(&sam)).await.0,
        404,
        "only for a plan you can open"
    );
    // A link to a plan you can't open: its name and whom to ask.
    let (_, a) = get(&addr, &format!("/v1/plan/ask/{marketing}"), Some(&sam)).await;
    assert_eq!((a["name"].as_str(), a["ask"].as_str()), (Some("Marketing"), Some("Priya Shah")));
}

#[tokio::test]
async fn mirrors_and_whole_plans() {
    let Some(db) = fresh_db("plan_whole").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let file = |name: &'static str| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move {
            let (_, n) = post(&addr, "/v1/nodes", Some(&admin), json!({ "space": mine, "parent": null, "kind": "file", "meta": json!({ "name": name, "app": "plan", "ext": "xplan" }).to_string() })).await;
            n["id"].as_i64().unwrap()
        }
    };
    let (web, shop, q4) = (file("Website").await, file("Shop").await, file("Q4").await);
    let mut w = new_plan("Website");
    let design = add(&mut w, json!({ "title": "Design", "work": 3 * 450, "start": "2030-11-18" }));
    add(&mut w, json!({ "title": "Build", "work": 2 * 450, "start": "2030-11-25" }));
    assert_eq!(save(&addr, &admin, web, &w).await, 200);
    let mut sh = new_plan("Shop");
    let basket = add(&mut sh, json!({ "title": "Basket", "work": 2 * 450, "start": "2030-11-18" }));
    assert_eq!(save(&addr, &admin, shop, &sh).await, 200);
    // The whole of Website: first start to last finish.
    let wr = format!("n{web}:*");
    let (_, o) = post(&addr, "/v1/plan/outside", Some(&admin), json!({ "refs": [wr] })).await;
    assert_eq!((o[&wr]["start"].as_str(), o[&wr]["finish"].as_str(), o[&wr]["title"].as_str()), (Some("2030-11-18"), Some("2030-11-26"), Some("Website")), "{o}");
    let (_, ps) = get(&addr, "/v1/plan/plans", Some(&admin)).await;
    assert!(ps.as_array().unwrap().iter().any(|p| p["name"] == "Website" && p["issues"] == 2), "the whole-plan row isn't an issue: {ps}");
    let (_, wi) = get(&addr, &format!("/v1/plan/issues/{web}"), Some(&admin)).await;
    assert_eq!(wi["issues"].as_array().unwrap().len(), 2);

    // Q4 mirrors all of Website and Shop's Basket: their dates, nothing moved.
    let mut p = new_plan("Q4 plan");
    let mw = p.call("add_mirror", &json!({ "ref": wr, "title": "Website" })).unwrap()["id"].as_str().unwrap().to_string();
    let mb = p.call("add_mirror", &json!({ "ref": format!("n{shop}:{basket}"), "title": "Basket" })).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(save(&addr, &admin, q4, &p).await, 200);
    let (_, qi) = get(&addr, &format!("/v1/plan/issues/{q4}"), Some(&admin)).await;
    let row = |v: &serde_json::Value, id: &str| v["issues"].as_array().unwrap().iter().find(|x| x["task"] == id).unwrap().clone();
    assert_eq!((row(&qi, &mw)["start"].as_str(), row(&qi, &mw)["finish"].as_str()), (Some("2030-11-18"), Some("2030-11-26")), "{qi}");
    assert_eq!(row(&qi, &mb)["start"], "2030-11-18");
    let (_, si) = get(&addr, &format!("/v1/plan/issues/{shop}"), Some(&admin)).await;
    assert_eq!(si["issues"][0]["start"], "2030-11-18", "mirroring moves nothing");
    // Website moves: the mirror follows.
    let mut w2 = new_plan("Website");
    add(&mut w2, json!({ "title": "Design", "work": 3 * 450, "start": "2030-12-02" }));
    assert_eq!(save(&addr, &admin, web, &w2).await, 200);
    let (_, qi) = get(&addr, &format!("/v1/plan/issues/{q4}"), Some(&admin)).await;
    assert_eq!(row(&qi, &mw)["start"], "2030-12-02", "{qi}");

    // A loop made anyway (Shop's Basket after all of Website, Website's Design
    // after all of Shop): the links that close it are left out, and nothing
    // keeps moving later and later.
    let _ = design;
    let d2 = w2.view()["tasks"][0]["id"].as_str().unwrap().to_string();
    sh.call("link", &json!({ "from": wr, "to": basket })).unwrap();
    assert_eq!(save(&addr, &admin, shop, &sh).await, 200);
    let (_, l) = post(&addr, "/v1/plan/loop", Some(&admin), json!({ "from": format!("n{shop}:*"), "to": format!("n{web}:{d2}") })).await;
    assert_eq!(l["loop"], true, "{l}");
    w2.call("link", &json!({ "from": format!("n{shop}:*"), "to": d2 })).unwrap();
    assert_eq!(save(&addr, &admin, web, &w2).await, 200);
    let mut seen = Vec::new();
    for _ in 0..4 {
        let (_, si) = get(&addr, &format!("/v1/plan/issues/{shop}"), Some(&admin)).await;
        let (_, wi) = get(&addr, &format!("/v1/plan/issues/{web}"), Some(&admin)).await;
        seen.push((si["issues"][0]["start"].clone(), wi["issues"][0]["start"].clone()));
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    }
    assert!(seen.windows(2).all(|x| x[0] == x[1]), "the dates settle: {seen:?}");
    let (_, o) = post(&addr, "/v1/plan/outside", Some(&admin), json!({ "refs": [format!("n{shop}:*")], "node": web })).await;
    assert_eq!(o[&format!("n{shop}:*")]["looped"], true, "{o}");
}

/// Not a test: a server with Alex (admin), Priya and Sam signed in, for
/// trying Plan's screens against it. Writes {addr, people: {name: session}}
/// to $PLAN_FIXTURE_OUT and keeps running for half an hour.
/// cargo test -p xenify-server --test plan -- --ignored plan_ui_fixture
#[tokio::test]
#[ignore]
async fn plan_ui_fixture() {
    let db = fresh_db("planui").await.expect("PostgreSQL");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design team" })).await;
    let design = g["id"].as_i64().unwrap();
    let mut out = json!({ "addr": addr, "people": {} });
    for (who, name) in [("priya", "Priya Shah"), ("sam", "Sam Lee"), ("noor", "Noor Khan")] {
        let groups = if who == "sam" { json!([]) } else { json!([design]) };
        let (_, p) = post(
            &addr,
            "/v1/admin/people",
            Some(&admin),
            json!({ "username": who, "display_name": name, "groups": groups }),
        )
        .await;
        let (_, s) = use_link(&addr, p["code"].as_str().unwrap(), who, &format!("{who} password")).await;
        out["people"][who] = json!({ "token": s["token"], "account": s["account"] });
    }
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    out["people"]["alex"] = json!({ "token": admin, "account": me });
    put(&addr, "/v1/admin/plan/holidays", &admin, json!({ "country": "GB", "region": "ENG", "company_days": [["2026-12-29", "2026-12-31", "Christmas shutdown"]] })).await;
    let path = std::env::var("PLAN_FIXTURE_OUT").unwrap_or_else(|_| "plan-fixture.json".into());
    std::fs::write(&path, out.to_string()).unwrap();
    eprintln!("fixture at {path}: {out}");
    tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
}
