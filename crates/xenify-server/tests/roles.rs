//! Roles: server permissions (Everyone and custom roles, through people and
//! groups) and document roles that limit which Plan changes the server takes.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open(addr: &str, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/live")).await.unwrap();
    ws.send(Message::Text(json!({ "t": "hello", "token": token }).to_string().into())).await.unwrap();
    next_of(&mut ws, "welcome").await;
    ws
}

async fn next_of(ws: &mut Ws, t: &str) -> Value {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("no answer").unwrap().unwrap();
        if let Message::Text(x) = m {
            let v: Value = serde_json::from_str(&x).unwrap();
            if v["t"] == t || v["t"] == "error" {
                return v;
            }
        }
    }
}

async fn person(addr: &str, admin: &str, name: &str, groups: &[i64]) -> (i64, String) {
    let (_, p) = post(
        addr,
        "/v1/admin/people",
        Some(admin),
        json!({ "username": name, "groups": groups }),
    )
    .await;
    let (_, s) = use_link(addr, p["code"].as_str().unwrap(), name, &format!("{name} password")).await;
    (p["id"].as_i64().unwrap_or_default(), s["token"].as_str().unwrap().to_string())
}

/// One Plan step as the app sends it.
fn step(ops: Value) -> String {
    json!({ "at": 1, "who": "", "what": "t", "ops": ops }).to_string()
}

