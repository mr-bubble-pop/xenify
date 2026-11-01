//! Who may do what, checked where it's easy to slip: the bin, open editing
//! windows after access changes, plans shared with a document role, people
//! who look after others' accounts, public links and Plan's shared things.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

macro_rules! db_or_skip {
    ($name:expr) => {
        match fresh_db($name).await {
            Some(db) => db,
            None => {
                eprintln!("no PostgreSQL to test with: skipped");
                return;
            }
        }
    };
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open(addr: &str, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/live")).await.unwrap();
    ws.send(Message::Text(json!({ "t": "hello", "token": token }).to_string().into())).await.unwrap();
    next_of(&mut ws, "welcome").await;
    ws
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// The next message of kind `t` (or an error).
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

/// Nothing but arrivals and leavings come for a moment.
async fn quiet(ws: &mut Ws) -> bool {
    let until = tokio::time::Instant::now() + Duration::from_millis(400);
    loop {
        match tokio::time::timeout_at(until, ws.next()).await {
            Err(_) => return true,
            Ok(Some(Ok(Message::Text(x)))) => {
                let v: Value = serde_json::from_str(&x).unwrap();
                if v["t"] != "arrived" && v["t"] != "left" {
                    return false;
                }
            }
            Ok(_) => return true,
        }
    }
}

async fn person(addr: &str, admin: &str, name: &str, groups: &[i64]) -> (i64, String) {
    let (status, p) = post(addr, "/v1/admin/people", Some(admin), json!({ "username": name, "groups": groups })).await;
    assert_eq!(status, 200, "{p}");
    let (_, s) = use_link(addr, p["code"].as_str().unwrap(), name, &format!("{name} password")).await;
    (p["id"].as_i64().unwrap(), s["token"].as_str().unwrap().to_string())
}

async fn node(addr: &str, token: &str, space: i64, parent: Option<i64>, kind: &str, meta: Value) -> i64 {
    let (status, n) =
        post(addr, "/v1/nodes", Some(token), json!({ "space": space, "parent": parent, "kind": kind, "meta": meta.to_string() }))
            .await;
    assert_eq!(status, 200, "{n}");
    n["id"].as_i64().unwrap()
}

/// One raw call: (status, head, body).
async fn raw(addr: &str, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> (u16, String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}Content-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).to_lowercase();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = out[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        body = dechunk(&String::from_utf8_lossy(&body)).into_bytes();
    }
    (status, head, body)
}

fn ids(list: &Value) -> Vec<i64> {
    list.as_array().map(|a| a.iter().filter_map(|n| n["id"].as_i64()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn the_bin_keeps_a_folders_own_list() {
    let db = db_or_skip!("acc_bin");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (ann_id, ann) = person(&addr, &admin, "ann", &[]).await;
    let (_, ben) = person(&addr, &admin, "ben", &[]).await;
    // A space everyone may change; a folder in it only Ann may.
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": "Team" })).await;
    let space = sp["id"].as_i64().unwrap();
    let secret = node(&addr, &admin, space, None, "folder", json!({ "name": "Pay" })).await;
    let (status, _) = post(
        &addr,
        &format!("/v1/nodes/{secret}/access"),
        Some(&admin),
        json!({ "own": true, "grants": [{ "who": { "user": ann_id }, "role": "editor" }] }),
    )
    .await;
    assert!(status < 300);
    let sheet = node(&addr, &ann, space, Some(secret), "file", json!({ "name": "Rises", "app": "write" })).await;
    let open_doc = node(&addr, &ben, space, None, "file", json!({ "name": "Menu", "app": "write" })).await;
    assert_eq!(get(&addr, &format!("/v1/nodes/{sheet}"), Some(&ben)).await.0, 404);

    // Ann deletes the folder; Ben deletes his own file.
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{secret}"), Some(&ann), None).await.0, 204);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{open_doc}"), Some(&ben), None).await.0, 204);
    // Ben's bin shows his file, not the folder he could never open.
    let (status, bin) = get(&addr, &format!("/v1/trash?space={space}"), Some(&ben)).await;
    assert_eq!(status, 200, "{bin}");
    assert_eq!(ids(&bin), vec![open_doc]);
    let (_, bin) = get(&addr, &format!("/v1/trash?space={space}"), Some(&ann)).await;
    assert!(ids(&bin).contains(&secret), "{bin}");
    // Nor can he put it (or what was in it) back to get at it.
    assert!(post(&addr, &format!("/v1/nodes/{secret}/restore"), Some(&ben), json!({})).await.0 >= 400);
    assert!(post(&addr, &format!("/v1/nodes/{sheet}/restore"), Some(&ben), json!({})).await.0 >= 400);
    assert_eq!(get(&addr, &format!("/v1/nodes/{sheet}"), Some(&ben)).await.0, 404);
    // Ann puts back the file alone: to the top of the space (its folder is
    // in the bin), still only hers.
    assert_eq!(post(&addr, &format!("/v1/nodes/{sheet}/restore"), Some(&ann), json!({})).await.0, 204);
    let (status, n) = get(&addr, &format!("/v1/nodes/{sheet}"), Some(&ann)).await;
    assert_eq!(status, 200);
    assert_eq!((n["parent"].clone(), n["own_access"].clone()), (Value::Null, json!(true)), "{n}");
    assert_eq!(get(&addr, &format!("/v1/nodes/{sheet}"), Some(&ben)).await.0, 404, "the folder's list came with it");
    // Ben's own file comes back as before.
    assert_eq!(post(&addr, &format!("/v1/nodes/{open_doc}/restore"), Some(&ben), json!({})).await.0, 204);
}

#[tokio::test]
async fn open_windows_stop_when_access_goes() {
    let db = db_or_skip!("acc_live");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (ann_id, ann) = person(&addr, &admin, "ann", &[]).await;
    let (ben_id, ben) = person(&addr, &admin, "ben", &[]).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&ann)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let doc = node(&addr, &ann, mine, None, "file", json!({ "name": "Notes", "app": "write" })).await;
    let share = |role: &str| json!({ "own": true, "grants": [{ "who": { "user": ben_id }, "role": role }] });
    post(&addr, &format!("/v1/nodes/{doc}/access"), Some(&ann), share("editor")).await;

    let mut a = open(&addr, &ann).await;
    send(&mut a, json!({ "t": "join", "node": doc, "since": 0 })).await;
    next_of(&mut a, "joined").await;
    let mut b = open(&addr, &ben).await;
    send(&mut b, json!({ "t": "join", "node": doc, "since": 0 })).await;
    assert_eq!(next_of(&mut b, "joined").await["role"], "editor");
    send(&mut b, json!({ "t": "send", "node": doc, "base": 0, "client": "b1", "ops": ["x"] })).await;
    assert_eq!(next_of(&mut b, "accepted").await["to"], 1);
    next_of(&mut a, "ops").await;

    // Ann makes him a reader: he's told at once that his changes aren't taken.
    post(&addr, &format!("/v1/nodes/{doc}/access"), Some(&ann), share("viewer")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    send(&mut b, json!({ "t": "send", "node": doc, "base": 1, "client": "b2", "ops": ["y"] })).await;
    assert_eq!(next_of(&mut b, "accepted").await["code"], "not_allowed");
    // Ann stops sharing: his window closes the document and hears no more of it.
    post(&addr, &format!("/v1/nodes/{doc}/access"), Some(&ann), json!({ "own": false, "grants": [] })).await;
    assert_eq!(next_of(&mut b, "nothing").await["code"], "no_access");
    send(&mut a, json!({ "t": "send", "node": doc, "base": 1, "client": "a1", "ops": ["z"] })).await;
    next_of(&mut a, "accepted").await;
    assert!(quiet(&mut b).await, "Ben no longer hears the changes");
    send(&mut b, json!({ "t": "send", "node": doc, "base": 2, "client": "b3", "ops": ["w"] })).await;
    assert_eq!(next_of(&mut b, "accepted").await["code"], "not_open");

    // Shared again, then deleted: the same.
    post(&addr, &format!("/v1/nodes/{doc}/access"), Some(&ann), share("editor")).await;
    send(&mut b, json!({ "t": "join", "node": doc, "since": 0 })).await;
    assert_eq!(next_of(&mut b, "joined").await["role"], "editor");
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{doc}"), Some(&ann), None).await.0, 204);
    assert_eq!(next_of(&mut b, "nothing").await["code"], "no_access");
    assert_eq!(next_of(&mut a, "nothing").await["code"], "no_access", "it's in the bin for her too");

    // Turned off by an admin: signed out, the window closes.
    let doc2 = node(&addr, &ann, mine, None, "file", json!({ "name": "More", "app": "write" })).await;
    post(&addr, &format!("/v1/nodes/{doc2}/access"), Some(&ann), share("editor")).await;
    send(&mut b, json!({ "t": "join", "node": doc2, "since": 0 })).await;
    next_of(&mut b, "joined").await;
    let (status, _) = post(&addr, &format!("/v1/admin/people/{ben_id}"), Some(&admin), json!({ "disabled": true })).await;
    assert!(status < 300);
    assert_eq!(next_of(&mut b, "nothing").await["code"], "signed_out");

    // Signing out closes Ann's window too.
    send(&mut a, json!({ "t": "join", "node": doc2, "since": 0 })).await;
    next_of(&mut a, "joined").await;
    assert!(post(&addr, "/v1/signout", Some(&ann), json!({})).await.0 < 300);
    assert_eq!(next_of(&mut a, "nothing").await["code"], "signed_out");
    let _ = ann_id;
}

#[tokio::test]
async fn behind_hands_back_no_more_than_joining() {
    let db = db_or_skip!("acc_behind");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let doc = node(&addr, &admin, mine, None, "file", json!({ "name": "Long", "app": "write" })).await;
    let mut w = open(&addr, &admin).await;
    send(&mut w, json!({ "t": "join", "node": doc, "since": 0 })).await;
    next_of(&mut w, "joined").await;
    // Over MAX_CATCH_UP changes, in batches.
    let mut seq = 0;
    for i in 0..21 {
        let ops: Vec<String> = (0..1000).map(|k| format!("{i}.{k}")).collect();
        send(&mut w, json!({ "t": "send", "node": doc, "base": seq, "client": format!("c{i}"), "ops": ops })).await;
        seq = next_of(&mut w, "accepted").await["to"].as_i64().unwrap();
    }
    send(&mut w, json!({ "t": "send", "node": doc, "base": 0, "client": "late", "ops": ["x"] })).await;
    assert_eq!(next_of(&mut w, "behind").await["code"], "too_far_behind");
    send(&mut w, json!({ "t": "send", "node": doc, "base": seq - 5, "client": "late2", "ops": ["x"] })).await;
    assert_eq!(next_of(&mut w, "behind").await["ops"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn a_plan_role_cant_be_got_round_through_the_files() {
    let db = db_or_skip!("acc_plan");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (ann_id, ann) = person(&addr, &admin, "ann", &[]).await;
    let (ben_id, ben) = person(&addr, &admin, "ben", &[]).await;
    // Ben may add issues and comment (works like "Can edit"); Ann edits.
    let (_, r) = post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "Reporter", "kind": "document", "base": "editor", "perms": ["plan.comment", "plan.create"] }),
    )
    .await;
    let reporter = r["id"].as_i64().unwrap();
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": "Plans" })).await;
    let space = sp["id"].as_i64().unwrap();
    post(
        &addr,
        &format!("/v1/spaces/{space}/access"),
        Some(&admin),
        json!({ "grants": [{ "who": { "user": ann_id }, "role": "editor" }, { "who": { "user": ben_id }, "role": "editor", "custom": reporter }] }),
    )
    .await;
    let plan = node(&addr, &admin, space, None, "file", json!({ "name": "Launch", "app": "plan", "ext": "xplan" })).await;
    let folder = node(&addr, &admin, space, None, "folder", json!({ "name": "Old" })).await;

    // Writing over it, renaming, moving, deleting, a public link: no.
    let (status, _, _) = raw(&addr, "POST", &format!("/v1/nodes/{plan}/content"), Some(&ben), b"anything at all").await;
    assert_eq!(status, 200, "saving still works for him");
    let (_, _, kept) = raw(&addr, "GET", &format!("/v1/nodes/{plan}/content"), Some(&ben), b"").await;
    assert_ne!(kept, b"anything at all", "but the version is the plan as the server has it");
    let (status, _) =
        post(&addr, &format!("/v1/nodes/{plan}"), Some(&ben), json!({ "meta": json!({ "name": "Mine", "app": "write" }).to_string() }))
            .await;
    assert_eq!(status, 403);
    let (status, _) = post(&addr, &format!("/v1/nodes/{plan}"), Some(&ben), json!({ "move_to": { "parent": folder } })).await;
    assert_eq!(status, 403);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{plan}"), Some(&ben), None).await.0, 403);
    assert_eq!(post(&addr, &format!("/v1/nodes/{plan}/public"), Some(&ben), json!({})).await.0, 403);

    // Ann (a plain editor) may; saying it's no longer a plan changes nothing.
    let (status, _) =
        post(&addr, &format!("/v1/nodes/{plan}"), Some(&ann), json!({ "meta": json!({ "name": "Launch", "app": "write" }).to_string() }))
            .await;
    assert_eq!(status, 204);
    let mut w = open(&addr, &ben).await;
    send(&mut w, json!({ "t": "join", "node": plan, "since": 0 })).await;
    assert_eq!(next_of(&mut w, "joined").await["may"], json!(["plan.comment", "plan.create"]));
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{plan}"), Some(&ben), None).await.0, 403);
    // Other files in the space: his role is "Can edit" there.
    let doc = node(&addr, &ben, space, None, "file", json!({ "name": "Notes", "app": "write" })).await;
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{doc}"), Some(&ben), None).await.0, 204);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{plan}"), Some(&ann), None).await.0, 204);
}

