//! Pictures and video clips through ComfyUI, which people install
//! themselves (Stability Matrix, or ComfyUI's own installer) and which the
//! server reaches by its address alone.
//!
//! - The server knows the models it was tested with (docs/ai-model-tests.md):
//!   FLUX.2 klein 4B and Z-Image Turbo for pictures, Wan 2.2 14B and
//!   MiniMax H3 for clips. Each is ComfyUI's own default workflow (the
//!   templates in `comfy/`, from Comfy-Org/workflow_templates, MIT; FLUX.2
//!   klein written out from its text-to-image template) with the person's
//!   words, the size and a new seed put in.
//! - ComfyUI says which model files it has (`/object_info`), so admins see
//!   which models are ready and which files the others still need. The
//!   server never downloads models into ComfyUI: Stability Matrix's model
//!   browser does that.
//! - A request queues the workflow (`/prompt`), waits for it (`/history`)
//!   and sends the file back (`/view`) the way the apps already take
//!   pictures and clips: `{"data": [{"b64_json": …}]}`.

use crate::ai::Settings;
use crate::{Server, admin_power, err, failed, unavailable};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A model file a workflow loads: which loader node lists it, and where to
/// get it.
pub struct Need {
    loader: &'static str,
    input: &'static str,
    /// ComfyUI's models folder it goes in.
    pub folder: &'static str,
    pub file: &'static str,
    pub url: &'static str,
}

/// A model ComfyUI can run for the apps.
pub struct Recipe {
    /// What apps and settings call it.
    pub id: &'static str,
    pub name: &'static str,
    /// "image" or "video".
    pub kind: &'static str,
    pub files: &'static [Need],
    /// The size used when a request names none (each template's own).
    size: (u32, u32),
    /// The workflow for these words, size and seed.
    build: fn(&str, u32, u32, u64) -> Value,
    /// The node whose output is the file.
    save: &'static str,
}

const UNET: (&str, &str) = ("UNETLoader", "unet_name");
const CLIP: (&str, &str) = ("CLIPLoader", "clip_name");
const VAE: (&str, &str) = ("VAELoader", "vae_name");
const LORA: (&str, &str) = ("LoraLoaderModelOnly", "lora_name");

const fn need(l: (&'static str, &'static str), folder: &'static str, file: &'static str, url: &'static str) -> Need {
    Need { loader: l.0, input: l.1, folder, file, url }
}

