//! MCP tool servers an admin adds for everyone: kept keys, messages passed on,
//! each person in their own session, tools that change things only when the
//! admin allows them.

mod common;

use axum::http::HeaderMap;
use axum::routing::post as on_post;
use common::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A pretend MCP tool server (Streamable HTTP) that wants a key and gives a
/// new session to each `initialize`. Returns its address and how many
/// messages it was sent.
async fn pretend() -> (String, Arc<AtomicUsize>) {
    let sent = Arc::new(AtomicUsize::new(0));
    let sessions = Arc::new(AtomicUsize::new(0));
    let s2 = sent.clone();
    let app = axum::Router::new().route(
        "/mcp",
        on_post(move |h: HeaderMap, body: String| {
            let (sent, sessions) = (s2.clone(), sessions.clone());
            async move {
                sent.fetch_add(1, Ordering::SeqCst);
                let v: Value = serde_json::from_str(&body).unwrap();
                if h.get("x-key").and_then(|k| k.to_str().ok()) != Some("tool-secret") {
                    return (axum::http::StatusCode::UNAUTHORIZED, [("mcp-session-id", String::new())], String::new());
                }
                let method = v["method"].as_str().unwrap_or("");
                let mut session = h.get("mcp-session-id").and_then(|k| k.to_str().ok()).unwrap_or("").to_string();
                if method == "initialize" {
                    session = format!("s-{}", sessions.fetch_add(1, Ordering::SeqCst) + 1);
                }
                let result = match method {
                    "initialize" => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "Accounts" } }),
                    "tools/list" => json!({ "tools": [
                        { "name": "balance", "description": "The bank balance", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": true } },
                        { "name": "pay", "description": "Pays someone", "inputSchema": { "type": "object" } },
                    ] }),
                    "tools/call" => {
                        json!({ "content": [{ "type": "text", "text": format!("{} done (session {session})", v["params"]["name"].as_str().unwrap_or("")) }] })
                    }
                    _ => json!({}),
                };
                // Answers come as an event stream, as many servers send them.
                let text = format!("event: message\ndata: {}\n\n", json!({ "jsonrpc": "2.0", "id": v["id"], "result": result }));
                (axum::http::StatusCode::OK, [("mcp-session-id", session)], text)
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://127.0.0.1:{}/mcp", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (addr, sent)
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[tokio::test]
async fn admins_add_tools_people_use_them() {
    let Some(db) = fresh_db("mcp").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (url, sent) = pretend().await;

    let (status, made) = post(
        &addr,
        "/v1/admin/mcp",
        Some(&admin),
        json!({ "name": "Accounts", "url": url, "headers": { "x-key": "tool-secret" } }),
    )
    .await;
    assert_eq!(status, 200, "{made}");
    let id = made["id"].as_u64().unwrap();
    let (_, list) = get(&addr, "/v1/admin/mcp", Some(&admin)).await;
    assert_eq!(list[0]["headers"], json!(["x-key"]));
    assert_eq!(list[0]["changes"], false, "tools that change things are off at first");
    assert!(!list.to_string().contains("tool-secret"), "keys never come back");
    // Changing it without the key keeps the key.
    let (_, saved) = post(
        &addr,
        "/v1/admin/mcp",
        Some(&admin),
        json!({ "id": id, "name": "Accounts", "url": url, "headers": { "x-key": "" } }),
    )
    .await;
    assert_eq!(saved["cleared"], json!([]));

    // Someone signed in (not an admin) sees it and uses it through the server.
    let sam = &add_person(&addr, &admin, "sam", false).await;
    let ann = &add_person(&addr, &admin, "ann", false).await;
    let (_, tools) = get(&addr, "/v1/mcp", Some(sam)).await;
    assert_eq!(tools, json!([{ "id": id, "name": "Accounts" }]));
    let path = format!("/v1/mcp/{id}");
    let (status, r) = post(&addr, &path, Some(sam), rpc(1, "initialize", json!({}))).await;
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["result"]["serverInfo"]["name"], "Accounts");
    post(&addr, &path, Some(ann), rpc(1, "initialize", json!({}))).await;

    // Only the tools that say they only read are listed, and callable.
    let (_, r) = post(&addr, &path, Some(sam), rpc(2, "tools/list", json!({}))).await;
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["balance"]);
    let (_, r) = post(&addr, &path, Some(sam), rpc(3, "tools/call", json!({ "name": "balance", "arguments": {} }))).await;
    assert_eq!(r["result"]["content"][0]["text"], "balance done (session s-1)", "Sam's own session");
    let (_, r) = post(&addr, &path, Some(ann), rpc(3, "tools/call", json!({ "name": "balance", "arguments": {} }))).await;
    assert_eq!(r["result"]["content"][0]["text"], "balance done (session s-2)", "Ann's own session");
    let before = sent.load(Ordering::SeqCst);
    let (_, r) = post(&addr, &path, Some(sam), rpc(4, "tools/call", json!({ "name": "pay", "arguments": { "to": "x" } }))).await;
    assert!(r["error"]["message"].as_str().unwrap().contains("only tools that read"), "{r}");
    assert!(r.get("result").is_none());
    // Other messages aren't passed on.
    let (_, r) = post(&addr, &path, Some(sam), rpc(5, "resources/read", json!({ "uri": "file:///etc/passwd" }))).await;
    assert!(r["error"].is_object(), "{r}");
    assert_eq!(sent.load(Ordering::SeqCst), before, "neither reached the tool server");

    // The admin allows tools that change things.
    post(&addr, "/v1/admin/mcp", Some(&admin), json!({ "id": id, "name": "Accounts", "url": url, "headers": {}, "changes": true }))
        .await;
    post(&addr, &path, Some(sam), rpc(1, "initialize", json!({}))).await;
    let (_, r) = post(&addr, &path, Some(sam), rpc(2, "tools/list", json!({}))).await;
    assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 2);
    let (_, r) = post(&addr, &path, Some(sam), rpc(3, "tools/call", json!({ "name": "pay", "arguments": {} }))).await;
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().starts_with("pay done"), "{r}");

    assert_eq!(get(&addr, "/v1/admin/mcp", Some(sam)).await.0, 403);

    // Without the permission (taken from Everyone), no tools.
    let (_, roles) = get(&addr, "/v1/roles", Some(&admin)).await;
    let everyone =
        roles["roles"].as_array().unwrap().iter().find(|r| r["everyone"] == true).unwrap()["id"].as_i64().unwrap();
    let (status, _) =
        post(&addr, &format!("/v1/admin/roles/{everyone}"), Some(&admin), json!({ "perms": ["ai.text"] })).await;
    assert_eq!(status, 200);
    assert_eq!(get(&addr, "/v1/mcp", Some(sam)).await.1, json!([]));
    let (status, _) = post(&addr, &path, Some(sam), rpc(6, "tools/list", json!({}))).await;
    assert_eq!(status, 403);

    // A new address for the tool: the kept key isn't sent there.
    let elsewhere = url.replace("127.0.0.1", "localhost");
    let (_, saved) = post(
        &addr,
        "/v1/admin/mcp",
        Some(&admin),
        json!({ "id": id, "name": "Accounts", "url": elsewhere, "headers": { "x-key": "" } }),
    )
    .await;
    assert_eq!(saved["cleared"], json!(["x-key"]));
    let (_, list) = get(&addr, "/v1/admin/mcp", Some(&admin)).await;
    assert_eq!(list[0]["headers"], json!([]));

    let (status, _) = call(&addr, "DELETE", &format!("/v1/admin/mcp/{id}"), Some(&admin), None).await;
    assert_eq!(status, 200);
    assert_eq!(get(&addr, "/v1/mcp", Some(&admin)).await.1, json!([]));
}
