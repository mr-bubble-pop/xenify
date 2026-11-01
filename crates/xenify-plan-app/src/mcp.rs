//! MCP (the Model Context Protocol), both ways.
//!
//! - Connecting in: AI programs on this computer use Plan's tools. Off until
//!   the person turns it on (AI › Programs that can use Plan). Plan then listens on a local
//!   port, with a token written to a file only this account can read;
//!   programs start `xenify-plan-app --mcp`, which passes their messages on
//!   (stdio). The open plans live in the page, so each request goes there
//!   (an `mcp-request` event) and the page answers (`mcp_reply`); the page
//!   asks the person before a program's first use, and keeps to the level
//!   they chose for it.
//! - Plan's Ask using other programs' tools: MCP servers started here
//!   (a command, over stdio) or reached by address (Streamable HTTP).

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;

const PROTOCOL: &str = "2025-06-18";
/// The MCP versions this server speaks (newest first).
const SPOKEN: [&str; 3] = [PROTOCOL, "2025-03-26", "2024-11-05"];

/// The version to answer `initialize` with: the program's own when we speak
/// it, else our newest (the program then decides whether it can go on),
/// never one we don't know.
fn negotiate(asked: Option<&str>) -> &'static str {
    SPOKEN.iter().copied().find(|v| Some(*v) == asked).unwrap_or(PROTOCOL)
}

/// Where the running Plan says its port and token: the app's config folder
/// (the same place Tauri puts it), readable by this account only.
pub fn port_file() -> Option<std::path::PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("app.xenify.plan").join("mcp.json"))
}

fn token() -> String {
    let mut b = [0u8; 24];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Clone)]
struct Program {
    name: String,
    since: u64,
    stream: Arc<TcpStream>,
}

