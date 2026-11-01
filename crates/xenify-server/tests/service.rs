//! The service's supervisor: runs the server, and a newer version takes
//! over with no failed requests.
#![cfg(target_os = "linux")]

mod common;
use common::*;
use std::path::Path;
use std::time::Duration;
use xenify_server::service::{Plan, supervise};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Puts a "version" (a copy of the server program) in the service's store.
fn add_version(dir: &Path, version: &str) -> std::path::PathBuf {
    let vdir = dir.join("versions/apps/server").join(version);
    std::fs::create_dir_all(&vdir).unwrap();
    let program = vdir.join("xenify-server");
    std::fs::copy(env!("CARGO_BIN_EXE_xenify-server"), &program).unwrap();
    std::fs::write(
        dir.join("versions/apps/server/current.json"),
        format!(r#"{{"version":"{version}","run":"xenify-server"}}"#),
    )
    .unwrap();
    program
}

/// Processes running `program`.
fn running(program: &Path) -> usize {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|e| std::fs::read_link(e.ok()?.path().join("exe")).ok())
        .filter(|exe| exe == program)
        .count()
}

#[tokio::test]
async fn hands_over_without_downtime() {
    let Some(db) = fresh_db("svc").await else { return };
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("svc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join("service");
    std::fs::create_dir_all(&dir).unwrap();
    let config = tmp.join("config.toml");
    std::fs::write(&config, format!("database = \"{db}\"\ndata = \"{}\"\n", tmp.join("data").display())).unwrap();
    let port = free_port();
    // The worker's flags come through its environment here.
    unsafe {
        std::env::set_var("XENIFY_LISTEN", format!("127.0.0.1:{port}"));
        std::env::set_var("XENIFY_HTTPS", "off");
    }
    let first = add_version(&dir, "1.0.0");
    let plan = Plan {
        dir: dir.clone(),
        args: vec!["--config".into(), config.display().to_string()],
        postgres: None,
        also: vec![],
        at_rest: None,
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let sup = tokio::spawn(supervise(plan, async {
        let _ = stopped.await;
    }));
    let addr = format!("127.0.0.1:{port}");
    let mut up = false;
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(up, "the server didn't start");
    assert_eq!(get(&addr, "/v1/health", None).await.0, 200);
    assert_eq!(running(&first), 1);

    // Asking all the while: none fails.
    let asking = tokio::spawn({
        let addr = addr.clone();
        async move {
            let (mut ok, mut failed) = (0, 0);
            let end = std::time::Instant::now() + Duration::from_secs(25);
            while std::time::Instant::now() < end {
                match tokio::time::timeout(Duration::from_secs(5), raw_health(&addr)).await {
                    Ok(true) => ok += 1,
                    _ => failed += 1,
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            (ok, failed)
        }
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    let second = add_version(&dir, "2.0.0");
    let mut moved = false;
    for _ in 0..200 {
        if running(&second) == 1 && running(&first) == 0 {
            moved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(moved, "the new version didn't take over (old {}, new {})", running(&first), running(&second));
    let (ok, failed) = asking.await.unwrap();
    assert!(ok > 100, "{ok} answers");
    assert_eq!(failed, 0, "{failed} of {} requests failed during the handover", ok + failed);

    // Stopping the service stops the server.
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(40), sup).await.unwrap().unwrap();
    assert_eq!(running(&second), 0);
}

/// One GET /v1/health on a fresh connection.
async fn raw_health(addr: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(mut s) = tokio::net::TcpStream::connect(addr).await else { return false };
    if s.write_all(b"GET /v1/health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.is_err() {
        return false;
    }
    let mut out = String::new();
    s.read_to_string(&mut out).await.is_ok() && out.starts_with("HTTP/1.1 200")
}

/// A pretend release site offering this program as the server's 9.9.0.
async fn releases(program: &Path, tmp: &Path) -> (String, [u8; 32]) {
    use xenify_hub_core::manifest::{new_key, sha256_hex, sign};
    use xenify_hub_core::{Channel, Download, Manifest, Product, Release};
    let stage = tmp.join("pkg/xenify-server");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::copy(program, stage.join("xenify-server")).unwrap();
    let tgz = tmp.join("server.tar.gz");
    assert!(
        std::process::Command::new("tar")
            .arg("-C")
            .arg(tmp.join("pkg"))
            .arg("-czf")
            .arg(&tgz)
            .arg("xenify-server")
            .status()
            .unwrap()
            .success()
    );
    let bytes = std::fs::read(&tgz).unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let key = new_key();
    let (os, arch) = xenify_hub_core::platform();
    let m = Manifest {
        published: "2026-10-25T10:00:00Z".into(),
        products: vec![Product {
            id: "server".into(),
            name: "Server".into(),
            about: String::new(),
            component: false,
            requires: Vec::new(),
            releases: vec![Release {
                version: "9.9.0".into(),
                channel: Channel::Release,
                date: "2026-10-25".into(),
                notes: String::new(),
                downloads: vec![Download {
                    os: os.into(),
                    arch: arch.into(),
                    url: format!("{site}/server.tar.gz"),
                    size: bytes.len() as u64,
                    sha256: sha256_hex(&bytes),
                    run: "xenify-server/xenify-server".into(),
                }],
            }],
        }],
    };
    let signed = sign(&m, &key);
    let app = axum::Router::new()
        .route("/manifest.json", axum::routing::get(move || async move { axum::Json(signed) }))
        .route("/server.tar.gz", axum::routing::get(move || async move { bytes }));
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (site, key.verifying_key().to_bytes())
}

#[tokio::test]
async fn updates_itself_from_the_signed_list() {
    let Some(db) = fresh_db("svcup").await else { return };
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("svcup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join("service");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_xenify-server"), dir.join("xenify-server")).unwrap();
    let data = tmp.join("data");
    let config = tmp.join("config.toml");
    std::fs::write(&config, format!("database = \"{db}\"\ndata = \"{}\"\n", data.display())).unwrap();
    let (site, key) = releases(Path::new(env!("CARGO_BIN_EXE_xenify-server")), &tmp).await;
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("trusted-keys.json"), serde_json::to_string(&[key.to_vec()]).unwrap()).unwrap();
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let mut cmd = tokio::process::Command::new(dir.join("xenify-server"));
    cmd.arg("--config")
        .arg(&config)
        .args(["service", "run", "--dir"])
        .arg(&dir)
        .env("XENIFY_LISTEN", &addr)
        .env("XENIFY_HTTPS", "off")
        .env("XENIFY_UPDATE_SOON", "1")
        .kill_on_drop(true);
    let mut sup = cmd.spawn().unwrap();
    let mut up = false;
    for _ in 0..200 {
        if get_maybe(&addr).await {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(up, "the server didn't start");
    // Point it at the pretend site (the server's own default is Xenify's).
    let admin = set_up_with_link(&addr, &data).await;
    let (status, _) =
        post(&addr, "/v1/admin/hub", Some(&admin), serde_json::json!({ "source": format!("{site}/manifest.json") }))
            .await;
    assert_eq!(status, 204);
    let new = dir.join("versions/apps/server/9.9.0/xenify-server/xenify-server");
    let mut moved = false;
    // Up to 2 minutes: other tests may be loading the machine.
    for _ in 0..1200 {
        if running(&new) == 1 {
            moved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(moved, "the server didn't update itself");
    let (status, s) = get(&addr, "/v1/admin/hub/status", Some(&admin)).await;
    assert_eq!(status, 200, "{s}");
    assert_eq!(s["self_updating"], true);
    terminate(sup.id().unwrap());
    tokio::time::timeout(Duration::from_secs(40), sup.wait()).await.unwrap().unwrap();
    assert_eq!(running(&new), 0);
}

async fn get_maybe(addr: &str) -> bool {
    tokio::net::TcpStream::connect(addr).await.is_ok()
}

/// Sets the server up with the link it wrote (in address.json).
async fn set_up_with_link(addr: &str, data: &Path) -> String {
    // Written once it listens: a moment after it first answers.
    for _ in 0..100 {
        if data.join("address.json").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let a: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data.join("address.json")).unwrap()).unwrap();
    let token = a["setup"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    set_up_token(addr, &token).await
}

/// SIGTERM, as systemd stops the service.
fn terminate(pid: u32) {
    let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
}