pub const RECIPES: &[Recipe] = &[
    Recipe {
        id: "flux.2-klein-4b",
        name: "FLUX.2 klein 4B",
        kind: "image",
        files: &[
            need(
                UNET,
                "diffusion_models",
                "flux-2-klein-4b.safetensors",
                "https://huggingface.co/Comfy-Org/flux2-klein/resolve/main/split_files/diffusion_models/flux-2-klein-4b.safetensors",
            ),
            need(
                CLIP,
                "text_encoders",
                "qwen_3_4b.safetensors",
                "https://huggingface.co/Comfy-Org/flux2-klein/resolve/main/split_files/text_encoders/qwen_3_4b.safetensors",
            ),
            need(
                VAE,
                "vae",
                "flux2-vae.safetensors",
                "https://huggingface.co/Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors",
            ),
        ],
        size: (1024, 1024),
        build: klein,
        save: "9",
    },
    Recipe {
        id: "z-image-turbo",
        name: "Z-Image Turbo",
        kind: "image",
        files: &[
            need(
                UNET,
                "diffusion_models",
                "z_image_turbo_bf16.safetensors",
                "https://huggingface.co/Comfy-Org/z_image_turbo/resolve/main/split_files/diffusion_models/z_image_turbo_bf16.safetensors",
            ),
            need(
                CLIP,
                "text_encoders",
                "qwen_3_4b.safetensors",
                "https://huggingface.co/Comfy-Org/flux2-klein/resolve/main/split_files/text_encoders/qwen_3_4b.safetensors",
            ),
            need(
                VAE,
                "vae",
                "ae.safetensors",
                "https://huggingface.co/Comfy-Org/z_image_turbo/resolve/main/split_files/vae/ae.safetensors",
            ),
        ],
        size: (1024, 1024),
        build: zimage,
        save: "9",
    },
    Recipe {
        id: "wan2.2-14b",
        name: "Wan 2.2 14B",
        kind: "video",
        files: &[
            need(
                UNET,
                "diffusion_models",
                "wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors",
                "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors",
            ),
            need(
                UNET,
                "diffusion_models",
                "wan2.2_t2v_low_noise_14B_fp8_scaled.safetensors",
                "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_t2v_low_noise_14B_fp8_scaled.safetensors",
            ),
            need(
                CLIP,
                "text_encoders",
                "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
            ),
            need(
                VAE,
                "vae",
                "wan_2.1_vae.safetensors",
                "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan_2.1_vae.safetensors",
            ),
            // The template loads the 4-step LoRAs even with them switched off.
            need(
                LORA,
                "loras",
                "wan2.2_t2v_lightx2v_4steps_lora_v1.1_high_noise.safetensors",
                "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/loras/wan2.2_t2v_lightx2v_4steps_lora_v1.1_high_noise.safetensors",
            ),
            need(
                LORA,
                "loras",
                "wan2.2_t2v_lightx2v_4steps_lora_v1.1_low_noise.safetensors",
                "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/loras/wan2.2_t2v_lightx2v_4steps_lora_v1.1_low_noise.safetensors",
            ),
        ],
        size: (640, 640),
        build: wan,
        save: "80",
    },
    Recipe {
        id: "minimax-h3",
        name: "MiniMax H3",
        kind: "video",
        files: &[
            need(
                UNET,
                "diffusion_models",
                "minimax_h3_fl2va_pruned_int8_convrot.safetensors",
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/main/diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors",
            ),
            need(
                CLIP,
                "text_encoders",
                "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors",
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/main/text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors",
            ),
            need(
                VAE,
                "vae",
                "minimax_h3_video_vae_int8_convrot.safetensors",
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/main/vae/minimax_h3_video_vae_int8_convrot.safetensors",
            ),
            need(
                VAE,
                "vae",
                "minimax_h3_audio_vae_fp32.safetensors",
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/main/vae/minimax_h3_audio_vae_fp32.safetensors",
            ),
            // The template loads the turbo LoRA even with it switched off.
            need(
                LORA,
                "loras",
                "minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors",
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/main/loras/minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors",
            ),
        ],
        size: (864, 480),
        build: minimax,
        save: "92",
    },
];

