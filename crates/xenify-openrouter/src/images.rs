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
         with generous margins on a plain background of one solid colour that the subject doesn't use (not white \
         if any part of the subject is white), no text, no watermark, no frame."
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

/// The chat-completions body asking `model` to change a picture as `change` says: `image` (a data
/// URL), only where `mask` (a data URL, white = change) is white when there is one, with reference
/// pictures (data URL, what it shows). The answer is the picture and a sentence or two on what changed.
pub fn edit_request(model: &str, change: &str, image: &str, mask: Option<&str>, refs: &[(String, String)]) -> Value {
    let mut text = format!(
        "Edit the first picture. The change: {change}\n\nKeep everything the change doesn't mention exactly as it is: \
         the framing, the size and shape of the picture, the people with their faces and poses, the background, \
         the lighting and the style."
    );
    if mask.is_some() {
        text.push_str(
            " The second picture is a mask the same size as the first: change only the part that is white in the \
             mask, and leave the black part exactly as it is.",
        );
    }
    if !refs.is_empty() {
        text.push_str(" The pictures after that are references for the change (labelled below): use what they show.");
    }
    text.push_str(
        " Answer with the edited picture (the whole picture, the same size and shape as the first), then one or \
         two short sentences saying what you changed.",
    );
    let pic = |url: &str| json!({"type": "image_url", "image_url": {"url": url}});
    let mut content =
        vec![json!({"type": "text", "text": text}), json!({"type": "text", "text": "The picture:"}), pic(image)];
    if let Some(m) = mask {
        content.push(json!({"type": "text", "text": "The mask (white: change this part):"}));
        content.push(pic(m));
    }
    for (i, (url, note)) in refs.iter().enumerate() {
        let label = if note.trim().is_empty() {
            format!("Reference {}:", i + 1)
        } else {
            format!("Reference {}: {}", i + 1, note.trim())
        };
        content.push(json!({"type": "text", "text": label}));
        content.push(pic(url));
    }
    json!({
        "model": model,
        "messages": [{"role": "user", "content": content}],
        "modalities": ["image", "text"],
        "usage": {"include": true},
    })
}

/// The chat-completions body asking `model` for a selection mask of `what` in `image` (a data URL): the picture
/// the same size, white where `what` is and black everywhere else.
pub fn mask_request(model: &str, what: &str, image: &str) -> Value {
    let text = format!(
        "Make a black-and-white mask of this picture: paint {what} pure white (all of it, out to its exact edges,          including hair, fingers or straps when they are part of it) and everything else pure black. Keep the same          size, shape and framing as the picture; no other colours, no grey, no text, no outline."
    );
    json!({
        "model": model,
        "messages": [{"role": "user", "content": [{"type": "text", "text": text}, {"type": "image_url", "image_url": {"url": image}}]}],
        "modalities": ["image", "text"],
        "usage": {"include": true},
    })
}

/// What the model said beside its picture (an edit's explanation), as plain text.
pub fn reply_text(resp: &Value) -> String {
    let c = &resp["choices"][0]["message"]["content"];
    match c {
        Value::String(s) => s.trim().to_string(),
        Value::Array(parts) => {
            parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(" ").trim().to_string()
        }
        _ => String::new(),
    }
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
        let edit = edit_request(
            DEFAULT_IMAGE_MODEL,
            "a red hat",
            "data:image/png;base64,AA",
            Some("data:image/png;base64,BB"),
            &[("data:image/png;base64,CC".into(), "the hat".into())],
        );
        let parts = edit["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts.iter().filter(|p| p["type"] == "image_url").count(), 3);
        assert!(parts.iter().any(|p| p["text"] == "Reference 1: the hat"));
        let said = json!({"choices": [{"message": {"content": [{"type": "text", "text": "Added a red hat."}]}}]});
        assert_eq!(reply_text(&said), "Added a red hat.");
    }
}
