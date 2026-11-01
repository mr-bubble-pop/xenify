//! Public links against a real PostgreSQL: anyone with the link reads what
//! was shared and nothing else; links stop at once; admins can turn them
//! all off.

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

fn meta(name: &str) -> String {
    json!({ "name": name, "app": "write" }).to_string()
}

/// A folder or file (with one saved version, when a file).
async fn node(addr: &str, token: &str, space: i64, parent: Option<i64>, kind: &str, name: &str) -> i64 {
    let (status, n) = post(
        addr,
        "/v1/nodes",
        Some(token),
        json!({ "space": space, "parent": parent, "kind": kind, "meta": meta(name) }),
    )
    .await;
    assert_eq!(status, 200, "{n}");
    let id = n["id"].as_i64().unwrap();
    if kind == "folder" {
        return id;
    }
    // The demo's real documents, when it's given some (XENIFY_DEMO_DOCS:
    // a folder of <first word of the name, lower case>.xwrite).
    let text = std::env::var("XENIFY_DEMO_DOCS")
        .ok()
        .and_then(|d| std::fs::read(format!("{d}/{}.xwrite", name.split(' ').next().unwrap().to_lowercase())).ok())
        .unwrap_or_else(|| format!("the words of {name}").into_bytes());
    assert_eq!(save(addr, token, id, &text).await, 200);
    id
}