pub fn recipe(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

fn template(json: &str) -> Value {
    serde_json::from_str(json).expect("a ComfyUI template that parses")
}

/// FLUX.2 klein 4B: the distilled text-to-image graph of ComfyUI's
/// image_flux2_klein_text_to_image template (4 steps, CFG 1).
fn klein(text: &str, w: u32, h: u32, seed: u64) -> Value {
    json!({
        "70": { "class_type": "UNETLoader", "inputs": { "unet_name": "flux-2-klein-4b.safetensors", "weight_dtype": "default" } },
        "71": { "class_type": "CLIPLoader", "inputs": { "clip_name": "qwen_3_4b.safetensors", "type": "flux2", "device": "default" } },
        "72": { "class_type": "VAELoader", "inputs": { "vae_name": "flux2-vae.safetensors" } },
        "74": { "class_type": "CLIPTextEncode", "inputs": { "text": text, "clip": ["71", 0] } },
        "76": { "class_type": "ConditioningZeroOut", "inputs": { "conditioning": ["74", 0] } },
        "63": { "class_type": "CFGGuider", "inputs": { "cfg": 1, "model": ["70", 0], "positive": ["74", 0], "negative": ["76", 0] } },
        "61": { "class_type": "KSamplerSelect", "inputs": { "sampler_name": "euler" } },
        "62": { "class_type": "Flux2Scheduler", "inputs": { "steps": 4, "width": w, "height": h } },
        "66": { "class_type": "EmptyFlux2LatentImage", "inputs": { "width": w, "height": h, "batch_size": 1 } },
        "73": { "class_type": "RandomNoise", "inputs": { "noise_seed": seed } },
        "64": { "class_type": "SamplerCustomAdvanced", "inputs": { "noise": ["73", 0], "guider": ["63", 0], "sampler": ["61", 0], "sigmas": ["62", 0], "latent_image": ["66", 0] } },
        "65": { "class_type": "VAEDecode", "inputs": { "samples": ["64", 0], "vae": ["72", 0] } },
        "9": { "class_type": "SaveImage", "inputs": { "filename_prefix": "xenify/flux.2-klein-4b", "images": ["65", 0] } },
    })
}

fn zimage(text: &str, w: u32, h: u32, seed: u64) -> Value {
    let mut g = template(include_str!("../comfy/image_z_image_turbo.json"));
    g["57:27"]["inputs"]["text"] = json!(text);
    g["57:13"]["inputs"]["width"] = json!(w);
    g["57:13"]["inputs"]["height"] = json!(h);
    g["57:3"]["inputs"]["seed"] = json!(seed);
    g["9"]["inputs"]["filename_prefix"] = json!("xenify/z-image-turbo");
    g
}

fn wan(text: &str, w: u32, h: u32, seed: u64) -> Value {
    let mut g = template(include_str!("../comfy/video_wan2_2_14B_t2v.json"));
    g["128:89"]["inputs"]["text"] = json!(text);
    g["128:74"]["inputs"]["width"] = json!(w);
    g["128:74"]["inputs"]["height"] = json!(h);
    g["128:81"]["inputs"]["noise_seed"] = json!(seed);
    g["80"]["inputs"]["filename_prefix"] = json!("xenify/wan2.2-14b");
    g
}

fn minimax(text: &str, w: u32, h: u32, seed: u64) -> Value {
    let mut g = template(include_str!("../comfy/video_minimax_h3_t2v.json"));
    let node = &mut g["140:131"]["inputs"];
    node["prompt"] = json!(text);
    node["width"] = json!(w);
    node["height"] = json!(h);
    g["140:129"]["inputs"]["noise_seed"] = json!(seed);
    g["92"]["inputs"]["filename_prefix"] = json!("xenify/minimax-h3");
    g
}

/// ComfyUI's address without a trailing slash (http:// added when left off).
fn base(a: &Settings) -> String {
    let u = a.comfy_url.trim().trim_end_matches('/');
    if u.starts_with("http://") || u.starts_with("https://") { u.to_string() } else { format!("http://{u}") }
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(secs)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn get_json(url: &str) -> Result<Value, String> {
    let mut r = agent(30).get(url).call().map_err(|e| format!("can't reach ComfyUI: {e}"))?;
    let status = r.status().as_u16();
    let text = r.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("ComfyUI answered {status}"));
    }
    serde_json::from_str(&text).map_err(|_| "ComfyUI sent something unexpected".into())
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The files a loader node offers, from `/object_info/<loader>`: older
/// ComfyUI lists them as `[[files…], {…}]`, newer as `["COMBO", {"options": [files…]}]`.
fn offered(info: &Value, loader: &str, input: &str) -> Vec<String> {
    let spec = &info[loader]["input"]["required"][input];
    let list = if spec[0].is_array() { &spec[0] } else { &spec[1]["options"] };
    list.as_array().map(|l| l.iter().filter_map(|f| f.as_str()).map(String::from).collect()).unwrap_or_default()
}

/// Which files each loader has (models in subfolders count by their name).
fn files_on(base: &str) -> Result<HashMap<(&'static str, &'static str), Vec<String>>, String> {
    let mut out = HashMap::new();
    for l in [UNET, CLIP, VAE, LORA] {
        let info = get_json(&format!("{base}/object_info/{}", l.0))?;
        let names = offered(&info, l.0, l.1)
            .into_iter()
            .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(&f).to_string())
            .collect();
        out.insert(l, names);
    }
    Ok(out)
}

/// The files a recipe still needs, given what ComfyUI has.
fn missing(r: &Recipe, have: &HashMap<(&'static str, &'static str), Vec<String>>) -> Vec<&'static Need> {
    r.files.iter().filter(|n| !have.get(&(n.loader, n.input)).is_some_and(|l| l.iter().any(|f| f == n.file))).collect()
}

/// Whether ComfyUI answers, and for each model whether it's ready or
/// which files it still needs.
pub fn status(a: &Settings) -> Value {
    if a.comfy_url.trim().is_empty() {
        return json!({ "set_up": false });
    }
    let base = base(a);
    match files_on(&base) {
        Ok(have) => {
            let models: Vec<Value> = RECIPES
                .iter()
                .map(|r| {
                    let need: Vec<Value> = missing(r, &have)
                        .iter()
                        .map(|n| json!({ "folder": n.folder, "file": n.file, "url": n.url }))
                        .collect();
                    json!({ "id": r.id, "name": r.name, "kind": r.kind, "ready": need.is_empty(), "missing": need })
                })
                .collect();
            json!({ "set_up": true, "reachable": true, "models": models })
        }
        Err(e) => json!({ "set_up": true, "reachable": false, "error": e }),
    }
}

