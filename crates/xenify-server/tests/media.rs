//! Pictures and clips in Draw canvases: sent in parts (carrying on after a
//! break), read back whole or by range, put together for downloads and taken
//! apart from uploads.

mod common;

use common::*;
use serde_json::{Value, json};
use std::io::{Read, Write};

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

/// An HTTP request with extra headers; (status, headers, body).
async fn req(addr: &str, method: &str, path: &str, token: &str, extra: &str, body: &[u8]) -> (u16, String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let kind = if extra.contains("Content-Type") { "" } else { "Content-Type: application/octet-stream\r\n" };
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n{kind}{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head, out[split + 4..].to_vec())
}

fn json_of(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

fn sha(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
}

async fn canvas(addr: &str, token: &str, space: i64, name: &str) -> i64 {
    let meta = json!({ "name": name, "app": "draw", "ext": "xdraw" }).to_string();
    let (status, n) =
        post(addr, "/v1/nodes", Some(token), json!({ "space": space, "kind": "file", "meta": meta })).await;
    assert_eq!(status, 200, "{n}");
    n["id"].as_i64().unwrap()
}

#[tokio::test]
async fn pictures_go_up_in_parts_once_and_come_back_whole_or_in_pieces() {
    let db = db_or_skip!("node_media");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = list[0]["id"].as_i64().unwrap();
    let file = canvas(&addr, &admin, mine, "Clip").await;

    // A clip of 9 MB: three parts.
    let clip: Vec<u8> = (0..9_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let hash = sha(&clip);
    let at = format!("/v1/nodes/{file}/media/{hash}");
    let size = clip.len();
    let start = |token: String| {
        let (addr, at) = (addr.clone(), at.clone());
        async move {
            let body = json!({ "size": size }).to_string();
            let (s, _, b) =
                req(&addr, "POST", &at, &token, "Content-Type: application/json\r\n", body.as_bytes()).await;
            (s, json_of(&b))
        }
    };
    let part = 4 << 20;
    let (s, got) = start(admin.clone()).await;
    assert_eq!((s, got["have"].as_i64(), got["part"].as_i64()), (200, Some(0), Some(part as i64)), "{got}");
    let (s, _, b) = req(&addr, "PUT", &format!("{at}?at=0&size={size}"), &admin, "", &clip[..part]).await;
    assert_eq!((s, json_of(&b)["have"].as_i64()), (200, Some(part as i64)));

    // The connection broke: asked again, it carries on from there.
    let (_, got) = start(admin.clone()).await;
    assert_eq!(got["have"].as_i64(), Some(part as i64));
    // A part sent twice goes nowhere, and says where the next one goes.
    let (s, _, b) = req(&addr, "PUT", &format!("{at}?at=0&size={size}"), &admin, "", &clip[..part]).await;
    assert_eq!((s, json_of(&b)["have"].as_i64()), (409, Some(part as i64)));
    // Nobody reads it before it's all there.
    assert_eq!(req(&addr, "GET", &at, &admin, "", b"").await.0, 404);
    let (s, _, _) = req(&addr, "PUT", &format!("{at}?at={part}&size={size}"), &admin, "", &clip[part..2 * part]).await;
    assert_eq!(s, 200);
    let (s, _, b) =
        req(&addr, "PUT", &format!("{at}?at={}&size={size}", 2 * part), &admin, "", &clip[2 * part..]).await;
    assert_eq!((s, json_of(&b)["done"].as_bool()), (200, Some(true)), "{}", String::from_utf8_lossy(&b));
    // Sent once: asking again says it's there.
    assert_eq!(start(admin.clone()).await.1["done"].as_bool(), Some(true));

    let (s, head, b) = req(&addr, "GET", &at, &admin, "", b"").await;
    assert_eq!(s, 200);
    assert!(b == clip, "the whole clip back ({} bytes)", b.len());
    assert!(head.to_lowercase().contains("accept-ranges: bytes"), "{head}");
    let (s, head, b) = req(&addr, "GET", &at, &admin, "Range: bytes=5000000-5000009\r\n", b"").await;
    assert_eq!((s, b.as_slice()), (206, &clip[5_000_000..5_000_010]));
    assert!(head.to_lowercase().contains(&format!("content-range: bytes 5000000-5000009/{size}")), "{head}");
    assert_eq!(req(&addr, "GET", &at, &admin, "Range: bytes=9000000-\r\n", b"").await.0, 416);

    // Bytes that aren't what their name says are turned away.
    let wrong = sha(b"something else");
    let (s, _, b) =
        req(&addr, "PUT", &format!("/v1/nodes/{file}/media/{wrong}?at=0&size=5"), &admin, "", b"hello").await;
    assert_eq!(s, 422, "{}", String::from_utf8_lossy(&b));

    // Someone who can't see the canvas can't see its pictures either.
    let (_, p) = post(&addr, "/v1/admin/people", Some(&admin), json!({ "username": "sam", "groups": [] })).await;
    let (_, sam) = use_link(&addr, p["code"].as_str().unwrap(), "sam", "sam password").await;
    let sam = sam["token"].as_str().unwrap().to_string();
    assert_eq!(req(&addr, "GET", &at, &sam, "", b"").await.0, 404);
    assert_eq!(start(sam.clone()).await.0, 404);

    // A version names the clip; downloaded as a file, the clip comes with it.
    let doc = format!(r#"{{"format_version":1,"media":[{{"source":{{"type":"embedded","data":"sha256:{hash}"}}}}]}}"#);
    let (s, _, _) = req(&addr, "POST", &format!("/v1/nodes/{file}/content"), &admin, "", doc.as_bytes()).await;
    assert_eq!(s, 200);
    let (s, _, plain) = req(&addr, "GET", &format!("/v1/nodes/{file}/content"), &admin, "", b"").await;
    assert_eq!((s, plain), (200, doc.as_bytes().to_vec()), "the canvas alone for the apps");
    let (s, head, zipped) = req(&addr, "GET", &format!("/v1/nodes/{file}/content?file=true"), &admin, "", b"").await;
    assert_eq!(s, 200);
    assert!(head.contains("application/zip"), "{head}");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&zipped)).unwrap();
    let mut text = String::new();
    z.by_name("document.json").unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, doc);
    let mut inside = Vec::new();
    z.by_name(&format!("media/{hash}")).unwrap().read_to_end(&mut inside).unwrap();
    assert!(inside == clip);

    // That file uploaded into another canvas: taken apart again.
    let other = canvas(&addr, &admin, mine, "Copy").await;
    let (s, _, _) = req(&addr, "POST", &format!("/v1/nodes/{other}/content"), &admin, "", &zipped).await;
    assert_eq!(s, 200);
    let (_, _, plain) = req(&addr, "GET", &format!("/v1/nodes/{other}/content"), &admin, "", b"").await;
    assert_eq!(plain, doc.as_bytes());
    let (s, _, b) = req(&addr, "GET", &format!("/v1/nodes/{other}/media/{hash}"), &admin, "", b"").await;
    assert!(s == 200 && b == clip);

    // A zip that isn't a canvas, in a canvas, is kept as it is.
    let mut odd = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    odd.start_file("notes.txt", zip::write::SimpleFileOptions::default()).unwrap();
    odd.write_all(b"hi").unwrap();
    let odd = odd.finish().unwrap().into_inner();
    req(&addr, "POST", &format!("/v1/nodes/{other}/content"), &admin, "", &odd).await;
    assert_eq!(req(&addr, "GET", &format!("/v1/nodes/{other}/content"), &admin, "", b"").await.2, odd);
}

