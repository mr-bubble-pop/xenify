//! Pictures and clips through ComfyUI, against a pretend ComfyUI that has
//! FLUX.2 klein 4B's files but not all of Wan 2.2's.

mod common;

use axum::extract::{Path as UrlPath, Query};
use axum::routing::{get as on_get, post as on_post};
use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The pretend ComfyUI; returns its address and the workflows it was sent.
async fn pretend() -> (String, Arc<Mutex<Vec<Value>>>) {
    let sent: Arc<Mutex<Vec<Value>>> = Arc::default();
    let s2 = sent.clone();
    let have: HashMap<&str, Vec<&str>> = HashMap::from([
        ("UNETLoader", vec!["flux-2-klein-4b.safetensors", "wan/wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors"]),
        ("CLIPLoader", vec!["qwen_3_4b.safetensors"]),
        ("VAELoader", vec!["flux2-vae.safetensors"]),
        ("LoraLoaderModelOnly", vec![]),
    ]);
    let app = axum::Router::new()
        .route(
            "/object_info/{loader}",
            on_get(move |UrlPath(loader): UrlPath<String>| {
                let input = match loader.as_str() {
                    "UNETLoader" => "unet_name",
                    "CLIPLoader" => "clip_name",
                    "VAELoader" => "vae_name",
                    _ => "lora_name",
                };
                // Newer ComfyUI's way of listing a loader's files.
                let files = have.get(loader.as_str()).cloned().unwrap_or_default();
                async move { axum::Json(json!({ loader.clone(): { "input": { "required": { input: ["COMBO", { "options": files }] } } } })) }
            }),
        )
        .route(
            "/prompt",
            on_post(move |body: String| {
                let s = s2.clone();
                async move {
                    let v: Value = serde_json::from_str(&body).unwrap();
                    let graph = v["prompt"].clone();
                    s.lock().unwrap().push(graph.clone());
                    if graph.to_string().contains("umt5_xxl") {
                        let e = json!({ "error": { "message": "Prompt outputs failed validation" }, "node_errors": { "128:71": { "errors": [
                            { "message": "Value not in list", "details": "clip_name: 'umt5_xxl_fp8_e4m3fn_scaled.safetensors' not in []" }] } } });
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(e));
                    }
                    (axum::http::StatusCode::OK, axum::Json(json!({ "prompt_id": "p1", "number": 1 })))
                }
            }),
        )
        .route(
            "/history/{id}",
            on_get(|UrlPath(id): UrlPath<String>| async move {
                axum::Json(json!({ id: { "status": { "status_str": "success", "completed": true },
                    "outputs": { "9": { "images": [{ "filename": "xenify/flux.2-klein-4b_00001_.png", "subfolder": "", "type": "output" }] } } } }))
            }),
        )
        .route(
            "/view",
            on_get(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q["filename"], "xenify/flux.2-klein-4b_00001_.png");
                &b"pretend png"[..]
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), sent)
}