#[derive(Default)]
pub struct Mcp {
    stop: Arc<AtomicBool>,
    port: Mutex<Option<u16>>,
    next: AtomicU64,
    /// Requests waiting for the page's answer, by number.
    waiting: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    /// Programs connected now, by connection number.
    programs: Arc<Mutex<HashMap<u64, Program>>>,
    /// MCP servers Ask uses, started here, by name.
    servers: Mutex<HashMap<String, Arc<Local>>>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn reply(id: &Value, out: Result<Value, Value>) -> Value {
    match out {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    }
}

impl Mcp {
    /// Starts listening (or says where it already is): the port.
    pub fn listen(self: &Arc<Self>, app: tauri::AppHandle) -> Result<u16, String> {
        if let Some(p) = *self.port.lock().unwrap() {
            return Ok(p);
        }
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = token();
        let file = port_file().ok_or("no folder for settings")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        write_private(&file, json!({ "port": port, "token": token }).to_string().as_bytes()).map_err(|e| e.to_string())?;
        self.stop.store(false, Ordering::SeqCst);
        *self.port.lock().unwrap() = Some(port);
        let me = self.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if me.stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (me, app, token) = (me.clone(), app.clone(), token.clone());
                std::thread::spawn(move || me.serve(conn, app, &token));
            }
        });
        Ok(port)
    }

    /// Stops listening and lets every program go.
    pub fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(p) = self.port.lock().unwrap().take() {
            // Wakes the accept loop so it sees `stop`.
            let _ = TcpStream::connect(("127.0.0.1", p));
        }
        if let Some(f) = port_file() {
            let _ = std::fs::remove_file(f);
        }
        for (_, p) in self.programs.lock().unwrap().drain() {
            let _ = p.stream.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn programs(&self) -> Vec<Value> {
        self.programs
            .lock()
            .unwrap()
            .iter()
            .map(|(id, p)| json!({ "id": id, "name": p.name, "since": p.since }))
            .collect()
    }

    pub fn drop_program(&self, id: u64) {
        if let Some(p) = self.programs.lock().unwrap().remove(&id) {
            let _ = p.stream.shutdown(std::net::Shutdown::Both);
        }
    }

    /// The page's answer to request `id`.
    pub fn answer(&self, id: u64, out: Value) {
        if let Some(tx) = self.waiting.lock().unwrap().remove(&id) {
            let _ = tx.send(out);
        }
    }

    /// One program's connection: the token first, then one JSON-RPC message a line.
    fn serve(self: Arc<Self>, conn: TcpStream, app: tauri::AppHandle, token: &str) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(write) = conn.try_clone() else { return };
        let write = Arc::new(write);
        let mut lines = BufReader::new(conn).lines();
        let ok = lines
            .next()
            .and_then(|l| l.ok())
            .and_then(|l| serde_json::from_str::<Value>(&l).ok())
            .is_some_and(|v| v["token"] == token);
        let send = |v: &Value| {
            let mut w: &TcpStream = &write;
            let _ = writeln!(w, "{v}");
        };
        if !ok {
            send(&json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32001, "message": "Not allowed" } }));
            return;
        }
        let conn_id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut name = "An AI program".to_string();
        for line in lines {
            let Ok(line) = line else { break };
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(id) = msg.get("id").cloned() else { continue }; // a notification
            let method = msg["method"].as_str().unwrap_or("");
            let out = match method {
                "initialize" => {
                    if let Some(n) = msg["params"]["clientInfo"]["name"].as_str() {
                        name = n.chars().take(80).collect();
                    }
                    self.programs
                        .lock()
                        .unwrap()
                        .insert(conn_id, Program { name: name.clone(), since: now(), stream: write.clone() });
                    let _ = app.emit("mcp-programs", ());
                    Ok(json!({
                        "protocolVersion": negotiate(msg["params"]["protocolVersion"].as_str()),
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "Xenify Plan", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": "Tools for the plans open in Xenify Plan. What you may do (read, propose changes, change) is up to the person at the computer.",
                    }))
                }
                "ping" => Ok(json!({})),
                _ => {
                    // To the page, and wait for its answer (a first use waits for the person).
                    let n = self.next.fetch_add(1, Ordering::SeqCst) + 1;
                    let (tx, rx) = channel();
                    self.waiting.lock().unwrap().insert(n, tx);
                    let _ = app.emit(
                        "mcp-request",
                        json!({ "id": n, "conn": conn_id, "program": name, "method": method, "params": msg["params"] }),
                    );
                    match rx.recv_timeout(Duration::from_secs(600)) {
                        Ok(v) if v.get("error").is_some() => Err(v["error"].clone()),
                        Ok(v) => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                        Err(_) => {
                            self.waiting.lock().unwrap().remove(&n);
                            Err(json!({ "code": -32000, "message": "Xenify Plan didn't answer in time" }))
                        }
                    }
                }
            };
            send(&reply(&id, out));
        }
        self.programs.lock().unwrap().remove(&conn_id);
        let _ = app.emit("mcp-programs", ());
    }

    // ----- Ask using other programs' tools -----

    /// Starts an MCP server (a command), says hello: its tools.
    pub fn start(
        &self,
        name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Value, String> {
        self.end(name);
        let local = Local::spawn(command, args, env)?;
        local.call("initialize", json!({ "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "Xenify Plan", "version": env!("CARGO_PKG_VERSION") } }))?;
        local.notify("notifications/initialized");
        let tools = local.call("tools/list", json!({}))?;
        self.servers.lock().unwrap().insert(name.to_string(), Arc::new(local));
        Ok(tools)
    }

    pub fn call(&self, name: &str, method: &str, params: Value) -> Result<Value, String> {
        let local = self.servers.lock().unwrap().get(name).cloned().ok_or_else(|| format!("{name} isn't running"))?;
        local.call(method, params)
    }

    pub fn end(&self, name: &str) {
        if let Some(l) = self.servers.lock().unwrap().remove(name) {
            l.kill();
        }
    }
}

/// An MCP server started here, spoken to over its stdin and stdout.
pub struct Local {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next: AtomicU64,
    waiting: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
}

impl Local {
    fn spawn(command: &str, args: &[String], env: &HashMap<String, String>) -> Result<Local, String> {
        let mut child = xenify_hub_core::command(command)
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start {command}: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let waiting: Arc<Mutex<HashMap<u64, Sender<Value>>>> = Arc::default();
        let w = waiting.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = v.get("id").and_then(Value::as_u64)
                    && let Some(tx) = w.lock().unwrap().remove(&id)
                {
                    let _ = tx.send(v);
                }
            }
        });
        Ok(Local { child: Mutex::new(child), stdin: Mutex::new(stdin), next: AtomicU64::new(1), waiting })
    }

    fn notify(&self, method: &str) {
        let _ = writeln!(self.stdin.lock().unwrap(), "{}", json!({ "jsonrpc": "2.0", "method": method }));
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        self.waiting.lock().unwrap().insert(id, tx);
        writeln!(
            self.stdin.lock().unwrap(),
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        )
        .map_err(|e| format!("the tool stopped: {e}"))?;
        let v = rx.recv_timeout(Duration::from_secs(120)).map_err(|_| "the tool didn't answer in time".to_string())?;
        match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or("the tool said no").to_string()),
            None => Ok(v["result"].clone()),
        }
    }

    fn kill(&self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

/// One JSON-RPC message to an MCP server by address (Streamable HTTP): its
/// answer, and the session it gave (to send with later messages).
pub fn http(
    url: &str,
    headers: &HashMap<String, String>,
    session: Option<&str>,
    body: &Value,
) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(s) = session {
        req = req.header("Mcp-Session-Id", s);
    }
    let mut r = req.send(body.to_string()).map_err(|e| format!("couldn't reach {url}: {e}"))?;
    let status = r.status().as_u16();
    let session = r.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_string);
    let ctype = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mut text = String::new();
    r.body_mut().as_reader().read_to_string(&mut text).map_err(|e| e.to_string())?;
    if status == 202 {
        return Ok(json!({ "session": session }));
    }
    if status >= 400 {
        return Err(format!("{url} answered {status}"));
    }
    // An event stream: the message that answers ours.
    let trimmed = text.trim_start();
    let msg = if ctype.contains("event-stream") || trimmed.starts_with("event:") || trimmed.starts_with("data:") {
        text.lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
            .find(|v| v.get("id") == body.get("id"))
            .unwrap_or(Value::Null)
    } else {
        serde_json::from_str(&text).map_err(|_| "the tool sent something unexpected".to_string())?
    };
    Ok(json!({ "message": msg, "session": session }))
}