#[tokio::test]
async fn storage_limits_and_half_sent_files() {
    let db = db_or_skip!("media_limits");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, list) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let mine = list[0]["id"].as_i64().unwrap();
    let file = canvas(&addr, &admin, mine, "Big").await;
    let start = |hash: String, size: i64| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move {
            let body = json!({ "size": size }).to_string();
            let at = format!("/v1/nodes/{file}/media/{hash}");
            let (s, _, b) = req(&addr, "POST", &at, &admin, "Content-Type: application/json\r\n", body.as_bytes()).await;
            (s, json_of(&b))
        }
    };

    // No limit at first.
    let (_, st) = get(&addr, "/v1/admin/storage", Some(&admin)).await;
    assert_eq!(st["quota_gb"].as_i64(), Some(0), "{st}");
    let two_gb = 2i64 << 30;
    assert_eq!(start(sha(b"x"), 1 << 20).await.0, 200);

    // 1 GB each: a 2 GB clip doesn't fit, a small picture does.
    let (s, _) = post(&addr, "/v1/admin/storage", Some(&admin), json!({ "quota_gb": 1 })).await;
    assert_eq!(s, 204);
    let (_, st) = get(&addr, "/v1/admin/storage", Some(&admin)).await;
    assert_eq!(st["quota_gb"].as_i64(), Some(1));
    let (s, b) = start(sha(b"x"), two_gb).await;
    assert_eq!((s, b["code"].as_str()), (507, Some("quota")), "{b}");
    let at = format!("/v1/nodes/{file}/media/{}?at=0&size={two_gb}", sha(b"x"));
    assert_eq!(req(&addr, "PUT", &at, &admin, "", b"start").await.0, 507);
    let pic = b"a small picture";
    let at = format!("/v1/nodes/{file}/media/{}?at=0&size={}", sha(pic), pic.len());
    let (s, _, b) = req(&addr, "PUT", &at, &admin, "", pic).await;
    assert_eq!(s, 200, "{}", String::from_utf8_lossy(&b));
    assert!(post(&addr, "/v1/admin/storage", Some(&admin), json!({ "quota_gb": -1 })).await.0 == 400);
    post(&addr, "/v1/admin/storage", Some(&admin), json!({ "quota_gb": 0 })).await;

    // Many files half sent at once: past the limit, the next is refused.
    let mut refused = false;
    for i in 0..70 {
        let hash = sha(format!("part {i}").as_bytes());
        let (s, _, _) = req(&addr, "PUT", &format!("/v1/nodes/{file}/media/{hash}?at=0&size=100"), &admin, "", b"x").await;
        if s == 429 {
            refused = true;
            break;
        }
        assert_eq!(s, 200);
    }
    assert!(refused);
}
