//! Backups to a folder: made from Admin › Backups, only changes stored the
//! second time, and restored into an empty database and data folder; and
//! the clean-up of bins and unused bytes.

mod common;

use common::*;
use serde_json::{Value, json};
use xenify_server::{Config, Server, backup};

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

async fn put(addr: &str, token: &str, id: i64, bytes: &[u8]) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /v1/nodes/{id}/content HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(bytes).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out.split_whitespace().nth(1).unwrap().parse().unwrap()
}

async fn fetch(addr: &str, token: &str, id: i64) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(
        format!("GET /v1/nodes/{id}/content HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).to_string();
    (head.split_whitespace().nth(1).unwrap().parse().unwrap(), out[split + 4..].to_vec())
}

async fn new_file(addr: &str, token: &str, name: &str) -> i64 {
    let (_, spaces) = get(addr, "/v1/spaces", Some(token)).await;
    let space = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let meta = json!({ "name": name, "app": "write", "ext": "xwrite" }).to_string();
    let (status, n) = post(addr, "/v1/nodes", Some(token), json!({ "space": space, "kind": "file", "meta": meta })).await;
    assert_eq!(status, 200, "{n}");
    n["id"].as_i64().unwrap()
}

/// Bytes that don't compress (like a video).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[tokio::test]
async fn backs_up_to_a_folder_and_restores_into_an_empty_server() {
    let db = db_or_skip!("backup");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let big = noise(3 << 20, 7);
    let id = new_file(&addr, &admin, "Holiday film").await;
    assert_eq!(put(&addr, &admin, id, &big).await, 200);

    let place = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("backups-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&place);
    // Encryption on needs a passphrase of 8 or more.
    let mut set = json!({ "on": true, "kind": "folder", "folder": place, "encrypt": true, "passphrase": "short" });
    let (status, _) = call(&addr, "PUT", "/v1/admin/backups", Some(&admin), Some(json!({ "settings": set }))).await;
    assert_eq!(status, 400);
    set["passphrase"] = json!("correct horse battery");
    let (status, page) = call(&addr, "PUT", "/v1/admin/backups", Some(&admin), Some(json!({ "settings": set, "bin_days": 14 }))).await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["settings"]["passphrase"], "");
    assert_eq!(page["settings"]["passphrase_set"], true);
    assert_eq!(page["bin_days"], 14);
    assert!(page["public_key"].as_str().unwrap().starts_with("ssh-ed25519 "));
    // Check: nothing there yet.
    let (_, c) = post(&addr, "/v1/admin/backups/check", Some(&admin), json!({ "settings": set })).await;
    assert_eq!(c["ok"], true, "{c}");
    assert_eq!(c["there"], false);

    backup::run(&server).await.unwrap();
    let (_, page) = get(&addr, "/v1/admin/backups", Some(&admin)).await;
    let st = &page["status"];
    assert_eq!(st["error"], "", "{st}");
    assert_eq!(st["snapshots"].as_array().unwrap().len(), 1);
    let first_added = st["added"].as_u64().unwrap();
    assert!(first_added > 3 << 20, "the film went up: {st}");
    // Encrypted: the film's bytes aren't there to read.
    let mut found = false;
    for e in walk(&place) {
        let b = std::fs::read(e).unwrap();
        found |= b.windows(64).any(|w| w == &big[1000..1064]);
    }
    assert!(!found, "the backup must be encrypted");

    // A second, after a small change: only that is stored.
    let small = new_file(&addr, &admin, "Notes").await;
    assert_eq!(put(&addr, &admin, small, b"a few words").await, 200);
    backup::run(&server).await.unwrap();
    let (_, page) = get(&addr, "/v1/admin/backups", Some(&admin)).await;
    let st = &page["status"];
    assert_eq!(st["snapshots"].as_array().unwrap().len(), 2, "{st}");
    assert!(st["added"].as_u64().unwrap() < 1 << 20, "only changes the second time: {st}");
    let (_, c) = post(&addr, "/v1/admin/backups/check", Some(&admin), json!({ "settings": set })).await;
    assert_eq!((c["there"].clone(), c["opens"].clone()), (json!(true), json!(true)), "{c}");
    let wrong = json!({ "kind": "folder", "folder": place, "encrypt": true, "passphrase": "not the right one" });
    let (_, c) = post(&addr, "/v1/admin/backups/check", Some(&admin), json!({ "settings": wrong })).await;
    assert_eq!(c["opens"], false, "{c}");

    // Restore into an empty database and data folder, as after losing the
    // computer: the place and passphrase are all that's needed.
    let db2 = fresh_db("backup_restored").await.unwrap();
    let data2 = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("restored-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data2);
    let mut from = backup::place_from(&place.to_string_lossy()).unwrap();
    from.passphrase = "correct horse battery".into();
    let paths = backup::Paths { database: db2.clone(), data: data2.clone(), secure: data2.clone(), postgres: None };
    let list = {
        let from = from.clone();
        tokio::task::spawn_blocking(move || backup::snapshots(&from, "")).await.unwrap().unwrap()
    };
    assert_eq!(list.len(), 2);
    tokio::task::spawn_blocking(move || backup::restore(&from, "", "latest", &paths, &|m| eprintln!("{m}")))
        .await
        .unwrap()
        .unwrap();
    let s2 = Server::start(Config { database: db2, data: data2, local_webhooks: true, web: None, secure: None, postgres: None })
        .await
        .unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });
    let (status, v) = sign_in(&addr2, "alex", "alex password 1").await;
    assert_eq!(status, 200, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let (status, bytes) = fetch(&addr2, &token, id).await;
    assert_eq!(status, 200);
    assert!(bytes == big, "the film came back whole");
    assert_eq!(fetch(&addr2, &token, small).await.1, b"a few words");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

#[tokio::test]
async fn bins_empty_themselves_and_unused_bytes_go() {
    let db = db_or_skip!("cleanup");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let id = new_file(&addr, &admin, "Old draft").await;
    let bytes = noise(1000, 3);
    assert_eq!(put(&addr, &admin, id, &bytes).await, 200);
    let keep = new_file(&addr, &admin, "Kept").await;
    assert_eq!(put(&addr, &admin, keep, b"still here").await, 200);
    let (status, _) = call(&addr, "DELETE", &format!("/v1/nodes/{id}"), Some(&admin), None).await;
    assert_eq!(status, 204);
    // In the bin, with the day it goes for good.
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&admin)).await;
    let space = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
    let (_, bin) = get(&addr, &format!("/v1/trash?space={space}"), Some(&admin)).await;
    assert!(bin[0]["gone"].is_string(), "{bin}");

    // Nothing goes yet: deleted just now.
    let r = xenify_server::cleanup::run(&server).await.unwrap();
    assert_eq!(r.from_bins, 0);
    // 31 days on (the bin keeps things 30), and the bytes not used for two.
    let (c, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    c.execute("UPDATE nodes SET trashed = now() - interval '31 days' WHERE id = $1", &[&id]).await.unwrap();
    let hash: String = c.query_one("SELECT blob FROM versions WHERE node = $1", &[&id]).await.unwrap().get(0);
    let data = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(db.rsplit('/').next().unwrap());
    let blob = data.join("blobs").join(&hash[..2]).join(&hash[2..]);
    assert!(blob.exists());
    let two_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 3600);
    for f in walk(&data.join("blobs")) {
        std::fs::File::options().write(true).open(&f).unwrap().set_modified(two_days_ago).unwrap();
    }
    let r = xenify_server::cleanup::run(&server).await.unwrap();
    assert_eq!(r.from_bins, 1);
    assert_eq!(r.blobs, 1, "{r:?}");
    assert!(!blob.exists());
    assert_eq!(fetch(&addr, &admin, keep).await.1, b"still here");
    // "Never": the bin keeps things until someone empties it.
    let (status, page) = call(&addr, "PUT", "/v1/admin/backups", Some(&admin), Some(json!({ "bin_days": 0 }))).await;
    assert_eq!((status, page["bin_days"].clone()), (200, Value::from(0)));
}

