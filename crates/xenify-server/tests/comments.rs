//! Sharing a Write document "can comment" against a real PostgreSQL:
//! comments (and replies, and deleting your own) go in, anything else is
//! refused; whoever may edit deletes anyone's.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use xenify_wp_doc::{Block, Document, Paragraph};

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

async fn raw_post(addr: &str, path: &str, token: &str, body: &[u8]) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// Joins `file` as `token`; returns a way to send changes and read answers.
async fn window(addr: &str, token: &str, file: i64) -> (impl FnMut(Value) -> Message, Ws, Value) {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/live")).await.unwrap();
    let msg = |v: Value| Message::Text(v.to_string().into());
    ws.send(msg(json!({ "t": "hello", "token": token }))).await.unwrap();
    ws.send(msg(json!({ "t": "join", "node": file, "since": 0 }))).await.unwrap();
    let joined = answer(&mut ws, "joined").await;
    (msg, ws, joined)
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next answer of kind `t` (or an error).
async fn answer(ws: &mut Ws, t: &str) -> Value {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
        if let Message::Text(text) = m {
            let v: Value = serde_json::from_str(&text).unwrap();
            if v["t"] == t || v["t"] == "error" {
                return v;
            }
        }
    }
}

/// The changes made in `ed` since last time, as sent.
fn made(ed: &mut xenify_wp_doc::Editor) -> Vec<String> {
    ed.take_changes().iter().map(|c| serde_json::to_string(c).unwrap()).collect()
}

#[tokio::test]
async fn someone_who_may_comment_adds_comments_and_nothing_else() {
    use xenify_wp_doc::{Editor, Pos, Selection, review};
    let db = db_or_skip!("pages_comment");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let kim = add_person(&addr, &admin, "kim", false).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&sam)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let mut doc = Document::new();
    let para = doc.alloc_id();
    doc.sections[0].blocks = vec![Block::Paragraph(Paragraph::new(para, "Normal", "The cat sat on the mat."))];
    doc.next_id = 1000;
    let meta = json!({ "name": "Cats", "app": "write", "ext": "xwrite" }).to_string();
    let (_, f) = post(&addr, "/v1/nodes", Some(&sam), json!({ "space": mine, "kind": "file", "meta": meta })).await;
    let file = f["id"].as_i64().unwrap();
    let bytes = xenify_wp_io::save(&doc, xenify_wp_io::Format::Igdoc).unwrap();
    assert_eq!(raw_post(&addr, &format!("/v1/nodes/{file}/content"), &sam, &bytes).await, 200);
    let grants = json!({ "own": true, "grants": [{ "who": "everyone", "role": "commenter" }] });
    post(&addr, &format!("/v1/nodes/{file}/access"), Some(&sam), grants).await;

    // Sam comments on "cat" (he may edit).
    let (msg, mut s, _) = window(&addr, &sam, file).await;
    let mut ed = Editor::new(doc.clone());
    ed.set_live(true);
    ed.doc.user = "sam".into();
    ed.sel = Selection { anchor: Pos::new(para, 4), focus: Pos::new(para, 7) };
    let sams = review::add_comment(&mut ed, "a dog?").unwrap();
    let mut msg = msg;
    s.send(msg(json!({ "t": "send", "node": file, "base": 0, "client": "s1", "ops": made(&mut ed) }))).await.unwrap();
    let seq = answer(&mut s, "accepted").await["to"].as_i64().unwrap();

    // Kim may comment: she gets ids, comments and replies.
    let (_, mut k, joined) = window(&addr, &kim, file).await;
    assert_eq!(joined["role"], "commenter");
    k.send(msg(json!({ "t": "lease", "node": file, "count": 100, "at_least": 1 }))).await.unwrap();
    let leased = answer(&mut k, "leased").await;
    assert_eq!(leased["t"], "leased", "{leased}");
    ed.doc.user = "kim".into();
    ed.doc.next_id = leased["from"].as_u64().unwrap();
    ed.sel = Selection { anchor: Pos::new(para, 19), focus: Pos::new(para, 22) };
    let kims = review::add_comment(&mut ed, "a rug?").unwrap();
    review::reply(&mut ed, sams, "no, a cat").unwrap();
    let ops = made(&mut ed);
    k.send(msg(json!({ "t": "send", "node": file, "base": seq, "client": "k1", "ops": ops }))).await.unwrap();
    let ok = answer(&mut k, "accepted").await;
    assert_eq!(ok["t"], "accepted", "{ok}");
    let seq = ok["to"].as_i64().unwrap();

    // Not the words, nor Sam's comment.
    let before = ed.doc.clone();
    ed.sel = Selection::caret(Pos::new(para, 3));
    xenify_wp_doc::ops::insert_text(&mut ed, " big", None).unwrap();
    k.send(msg(json!({ "t": "send", "node": file, "base": seq, "client": "k2", "ops": made(&mut ed) }))).await.unwrap();
    let no = answer(&mut k, "accepted").await;
    assert_eq!((no["code"].clone(), no["error"].clone()), (json!("not_allowed"), json!("you may only comment on this document")));
    ed.reset_to(before.clone());
    review::delete_comment(&mut ed, sams).unwrap();
    k.send(msg(json!({ "t": "send", "node": file, "base": seq, "client": "k3", "ops": made(&mut ed) }))).await.unwrap();
    let no = answer(&mut k, "accepted").await;
    assert_eq!(no["error"], "you can only change or delete your own comments");
    // Her own, yes.
    ed.reset_to(before.clone());
    review::delete_comment(&mut ed, kims).unwrap();
    k.send(msg(json!({ "t": "send", "node": file, "base": seq, "client": "k4", "ops": made(&mut ed) }))).await.unwrap();
    let seq = answer(&mut k, "accepted").await["to"].as_i64().unwrap();

    // Whoever may edit deletes anyone's.
    ed.doc.user = "sam".into();
    review::delete_comment(&mut ed, sams).unwrap();
    s.send(msg(json!({ "t": "send", "node": file, "base": seq, "client": "s2", "ops": made(&mut ed) }))).await.unwrap();
    assert_eq!(answer(&mut s, "accepted").await["t"], "accepted");
    let (_, page) = get(&addr, &format!("/v1/nodes/{file}/page"), Some(&sam)).await;
    assert!(page["html"].as_str().unwrap().contains("The cat sat on the mat."), "{page}");
}