/// GET /v1/admin/ai/comfy: `status` for the admin pages.
pub async fn admin_status(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.ai").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let a = crate::ai::settings(&c).await;
    drop(c);
    match tokio::task::spawn_blocking(move || status(&a)).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => failed(),
    }
}

/// The ComfyUI model a picture or clip request is for: the model it names,
/// or the default for its kind. None when ComfyUI isn't set up or the
/// model isn't one of its.
pub fn chosen(a: &Settings, body: &[u8], kind: &str) -> Option<&'static Recipe> {
    if a.comfy_url.trim().is_empty() {
        return None;
    }
    let req: Value = serde_json::from_slice(body).ok()?;
    let named = req.get("model").and_then(Value::as_str).filter(|m| !m.is_empty());
    let id = named.unwrap_or(if kind == "video" { &a.video_model } else { &a.image_model });
    recipe(id).filter(|r| r.kind == kind)
}

/// A size asked for as "1536x1024": kept between 256 and 2048 and rounded
/// to a multiple of 16; the recipe's own size when none is given.
fn size_of(req: &Value, r: &Recipe) -> (u32, u32) {
    let fit = |v: u32| (v.clamp(256, 2048) / 16) * 16;
    req.get("size")
        .and_then(Value::as_str)
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some((fit(w.trim().parse().ok()?), fit(h.trim().parse().ok()?))))
        .unwrap_or(r.size)
}

/// A new seed for every request: ComfyUI would hand back the last result for
/// a workflow it has already run.
fn seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (t.as_nanos() as u64 ^ (std::process::id() as u64) << 32) & ((1 << 53) - 1)
}

/// Runs blocking work (one call to ComfyUI) on a worker thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// A workflow queued on ComfyUI: stopped there if it's dropped before it
/// finished (the person went away).
struct Queued {
    base: String,
    id: String,
    finished: bool,
}

impl Drop for Queued {
    fn drop(&mut self) {
        if !self.finished {
            let (base, id) = (self.base.clone(), self.id.clone());
            std::thread::spawn(move || cancel(&base, &id));
        }
    }
}

/// Stops workflow `id`: interrupted when it's running (only when it's this
/// one, as older ComfyUI stops whatever runs), taken off the queue when
/// it's waiting.
pub fn cancel(base: &str, id: &str) {
    let running = get_json(&format!("{base}/queue"))
        .ok()
        .and_then(|q| q["queue_running"].as_array().map(|l| l.iter().any(|x| x[1] == id)))
        .unwrap_or(false);
    let post = |path: &str, body: Value| {
        let _ = agent(30).post(&format!("{base}{path}")).header("Content-Type", "application/json").send(body.to_string());
    };
    if running {
        post("/interrupt", json!({ "prompt_id": id }));
    }
    post("/queue", json!({ "delete": [id] }));
}

