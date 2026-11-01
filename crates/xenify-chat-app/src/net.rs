//! The connection to the chat server. The screens call the commands here and
//! listen for two events: `to_app` (whatever the server says, as
//! [`ToApp`]) and `net` (connected, connecting, offline or signed out).
//!
//! Which server: a link the server made (`<server>/join/#<id>.<secret>`, or
//! `xenify://join?link=…` from the browser) wins, then
//! `XENIFY_SERVER`, then the one remembered from last time, then the one
//! built in. People never type a server address to sign in.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use xenify_chat_core::account::{self, AppSecret, SecondStepState, SignedIn, parse_link};
use xenify_chat_proto::{Session, ToApp, ToServer, User};

/// The server this copy of Xenify Chat was built for (set `XENIFY_SERVER` when
/// building to change it).
const BUILT_IN: &str = match option_env!("XENIFY_SERVER") {
    Some(s) => s,
    None => "http://127.0.0.1:7700",
};

/// What's remembered between runs.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    server: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    user: Option<User>,
    /// The Hub's sign-in in use (Chat follows it unless `own`: set to a
    /// server of its own).
    #[serde(default)]
    hub: String,
    #[serde(default)]
    own: bool,
    /// Server address -> token from "Remember this computer for 30 days"
    /// (the second sign-in step): kept after signing out.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    remember: std::collections::BTreeMap<String, String>,
}

impl Saved {
    /// Signed out, at `server`; remembered computers stay remembered.
    fn fresh(&self, server: String, own: bool) -> Saved {
        Saved { server, own, remember: self.remember.clone(), ..Saved::default() }
    }
}

/// The Hub's sign-in as Chat keeps one (None if it has no session token).
fn from_hub(h: &xenify_hub_core::signin::SignIn) -> Option<Saved> {
    let a = &h.session["account"];
    Some(Saved {
        server: h.url.clone(),
        token: h.session["token"].as_str()?.to_string(),
        user: Some(User {
            id: a["id"].as_i64().unwrap_or_default(),
            username: a["username"].as_str().unwrap_or_default().to_string(),
            display_name: a["display_name"].as_str().unwrap_or_default().to_string(),
            app: false,
            email: a["email"].as_str().unwrap_or_default().to_string(),
        }),
        hub: h.id.clone(),
        own: false,
        remember: Default::default(),
    })
}

pub struct Net {
    saved: Mutex<Saved>,
    /// The `<id>.<secret>` of the link Xenify Chat was opened with.
    invite: Mutex<String>,
    /// To the connection; dropping it closes the connection.
    tx: Mutex<Option<UnboundedSender<ToServer>>>,
}

fn saved_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("xenify").join("chat.json"))
}

