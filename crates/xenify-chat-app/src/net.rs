//! The connection to the chat server. The screens call the commands here and
//! listen for two events: `to_app` (whatever the server says, as
//! [`ToApp`]) and `net` (connected, connecting, offline or signed out).
//!
//! Which server: an invite link (`xenify://join?server=…&code=…`) wins, then
//! `XENIFY_SERVER`, then the one remembered from last time, then the one
//! built in. People never type a server address to sign in.

use futures_util::{SinkExt, StreamExt};
use xenify_chat_proto::{Login, Session, ToApp, ToServer, User};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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
}

pub struct Net {
    saved: Mutex<Saved>,
    /// An invite code from the link Xenify Chat was opened with.
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

/// `xenify://join?server=https%3A%2F%2Fchat.example.org&code=ABC` → (server, code).
pub fn parse_invite(link: &str) -> Option<(String, String)> {
    let rest = link.trim().strip_prefix("xenify://join")?;
    let query = rest.trim_start_matches('/').strip_prefix('?')?;
    let (mut server, mut code) = (String::new(), String::new());
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "server" => server = unescape(v),
            "code" => code = unescape(v),
            _ => {}
        }
    }
    (server.starts_with("http://") || server.starts_with("https://"))
        .then_some((server.trim_end_matches('/').into(), code))
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Net {
    /// `args`: the command line, which may hold an invite link.
    pub fn new(args: &[String]) -> Net {
        let mut saved = load();
        let mut invite = String::new();
        if let Some((server, code)) = args.iter().find_map(|a| parse_invite(a)) {
            if server != saved.server {
                saved = Saved { server, ..Saved::default() };
            }
            invite = code;
        } else if let Ok(s) = std::env::var("XENIFY_SERVER")
            && s.trim_end_matches('/') != saved.server
        {
            saved = Saved { server: s.trim_end_matches('/').into(), ..Saved::default() };
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
}

#[derive(Serialize)]
pub struct Start {
    server: String,
    user: Option<User>,
    invite: String,
    /// Show the built-in sample chats instead (for trying the screens).
    sample: bool,
}

/// Called once the screens are up: who's signed in, and connects if anyone is.
#[tauri::command]
pub fn start(app: AppHandle, net: State<'_, Net>) -> Start {
    let s = net.saved.lock().unwrap().clone();
    let sample = std::env::var("XENIFY_SAMPLE").is_ok_and(|v| v == "1");
    let signed_in = !s.token.is_empty() && !sample;
    if signed_in && net.tx.lock().unwrap().is_none() {
        net.connect(&app);
    }
    Start {
        server: s.server,
        user: if signed_in { s.user } else { None },
        invite: net.invite.lock().unwrap().clone(),
        sample,
    }
}

/// Signs in, or with an invite code (or link), creates the account.
#[tauri::command]
pub async fn sign_in(
    app: AppHandle,
    net: State<'_, Net>,
    username: String,
    password: String,
    invite: String,
    display_name: String,
) -> Result<User, String> {
    let mut code = invite.trim().to_string();
    if code.starts_with("xenify://") {
        let (server, c) = parse_invite(&code).ok_or("That invite link doesn't look right.")?;
        net.saved.lock().unwrap().server = server;
        code = c;
    }
    let server = net.saved.lock().unwrap().server.clone();
    let device = format!(
        "Xenify Chat on {}",
        std::fs::read_to_string("/etc/hostname")
            .map(|h| h.trim().to_string())
            .unwrap_or_else(|_| "this computer".into())
    );
    let login = Login { username: username.trim().into(), password, device, invite: code.clone(), display_name };
    let path = if code.is_empty() { "/v1/login" } else { "/v1/register" };
    let url = format!("{server}{path}");
    let session =
        tauri::async_runtime::spawn_blocking(move || post(&url, &login)).await.map_err(|e| e.to_string())??;
    {
        let mut s = net.saved.lock().unwrap();
        s.token = session.token;
        s.user = Some(session.user.clone());
        store(&s);
    }
    net.invite.lock().unwrap().clear();
    net.disconnect();
    net.connect(&app);
    Ok(session.user)
}

fn post(url: &str, login: &Login) -> Result<Session, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .post(url)
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(login).expect("a login serialises"))
        .map_err(|_| "Can't reach the chat server. Check you're online and try again.".to_string())?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 200 {
        return serde_json::from_str(&text).map_err(|_| "The chat server sent something unexpected.".to_string());
    }
    let why = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("The chat server said no ({status})."));
    // "wrong user name or password" → "Wrong user name or password."
    let mut c = why.chars();
    Err(match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => why,
    })
}

#[tauri::command]
pub fn send(net: State<'_, Net>, msg: ToServer) -> Result<(), String> {
    match net.tx.lock().unwrap().as_ref() {
        Some(tx) => tx.send(msg).map_err(|_| "not connected".into()),
        None => Err("not signed in".into()),
    }
}

