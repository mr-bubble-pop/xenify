//! Shared test helpers: a fresh database per test, a server on a free port,
//! JSON calls, signing in and setting up.

#![allow(dead_code)]

use serde_json::{Value, json};
use xenify_server::{Config, Server};

pub fn admin_db() -> String {
    std::env::var("XENIFY_TEST_PG").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/postgres".into())
}

/// A new empty database; None when there's no PostgreSQL to test with.
pub async fn fresh_db(name: &str) -> Option<String> {
    crate::pg_slots::take();
    let (c, conn) = tokio_postgres::connect(&admin_db(), tokio_postgres::NoTls).await.ok()?;
    tokio::spawn(conn);
    let db = format!("srv_{name}_{}", std::process::id());
    let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {db}")).await;
    c.batch_execute(&format!("CREATE DATABASE {db}")).await.ok()?;
    let base = admin_db();
    Some(format!("{}/{db}", base.rsplit_once('/').unwrap().0))
}

pub async fn serve(database: &str) -> (String, Server) {
    let data = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(database.rsplit('/').next().unwrap());
    let _ = std::fs::remove_dir_all(&data);
    let s = Server::start(Config {
        database: database.into(),
        data,
        local_webhooks: true,
        web: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web")),
        secure: None,
        postgres: None,
    })
    .await
    .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
    let s2 = s.clone();
    tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });
    (addr, s)
}

/// One HTTP call; returns (status, JSON body or Null).
pub async fn call(addr: &str, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let json = body.map(|b| b.to_string()).unwrap_or_default();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    let (head, text) = out.split_once("\r\n\r\n").unwrap_or(("", ""));
    let text =
        if head.to_lowercase().contains("transfer-encoding: chunked") { dechunk(text) } else { text.to_string() };
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// A streamed ("chunked") body put back together.
pub fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        s = rest[n..].trim_start_matches("\r\n");
    }
    out
}

pub async fn get(addr: &str, path: &str, token: Option<&str>) -> (u16, Value) {
    call(addr, "GET", path, token, None).await
}

pub async fn post(addr: &str, path: &str, token: Option<&str>, body: Value) -> (u16, Value) {
    call(addr, "POST", path, token, Some(body)).await
}

/// Signs in like an app. Returns (status, session JSON).
pub async fn sign_in(addr: &str, user: &str, password: &str) -> (u16, Value) {
    post(addr, "/v1/login", None, json!({ "username": user, "password": password, "device": "test" })).await
}

/// The code to type: a code itself, or the `<id>.<secret>` part of an
/// older link (`…/join/#<it>`).
pub fn link_part(link: &str) -> String {
    link.rsplit_once('#').map(|(_, l)| l.to_string()).unwrap_or_else(|| link.to_string())
}

/// Uses `user`'s code (from an admin or an email) to pick `password`.
/// Returns (status, session JSON).
pub async fn use_link(addr: &str, link: &str, user: &str, password: &str) -> (u16, Value) {
    let body = json!({ "link": link_part(link), "password": password, "username": user, "device": "test" });
    post(addr, "/v1/link/use", None, body).await
}

/// Adds `user` (with `admin` set by the first admin) and signs them in;
/// returns their session token.
pub async fn add_person(addr: &str, admin: &str, user: &str, is_admin: bool) -> String {
    let (status, made) =
        post(addr, "/v1/admin/people", Some(admin), json!({ "username": user, "admin": is_admin })).await;
    assert_eq!(status, 200, "{made}");
    let (status, session) = use_link(addr, made["code"].as_str().unwrap(), user, &format!("{user} password 1")).await;
    assert_eq!(status, 200, "{session}");
    session["token"].as_str().unwrap().to_string()
}

/// Sets the server up as its first admin "alex" (password "alex password
/// 1"); returns the session's token.
pub async fn set_up(addr: &str, server: &Server) -> String {
    set_up_token(addr, &server.setup_token().unwrap()).await
}

/// The same, with the setup token the server printed.
#[allow(dead_code)]
pub async fn set_up_token(addr: &str, token: &str) -> String {
    let (status, session) = post(
        addr,
        "/v1/setup",
        None,
        json!({
            "token": token,
            "server_name": "Office server",
            "username": "alex",
            "display_name": "Alex Morgan",
            "email": "alex@example.org",
            "password": "alex password 1",
            "device": "test",
        }),
    )
    .await;
    assert_eq!(status, 200, "{session}");
    session["token"].as_str().unwrap().to_string()
}
