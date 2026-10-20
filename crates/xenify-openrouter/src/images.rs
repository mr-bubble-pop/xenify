//! Image generation through OpenRouter (models with image output), used to
//! make illustrations that are then traced into vectors.

use crate::AiError;
use base64::Engine;
use serde_json::{Value, json};

/// Default image model: cheap, and zero-data-retention compliant.
pub const DEFAULT_IMAGE_MODEL: &str = "google/gemini-3.1-flash-lite-image";

/// Image models to offer in settings (all have ZDR providers at the time of writing).
pub const IMAGE_PRESETS: &[crate::ModelPreset] = &[
    crate::ModelPreset { id: "google/gemini-3.1-flash-lite-image", note: "default · cheapest · ZDR" },
    crate::ModelPreset { id: "google/gemini-3.1-flash-image", note: "better detail · ZDR" },
    crate::ModelPreset { id: "google/gemini-2.5-flash-image", note: "ZDR" },
];

/// How the prompt is steered so the result traces cleanly into vectors.
pub fn vector_friendly_prompt(subject: &str) -> String {
    format!(
        "{subject}\n\nStyle: flat vector illustration, clean bold shapes, a limited palette of solid colours \
         (no gradients, no textures, no noise, no photographic detail), crisp edges, a single subject centred \
         on a plain white background with generous margins, no text, no watermark, no frame."
    )
}

/// The chat-completions body asking `model` for one image.
pub fn image_request(model: &str, prompt: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "modalities": ["image", "text"],
        "usage": {"include": true},
    })
}

/// Extracts the first image (decoded bytes) from a chat-completions response.
pub fn image_from_response(resp: &Value) -> Result<Vec<u8>, AiError> {
    let msg = &resp["choices"][0]["message"];
    let url = msg["images"][0]["image_url"]["url"].as_str().or_else(|| msg["images"][0]["url"].as_str()).ok_or_else(
        || {
            let said = msg["content"].as_str().unwrap_or("").chars().take(200).collect::<String>();
            AiError::BadResponse(if said.is_empty() {
                "the model didn't return an image".into()
            } else {
                format!("the model didn't return an image; it said: {said}")
            })
        },
    )?;
    let data = url
        .split_once(";base64,")
        .map(|(_, d)| d)
        .ok_or_else(|| AiError::BadResponse("image wasn't a base64 data URL".into()))?;
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| AiError::BadResponse(format!("bad image data: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openrouter_image_responses() {
        let png = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png));
        let resp = json!({"choices": [{"message": {"role": "assistant", "content": "", "images": [{"type": "image_url", "image_url": {"url": url}}]}}]});
        assert_eq!(image_from_response(&resp).unwrap(), png);
        let none = json!({"choices": [{"message": {"content": "I can't draw that"}}]});
        let e = image_from_response(&none).unwrap_err().to_string();
        assert!(e.contains("I can't draw that"), "{e}");
        let body = image_request(DEFAULT_IMAGE_MODEL, "a cat");
        assert_eq!(body["modalities"], json!(["image", "text"]));
    }
}
