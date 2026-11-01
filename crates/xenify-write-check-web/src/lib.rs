//! Xenify Write's checker in the browser, in a Web Worker of its own so the
//! page never waits for it: the screens hand it the paragraphs to check
//! (the document engine's `proof_request`) and get the findings back. It
//! also answers the word Look up, and runs the mistake spotter once the
//! model has been downloaded (`set_spotter`).

use serde_json::{Value, json};
use std::cell::RefCell;
use xenify_write_host::proof::{Proof, Request};

thread_local! {
    static PROOF: RefCell<Option<Proof>> = const { RefCell::new(None) };
}

fn with_proof<R>(f: impl FnOnce(&Proof) -> R) -> R {
    PROOF.with(|p| {
        let mut p = p.borrow_mut();
        f(p.get_or_insert_with(Proof::new))
    })
}

/// One call: `check` ({request} → issues), `look_up` ({word}), `words`,
/// `set_words` ({words}) and `spotter_status`.
pub fn call_json(name: &str, args: &str) -> Result<String, String> {
    let args: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let out = match name {
        "check" => {
            let req: Request =
                serde_json::from_value(args.get("request").cloned().unwrap_or_default()).map_err(|e| e.to_string())?;
            json!(with_proof(|p| p.check(req)))
        }
        "look_up" => xenify_write_host::proof::look_up(args.get("word").and_then(Value::as_str).unwrap_or_default()),
        "words" => json!(with_proof(|p| p.words())),
        "set_words" => {
            let words: Vec<String> =
                serde_json::from_value(args.get("words").cloned().unwrap_or_default()).unwrap_or_default();
            with_proof(|p| p.set_words(words));
            Value::Null
        }
        "spotter_status" => json!(with_proof(|p| p.spotter_status())),
        _ => return Err(format!("the checker has no call {name}")),
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

    /// Loads the mistake spotter from the bytes of its three files.
    #[wasm_bindgen]
    pub fn set_spotter(config: &[u8], tokenizer: &[u8], weights: Vec<u8>) {
        super::with_proof(|p| p.set_spotter(config, tokenizer, weights));
    }
}
