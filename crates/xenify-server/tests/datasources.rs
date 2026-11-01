//! Calc's data sources: a PostgreSQL database (with a reading and a
//! changing user), a SQLite file and a folder of CSV files, used by an
//! admin and by someone in the source's group; who may set sources up and
//! write queries on them; and tickets, which let anyone with a file run the
//! queries in it.

mod common;

use common::*;
use serde_json::{Value, json};

/// A shop database on the test PostgreSQL, with a user that can only read
/// and one that can change prices. Returns (database name, reader, changer).
async fn shop() -> Option<(String, String, String)> {
    let (c, conn) = tokio_postgres::connect(&admin_db(), tokio_postgres::NoTls).await.ok()?;
    tokio::spawn(conn);
    let pid = std::process::id();
    let db = format!("shop_{pid}");
    let (reader, changer) = (format!("shop_read_{pid}"), format!("shop_change_{pid}"));
    let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {db}")).await;
    for r in [&reader, &changer] {
        let _ = c.batch_execute(&format!("DROP ROLE IF EXISTS {r}")).await;
        c.batch_execute(&format!("CREATE ROLE {r} LOGIN PASSWORD 'pw-{r}'")).await.ok()?;
    }
    c.batch_execute(&format!("CREATE DATABASE {db}")).await.ok()?;
    let url = format!("{}/{db}", admin_db().rsplit_once('/').unwrap().0);
    let (s, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.ok()?;
    tokio::spawn(conn);
    s.batch_execute(&format!(
        "CREATE TABLE products (id INT PRIMARY KEY, name TEXT NOT NULL, price NUMERIC(10,2), added DATE);
         COMMENT ON TABLE products IS 'What we sell';
         COMMENT ON COLUMN products.price IS 'Before tax, in pounds';
         INSERT INTO products VALUES (1, 'Tea', 3.50, '2026-01-04'), (2, 'Coffee', 4.25, '2026-02-10'), (3, 'O''Brien''s jam', 2.80, NULL);
         GRANT CONNECT ON DATABASE {db} TO {reader}, {changer};
         GRANT SELECT ON products TO {reader};
         GRANT SELECT, UPDATE (price, name) ON products TO {changer};
         GRANT INSERT ON products TO {changer};
         CREATE TABLE notes (id SERIAL PRIMARY KEY, body TEXT, made DATE DEFAULT '2026-10-25');
         GRANT SELECT ON notes TO {reader};
         GRANT SELECT, INSERT, UPDATE, DELETE ON notes TO {changer};
         GRANT USAGE ON SEQUENCE notes_id_seq TO {changer};"
    ))
    .await
    .ok()?;
    Some((db, reader, changer))
}

async fn drop_shop(db: &str, reader: &str, changer: &str) {
    let Ok((c, conn)) = tokio_postgres::connect(&admin_db(), tokio_postgres::NoTls).await else { return };
    tokio::spawn(conn);
    let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)")).await;
    for r in [reader, changer] {
        let _ = c.batch_execute(&format!("DROP ROLE IF EXISTS {r}")).await;
    }
}

fn pg_address() -> String {
    // postgres://user@host:port/db → host:port
    let base = admin_db();
    let after = base.split_once('@').map(|x| x.1).unwrap_or(&base);
    after.split('/').next().unwrap().to_string()
}

