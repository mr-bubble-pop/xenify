//! Xenify Plan's engine in the page: one [`xenify_plan_core::Session`] per
//! open plan (tab), and the calls the screens make. The desktop app loads
//! this too, so every platform works the same way.

use serde_json::{Value as Json, json};
use xenify_plan_core::Session;

/// The open plans, by tab id.
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

    /// Editing together: steps made in any plan since last asked, to send
    /// to the server (JSON: [{id, batch}]).
    pub fn live_out(&mut self) -> String {
        let out: Vec<Json> =
            self.tabs.iter_mut().filter_map(|(id, s)| s.live_out().map(|b| json!({ "id": id, "batch": b }))).collect();
        Json::from(out).to_string()
    }

    pub fn session(&mut self) -> Result<&mut Session, String> {
        let front = self.front;
        self.tabs.iter_mut().find(|t| t.0 == front).map(|t| &mut t.1).ok_or_else(|| "no plan is open".to_string())
    }

    /// Every open plan (Resources and My work add them all up).
    pub fn all(&mut self) -> impl Iterator<Item = (u32, &mut Session)> {
        self.tabs.iter_mut().map(|(id, s)| (*id, s))
    }

    /// `name` with JSON `args` on the front plan (or the tab `tab`); JSON
    /// back, or `{"error": …}`. "all_tabs" runs the call on every plan.
    pub fn call(&mut self, name: &str, args: &str) -> String {
        let out = match serde_json::from_str::<Json>(args) {
            Err(e) => Err(e.to_string()),
            Ok(a) if name == "all_tabs" => {
                let inner = a["name"].as_str().unwrap_or("").to_string();
                let args = a.get("args").cloned().unwrap_or(Json::Null);
                let mut out = Vec::new();
                for (id, s) in self.all() {
                    out.push(match s.call(&inner, &args) {
                        Ok(v) => json!({ "tab": id, "out": v }),
                        Err(e) => json!({ "tab": id, "error": e }),
                    });
                }
                Ok(Json::Array(out))
            }
            Ok(a) => match a.get("tab").and_then(Json::as_u64) {
                Some(t) => self
                    .tabs
                    .iter_mut()
                    .find(|x| x.0 == t as u32)
                    .map(|x| &mut x.1)
                    .ok_or_else(|| "that plan isn't open".to_string())
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
    use xenify_plan_core::Session;

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

        /// A new, empty session in a new tab (the screens make the plan with "new"): its id.
        pub fn open_tab(&mut self) -> u32 {
            self.tabs.open(Session::new())
        }

        /// A `.xplan` file's bytes in a new tab: its id.
        pub fn open_file(&mut self, bytes: &[u8]) -> Result<u32, JsError> {
            let mut s = Session::new();
            s.open_file(bytes).map_err(|e| JsError::new(&e))?;
            Ok(self.tabs.open(s))
        }

        /// The front plan as a `.xplan` file.
        pub fn save_file(&mut self) -> Result<Vec<u8>, JsError> {
            self.tabs.session().and_then(|s| s.save_file()).map_err(|e| JsError::new(&e))
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
        t.call("new", r#"{"title":"Two"}"#);
        let id: Json = serde_json::from_str(&t.call("add_task", r#"{"title":"Write it"}"#)).unwrap();
        assert!(id["id"].is_string());
        let v: Json = serde_json::from_str(&t.call("view", "{}")).unwrap();
        assert_eq!(v["tasks"][0]["title"], "Write it");
        assert!(t.switch(a));
        let all: Json = serde_json::from_str(&t.call("all_tabs", r#"{"name":"view"}"#)).unwrap();
        assert_eq!(all.as_array().unwrap().len(), 2);
        let err: Json = serde_json::from_str(&t.call("nope", "{}")).unwrap();
        assert!(err["error"].is_string());
        t.close(a);
        assert_eq!(t.front(), b);
    }
}
