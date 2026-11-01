//! What the Files page relies on: the server's address leads to it, files
//! made there, and which sharing levels each kind of file has.

mod common;

use common::*;
use serde_json::json;

/// The address leads to Files even when the server has the browser apps
/// (Files links to them, and to the admin pages).
#[tokio::test]
async fn the_address_leads_to_files() {
    let Some(db) = fresh_db("files_page_address").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let apps = std::env::temp_dir().join(format!("xenify-apps-{}", std::process::id()));
    std::fs::create_dir_all(apps.join("write")).unwrap();
    std::fs::write(apps.join("index.html"), "apps").unwrap();
    server.host_apps(apps.clone());
    set_up(&addr, &server).await;
    let (status, page) = raw(&addr, "/").await;
    assert_eq!(status, 303, "{page}");
    assert!(page.contains("location: /files/"));
    std::fs::remove_dir_all(apps).ok();
}

/// Someone not signed in doesn't learn which version the server runs.
#[tokio::test]
async fn the_version_is_only_for_people_signed_in() {
    let Some(db) = fresh_db("files_page_version").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, info) = get(&addr, "/v1/info", None).await;
    assert_eq!(info["version"], "", "{info}");
    let (_, info) = get(&addr, "/v1/info", Some(&admin)).await;
    assert_ne!(info["version"], "", "{info}");
}

/// A file made on the Files page has a name and its app, and nothing saved
/// yet (its app starts it blank); Draw and Calc files can't be shared "can
/// comment", as they have no comments.
#[tokio::test]
async fn files_made_on_the_page_and_how_they_are_shared() {
    let Some(db) = fresh_db("files_page_kinds").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    for (app, ext, comments) in [("write", "xwrite", true), ("plan", "xplan", true), ("draw", "xdraw", false), ("calc", "xcalc", false)] {
        let meta = json!({ "name": format!("New {app}"), "app": app, "ext": ext }).to_string();
        let (status, made) = post(&addr, "/v1/nodes", Some(&admin), json!({ "space": mine, "kind": "file", "meta": meta })).await;
        assert_eq!(status, 200, "{made}");
        let id = made["id"].as_i64().unwrap();
        let (_, n) = get(&addr, &format!("/v1/nodes/{id}"), Some(&admin)).await;
        assert_eq!(n["version"], 0);
        let share = |role: &str| {
            json!({ "own": true, "grants": [
                { "who": { "user": me["id"] }, "role": "manager" },
                { "who": "everyone", "role": role },
            ]})
        };
        let (status, e) = post(&addr, &format!("/v1/nodes/{id}/access"), Some(&admin), share("commenter")).await;
        if comments {
            assert_eq!(status, 204, "{app}: {e}");
        } else {
            assert_eq!(status, 400, "{app}");
            assert_eq!(e["code"], "no_comments");
            assert_eq!(post(&addr, &format!("/v1/nodes/{id}/access"), Some(&admin), share("viewer")).await.0, 204);
        }
    }
}

/// The status line and headers of a plain GET.
async fn raw(addr: &str, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, out.to_lowercase())
}