fn load() -> Saved {
    saved_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Saves where only you can read it (it holds your sign-in).
fn store(s: &Saved) {
    let Some(p) = saved_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    if let Ok(mut f) = o.open(&tmp) {
        use std::io::Write;
        if f.write_all(&serde_json::to_vec_pretty(s).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

impl Net {
    /// Who's signed in, as the server's account ({id, username, display_name}).
    pub fn me(&self) -> serde_json::Value {
        let s = self.saved.lock().unwrap();
        match &s.user {
            Some(u) => serde_json::json!({ "id": u.id, "username": u.username, "display_name": u.display_name }),
            None => serde_json::Value::Null,
        }
    }

    /// The server and your session token (empty when signed out).
    pub fn creds(&self) -> (String, String) {
        let s = self.saved.lock().unwrap();
        (s.server.clone(), s.token.clone())
    }

    /// `args`: the command line, which may hold a link the server made.
    pub fn new(args: &[String]) -> Net {
        let mut saved = load();
        let mut invite = String::new();
        if let Some((server, link)) = args.iter().find_map(|a| parse_link(a)) {
            if server != saved.server {
                saved = saved.fresh(server, false);
            }
            invite = link;
        } else if let Ok(s) = std::env::var("XENIFY_SERVER")
            && s.trim_end_matches('/') != saved.server
        {
            saved = saved.fresh(s.trim_end_matches('/').into(), false);
        }
        if saved.server.is_empty() {
            saved.server = BUILT_IN.into();
        }
        Net { saved: Mutex::new(saved), invite: Mutex::new(invite), tx: Mutex::new(None) }
    }

    fn connect(&self, app: &AppHandle) {
        let s = self.saved.lock().unwrap().clone();
        let (tx, rx) = unbounded_channel();
        *self.tx.lock().unwrap() = Some(tx);
        tauri::async_runtime::spawn(run(app.clone(), s.server, s.token, rx));
    }

    fn disconnect(&self) {
        self.tx.lock().unwrap().take();
    }

    /// Takes up the Hub's sign-in, a new one, or none (the Hub signed out:
    /// so does Chat), unless set to a server of its own. Whether anything
    /// changed.
    pub fn follow_hub(&self) -> bool {
        let h = xenify_hub_core::signin::read();
        let mut s = self.saved.lock().unwrap();
        if s.own {
            return false;
        }
        let mut new = match &h {
            None if !s.hub.is_empty() => s.fresh(s.server.clone(), false),
            Some(h) if h.id != s.hub => match from_hub(h) {
                Some(n) => n,
                None => return false,
            },
            _ => return false,
        };
        new.remember = std::mem::take(&mut s.remember);
        *s = new;
        store(&s);
        drop(s);
        self.disconnect();
        true
    }
}

#[derive(Serialize)]
pub struct Start {
    server: String,
    /// Signed in through Xenify Hub (signing out is done there).
    via_hub: bool,
    /// The Hub is signed in to a server (Chat can go back to following it).
    hub: bool,
    user: Option<User>,
    /// The link Xenify Chat was opened with (`<id>.<secret>`; see
    /// [`link_info`]).
    invite: String,
    /// Show the built-in sample chats instead (for trying the screens).
    sample: bool,
    /// Say what happens through `log` (tests).
    trace: bool,
    /// Chat signs in by itself (a server given for this run with
    /// `XENIFY_SERVER`: scripts/try-chat.sh, tests). Otherwise Xenify Hub
    /// signs every app in and Chat only points to it.
    sign_in_here: bool,
}

/// Called once the screens are up: who's signed in, and connects if anyone is.
#[tauri::command]
pub fn start(app: AppHandle, net: State<'_, Net>) -> Start {
    net.follow_hub();
    forget_old_copies();
    let s = net.saved.lock().unwrap().clone();
    let sample = std::env::var("XENIFY_SAMPLE").is_ok_and(|v| v == "1");
    let signed_in = !s.token.is_empty() && !sample;
    if signed_in && net.tx.lock().unwrap().is_none() {
        net.connect(&app);
    }
    Start {
        server: s.server,
        via_hub: signed_in && !s.hub.is_empty(),
        hub: xenify_hub_core::signin::read().is_some(),
        user: if signed_in { s.user } else { None },
        invite: net.invite.lock().unwrap().clone(),
        sample,
        trace: tracing(),
        sign_in_here: std::env::var_os("XENIFY_SERVER").is_some(),
    }
}

/// What `username`'s code (emailed, or from an admin) is for, before using
/// it; or an older link: pasted (`<server>/join/#<id>.<secret>`), opened
/// (`xenify://join?link=…`), or "" for the one Xenify Chat was opened with.
/// A pasted link's server becomes this app's. Answers the server's
/// `LinkInfo`, with `link` (the code).
#[tauri::command]
pub async fn link_info(net: State<'_, Net>, link: String, username: String) -> Result<serde_json::Value, String> {
    let link = take_link(&net, &link)?;
    let server = net.saved.lock().unwrap().server.clone();
    let fetch = crate::host::Http { server, token: String::new() };
    let info = xenify_chat_core::account::link_info(&fetch, &link, &username).await?;
    let mut v = serde_json::to_value(info).unwrap_or_default();
    v["link"] = link.into();
    Ok(v)
}

/// A link's `<id>.<secret>`, switching to its server when it names another.
fn take_link(net: &Net, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        let kept = net.invite.lock().unwrap().clone();
        return if kept.is_empty() { Err("Paste the link you were sent.".into()) } else { Ok(kept) };
    }
    if let Some((server, link)) = parse_link(text) {
        let mut s = net.saved.lock().unwrap();
        if s.server != server {
            let own = s.own || !s.hub.is_empty() || xenify_hub_core::signin::read().is_some();
            *s = s.fresh(server, own);
        }
        return Ok(link);
    }
    // A code, or the part of an older link after `#`.
    if text.contains(['/', ' ']) {
        Err("That isn't the code you were sent.".into())
    } else {
        Ok(text.into())
    }
}

/// Signs in; or, with a link or code (see [`link_info`]), picks your
/// password.
#[tauri::command]
pub async fn sign_in(
    app: AppHandle,
    net: State<'_, Net>,
    username: String,
    password: String,
    invite: String,
) -> Result<serde_json::Value, String> {
    let link = if invite.trim().is_empty() { String::new() } else { take_link(&net, &invite)? };
    let (server, remember) = {
        let s = net.saved.lock().unwrap();
        (s.server.clone(), s.remember.get(&s.server).cloned().unwrap_or_default())
    };
    let fetch = crate::host::Http { server, token: String::new() };
    let answer =
        xenify_chat_core::account::account(&fetch, &username, &password, &link, &device(), &remember)
            .await?;
    match answer {
        SignedIn::Session { session, remember } => {
            Ok(serde_json::to_value(signed_in(&app, &net, session, remember)).unwrap_or_default())
        }
        // The password was right: the second step next (second_code, or setting one up).
        SignedIn::Step(step) => Ok(serde_json::json!({ "step": step })),
    }
}

/// What this computer is called in the server's list of sign-ins.
fn device() -> String {
    format!(
        "Xenify Chat on {}",
        std::fs::read_to_string("/etc/hostname")
            .map(|h| h.trim().to_string())
            .unwrap_or_else(|_| "this computer".into())
    )
}

/// Signed in: keeps the session (and a remembered computer's token) and connects.
fn signed_in(app: &AppHandle, net: &Net, session: Session, remember: Option<String>) -> User {
    {
        let mut s = net.saved.lock().unwrap();
        s.token = session.token;
        s.user = Some(session.user.clone());
        if let Some(r) = remember {
            let server = s.server.clone();
            s.remember.insert(server, r);
        }
        // Signed in here: its own sign-in, even if the Hub has one later.
        s.own = s.own || !s.hub.is_empty() || xenify_hub_core::signin::read().is_some();
        s.hub.clear();
        store(&s);
    }
    net.invite.lock().unwrap().clear();
    net.disconnect();
    net.connect(app);
    session.user
}

/// The server, with the session when signed in (for the second step's
/// setup and settings).
fn fetch(net: &Net) -> crate::host::Http {
    let (server, token) = net.creds();
    crate::host::Http { server, token }
}

/// The second step's code (`method` "app", "email" or "backup").
#[tauri::command]
pub async fn second_code(
    app: AppHandle,
    net: State<'_, Net>,
    ticket: String,
    method: String,
    code: String,
    remember: bool,
) -> Result<User, String> {
    let (session, kept) = account::second_code(&fetch(&net), &ticket, &method, &code, remember, &device()).await?;
    Ok(signed_in(&app, &net, session, kept))
}

/// "Email me a code" during the second step.
#[tauri::command]
pub async fn second_email(net: State<'_, Net>, ticket: String) -> Result<(), String> {
    account::email_code(&fetch(&net), &ticket).await
}

/// Setting a second step up: while signing in (`ticket`), or signed in
/// (`ticket` ""). An authenticator key to scan: { secret, uri, qr_svg }.
#[tauri::command]
pub async fn mfa_setup_app(net: State<'_, Net>, ticket: String) -> Result<AppSecret, String> {
    account::setup_app(&fetch(&net), &ticket).await
}

#[tauri::command]
pub async fn mfa_setup_email(net: State<'_, Net>, ticket: String) -> Result<(), String> {
    account::setup_email(&fetch(&net), &ticket).await
}

/// Turns it on: { backup_codes, user } (`user` when it signed in).
#[tauri::command]
pub async fn mfa_setup_finish(
    app: AppHandle,
    net: State<'_, Net>,
    ticket: String,
    method: String,
    code: String,
    remember: bool,
) -> Result<serde_json::Value, String> {
    let (codes, session) = account::setup_finish(&fetch(&net), &ticket, &method, &code, remember).await?;
    let user = session.map(|(s, r)| signed_in(&app, &net, s, r));
    Ok(serde_json::json!({ "backup_codes": codes, "user": user }))
}

/// Settings › Account › Two-step sign-in.
#[tauri::command]
pub async fn mfa_state(net: State<'_, Net>) -> Result<SecondStepState, String> {
    account::second_step(&fetch(&net)).await
}

#[tauri::command]
pub async fn mfa_off(net: State<'_, Net>, password: String) -> Result<(), String> {
    account::second_step_off(&fetch(&net), &password).await
}

#[tauri::command]
pub async fn mfa_backup(net: State<'_, Net>, password: String) -> Result<Vec<String>, String> {
    account::new_backup_codes(&fetch(&net), &password).await
}

/// The server's name, version and what it can do (`features`: "mail"…).
#[tauri::command]
pub async fn server_info(net: State<'_, Net>) -> Result<serde_json::Value, String> {
    xenify_chat_core::Fetch::request(&fetch(&net), "", "/v1/info", None).await
}

/// The server's days off in `year` (Admin › Holidays): public holidays and
/// company days, `[{ from, to, name, moved_from, company }]`.
#[tauri::command]
pub async fn holidays(net: State<'_, Net>, year: i32) -> Result<serde_json::Value, String> {
    xenify_chat_core::Fetch::request(&fetch(&net), "", &format!("/v1/plan/holidays/{year}"), None).await
}

/// A one-time secret, encrypted by the screens: `{ id, link, expires }`
/// (the key goes on the end of the link, after the server's part); `check`
/// with a passphrase (see the server's src/secrets.rs).
#[tauri::command]
pub async fn secret_new(
    net: State<'_, Net>,
    sealed: String,
    days: i32,
    check: Option<String>,
) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({ "sealed": sealed, "days": days, "check": check });
    xenify_chat_core::Fetch::request(&fetch(&net), "", "/v1/secrets", Some(body)).await
}

/// Signing in, the user name first: `{ next: "password" | "pick_password",
/// can_email }` ("pick_password": an account an admin added that has no
/// password yet, picked with the code from the admin).
#[tauri::command]
pub async fn sign_in_start(net: State<'_, Net>, username: String) -> Result<serde_json::Value, String> {
    let server = net.saved.lock().unwrap().server.clone();
    let fetch = crate::host::Http { server, token: String::new() };
    let step = xenify_chat_core::account::start(&fetch, &username).await?;
    Ok(serde_json::to_value(step).unwrap_or_default())
}

/// "Forgot password?" or "Email me a link": a link to pick one, by email.
#[tauri::command]
pub async fn forgot(net: State<'_, Net>, username: String) -> Result<(), String> {
    let server = net.saved.lock().unwrap().server.clone();
    let fetch = crate::host::Http { server, token: String::new() };
    xenify_chat_core::account::forgot(&fetch, &username).await
}

#[tauri::command]
pub fn send(net: State<'_, Net>, msg: ToServer) -> Result<(), String> {
    match net.tx.lock().unwrap().as_ref() {
        Some(tx) => tx.send(msg).map_err(|_| "not connected".into()),
        None => Err("not signed in".into()),
    }
}

#[tauri::command]
pub fn sign_out(net: State<'_, Net>) {
    net.disconnect();
    let mut s = net.saved.lock().unwrap();
    s.token.clear();
    s.user = None;
    // Leaving the Hub's sign-in (the Hub and the other apps stay signed in).
    if !s.hub.is_empty() {
        s.hub.clear();
        s.own = true;
    }
    store(&s);
}

/// Settings › Account › Use the Hub's server.
#[tauri::command]
pub fn use_hub(net: State<'_, Net>) -> bool {
    net.saved.lock().unwrap().own = false;
    net.follow_hub()
}

/// Settings › Account › Change server (signs you out).
#[tauri::command]
pub fn change_server(net: State<'_, Net>, server: String) -> Result<String, String> {
    let server = server.trim().trim_end_matches('/').to_string();
    let server = if server.contains("://") { server } else { format!("https://{server}") };
    if !(server.starts_with("http://") || server.starts_with("https://")) || server.len() < 10 {
        return Err("That doesn't look like a server address.".into());
    }
    net.disconnect();
    let mut s = net.saved.lock().unwrap();
    let own = s.own || !s.hub.is_empty() || xenify_hub_core::signin::read().is_some();
    *s = s.fresh(server.clone(), own);
    store(&s);
    Ok(server)
}

/// The server's own HTTPS certificate (no domain name): what it is and
/// whether it's trusted here, or (with `fingerprint`, what the person was
/// shown) trusting it from now on (`remember`) or until Chat closes.
#[tauri::command]
pub async fn certificate(
    window: tauri::WebviewWindow,
    net: State<'_, Net>,
    fingerprint: Option<String>,
    remember: bool,
) -> Result<xenify_platform::trust::Seen, String> {
    let url = net.saved.lock().unwrap().server.clone();
    let seen = tauri::async_runtime::spawn_blocking(move || match fingerprint {
        Some(f) => xenify_platform::trust::trust(&url, &f, remember),
        None => xenify_platform::trust::look(&url),
    })
    .await
    .map_err(|e| e.to_string())??;
    // Calls and pictures go through the web view.
    #[cfg(target_os = "linux")]
    if seen.trusted || seen.ignored {
        let s = seen.clone();
        // Done before answering, so a call started next finds it allowed.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = window.with_webview(move |w| {
            xenify_platform::trust::allow(&w.inner(), &s);
            let _ = tx.send(());
        });
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = window;
    Ok(seen)
}

#[derive(Clone, Serialize)]
struct NetState {
    state: &'static str,
}

/// `XENIFY_CHAT_LOG=1`: the screens and this side say what they're doing, on
/// standard error (for tests).
pub fn tracing() -> bool {
    std::env::var_os("XENIFY_CHAT_LOG").is_some()
}

/// The screens' trace (only with `XENIFY_CHAT_LOG`).
#[tauri::command]
pub fn log(text: String) {
    if tracing() {
        eprintln!("screens: {text}");
    }
}

fn status(app: &AppHandle, state: &'static str) {
    if tracing() {
        eprintln!("net: {state}");
    }
    let _ = app.emit("net", NetState { state });
}

enum Fail {
    SignedOut,
    Other,
}

/// Earlier versions kept a copy of your messages for searching (the server
/// searches now): removed.
fn forget_old_copies() {
    let dir = xenify_wp_check::data_dir().join("chat");
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "sqlite") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// Keeps a connection open until `rx` is dropped, reconnecting with growing
/// pauses. Messages sent while offline wait in `rx`.
async fn run(app: AppHandle, server: String, token: String, mut rx: UnboundedReceiver<ToServer>) {
    let ws_url = format!("{}/v1/ws", server.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1));
    let mut pause = 1u64;
    // A message the connection broke while sending: sent first next time.
    let mut unsent: Option<String> = None;
    loop {
        if rx.is_closed() {
            return;
        }
        status(&app, "connecting");
        let socket = match open(&ws_url, &token).await {
            Ok(s) => s,
            Err(Fail::SignedOut) => {
                sign_out_here(&app, "signed_out");
                return;
            }
            Err(Fail::Other) => {
                status(&app, "offline");
                // Wait, but stop at once if signed out meanwhile.
                for _ in 0..pause * 4 {
                    if rx.is_closed() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                pause = (pause * 2).min(30);
                continue;
            }
        };
        pause = 1;
        status(&app, "online");
        let (mut out, mut inp) = socket.split();
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        ping.tick().await;
        let mut quiet = 0;
        if let Some(json) = unsent.take()
            && out.send(Ws::Text(json.clone().into())).await.is_err()
        {
            unsent = Some(json);
            status(&app, "offline");
            continue;
        }
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => {
                        let json = serde_json::to_string(&m).expect("messages serialise");
                        if out.send(Ws::Text(json.clone().into())).await.is_err() {
                            unsent = Some(json);
                            break;
                        }
                    }
                    None => {
                        let _ = out.send(Ws::Close(None)).await;
                        return;
                    }
                },
                f = inp.next() => match f {
                    Some(Ok(Ws::Text(t))) => {
                        quiet = 0;
                        if let Ok(m) = serde_json::from_str::<ToApp>(&t) {
                            // Meetings, your calendar and busy times aren't chats.
                            if let Some(m) = app.state::<crate::host::Chat>().incoming(m) {
                                let _ = app.emit("to_app", m);
                            }
                        }
                    }
                    Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => quiet = 0,
                },
                _ = ping.tick() => {
                    // Two pings with no answer: the connection is dead.
                    quiet += 1;
                    if quiet > 2 || out.send(Ws::Text(serde_json::to_string(&ToServer::Ping).unwrap().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        status(&app, "offline");
    }
}

/// Forgets the sign-in (the server ended it) and tells the screens why.
fn sign_out_here(app: &AppHandle, why: &'static str) {
    if let Some(net) = tauri::Manager::try_state::<Net>(app) {
        let mut s = net.saved.lock().unwrap();
        s.token.clear();
        s.user = None;
        store(&s);
        net.tx.lock().unwrap().take();
    }
    status(app, why);
}

async fn open(
    url: &str,
    token: &str,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Fail> {
    let mut req = url.into_client_request().map_err(|_| Fail::Other)?;
    req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().map_err(|_| Fail::Other)?);
    let connect = tokio_tungstenite::connect_async_tls_with_config(req, None, false, crate::roots::websocket());
    match tokio::time::timeout(Duration::from_secs(15), connect).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(r))) if r.status() == 401 => Err(Fail::SignedOut),
        _ => Err(Fail::Other),
    }
}

