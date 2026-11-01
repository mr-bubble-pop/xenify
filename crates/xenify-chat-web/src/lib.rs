//! Xenify Chat in the browser: the same core as the desktop app (signing
//! in, meetings, your calendar and address books on the server) as
//! WebAssembly. The page's
//! `tauri-shim.js` keeps the WebSocket and does HTTP, and hands everything
//! to these exports; the screens are the desktop app's.

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use xenify_chat_core::{Core, Details, Fetch, Host};
    use xenify_chat_proto::{ToApp, ToServer};

    /// What the page gives: functions to tell the screens, keep the
    /// calendar and address books, do HTTP and send on the WebSocket.
    struct Page {
        emit: js_sys::Function,
        save: js_sys::Function,
        fetch: js_sys::Function,
        send: js_sys::Function,
        /// `invite(json)`: a meeting's email invitations, sent by the server.
        invite: Option<js_sys::Function>,
    }

    struct Web {
        page: Rc<Page>,
        server: RefCell<String>,
        tz: chrono_tz::Tz,
        /// Messages for the server from the core, sent after what it's doing.
        queue: Rc<RefCell<Vec<ToServer>>>,
        /// The email addresses the server keeps for you.
        mail: RefCell<Vec<String>>,
    }

    impl Host for Web {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            let _ = self.page.emit.call2(&JsValue::NULL, &event.into(), &payload.to_string().into());
        }
        fn tell(&self, m: ToServer) {
            self.queue.borrow_mut().push(m);
        }
        fn save(&self, what: &str, json: String) {
            let _ = self.page.save.call2(&JsValue::NULL, &what.into(), &json.into());
        }
        fn now(&self) -> i64 {
            js_sys::Date::now() as i64
        }
        fn tz(&self) -> chrono_tz::Tz {
            self.tz
        }
        fn server(&self) -> String {
            self.server.borrow().clone()
        }
        fn mail_accounts(&self) -> Vec<String> {
            self.mail.borrow().clone()
        }
        // The server sends the invitations (from the email it keeps for you).
        fn email_guests(&self, d: &Details, o: &str, p: &[String], cancel: bool) {
            let Some(f) = &self.page.invite else { return };
            let args = serde_json::json!({ "details": d, "organiser": o, "people": p, "cancel": cancel });
            let _ = f.call1(&JsValue::NULL, &args.to_string().into());
        }
    }

    /// HTTP through the page, as someone (`token`).
    struct PageFetch {
        page: Rc<Page>,
        token: String,
    }

    impl Fetch for PageFetch {
        async fn request(
            &self,
            token: &str,
            path: &str,
            body: Option<serde_json::Value>,
        ) -> Result<serde_json::Value, String> {
            let token = if token.is_empty() { self.token.as_str() } else { token };
            let body = body.map(|b| JsValue::from(b.to_string())).unwrap_or(JsValue::UNDEFINED);
            let promise = self
                .page
                .fetch
                .call3(&JsValue::NULL, &token.into(), &path.into(), &body)
                .map_err(|e| e.as_string().unwrap_or_else(|| "Can't reach the server.".into()))?;
            let text = JsFuture::from(js_sys::Promise::from(promise))
                .await
                .map_err(|e| e.as_string().unwrap_or_else(|| "Can't reach the server.".into()))?;
            let text = text.as_string().unwrap_or_default();
            if text.is_empty() {
                return Ok(serde_json::Value::Null);
            }
            serde_json::from_str(&text).map_err(|_| "The server sent something unexpected.".to_string())
        }
    }

    struct Chat {
        core: Core<Web>,
        page: Rc<Page>,
        queue: Rc<RefCell<Vec<ToServer>>>,
    }

    thread_local! {
        static CHAT: RefCell<Option<Rc<Chat>>> = const { RefCell::new(None) };
    }

    fn chat() -> Result<Rc<Chat>, JsValue> {
        CHAT.with(|c| c.borrow().clone()).ok_or_else(|| JsValue::from_str("not started"))
    }

    fn get(o: &JsValue, k: &str) -> Result<js_sys::Function, JsValue> {
        js_sys::Reflect::get(o, &k.into())?.dyn_into::<js_sys::Function>()
    }

    /// Starts: `page` has `emit(event, json)`, `save(what, json)`,
    /// `fetch(token, path, body?) → Promise<text>` and `send(json)`; `tz` is
    /// the browser's time zone; `calendar` and `contacts` what `save` kept.
    #[wasm_bindgen]
    pub fn start(page: JsValue, tz: String, calendar: String, contacts: String) -> Result<(), JsValue> {
        let page = Rc::new(Page {
            emit: get(&page, "emit")?,
            save: get(&page, "save")?,
            fetch: get(&page, "fetch")?,
            send: get(&page, "send")?,
            invite: get(&page, "invite").ok(),
        });
        let tz = tz.parse().unwrap_or(chrono_tz::UTC);
        let queue = Rc::new(RefCell::new(Vec::new()));
        let host = Web {
            page: page.clone(),
            server: RefCell::new(String::new()),
            tz,
            queue: queue.clone(),
            mail: RefCell::new(Vec::new()),
        };
        let core = Core::new(host, &calendar, &contacts);
        core.cal.set_tz(tz);
        let chat = Rc::new(Chat { core, page, queue });
        CHAT.with(|c| *c.borrow_mut() = Some(chat));
        Ok(())
    }

    /// Signed in on `server` (a new connection follows).
    #[wasm_bindgen]
    pub fn session(server: String) -> Result<(), JsValue> {
        *chat()?.core.host.server.borrow_mut() = server;
        Ok(())
    }

    /// Signs in (or, with a link's `<id>.<secret>`, picks the password):
    /// answers `{ session, remember }`,
    /// or the second step (`{ ticket, methods, setup, setup_methods }`), as
    /// JSON. `remember`: the token from an earlier second step.
    #[wasm_bindgen]
    pub async fn sign_in(
        server: String,
        username: String,
        password: String,
        link: String,
        device: String,
        remember: String,
    ) -> Result<String, JsValue> {
        let c = chat()?;
        *c.core.host.server.borrow_mut() = server;
        let fetch = PageFetch { page: c.page.clone(), token: String::new() };
        let answer = xenify_chat_core::account::account(&fetch, &username, &password, &link, &device, &remember)
            .await
            .map_err(|e| JsValue::from_str(&e))?;
        Ok(serde_json::to_string(&answer).unwrap_or_default())
    }

    /// The second step and two-step sign-in's settings, as the desktop
    /// app's commands of the same names (`args` and the answer as JSON).
    /// `token`: the session, for the settings ("" while signing in).
    #[wasm_bindgen]
    pub async fn second_step(name: String, token: String, args: String) -> Result<String, JsValue> {
        use xenify_chat_core::account as acc;
        let c = chat()?;
        let fetch = PageFetch { page: c.page.clone(), token };
        let a: serde_json::Value = serde_json::from_str(&args).unwrap_or_default();
        let s = |k: &str| a[k].as_str().unwrap_or("").to_string();
        let remember = a["remember"].as_bool().unwrap_or(false);
        let session =
            |(s, r): (xenify_chat_proto::Session, Option<String>)| serde_json::json!({ "session": s, "remember": r });
        let out = match name.as_str() {
            "second_code" => acc::second_code(&fetch, &s("ticket"), &s("method"), &s("code"), remember, &s("device"))
                .await
                .map(session),
            "second_email" => acc::email_code(&fetch, &s("ticket")).await.map(|_| serde_json::Value::Null),
            "mfa_setup_app" => {
                acc::setup_app(&fetch, &s("ticket")).await.map(|k| serde_json::to_value(k).unwrap_or_default())
            }
            "mfa_setup_email" => acc::setup_email(&fetch, &s("ticket")).await.map(|_| serde_json::Value::Null),
            "mfa_setup_finish" => acc::setup_finish(&fetch, &s("ticket"), &s("method"), &s("code"), remember)
                .await
                .map(|(codes, signed)| serde_json::json!({ "backup_codes": codes, "signed_in": signed.map(session) })),
            "mfa_state" => acc::second_step(&fetch).await.map(|st| serde_json::to_value(st).unwrap_or_default()),
            "mfa_off" => acc::second_step_off(&fetch, &s("password")).await.map(|_| serde_json::Value::Null),
            "mfa_backup" => acc::new_backup_codes(&fetch, &s("password")).await.map(|c| serde_json::json!(c)),
            _ => Err(format!("unknown call “{name}”")),
        };
        out.map(|v| v.to_string()).map_err(|e| JsValue::from_str(&e))
    }

    /// What `username`'s code (or an older link's `<id>.<secret>`) is
    /// for, as JSON (the server's `LinkInfo`).
    #[wasm_bindgen]
    pub async fn link_info(link: String, username: String) -> Result<String, JsValue> {
        let c = chat()?;
        let fetch = PageFetch { page: c.page.clone(), token: String::new() };
        let info =
            xenify_chat_core::account::link_info(&fetch, &link, &username).await.map_err(|e| JsValue::from_str(&e))?;
        Ok(serde_json::to_string(&info).unwrap_or_default())
    }

    /// A pasted or opened link the server made: `[server, "<id>.<secret>"]`
    /// as JSON, or "" when it isn't one.
    #[wasm_bindgen]
    pub fn parse_link(text: String) -> String {
        xenify_chat_core::account::parse_link(&text)
            .map(|l| serde_json::to_string(&l).unwrap_or_default())
            .unwrap_or_default()
    }

    fn send_now(c: &Chat, m: &ToServer) {
        let _ = c.page.send.call1(&JsValue::NULL, &serde_json::to_string(m).unwrap_or_default().into());
    }

    /// Sends what's waiting.
    fn flush(c: &Chat) {
        loop {
            let waiting: Vec<ToServer> = std::mem::take(&mut *c.queue.borrow_mut());
            if waiting.is_empty() {
                break;
            }
            for m in waiting {
                send_now(c, &m);
            }
        }
    }

    /// From the screens: a message for the server.
    #[wasm_bindgen]
    pub fn send(json: String) -> Result<(), JsValue> {
        let c = chat()?;
        let m: ToServer = serde_json::from_str(&json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        c.queue.borrow_mut().push(m);
        flush(&c);
        Ok(())
    }

    /// From the server (one WebSocket message): sorted (meetings,
    /// calendar…) and shown.
    #[wasm_bindgen]
    pub fn heard(text: String) -> Result<(), JsValue> {
        let c = chat()?;
        let Ok(m) = serde_json::from_str::<ToApp>(&text) else { return Ok(()) };
        if let Some(m) = c.core.incoming(m) {
            c.core.host.emit("to_app", serde_json::to_value(m).unwrap_or_default());
        }
        flush(&c);
        Ok(())
    }

    /// The core's commands (calendar, meetings, address books, out of
    /// office): the answer as JSON, or `undefined` when it isn't one.
    #[wasm_bindgen]
    pub fn call(name: String, args: String) -> Result<JsValue, JsValue> {
        let c = chat()?;
        let a: serde_json::Value = serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
        let out = match c.core.call(&name, &a) {
            None => Ok(JsValue::UNDEFINED),
            Some(Ok(v)) => Ok(JsValue::from(v.to_string())),
            Some(Err(e)) => Err(JsValue::from_str(&e)),
        };
        flush(&c);
        out
    }

    /// The email addresses the server keeps for you (JSON list), for
    /// meeting invitations.
    #[wasm_bindgen]
    pub fn mail_accounts(json: String) -> Result<(), JsValue> {
        *chat()?.core.host.mail.borrow_mut() = serde_json::from_str(&json).unwrap_or_default();
        Ok(())
    }

    /// Whom you just emailed (`[[name, email]…]`), for My contacts.
    #[wasm_bindgen]
    pub fn emailed(json: String) -> Result<(), JsValue> {
        let people: Vec<(String, String)> = serde_json::from_str(&json).unwrap_or_default();
        chat()?.core.emailed(&people);
        Ok(())
    }

    /// Someone invited by email answered meeting `uid`.
    #[wasm_bindgen]
    pub fn guest_answered(uid: String, email: String, answer: String) -> Result<(), JsValue> {
        let c = chat()?;
        c.core.guest_answered(&uid, &email, &answer);
        flush(&c);
        Ok(())
    }

    /// The meeting with this invitation `uid` is in your calendar.
    #[wasm_bindgen]
    pub fn in_calendar(uid: String) -> Result<bool, JsValue> {
        Ok(chat()?.core.cal.by_uid(&uid).ok().flatten().is_some())
    }

    /// An invitation answered: the server's `{ event }` goes in your
    /// calendar (replacing the same meeting), `{ declined: uid }` comes out.
    #[wasm_bindgen]
    pub fn invite_answered(json: String) -> Result<(), JsValue> {
        let c = chat()?;
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        let r = if let Some(uid) = v["declined"].as_str() {
            c.core.cal.delete_uid(uid)
        } else if let Ok(mut e) = serde_json::from_value::<xenify_chat_core::Event>(v["event"].clone()) {
            e.id = c.core.cal.by_uid(&e.ical_uid).ok().flatten().map(|x| x.id).unwrap_or(0);
            c.core.cal.save(&e).map(|_| ())
        } else {
            Ok(())
        };
        r.map_err(|e| JsValue::from_str(&e))?;
        c.core.calendar_changed();
        flush(&c);
        Ok(())
    }

    /// Takes a cancelled meeting out of your calendar.
    #[wasm_bindgen]
    pub fn forget_invite(uid: String) -> Result<(), JsValue> {
        let c = chat()?;
        c.core.cal.delete_uid(&uid).map_err(|e| JsValue::from_str(&e))?;
        c.core.calendar_changed();
        flush(&c);
        Ok(())
    }
}
