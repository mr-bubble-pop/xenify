//! Xenify Calc's engine in the page: one [`xenify_calc_core::Session`] per
//! open book (tab), and the calls the screens make. The desktop app loads
//! this too, so every platform works the same way.

use serde_json::{Value as Json, json};
use xenify_calc_core::Session;

/// The open books, by tab id.
#[derive(Default)]
pub struct Tabs {
    tabs: Vec<(u32, Session)>,
    front: u32,
    next: u32,
}

impl Tabs {
    pub fn open(&mut self, session: Session) -> u32 {
        self.next += 1;
        self.tabs.push((self.next, session));
        self.front = self.next;
        self.next
    }

    pub fn switch(&mut self, id: u32) -> bool {
        let there = self.tabs.iter().any(|t| t.0 == id);
        if there {
            self.front = id;
        }
        there
    }

    pub fn close(&mut self, id: u32) {
        self.tabs.retain(|t| t.0 != id);
        if self.front == id {
            self.front = self.tabs.last().map_or(0, |t| t.0);
        }
    }

    pub fn front(&self) -> u32 {
        self.front
    }

    /// Editing together: changes made in any book since last asked, to send
    /// to the server (JSON: [{id, batch}]).
    pub fn live_out(&mut self) -> String {
        let out: Vec<Json> =
            self.tabs.iter_mut().filter_map(|(id, s)| s.live_out().map(|b| json!({ "id": id, "batch": b }))).collect();
        Json::from(out).to_string()
    }

    pub fn session(&mut self) -> Result<&mut Session, String> {
        let front = self.front;
        self.tabs.iter_mut().find(|t| t.0 == front).map(|t| &mut t.1).ok_or_else(|| "no book is open".to_string())
    }

    /// `name` with JSON `args` on the front book; JSON back, or `{"error": …}`.
    pub fn call(&mut self, name: &str, args: &str) -> String {
        let out = match serde_json::from_str::<Json>(args) {
            Err(e) => Err(e.to_string()),
            // A book named by its tab (server documents: any open book), else the front one.
            Ok(a) => match a.get("tab").and_then(Json::as_u64) {
                Some(t) => self
                    .tabs
                    .iter_mut()
                    .find(|x| x.0 == t as u32)
                    .map(|x| &mut x.1)
                    .ok_or_else(|| "that book isn't open".to_string())
                    .and_then(|s| s.call(name, &a)),
                None => self.session().and_then(|s| s.call(name, &a)),
            },
        };
        match out {
            Ok(v) => v.to_string(),
            Err(e) => json!({ "error": e }).to_string(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::Tabs;
    use wasm_bindgen::prelude::*;
    use xenify_calc_core::Session;

    #[wasm_bindgen]
    pub struct Engine {
        tabs: Tabs,
    }

    #[wasm_bindgen]
    impl Engine {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Engine {
            Engine { tabs: Tabs::default() }
        }

        /// A new, empty book in a new tab: its id.
        pub fn open_tab(&mut self) -> u32 {
            self.tabs.open(Session::new())
        }

        /// A file's bytes (any kind Calc reads, told apart by `name`) in a new tab: its id.
        pub fn open_file(&mut self, bytes: &[u8], name: &str) -> Result<u32, JsError> {
            let s = Session::open_file(bytes, name).map_err(|e| JsError::new(&e))?;
            Ok(self.tabs.open(s))
        }

        /// The front book as a file of `format` (xcalc, xlsx, xlsm, csv, tsv; CSV and
        /// TSV: the sheet `sheet`). Saving in its own kind of file counts as saved.
        pub fn save_file(&mut self, format: &str, sheet: Option<u32>) -> Result<Vec<u8>, JsError> {
            let f: xenify_calc_core::Format =
                serde_json::from_value(serde_json::json!(format)).map_err(|e| JsError::new(&e.to_string()))?;
            self.tabs.session().and_then(|s| s.save_as(f, sheet)).map_err(|e| JsError::new(&e))
        }

        /// What Save writes for the front book: xcalc, xlsx, xlsm, csv or tsv.
        pub fn format(&mut self) -> String {
            self.tabs.session().map(|s| s.format.extension().to_string()).unwrap_or_default()
        }

        /// The front book as a `.xcalc` file for the recovery copy (it still
        /// counts as unsaved).
        pub fn recovery_copy(&mut self) -> Result<Vec<u8>, JsError> {
            let s = self.tabs.session().map_err(|e| JsError::new(&e))?;
            xenify_calc_core::file::save(&s.book).map_err(|e| JsError::new(&e))
        }

        pub fn switch_tab(&mut self, id: u32) -> bool {
            self.tabs.switch(id)
        }

        pub fn close_tab(&mut self, id: u32) {
            self.tabs.close(id)
        }

        pub fn front_tab(&self) -> u32 {
            self.tabs.front()
        }

        pub fn live_out(&mut self) -> String {
            self.tabs.live_out()
        }

        pub fn call(&mut self, name: &str, args: &str) -> String {
            self.tabs.call(name, args)
        }
    }

    impl Default for Engine {
        fn default() -> Self {
            Engine::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_and_calls() {
        let mut t = Tabs::default();
        let a = t.open(Session::new());
        let b = t.open(Session::new());
        assert_eq!(t.front(), b);
        let sheets: Json = serde_json::from_str(&t.call("sheets", "{}")).unwrap();
        let id = sheets[0]["id"].as_u64().unwrap();
        t.call("type", &json!({ "sheet": id, "row": 1, "col": 1, "input": "=1+2" }).to_string());
        let cell: Json =
            serde_json::from_str(&t.call("cell", &json!({ "sheet": id, "row": 1, "col": 1 }).to_string())).unwrap();
        assert_eq!(cell["text"], "3");
        assert!(t.switch(a));
        let cell: Json =
            serde_json::from_str(&t.call("cell", &json!({ "sheet": id, "row": 1, "col": 1 }).to_string())).unwrap();
        assert_eq!(cell["text"], "");
        let err: Json = serde_json::from_str(&t.call("nope", "{}")).unwrap();
        assert!(err["error"].is_string());
        t.close(a);
        assert_eq!(t.front(), b);
    }
}