/// A server with backups set up, for looking at Admin › Backups:
/// `BK_WEB=<copy of web/> cargo test -p xenify-server --test backup screen -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn screen() {
    let Some(db) = fresh_db("backup_screen").await else { return };
    let data = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("backup_screen");
    let _ = std::fs::remove_dir_all(&data);
    let web = std::env::var("BK_WEB").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web"));
    let server = Server::start(Config { database: db, data, local_webhooks: true, web: Some(web), secure: None, postgres: None }).await.unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
    let s2 = server.clone();
    tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });
    let admin = set_up(&addr, &server).await;
    let id = new_file(&addr, &admin, "Holiday film").await;
    put(&addr, &admin, id, &noise(5 << 20, 9)).await;
    let place = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("backup_screen_place");
    let _ = std::fs::remove_dir_all(&place);
    let set = json!({ "on": true, "kind": "folder", "folder": place, "encrypt": true, "passphrase": "amber-river-quill-lotus-cedar-frost" });
    call(&addr, "PUT", "/v1/admin/backups", Some(&admin), Some(json!({ "settings": set }))).await;
    backup::run(&server).await.unwrap();
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    println!("SCREENS addr=http://{addr}");
    println!("SCREENS session={}", json!({ "token": admin, "account": me }));
    tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
}