/// A space's pages (a wiki): a page holds pages, and only pages; they're
/// listed, found, moved, read by whoever may read the space, and go to the
/// bin with the page above them.
#[tokio::test]
async fn pages_hold_pages() {
    let Some(db) = fresh_db("files_page_wiki").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let kim = add_person(&addr, &admin, "kim", false).await;
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": json!({ "name": "Handbook" }).to_string() })).await;
    let space = sp["id"].as_i64().unwrap();
    let (status, _) = post(
        &addr,
        &format!("/v1/spaces/{space}/access"),
        Some(&admin),
        json!({ "grants": [{ "who": "everyone", "role": "viewer" }] }),
    )
    .await;
    assert_eq!(status, 204);
    let page = |name: &str| json!({ "name": name, "app": "write", "ext": "xwrite", "page": true }).to_string();
    let make = |parent: Option<i64>, kind: &'static str, meta: String| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move {
            let mut body = json!({ "space": space, "kind": kind, "meta": meta });
            if let Some(p) = parent {
                body["parent"] = json!(p);
            }
            post(&addr, "/v1/nodes", Some(&admin), body).await
        }
    };
    let id = |r: (u16, serde_json::Value)| {
        assert_eq!(r.0, 200, "{}", r.1);
        r.1["id"].as_i64().unwrap()
    };
    let holidays = id(make(None, "file", page("Holidays")).await);
    let booking = id(make(Some(holidays), "file", page("Booking time off")).await);
    let deep = id(make(Some(booking), "file", page("Bank holidays")).await);

    // Under a page: only pages, never a folder or another file.
    assert_eq!(make(Some(holidays), "folder", json!({ "name": "abc" }).to_string()).await.0, 400);
    let plain = json!({ "name": "abc", "app": "write", "ext": "xwrite" }).to_string();
    assert_eq!(make(Some(holidays), "file", plain.clone()).await.0, 400);
    let doc = id(make(None, "file", plain).await);
    let to = |parent: i64| json!({ "move_to": { "parent": parent } });
    assert_eq!(post(&addr, &format!("/v1/nodes/{doc}"), Some(&admin), to(holidays)).await.0, 400, "a file isn't a page");
    // Made a page, it can go under one.
    let (status, _) = post(
        &addr,
        &format!("/v1/nodes/{doc}"),
        Some(&admin),
        json!({ "meta": page("abc"), "move_to": { "parent": holidays } }),
    )
    .await;
    assert_eq!(status, 204);
    // Never under itself or a page below it.
    assert_eq!(post(&addr, &format!("/v1/nodes/{holidays}"), Some(&admin), to(deep)).await.0, 400);
    // A page with pages under it stays a page.
    let (status, e) = post(
        &addr,
        &format!("/v1/nodes/{booking}"),
        Some(&admin),
        json!({ "meta": json!({ "name": "Booking", "app": "write", "ext": "xwrite" }).to_string() }),
    )
    .await;
    assert_eq!(status, 409, "{e}");

    // Its place among the pages beside it is tidying, not a change.
    let (_, before) = get(&addr, &format!("/v1/nodes/{holidays}"), Some(&admin)).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let ordered = json!({ "name": "Holidays", "app": "write", "ext": "xwrite", "page": true, "order": 3 }).to_string();
    assert_eq!(post(&addr, &format!("/v1/nodes/{holidays}"), Some(&kim), json!({ "meta": ordered.clone() })).await.0, 403);
    assert_eq!(post(&addr, &format!("/v1/nodes/{holidays}"), Some(&admin), json!({ "meta": ordered })).await.0, 204);
    let (_, after) = get(&addr, &format!("/v1/nodes/{holidays}"), Some(&admin)).await;
    assert_eq!(after["changed"], before["changed"], "moving a page up or down doesn't change it");
    assert!(after["meta"].as_str().unwrap().contains("\"order\":3"), "{after}");

    // Listed under their page, for anyone who may read the space.
    let (status, list) = get(&addr, &format!("/v1/nodes?space={space}&parent={holidays}"), Some(&kim)).await;
    assert_eq!(status, 200, "{list}");
    let mut names: Vec<_> = list.as_array().unwrap().iter().map(|n| n["id"].as_i64().unwrap()).collect();
    names.sort();
    assert_eq!(names, [booking, doc]);
    let (status, n) = get(&addr, &format!("/v1/nodes/{deep}"), Some(&kim)).await;
    assert_eq!(status, 200, "{n}");
    assert_eq!(n["role"], "viewer");
    assert_eq!(n["parent"], booking);
    // Found by name.
    let (_, found) = get(&addr, "/v1/search?q=bank", Some(&kim)).await;
    assert!(found.as_array().unwrap().iter().any(|n| n["id"] == deep), "{found}");

    // A page goes to the bin with the pages under it.
    let (status, _) = call(&addr, "DELETE", &format!("/v1/nodes/{holidays}"), Some(&admin), None).await;
    assert_eq!(status, 204);
    for gone in [holidays, booking, deep, doc] {
        assert_eq!(get(&addr, &format!("/v1/nodes/{gone}"), Some(&admin)).await.0, 404, "{gone}");
    }
    // The bin lists the page deleted (not each one with it), for whoever
    // may change the space; restoring puts it all back.
    assert_eq!(get(&addr, &format!("/v1/trash?space={space}"), Some(&kim)).await.0, 403, "Kim only reads the space");
    let (status, bin) = get(&addr, &format!("/v1/trash?space={space}"), Some(&admin)).await;
    assert_eq!(status, 200, "{bin}");
    let bin = bin.as_array().unwrap();
    assert_eq!(bin.len(), 1, "{bin:?}");
    assert_eq!(bin[0]["id"], holidays);
    assert_eq!(bin[0]["with"], 3);
    assert_eq!(call(&addr, "POST", &format!("/v1/nodes/{holidays}/restore"), Some(&kim), None).await.0, 403);
    assert_eq!(call(&addr, "POST", &format!("/v1/nodes/{holidays}/restore"), Some(&admin), None).await.0, 204);
    for back in [holidays, booking, deep, doc] {
        assert_eq!(get(&addr, &format!("/v1/nodes/{back}"), Some(&admin)).await.0, 200, "{back}");
    }
    // A page under one deleted later goes back to the top of the space.
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{deep}"), Some(&admin), None).await.0, 204);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{booking}"), Some(&admin), None).await.0, 204);
    assert_eq!(call(&addr, "POST", &format!("/v1/nodes/{deep}/restore"), Some(&admin), None).await.0, 204);
    let (_, n) = get(&addr, &format!("/v1/nodes/{deep}"), Some(&admin)).await;
    assert_eq!(n["parent"], serde_json::Value::Null, "{n}");
    // For good: one thing, or the whole bin (the space's managers only).
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{booking}/forever"), Some(&kim), None).await.0, 403);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{booking}/forever"), Some(&admin), None).await.0, 204);
    assert_eq!(call(&addr, "POST", &format!("/v1/nodes/{booking}/restore"), Some(&admin), None).await.0, 404);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{deep}/forever"), Some(&admin), None).await.0, 404, "not in the bin");
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{deep}"), Some(&admin), None).await.0, 204);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/trash?space={space}"), Some(&kim), None).await.0, 403);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/trash?space={space}"), Some(&admin), None).await.0, 204);
    let (_, bin) = get(&addr, &format!("/v1/trash?space={space}"), Some(&admin)).await;
    assert_eq!(bin, json!([]));
    assert_eq!(get(&addr, &format!("/v1/nodes/{holidays}"), Some(&admin)).await.0, 200, "what wasn't deleted stays");
}

