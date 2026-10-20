//! Blocking HTTP to OpenRouter. Call from a background thread.

use crate::{AiError, AiSettings};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct OpenRouter {
    agent: ureq::Agent,
    api_key: String,
    base_url: String,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // Read error bodies ourselves: they carry OpenRouter's explanation.
        .http_status_as_error(false)
        .build()
        .into()
}

impl OpenRouter {
    pub fn new(settings: &AiSettings) -> Result<Self, AiError> {
        let api_key = settings.resolve_api_key().ok_or(AiError::NoApiKey)?;
        Ok(Self { agent: agent(Duration::from_secs(180)), api_key, base_url: settings.base_url.clone() })
    }

    /// POSTs an OpenAI-compatible chat completion request.
    pub fn chat(&self, body: &Value) -> Result<Value, AiError> {
        let mut resp = self
            .agent
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://github.com/xenify-app")
            .header("X-Title", "Xenify")
            .send(serde_json::to_vec(body).expect("request serialises"))
            .map_err(|e| AiError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .with_config()
            .limit(32 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| AiError::Network(e.to_string()))?;
        let json: Value = serde_json::from_str(&text).map_err(|_| AiError::BadResponse(truncate(&text, 300)))?;
        if status >= 400 || json.get("error").is_some() {
            return Err(api_error(status.max(400), &text, body));
        }
        Ok(json)
    }
}

impl OpenRouter {
    /// Like [`Self::chat`], but streams the reply: `on_text` gets each piece of
    /// the assistant's text as it arrives. Returns the same shape as a
    /// non-streamed response (`choices[0].message` with content and tool calls,
    /// plus `usage`), so callers don't need to care.
    pub fn chat_stream(&self, body: &Value, on_text: &mut dyn FnMut(&str)) -> Result<Value, AiError> {
        use std::io::BufRead;
        let mut body = body.clone();
        body["stream"] = Value::Bool(true);
        let mut resp = self
            .agent
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://github.com/xenify-app")
            .header("X-Title", "Xenify")
            .send(serde_json::to_vec(&body).expect("request serialises"))
            .map_err(|e| AiError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status >= 400 {
            let text = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(api_error(status, &text, &body));
        }
        let reader = std::io::BufReader::new(resp.body_mut().as_reader());
        let mut stream = StreamAssembler::default();
        for line in reader.lines() {
            let line = line.map_err(|e| AiError::Network(e.to_string()))?;
            if stream.feed(&line, on_text)? {
                break;
            }
        }
        Ok(stream.finish())
    }
}

/// Builds a chat completion out of server-sent-event chunks.
#[derive(Default)]
pub(crate) struct StreamAssembler {
    content: String,
    /// (id, name, arguments) by tool-call index.
    tools: Vec<(String, String, String)>,
    finish: Option<Value>,
    usage: Option<Value>,
}

impl StreamAssembler {
    /// Handles one line; returns true at the end of the stream.
    pub(crate) fn feed(&mut self, line: &str, on_text: &mut dyn FnMut(&str)) -> Result<bool, AiError> {
        // Blank lines separate events; `:` lines are keep-alive comments.
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { return Ok(false) };
        if data == "[DONE]" {
            return Ok(true);
        }
        let chunk: Value = serde_json::from_str(data).map_err(|_| AiError::BadResponse(truncate(data, 300)))?;
        if let Some(e) = chunk.get("error") {
            let message = e["message"].as_str().unwrap_or("error during streaming").to_string();
            return Err(AiError::Api { status: e["code"].as_u64().unwrap_or(500) as u16, message });
        }
        if chunk.get("usage").is_some_and(|u| !u.is_null()) {
            self.usage = Some(chunk["usage"].clone());
        }
        let choice = &chunk["choices"][0];
        if !choice["finish_reason"].is_null() {
            self.finish = Some(choice["finish_reason"].clone());
        }
        let delta = &choice["delta"];
        if let Some(t) = delta["content"].as_str()
            && !t.is_empty()
        {
            self.content.push_str(t);
            on_text(t);
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let i = call["index"].as_u64().unwrap_or(self.tools.len().saturating_sub(1) as u64) as usize;
            if self.tools.len() <= i {
                self.tools.resize(i + 1, Default::default());
            }
            let t = &mut self.tools[i];
            if let Some(id) = call["id"].as_str() {
                t.0 = id.to_string();
            }
            if let Some(n) = call["function"]["name"].as_str() {
                t.1.push_str(n);
            }
            if let Some(a) = call["function"]["arguments"].as_str() {
                t.2.push_str(a);
            }
        }
        Ok(false)
    }

