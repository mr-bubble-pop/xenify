//! Editing together over `/v1/live` against a real PostgreSQL.

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

struct Window {
    ws: Ws,
}

impl Window {
    async fn open(addr: &str, token: &str) -> (Window, Value) {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/live")).await.unwrap();
        let mut w = Window { ws };
        w.send(json!({ "t": "hello", "token": token })).await;
        let welcome = w.next().await;
        (w, welcome)
    }

    async fn send(&mut self, v: Value) {
        self.ws.send(Message::Text(v.to_string().into())).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(5), self.ws.next())
                .await
                .expect("no answer")
                .unwrap()
                .unwrap();
            if let Message::Text(t) = m {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }

    /// The next message of kind `t`, skipping others (arrivals and such).
    async fn next_of(&mut self, t: &str) -> Value {
        loop {
            let v = self.next().await;
            if v["t"] == t {
                return v;
            }
        }
    }

    /// Nothing arrives for a moment.
    async fn quiet(&mut self) -> bool {
        tokio::time::timeout(Duration::from_millis(300), self.ws.next()).await.is_err()
    }
}

async fn person(addr: &str, admin: &str, name: &str, groups: &[i64]) -> String {
    let body = json!({ "username": name, "groups": groups });
    let (_, p) = post(addr, "/v1/admin/people", Some(admin), body).await;
    let (_, s) = use_link(addr, p["code"].as_str().unwrap(), name, &format!("{name} password")).await;
    s["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn two_people_edit_one_document_in_one_order() {
    let db = db_or_skip!("live");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design" })).await;
    let design = g["id"].as_i64().unwrap();
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Readers" })).await;
    let readers = g["id"].as_i64().unwrap();
    let priya = person(&addr, &admin, "priya", &[design]).await;
    let sam = person(&addr, &admin, "sam", &[design]).await;
    let kim = person(&addr, &admin, "kim", &[readers]).await;
    let (_, sp) = post(&addr, "/v1/spaces", Some(&admin), json!({ "name": "Design" })).await;
    let space = sp["id"].as_i64().unwrap();
    post(
        &addr,
        &format!("/v1/spaces/{space}/access"),
        Some(&admin),
        json!({ "grants": [
            { "who": { "group": design }, "role": "editor" }, { "who": { "group": readers }, "role": "viewer" },
        ]}),
    )
    .await;
    let (_, f) = post(&addr, "/v1/nodes", Some(&priya), json!({ "space": space, "kind": "file", "meta": "m" })).await;
    let file = f["id"].as_i64().unwrap();

    // A bad token is turned away.
    let (_, bad) = Window::open(&addr, "nope").await;
    assert_eq!(bad["code"], "signed_out");

    let (mut p, welcome) = Window::open(&addr, &priya).await;
    assert_eq!(welcome["t"], "welcome");
    assert_eq!(welcome["account"]["username"], "priya");
    p.send(json!({ "t": "join", "node": file, "since": 0 })).await;
    let joined = p.next_of("joined").await;
    assert_eq!((joined["role"].clone(), joined["seq"].clone()), (json!("editor"), json!(0)));

    let (mut s, _) = Window::open(&addr, &sam).await;
    s.send(json!({ "t": "join", "node": file, "since": 0 })).await;
    let joined = s.next_of("joined").await;
    assert_eq!(joined["people"][0]["name"], "priya");
    let arrived = p.next_of("arrived").await;
    assert_eq!(arrived["who"]["name"], "sam");

    // Priya's batch goes in first.
    p.send(json!({ "t": "send", "node": file, "base": 0, "client": "p1", "ops": ["a", "b"] })).await;
    let ok = p.next_of("accepted").await;
    assert_eq!((ok["from"].clone(), ok["to"].clone()), (json!(1), json!(2)));
    let got = s.next_of("ops").await;
    assert_eq!(
        got["ops"].as_array().unwrap().iter().map(|o| o["data"].clone()).collect::<Vec<_>>(),
        [json!("a"), json!("b")]
    );
    assert!(p.quiet().await, "Priya isn't sent her own changes back");

    // Sam's comes too late: he's told what he missed and sends it again.
    s.send(json!({ "t": "send", "node": file, "base": 0, "client": "s1", "ops": ["c"] })).await;
    let behind = s.next_of("behind").await;
    assert_eq!(behind["client"], "s1");
    assert_eq!(behind["ops"].as_array().unwrap().len(), 2);
    s.send(json!({ "t": "send", "node": file, "base": 2, "client": "s1", "ops": ["c'"] })).await;
    let ok = s.next_of("accepted").await;
    assert_eq!(ok["from"], 3);
    let got = p.next_of("ops").await;
    assert_eq!(got["ops"][0]["seq"], 3);
    assert_eq!(got["ops"][0]["data"], "c'");

    // Sent again after a lost connection: taken once.
    s.send(json!({ "t": "send", "node": file, "base": 0, "client": "s1", "ops": ["c'"] })).await;
    let ok = s.next_of("accepted").await;
    assert_eq!((ok["from"].clone(), ok["to"].clone()), (json!(3), json!(3)));

    // Carets go to the others, and aren't kept.
    p.send(json!({ "t": "say", "node": file, "data": "caret 12" })).await;
    let said = s.next_of("said").await;
    assert_eq!((said["data"].clone(), said["from"]["name"].clone()), (json!("caret 12"), json!("priya")));

    // Ids never collide.
    p.send(json!({ "t": "lease", "node": file, "count": 10, "at_least": 50 })).await;
    let l = p.next_of("leased").await;
    assert_eq!((l["from"].clone(), l["to"].clone()), (json!(50), json!(60)));
    s.send(json!({ "t": "lease", "node": file, "count": 5, "at_least": 1 })).await;
    let l = s.next_of("leased").await;
    assert_eq!((l["from"].clone(), l["to"].clone()), (json!(60), json!(65)));

    // Readers can follow along but not change anything.
    let (mut k, _) = Window::open(&addr, &kim).await;
    k.send(json!({ "t": "join", "node": file, "since": 0 })).await;
    let joined = k.next_of("joined").await;
    assert_eq!(joined["role"], "viewer");
    assert_eq!(joined["ops"].as_array().unwrap().len(), 3);
    k.send(json!({ "t": "send", "node": file, "base": 3, "client": "k", "ops": ["x"] })).await;
    assert_eq!(k.next_of("error").await["code"], "not_allowed");

    // A saved version holds the first two changes; opening later starts there.
    let (status, saved) = {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
        c.write_all(format!("POST /v1/nodes/{file}/content?upto=2 HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {priya}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nsnap").as_bytes()).await.unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).await.unwrap();
        (out.split_whitespace().nth(1).unwrap().to_string(), out.split_once("\r\n\r\n").unwrap().1.to_string())
    };
    assert_eq!(status, "200", "{saved}");
    let (mut late, _) = Window::open(&addr, &sam).await;
    late.send(json!({ "t": "join", "node": file, "since": 0 })).await;
    let joined = late.next_of("joined").await;
    assert_eq!((joined["version"].clone(), joined["upto"].clone()), (json!(1), json!(2)));
    assert_eq!(joined["ops"].as_array().unwrap().iter().map(|o| o["seq"].clone()).collect::<Vec<_>>(), [json!(3)]);

    // A big change (a picture put in a canvas) goes in too.
    p.send(json!({ "t": "send", "node": file, "base": 3, "client": "p2", "ops": ["p".repeat(3 << 20)] })).await;
    let ok = p.next_of("accepted").await;
    assert_eq!(ok["from"], 4);

    // Leaving tells the others.
    drop(s);
    let left = p.next_of("left").await;
    assert_eq!(left["t"], "left");
}