/// A place that stops working after `left` uploads of data, like a
/// connection that drops partway through a big first backup.
#[derive(Debug)]
struct Drops {
    inner: std::sync::Arc<dyn backup::rustic_core::WriteBackend>,
    left: std::sync::atomic::AtomicI64,
    sent: std::sync::atomic::AtomicU64,
}

mod drops {
    use super::Drops;
    use backup::rustic_core::{BytesList, FileType, Id, ReadBackend, RusticResult, WriteBackend};
    use bytes::Bytes;
    use std::sync::atomic::Ordering;
    use xenify_server::backup;
    impl ReadBackend for Drops {
        fn location(&self) -> String {
            self.inner.location()
        }
        fn list_with_size(&self, t: FileType) -> RusticResult<Vec<(Id, u32)>> {
            self.inner.list_with_size(t)
        }
        fn read_full(&self, t: FileType, id: &Id) -> RusticResult<Bytes> {
            self.inner.read_full(t, id)
        }
        fn read_partial(&self, t: FileType, id: &Id, c: bool, o: u32, l: u32) -> RusticResult<Bytes> {
            self.inner.read_partial(t, id, c, o, l)
        }
        fn warmup_path(&self, t: FileType, id: &Id) -> String {
            self.inner.warmup_path(t, id)
        }
    }
    impl WriteBackend for Drops {
        fn create(&self) -> RusticResult<()> {
            self.inner.create()
        }
        fn write_bytes(&self, t: FileType, id: &Id, c: bool, b: BytesList) -> RusticResult<()> {
            if t == FileType::Pack && self.left.fetch_sub(1, Ordering::SeqCst) <= 0 {
                return Err(backup::rustic_core::RusticError::new(backup::rustic_core::ErrorKind::Backend, "the connection dropped"));
            }
            self.sent.fetch_add(b.size() as u64, Ordering::SeqCst);
            self.inner.write_bytes(t, id, c, b)
        }
        fn remove(&self, t: FileType, id: &Id, c: bool) -> RusticResult<()> {
            self.inner.remove(t, id, c)
        }
    }
}