#[tokio::test]
async fn pictures_and_clips_through_comfyui() {
    let Some(db) = fresh_db("comfy").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (comfy, sent) = pretend().await;

    // ComfyUI only, by its address without http://, and FLUX.2 klein as the picture model.
    let (status, s) = post(&addr, "/v1/admin/ai", Some(&admin), json!({
        "comfy_url": comfy.trim_start_matches("http://"), "image_model": "flux.2-klein-4b", "video_model": "wan2.2-14b",
    }))
    .await;
    assert_eq!(status, 200, "{s}");
    let (_, info) = get(&addr, "/v1/ai", Some(&admin)).await;
    assert_eq!(info["available"], true, "ComfyUI alone is AI");

    // Which models are ready, and what the others still need.
    let (_, st) = get(&addr, "/v1/admin/ai/comfy", Some(&admin)).await;
    assert_eq!(st["reachable"], true, "{st}");
    let by_id = |id: &str| st["models"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap().clone();
    assert_eq!(by_id("flux.2-klein-4b")["ready"], true);
    let wan = by_id("wan2.2-14b");
    assert_eq!(wan["ready"], false);
    let missing: Vec<&str> = wan["missing"].as_array().unwrap().iter().map(|m| m["file"].as_str().unwrap()).collect();
    assert!(
        !missing.contains(&"wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors"),
        "found in a subfolder: {missing:?}"
    );
    assert!(missing.contains(&"umt5_xxl_fp8_e4m3fn_scaled.safetensors"), "{missing:?}");
    assert!(wan["missing"][0]["url"].as_str().unwrap().starts_with("https://huggingface.co/Comfy-Org/"));

    // A picture the way Draw asks for one: the words and size go in, the file comes back.
    let (status, out) = post(
        &addr,
        "/v1/ai/images/generations",
        Some(&admin),
        json!({
            "prompt": "a red kite over the sea", "size": "1536x1024", "n": 1, "response_format": "b64_json",
        }),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["data"][0]["b64_json"], "cHJldGVuZCBwbmc=", "standard base64 of the file, as the apps decode it");
    let g = sent.lock().unwrap()[0].clone();
    assert_eq!(g["74"]["inputs"]["text"], "a red kite over the sea");
    assert_eq!((g["66"]["inputs"]["width"].clone(), g["66"]["inputs"]["height"].clone()), (json!(1536), json!(1024)));

    // A clip whose files aren't all there: ComfyUI's reason is passed on.
    let (status, e) =
        post(&addr, "/v1/ai/video", Some(&admin), json!({ "prompt": "paper boats on a rainy street" })).await;
    assert_eq!(status, 502);
    assert!(e["error"].as_str().unwrap().contains("umt5_xxl"), "{e}");

    // A picture model that isn't offered: refused.
    let (status, _) =
        post(&addr, "/v1/ai/images/generations", Some(&admin), json!({ "prompt": "x", "model": "sdxl" })).await;
    assert_eq!(status, 400);
}

/// A pretend ComfyUI that never finishes; returns its address and the
/// calls that stopped the work.
async fn slow() -> (String, Arc<Mutex<Vec<String>>>) {
    let stops: Arc<Mutex<Vec<String>>> = Arc::default();
    let (s1, s2) = (stops.clone(), stops.clone());
    let app = axum::Router::new()
        .route("/prompt", on_post(|| async { axum::Json(json!({ "prompt_id": "p9", "number": 1 })) }))
        .route("/history/{id}", on_get(|| async { axum::Json(json!({})) }))
        .route(
            "/queue",
            on_get(|| async { axum::Json(json!({ "queue_running": [[1, "p9", {}, {}, []]], "queue_pending": [] })) })
                .post(move |body: String| {
                    let s = s1.clone();
                    async move {
                        s.lock().unwrap().push(format!("queue {body}"));
                        axum::Json(json!({}))
                    }
                }),
        )
        .route(
            "/interrupt",
            on_post(move |body: String| {
                let s = s2.clone();
                async move {
                    s.lock().unwrap().push(format!("interrupt {body}"));
                    axum::Json(json!({}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), stops)
}

#[tokio::test]
async fn going_away_stops_the_work_and_limits_hold() {
    let Some(db) = fresh_db("comfy_stop").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (comfy, stops) = slow().await;
    let (status, s) = post(&addr, "/v1/admin/ai", Some(&admin), json!({
        "comfy_url": comfy, "image_model": "flux.2-klein-4b", "limits": { "pictures_at_once": 1, "server_at_once": 4, "media_per_hour": 60 },
    }))
    .await;
    assert_eq!(status, 200, "{s}");
    assert_eq!(s["limits"]["pictures_at_once"], 1);

    // A picture asked for, then the app goes away.
    use tokio::io::AsyncWriteExt;
    let body = json!({ "prompt": "a red kite" }).to_string();
    let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
    c.write_all(format!("POST /v1/ai/images/generations HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {admin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    // While it's being made, a second one waits its turn.
    let (status, e) = post(&addr, "/v1/ai/images/generations", Some(&admin), json!({ "prompt": "another" })).await;
    assert_eq!((status, e["code"].clone()), (429, json!("busy")), "{e}");
    drop(c);
    let mut stopped = Vec::new();
    for _ in 0..50 {
        stopped = stops.lock().unwrap().clone();
        if stopped.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(stopped.iter().any(|x| x.starts_with("interrupt") && x.contains("p9")), "{stopped:?}");
    assert!(stopped.iter().any(|x| x.starts_with("queue") && x.contains("p9")), "{stopped:?}");
    // Its place is given back: the next one is being made (not refused).
    use tokio::io::AsyncReadExt;
    let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
    c.write_all(format!("POST /v1/ai/images/generations HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {admin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    let mut buf = [0u8; 64];
    let got = tokio::time::timeout(std::time::Duration::from_millis(1500), c.read(&mut buf)).await;
    assert!(got.is_err(), "still being made: {:?}", got.map(|n| String::from_utf8_lossy(&buf[..n.unwrap_or(0)]).to_string()));
}
