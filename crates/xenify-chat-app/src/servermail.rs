//! Email kept by the Xenify server: when the app is signed in to a server
//! that keeps people's email (`/v1/info` lists "email"), the screens' email
//! calls go there instead of to accounts on this computer, so the inbox,
//! drafts and accounts are the same in every app and browser. The server
//! answers with the same shapes as this app's own commands.
//!
//! What stays here: the calendar (answering an invitation keeps the
//! meeting in yours), My contacts (whoever you email), attachments saved to
//! Downloads, and meetings hearing guests' answers.

use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct ServerMail {
    /// The server last asked, and whether it keeps email.
    known: Mutex<Option<(String, bool)>>,
    /// The addresses the server keeps for you (for meeting invitations).
    accounts: Mutex<Vec<String>>,
    /// The server the events are being heard from.
    hearing: Mutex<String>,
}

fn creds(app: &AppHandle) -> (String, String) {
    app.state::<crate::net::Net>().creds()
}

/// The server and token, when email goes through the server.
pub fn on(app: &AppHandle) -> Option<(String, String)> {
    let (server, token) = creds(app);
    if server.is_empty() || token.is_empty() {
        return None;
    }
    let sm = app.state::<ServerMail>();
    let known = sm.known.lock().unwrap().clone();
    let yes = match known {
        Some((s, yes)) if s == server => yes,
        _ => {
            let yes = crate::host::request(&server, &token, "/v1/info", None)
                .ok()
                .and_then(|v| v["features"].as_array().map(|f| f.iter().any(|x| x == "email")))
                .unwrap_or(false);
            *sm.known.lock().unwrap() = Some((server.clone(), yes));
            yes
        }
    };
    yes.then_some((server, token))
}

/// The addresses the server keeps for you.
pub fn accounts(app: &AppHandle) -> Vec<String> {
    app.state::<ServerMail>().accounts.lock().unwrap().clone()
}

/// One email call to the server (sending may take a while).
fn post(server: &str, token: &str, cmd: &str, args: &Value) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .tls_config(crate::roots::tls())
        .http_status_as_error(false)
        .build()
        .into();
    let mut r = agent
        .post(&format!("{server}/v1/mail/{cmd}"))
        .header("Content-Type", "application/json")
        .header("Authorization", &format!("Bearer {token}"))
        .send(serde_json::to_vec(args).unwrap_or_default())
        .map_err(|_| "Can't reach the server. Check you're online and try again.".to_string())?;
    let status = r.status().as_u16();
    let text = r.body_mut().with_config().limit(64 << 20).read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return Ok(v);
    }
    Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("The server said no ({status}).")))
}

fn tz() -> String {
    crate::calendar::local_tz().name().to_string()
}

/// Handles an email call through the server, if email goes there (None:
/// it's this computer's).
pub fn intercept(
    app: &AppHandle,
    cmd: &str,
    args: &Value,
) -> Option<tauri::async_runtime::JoinHandle<Result<Value, String>>> {
    if !cmd.starts_with("mail_") {
        return None;
    }
    let (server, token) = on(app)?;
    hear(app, &server, &token);
    let (app, cmd, mut args) = (app.clone(), cmd.to_string(), args.clone());
    Some(tauri::async_runtime::spawn_blocking(move || {
        let chat = app.state::<crate::host::Chat>();
        match cmd.as_str() {
            // The calendar is this computer's.
            "mail_forget_invite" => {
                chat.cal.delete_uid(args["uid"].as_str().unwrap_or(""))?;
                chat.calendar_changed();
                Ok(Value::Null)
            }
            "mail_save_attachment" => save_attachment(&server, &token, &args),
            _ => {
                args["tz"] = json!(tz());
                let mut out = post(&server, &token, &cmd, &args)?;
                match cmd.as_str() {
                    "mail_accounts" => {
                        let list = out
                            .as_array()
                            .map(|l| l.iter().filter_map(|a| a["email"].as_str().map(String::from)).collect())
                            .unwrap_or_default();
                        *app.state::<ServerMail>().accounts.lock().unwrap() = list;
                    }
                    "mail_open" => {
                        if let Some(uid) = out["invite"]["uid"].as_str() {
                            let has = chat.cal.by_uid(uid).ok().flatten().is_some();
                            out["invite"]["in_calendar"] = json!(has);
                        }
                    }
                    "mail_send" => {
                        let people: Vec<(String, String)> =
                            serde_json::from_value(out["people"].clone()).unwrap_or_default();
                        chat.emailed(&people);
                        out = Value::Null;
                    }
                    "mail_answer_invite" => {
                        if let Some(uid) = out["declined"].as_str() {
                            chat.cal.delete_uid(uid)?;
                        } else if let Ok(mut e) =
                            serde_json::from_value::<xenify_chat_core::Event>(out["event"].clone())
                        {
                            e.id = chat.cal.by_uid(&e.ical_uid).ok().flatten().map(|x| x.id).unwrap_or(0);
                            chat.cal.save(&e)?;
                        }
                        chat.calendar_changed();
                        out = Value::Null;
                    }
                    _ => {}
                }
                Ok(out)
            }
        }
    }))
}