#[tokio::test]
async fn data_sources_read_and_change() {
    let Some(db) = fresh_db("data").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let Some((shop_db, reader, changer)) = shop().await else {
        eprintln!("couldn't make the shop database: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;

    // Two people: Priya in Sales, Sam in Design.
    let priya = add_person(&addr, &admin, "priya", false).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    let (_, g) =
        post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Sales", "members": [id_of(&addr, &priya).await] })).await;
    let sales = g["id"].as_i64().unwrap();
    post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Design", "members": [id_of(&addr, &sam).await] })).await;

    // The admin adds the shop, readable by Sales, prices editable.
    let (status, made) = post(
        &addr,
        "/v1/admin/data",
        Some(&admin),
        json!({
            "name": "Shop", "kind": "postgres", "address": pg_address(), "database": shop_db,
            // The test PostgreSQL is on this computer, unencrypted.
            "tls": "off", "allow_local": true,
            "read_user": reader, "read_password": format!("pw-{reader}"),
            "write_user": changer, "write_password": format!("pw-{changer}"),
            "time_limit": 5, "row_limit": 2,
            "editable": [{ "table": "products", "key": ["id"], "locked": ["added"] }, { "table": "notes", "key": ["id"], "locked": ["made"], "delete": true }],
            "notes": { "products.price": "Before tax" },
        }),
    )
    .await;
    assert_eq!(status, 200, "{made}");
    let shop_id = made["id"].as_i64().unwrap();

    // Passwords never come back.
    let (_, list) = get(&addr, "/v1/admin/data", Some(&admin)).await;
    let s0 = &list[0];
    assert_eq!(s0["read_password_set"], true);
    assert!(s0.get("read_password").is_none() && s0.get("write_password").is_none(), "{s0}");
    assert!(!list.to_string().contains("pw-"), "{list}");

    let (_, tested) = post(&addr, &format!("/v1/admin/data/{shop_id}/test"), Some(&admin), json!({})).await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(tested["changes"]["ok"], true, "{tested}");
    assert_eq!(tested["changes"]["tables"], json!(["notes", "products"]), "{tested}");
    assert_eq!(tested["reader_can_change"], json!([]), "{tested}");
    // Before saving (the test PostgreSQL trusts local sign-ins, so passwords aren't checked here).
    let (_, t) = post(&addr, "/v1/admin/data/test", Some(&admin), json!({ "kind": "postgres", "address": pg_address(), "database": shop_db, "tls": "off", "allow_local": true, "read_user": changer, "read_password": format!("pw-{changer}") })).await;
    assert_eq!(
        t["reader_can_change"],
        json!(["notes", "products"]),
        "a reading user that can change things is pointed out: {t}"
    );

    // A saved query with a value from the sheet.
    let (status, q) = post(
        &addr,
        &format!("/v1/admin/data/{shop_id}/queries"),
        Some(&admin),
        json!({ "name": "Dearer than", "sql": "SELECT name, price FROM products WHERE price > :least ORDER BY price DESC;" }),
    )
    .await;
    assert_eq!(status, 200, "{q}");
    let query_id = q["id"].as_i64().unwrap();

    // Only those who write queries on it see it, or run anything on it
    // without a ticket.
    for who in [&sam, &priya] {
        let (_, theirs) = get(&addr, "/v1/data", Some(who)).await;
        assert_eq!(theirs, json!([]));
        let (status, _) = post(
            &addr,
            &format!("/v1/data/{shop_id}/query"),
            Some(who),
            json!({ "query": query_id, "params": { "least": 1 } }),
        )
        .await;
        assert_eq!(status, 403);
    }
    // The admin puts the saved query in a file: the answer has the ticket
    // the file keeps, so Priya, who has the file, can run it again.
    let (status, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "query": query_id, "params": { "least": 3 } }),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    let ticket = a["ticket"].as_str().unwrap().to_string();
    let (status, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&priya),
        json!({ "query": query_id, "params": { "least": 3 }, "ticket": ticket }),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    assert!(a.get("ticket").is_none(), "only writers get tickets: {a}");
    assert_eq!(a["columns"], json!([["name", "text"], ["price", "number"]]));
    assert_eq!(a["decimals"], json!([null, 2]), "numeric(10,2): two decimals");
    assert_eq!(a["rows"], json!([["Coffee", 4.25], ["Tea", 3.5]]));
    assert_eq!(a["truncated"], false);

    // The ticket fits nothing else: not her own SQL, nor another source.
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&priya),
        json!({ "sql": "SELECT 1", "ticket": ticket }),
    )
    .await;
    assert_eq!(status, 403);
    // The admin's own SQL in a file: its ticket runs exactly that SQL.
    let sql = "SELECT name FROM products WHERE price > :least ORDER BY name";
    let (_, a) =
        post(&addr, &format!("/v1/data/{shop_id}/query"), Some(&admin), json!({ "sql": sql, "params": { "least": 4 } }))
            .await;
    let sql_ticket = a["ticket"].as_str().unwrap().to_string();
    let (status, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&priya),
        json!({ "sql": sql, "params": { "least": 1 }, "ticket": sql_ticket }),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    assert_eq!(a["rows"][0], json!(["Coffee"]), "values from her sheet are hers to change: {a}");
    let (status, e) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&priya),
        json!({ "sql": format!("{sql} -- mine now"), "ticket": sql_ticket }),
    )
    .await;
    assert_eq!((status, e["code"].as_str()), (403, Some("changed")), "{e}");
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&priya),
        json!({ "query": query_id, "ticket": sql_ticket }),
    )
    .await;
    assert_eq!(status, 403);

    // The admin's is; the row limit (2) cuts it short; it can't change anything.
    let (_, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "sql": "SELECT id, name, added FROM products ORDER BY id" }),
    )
    .await;
    assert_eq!(a["rows"].as_array().unwrap().len(), 2, "{a}");
    assert_eq!(a["truncated"], true);
    assert_eq!(a["rows"][0][2], "2026-01-04");
    assert_eq!(a["columns"][2], json!(["added", "date"]));
    for bad in [
        "SELECT 1; DELETE FROM products",
        "DELETE FROM products",
        "UPDATE products SET price = 0",
        "WITH x AS (DELETE FROM products RETURNING *) SELECT * FROM x",
        "SELECT pg_sleep(30)",
    ] {
        let (status, e) = post(&addr, &format!("/v1/data/{shop_id}/query"), Some(&admin), json!({ "sql": bad })).await;
        assert_ne!(status, 200, "{bad}: {e}");
    }
    // A value from a sheet is a value, never SQL.
    let (status, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "sql": "SELECT name FROM products WHERE name = :n", "params": { "n": "O'Brien's jam" } }),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    assert_eq!(a["rows"], json!([["O'Brien's jam"]]));
    let (_, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "sql": "SELECT count(*) AS n FROM products WHERE name = :n", "params": { "n": "x' OR '1'='1" } }),
    )
    .await;
    assert_eq!(a["rows"], json!([[0.0]]), "{a}");
    // A note at the end of the query is only a note.
    let (status, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "sql": "SELECT name FROM products WHERE name = :n -- the jam", "params": { "n": "O'Brien's jam" } }),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    assert_eq!(a["rows"], json!([["O'Brien's jam"]]), "{a}");

    // Tables, with the database's own descriptions (for writers).
    let (status, _) = get(&addr, &format!("/v1/data/{shop_id}/tables"), Some(&priya)).await;
    assert_eq!(status, 403);
    let (_, tables) = get(&addr, &format!("/v1/data/{shop_id}/tables"), Some(&admin)).await;
    let products = tables.as_array().unwrap().iter().find(|t| t["table"] == "products").unwrap();
    assert_eq!(products["note"], "What we sell");
    let price = products["columns"].as_array().unwrap().iter().find(|c| c["name"] == "price").unwrap();
    assert_eq!(price["note"], "Before tax, in pounds");

    // Table editing, in a file the admin added the table to: a price
    // changed; a stale one refused; a locked column refused.
    let (status, _) =
        post(&addr, &format!("/v1/data/{shop_id}/table"), Some(&priya), json!({ "table": "products" })).await;
    assert_eq!(status, 403, "no ticket");
    // The file: in Priya's own files (she may edit it); another in the
    // admin's (she can't).
    let file_in = |who: String| {
        let addr = addr.clone();
        async move {
            let (_, spaces) = get(&addr, "/v1/spaces", Some(&who)).await;
            let mine = spaces.as_array().unwrap().iter().find(|s| s["kind"] == "personal").unwrap()["id"].as_i64().unwrap();
            let meta = json!({ "name": "Shop", "app": "calc", "ext": "xcalc" }).to_string();
            let (status, made) =
                post(&addr, "/v1/nodes", Some(&who), json!({ "space": mine, "kind": "file", "meta": meta })).await;
            assert_eq!(status, 200, "{made}");
            made["id"].as_i64().unwrap()
        }
    };
    let pnode = file_in(priya.clone()).await;
    let anode = file_in(admin.clone()).await;
    let table_ticket = |t: &'static str| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move {
            let (_, r) =
                post(&addr, &format!("/v1/data/{shop_id}/table"), Some(&admin), json!({ "table": t, "node": pnode })).await;
            r["ticket"].as_str().unwrap().to_string()
        }
    };
    let products_ticket = table_ticket("products").await;
    let notes_ticket = table_ticket("notes").await;
    let (status, t) = post(
        &addr,
        &format!("/v1/data/{shop_id}/table"),
        Some(&priya),
        json!({ "table": "products", "ticket": products_ticket, "node": pnode }),
    )
    .await;
    assert_eq!(status, 200, "{t}");
    assert_eq!(t["notes"]["key"], json!(["id"]));
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/table"),
        Some(&priya),
        json!({ "table": "notes", "ticket": products_ticket, "node": pnode }),
    )
    .await;
    assert_eq!(status, 403, "a ticket is for one table");
    let price = json!([{ "table": "products", "key": { "id": 1 }, "column": "price", "old": 3.5, "new": 3.75 }]);
    let (status, e) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "changes": price }),
    )
    .await;
    assert_eq!(status, 403, "rows change only in a file: {e}");
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "node": anode, "changes": price }),
    )
    .await;
    assert_eq!(status, 403, "the ticket is for her file, not another");
    let (_, r) =
        post(&addr, &format!("/v1/data/{shop_id}/table"), Some(&admin), json!({ "table": "products", "node": anode })).await;
    let admins_ticket = r["ticket"].as_str().unwrap().to_string();
    let (status, e) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": admins_ticket, "node": anode, "changes": price }),
    )
    .await;
    assert_eq!(status, 403, "she can't edit the admin's file: {e}");
    assert_eq!(e["code"], "cant_edit", "{e}");
    let (status, done) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "node": pnode, "changes": [
            { "table": "products", "key": { "id": 1 }, "column": "price", "old": 3.5, "new": 3.75 },
            { "table": "products", "key": { "id": 2 }, "column": "price", "old": 9.99, "new": 1 },
            { "table": "products", "key": { "id": 99 }, "column": "price", "old": 1, "new": 2 },
        ] }),
    )
    .await;
    assert_eq!(status, 200, "{done}");
    let outcomes: Vec<Value> = done["outcomes"].as_array().unwrap().iter().map(|o| o["outcome"].clone()).collect();
    assert_eq!(outcomes, vec![json!("changed"), json!("conflict"), json!("missing")]);
    assert_eq!(done["outcomes"][1]["now"], "4.25");
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "node": pnode, "changes": [{ "table": "products", "key": { "id": 1 }, "column": "added", "old": null, "new": "2026-01-01" }] }),
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "node": pnode, "changes": [{ "table": "products", "key": { "id": 1 }, "column": "id", "old": 1, "new": 5 }] }),
    )
    .await;
    assert_eq!(status, 403);
    let (_, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "sql": "SELECT price FROM products WHERE id = 1" }),
    )
    .await;
    assert_eq!(a["rows"], json!([[3.75]]));

    // Reading a table: only some rows, in an order, and how many in all.
    let (_, t) = post(
        &addr,
        &format!("/v1/data/{shop_id}/table"),
        Some(&priya),
        json!({ "table": "products", "order": { "column": "price", "desc": true }, "ticket": products_ticket, "node": pnode }),
    )
    .await;
    assert_eq!(t["rows"].as_array().unwrap().len(), 2, "{t}");
    assert_eq!(t["truncated"], true);
    assert_eq!(t["notes"]["total"], 3.0);
    assert_eq!(t["rows"][0][1], "Coffee");
    assert_eq!(t["notes"]["add"], true);
    assert_eq!(t["notes"]["delete"], false);
    let (_, t) = post(
        &addr,
        &format!("/v1/data/{shop_id}/table"),
        Some(&priya),
        json!({ "table": "products", "only": [{ "column": "name", "value": "Tea" }], "ticket": products_ticket, "node": pnode }),
    )
    .await;
    assert_eq!(t["rows"].as_array().unwrap().len(), 1, "{t}");

    // New rows say the key they got; rows go only where the admin allows it.
    let (status, done) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": notes_ticket, "node": pnode, "changes": [
            { "kind": "insert", "table": "notes", "values": { "body": "Order more tea" } },
            { "kind": "insert", "table": "notes", "values": {} },
        ] }),
    )
    .await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["outcomes"][0], json!({ "outcome": "added", "key": { "id": "1" } }));
    assert_eq!(done["outcomes"][1]["key"]["id"], "2");
    let (status, e) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": notes_ticket, "node": pnode, "changes": [
            { "kind": "insert", "table": "notes", "values": { "body": "x" } },
            { "kind": "insert", "table": "products", "values": { "id": 4, "name": "Cocoa", "price": 5 } },
        ] }),
    )
    .await;
    assert_eq!(status, 400, "one table at a time: {e}");
    let (status, done) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "node": pnode, "changes": [{ "kind": "insert", "table": "products", "values": { "id": 4, "name": "Cocoa", "price": 5 } }] }),
    )
    .await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["outcomes"][0]["key"]["id"], "4");
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": notes_ticket, "node": pnode, "changes": [{ "kind": "insert", "table": "notes", "values": { "made": "2020-01-01" } }] }),
    )
    .await;
    assert_eq!(status, 403, "a locked column can't be set on a new row");
    let (status, _) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": products_ticket, "node": pnode, "changes": [{ "kind": "delete", "table": "products", "key": { "id": 4 } }] }),
    )
    .await;
    assert_eq!(status, 403, "products: no deleting");
    let (status, done) = post(
        &addr,
        &format!("/v1/data/{shop_id}/changes"),
        Some(&priya),
        json!({ "ticket": notes_ticket, "node": pnode, "changes": [
            { "kind": "delete", "table": "notes", "key": { "id": 2 } },
            { "kind": "delete", "table": "notes", "key": { "id": 2 } },
            { "table": "notes", "key": { "id": 1 }, "column": "body", "old": "Order more tea", "new": "Order more tea and jam" },
        ] }),
    )
    .await;
    assert_eq!(status, 200, "{done}");
    let outcomes: Vec<Value> = done["outcomes"].as_array().unwrap().iter().map(|o| o["outcome"].clone()).collect();
    assert_eq!(outcomes, vec![json!("deleted"), json!("missing"), json!("changed")]);
    let (_, a) = post(
        &addr,
        &format!("/v1/data/{shop_id}/query"),
        Some(&admin),
        json!({ "sql": "SELECT id, body FROM notes ORDER BY id" }),
    )
    .await;
    assert_eq!(a["rows"], json!([[1.0, "Order more tea and jam"]]), "{a}");

    // The log says who did what.
    let (_, log) = get(&addr, &format!("/v1/admin/data/{shop_id}/log"), Some(&admin)).await;
    let log = log.as_array().unwrap();
    assert!(
        log.iter()
            .any(|l| l["who"] == "priya" && l["what"] == "change" && l["detail"]["outcome"]["outcome"] == "changed")
    );
    assert!(
        log.iter()
            .any(|l| l["who"] == "priya" && l["what"] == "read" && l["detail"]["which"]["query"] == "Dearer than")
    );

    // A SQLite file and a folder of CSV files.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("datasources_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("csv")).unwrap();
    let file = dir.join("stock.sqlite");
    {
        let c = rusqlite::Connection::open(&file).unwrap();
        c.execute_batch("CREATE TABLE stock (item TEXT, qty INTEGER, cost REAL); INSERT INTO stock VALUES ('Tea', 40, 1.2), ('Coffee', 12, 2.5);").unwrap();
    }
    std::fs::write(dir.join("csv/orders.csv"), "Region,Units,Code\nNorth,10,007\nSouth,4,012\n\"West, far\",7,100\n")
        .unwrap();
    allow_folders(&addr, &admin, &[&dir.to_string_lossy()]).await;
    let (_, made) = post(&addr, "/v1/admin/data", Some(&admin), json!({ "name": "Stock", "kind": "sqlite", "address": file.to_string_lossy(), "writers": [{ "group": sales }] })).await;
    let stock = made["id"].as_i64().unwrap();
    let (_, a) = post(
        &addr,
        &format!("/v1/data/{stock}/query"),
        Some(&priya),
        json!({ "sql": "SELECT item, qty * cost AS worth FROM stock WHERE qty > :q", "params": { "q": 20 } }),
    )
    .await;
    assert_eq!(a["rows"], json!([["Tea", 48.0]]), "{a}");
    assert_eq!(a["columns"][1], json!(["worth", "number"]));
    let (status, _) =
        post(&addr, &format!("/v1/data/{stock}/query"), Some(&priya), json!({ "sql": "DELETE FROM stock" })).await;
    assert_ne!(status, 200);
    let (status, _) = post(&addr, &format!("/v1/data/{stock}/changes"), Some(&priya), json!([{ "table": "stock", "key": {}, "column": "qty", "old": 40, "new": 41 }])).await;
    assert_eq!(status, 403, "no changing user: read only");

    let (_, made) = post(&addr, "/v1/admin/data", Some(&admin), json!({ "name": "Orders", "kind": "csv", "address": dir.join("csv").to_string_lossy(), "writers": [{ "group": sales }] })).await;
    let orders = made["id"].as_i64().unwrap();
    let (_, a) = post(
        &addr,
        &format!("/v1/data/{orders}/query"),
        Some(&priya),
        json!({ "sql": "SELECT Region, Units, Code FROM orders ORDER BY Units DESC" }),
    )
    .await;
    assert_eq!(a["rows"], json!([["North", 10, "007"], ["West, far", 7, 100], ["South", 4, "012"]]), "{a}");
    let (_, tables) = get(&addr, &format!("/v1/data/{orders}/tables"), Some(&priya)).await;
    assert_eq!(tables[0]["table"], "orders");

    // Deleting the source takes its queries with it.
    let (status, _) = post(
        &addr,
        &format!("/v1/admin/data/{shop_id}/queries"),
        Some(&priya),
        json!({ "name": "x", "sql": "SELECT 1" }),
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = call(&addr, "DELETE", &format!("/v1/admin/data/{shop_id}"), Some(&admin), None).await;
    assert_eq!(status, 200);
    let (_, hers) = get(&addr, "/v1/data", Some(&priya)).await;
    assert_eq!(hers.as_array().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
    drop_shop(&shop_db, &reader, &changer).await;
}

/// For looking at Admin › Data sources: a server with a few sources that
/// stays up. `XENIFY_SHOW=<file> cargo test --test datasources show -- --ignored`
/// writes "address token" to the file.
#[tokio::test]
#[ignore]
async fn show() {
    let Some(db) = fresh_db("datashow").await else { return };
    let Some((shop_db, reader, changer)) = shop().await else { return };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Shop team" })).await;
    let team = g["id"].as_i64().unwrap();
    post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Buyers" })).await;
    let (_, made) = post(&addr, "/v1/admin/data", Some(&admin), json!({
        "name": "Shop database", "kind": "postgres", "address": pg_address(), "database": shop_db, "tls": "off", "allow_local": true,
        "read_user": reader, "read_password": format!("pw-{reader}"), "write_user": changer, "write_password": format!("pw-{changer}"),
        "writers": [{ "group": team }], "editable": [{ "table": "products", "key": ["id"], "locked": ["added"] }, { "table": "notes", "key": ["id"], "locked": ["made"], "delete": true }],
    })).await;
    let id = made["id"].as_i64().unwrap();
    post(
        &addr,
        &format!("/v1/admin/data/{id}/queries"),
        Some(&admin),
        json!({ "name": "Dearer than", "sql": "SELECT name, price FROM products WHERE price > :least" }),
    )
    .await;
    post(&addr, &format!("/v1/data/{id}/query"), Some(&admin), json!({ "sql": "SELECT * FROM products" })).await;
    allow_folders(&addr, &admin, &["/tmp"]).await;
    post(
        &addr,
        "/v1/admin/data",
        Some(&admin),
        json!({ "name": "Supplier prices", "kind": "csv", "address": "/tmp" }),
    )
    .await;
    std::fs::write(std::env::var("XENIFY_SHOW").unwrap(), format!("{addr} {admin}")).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    drop_shop(&shop_db, &reader, &changer).await;
}

/// Lets file sources be in `folders` (an admin's choice).
async fn allow_folders(addr: &str, admin: &str, folders: &[&str]) {
    let (status, v) = call(addr, "PUT", "/v1/admin/data/folders", Some(admin), Some(json!({ "folders": folders }))).await;
    assert_eq!(status, 200, "{v}");
}

/// Signed-in person's id.
async fn id_of(addr: &str, token: &str) -> i64 {
    let (_, me) = get(addr, "/v1/me", Some(token)).await;
    me["id"].as_i64().unwrap_or_else(|| panic!("{me}"))
}

/// Dana may set up database connections (a role); Wes writes queries on
/// Stock, and Uma through a group; Nia only has a file with a query in it.
/// Sources from before keep their writers.
#[tokio::test]
async fn who_sets_up_sources_and_writes_queries() {
    let Some(db) = fresh_db("datashare").await else {
        eprintln!("no PostgreSQL to test with: skipped");
        return;
    };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let dana = add_person(&addr, &admin, "dana", false).await;
    let wes = add_person(&addr, &admin, "wes", false).await;
    let uma = add_person(&addr, &admin, "uma", false).await;
    let nia = add_person(&addr, &admin, "nia", false).await;
    let (dana_id, wes_id, uma_id) = (id_of(&addr, &dana).await, id_of(&addr, &wes).await, id_of(&addr, &uma).await);
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&admin), json!({ "name": "Analysts", "members": [uma_id] })).await;
    let analysts = g["id"].as_i64().unwrap();
    let (status, r) = post(
        &addr,
        "/v1/admin/roles",
        Some(&admin),
        json!({ "name": "Data keeper", "perms": ["admin.data"], "members": [{ "user": dana_id }] }),
    )
    .await;
    assert_eq!(status, 200, "{r}");

    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("datashare_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("stock.sqlite");
    {
        let c = rusqlite::Connection::open(&file).unwrap();
        c.execute_batch("CREATE TABLE stock (item TEXT, qty INTEGER); INSERT INTO stock VALUES ('Tea', 40), ('Coffee', 12);").unwrap();
    }
    let stock_src = |writers: Value| json!({ "name": "Stock", "kind": "sqlite", "address": file.to_string_lossy(), "writers": writers });
    allow_folders(&addr, &admin, &[&dir.to_string_lossy(), "/tmp"]).await;

    // Only people who may set up connections add them (and change any).
    let (status, _) = post(&addr, "/v1/admin/data", Some(&wes), stock_src(json!([]))).await;
    assert_eq!(status, 403);
    let (status, made) =
        post(&addr, "/v1/admin/data", Some(&dana), stock_src(json!([{ "user": wes_id }, { "group": analysts }]))).await;
    assert_eq!(status, 200, "{made}");
    let stock = made["id"].as_i64().unwrap();
    let (_, hers) = get(&addr, "/v1/admin/data", Some(&dana)).await;
    let names: Vec<&str> = hers[0]["writers"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Analysts", "wes"], "{hers}");
    assert_eq!(get(&addr, "/v1/admin/data", Some(&wes)).await.1, json!([]), "Wes doesn't set sources up");
    let (status, made) = post(&addr, "/v1/admin/data", Some(&admin), json!({ "name": "Admin's", "kind": "csv", "address": "/tmp" })).await;
    assert_eq!(status, 200, "{made}");
    let (_, hers) = get(&addr, "/v1/admin/data", Some(&dana)).await;
    assert_eq!(hers.as_array().unwrap().len(), 2, "Dana sees every source, the admin's too");
    let (_, mine) = get(&addr, "/v1/data", Some(&dana)).await;
    assert_eq!(mine.as_array().unwrap().len(), 2, "and writes queries on each");
    assert_eq!(mine[0]["level"], "manage");

    // Nia doesn't write queries: nothing listed, nothing run on her own.
    assert_eq!(get(&addr, "/v1/data", Some(&nia)).await.1, json!([]));
    let (status, e) = post(&addr, &format!("/v1/data/{stock}/query"), Some(&nia), json!({ "sql": "SELECT 1" })).await;
    assert_eq!((status, e["code"].as_str()), (403, Some("no_ticket")), "{e}");

    // Wes and Uma write their own SQL, save it for others and pick saved ones.
    for who in [&wes, &uma] {
        let (_, theirs) = get(&addr, "/v1/data", Some(who)).await;
        assert_eq!((theirs[0]["level"].as_str(), theirs[0]["own_queries"].as_bool()), (Some("write"), Some(true)));
    }
    let (status, a) =
        post(&addr, &format!("/v1/data/{stock}/query"), Some(&wes), json!({ "sql": "SELECT item FROM stock ORDER BY qty" })).await;
    assert_eq!((status, &a["rows"]), (200, &json!([["Coffee"], ["Tea"]])));
    let (status, q) = post(&addr, &format!("/v1/data/{stock}/queries"), Some(&wes), json!({ "name": "Low stock", "sql": "SELECT item FROM stock WHERE qty < :under" })).await;
    assert_eq!(status, 200, "{q}");
    let low = q["id"].as_i64().unwrap();
    let (_, hers) = get(&addr, "/v1/data", Some(&uma)).await;
    assert_eq!(hers[0]["queries"][0]["name"], "Low stock");
    let (status, a) = post(&addr, &format!("/v1/data/{stock}/query"), Some(&uma), json!({ "query": low, "params": { "under": 20 } })).await;
    assert_eq!((status, &a["rows"]), (200, &json!([["Coffee"]])));
    assert_eq!(get(&addr, &format!("/v1/admin/data/{stock}/log"), Some(&wes)).await.0, 403, "the log is for whoever sets it up");
    assert_eq!(post(&addr, "/v1/admin/data", Some(&wes), json!({ "id": stock, "name": "Mine", "kind": "sqlite", "address": "/etc" })).await.0, 403);

    // Uma's sheet goes in a file Nia opens: Nia runs it (with her values),
    // and nothing else.
    let ticket = a["ticket"].as_str().unwrap().to_string();
    let (status, a) = post(&addr, &format!("/v1/data/{stock}/query"), Some(&nia), json!({ "query": low, "params": { "under": 50 }, "ticket": ticket })).await;
    assert_eq!((status, &a["rows"]), (200, &json!([["Tea"], ["Coffee"]])), "{a}");
    let (status, _) = post(&addr, &format!("/v1/data/{stock}/query"), Some(&nia), json!({ "sql": "SELECT * FROM stock", "ticket": ticket })).await;
    assert_eq!(status, 403);
    assert_eq!(post(&addr, &format!("/v1/data/{stock}/queries"), Some(&nia), json!({ "name": "x", "sql": "SELECT 1" })).await.0, 403);
    assert_eq!(get(&addr, &format!("/v1/data/{stock}/tables"), Some(&nia)).await.0, 403);
    // Not signed in: nothing, ticket or not.
    assert_eq!(post(&addr, &format!("/v1/data/{stock}/query"), None, json!({ "query": low, "ticket": ticket })).await.0, 401);

    // Wes changes only his own saved queries; Dana changes them all.
    let (status, dq) = post(&addr, &format!("/v1/admin/data/{stock}/queries"), Some(&dana), json!({ "name": "All", "sql": "SELECT * FROM stock" })).await;
    assert_eq!(status, 200, "{dq}");
    let all = dq["id"].as_i64().unwrap();
    assert_eq!(post(&addr, &format!("/v1/data/{stock}/queries"), Some(&wes), json!({ "id": low, "name": "Low stock", "sql": "SELECT item FROM stock WHERE qty <= :under" })).await.0, 200);
    assert_eq!(post(&addr, &format!("/v1/data/{stock}/queries"), Some(&wes), json!({ "id": all, "name": "Mine now", "sql": "SELECT 1" })).await.0, 403);
    assert_eq!(call(&addr, "DELETE", &format!("/v1/data/{stock}/queries/{all}"), Some(&wes), None).await.0, 403);
    let (_, qs) = get(&addr, &format!("/v1/admin/data/{stock}/queries"), Some(&dana)).await;
    assert_eq!(qs.as_array().unwrap().iter().find(|q| q["name"] == "Low stock").unwrap()["by"], "wes");
    assert_eq!(call(&addr, "DELETE", &format!("/v1/admin/data/{stock}/queries/{low}"), Some(&dana), None).await.0, 200);
    let (status, e) = post(&addr, &format!("/v1/data/{stock}/query"), Some(&nia), json!({ "query": low, "ticket": ticket })).await;
    assert_eq!(status, 404, "a removed query is gone from files too: {e}");

    // Dana takes the group off: Uma no longer writes queries on it.
    assert_eq!(post(&addr, "/v1/admin/data", Some(&dana), json!({ "id": stock, "name": "Stock", "kind": "sqlite", "address": file.to_string_lossy(), "writers": [{ "user": wes_id }] })).await.0, 200);
    assert_eq!(get(&addr, "/v1/data", Some(&uma)).await.1, json!([]));
    assert_eq!(get(&addr, "/v1/data", Some(&wes)).await.1[0]["name"], "Stock");

    // From before: the groups of a source that allowed their own queries
    // write queries; the rest (who could only run saved ones) need tickets
    // now, which writers' runs give.
    let (c, conn) = tokio_postgres::connect(&db, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    for (name, own) in [("Old", true), ("Older", false)] {
        c.execute(
            "INSERT INTO data_sources (name, kind, address, own_queries, groups) VALUES ($1, 'sqlite', $2, $3, $4)",
            &[&name, &file.to_string_lossy().to_string(), &own, &vec![analysts]],
        )
        .await
        .unwrap();
    }
    drop(server);
    let (addr, _server) = serve(&db).await;
    let (_, hers) = get(&addr, "/v1/data", Some(&uma)).await;
    let names: Vec<&str> = hers.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Old"], "{hers}");
    let _ = std::fs::remove_dir_all(&dir);
}
