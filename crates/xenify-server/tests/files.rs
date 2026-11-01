//! Spaces, folders, files, access and versions against a real PostgreSQL.

mod common;

use common::*;
use serde_json::{Value, json};

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

/// Adds someone (in `groups`) and signs them in; returns their token and id.
async fn person(addr: &str, admin: &str, name: &str, groups: &[i64]) -> (String, i64) {
    let (_, p) = post(
        addr,
        "/v1/admin/people",
        Some(admin),
        json!({ "username": name, "groups": groups }),
    )
    .await;
    let (status, s) = use_link(addr, p["code"].as_str().unwrap(), name, &format!("{name} password")).await;
    assert_eq!(status, 200, "{s}");
    (s["token"].as_str().unwrap().to_string(), p["id"].as_i64().unwrap())
}

/// Names as the apps send them.
fn meta(name: &str) -> String {
    json!({ "name": name, "app": "write" }).to_string()
}

fn name_of(node: &Value) -> String {
    serde_json::from_str::<Value>(node["meta"].as_str().unwrap()).unwrap()["name"].as_str().unwrap().to_string()
}

async fn new_file(addr: &str, token: &str, space: i64, parent: Option<i64>, name: &str) -> i64 {
    let (status, n) = post(
        addr,
        "/v1/nodes",
        Some(token),
        json!({ "space": space, "parent": parent, "kind": "file", "meta": meta(name) }),
    )
    .await;
    assert_eq!(status, 200, "{n}");
    n["id"].as_i64().unwrap()
}

/// Saves raw bytes; returns (status, body).
async fn put(addr: &str, token: &str, id: i64, base: Option<i64>, bytes: &[u8]) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let q = base.map(|b| format!("?base={b}")).unwrap_or_default();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /v1/nodes/{id}/content{q} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(bytes).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).unwrap_or(Value::Null))
}