/// A big first backup cut off partway: how much the next one sends again.
#[tokio::test]
async fn a_backup_cut_off_partway_carries_on() {
    let db = db_or_skip!("backup_cut");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    // 4 files of 30 MB that don't compress (videos).
    for i in 0..4 {
        let id = new_file(&addr, &admin, &format!("Clip {i}")).await;
        assert_eq!(put(&addr, &admin, id, &noise(30 << 20, 11 + 2 * i)).await, 200);
    }
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let place = tmp.join(format!("backups-cut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&place);
    let set = backup::Settings { on: true, kind: "folder".into(), folder: place.to_string_lossy().into(), passphrase: "correct horse battery".into(), ..Default::default() };
    let paths = backup::Paths { database: db.clone(), data: tmp.join(db.rsplit('/').next().unwrap()), secure: tmp.join(db.rsplit('/').next().unwrap()), postgres: None };
    let folder: std::sync::Arc<dyn backup::rustic_core::WriteBackend> = std::sync::Arc::new(backup::store::Folder::new(place.clone()));
    let run = |left: i64, repair: bool| {
        let (set, paths, folder) = (set.clone(), paths.clone(), folder.clone());
        tokio::task::spawn_blocking(move || {
            let be = std::sync::Arc::new(Drops { inner: folder, left: left.into(), sent: 0.into() });
            let r = backup::back_up_to(be.clone(), &set, &paths, false, repair, &|_| {});
            (r.map(|d| d.snapshots.len()), be.sent.load(std::sync::atomic::Ordering::SeqCst))
        })
    };
    let (r, sent) = run(3, false).await.unwrap();
    assert!(r.is_err(), "cut off");
    eprintln!("cut off after sending {} MB", sent >> 20);
    let (r, again) = run(i64::MAX, true).await.unwrap();
    assert_eq!(r.unwrap(), 1);
    eprintln!("the next one sent {} MB", again >> 20);
    // What went up before isn't sent again (the files are 120 MB).
    assert!(again < (120 << 20) - sent + (8 << 20), "sent {again} again after {sent}");
    // Whole: everything there now.
    let mut from = backup::place_from(&place.to_string_lossy()).unwrap();
    from.passphrase = "correct horse battery".into();
    let n = tokio::task::spawn_blocking(move || backup::snapshots(&from, "")).await.unwrap().unwrap();
    assert_eq!(n.len(), 1);
}

/// To a real SFTP server: `XENIFY_TEST_SFTP=user@host[:port]` and
/// `XENIFY_TEST_SFTP_KEY=<private key file>` (skipped without them).
#[tokio::test]
async fn backs_up_over_sftp_and_restores() {
    let (Ok(address), Ok(key)) = (std::env::var("XENIFY_TEST_SFTP"), std::env::var("XENIFY_TEST_SFTP_KEY")) else {
        return eprintln!("no SFTP server to test with: skipped");
    };
    let key = std::fs::read_to_string(key).unwrap();
    let db = db_or_skip!("backup_sftp");
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let film = noise(12 << 20, 5);
    let id = new_file(&addr, &admin, "Film").await;
    assert_eq!(put(&addr, &admin, id, &film).await, 200);
    let folder = format!("xenify-backup-test-{}", std::process::id());
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let data = tmp.join(db.rsplit('/').next().unwrap());
    let paths = backup::Paths { database: db.clone(), data: data.clone(), secure: data, postgres: None };
    let mut set = backup::Settings {
        on: true,
        kind: "sftp".into(),
        sftp_address: address.clone(),
        sftp_folder: folder.clone(),
        sftp_auth: "key".into(),
        passphrase: "correct horse battery".into(),
        ..Default::default()
    };
    // The first time: the server's key is learnt.
    let (_, fp) = tokio::task::spawn_blocking({
        let (set, key) = (set.clone(), key.clone());
        move || backup::open_place(&set, &key).map(|(_, fp)| ((), fp.unwrap()))
    })
    .await
    .unwrap()
    .unwrap();
    assert!(fp.starts_with("SHA256:"), "{fp}");
    // A different key there stops it.
    set.sftp_host_key = "SHA256:not-the-one".into();
    let wrong = tokio::task::spawn_blocking({
        let (set, key) = (set.clone(), key.clone());
        move || backup::open_place(&set, &key).map(|_| ())
    })
    .await
    .unwrap();
    assert!(wrong.unwrap_err().starts_with(backup::store::KEY_CHANGED));
    set.sftp_host_key = fp;
    for _ in 0..2 {
        let (set, key, paths) = (set.clone(), key.clone(), paths.clone());
        let d = tokio::task::spawn_blocking(move || backup::back_up(&set, &key, &paths, true, false, &|m| eprintln!("{m}")))
            .await
            .unwrap()
            .unwrap();
        eprintln!("added {} KB", d.added >> 10);
    }
    let db2 = fresh_db("backup_sftp_restored").await.unwrap();
    let data2 = tmp.join(format!("sftp-restored-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data2);
    let to = backup::Paths { database: db2.clone(), data: data2.clone(), secure: data2.clone(), postgres: None };
    {
        let (set, key) = (set.clone(), key.clone());
        tokio::task::spawn_blocking(move || backup::restore(&set, &key, "latest", &to, &|m| eprintln!("{m}"))).await.unwrap().unwrap();
    }
    let s2 = Server::start(Config { database: db2, data: data2, local_webhooks: true, web: None, secure: None, postgres: None }).await.unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = format!("127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });
    let (_, v) = sign_in(&addr2, "alex", "alex password 1").await;
    let (status, bytes) = fetch(&addr2, v["token"].as_str().unwrap(), id).await;
    assert_eq!(status, 200);
    assert!(bytes == film);
    eprintln!("SFTP folder left on the server: {folder}");
}
