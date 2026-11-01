//! Shared test helpers: a fresh database per test, a server on a free port,
//! accounts, HTTP posts, and a WebSocket app.

#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use xenify_chat_proto::{Session, ToApp, ToServer};
use xenify_chat_server::{Config, Server};

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
    let s = Server::start(Config { database: database.into(), local_webhooks: true }).await.unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let r = s.chat_routes();
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    (format!("127.0.0.1:{}", addr.port()), s)
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

/// A new account with a session (the Xenify server makes accounts; chat
/// only needs them in its tables).
pub async fn register(addr: &str, s: &Server, user: &str) -> Session {
    register_as(addr, s, user, "").await
}

pub async fn register_as(_addr: &str, s: &Server, user: &str, display_name: &str) -> Session {
    let c = s.pool().get().await.unwrap();
    let user =
        xenify_chat_server::db::create_user(&c, user, &xenify_chat_server::clean_name(display_name), "").await.unwrap();
    let token = xenify_chat_server::issue_token(s.pool(), user.id, "test").await.unwrap();
    Session { token, user }
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