    pub(crate) fn finish(self) -> Value {
        let mut message = serde_json::json!({"role": "assistant", "content": self.content});
        if self.content.is_empty() {
            message["content"] = Value::Null;
        }
        let calls: Vec<Value> = self
            .tools
            .into_iter()
            .filter(|t| !t.1.is_empty())
            .enumerate()
            .map(|(i, (id, name, args))| {
                let id = if id.is_empty() { format!("call_{i}") } else { id };
                serde_json::json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})
            })
            .collect();
        if !calls.is_empty() {
            message["tool_calls"] = Value::Array(calls);
        }
        serde_json::json!({
            "choices": [{"message": message, "finish_reason": self.finish.unwrap_or(Value::Null)}],
            "usage": self.usage.unwrap_or(Value::Null),
        })
    }
}

/// OpenRouter's error message, made friendlier for privacy-guardrail failures.
fn api_error(status: u16, text: &str, body: &Value) -> AiError {
    let json: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mut message = json["error"]["message"].as_str().map_or_else(|| truncate(text, 300), str::to_string);
    if message.contains("guardrail") || message.contains("data policy") {
        let model = body["model"].as_str().unwrap_or("this model");
        message = format!(
            "{model} has no provider allowed by your OpenRouter privacy settings (e.g. Zero Data Retention). \
             Pick a model marked ZDR in AI Settings, or change the guardrail at openrouter.ai."
        );
    }
    AiError::Api { status, message }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// A model from OpenRouter's public catalogue.
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    /// USD per million tokens.
    pub prompt_price: f64,
    pub completion_price: f64,
    pub tools: bool,
    pub vision: bool,
    pub context: u64,
    /// Has at least one zero-data-retention provider.
    pub zdr: bool,
}

fn get_json(url: &str) -> Result<Value, AiError> {
    let mut resp = agent(Duration::from_secs(30)).get(url).call().map_err(|e| AiError::Network(e.to_string()))?;
    let text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| AiError::Network(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| AiError::BadResponse(e.to_string()))
}

/// Downloads the model catalogue (no API key needed), marking which models
/// have zero-data-retention providers.
pub fn fetch_models(base_url: &str) -> Result<Vec<ModelInfo>, AiError> {
    let json = get_json(&format!("{base_url}/models"))?;
    let zdr: std::collections::HashSet<String> = get_json(&format!("{base_url}/endpoints/zdr"))
        .ok()
        .and_then(|z| {
            z["data"].as_array().map(|a| a.iter().filter_map(|e| e["model_id"].as_str().map(str::to_string)).collect())
        })
        .unwrap_or_default();
    let price = |v: &Value| v.as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) * 1e6;
    let mut models: Vec<ModelInfo> = json["data"]
        .as_array()
        .ok_or_else(|| AiError::BadResponse("no model list".into()))?
        .iter()
        .filter_map(|m| {
            let has = |list: &Value, x: &str| list.as_array().is_some_and(|a| a.iter().any(|v| v == x));
            let prompt_price = price(&m["pricing"]["prompt"]);
            // Router pseudo-models have negative prices.
            (prompt_price >= 0.0).then(|| ModelInfo {
                id: m["id"].as_str().unwrap_or_default().to_string(),
                name: m["name"].as_str().unwrap_or_default().to_string(),
                prompt_price,
                completion_price: price(&m["pricing"]["completion"]),
                tools: has(&m["supported_parameters"], "tools"),
                vision: has(&m["architecture"]["input_modalities"], "image"),
                context: m["context_length"].as_u64().unwrap_or(0),
                zdr: zdr.contains(m["id"].as_str().unwrap_or_default()),
            })
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_streamed_text_and_tool_calls() {
        let lines = [
            ": OPENROUTER PROCESSING",
            r#"data: {"choices":[{"delta":{"role":"assistant","content":"Hel"}}]}"#,
            "",
            r#"data: {"choices":[{"delta":{"content":"lo"}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"add_","arguments":"{\"a\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"layer","arguments":":1}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","function":{"name":"get_scene","arguments":""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":7,"cost":0.001}}"#,
            "data: [DONE]",
        ];
        let mut s = StreamAssembler::default();
        let mut seen = String::new();
        for l in lines {
            if s.feed(l, &mut |t| seen.push_str(t)).unwrap() {
                break;
            }
        }
        assert_eq!(seen, "Hello");
        let r = s.finish();
        let m = &r["choices"][0]["message"];
        assert_eq!(m["content"], "Hello");
        assert_eq!(m["tool_calls"][0]["function"]["name"], "add_layer");
        assert_eq!(m["tool_calls"][0]["function"]["arguments"], "{\"a\":1}");
        assert_eq!(m["tool_calls"][1]["id"], "c2");
        assert_eq!(r["usage"]["cost"], 0.001);
    }

    #[test]
    fn errors_mid_stream_are_reported() {
        let mut s = StreamAssembler::default();
        let e = s.feed(r#"data: {"error":{"message":"overloaded","code":502}}"#, &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("overloaded"));
    }
}