/// Runs a recipe on ComfyUI and returns the file, waiting as long as an hour
/// (a clip on a 16 GB card takes about half an hour). Dropping it (the
/// person went away) stops the workflow on ComfyUI.
async fn run(base: String, r: &'static Recipe, req: Value) -> Result<(Vec<u8>, String), String> {
    let text = req.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return Err("say what to make".into());
    }
    let (w, h) = size_of(&req, r);
    let graph = (r.build)(&text, w, h, seed());
    let b = base.clone();
    let (status, queued) = blocking(move || {
        let mut resp = agent(60)
            .post(&format!("{b}/prompt"))
            .header("Content-Type", "application/json")
            .send(json!({ "prompt": graph, "client_id": "xenify-server" }).to_string())
            .map_err(|e| format!("can't reach ComfyUI: {e}"))?;
        let status = resp.status().as_u16();
        let queued: Value = serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?)
            .map_err(|_| "ComfyUI sent something unexpected".to_string())?;
        Ok((status, queued))
    })
    .await?;
    if status != 200 {
        // ComfyUI says which node or file is the trouble.
        let why = queued["node_errors"]
            .as_object()
            .and_then(|m| m.values().next())
            .and_then(|n| n["errors"][0]["details"].as_str().or(n["errors"][0]["message"].as_str()))
            .or(queued["error"]["message"].as_str())
            .unwrap_or("it didn't take the workflow");
        return Err(format!("ComfyUI: {why}"));
    }
    let id = queued["prompt_id"].as_str().ok_or("ComfyUI didn't queue it")?.to_string();
    let mut job = Queued { base: base.clone(), id: id.clone(), finished: false };
    let start = Instant::now();
    let done = loop {
        let (b, i) = (base.clone(), id.clone());
        let h = blocking(move || get_json(&format!("{b}/history/{}", enc(&i)))).await?;
        if let Some(entry) = h.get(&id) {
            break entry.clone();
        }
        if start.elapsed() > Duration::from_secs(3600) {
            return Err("ComfyUI didn't finish within an hour".into());
        }
        tokio::time::sleep(Duration::from_millis(if start.elapsed() < Duration::from_secs(60) { 500 } else { 3000 }))
            .await;
    };
    job.finished = true;
    if done["status"]["status_str"].as_str() != Some("success") {
        let why = done["status"]["messages"]
            .as_array()
            .and_then(|m| m.iter().find(|x| x[0] == "execution_error"))
            .and_then(|x| x[1]["exception_message"].as_str())
            .unwrap_or("it stopped");
        return Err(format!("ComfyUI: {}", why.trim()));
    }
    let out = &done["outputs"][r.save];
    let item = ["images", "videos", "gifs"]
        .iter()
        .find_map(|k| out[*k].as_array().and_then(|l| l.first()))
        .ok_or("ComfyUI made no file")?;
    let name = item["filename"].as_str().unwrap_or("").to_string();
    let url = format!(
        "{base}/view?filename={}&subfolder={}&type={}",
        enc(&name),
        enc(item["subfolder"].as_str().unwrap_or("")),
        enc(item["type"].as_str().unwrap_or("output"))
    );
    let bytes = blocking(move || {
        agent(600)
            .get(&url)
            .call()
            .map_err(|e| format!("can't fetch the file from ComfyUI: {e}"))?
            .body_mut()
            .with_config()
            .limit(1 << 30)
            .read_to_vec()
            .map_err(|e| e.to_string())
    })
    .await?;
    Ok((bytes, name))
}

/// Answers a picture or clip request with ComfyUI, the way the apps already
/// take them: `{"data": [{"b64_json": …}]}`.
pub async fn make(a: &Settings, r: &'static Recipe, body: &[u8]) -> Response {
    let Ok(req) = serde_json::from_slice::<Value>(body) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "that isn't JSON");
    };
    match run(base(a), r, req).await {
        Ok((bytes, name)) => Json(json!({
            "created": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "model": r.id,
            "data": [{ "b64_json": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes), "file": name }],
        }))
        .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "comfyui", &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_has_the_nodes_it_changes() {
        for r in RECIPES {
            let g = (r.build)("a red kite", 832, 480, 7);
            assert!(g[r.save]["inputs"]["filename_prefix"].as_str().unwrap().starts_with("xenify/"), "{}", r.id);
            let s = g.to_string();
            assert!(s.contains("a red kite"), "{}: the words go in", r.id);
            for n in r.files {
                assert!(s.contains(n.file), "{}: the workflow loads {}", r.id, n.file);
            }
        }
        // The size and seed land where each template reads them.
        assert_eq!(wan("x", 832, 480, 7)["128:74"]["inputs"]["width"], 832);
        assert_eq!(minimax("x", 864, 480, 7)["140:131"]["inputs"]["height"], 480);
        assert_eq!(zimage("x", 1536, 1024, 9)["57:3"]["inputs"]["seed"], 9);
    }

    #[test]
    fn sizes_are_kept_sensible() {
        let r = recipe("flux.2-klein-4b").unwrap();
        assert_eq!(size_of(&json!({ "size": "1536x1024" }), r), (1536, 1024));
        assert_eq!(size_of(&json!({ "size": "5000x100" }), r), (2048, 256));
        assert_eq!(size_of(&json!({ "size": "1000x1000" }), r), (992, 992));
        assert_eq!(size_of(&json!({}), r), (1024, 1024));
        assert_eq!(size_of(&json!({}), recipe("minimax-h3").unwrap()), (864, 480));
    }

    #[test]
    fn both_ways_comfyui_lists_files() {
        let old = json!({ "UNETLoader": { "input": { "required": { "unet_name": [["a.safetensors", "sub/b.safetensors"], {}] } } } });
        let new = json!({ "UNETLoader": { "input": { "required": { "unet_name": ["COMBO", { "options": ["a.safetensors"] }] } } } });
        assert_eq!(offered(&old, "UNETLoader", "unet_name"), ["a.safetensors", "sub/b.safetensors"]);
        assert_eq!(offered(&new, "UNETLoader", "unet_name"), ["a.safetensors"]);
    }
}
