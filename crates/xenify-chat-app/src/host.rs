//! The desktop app as the core's host (`xenify_chat_core::Host`): the
//! screens hear Tauri events, messages go through the connection, the
//! calendar and address books are kept as files in the data folder, and
//! email comes from your email accounts.

use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use xenify_chat_core::{Core, Details, Fetch, Host};
use xenify_chat_proto::{ToApp, ToServer};

pub struct Desktop {
    app: AppHandle,
}

/// The core, as the desktop app keeps it (in Tauri's state).
pub type Chat = Core<Desktop>;

fn kept_path(what: &str) -> PathBuf {
    xenify_wp_check::data_dir().join(format!("{what}.json"))
}

/// The core, with the calendar and address books kept on this computer
/// (moved over from the SQLite files of earlier versions the first time).
pub fn chat(app: &AppHandle) -> Chat {
    let read = |what: &str| std::fs::read_to_string(kept_path(what)).ok();
    let calendar = read("calendar").or_else(|| crate::calendar::migrate().and_then(|d| serde_json::to_string(&d).ok()));
    let contacts =
        read("contacts").or_else(|| crate::calendar::migrate_contacts().and_then(|d| serde_json::to_string(&d).ok()));
    let core =
        Core::new(Desktop { app: app.clone() }, calendar.as_deref().unwrap_or(""), contacts.as_deref().unwrap_or(""));
    core.cal.set_tz(crate::calendar::local_tz());
    // Keep the moved-over copies at once.
    for (what, json) in [("calendar", calendar), ("contacts", contacts)] {
        if let Some(json) = json
            && !kept_path(what).exists()
        {
            core.host.save(what, json);
        }
    }
    core
}

impl Host for Desktop {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.app.emit(event, payload);
    }

    fn tell(&self, m: ToServer) {
        let _ = crate::net::send(self.app.state::<crate::net::Net>(), m);
    }

    /// Written whole, only you can read it, replacing the old copy at once.
    fn save(&self, what: &str, json: String) {
        let path = kept_path(what);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        if let Ok(mut f) = o.open(&tmp) {
            use std::io::Write;
            if f.write_all(json.as_bytes()).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
    }

    fn tz(&self) -> chrono_tz::Tz {
        crate::calendar::local_tz()
    }

    fn server(&self) -> String {
        self.app.state::<crate::net::Net>().creds().0
    }

    fn mail_accounts(&self) -> Vec<String> {
        if crate::servermail::on(&self.app).is_some() {
            return crate::servermail::accounts(&self.app);
        }
        crate::mail::account_emails(&self.app)
    }

    fn email_guests(&self, d: &Details, organiser: &str, people: &[String], cancel: bool) {
        let (app, d, organiser, people) = (self.app.clone(), d.clone(), organiser.to_string(), people.to_vec());
        tauri::async_runtime::spawn(async move {
            let by_server = {
                let (app, d, o, p) = (app.clone(), d.clone(), organiser.clone(), people.clone());
                tauri::async_runtime::spawn_blocking(move || crate::servermail::invite(&app, &d, &o, &p, cancel)).await.ok().flatten()
            };
            let sent = match by_server {
                Some(r) => r,
                None => crate::mail::send_invitation(&app, &d, &organiser, &people, cancel).await,
            };
            if let Err(e) = sent {
                let _ = app.emit(
                    "to_app",
                    ToApp::Error { code: "mail".into(), message: format!("The email invitations weren't sent: {e}") },
                );
            }
        });
    }
}

/// The server over HTTP, as someone signed in (`token`; a request's own
/// token wins).
pub struct Http {
    pub server: String,
    pub token: String,
}

impl Fetch for Http {
    async fn request(
        &self,
        token: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let (server, path) = (self.server.clone(), path.to_string());
        let token = if token.is_empty() { self.token.clone() } else { token.to_string() };
        tauri::async_runtime::spawn_blocking(move || request(&server, &token, &path, body.as_ref()))
            .await
            .map_err(|e| e.to_string())?
    }
}

/// One request; the answer, or what went wrong in words.
pub fn request(
    server: &str,
    token: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .tls_config(crate::roots::tls())
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("{server}{path}");
    let auth = format!("Bearer {token}");
    let sent = match body {
        Some(body) => {
            let mut r = agent.post(&url).header("Content-Type", "application/json");
            if !token.is_empty() {
                r = r.header("Authorization", &auth);
            }
            r.send(serde_json::to_vec(body).expect("JSON serialises"))
        }
        None => {
            let mut r = agent.get(&url);
            if !token.is_empty() {
                r = r.header("Authorization", &auth);
            }
            r.call()
        }
    };
    let mut resp = sent.map_err(|e| {
        crate::roots::untrusted(&e.to_string())
            .unwrap_or_else(|| "Can't reach the server. Check you're online and try again.".to_string())
    })?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 204 {
        return Ok(serde_json::Value::Null);
    }
    if status == 200 {
        return serde_json::from_str(&text).map_err(|_| "The server sent something unexpected.".to_string());
    }
    let why = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("The server said no ({status})."));
    // "wrong email address or password" → "Wrong email address or password."
    let mut c = why.chars();
    Err(match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => why,
    })
}
