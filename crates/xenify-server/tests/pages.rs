//! Write documents read on the server against a real PostgreSQL: the
//! Files page's page view and public links work without the apps in the
//! browser, show changes made together since the last saved version, and
//! search finds their words (names only from a file's details).

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use xenify_wp_doc::collab::Change;
use xenify_wp_doc::{Block, CharFmt, Document, Paragraph, Span, Step};

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

#[tokio::test]
async fn pages_show_what_is_in_the_document_now() {
    let db = db_or_skip!("pages");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&sam)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();

    // A Write document with one paragraph, saved once.
    let mut doc = Document::new();
    let para = doc.alloc_id();
    doc.sections[0].blocks = vec![Block::Paragraph(Paragraph::new(para, "Normal", "Wombat handbook"))];
    let bytes = xenify_wp_io::save(&doc, xenify_wp_io::Format::Igdoc).unwrap();
    let meta = json!({ "name": "Burrows", "app": "write", "ext": "xwrite" }).to_string();
    let (_, f) = post(&addr, "/v1/nodes", Some(&sam), json!({ "space": mine, "kind": "file", "meta": meta })).await;
    let file = f["id"].as_i64().unwrap();
    // Nothing saved yet: no page.
    assert_eq!(get(&addr, &format!("/v1/nodes/{file}/page"), Some(&sam)).await.0, 404);
    assert_eq!(raw_post(&addr, &format!("/v1/nodes/{file}/content"), &sam, &bytes).await, 200);

    let (status, page) = get(&addr, &format!("/v1/nodes/{file}/page"), Some(&sam)).await;
    assert_eq!(status, 200, "{page}");
    assert!(page["html"].as_str().unwrap().contains("Wombat handbook"));
    // Saved without the app sending its words (uploaded): still found.
    let (_, found) = get(&addr, "/v1/search?q=wombat", Some(&sam)).await;
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    // A file's details other than its name don't match.
    let (_, found) = get(&addr, "/v1/search?q=write", Some(&sam)).await;
    assert_eq!(found, json!([]));
    let (_, found) = get(&addr, "/v1/search?q=burr", Some(&sam)).await;
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    // Someone who may not see it gets nothing.
    let kim = add_person(&addr, &admin, "kim", false).await;
    assert_eq!(get(&addr, &format!("/v1/nodes/{file}/page"), Some(&kim)).await.0, 404);

    // Typed while editing together, never saved as a version.
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/live")).await.unwrap();
    let (mut tx, mut rx) = ws.split();
    let send = |v: Value| Message::Text(v.to_string().into());
    tx.send(send(json!({ "t": "hello", "token": sam }))).await.unwrap();
    tx.send(send(json!({ "t": "join", "node": file, "since": 0 }))).await.unwrap();
    let change = Change {
        step: Step::Replace { para, offset: 15, remove: 0, insert: Span::plain(" and the quokka", CharFmt::default()) },
        ctx: Default::default(),
    };
    let op = serde_json::to_string(&change).unwrap();
    tx.send(send(json!({ "t": "send", "node": file, "base": 0, "client": "c1", "ops": [op] }))).await.unwrap();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), rx.next()).await.unwrap().unwrap().unwrap();
        if let Message::Text(t) = m
            && serde_json::from_str::<Value>(&t).unwrap()["t"] == "accepted"
        {
            break;
        }
    }
    let (_, page) = get(&addr, &format!("/v1/nodes/{file}/page"), Some(&sam)).await;
    assert!(page["html"].as_str().unwrap().contains("Wombat handbook and the quokka"), "{page}");
    // Its words follow a little later.
    let mut hits = json!([]);
    // Up to 30 s: other tests may be loading the machine.
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        hits = get(&addr, "/v1/search?q=quokka", Some(&sam)).await.1;
        if !hits.as_array().unwrap().is_empty() {
            break;
        }
    }
    assert_eq!(hits.as_array().unwrap().len(), 1, "{hits}");

    // A public link shows the same page.
    let (status, link) = post(&addr, &format!("/v1/nodes/{file}/public"), Some(&sam), json!({})).await;
    assert_eq!(status, 200, "{link}");
    let token = link["token"].as_str().unwrap();
    let (status, page) = get(&addr, &format!("/v1/p/{token}/page/{file}"), None).await;
    assert_eq!(status, 200, "{page}");
    assert!(page["html"].as_str().unwrap().contains("and the quokka"));
    assert_eq!(get(&addr, &format!("/v1/p/{token}/preview/{file}"), None).await.0, 404, "no picture of it yet");
}

/// A table from a Calc book on the server shows the book as it is now, even
/// when nobody has had the document open in Write since the book changed.
#[tokio::test]
async fn calc_tables_in_pages_follow_the_book() {
    let db = db_or_skip!("pages_calc");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&sam)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let (_, info) = get(&addr, "/v1/info", None).await;
    let here: String =
        info["server_id"].as_str().unwrap().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();

    // The book, with what Calc puts up for other documents.
    let meta = json!({ "name": "Stock", "app": "calc", "ext": "xcalc" }).to_string();
    let (_, b) = post(&addr, "/v1/nodes", Some(&sam), json!({ "space": mine, "kind": "file", "meta": meta })).await;
    let book = b["id"].as_i64().unwrap();
    let snap = |n: &str| {
        json!({ "format": "xenify-calc-link", "version": 1, "items": [
            { "id": "sheet:1", "kind": "sheet", "name": "Sheet1", "rows": [["Fruit", "Boxes"], ["Apples", n]], "header": true, "numbers": [false, true] },
        ]})
        .to_string()
    };
    assert_eq!(raw_post(&addr, &format!("/v1/nodes/{book}/preview"), &sam, snap("12").as_bytes()).await, 204);

    // A document showing its sheet as a table.
    let mut ed = xenify_wp_doc::Editor::new(Document::new());
    xenify_wp_doc::tables::insert_table(&mut ed, 2, 2).unwrap();
    let (id, _, _) = xenify_wp_doc::tables::cell_at(&ed, ed.sel.focus.para).unwrap();
    let rows = vec![vec!["Fruit".to_string(), "Boxes".to_string()], vec!["Apples".to_string(), "12".to_string()]];
    xenify_wp_doc::tables::fill_table(&mut ed, id, &rows, true, false, &[false, true]).unwrap();
    xenify_wp_doc::tables::set_linked(&mut ed, id, Some(format!("{here}|{book}|sheet:1"))).unwrap();
    let bytes = xenify_wp_io::save(&ed.doc, xenify_wp_io::Format::Igdoc).unwrap();
    let meta = json!({ "name": "Report", "app": "write", "ext": "xwrite" }).to_string();
    let (_, f) = post(&addr, "/v1/nodes", Some(&sam), json!({ "space": mine, "kind": "file", "meta": meta })).await;
    let file = f["id"].as_i64().unwrap();
    assert_eq!(raw_post(&addr, &format!("/v1/nodes/{file}/content"), &sam, &bytes).await, 200);

    // The book changes; nobody opens the document.
    assert_eq!(raw_post(&addr, &format!("/v1/nodes/{book}/preview"), &sam, snap("555").as_bytes()).await, 204);
    let (_, page) = get(&addr, &format!("/v1/nodes/{file}/page"), Some(&sam)).await;
    let html = page["html"].as_str().unwrap();
    assert!(html.contains("555") && !html.contains(">12<"), "{html}");
}
