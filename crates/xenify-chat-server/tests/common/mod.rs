//! Shared test helpers: a fresh database per test, a server on a free port,
//! HTTP posts, and a WebSocket app.

#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use xenify_chat_proto::{Login, Session, ToApp, ToServer};
use xenify_chat_server::{Config, Server};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub fn admin() -> String {
    std::env::var("XENIFY_TEST_PG").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/postgres".into())
}

/// A new empty database; None when there's no PostgreSQL to test with.
pub async fn fresh_db(name: &str) -> Option<String> {
    let (c, conn) = tokio_postgres::connect(&admin(), tokio_postgres::NoTls).await.ok()?;
    tokio::spawn(conn);
    let db = format!("chat_{name}_{}", std::process::id());
    let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {db}")).await;
    c.batch_execute(&format!("CREATE DATABASE {db}")).await.ok()?;
    let base = admin();
    Some(format!("{}/{db}", base.rsplit_once('/').unwrap().0))
}

pub async fn serve(database: &str) -> (String, Server) {
    let s = Server::start(Config { database: database.into(), open_registration: false, local_webhooks: true })
        .await
        .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let r = s.router();
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    (format!("127.0.0.1:{}", addr.port()), s)
}

/// POSTs JSON; returns (status, body).
pub async fn post(addr: &str, path: &str, body: &Login) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let json = serde_json::to_string(body).unwrap();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

pub fn login(user: &str, password: &str, invite: &str) -> Login {
    Login {
        username: user.into(),
        password: password.into(),
        device: "test".into(),
        invite: invite.into(),
        display_name: String::new(),
    }
}

pub struct App {
    pub ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl App {
    pub async fn connect(addr: &str, token: &str) -> App {
        let mut req = format!("ws://{addr}/v1/ws").into_client_request().unwrap();
        req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let mut a = App { ws };
        assert!(matches!(a.next().await, ToApp::Welcome { .. }));
        a
    }
    pub async fn send(&mut self, m: ToServer) {
        self.ws.send(Ws::Text(serde_json::to_string(&m).unwrap().into())).await.unwrap();
    }
    pub async fn next(&mut self) -> ToApp {
        loop {
            let f = tokio::time::timeout(Duration::from_secs(10), self.ws.next())
                .await
                .expect("a reply in time")
                .unwrap()
                .unwrap();
            if let Ws::Text(t) = f {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }
    /// The next reply matching `ok`, skipping others.
    pub async fn wait(&mut self, ok: impl Fn(&ToApp) -> bool) -> ToApp {
        loop {
            let m = self.next().await;
            if ok(&m) {
                return m;
            }
        }
    }
}

pub async fn register(addr: &str, s: &Server, user: &str) -> Session {
    let code = xenify_chat_server::invite(s.pool(), 1).await.unwrap();
    let (status, body) = post(addr, "/v1/register", &login(user, "correct horse battery", &code)).await;
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

/// POSTs JSON with an `Authorization: Bearer` key; returns (status, body).
pub async fn post_as(addr: &str, path: &str, token: &str, body: &serde_json::Value) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let json = body.to_string();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}
