//! Virus scanning of uploads, against a stand-in for ClamAV's clamd that
//! finds the EICAR test file and nothing else.

mod common;

use common::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A tiny clamd: reads one INSTREAM and answers FOUND for EICAR, else OK.
async fn fake_clamd() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let mut cmd = [0u8; 10];
                if s.read_exact(&mut cmd).await.is_err() || &cmd != b"zINSTREAM\0" {
                    return;
                }
                let mut data = Vec::new();
                loop {
                    let mut len = [0u8; 4];
                    if s.read_exact(&mut len).await.is_err() {
                        return;
                    }
                    let n = u32::from_be_bytes(len) as usize;
                    if n == 0 {
                        break;
                    }
                    let mut part = vec![0u8; n];
                    if s.read_exact(&mut part).await.is_err() {
                        return;
                    }
                    data.extend(part);
                }
                let found = data.windows(5).any(|w| w == b"EICAR");
                let _ = s.write_all(if found { b"stream: Eicar-Test-Signature FOUND\0" as &[u8] } else { b"stream: OK\0" }).await;
            });
        }
    });
    addr
}

/// Saves raw bytes as a file's content; returns (status, body).
async fn put(addr: &str, token: &str, id: i64, bytes: &[u8]) -> (u16, Value) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /v1/nodes/{id}/content HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(bytes).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).unwrap_or(Value::Null))
}

const EICAR: &str = r"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

#[tokio::test]
async fn uploads_are_scanned_when_an_admin_turns_it_on() {
    let Some(db) = fresh_db("scan_uploads").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let clamd = fake_clamd().await;
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let (_, f) = post(&addr, "/v1/nodes", Some(&admin), json!({ "space": mine, "kind": "file", "meta": "{\"name\":\"x.txt\"}" })).await;
    let id = f["id"].as_i64().unwrap();

    // Off: anything is kept.
    assert_eq!(put(&addr, &admin, id, EICAR.as_bytes()).await.0, 200);

    // The Test button: the stand-in finds the test file; nothing listening fails.
    let (_, t) = post(&addr, "/v1/admin/scan/test", Some(&admin), json!({ "address": clamd })).await;
    assert_eq!(t["ok"], true, "{t}");
    let (_, t) = post(&addr, "/v1/admin/scan/test", Some(&admin), json!({ "address": "127.0.0.1:1" })).await;
    assert_eq!(t["ok"], false, "{t}");
    // Turning it on with a scanner that doesn't answer is refused.
    let (status, _) = post(&addr, "/v1/admin/scan", Some(&admin), json!({ "on": true, "address": "127.0.0.1:1" })).await;
    assert_eq!(status, 400);
    let (status, _) = post(&addr, "/v1/admin/scan", Some(&admin), json!({ "on": true, "address": clamd })).await;
    assert_eq!(status, 204);

    // On: a clean file is kept; the test virus isn't, and the reason says what it was.
    assert_eq!(put(&addr, &admin, id, b"just some words").await.0, 200);
    let (status, e) = put(&addr, &admin, id, format!("hello {EICAR}").as_bytes()).await;
    assert_eq!(status, 422, "{e}");
    assert_eq!(e["code"], "virus");
    assert!(e["error"].as_str().unwrap().contains("Eicar-Test-Signature"), "{e}");
    // A file bigger than one chunk goes in many chunks, and is still found.
    let mut big = vec![b'a'; 300 * 1024];
    big.extend(EICAR.as_bytes());
    assert_eq!(put(&addr, &admin, id, &big).await.0, 422);

    // The scanner gone: nothing is uploaded, and the admin pages say why.
    let (status, _) = post(&addr, "/v1/admin/scan", Some(&admin), json!({ "on": false, "address": "127.0.0.1:1" })).await;
    assert_eq!(status, 204);
    let c = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(c.1);
    c.0.execute("UPDATE settings SET value = '{\"on\":true,\"address\":\"127.0.0.1:1\"}' WHERE key = 'scan'", &[]).await.unwrap();
    let (status, e) = put(&addr, &admin, id, b"just some words").await;
    assert_eq!(status, 503, "{e}");
    assert_eq!(e["code"], "scan_down");
    let (_, set) = get(&addr, "/v1/admin/scan", Some(&admin)).await;
    assert_eq!(set["status"]["ok"], false, "{set}");
    assert!(set["status"]["text"].as_str().unwrap().contains("refused"), "{set}");
}