#[tokio::test]
async fn roles_decide_what_people_may_do() {
    let Some(db) = fresh_db("roles").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Leads" })).await;
    let leads = g["id"].as_i64().unwrap();
    let (ann_id, ann) = person(&addr, &admin, "ann", &[leads]).await;
    let (ben_id, ben) = person(&addr, &admin, "ben", &[]).await;

    // Everyone: AI and the rest, but not shared spaces.
    let (_, p) = get(&addr, "/v1/me/perms", Some(&ben)).await;
    let perms: Vec<&str> = p["perms"].as_array().unwrap().iter().filter_map(|x| x.as_str()).collect();
    assert!(perms.contains(&"ai.text") && perms.contains(&"chat.create_channel"), "{p}");
    assert!(!perms.contains(&"files.spaces") && !perms.contains(&"chat.moderate"));
    let (status, _) = post(&addr, "/v1/spaces", Some(&ben), json!({ "name": "Ben's" })).await;
    assert_eq!(status, 403);

    // A role for the Leads group: shared spaces and other people's hours.
    let (status, r) = post(&addr, "/v1/admin/roles", Some(&admin), json!({ "name": "Team lead", "kind": "server", "perms": ["files.spaces", "plan.people"], "members": [{ "group": leads }] })).await;
    assert_eq!(status, 200, "{r}");
    let (status, _) = post(&addr, "/v1/spaces", Some(&ann), json!({ "name": "Launch" })).await;
    assert_eq!(status, 200, "a lead may");
    // A change with someone who isn't here changes nothing at all.
    let lead = r["id"].as_i64().unwrap();
    let (status, _) = post(
        &addr,
        &format!("/v1/admin/roles/{lead}"),
        Some(&admin),
        json!({ "perms": [], "members": [{ "user": 999999 }] }),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = post(&addr, "/v1/spaces", Some(&ann), json!({ "name": "Launch 2" })).await;
    assert_eq!(status, 200, "still a lead, still allowed");
    let (status, _) =
        post(&addr, "/v1/admin/roles", Some(&ben), json!({ "name": "Mine", "perms": ["files.spaces"] })).await;
    assert_eq!(status, 403, "only admins make roles");
    let (_, bad) = post(&addr, "/v1/admin/roles", Some(&admin), json!({ "name": "Odd", "perms": ["files.fly"] })).await;
    assert_eq!(bad["code"], "unknown_perm");

    // Taking AI away from Everyone stops it for Ben.
    let (_, list) = get(&addr, "/v1/roles", Some(&admin)).await;
    let everyone =
        list["roles"].as_array().unwrap().iter().find(|r| r["everyone"] == true).unwrap()["id"].as_i64().unwrap();
    post(&addr, &format!("/v1/admin/roles/{everyone}"), Some(&admin), json!({ "perms": ["files.public_links"] })).await;
    let (status, body) = post(&addr, "/v1/ai/chat/completions", Some(&ben), json!({ "messages": [] })).await;
    assert_eq!(status, 403, "{body}");
    let (status, _) = call(&addr, "DELETE", &format!("/v1/admin/roles/{everyone}"), Some(&admin), None).await;
    assert_eq!(status, 400, "Everyone stays");

    // A document role for Plan: comment and create, nothing else.
    let (_, r) = post(&addr, "/v1/admin/roles", Some(&admin), json!({ "name": "Reporter", "kind": "document", "base": "commenter", "perms": ["plan.comment", "plan.create"] })).await;
    let reporter = r["id"].as_i64().unwrap();
    let (_, list) = get(&addr, "/v1/roles", Some(&ben)).await;
    assert!(
        list["roles"].as_array().unwrap().iter().all(|r| r["kind"] == "document"),
        "people see document roles only"
    );
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": "Plans" })).await;
    let space = sp["id"].as_i64().unwrap();
    let (status, _) = post(
        &addr,
        &format!("/v1/spaces/{space}/access"),
        Some(&admin),
        json!({ "grants": [
        { "who": { "user": ann_id }, "role": "editor" },
        { "who": "everyone", "role": "viewer", "custom": reporter },
    ] }),
    )
    .await;
    assert!(status < 300);
    let (_, access) = get(&addr, &format!("/v1/spaces/{space}/access"), Some(&admin)).await;
    let g = access["grants"].as_array().unwrap().iter().find(|g| g["custom"] == reporter).unwrap();
    assert_eq!(g["role"], "commenter", "a document role's grant is its base");
    let (_, f) = post(
        &addr,
        "/v1/nodes",
        Some(&admin),
        json!({ "space": space, "kind": "file", "meta": "{\"name\":\"Launch\",\"app\":\"plan\"}" }),
    )
    .await;
    let plan = f["id"].as_i64().unwrap();

    let mut w = open(&addr, &ben).await;
    w.send(Message::Text(json!({ "t": "join", "node": plan, "since": 0 }).to_string().into())).await.unwrap();
    let joined = next_of(&mut w, "joined").await;
    assert_eq!(joined["may"], json!(["plan.comment", "plan.create"]), "{joined}");
    // A new issue (and its details, in the same step): yes.
    let add = step(
        json!([{ "op": "add_task", "task": { "id": "t1", "title": "Broken link" }, "before": null }, { "op": "set_task", "id": "t1", "field": "labels", "value": ["web"] }]),
    );
    w.send(Message::Text(
        json!({ "t": "send", "node": plan, "base": 0, "client": "b1", "ops": [add] }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_of(&mut w, "accepted").await["t"], "accepted");
    // Renaming it afterwards: no.
    let rename = step(json!([{ "op": "set_task", "id": "t1", "field": "title", "value": "Mine now" }]));
    w.send(Message::Text(
        json!({ "t": "send", "node": plan, "base": 1, "client": "b2", "ops": [rename] }).to_string().into(),
    ))
    .await
    .unwrap();
    let e = next_of(&mut w, "accepted").await;
    assert_eq!(e["code"], "not_allowed", "{e}");
    assert!(e["error"].as_str().unwrap().contains("edit issues"));
    // A comment of his own: yes. In someone else's name: no.
    let bens = json!({ "id": "c1", "at": 5, "who": "ben", "by": format!("u{ben_id}"), "html": "Seen on mobile" });
    let fake = step(
        json!([{ "op": "set_task", "id": "t1", "field": "comments", "value": [{ "id": "c0", "who": "ann", "by": format!("u{ann_id}"), "html": "Ship it" }] }]),
    );
    w.send(Message::Text(
        json!({ "t": "send", "node": plan, "base": 1, "client": "b3", "ops": [fake] }).to_string().into(),
    ))
    .await
    .unwrap();
    let e = next_of(&mut w, "accepted").await;
    assert_eq!(e["code"], "not_allowed", "{e}");
    assert!(e["error"].as_str().unwrap().contains("your own comments"), "{e}");
    let comment = step(json!([{ "op": "set_task", "id": "t1", "field": "comments", "value": [bens] }]));
    w.send(Message::Text(
        json!({ "t": "send", "node": plan, "base": 1, "client": "b3b", "ops": [comment] }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_of(&mut w, "accepted").await["t"], "accepted");
    // Ann edits the plan: Ben's words stay his (she can't change them), but
    // she may delete his comment, and put it back (Undo) only as it was.
    let mut wa = open(&addr, &ann).await;
    wa.send(Message::Text(json!({ "t": "join", "node": plan, "since": 0 }).to_string().into())).await.unwrap();
    next_of(&mut wa, "joined").await;
    let mut changed = bens.clone();
    changed["html"] = json!("Fine on mobile");
    let send = |client: &str, base: i64, list: Value| {
        json!({ "t": "send", "node": plan, "base": base, "client": client, "ops": [step(json!([{ "op": "set_task", "id": "t1", "field": "comments", "value": list }]))] })
            .to_string()
    };
    wa.send(Message::Text(send("a1", 2, json!([changed])).into())).await.unwrap();
    assert_eq!(next_of(&mut wa, "accepted").await["code"], "not_allowed", "changing his words");
    wa.send(Message::Text(send("a2", 2, json!([])).into())).await.unwrap();
    assert_eq!(next_of(&mut wa, "accepted").await["t"], "accepted", "deleting his comment");
    wa.send(Message::Text(send("a2b", 3, json!([changed])).into())).await.unwrap();
    assert_eq!(next_of(&mut wa, "accepted").await["code"], "not_allowed", "putting it back changed");
    wa.send(Message::Text(send("a2c", 3, json!([bens])).into())).await.unwrap();
    assert_eq!(next_of(&mut wa, "accepted").await["t"], "accepted", "putting it back as it was");
    // Adding her own beside his: yes. Ben changing his: yes.
    let anns = json!({ "id": "c2", "at": 6, "who": "ann", "by": format!("u{ann_id}"), "html": "Thanks" });
    wa.send(Message::Text(send("a3", 4, json!([bens, anns])).into())).await.unwrap();
    assert_eq!(next_of(&mut wa, "accepted").await["t"], "accepted");
    w.send(Message::Text(send("b5", 5, json!([changed, anns])).into())).await.unwrap();
    assert_eq!(next_of(&mut w, "accepted").await["t"], "accepted");
    w.send(Message::Text(send("b6", 6, json!([changed])).into())).await.unwrap();
    assert_eq!(next_of(&mut w, "accepted").await["code"], "not_allowed", "a commenter deleting another's");

    // A plain "can comment" grant: comments only.
    let (_, f) = post(
        &addr,
        "/v1/nodes",
        Some(&admin),
        json!({ "space": space, "kind": "file", "meta": "{\"name\":\"Other\",\"app\":\"plan\"}" }),
    )
    .await;
    let other = f["id"].as_i64().unwrap();
    post(
        &addr,
        &format!("/v1/nodes/{other}/access"),
        Some(&admin),
        json!({ "own": true, "grants": [{ "who": "everyone", "role": "commenter" }] }),
    )
    .await;
    w.send(Message::Text(json!({ "t": "join", "node": other, "since": 0 }).to_string().into())).await.unwrap();
    assert_eq!(next_of(&mut w, "joined").await["may"], json!(["plan.comment"]));
    w.send(Message::Text(
        json!({ "t": "send", "node": other, "base": 0, "client": "b4", "ops": [add] }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_of(&mut w, "accepted").await["code"], "not_allowed");

    // Shared from someone's own files: the role counts there too.
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let (_, f) = post(
        &addr,
        "/v1/nodes",
        Some(&admin),
        json!({ "space": mine, "kind": "file", "meta": "{\"name\":\"Mine\",\"app\":\"plan\"}" }),
    )
    .await;
    let own = f["id"].as_i64().unwrap();
    post(
        &addr,
        &format!("/v1/nodes/{own}/access"),
        Some(&admin),
        json!({ "own": true, "grants": [{ "who": "everyone", "role": "commenter", "custom": reporter }] }),
    )
    .await;
    w.send(Message::Text(json!({ "t": "join", "node": own, "since": 0 }).to_string().into())).await.unwrap();
    assert_eq!(next_of(&mut w, "joined").await["may"], json!(["plan.comment", "plan.create"]));

    // A document role that works like "Can edit" but ticks nothing in Plan:
    // no Plan changes at all.
    let (_, r) = post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "Writer only", "kind": "document", "base": "editor", "perms": [] }),
    )
    .await;
    let writer = r["id"].as_i64().unwrap();
    let (_, f) = post(
        &addr,
        "/v1/nodes",
        Some(&admin),
        json!({ "space": mine, "kind": "file", "meta": "{\"name\":\"Hands off\",\"app\":\"plan\"}" }),
    )
    .await;
    let hands_off = f["id"].as_i64().unwrap();
    post(
        &addr,
        &format!("/v1/nodes/{hands_off}/access"),
        Some(&admin),
        json!({ "own": true, "grants": [{ "who": "everyone", "role": "editor", "custom": writer }] }),
    )
    .await;
    w.send(Message::Text(json!({ "t": "join", "node": hands_off, "since": 0 }).to_string().into())).await.unwrap();
    let j = next_of(&mut w, "joined").await;
    assert_eq!((j["role"].as_str(), &j["may"]), (Some("editor"), &json!([])), "{j}");
    w.send(Message::Text(
        json!({ "t": "send", "node": hands_off, "base": 0, "client": "b5", "ops": [add] }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_of(&mut w, "accepted").await["code"], "not_allowed");

    // Admins are never limited.
    let mut a = open(&addr, &admin).await;
    a.send(Message::Text(json!({ "t": "join", "node": plan, "since": 0 }).to_string().into())).await.unwrap();
    let j = next_of(&mut a, "joined").await;
    assert_eq!(j["may"].as_array().map(|x| x.len()), Some(8));
    let _ = ann;
}

#[tokio::test]
async fn roles_can_hold_parts_of_running_the_server() {
    let Some(db) = fresh_db("powers").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    let admin_id = me["id"].as_i64().unwrap();
    let (ann_id, ann) = person(&addr, &admin, "ann", &[]).await;
    let (ben_id, ben) = person(&addr, &admin, "ben", &[]).await;
    let (cat_id, _) = person(&addr, &admin, "cat", &[]).await;

    // The catalogue offers them; Everyone never gets one.
    let (_, list) = get(&addr, "/v1/roles", Some(&admin)).await;
    assert!(list["catalogue"]["admin"].as_array().unwrap().iter().any(|p| p["id"] == "admin.ai"), "{list}");
    let everyone =
        list["roles"].as_array().unwrap().iter().find(|r| r["everyone"] == true).unwrap()["id"].as_i64().unwrap();
    let (status, e) =
        post(&addr, &format!("/v1/admin/roles/{everyone}"), Some(&admin), json!({ "perms": ["admin.ai"] })).await;
    assert_eq!((status, e["code"].clone()), (400, json!("power_for_everyone")));
    let (status, _) = get(&addr, "/v1/admin/ai", Some(&ann)).await;
    assert_eq!(status, 403, "not before the role");

    // Ann looks after AI, nothing else.
    let (status, _) = post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "AI keeper", "perms": ["admin.ai"], "members": [{ "user": ann_id }] }),
    )
    .await;
    assert_eq!(status, 200);
    let (_, p) = get(&addr, "/v1/me/perms", Some(&ann)).await;
    assert_eq!(p["powers"], json!(["admin.ai"]), "{p}");
    assert_eq!(get(&addr, "/v1/admin/ai", Some(&ann)).await.0, 200);
    assert_eq!(get(&addr, "/v1/admin/people", Some(&ann)).await.0, 403);
    assert_eq!(get(&addr, "/v1/admin/hub", Some(&ann)).await.0, 403);

    // Ben looks after people and roles, but can't make or touch admins, or
    // hand out powers.
    let (_, r) = post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "People keeper", "perms": ["admin.people", "admin.roles"], "members": [{ "user": ben_id }] }),
    )
    .await;
    let keeper = r["id"].as_i64().unwrap();
    assert_eq!(get(&addr, "/v1/admin/people", Some(&ben)).await.0, 200);
    let (status, _) =
        post(&addr, "/v1/admin/people", Some(&ben), json!({ "username": "dee" })).await;
    assert_eq!(status, 200, "an ordinary person: yes");
    let (status, _) = post(
        &addr,
        "/v1/admin/people",
        Some(&ben),
        json!({ "username": "eve", "admin": true }),
    )
    .await;
    assert_eq!(status, 403, "no new admins");
    assert_eq!(post(&addr, &format!("/v1/admin/people/{cat_id}"), Some(&ben), json!({ "admin": true })).await.0, 403);
    assert_eq!(post(&addr, &format!("/v1/admin/people/{admin_id}/reset"), Some(&ben), json!({})).await.0, 403);
    assert_eq!(
        post(&addr, &format!("/v1/admin/people/{admin_id}"), Some(&ben), json!({ "disabled": true })).await.0,
        403
    );
    assert_eq!(post(&addr, &format!("/v1/admin/people/{cat_id}/reset"), Some(&ben), json!({})).await.0, 200);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/admin/people/{admin_id}/mfa"), Some(&ben), None).await.0, 403);
    // Roles without powers: yes. Powers, or the roles that hold them: no.
    let (status, _) =
        post(&addr, "/v1/admin/roles", Some(&ben), json!({ "name": "Space makers", "perms": ["files.spaces"] })).await;
    assert_eq!(status, 200);
    let (status, _) =
        post(&addr, "/v1/admin/roles", Some(&ben), json!({ "name": "Mine", "perms": ["admin.network"] })).await;
    assert_eq!(status, 403);
    let (status, _) = post(
        &addr,
        &format!("/v1/admin/roles/{keeper}"),
        Some(&ben),
        json!({ "members": [{ "user": ben_id }, { "user": cat_id }] }),
    )
    .await;
    assert_eq!(status, 403, "can't give his own powers away");
    // A group a power-holding role is given to: only admins change who's in it.
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Ops", "members": [] })).await;
    let ops = g["id"].as_i64().unwrap();
    post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "Ops", "perms": ["admin.network"], "members": [{ "group": ops }] }),
    )
    .await;
    assert_eq!(post(&addr, &format!("/v1/admin/people/{ben_id}"), Some(&ben), json!({ "groups": [ops] })).await.0, 403);
    assert_eq!(
        post(&addr, &format!("/v1/admin/groups/{ops}"), Some(&ben), json!({ "name": "Ops", "members": [ben_id] }))
            .await
            .0,
        403
    );
    // Settings: network parts need that power; admin access and second steps are admins' alone.
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&ben), json!({ "name": "Ben's server" })).await.0, 403);
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&ben), json!({ "mfa_required": "off" })).await.0, 403);
    assert_eq!(get(&addr, "/v1/admin/settings", Some(&ben)).await.0, 200, "the People page reads it");
    let _ = ann;
}