#[tokio::test]
async fn people_keepers_cant_take_over_powerful_accounts() {
    let db = db_or_skip!("acc_keepers");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Ops" })).await;
    let ops = g["id"].as_i64().unwrap();
    let (ann_id, ann) = person(&addr, &admin, "ann", &[]).await;
    let (ben_id, ben) = person(&addr, &admin, "ben", &[]).await;
    let (cat_id, _) = person(&addr, &admin, "cat", &[ops]).await;
    let (dan_id, _) = person(&addr, &admin, "dan", &[]).await;
    // Ann looks after people; Ben looks after AI himself; Cat through Ops.
    for (name, perms, members) in [
        ("Keeper", json!(["admin.people"]), json!([{ "user": ann_id }])),
        ("AI keeper", json!(["admin.ai"]), json!([{ "user": ben_id }, { "group": ops }])),
    ] {
        let (status, r) =
            post(&addr, "/v1/admin/roles", Some(&admin), json!({ "name": name, "perms": perms, "members": members })).await;
        assert_eq!(status, 200, "{r}");
    }
    for target in [ben_id, cat_id] {
        let (status, _) = post(&addr, &format!("/v1/admin/people/{target}/reset"), Some(&ann), json!({})).await;
        assert_eq!(status, 403, "not someone with a part of running the server");
        let (status, _) =
            post(&addr, &format!("/v1/admin/people/{target}"), Some(&ann), json!({ "email": "mine@example.com" })).await;
        assert_eq!(status, 403);
    }
    let (status, _) = post(&addr, &format!("/v1/admin/people/{dan_id}/reset"), Some(&ann), json!({})).await;
    assert_eq!(status, 200, "anyone else, as before");

    // With company mailboxes set up, an address at its domain is a mailbox:
    // not Ann's to hand out.
    let (pg, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    pg.execute(
        "INSERT INTO settings (key, value) VALUES ('user_mail', $1) ON CONFLICT (key) DO UPDATE SET value = $1",
        &[&json!({ "org": "imap", "domains": "corp.example" }).to_string()],
    )
    .await
    .unwrap();
    let (status, e) = post(&addr, &format!("/v1/admin/people/{dan_id}"), Some(&ann), json!({ "email": "ceo@corp.example" })).await;
    assert_eq!((status, e["code"].clone()), (403, json!("company_mail")));
    let (status, _) = post(&addr, "/v1/admin/people", Some(&ann), json!({ "email": "cfo@Corp.example" })).await;
    assert_eq!(status, 403);
    let (status, _) = post(&addr, &format!("/v1/admin/people/{dan_id}"), Some(&ann), json!({ "email": "dan@home.example" })).await;
    assert!(status < 300, "other addresses as before");
    let (status, _) = post(&addr, &format!("/v1/admin/people/{dan_id}"), Some(&admin), json!({ "email": "dan@corp.example" })).await;
    assert!(status < 300, "admins may");

    // A new email signs them out everywhere.
    assert_eq!(get(&addr, "/v1/me", Some(&ben)).await.0, 200);
    let (status, _) =
        post(&addr, &format!("/v1/admin/people/{ben_id}"), Some(&admin), json!({ "email": "ben@example.com" })).await;
    assert!(status < 300);
    assert_eq!(get(&addr, "/v1/me", Some(&ben)).await.0, 401);
    // Not when nothing changed, nor the admin's own.
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    let admin_id = me["id"].as_i64().unwrap();
    post(&addr, &format!("/v1/admin/people/{admin_id}"), Some(&admin), json!({ "email": "boss@example.com" })).await;
    assert_eq!(get(&addr, "/v1/me", Some(&admin)).await.0, 200);
    let _ = ann;
}

#[tokio::test]
async fn links_go_with_their_maker_and_their_grants() {
    let db = db_or_skip!("acc_links");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (ann_id, ann) = person(&addr, &admin, "ann", &[]).await;
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": "Team" })).await;
    let space = sp["id"].as_i64().unwrap();
    let folder = node(&addr, &admin, space, None, "folder", json!({ "name": "Shared" })).await;
    let menu = node(&addr, &admin, space, Some(folder), "file", json!({ "name": "Menu", "app": "write" })).await;
    let hr = node(&addr, &admin, space, Some(folder), "folder", json!({ "name": "HR" })).await;
    let pay = node(&addr, &admin, space, Some(hr), "file", json!({ "name": "Pay", "app": "write" })).await;
    for f in [menu, pay] {
        assert_eq!(raw(&addr, "POST", &format!("/v1/nodes/{f}/content"), Some(&admin), b"words").await.0, 200);
    }
    // HR is Ann's alone; the admin isn't on its list.
    post(
        &addr,
        &format!("/v1/nodes/{hr}/access"),
        Some(&admin),
        json!({ "own": true, "grants": [{ "who": { "user": ann_id }, "role": "manager" }] }),
    )
    .await;
    let (status, l) = post(&addr, &format!("/v1/nodes/{folder}/public"), Some(&admin), json!({})).await;
    assert_eq!(status, 200, "{l}");
    let token = l["token"].as_str().unwrap().to_string();
    let (_, open) = get(&addr, &format!("/v1/p/{token}"), None).await;
    let shown: Vec<i64> = open["items"].as_array().unwrap().iter().map(|i| i["id"].as_i64().unwrap()).collect();
    assert!(shown.contains(&menu));
    assert!(!shown.contains(&hr) && !shown.contains(&pay), "an admin's link doesn't reach past lists they aren't on");
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{token}/content/{pay}"), None, b"").await.0, 404);
    let (status, head, _) = raw(&addr, "GET", &format!("/v1/p/{token}/content/{menu}"), None, b"").await;
    assert_eq!(status, 200);
    assert!(head.contains("content-disposition: attachment"), "{head}");

    // Ann's link on HR works until she's turned off.
    let (_, l) = post(&addr, &format!("/v1/nodes/{hr}/public"), Some(&ann), json!({})).await;
    let hr_token = l["token"].as_str().unwrap().to_string();
    assert_eq!(get(&addr, &format!("/v1/p/{hr_token}"), None).await.0, 200);
    post(&addr, &format!("/v1/admin/people/{ann_id}"), Some(&admin), json!({ "disabled": true })).await;
    assert_eq!(get(&addr, &format!("/v1/p/{hr_token}"), None).await.0, 404);
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{hr_token}/content/{pay}"), None, b"").await.0, 404);
    post(&addr, &format!("/v1/admin/people/{ann_id}"), Some(&admin), json!({ "disabled": false })).await;
    assert_eq!(get(&addr, &format!("/v1/p/{hr_token}"), None).await.0, 200, "back when she is");

    // Files' own bytes are downloads too.
    let (_, head, _) = raw(&addr, "GET", &format!("/v1/nodes/{menu}/content"), Some(&admin), b"").await;
    assert!(head.contains("content-disposition: attachment"), "{head}");
}

#[tokio::test]
async fn plans_things_change_by_their_maker_or_people_keepers() {
    let db = db_or_skip!("acc_things");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, ann) = person(&addr, &admin, "ann", &[]).await;
    let (_, ben) = person(&addr, &admin, "ben", &[]).await;
    let (_, t) = post(&addr, "/v1/plan/things", Some(&ann), json!({ "name": "Van" })).await;
    let van = t["id"].as_str().unwrap().to_string();
    let (status, _) = call(&addr, "PUT", &format!("/v1/plan/things/{van}"), Some(&ben), Some(json!({ "name": "Mine" }))).await;
    assert_eq!(status, 403);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/plan/things/{van}"), Some(&ben), None).await.0, 403);
    let (status, _) =
        call(&addr, "PUT", &format!("/v1/plan/things/{van}"), Some(&ann), Some(json!({ "name": "Blue van" }))).await;
    assert_eq!(status, 200);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/plan/things/{van}"), Some(&admin), None).await.0, 200);

    // A plan nobody refers to: not even its name for Ben.
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&ann)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let plan = node(&addr, &ann, mine, None, "file", json!({ "name": "Secret plan", "app": "plan", "ext": "xplan" })).await;
    assert_eq!(get(&addr, &format!("/v1/plan/ask/{plan}"), Some(&ben)).await.0, 404);
    let (_, a) = get(&addr, &format!("/v1/plan/ask/{plan}"), Some(&ann)).await;
    assert_eq!(a["name"], "Secret plan");
    let r = format!("n{plan}:t1");
    let (_, o) = post(&addr, "/v1/plan/outside", Some(&ben), json!({ "refs": [r] })).await;
    assert_eq!(o[&r], json!({ "gone": true }));
}
