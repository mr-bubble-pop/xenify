//! Xenify Write in the browser: the same document calls as the desktop app
//! (`call(name, args)`, JSON in and out), answered by WebAssembly. The
//! screens are the desktop app's own `ui/` files.
//!
//! Spelling and grammar aren't in here, so this first download stays small:
//! they run in a worker of their own (`xenify-write-check-web`), which is
//! handed what to check (`proof_request`) and whose findings come back
//! (`set_issues`).

use serde_json::{Value, json};
use std::cell::RefCell;
use xenify_write_host::docs::Docs;

thread_local! {
    static DOCS: RefCell<Option<Docs>> = const { RefCell::new(None) };
}

fn with_docs<R>(f: impl FnOnce(&mut Docs) -> R) -> R {
    DOCS.with(|d| {
        let mut d = d.borrow_mut();
        f(d.get_or_insert_with(|| Docs::new(&[])))
    })
}

/// One call from the screens (the desktop's `engine` command), plus the
/// two the checker's worker needs.
pub fn call_json(name: &str, args: &str) -> Result<String, String> {
    let args: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let out = match name {
        "proof_request" => {
            let (id, req) = with_docs(|d| d.proof_request());
            json!({ "doc": id, "request": req })
        }
        "set_issues" => {
            let doc = args.get("doc").and_then(Value::as_u64).unwrap_or_default() as u32;
            let texts: Vec<(xenify_wp_doc::NodeId, String)> =
                serde_json::from_value(args.get("texts").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
            let issues =
                serde_json::from_value(args.get("issues").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
            with_docs(|d| d.set_issues(doc, &texts, issues));
            Value::Null
        }
        _ => with_docs(|d| d.call(name, &args))?,
    };
    Ok(out.to_string())
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn call(name: &str, args: &str) -> Result<String, JsValue> {
        super::call_json(name, args).map_err(|e| JsValue::from_str(&e))
    }

    /// The fonts left out of this download (`fonts.bin`, made by
    /// scripts/build-write-web.sh): STIX Two Math, the four CMU Serif files
    /// and the packed emoji set, each as a little-endian u32 length and its
    /// bytes. Documents are laid out again with them.
    #[wasm_bindgen]
    pub fn late_fonts(pack: &[u8]) -> Result<(), JsValue> {
        let mut parts = Vec::new();
        let mut rest = pack;
        while rest.len() >= 4 {
            let n = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            let part = rest.get(4..4 + n).ok_or_else(|| JsValue::from_str("fonts.bin is cut short"))?;
            parts.push(part.to_vec());
            rest = &rest[4 + n..];
        }
        if parts.len() != 6 {
            return Err(JsValue::from_str("fonts.bin should hold 6 files"));
        }
        let emoji = parts.pop().unwrap_or_default();
        let symbol = parts.remove(0);
        xenify_wp_layout::provide_late_fonts(symbol, parts, emoji);
        super::with_docs(|d| d.fonts_arrived());
        Ok(())
    }
}