/// `xenify-plan-app --mcp`: passes an AI program's messages (stdin) to the
/// running Plan and its answers back (stdout).
pub fn bridge() -> ! {
    let fail = |msg: &str| -> ! {
        // Every request gets the reason, so the program can show it.
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && let Some(id) = v.get("id")
            {
                println!("{}", json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32002, "message": msg } }));
            }
        }
        std::process::exit(0)
    };
    let Some(conf) =
        port_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        fail("Open Xenify Plan and turn on AI › Programs that can use Plan › Let programs connect")
    };
    let port = conf["port"].as_u64().unwrap_or(0) as u16;
    let Ok(conn) = TcpStream::connect(("127.0.0.1", port)) else {
        fail("Xenify Plan isn't open (or stopped letting AI programs in)")
    };
    let mut out = conn.try_clone().expect("a socket copies");
    let _ = writeln!(out, "{}", json!({ "token": conf["token"] }));
    // Plan → program.
    let back = conn.try_clone().expect("a socket copies");
    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        for line in BufReader::new(back).lines() {
            let Ok(line) = line else { break };
            let mut o = stdout.lock();
            let _ = writeln!(o, "{line}");
            let _ = o.flush();
        }
        std::process::exit(0);
    });
    // Program → Plan.
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
    std::process::exit(0)
}

/// Writes `bytes` to `file` readable by this account only, from the moment
/// it exists (not written first and narrowed after): the old file is
/// removed and a new one made, never following a link put in its place.
fn write_private(file: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if std::fs::symlink_metadata(file).is_ok() {
        std::fs::remove_file(file)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(file)?.write_all(bytes)
}

#[cfg(test)]
mod private_file_tests {
    #[test]
    fn token_file_is_private_from_the_start() {
        let dir = std::env::temp_dir().join(format!("xenify-mcp-test-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mcp.json");
        std::fs::write(&f, "old").unwrap();
        super::write_private(&f, b"{}").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn answers_only_with_a_version_it_speaks() {
        assert_eq!(super::negotiate(Some("2024-11-05")), "2024-11-05");
        assert_eq!(super::negotiate(Some("2031-01-01")), super::PROTOCOL);
        assert_eq!(super::negotiate(None), super::PROTOCOL);
    }
}
