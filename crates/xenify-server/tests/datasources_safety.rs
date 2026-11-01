//! Data sources reach only what they're let: files in the folders an admin
//! picks, no local address unless an admin allows it, passwords only for
//! the address they were given for, encryption as set, connection errors
//! that don't say what answered, and values bound, never written into SQL.

mod common;

use common::*;
use serde_json::{Value, json};

fn pg_address() -> String {
    // postgres://user@host:port/db → host:port
    let base = admin_db();
    let after = base.split_once('@').map(|x| x.1).unwrap_or(&base);
    after.split('/').next().unwrap().to_string()
}

async fn id_of(addr: &str, token: &str) -> i64 {
    let (_, me) = get(addr, "/v1/me", Some(token)).await;
    me["id"].as_i64().unwrap_or_else(|| panic!("{me}"))
}

async fn listed(addr: &str, token: &str, id: i64) -> Value {
    let (_, list) = get(addr, "/v1/admin/data", Some(token)).await;
    list.as_array().unwrap().iter().find(|x| x["id"] == id).cloned().unwrap_or(Value::Null)
}

#[tokio::test]
async fn data_sources_stay_where_they_are_put() {
    let Some(db) = fresh_db("datasafe").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    // Dana may set up database connections, but isn't an admin.
    let dana = add_person(&addr, &admin, "dana", false).await;
    let dana_id = id_of(&addr, &dana).await;
    let (status, r) = post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "Data keeper", "perms": ["admin.data"], "members": [{ "user": dana_id }] }),
    )
    .await;
    assert_eq!(status, 200, "{r}");

    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let (inside, outside) =
        (tmp.join(format!("datasafe_in_{}", std::process::id())), tmp.join(format!("datasafe_out_{}", std::process::id())));
    for d in [&inside, &outside] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).unwrap();
        let c = rusqlite::Connection::open(d.join("stock.sqlite")).unwrap();
        c.execute_batch("CREATE TABLE stock (item TEXT, qty INTEGER); INSERT INTO stock VALUES ('Tea', 40);").unwrap();
    }
    let path = |d: &std::path::Path, f: &str| d.join(f).to_string_lossy().into_owned();

    // The folders: Dana sees them; only the admin picks them.
    let (status, f) = get(&addr, "/v1/admin/data/folders", Some(&dana)).await;
    assert_eq!((status, &f["can_change"]), (200, &json!(false)), "{f}");
    let folders = json!({ "folders": [inside.to_string_lossy()] });
    assert_eq!(call(&addr, "PUT", "/v1/admin/data/folders", Some(&dana), Some(folders.clone())).await.0, 403);
    assert_eq!(call(&addr, "PUT", "/v1/admin/data/folders", Some(&admin), Some(folders)).await.0, 200);

    // A file outside them is refused, also through a link.
    let file = |name: &str, at: String| json!({ "name": name, "kind": "sqlite", "address": at });
    let (status, e) = post(&addr, "/v1/admin/data", Some(&dana), file("Out", path(&outside, "stock.sqlite"))).await;
    assert_eq!(status, 400, "{e}");
    assert!(e["error"].as_str().unwrap().contains("isn’t in a folder"), "{e}");
    assert_eq!(post(&addr, "/v1/admin/data", Some(&dana), file("Up", format!("{}/../x.sqlite", inside.display()))).await.0, 400);
    std::os::unix::fs::symlink(outside.join("stock.sqlite"), inside.join("link.sqlite")).unwrap();
    let (status, made) = post(&addr, "/v1/admin/data", Some(&dana), file("Link", path(&inside, "link.sqlite"))).await;
    assert_eq!(status, 200, "{made}");
    let link = made["id"].as_i64().unwrap();
    let (status, e) = post(&addr, &format!("/v1/data/{link}/query"), Some(&dana), json!({ "sql": "SELECT * FROM stock" })).await;
    assert_ne!(status, 200, "{e}");
    // Inside: fine, but no other database files from a query.
    let (_, made) = post(&addr, "/v1/admin/data", Some(&dana), file("Stock", path(&inside, "stock.sqlite"))).await;
    let stock = made["id"].as_i64().unwrap();
    let (status, a) = post(&addr, &format!("/v1/data/{stock}/query"), Some(&dana), json!({ "sql": "SELECT item FROM stock" })).await;
    assert_eq!((status, &a["rows"]), (200, &json!([["Tea"]])), "{a}");
    let attach = format!("ATTACH DATABASE '{}' AS other", path(&outside, "stock.sqlite"));
    assert_ne!(post(&addr, &format!("/v1/data/{stock}/query"), Some(&dana), json!({ "sql": attach })).await.0, 200);

    // Local addresses (and the server's own database): Dana can't, the admin can.
    let pg = |more: Value| {
        let mut v = json!({ "name": "Here", "kind": "postgres", "address": pg_address(), "database": "postgres", "tls": "off",
            "read_user": "postgres", "read_password": "pw" });
        v.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        v
    };
    let (status, e) = post(&addr, "/v1/admin/data", Some(&dana), pg(json!({}))).await;
    assert_eq!(status, 400, "{e}");
    assert!(e["error"].as_str().unwrap().contains("Only an admin"), "{e}");
    assert_eq!(post(&addr, "/v1/admin/data", Some(&dana), pg(json!({ "allow_local": true }))).await.0, 403);
    let (status, _) = post(&addr, "/v1/admin/data", Some(&dana), pg(json!({ "address": "169.254.169.254:5432" }))).await;
    assert_eq!(status, 400, "the cloud metadata address");
    let (_, t) = post(&addr, "/v1/admin/data/test", Some(&dana), pg(json!({}))).await;
    assert_eq!(t["ok"], false, "{t}");
    let (status, made) = post(&addr, "/v1/admin/data", Some(&admin), pg(json!({ "allow_local": true }))).await;
    assert_eq!(status, 200, "{made}");
    let here = made["id"].as_i64().unwrap();
    let (_, t) = post(&addr, &format!("/v1/admin/data/{here}/test"), Some(&dana), json!({})).await;
    assert_eq!(t["ok"], true, "{t}");

    // Values are bound, never SQL.
    let tricky = "x'); DROP TABLE nothing; -- \\' $$ E'";
    let (status, a) = post(
        &addr,
        &format!("/v1/data/{here}/query"),
        Some(&dana),
        json!({ "sql": "SELECT :a::text AS a, :n + 1 AS n, :a = 'x' AS same, ':a' AS words", "params": { "a": tricky, "n": 41 } }),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    assert_eq!(a["rows"][0][0], tricky);
    assert_eq!(a["rows"][0][1].as_f64(), Some(42.0));
    assert_eq!((&a["rows"][0][2], &a["rows"][0][3]), (&json!(false), &json!(":a")));
    let (_, a) = post(&addr, &format!("/v1/data/{here}/query"), Some(&dana), json!({ "sql": "SELECT :x IS NULL", "params": { "x": null } })).await;
    assert_eq!(a["rows"], json!([[true]]), "{a}");

    // Dana keeps it as it is: Allow local addresses and the password stay.
    assert_eq!(post(&addr, "/v1/admin/data", Some(&dana), json!({ "id": here, "name": "Here now", "kind": "postgres", "address": pg_address(), "database": "postgres", "read_user": "postgres" })).await.0, 200);
    let l = listed(&addr, &admin, here).await;
    assert_eq!((&l["allow_local"], &l["read_password_set"], &l["tls"]), (&json!(true), &json!(true), &json!("off")), "{l}");
    // A new address: the kept password isn't sent there, nor kept.
    let moved = json!({ "id": here, "name": "Here now", "kind": "postgres", "address": pg_address().replace("127.0.0.1", "localhost"),
        "database": "postgres", "read_user": "postgres", "allow_local": true, "tls": "off" });
    let (_, t) = post(&addr, "/v1/admin/data/test", Some(&admin), moved.clone()).await;
    assert_eq!(t["message"], "Type the password again for the new address.", "{t}");
    assert_eq!(post(&addr, "/v1/admin/data", Some(&admin), moved).await.0, 200);
    assert_eq!(listed(&addr, &admin, here).await["read_password_set"], false);
    // Dana moving it loses Allow local addresses.
    let (status, _) = post(&addr, "/v1/admin/data", Some(&dana), json!({ "id": here, "name": "Here now", "kind": "postgres", "address": pg_address(), "database": "postgres" })).await;
    assert_eq!(status, 400);

    // Encryption: required by default; the test PostgreSQL has none.
    let (_, made) = post(&addr, "/v1/admin/data", Some(&admin), json!({ "name": "Strict", "kind": "postgres", "address": pg_address(), "database": "postgres", "read_user": "postgres", "allow_local": true })).await;
    let strict = made["id"].as_i64().unwrap();
    assert_eq!(listed(&addr, &admin, strict).await["tls"], "require");
    let (_, t) = post(&addr, &format!("/v1/admin/data/{strict}/test"), Some(&admin), json!({})).await;
    assert!(t["message"].as_str().unwrap_or("").contains("doesn’t offer encryption"), "{t}");
    let (_, t) = post(&addr, "/v1/admin/data/test", Some(&admin), json!({ "id": strict, "kind": "postgres", "address": pg_address(), "database": "postgres", "read_user": "postgres", "tls": "prefer" })).await;
    assert_eq!(t["ok"], true, "Prefer goes on unencrypted: {t}");
    let (status, _) = post(&addr, "/v1/admin/data", Some(&admin), json!({ "id": strict, "name": "Strict", "kind": "postgres", "address": pg_address(), "ca_cert": "not a certificate" })).await;
    assert_eq!(status, 400);

    // Errors say what kind of trouble, not what answered.
    let (_, t) = post(&addr, "/v1/admin/data/test", Some(&admin), json!({ "kind": "postgres", "address": "127.0.0.1:1", "database": "x", "tls": "off", "allow_local": true })).await;
    assert_eq!(t["message"], "Can’t connect: the connection was refused. Check the address and port.", "{t}");
    let (_, t) = post(&addr, "/v1/admin/data/test", Some(&admin), json!({ "kind": "mysql", "address": pg_address(), "database": "x", "tls": "off", "allow_local": true })).await;
    let m = t["message"].as_str().unwrap_or("");
    assert!(m.starts_with("Can’t connect"), "{t}");

    let _ = std::fs::remove_dir_all(&inside);
    let _ = std::fs::remove_dir_all(&outside);
}