/// An attachment into Downloads (a new name if the file is there); says where.
fn save_attachment(server: &str, token: &str, a: &Value) -> Result<Value, String> {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    let url = format!(
        "{server}/v1/mail/attachment?account={}&folder={}&uid={}&index={}",
        enc(a["account"].as_str().unwrap_or("")),
        enc(a["folder"].as_str().unwrap_or("")),
        a["uid"].as_u64().unwrap_or(0),
        a["index"].as_u64().unwrap_or(0)
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .tls_config(crate::roots::tls())
        .http_status_as_error(false)
        .build()
        .into();
    let mut r =
        agent.get(&url).header("Authorization", &format!("Bearer {token}")).call().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err("The attachment couldn't be fetched from the server.".into());
    }
    let name = r
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split("filename*=UTF-8''").nth(1))
        .map(pct_decode)
        .unwrap_or_else(|| "attachment".into());
    let data = r.body_mut().with_config().limit(40 << 20).read_to_vec().map_err(|e| e.to_string())?;
    let dir = xenify_wp_check::home_dir().join("Downloads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = xenify_mail_core::message::safe_file_name(&name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    Ok(json!(path.display().to_string()))
}

/// "Caf%C3%A9.pdf" → "Café.pdf".
fn pct_decode(n: &str) -> String {
    let b = n.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(x) = u8::from_str_radix(&n[i + 1..i + 3], 16)
        {
            out.push(x);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Meeting invitations for guests outside the server, sent by the server.
pub fn invite(
    app: &AppHandle,
    d: &xenify_chat_core::Details,
    organiser: &str,
    people: &[String],
    cancel: bool,
) -> Option<Result<(), String>> {
    let (server, token) = on(app)?;
    let args = json!({ "details": d, "organiser": organiser, "people": people, "cancel": cancel, "tz": tz() });
    Some(post(&server, &token, "mail_invite", &args).map(|_| ()))
}

/// Hears the server's email news (once per server): new mail, sign-in
/// trouble, accounts added, answers to meetings.
fn hear(app: &AppHandle, server: &str, token: &str) {
    let sm = app.state::<ServerMail>();
    {
        let mut h = sm.hearing.lock().unwrap();
        if *h == server {
            return;
        }
        *h = server.to_string();
    }
    let (app, server, token) = (app.clone(), server.to_string(), token.to_string());
    std::thread::spawn(move || {
        let mut pause = Duration::from_secs(2);
        loop {
            // Signed out or moved to another server: stop.
            if creds(&app) != (server.clone(), token.clone()) {
                let sm = app.state::<ServerMail>();
                let mut h = sm.hearing.lock().unwrap();
                if *h == server {
                    h.clear();
                }
                return;
            }
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(20)))
                .timeout_recv_body(Some(Duration::from_secs(90)))
                .tls_config(crate::roots::tls())
                .http_status_as_error(false)
                .build()
                .into();
            if let Ok(mut r) = agent
                .get(&format!("{server}/v1/mail/events"))
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                && r.status().is_success()
            {
                pause = Duration::from_secs(2);
                use std::io::BufRead;
                let reader = std::io::BufReader::new(r.body_mut().with_config().limit(u64::MAX).reader());
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if creds(&app) != (server.clone(), token.clone()) {
                        break;
                    }
                    let Some(data) = line.strip_prefix("data: ") else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                    if v["kind"] == "invite_answer" {
                        let chat = app.state::<crate::host::Chat>();
                        chat.guest_answered(
                            v["uid"].as_str().unwrap_or(""),
                            v["email"].as_str().unwrap_or(""),
                            v["answer"].as_str().unwrap_or(""),
                        );
                        continue;
                    }
                    let _ = app.emit("mail", v);
                }
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_secs(60));
        }
    });
}