/// Downloads raw bytes.
async fn fetch(addr: &str, token: &str, id: i64, version: Option<i64>) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let q = version.map(|v| format!("?version={v}")).unwrap_or_default();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(
        format!("GET /v1/nodes/{id}/content{q} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out[split + 4..].to_vec())
}

fn find(list: &Value, id: i64) -> Option<Value> {
    list.as_array().unwrap().iter().find(|n| n["id"] == id).cloned()
}

#[tokio::test]
async fn people_share_spaces_by_group_and_keep_every_version() {
    let db = db_or_skip!("files");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design" })).await;
    let design = g["id"].as_i64().unwrap();
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Office" })).await;
    let office = g["id"].as_i64().unwrap();
    let (priya, priya_id) = person(&addr, &admin, "priya", &[design, office]).await;
    let (sam, _) = person(&addr, &admin, "sam", &[office]).await;
    let (kim, _) = person(&addr, &admin, "kim", &[]).await;

    // An admin makes the space; Design may edit, Office may comment.
    let (status, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": meta("Design") })).await;
    assert_eq!(status, 200);
    let space = sp["id"].as_i64().unwrap();
    let (status, _) = post(
        &addr,
        &format!("/v1/spaces/{space}/access"),
        Some(&admin),
        json!({ "grants": [
            { "who": { "group": design }, "role": "editor" },
            { "who": { "group": office }, "role": "commenter" },
        ]}),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(post(&addr, "/v1/spaces", Some(&priya), json!({ "name": "x" })).await.0, 403);

    // Priya sees it (and her own My files, and Templates); Kim doesn't.
    let (_, list) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let kinds: Vec<_> = list.as_array().unwrap().iter().map(|s| s["kind"].as_str().unwrap().to_string()).collect();
    assert_eq!(kinds, ["personal", "shared", "templates"]);
    assert_eq!(find(&list, space).unwrap()["role"], "editor");
    let (_, list) = get(&addr, "/v1/spaces", Some(&kim)).await;
    assert!(find(&list, space).is_none());

    // A folder and a file.
    let (_, f) = post(
        &addr,
        "/v1/nodes",
        Some(&priya),
        json!({ "space": space, "kind": "folder", "meta": meta("Client work") }),
    )
    .await;
    let folder = f["id"].as_i64().unwrap();
    let file = new_file(&addr, &priya, space, Some(folder), "Autumn campaign brief").await;
    let v1 = b"Autumn campaign: first draft".to_vec();
    assert_eq!(put(&addr, &priya, file, Some(0), &v1).await, (200, json!({ "version": 1 })));
    let v2 = b"Autumn campaign: second draft".to_vec();
    assert_eq!(put(&addr, &priya, file, Some(1), &v2).await.0, 200);
    // Saving on top of an old version is refused.
    let (status, behind) = put(&addr, &priya, file, Some(1), &v2).await;
    assert_eq!((status, behind["code"].clone(), behind["version"].clone()), (409, json!("behind"), json!(2)));

    // Sam (Office) can read but not change; Kim can't even see it.
    let (_, kids) = get(&addr, &format!("/v1/nodes?space={space}&parent={folder}"), Some(&sam)).await;
    let n = find(&kids, file).unwrap();
    assert_eq!(n["role"], "commenter");
    assert_eq!(name_of(&n), "Autumn campaign brief");
    let (status, bytes) = fetch(&addr, &sam, file, None).await;
    assert_eq!(status, 200);
    assert_eq!(bytes, b"Autumn campaign: second draft");
    let (_, old) = fetch(&addr, &sam, file, Some(1)).await;
    assert_eq!(old, b"Autumn campaign: first draft");
    assert_eq!(put(&addr, &sam, file, None, b"x").await.0, 403);
    assert_eq!(get(&addr, &format!("/v1/nodes/{file}"), Some(&kim)).await.0, 404);
    assert_eq!(fetch(&addr, &kim, file, None).await.0, 404);
    let (_, versions) = get(&addr, &format!("/v1/nodes/{file}/versions"), Some(&sam)).await;
    assert_eq!(versions.as_array().unwrap().iter().map(|v| v["n"].as_i64().unwrap()).collect::<Vec<_>>(), [2, 1]);

    // A folder with its own list: only Priya.
    let (_, c) = post(
        &addr,
        "/v1/nodes",
        Some(&priya),
        json!({ "space": space, "parent": folder, "kind": "folder", "meta": meta("Contracts") }),
    )
    .await;
    let contracts = c["id"].as_i64().unwrap();
    let (status, _) = post(
        &addr,
        &format!("/v1/nodes/{contracts}/access"),
        Some(&priya),
        json!({ "own": true, "grants": [
            { "who": { "user": priya_id }, "role": "manager" },
        ]}),
    )
    .await;
    assert_eq!(status, 403, "editors can't change who has access");
    let (status, _) = post(
        &addr,
        &format!("/v1/nodes/{contracts}/access"),
        Some(&admin),
        json!({ "own": true, "grants": [
            { "who": { "user": priya_id }, "role": "manager" },
        ]}),
    )
    .await;
    assert_eq!(status, 204);
    let (_, kids) = get(&addr, &format!("/v1/nodes?space={space}&parent={folder}"), Some(&sam)).await;
    assert!(find(&kids, contracts).is_none());
    let (_, kids) = get(&addr, &format!("/v1/nodes?space={space}&parent={folder}"), Some(&priya)).await;
    assert_eq!(find(&kids, contracts).unwrap()["role"], "manager");

    // Moving a folder into itself is refused.
    let (status, _) =
        post(&addr, &format!("/v1/nodes/{folder}"), Some(&priya), json!({ "move_to": { "parent": contracts } })).await;
    assert_eq!(status, 400);

    // Each version is kept once in the data folder.
    let mut stack = vec![std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(db.rsplit('/').next().unwrap())];
    let mut files = 0;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            } else if e.file_name() != "mail.key" {
                // (The mail key the server makes for itself isn't a version.)
                files += 1;
            }
        }
    }
    assert_eq!(files, 2);

    // Deleting the folder hides everything in it.
    assert_eq!(call(&addr, "DELETE", &format!("/v1/nodes/{folder}"), Some(&priya), None).await.0, 204);
    assert_eq!(get(&addr, &format!("/v1/nodes/{file}"), Some(&priya)).await.0, 404);
    let (_, top) = get(&addr, &format!("/v1/nodes?space={space}"), Some(&priya)).await;
    assert!(top.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn my_files_are_mine_until_i_share_one() {
    let db = db_or_skip!("myfiles");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (priya, _) = person(&addr, &admin, "priya", &[]).await;
    let (sam, sam_id) = person(&addr, &admin, "sam", &[]).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = list[0]["id"].as_i64().unwrap();
    assert_eq!(list[0]["kind"], "personal");

    let file = new_file(&addr, &priya, mine, None, "Notes").await;
    put(&addr, &priya, file, None, b"private".as_ref()).await;
    // Not even an admin can open someone's My files.
    assert_eq!(get(&addr, &format!("/v1/nodes/{file}"), Some(&admin)).await.0, 404);
    assert_eq!(get(&addr, &format!("/v1/nodes?space={mine}"), Some(&sam)).await.0, 404);
    let (_, shared) = get(&addr, "/v1/shared", Some(&sam)).await;
    assert!(shared.as_array().unwrap().is_empty());

    let (status, _) = post(
        &addr,
        &format!("/v1/nodes/{file}/access"),
        Some(&priya),
        json!({ "own": true, "grants": [
            { "who": { "user": sam_id }, "role": "viewer" },
        ]}),
    )
    .await;
    assert_eq!(status, 204);
    let (_, shared) = get(&addr, "/v1/shared", Some(&sam)).await;
    assert_eq!(shared[0]["id"], file);
    assert_eq!(shared[0]["role"], "viewer");
    assert_eq!(fetch(&addr, &sam, file, None).await.0, 200);
    let (_, recent) = get(&addr, "/v1/recent", Some(&sam)).await;
    assert_eq!(recent[0]["id"], file);
    // The owner keeps everything; the space itself can't be shared.
    assert_eq!(post(&addr, &format!("/v1/spaces/{mine}/access"), Some(&priya), json!({ "grants": [] })).await.0, 400);
}

#[tokio::test]
async fn everyone_uses_templates_and_admins_change_them() {
    let db = db_or_skip!("templates");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (priya, _) = person(&addr, &admin, "priya", &[]).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let templates = list.as_array().unwrap().iter().find(|s| s["kind"] == "templates").unwrap()["id"].as_i64().unwrap();
    let t = new_file(&addr, &admin, templates, None, "Letter").await;
    put(&addr, &admin, t, None, b"Dear [NAME]".as_ref()).await;

    let (_, list) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = list.as_array().unwrap().iter().find(|s| s["kind"] == "templates").unwrap().clone();
    assert_eq!(mine["role"], "viewer");
    let (_, kids) = get(&addr, &format!("/v1/nodes?space={templates}"), Some(&priya)).await;
    assert_eq!(name_of(&kids[0]), "Letter");
    assert_eq!(fetch(&addr, &priya, t, None).await.0, 200);
    let (status, _) =
        post(&addr, "/v1/nodes", Some(&priya), json!({ "space": templates, "kind": "file", "meta": "x" })).await;
    assert_eq!(status, 403);
}

/// Any request with raw bytes; returns (status, body).
async fn raw(addr: &str, method: &str, path: &str, token: &str, body: &[u8]) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = String::from_utf8_lossy(&out[..split]).split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out[split + 4..].to_vec())
}