async fn raw(addr: &str, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> (u16, Vec<u8>) {
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
    let head = String::from_utf8_lossy(&out[..split]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = out[split + 4..].to_vec();
    let body = if head.to_lowercase().contains("transfer-encoding: chunked") {
        dechunk(&String::from_utf8_lossy(&body)).into_bytes()
    } else {
        body
    };
    (status, body)
}

async fn save(addr: &str, token: &str, id: i64, bytes: &[u8]) -> u16 {
    raw(addr, "POST", &format!("/v1/nodes/{id}/content"), Some(token), bytes).await.0
}

async fn delete(addr: &str, path: &str, token: &str) -> u16 {
    call(addr, "DELETE", path, Some(token), None).await.0
}

fn name_of(item: &Value) -> String {
    serde_json::from_str::<Value>(item["meta"].as_str().unwrap()).unwrap()["name"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_link_opens_what_was_shared_and_nothing_else() {
    let db = db_or_skip!("publinks");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let kim = add_person(&addr, &admin, "kim", false).await;

    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": meta("Office handbook") })).await;
    let space = sp["id"].as_i64().unwrap();
    let (_, me) = get(&addr, "/v1/me", Some(&sam)).await;
    let (_, kim_me) = get(&addr, "/v1/me", Some(&kim)).await;
    let grants = json!({ "grants": [
        { "who": { "user": me["id"] }, "role": "editor" },
        { "who": { "user": kim_me["id"] }, "role": "commenter" },
    ]});
    assert_eq!(post(&addr, &format!("/v1/spaces/{space}/access"), Some(&admin), grants).await.0, 204);

    let money = node(&addr, &sam, space, None, "folder", "Money").await;
    let expenses = node(&addr, &sam, space, Some(money), "file", "Expenses").await;
    let travel = node(&addr, &sam, space, Some(money), "file", "Travel").await;
    let secret = node(&addr, &sam, space, None, "file", "Pay rises").await;

    // Commenters can't share; editors can.
    assert_eq!(post(&addr, &format!("/v1/nodes/{money}/public"), Some(&kim), json!({})).await.0, 403);
    let (status, made) = post(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam), json!({})).await;
    assert_eq!(status, 200, "{made}");
    let token = made["token"].as_str().unwrap().to_string();
    let (_, mine) = get(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam)).await;
    assert_eq!(mine["link"]["files"], 2);

    // Anyone with the link: the folder and what's in it.
    let (status, open) = get(&addr, &format!("/v1/p/{token}"), None).await;
    assert_eq!(status, 200, "{open}");
    assert_eq!(open["root"], money);
    assert_eq!(open["server"], "Office server");
    let items = open["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["parent"], Value::Null);
    let exp = items.iter().find(|i| i["id"] == expenses).unwrap();
    assert_eq!(name_of(exp), "Expenses");
    let (status, bytes) = raw(&addr, "GET", &format!("/v1/p/{token}/content/{expenses}"), None, b"").await;
    assert_eq!((status, bytes.as_slice()), (200, b"the words of Expenses".as_slice()));
    // Not shared, so not served.
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{token}/content/{secret}"), None, b"").await.0, 404);
    assert_eq!(get(&addr, "/v1/p/nottherealtoken", None).await.0, 404);

    // A folder in it with its own list Sam isn't on: Sam's link doesn't show it.
    let hr = node(&addr, &admin, space, Some(money), "folder", "HR").await;
    let reviews = node(&addr, &admin, space, Some(hr), "file", "Reviews").await;
    assert_eq!(
        post(&addr, &format!("/v1/nodes/{hr}/access"), Some(&admin), json!({ "own": true, "grants": [] })).await.0,
        204
    );
    let (_, open) = get(&addr, &format!("/v1/p/{token}"), None).await;
    assert!(open["items"].as_array().unwrap().iter().all(|i| i["id"] != hr && i["id"] != reviews), "{open}");
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{token}/content/{reviews}"), None, b"").await.0, 404);
    assert_eq!(get(&addr, &format!("/v1/p/{token}/page/{reviews}"), None).await.0, 404);
    // Once Sam may open it, so may the link.
    assert_eq!(
        post(
            &addr,
            &format!("/v1/nodes/{hr}/access"),
            Some(&admin),
            json!({ "own": true, "grants": [{ "who": { "user": me["id"] }, "role": "viewer" }] })
        )
        .await
        .0,
        204
    );
    let (_, open) = get(&addr, &format!("/v1/p/{token}"), None).await;
    assert!(open["items"].as_array().unwrap().iter().any(|i| i["id"] == reviews), "{open}");
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{token}/content/{reviews}"), None, b"").await.0, 200);
    assert_eq!(delete(&addr, &format!("/v1/nodes/{hr}"), &admin).await, 204);

    // A new file in the folder is in the link straight away.
    let car = node(&addr, &sam, space, Some(money), "file", "Car hire").await;
    let (_, open) = get(&addr, &format!("/v1/p/{token}"), None).await;
    assert!(open["items"].as_array().unwrap().iter().any(|i| i["id"] == car));

    // Into the bin: it drops out of the link.
    assert_eq!(delete(&addr, &format!("/v1/nodes/{travel}"), &sam).await, 204);
    let (_, open) = get(&addr, &format!("/v1/p/{token}"), None).await;
    assert!(open["items"].as_array().unwrap().iter().all(|i| i["id"] != travel));
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{token}/content/{travel}"), None, b"").await.0, 404);

    // A new link stops the old one at once.
    let (_, again) = post(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam), json!({})).await;
    let token2 = again["token"].as_str().unwrap().to_string();
    assert_ne!(token, token2);
    let (status, gone) = get(&addr, &format!("/v1/p/{token}"), None).await;
    assert_eq!((status, gone["code"].as_str()), (404, Some("gone")));
    assert_eq!(get(&addr, &format!("/v1/p/{token2}"), None).await.0, 200);

    // A single file's link shares just it.
    let (_, one) = post(&addr, &format!("/v1/nodes/{secret}/public"), Some(&sam), json!({})).await;
    let (_, open) = get(&addr, &format!("/v1/p/{}", one["token"].as_str().unwrap()), None).await;
    assert_eq!(open["items"].as_array().unwrap().len(), 1);
    assert_eq!(delete(&addr, &format!("/v1/nodes/{secret}/public"), &sam).await, 204);

    // Admins see every link and can turn them all off, then on again.
    let (_, list) = get(&addr, "/v1/admin/public-links", Some(&admin)).await;
    assert_eq!(list["on"], true);
    assert_eq!(list["links"].as_array().unwrap().len(), 1);
    // With the link itself, so an admin can open it to check what it shows.
    assert!(list["links"][0]["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert_eq!(get(&addr, "/v1/admin/public-links", Some(&sam)).await.0, 403);
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&admin), json!({ "public_links": false })).await.0, 204);
    let (_, info) = get(&addr, "/v1/info", None).await;
    assert!(!info["features"].as_array().unwrap().contains(&json!("public_links")));
    let (status, off) = get(&addr, &format!("/v1/p/{token2}"), None).await;
    assert_eq!((status, off["code"].as_str()), (403, Some("links_off")));
    assert_eq!(raw(&addr, "GET", &format!("/v1/p/{token2}/content/{expenses}"), None, b"").await.0, 403);
    assert_eq!(post(&addr, &format!("/v1/nodes/{expenses}/public"), Some(&sam), json!({})).await.0, 403);
    let (_, mine) = get(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam)).await;
    assert_eq!(mine["allowed"], false);
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&admin), json!({ "public_links": true })).await.0, 204);
    assert_eq!(get(&addr, &format!("/v1/p/{token2}"), None).await.0, 200);

    // Stopping it: by its sharer, or by an admin.
    assert_eq!(delete(&addr, &format!("/v1/nodes/{money}/public"), &sam).await, 204);
    assert_eq!(get(&addr, &format!("/v1/p/{token2}"), None).await.0, 404);
    let (_, made) = post(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam), json!({})).await;
    let id = made["id"].as_i64().unwrap();
    assert_eq!(delete(&addr, &format!("/v1/admin/public-links/{id}"), &admin).await, 204);
    assert_eq!(get(&addr, &format!("/v1/p/{}", made["token"].as_str().unwrap()), None).await.0, 404);
    let (_, mine) = get(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam)).await;
    assert!(mine["link"].is_null());
}

