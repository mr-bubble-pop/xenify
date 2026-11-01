//! Admin › Network: ports change while the server runs.

mod common;
use common::*;
use serde_json::json;
use xenify_server::net::{Fixed, Net};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn ports_change_live() {
    let Some(db) = fresh_db("net").await else { return };
    let (addr, s) = serve(&db).await;
    let token = set_up(&addr, &s).await;
    s.fix_network(Fixed::default());
    let first = free_port();
    s.run_network(Net { http_port: first, https_port: None, announce: false, apps: false }).await.unwrap();
    let at = |p: u16| format!("127.0.0.1:{p}");
    assert_eq!(get(&at(first), "/v1/health", None).await.0, 200);

    let (status, r) = get(&at(first), "/v1/admin/network", Some(&token)).await;
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["net"]["http_port"], first);

    // Moving to a port something else holds is refused; nothing changes.
    let taken = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let busy = taken.local_addr().unwrap().port();
    let (status, _) = post(
        &at(first),
        "/v1/admin/network",
        Some(&token),
        json!({ "http_port": busy, "https_port": null, "announce": false, "apps": false }),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(get(&at(first), "/v1/health", None).await.0, 200);
    drop(taken);

    // A free one: the new port answers, the old one stops.
    let second = free_port();
    let (status, r) = post(
        &at(first),
        "/v1/admin/network",
        Some(&token),
        json!({ "http_port": second, "https_port": null, "announce": false, "apps": false }),
    )
    .await;
    assert_eq!(status, 204, "{r}");
    assert_eq!(get(&at(second), "/v1/health", None).await.0, 200);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(tokio::net::TcpStream::connect(at(first)).await.is_err());
    // Saved for the next start.
    assert_eq!(s.net().await.http_port, second);

    // Another Xenify server on a port isn't shared with.
    let Some(db2) = fresh_db("net2").await else { return };
    let (_, other) = serve(&db2).await;
    other.fix_network(Fixed::default());
    let e =
        other.run_network(Net { http_port: second, https_port: None, announce: false, apps: false }).await.unwrap_err();
    assert!(e.contains("another program"), "{e}");

    // What the command line set can't be changed here.
    s.fix_network(Fixed { listen: Some(format!("0.0.0.0:{second}")), ..Fixed::default() });
    let (status, _) = post(
        &at(second),
        "/v1/admin/network",
        Some(&token),
        json!({ "http_port": free_port(), "https_port": null, "announce": false, "apps": false }),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(get(&at(second), "/v1/health", None).await.0, 200);
}