#[tokio::test]
async fn a_picture_of_a_file_is_kept_for_those_who_may_see_it() {
    let db = db_or_skip!("preview");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (priya, _) = person(&addr, &admin, "priya", &[]).await;
    let (sam, sam_id) = person(&addr, &admin, "sam", &[]).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = list[0]["id"].as_i64().unwrap();
    let file = new_file(&addr, &priya, mine, None, "Poster").await;
    let path = format!("/v1/nodes/{file}/preview");

    // None until the app puts one up.
    assert_eq!(raw(&addr, "GET", &path, &priya, b"").await.0, 404);
    let png = b"\x89PNG picture".to_vec();
    assert_eq!(raw(&addr, "POST", &path, &priya, &png).await.0, 204);
    let (status, got) = raw(&addr, "GET", &path, &priya, b"").await;
    assert_eq!(status, 200);
    assert_eq!(got, b"\x89PNG picture");

    // Others need access: viewers may see it, but only editors change it.
    assert_eq!(raw(&addr, "GET", &path, &sam, b"").await.0, 404);
    post(
        &addr,
        &format!("/v1/nodes/{file}/access"),
        Some(&priya),
        json!({ "own": true, "grants": [{ "who": { "user": sam_id }, "role": "viewer" }] }),
    )
    .await;
    assert_eq!(raw(&addr, "GET", &path, &sam, b"").await.0, 200);
    assert_eq!(raw(&addr, "POST", &path, &sam, b"other").await.0, 403);
}