#[tokio::test]
async fn admin_is_a_role_and_people_get_roles_when_added() {
    let Some(db) = fresh_db("roles_admin_role").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, r) = post(&addr, "/v1/admin/roles", Some(&admin), json!({ "name": "Artists", "perms": ["ai.pictures"] })).await;
    let artists = r["id"].as_i64().unwrap();
    // Added with the Artists role, and Admin.
    let (status, p) =
        post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "kim", "roles": [artists], "admin": true })).await;
    assert_eq!(status, 200, "{p}");
    let kim = p["id"].as_i64().unwrap();
    let (_, roles) = get(&addr, "/v1/roles", Some(&admin)).await;
    let role = |id: i64| roles["roles"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap().clone();
    assert_eq!(role(0)["name"], "Admin");
    assert!(role(0)["members"].as_array().unwrap().contains(&json!({ "user": kim })));
    assert_eq!(role(artists)["members"], json!([{ "user": kim }]));
    // The Admin role's members change; it can't be emptied, or go to groups.
    assert_eq!(post(&addr, "/v1/admin/roles/0", Some(&admin), json!({ "members": [] })).await.0, 409);
    assert_eq!(post(&addr, "/v1/admin/roles/0", Some(&admin), json!({ "members": [{ "group": 1 }] })).await.0, 400);
    assert_eq!(post(&addr, "/v1/admin/roles/0", Some(&admin), json!({ "name": "Boss" })).await.0, 400);
    assert_eq!(post(&addr, "/v1/admin/roles/0", Some(&admin), json!({ "members": [{ "user": kim }] })).await.0, 200);
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    assert_eq!(me["admin"], false, "alex isn't an admin any more");
    // Roles change with the person.
    let (_, kim_session) = use_link(&addr, p["code"].as_str().unwrap(), "kim", "kim password 1").await;
    let kim_token = kim_session["token"].as_str().unwrap();
    assert_eq!(post(&addr, &format!("/v1/admin/people/{kim}"), Some(kim_token), json!({ "roles": [] })).await.0, 204);
    let (_, roles) = get(&addr, "/v1/roles", Some(kim_token)).await;
    let artists_now = roles["roles"].as_array().unwrap().iter().find(|r| r["id"] == artists).unwrap().clone();
    assert_eq!(artists_now["members"], json!([]));
}