/// A server with a shared handbook and two links, kept running for looking
/// at the pages by hand or in a headless browser:
/// `cargo test -p xenify-server --test publinks demo -- --ignored --nocapture`
/// prints the address and an admin session (sessionStorage "xenify-admin";
/// the Files page's is "xenify-files"). XENIFY_DEMO_DOCS and
/// XENIFY_DEMO_APPS give it real documents and the apps in the browser.
#[tokio::test]
#[ignore]
async fn demo() {
    let db = db_or_skip!("publinks_demo");
    let data = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("publinks_demo");
    let _ = std::fs::remove_dir_all(&data);
    let server = xenify_server::Server::start(xenify_server::Config {
        database: db.clone(),
        data,
        local_webhooks: true,
        web: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web")),
        secure: None,
        postgres: None,
    })
    .await
    .unwrap();
    // The apps in the browser (XENIFY_DEMO_APPS: a folder with write/ in it).
    if let Ok(apps) = std::env::var("XENIFY_DEMO_APPS") {
        server.host_apps(apps.into());
    }
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
    let s2 = server.clone();
    tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": meta("Office handbook") })).await;
    let space = sp["id"].as_i64().unwrap();
    let (_, me) = get(&addr, "/v1/me", Some(&sam)).await;
    let grants = json!({ "grants": [{ "who": { "user": me["id"] }, "role": "editor" }, { "who": "everyone", "role": "viewer" }] });
    post(&addr, &format!("/v1/spaces/{space}/access"), Some(&admin), grants).await;
    let money = node(&addr, &sam, space, None, "folder", "Money").await;
    node(&addr, &sam, space, Some(money), "file", "Expenses").await;
    node(&addr, &sam, space, Some(money), "file", "Travel").await;
    let hours = node(&addr, &admin, space, None, "file", "Office opening hours").await;
    let (_, a) = post(&addr, &format!("/v1/nodes/{money}/public"), Some(&sam), json!({})).await;
    let (_, b) = post(&addr, &format!("/v1/nodes/{hours}/public"), Some(&admin), json!({})).await;
    get(&addr, &format!("/v1/p/{}", b["token"].as_str().unwrap()), None).await;
    let session = json!({ "token": admin, "account": { "username": "alex", "admin": true } });
    println!("DEMO addr=http://{addr}");
    println!("DEMO session={session}");
    println!("DEMO links=/p/{} /p/{}", a["token"].as_str().unwrap(), b["token"].as_str().unwrap());
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
}