#[tauri::command]
pub fn sign_out(net: State<'_, Net>, kept: State<'_, crate::store::Store>) {
    net.disconnect();
    // Your messages don't stay on this computer after you sign out.
    kept.forget();
    let mut s = net.saved.lock().unwrap();
    s.token.clear();
    s.user = None;
    store(&s);
}

/// Settings › Account › Change server (signs you out).
#[tauri::command]
pub fn change_server(
    net: State<'_, Net>,
    kept: State<'_, crate::store::Store>,
    server: String,
) -> Result<String, String> {
    let server = server.trim().trim_end_matches('/').to_string();
    let server = if server.contains("://") { server } else { format!("https://{server}") };
    if !(server.starts_with("http://") || server.starts_with("https://")) || server.len() < 10 {
        return Err("That doesn't look like a server address.".into());
    }
    net.disconnect();
    kept.forget();
    let mut s = net.saved.lock().unwrap();
    *s = Saved { server: server.clone(), ..Saved::default() };
    store(&s);
    Ok(server)
}

#[derive(Clone, Serialize)]
struct NetState {
    state: &'static str,
}

fn status(app: &AppHandle, state: &'static str) {
    let _ = app.emit("net", NetState { state });
}

enum Fail {
    SignedOut,
    Other,
}

/// Keeps a connection open until `rx` is dropped, reconnecting with growing
/// pauses. Messages sent while offline wait in `rx`.
/// Marks history fetched in the background to fill in the store.
const BACKFILL: &str = "backfill";

/// Keeps messages in the store, and asks for older ones until each chat's
/// whole history is there. Returns what to send the server next.
fn keep(app: &AppHandle, server: &str, m: &ToApp) -> Vec<ToServer> {
    let st = app.state::<crate::store::Store>();
    let older = |conv: i64, before: Option<i64>| ToServer::History {
        conv,
        before,
        limit: crate::store::BACKFILL_PAGE,
        tag: BACKFILL.into(),
    };
    match m {
        ToApp::Welcome { user, .. } => st.open(server, user.id),
        ToApp::Message { msg } => st.put(std::slice::from_ref(msg)),
        ToApp::History { conv, messages, tag } => {
            st.put(messages);
            if tag == BACKFILL {
                // A short page: that was the start of the chat.
                return match messages.first() {
                    Some(first) if messages.len() as u32 == crate::store::BACKFILL_PAGE && first.seq > 1 => {
                        vec![older(*conv, Some(first.seq))]
                    }
                    _ => {
                        st.mark_complete(*conv);
                        Vec::new()
                    }
                };
            }
        }
        ToApp::Conversations { list } => {
            return list.iter().filter_map(|c| st.needs_older(c.id).map(|before| older(c.id, before))).collect();
        }
        _ => {}
    }
    Vec::new()
}

async fn run(app: AppHandle, server: String, token: String, mut rx: UnboundedReceiver<ToServer>) {
    let ws_url = format!("{}/v1/ws", server.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1));
    let mut pause = 1u64;
    loop {
        if rx.is_closed() {
            return;
        }
        status(&app, "connecting");
        let socket = match open(&ws_url, &token).await {
            Ok(s) => s,
            Err(Fail::SignedOut) => {
                if let Some(net) = tauri::Manager::try_state::<Net>(&app) {
                    let mut s = net.saved.lock().unwrap();
                    s.token.clear();
                    s.user = None;
                    store(&s);
                    net.tx.lock().unwrap().take();
                }
                app.state::<crate::store::Store>().forget();
                status(&app, "signed_out");
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
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => {
                        let json = serde_json::to_string(&m).expect("messages serialise");
                        if out.send(Ws::Text(json.into())).await.is_err() {
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
                            for more in keep(&app, &server, &m) {
                                let json = serde_json::to_string(&more).expect("messages serialise");
                                if out.send(Ws::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                            // Background history is only for the store.
                            if !matches!(&m, ToApp::History { tag, .. } if tag == BACKFILL) {
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

async fn open(
    url: &str,
    token: &str,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Fail> {
    let mut req = url.into_client_request().map_err(|_| Fail::Other)?;
    req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().map_err(|_| Fail::Other)?);
    match tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(req)).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(r))) if r.status() == 401 => Err(Fail::SignedOut),
        _ => Err(Fail::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_links() {
        assert_eq!(
            parse_invite("xenify://join?server=https%3A%2F%2Fchat.example.org%2F&code=AB12"),
            Some(("https://chat.example.org".into(), "AB12".into()))
        );
        assert_eq!(
            parse_invite("xenify://join/?code=X&server=http://10.0.0.2:7700"),
            Some(("http://10.0.0.2:7700".into(), "X".into()))
        );
        assert_eq!(parse_invite("xenify://join?server=ftp://x&code=1"), None);
        assert_eq!(parse_invite("https://example.org"), None);
        assert_eq!(unescape("a%2"), "a%2");
    }
}