#[cfg(test)]
mod tests {
    /// The chat app's own sign-in code against a real Xenify server (needs
    /// the test PostgreSQL; skipped without it).
    #[tokio::test(flavor = "multi_thread")]
    async fn signs_in_and_uses_links_on_the_xenify_server() {
        use serde_json::json;
        use xenify_chat_core::account::{self as acc, SignedIn, account, link_info, parse_link};
        use xenify_server_proto::LinkKind;
        let done = |a: Result<SignedIn, String>| match a.unwrap() {
            SignedIn::Session { session, .. } => session,
            SignedIn::Step(s) => panic!("a second step: {s:?}"),
        };
        let admin_db =
            std::env::var("XENIFY_TEST_PG").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/postgres".into());
        let Ok((c, conn)) = tokio_postgres::connect(&admin_db, tokio_postgres::NoTls).await else {
            return eprintln!("no PostgreSQL to test with: skipped");
        };
        tokio::spawn(conn);
        let name = format!("chatapp_{}", std::process::id());
        let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {name}")).await;
        c.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
        let db = format!("{}/{name}", admin_db.rsplit_once('/').unwrap().0);
        let data = std::env::temp_dir().join(&name);
        let server = xenify_server::Server::start(xenify_server::Config {
            database: db,
            data,
            local_webhooks: false,
            web: None,
            secure: None,
            postgres: None,
        })
        .await
        .unwrap();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let s2 = server.clone();
        tokio::spawn(async move { s2.serve(l, std::future::pending()).await.unwrap() });

        // The first admin sets the server up and adds Priya and Sam.
        let setup = json!({
            "token": server.setup_token().unwrap(), "server_name": "Office", "username": "alex",
            "password": "alex password 1", "device": "test",
        });
        let u = url.clone();
        let (welcome, sam, admin) = tokio::task::spawn_blocking(move || {
            let admin = crate::host::request(&u, "", "/v1/setup", Some(&setup)).unwrap();
            let t = admin["token"].as_str().unwrap().to_string();
            let t = t.as_str();
            let welcome =
                crate::host::request(&u, t, "/v1/admin/people", Some(&json!({ "username": "priya" }))).unwrap();
            let sam = crate::host::request(&u, t, "/v1/admin/people", Some(&json!({ "username": "sam" }))).unwrap();
            let codes = (welcome["code"].as_str().unwrap().to_string(), sam["code"].as_str().unwrap().to_string());
            (codes.0, codes.1, t.to_string())
        })
        .await
        .unwrap();

        let fetch = crate::host::Http { server: url.clone(), token: String::new() };
        // Signing in starts with the user name: Priya has no password yet.
        assert_eq!(acc::start(&fetch, "priya").await.unwrap().next, acc::LoginNext::PickPassword);
        assert_eq!(acc::start(&fetch, "alex").await.unwrap().next, acc::LoginNext::Password);
        // Priya types the code from her admin and picks her password.
        let info = link_info(&fetch, &welcome, "priya").await.unwrap();
        assert_eq!((info.kind, info.username.as_str()), (LinkKind::Welcome, "priya"));
        assert!(link_info(&fetch, &welcome, "sam").await.is_err(), "it's hers only");
        let s = done(account(&fetch, "priya", "priya password", &welcome, "test", "").await);
        assert_eq!(s.user.username, "priya");
        assert!(account(&fetch, "priya", "priya password", &welcome, "test", "").await.is_err(), "codes work once");
        let wrong = account(&fetch, "priya", "not it at all", "", "test", "").await;
        assert_eq!(wrong.unwrap_err(), "Wrong email address or password.");
        let s = done(account(&fetch, "priya", "priya password", "", "test", "").await);
        assert!(!s.token.is_empty());

        // Priya turns two-step sign-in on in Settings (an authenticator app).
        let priya = crate::host::Http { server: url.clone(), token: s.token.clone() };
        let key = acc::setup_app(&priya, "").await.unwrap();
        assert!(key.qr_svg.starts_with("<svg") && key.uri.starts_with("otpauth://totp/"));
        assert!(acc::setup_finish(&priya, "", "app", "000000x", false).await.is_err());
        let (codes, session) = acc::setup_finish(&priya, "", "app", &code_now(&key.secret, 0), false).await.unwrap();
        assert_eq!((codes.len(), session.is_none()), (10, true));
        let st = acc::second_step(&priya).await.unwrap();
        assert!(st.app && !st.email && !st.required && st.backup_left == 10);
        // Signing in now takes a code: a wrong one, then a backup code, remembering this computer.
        let SignedIn::Step(step) = account(&fetch, "priya", "priya password", "", "test", "").await.unwrap() else {
            panic!("no second step")
        };
        assert!(!step.setup && step.methods.contains(&"app".into()) && step.methods.contains(&"backup".into()));
        assert_eq!(
            acc::second_code(&fetch, &step.ticket, "app", "123456", false, "test").await.unwrap_err(),
            "That code isn't right."
        );
        let (s, remember) = acc::second_code(&fetch, &step.ticket, "backup", &codes[0], true, "test").await.unwrap();
        assert_eq!(s.user.username, "priya");
        let remember = remember.expect("a remember token");
        // Remembered: no step for 30 days; a used backup code is gone.
        done(account(&fetch, "priya", "priya password", "", "test", &remember).await);
        let priya = crate::host::Http { server: url.clone(), token: s.token.clone() };
        assert_eq!(acc::second_step(&priya).await.unwrap().backup_left, 9);
        assert!(acc::new_backup_codes(&priya, "not it").await.is_err());
        assert_eq!(acc::new_backup_codes(&priya, "priya password").await.unwrap().len(), 10);
        acc::second_step_off(&priya, "priya password").await.unwrap();
        done(account(&fetch, "priya", "priya password", "", "test", "").await);

        // The server requires it of admins: Alex sets it up while signing in.
        let u = url.clone();
        tokio::task::spawn_blocking(move || {
            crate::host::request(&u, &admin, "/v1/admin/settings", Some(&json!({ "mfa_required": "admins" }))).unwrap()
        })
        .await
        .unwrap();
        let SignedIn::Step(step) = account(&fetch, "alex", "alex password 1", "", "test", "").await.unwrap() else {
            panic!("no setup step")
        };
        assert!(step.setup && step.methods.is_empty() && step.setup_methods == ["app"]);
        let key = acc::setup_app(&fetch, &step.ticket).await.unwrap();
        let (codes, session) =
            acc::setup_finish(&fetch, &step.ticket, "app", &code_now(&key.secret, 0), true).await.unwrap();
        let (s, remember) = session.expect("signed in");
        assert_eq!((codes.len(), s.user.username.as_str(), remember.is_some()), (10, "alex", true));
        let alex = crate::host::Http { server: url.clone(), token: s.token };
        assert!(acc::second_step(&alex).await.unwrap().required);
        assert!(acc::second_step_off(&alex, "alex password 1").await.is_err(), "required");
        // Sam's code too.
        assert_eq!(link_info(&fetch, &sam, "sam").await.unwrap().kind, LinkKind::Welcome);
        let s = done(account(&fetch, "sam", "sam password 1", &sam, "test", "").await);
        assert_eq!(s.user.username.as_str(), "sam");
        // Older links still parse.
        let deep = "xenify://join?link=https%3A%2F%2Fchat.example.org%2Fjoin%2F%23AbC.dEf";
        assert_eq!(parse_link(deep), Some(("https://chat.example.org".into(), "AbC.dEf".into())));
    }

    /// What an authenticator app shows now for `secret` (base32), as RFC 6238
    /// works it out.
    fn code_now(secret: &str, ahead: i64) -> String {
        use hmac::{KeyInit, Mac};
        let mut key = Vec::new();
        let (mut buf, mut bits) = (0u32, 0);
        for c in secret.chars() {
            buf = (buf << 5) | "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".find(c).unwrap() as u32;
            bits += 5;
            if bits >= 8 {
                key.push((buf >> (bits - 8)) as u8);
                bits -= 8;
            }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let step = (now / 30) as i64 + ahead;
        let mut m = hmac::Hmac::<sha1::Sha1>::new_from_slice(&key).unwrap();
        m.update(&(step as u64).to_be_bytes());
        let h = m.finalize().into_bytes();
        let o = (h[19] & 0x0f) as usize;
        format!("{:06}", (u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) & 0x7fff_ffff) % 1_000_000)
    }
}