/// Spaces made from Files: everyone may read and change one unless its
/// maker chose who; the maker manages it (and may remove it), and making
/// one is a role's permission, not only admins'.
#[tokio::test]
async fn spaces_made_from_files() {
    let Some(db) = fresh_db("files_page_spaces").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let kim = add_person(&addr, &admin, "kim", false).await;
    let ann = add_person(&addr, &admin, "ann", false).await;
    let (_, me) = get(&addr, "/v1/me", Some(&ann)).await;
    let ann_id = me["id"].as_i64().unwrap();
    let meta = |n: &str| json!({ "name": n }).to_string();
    let role_of = |list: &serde_json::Value, id: i64| {
        list.as_array().unwrap().iter().find(|s| s["id"] == id).map(|s| s["role"].as_str().unwrap().to_string())
    };

    // An admin's new space: everyone can change it.
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": meta("Fred") })).await;
    let fred = sp["id"].as_i64().unwrap();
    let (_, list) = get(&addr, "/v1/spaces", Some(&kim)).await;
    assert_eq!(role_of(&list, fred).as_deref(), Some("editor"), "{list}");

    // Ann may make spaces once a role says so; hers is only for her.
    assert_eq!(post(&addr, "/v1/spaces", Some(&ann), json!({ "name": meta("x") })).await.0, 403);
    let (status, r) = post(&addr, "/v1/admin/roles", Some(&admin), json!({ "name": "Space makers", "kind": "server", "perms": ["files.spaces"], "members": [{ "user": ann_id }] })).await;
    assert_eq!(status, 200, "{r}");
    let (status, sp) = post(&addr, "/v1/spaces", Some(&ann), json!({ "name": meta("Ann's"), "grants": [] })).await;
    assert_eq!(status, 200, "{sp}");
    let mine = sp["id"].as_i64().unwrap();
    let (_, list) = get(&addr, "/v1/spaces", Some(&ann)).await;
    assert_eq!(role_of(&list, mine).as_deref(), Some("manager"));
    let (_, list) = get(&addr, "/v1/spaces", Some(&kim)).await;
    assert_eq!(role_of(&list, mine), None, "only the people chosen");
    // Its manager removes it; others can't.
    assert_eq!(call(&addr, "DELETE", &format!("/v1/spaces/{fred}"), Some(&kim), None).await.0, 403);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/spaces/{mine}"), Some(&ann), None).await.0, 204);
}

