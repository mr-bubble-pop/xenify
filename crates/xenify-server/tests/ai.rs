//! AI through the server, against a pretend runner, Hugging Face and
//! Civitai (one small web server standing in for all three).

mod common;

use axum::extract::Path as UrlPath;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get as on_get, post as on_post};
use common::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const GGUF: &[u8] = b"GGUF pretend model weights 0123456789";

fn sha(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// The pretend sites; returns their address and the configs LocalAI was asked to load.
async fn pretend() -> (String, Arc<Mutex<Vec<Value>>>) {
    let applied: Arc<Mutex<Vec<Value>>> = Arc::default();
    let a2 = applied.clone();
    let app = axum::Router::new()
        // The runner.
        .route("/v1/models", on_get(|| async { axum::Json(json!({ "data": [{ "id": "qwen" }] })) }))
        .route(
            "/v1/chat/completions",
            on_post(|h: HeaderMap, body: String| async move {
                let v: Value = serde_json::from_str(&body).unwrap();
                let auth = h.get("authorization").and_then(|x| x.to_str().ok()).unwrap_or("").to_string();
                let text = format!(
                    "data: {{\"model\":\"{}\",\"auth\":\"{auth}\"}}\n\ndata: {{\"choices\":[{{\"delta\":{{\"content\":\"<think>Pondering</th\"}}}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{\"content\":\"ink>Profit is 12\"}}}}]}}\n\ndata: [DONE]\n\n",
                    v["model"].as_str().unwrap_or("")
                );
                ([("content-type", "text/event-stream")], text)
            }),
        )
        .route(
            "/models/apply",
            on_post(move |body: String| {
                let a = a2.clone();
                async move {
                    a.lock().unwrap().push(serde_json::from_str(&body).unwrap());
                    axum::Json(json!({ "uuid": "1" }))
                }
            }),
        )
        .route(
            "/video",
            on_post(|body: String| async move {
                let v: Value = serde_json::from_str(&body).unwrap();
                // As LocalAI: a link to the file on the runner.
                axum::Json(json!({ "made_with": v["model"], "data": [{ "url": "/generated-videos/boat.mp4" }] }))
            }),
        )
        .route("/generated-videos/boat.mp4", on_get(|| async { &b"pretend mp4"[..] }))
        // Hugging Face.
        .route(
            "/api/models",
            on_get(|| async {
                axum::Json(json!([{ "id": "acme/tiny-GGUF", "downloads": 9, "tags": ["gguf", "license:apache-2.0"], "gated": false }]))
            }),
        )
        .route(
            "/api/models/{owner}/{repo}",
            on_get(|| async {
                axum::Json(json!({ "tags": ["diffusers:WanPipeline"], "siblings": [
                    { "rfilename": "README.md", "size": 10 },
                    { "rfilename": "tiny-Q4_K_M.gguf", "size": GGUF.len(), "lfs": { "sha256": sha(GGUF) } },
                    { "rfilename": "tiny-bad.gguf", "size": GGUF.len(), "lfs": { "sha256": "00" } },
                    { "rfilename": "old.ckpt", "size": 5 },
                ]}))
            }),
        )
        .route(
            "/acme/tiny-GGUF/resolve/main/{file}",
            on_get(|UrlPath(file): UrlPath<String>, h: HeaderMap| async move {
                if file == "old.ckpt" {
                    return (StatusCode::OK, b"pickle".to_vec()).into_response();
                }
                let from: usize = h
                    .get("range")
                    .and_then(|r| r.to_str().ok())
                    .and_then(|r| r.strip_prefix("bytes=")?.trim_end_matches('-').parse().ok())
                    .unwrap_or(0);
                if from > 0 {
                    (StatusCode::PARTIAL_CONTENT, GGUF[from..].to_vec()).into_response()
                } else {
                    (StatusCode::OK, GGUF.to_vec()).into_response()
                }
            }),
        )
        // Civitai.
        .route(
            "/api/v1/models",
            on_get(|| async {
                axum::Json(json!({ "items": [
                    { "id": 7, "name": "Watercolour", "type": "Checkpoint", "nsfw": false, "allowCommercialUse": ["Image"],
                      "stats": { "downloadCount": 5 }, "modelVersions": [{ "baseModel": "SDXL 1.0" }] },
                    { "id": 8, "name": "Not for work", "type": "Checkpoint", "nsfw": true, "modelVersions": [] },
                ]}))
            }),
        )
        .route(
            "/api/v1/models/7",
            on_get(|| async {
                axum::Json(json!({ "modelVersions": [{ "name": "v1", "baseModel": "SDXL 1.0", "files": [
                    { "name": "watercolour.ckpt", "sizeKB": 1.0, "downloadUrl": "x", "metadata": { "format": "PickleTensor" }, "hashes": {} },
                    { "name": "watercolour.safetensors", "sizeKB": 1.0, "downloadUrl": "x", "metadata": { "format": "SafeTensor" }, "hashes": {} },
                ]}]}))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (addr, applied)
}

async fn wait_job(addr: &str, admin: &str, id: &str) -> Value {
    for _ in 0..100 {
        let (_, jobs) = get(addr, "/v1/admin/ai/jobs", Some(admin)).await;
        let j = jobs.as_array().unwrap().iter().find(|j| j["id"] == id).unwrap().clone();
        if ["installed", "failed", "stopped"].contains(&j["state"].as_str().unwrap()) {
            return j;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the install didn't finish");
}

#[tokio::test]
async fn apps_use_the_servers_ai_and_admins_install_models() {
    let Some(db) = fresh_db("ai").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (site, applied) = pretend().await;
    let models = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("models-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&models);
    std::fs::create_dir_all(&models).unwrap();

    // Not set up yet.
    let (status, _) = post(&addr, "/v1/ai/chat/completions", Some(&admin), json!({ "messages": [] })).await;
    assert_eq!(status, 503);

    let (status, s) = post(&addr, "/v1/admin/ai", Some(&admin), json!({
        "runner": "localai", "url": format!("{site}/v1"), "key": "runner-secret", "models_dir": models.to_str().unwrap(),
        "text_model": "qwen", "hf_base": site, "civitai_base": site,
    }))
    .await;
    assert_eq!(status, 200, "{s}");
    assert_eq!((s["key"].clone(), s["key_set"].clone()), (json!(""), json!(true)), "secrets never come back");
    // Changing something else keeps the key.
    post(&addr, "/v1/admin/ai", Some(&admin), json!({ "image_model": "sdxl" })).await;
    let (_, s) = get(&addr, "/v1/admin/ai", Some(&admin)).await;
    assert_eq!(s["key_set"], true);
    let (_, st) = get(&addr, "/v1/admin/ai/status", Some(&admin)).await;
    assert_eq!((st["reachable"].clone(), st["models"].clone()), (json!(true), json!(["qwen"])));

    // An app asks without naming a model: the default goes in, the runner's key is added.
    let priya = &add_person(&addr, &admin, "priya", false).await;
    let (_, info) = get(&addr, "/v1/ai", Some(priya)).await;
    assert_eq!((info["available"].clone(), info["text_model"].clone()), (json!(true), json!("qwen")));
    let raw = {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let body = json!({ "messages": [{ "role": "user", "content": "hi" }], "stream": true }).to_string();
        let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
        c.write_all(format!("POST /v1/ai/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {priya}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).await.unwrap();
        out
    };
    assert!(raw.contains("text/event-stream"), "{raw}");
    assert!(raw.contains("\"model\":\"qwen\"") && raw.contains("Bearer runner-secret"), "{raw}");
    assert_eq!(get(&addr, "/v1/admin/ai", Some(priya)).await.0, 403);

    // Working with data: its own model (the writing one when there's none),
    // and the thinking apart from the answer.
    let ask = |path: &'static str| {
        let (addr, priya) = (addr.clone(), priya.to_string());
        async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let body = json!({ "messages": [{ "role": "user", "content": "profit?" }], "stream": true }).to_string();
            let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
            c.write_all(format!("POST {path} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {priya}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).await.unwrap();
            out
        }
    };
    let raw = ask("/v1/ai/data/chat/completions").await;
    assert!(raw.contains("\"model\":\"qwen\""), "no data model: the writing one: {raw}");
    post(&addr, "/v1/admin/ai", Some(&admin), json!({ "data_model": "qwen-think", "data_thinking": "thorough" })).await;
    let (_, info) = get(&addr, "/v1/ai", Some(priya)).await;
    assert_eq!((info["data_model"].clone(), info["data_thinking"].clone()), (json!("qwen-think"), json!("thorough")));
    assert_eq!(info["offered"]["data"], json!(["qwen-think"]));
    let raw = ask("/v1/ai/data/chat/completions").await;
    assert!(raw.contains("\"model\":\"qwen-think\""), "{raw}");
    assert!(
        raw.contains("\"reasoning\":\"Pondering\"")
            && raw.contains("\"content\":\"Profit is 12\"")
            && !raw.contains("<think>"),
        "{raw}"
    );
    // Writing keeps the answer as the model wrote it.
    assert!(ask("/v1/ai/chat/completions").await.contains("<think>"));

    // Searching and looking at files.
    let (_, hits) = get(&addr, "/v1/admin/ai/search?source=hf&q=tiny&kind=text", Some(&admin)).await;
    assert_eq!((hits[0]["id"].clone(), hits[0]["license"].clone()), (json!("acme/tiny-GGUF"), json!("apache-2.0")));
    let (_, files) = get(&addr, "/v1/admin/ai/files?source=hf&id=acme/tiny-GGUF", Some(&admin)).await;
    let safe: Vec<(Value, Value)> =
        files.as_array().unwrap().iter().map(|f| (f["name"].clone(), f["safe"].clone())).collect();
    assert_eq!(
        safe,
        [
            (json!("tiny-Q4_K_M.gguf"), json!(true)),
            (json!("tiny-bad.gguf"), json!(true)),
            (json!("old.ckpt"), json!(false))
        ]
    );
    let (_, hits) = get(&addr, "/v1/admin/ai/search?source=civitai&q=water&kind=image", Some(&admin)).await;
    assert_eq!(hits.as_array().unwrap().len(), 1, "adult models stay hidden");
    assert_eq!(hits[0]["base_model"], "SDXL 1.0");
    let (_, cf) = get(&addr, "/v1/admin/ai/files?source=civitai&id=7", Some(&admin)).await;
    assert_eq!((cf[0]["safe"].clone(), cf[1]["safe"].clone()), (json!(false), json!(true)));

    // Old checkpoint files are refused.
    let (status, e) = post(
        &addr,
        "/v1/admin/ai/install",
        Some(&admin),
        json!({ "source": "hf", "id": "acme/tiny-GGUF", "file": "old.ckpt", "kind": "text" }),
    )
    .await;
    assert_eq!((status, e["code"].clone()), (400, json!("unsafe")));

    // A half-finished download carries on, is checked, and LocalAI is told.
    std::fs::write(models.join("tiny-Q4_K_M.gguf.part"), &GGUF[..10]).unwrap();
    let (status, job) = post(&addr, "/v1/admin/ai/install", Some(&admin), json!({ "source": "hf", "id": "acme/tiny-GGUF", "file": "tiny-Q4_K_M.gguf", "name": "Tiny Chat", "kind": "text" })).await;
    assert_eq!(status, 200, "{job}");
    let done = wait_job(&addr, &admin, job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "installed", "{done}");
    assert_eq!(std::fs::read(models.join("tiny-Q4_K_M.gguf")).unwrap(), GGUF);
    let yaml = std::fs::read_to_string(models.join("tiny-chat.yaml")).unwrap();
    assert!(
        yaml.contains("name: tiny-chat") && yaml.contains("model: tiny-Q4_K_M.gguf") && yaml.contains("llama-cpp"),
        "{yaml}"
    );
    assert!(applied.lock().unwrap()[0]["config_url"].as_str().unwrap().ends_with("tiny-chat.yaml"));

    // A file that doesn't match its checksum is thrown away.
    let (_, job) = post(
        &addr,
        "/v1/admin/ai/install",
        Some(&admin),
        json!({ "source": "hf", "id": "acme/tiny-GGUF", "file": "tiny-bad.gguf", "kind": "text" }),
    )
    .await;
    let done = wait_job(&addr, &admin, job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed");
    assert!(!models.join("tiny-bad.gguf").exists() && !models.join("tiny-bad.gguf.part").exists());

    // People choose among the models offered; the default is always there.
    post(
        &addr,
        "/v1/admin/ai",
        Some(&admin),
        json!({ "video_model": "wan", "offered": { "text": ["qwen-big"], "image": ["sdxl", "water"] } }),
    )
    .await;
    let (_, info) = get(&addr, "/v1/ai", Some(priya)).await;
    assert_eq!(info["offered"]["text"], json!(["qwen", "qwen-big"]));
    assert_eq!(info["offered"]["image"], json!(["sdxl", "water"]));
    assert_eq!(info["offered"]["video"], json!(["wan"]));
    // Only those: not any model the runner (or a paid service) has.
    let (status, e) = post(
        &addr,
        "/v1/ai/chat/completions",
        Some(priya),
        json!({ "model": "openai/very-expensive", "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await;
    assert_eq!((status, e["code"].clone()), (400, json!("model_not_offered")), "{e}");
    let (status, _) =
        post(&addr, "/v1/ai/images/generations", Some(priya), json!({ "model": "qwen-big", "prompt": "x" })).await;
    assert_eq!(status, 400, "a writing model doesn't make pictures");
    // Pictures go in the request: the runner isn't sent to fetch an address.
    let (status, e) = post(&addr, "/v1/ai/chat/completions", Some(priya), json!({ "model": "qwen-big", "messages": [{ "role": "user", "content": [
        { "type": "image_url", "image_url": { "url": "http://169.254.169.254/latest/meta-data/" } }] }] })).await;
    assert_eq!((status, e["code"].clone()), (400, json!("bad_request")), "{e}");
    let (status, v) = post(&addr, "/v1/ai/video", Some(priya), json!({ "prompt": "a paper boat" })).await;
    assert_eq!((status, v["made_with"].clone()), (200, json!("wan")));
    // The file itself comes back (the apps can't reach the runner).
    assert_eq!(v["data"][0]["b64_json"], json!("cHJldGVuZCBtcDQ="));
    assert!(v["data"][0].get("url").is_none());

    // A whole diffusers model from Hugging Face: settings only, LocalAI fetches it.
    let (status, job) = post(
        &addr,
        "/v1/admin/ai/install",
        Some(&admin),
        json!({ "source": "hf", "id": "acme/tiny-GGUF", "name": "Boat Video", "kind": "video" }),
    )
    .await;
    assert_eq!((status, job["state"].clone()), (200, json!("installed")), "{job}");
    let y = std::fs::read_to_string(models.join("boat-video.yaml")).unwrap();
    assert!(y.contains("model: acme/tiny-GGUF") && y.contains("pipeline_type: WanPipeline"), "{y}");

    // Picture models get diffusers settings that fit their base model.
    let y = xenify_server::ai::localai_config("water", "watercolour.safetensors", "image", "SDXL 1.0");
    assert!(y.contains("backend: diffusers") && y.contains("StableDiffusionXLPipeline"), "{y}");

    // A picture style: a picture model with a LoRA, offered as a model of its own.
    std::fs::write(models.join("water.yaml"), &y).unwrap();
    std::fs::create_dir_all(models.join("loras")).unwrap();
    std::fs::write(models.join("loras/ink-wash.safetensors"), b"pretend").unwrap();
    std::fs::write(models.join("loras/run-me.ckpt"), b"pretend").unwrap();
    let (_, parts) = get(&addr, "/v1/admin/ai/styles", Some(&admin)).await;
    assert_eq!((parts["bases"].clone(), parts["loras"].clone()), (json!(["water"]), json!(["ink-wash.safetensors"])));
    let style =
        |lora: &str, strength: f64| json!({ "name": "Ink Wash", "base": "water", "lora": lora, "strength": strength });
    for (bad, code) in [
        (style("../water.yaml", 1.0), 400),
        (style("run-me.ckpt", 1.0), 400),
        (style("ink-wash.safetensors", 5.0), 400),
        (style("gone.safetensors", 1.0), 404),
    ] {
        assert_eq!(post(&addr, "/v1/admin/ai/styles", Some(&admin), bad).await.0, code);
    }
    assert_eq!(post(&addr, "/v1/admin/ai/styles", Some(priya), style("ink-wash.safetensors", 0.8)).await.0, 403);
    let (status, s) = post(&addr, "/v1/admin/ai/styles", Some(&admin), style("ink-wash.safetensors", 0.8)).await;
    assert_eq!(status, 200, "{s}");
    let y = std::fs::read_to_string(models.join("ink-wash.yaml")).unwrap();
    assert!(
        y.starts_with("name: ink-wash\n")
            && y.contains("model: watercolour.safetensors")
            && y.contains("lora_adapters:\n  - loras/ink-wash.safetensors\nlora_scales:\n  - 0.8\n"),
        "{y}"
    );
    // LocalAI is told in the background.
    let told = || applied.lock().unwrap().iter().any(|a| a["config_url"].as_str().unwrap().ends_with("ink-wash.yaml"));
    for _ in 0..50 {
        if told() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(told());
    let (_, info) = get(&addr, "/v1/ai", Some(priya)).await;
    assert!(info["offered"]["image"].as_array().unwrap().contains(&json!("ink-wash")), "{info}");
    // A style isn't a base for another.
    let (_, parts) = get(&addr, "/v1/admin/ai/styles", Some(&admin)).await;
    assert_eq!(parts["bases"], json!(["water"]));
    // Made from a style, the LoRA is replaced, not added.
    let again = xenify_server::ai::style_config(&y, "ink-soft", "soft.safetensors", 0.5);
    assert_eq!(again.matches("lora_adapters:").count(), 1);
    assert!(again.contains("  - loras/soft.safetensors\n") && !again.contains("ink-wash.safetensors"), "{again}");

    // A new address for the runner: the kept key isn't sent there.
    let (status, s) = post(&addr, "/v1/admin/ai", Some(&admin), json!({ "url": "http://elsewhere.example:9/v1" })).await;
    assert_eq!(status, 200, "{s}");
    assert_eq!((s["key_set"].clone(), s["cleared"].clone()), (json!(false), json!(["key"])));
    // A key typed with the new address stays.
    let (_, s) = post(&addr, "/v1/admin/ai", Some(&admin), json!({ "url": format!("{site}/v1"), "key": "new-secret" })).await;
    assert_eq!((s["key_set"].clone(), s["cleared"].clone()), (json!(true), json!([])));
    // The runner's address is shown only to those who look after AI.
    post(&addr, "/v1/admin/ai", Some(&admin), json!({ "url": "http://127.0.0.1:9/v1" })).await;
    let (status, e) = get(&addr, "/v1/ai/models", Some(priya)).await;
    assert_eq!(status, 502);
    assert!(!e.to_string().contains("127.0.0.1"), "{e}");
    let (_, e) = get(&addr, "/v1/ai/models", Some(&admin)).await;
    assert!(e.to_string().contains("127.0.0.1:9"), "{e}");
}