#[tokio::test]
async fn pictures_put_in_while_editing_are_part_of_the_document_without_a_version() {
    let db = db_or_skip!("node_pictures");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (priya, _) = person(&addr, &admin, "priya", &[]).await;
    let (sam, sam_id) = person(&addr, &admin, "sam", &[]).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = list[0]["id"].as_i64().unwrap();
    let file = new_file(&addr, &priya, mine, None, "Notes").await;
    let path = format!("/v1/nodes/{file}/pictures/xen-s-9.png");

    assert_eq!(raw(&addr, "GET", &path, &priya, b"").await.0, 404);
    assert_eq!(raw(&addr, "PUT", &path, &priya, b"\x89PNG one").await.0, 204);
    assert_eq!(raw(&addr, "GET", &path, &priya, b"").await, (200, b"\x89PNG one".to_vec()));
    // No version was made for it.
    let (_, versions) = get(&addr, &format!("/v1/nodes/{file}/versions"), Some(&priya)).await;
    assert_eq!(versions.as_array().map(Vec::len).unwrap_or(0), 0, "{versions}");
    // A name can't climb out of the document.
    assert_eq!(raw(&addr, "PUT", &format!("/v1/nodes/{file}/pictures/../x"), &priya, b"x").await.0 / 100, 4);

    // Others need access: viewers see it, only editors put pictures in.
    assert_eq!(raw(&addr, "GET", &path, &sam, b"").await.0, 404);
    post(
        &addr,
        &format!("/v1/nodes/{file}/access"),
        Some(&priya),
        json!({ "own": true, "grants": [{ "who": { "user": sam_id }, "role": "viewer" }] }),
    )
    .await;
    assert_eq!(raw(&addr, "GET", &path, &sam, b"").await.0, 200);
    assert_eq!(raw(&addr, "PUT", &path, &sam, b"other").await.0, 403);
}

#[tokio::test]
async fn your_words_follow_you_on_the_server() {
    let db = db_or_skip!("my_words");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (priya, _) = person(&addr, &admin, "priya", &[]).await;
    let (sam, _) = person(&addr, &admin, "sam", &[]).await;
    let words: Vec<String> = (0..3000).map(|i| format!("Okafor{i}")).collect();
    let body = json!({ "words": words }).to_string();
    assert_eq!(raw(&addr, "PUT", "/v1/me/settings/words", &priya, body.as_bytes()).await.0, 200);
    let (_, got) = get(&addr, "/v1/me/settings/words", Some(&priya)).await;
    assert_eq!(got["words"].as_array().unwrap().len(), 3000);
    // Each person has their own.
    let (_, other) = get(&addr, "/v1/me/settings/words", Some(&sam)).await;
    assert_eq!(other, json!({}));
}

#[tokio::test]
async fn search_finds_names_and_words_people_may_see() {
    let db = db_or_skip!("search");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (priya, _) = person(&addr, &admin, "priya", &[]).await;
    let (sam, _) = person(&addr, &admin, "sam", &[]).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&priya)).await;
    let mine = list[0]["id"].as_i64().unwrap();
    let brief = new_file(&addr, &priya, mine, None, "Autumn campaign brief").await;
    let notes = new_file(&addr, &priya, mine, None, "Notes").await;
    // The app sends the words when it saves.
    let text = "Budget for the winter launch: posters, radio and the new website.";
    let (status, _) = call_text(&addr, &priya, notes, text).await;
    assert_eq!(status, 204);
    assert_eq!(call_text(&addr, &sam, notes, "not mine").await.0, 404);

    let found = |v: &Value| v.as_array().unwrap().iter().map(|n| n["id"].as_i64().unwrap()).collect::<Vec<_>>();
    // By name, part of a word; by the words inside, with the bit around them.
    let (_, r) = get(&addr, "/v1/search?q=autu%20camp", Some(&priya)).await;
    assert_eq!(found(&r), [brief]);
    let (_, r) = get(&addr, "/v1/search?q=radio", Some(&priya)).await;
    assert_eq!(found(&r), [notes]);
    assert!(r[0]["snippet"].as_str().unwrap().contains("<<radio>>"), "{r}");
    // Only what you may see.
    let (_, r) = get(&addr, "/v1/search?q=radio", Some(&sam)).await;
    assert!(found(&r).is_empty());
    let (_, r) = get(&addr, "/v1/search?q=%25", Some(&priya)).await;
    assert!(found(&r).is_empty(), "% is just a character");
}

/// Sends a file's words for searching.
async fn call_text(addr: &str, token: &str, id: i64, text: &str) -> (u16, Vec<u8>) {
    raw(addr, "PUT", &format!("/v1/nodes/{id}/text"), token, text.as_bytes()).await
}