/// What's typed reaches the saved document without Save: a save "as you
/// type" goes into your latest version while it's recent, so the history
/// keeps checkpoints; Save (or someone else's) makes a new one.
#[tokio::test]
async fn saving_as_you_type_keeps_checkpoints() {
    let Some(db) = fresh_db("files_page_autosave").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let meta = json!({ "name": "Notes", "app": "write", "ext": "xwrite" }).to_string();
    let (_, made) = post(&addr, "/v1/nodes", Some(&admin), json!({ "space": mine, "kind": "file", "meta": meta })).await;
    let id = made["id"].as_i64().unwrap();
    let put = |q: &'static str, body: &'static str| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move { raw_post(&addr, &format!("/v1/nodes/{id}/content{q}"), &admin, body.as_bytes()).await }
    };
    assert_eq!(put("?auto=true", "one").await.0, 200);
    assert_eq!(put("?auto=true", "two").await.0, 200);
    let (_, v) = get(&addr, &format!("/v1/nodes/{id}/versions"), Some(&admin)).await;
    assert_eq!(v.as_array().unwrap().len(), 1, "typing makes one checkpoint: {v}");
    assert_eq!(raw_get(&addr, &format!("/v1/nodes/{id}/content"), &admin).await, "two");
    assert_eq!(put("", "three").await.0, 200);
    assert_eq!(put("?auto=true", "four").await.0, 200);
    let (_, v) = get(&addr, &format!("/v1/nodes/{id}/versions"), Some(&admin)).await;
    assert_eq!(v.as_array().unwrap().len(), 2, "Save makes a new one, typing goes on into it: {v}");
    assert_eq!(raw_get(&addr, &format!("/v1/nodes/{id}/content"), &admin).await, "four");

    // A checkpoint: a version of its own with a description, which typing
    // after it doesn't go into.
    assert_eq!(put("?label=Sent%20to%20Ann", "five").await.0, 200);
    assert_eq!(put("?auto=true", "six").await.0, 200);
    let (_, v) = get(&addr, &format!("/v1/nodes/{id}/versions"), Some(&admin)).await;
    let v = v.as_array().unwrap().clone();
    assert_eq!(v.len(), 4, "{v:?}");
    assert_eq!(v[1]["label"], "Sent to Ann");
    let kept = v[1]["n"].as_i64().unwrap();
    // Only the newest 20 versions without a description are kept; checkpoints stay.
    for _ in 0..25 {
        assert_eq!(put("", "more").await.0, 200);
    }
    let (_, v) = get(&addr, &format!("/v1/nodes/{id}/versions"), Some(&admin)).await;
    let v = v.as_array().unwrap().clone();
    assert_eq!(v.iter().filter(|x| x["label"].is_null()).count(), 20, "{v:?}");
    assert!(v.iter().any(|x| x["n"] == kept && x["label"] == "Sent to Ann"));
    let current = v[0]["n"].as_i64().unwrap();
    // Naming one keeps it; the current version can't be deleted; others can.
    let other = v[5]["n"].as_i64().unwrap();
    assert_eq!(post(&addr, &format!("/v1/nodes/{id}/versions/{other}"), Some(&admin), json!({ "label": "Before the cuts" })).await.0, 204);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{id}/versions/{current}"), Some(&admin), None).await.0, 400);
    let gone = v[3]["n"].as_i64().unwrap();
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{id}/versions/{gone}"), Some(&admin), None).await.0, 204);
    // All but the checkpoints and the current one; then all but the current one.
    let (status, r) = call(&addr, "DELETE", &format!("/v1/nodes/{id}/versions"), Some(&admin), None).await;
    assert_eq!(status, 200, "{r}");
    let (_, v) = get(&addr, &format!("/v1/nodes/{id}/versions"), Some(&admin)).await;
    let ns: Vec<i64> = v.as_array().unwrap().iter().map(|x| x["n"].as_i64().unwrap()).collect();
    let mut want = vec![current, other, kept];
    want.sort_by(|a, b| b.cmp(a));
    assert_eq!(ns, want);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{id}/versions?checkpoints=true"), Some(&admin), None).await.0, 200);
    let (_, v) = get(&addr, &format!("/v1/nodes/{id}/versions"), Some(&admin)).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(raw_get(&addr, &format!("/v1/nodes/{id}/content"), &admin).await, "more");
}

/// Bytes sent to `path` (POST, signed in): the status.
async fn raw_post(addr: &str, path: &str, token: &str, body: &[u8]) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    (out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0), out)
}

/// The body of a GET (signed in), as text.
async fn raw_get(addr: &str, path: &str, token: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or(("", ""));
    if head.to_lowercase().contains("transfer-encoding: chunked") { dechunk(body) } else { body.to_string() }
}
