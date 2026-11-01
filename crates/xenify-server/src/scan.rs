//! Virus scanning of what people upload (Admin › Network › Virus scanning):
//! off unless an admin turns it on. Each upload goes to ClamAV's scanner
//! (clamd, a separate program on this computer or the network, reached
//! over TCP or its unix socket with its INSTREAM command) before it's kept.
//! A file with a virus in it is refused, naming what was found; while the
//! scanner doesn't answer, uploads are refused too (nothing gets in
//! unscanned), and the admin pages say why.

use crate::{Server, admin_power, db, err, failed, unavailable};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::GenericClient;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub fn routes() -> Router<Server> {
    Router::new().route("/v1/admin/scan", get(get_scan).post(set_scan)).route("/v1/admin/scan/test", post(test_scan))
}

/// The `scan` setting.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Scan {
    pub on: bool,
    /// clamd's address: host:port (TCPSocket), or the path of its unix
    /// socket (LocalSocket, e.g. /run/clamav/clamd.ctl).
    pub address: String,
}

/// ClamAV's harmless test file: every scanner reports it.
const EICAR: &[u8] = br"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

/// What clamd answers for one stream.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Clean,
    Found(String),
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// One INSTREAM conversation with clamd.
pub struct Stream {
    io: Box<dyn Io>,
    /// clamd closed the connection (the stream is over its limit): the
    /// rest isn't sent, and its answer says why.
    closed: bool,
}

const CHUNK: usize = 64 * 1024;

impl Stream {
    pub async fn start(address: &str) -> Result<Stream, String> {
        let address = address.trim();
        if address.is_empty() {
            return Err("no scanner address is set".into());
        }
        let connect = async {
            let io: Box<dyn Io> = if address.starts_with('/') {
                #[cfg(unix)]
                {
                    Box::new(tokio::net::UnixStream::connect(address).await?)
                }
                #[cfg(not(unix))]
                {
                    return Err(std::io::Error::other("unix sockets need Linux or a Mac"));
                }
            } else {
                Box::new(tokio::net::TcpStream::connect(address).await?)
            };
            Ok::<_, std::io::Error>(io)
        };
        let mut io = match tokio::time::timeout(Duration::from_secs(5), connect).await {
            Ok(Ok(io)) => io,
            Ok(Err(e)) => return Err(format!("can't reach the scanner at {address}: {e}")),
            Err(_) => return Err(format!("the scanner at {address} didn't answer")),
        };
        io.write_all(b"zINSTREAM\0").await.map_err(|e| format!("the scanner stopped: {e}"))?;
        Ok(Stream { io, closed: false })
    }

    /// Sends the next bytes (in chunks clamd takes).
    pub async fn feed(&mut self, data: &[u8]) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        for part in data.chunks(CHUNK) {
            let len = (part.len() as u32).to_be_bytes();
            let sent = async {
                self.io.write_all(&len).await?;
                self.io.write_all(part).await
            };
            // clamd closes the connection once a stream is over its
            // StreamMaxLength: its answer says so.
            if sent.await.is_err() {
                self.closed = true;
                return Ok(());
            }
        }
        Ok(())
    }

    /// Ends the stream and reads clamd's verdict.
    pub async fn finish(mut self) -> Result<Verdict, String> {
        let _ = self.io.write_all(&[0, 0, 0, 0]).await;
        let mut answer = Vec::new();
        match tokio::time::timeout(Duration::from_secs(120), self.io.read_to_end(&mut answer)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if answer.is_empty() => return Err(format!("the scanner stopped: {e}")),
            Ok(Err(_)) => {}
            Err(_) => return Err("the scanner took too long".into()),
        }
        verdict(&String::from_utf8_lossy(&answer))
    }
}

/// "stream: OK", "stream: <name> FOUND", or an error.
fn verdict(answer: &str) -> Result<Verdict, String> {
    let a = answer.trim_end_matches(['\0', '\n', '\r']).trim();
    let a = a.strip_prefix("stream:").map(str::trim).unwrap_or(a);
    if a == "OK" {
        Ok(Verdict::Clean)
    } else if let Some(name) = a.strip_suffix("FOUND") {
        Ok(Verdict::Found(name.trim().to_string()))
    } else if a.contains("size limit exceeded") {
        Err("the file is bigger than the scanner takes (its StreamMaxLength)".into())
    } else if a.is_empty() {
        Err("the scanner gave no answer".into())
    } else {
        Err(format!("the scanner said: {a}"))
    }
}

/// Scans `parts` (in order, as one file) at `address`.
pub async fn scan(address: &str, parts: &[&[u8]]) -> Result<Verdict, String> {
    let mut s = Stream::start(address).await?;
    for p in parts {
        s.feed(p).await?;
    }
    s.finish().await
}

async fn settings(c: &impl GenericClient) -> Scan {
    db::setting(c, "scan").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// The last thing worth an admin's attention (shown under the setting).
async fn note(c: &impl GenericClient, ok: bool, text: &str) {
    let v = json!({ "at": chrono::Utc::now().to_rfc3339(), "ok": ok, "text": text }).to_string();
    let _ = db::set_setting(c, "scan_status", &v).await;
}

/// Whether an upload may be kept: always, while scanning is off; else only
/// when the scanner says it's clean. `what` names it in the log.
pub(crate) async fn allow(c: &impl GenericClient, what: &str, parts: &[&[u8]]) -> Result<(), Response> {
    let set = settings(c).await;
    if !set.on {
        return Ok(());
    }
    judge(c, what, scan(&set.address, parts).await).await
}

/// What the scanner's answer means for the upload.
async fn judge(c: &impl GenericClient, what: &str, answer: Result<Verdict, String>) -> Result<(), Response> {
    match answer {
        Ok(Verdict::Clean) => Ok(()),
        Ok(Verdict::Found(name)) => {
            eprintln!("xenify-server: virus scan: refused {what}: {name}");
            note(c, true, &format!("Refused {what}: {name}")).await;
            Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "virus",
                &format!("this file has a virus in it ({name}), so it wasn't kept"),
            ))
        }
        Err(e) => {
            eprintln!("xenify-server: virus scan: couldn't scan {what}: {e}");
            note(c, false, &format!("Uploads are refused: {e}")).await;
            Err(err(
                StatusCode::SERVICE_UNAVAILABLE,
                "scan_down",
                "the virus scanner isn't working, so nothing can be uploaded until an admin fixes it",
            ))
        }
    }
}

/// As `allow`, for a file kept in parts on disk.
/// The files are read a chunk at a time (clips can be many GB), never
/// whole.
pub(crate) async fn allow_files(c: &impl GenericClient, what: &str, files: &[std::path::PathBuf]) -> Result<(), Response> {
    let set = settings(c).await;
    if !set.on {
        return Ok(());
    }
    let mut stream = match Stream::start(&set.address).await {
        Ok(s) => s,
        Err(e) => return judge(c, what, Err(e)).await,
    };
    if feed_files(&mut stream, files).await.is_err() {
        return Err(failed());
    }
    judge(c, what, stream.finish().await).await
}

/// Sends `files` (in order, as one file) a chunk at a time.
async fn feed_files(stream: &mut Stream, files: &[std::path::PathBuf]) -> std::io::Result<()> {
    let mut buf = vec![0u8; 1 << 20];
    for f in files {
        let mut f = tokio::fs::File::open(f).await?;
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 || stream.closed {
                break;
            }
            let _ = stream.feed(&buf[..n]).await;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ScanOut {
    #[serde(flatten)]
    scan: Scan,
    status: Option<serde_json::Value>,
}

async fn get_scan(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let status = db::setting(&c, "scan_status").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok());
    Json(ScanOut { scan: settings(&c).await, status }).into_response()
}

/// Saves the setting. Turning it on checks the scanner first, so uploads
/// don't start failing by surprise.
async fn set_scan(State(s): State<Server>, h: HeaderMap, Json(mut v): Json<Scan>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    v.address = v.address.trim().to_string();
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if v.on {
        if let Err(e) = check(&v.address).await {
            return err(StatusCode::BAD_REQUEST, "scan_down", &format!("not turned on: {e}"));
        }
        note(&c, true, "Turned on; the scanner answers").await;
    }
    match db::set_setting(&c, "scan", &serde_json::to_string(&v).expect("serialises")).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

/// Sends the test file: a working scanner finds it.
async fn check(address: &str) -> Result<(), String> {
    match scan(address, &[EICAR]).await? {
        Verdict::Found(_) => Ok(()),
        Verdict::Clean => Err("the scanner answered but didn't find the test file: is it ClamAV?".into()),
    }
}

#[derive(Deserialize)]
struct Test {
    #[serde(default)]
    address: Option<String>,
}

async fn test_scan(State(s): State<Server>, h: HeaderMap, Json(t): Json<Test>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let address = match t.address {
        Some(a) if !a.trim().is_empty() => a,
        _ => settings(&c).await.address,
    };
    match check(&address).await {
        Ok(()) => {
            note(&c, true, "Tested: the scanner found the test file").await;
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => {
            note(&c, false, &format!("Test failed: {e}")).await;
            Json(json!({ "ok": false, "error": e })).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read() {
        assert_eq!(verdict("stream: OK\0"), Ok(Verdict::Clean));
        assert_eq!(verdict("stream: Eicar-Test-Signature FOUND\0"), Ok(Verdict::Found("Eicar-Test-Signature".into())));
        assert!(verdict("INSTREAM size limit exceeded. ERROR\0").unwrap_err().contains("StreamMaxLength"));
        assert!(verdict("").is_err());
    }

    /// Files on disk go to the scanner a chunk at a time, all of them.
    #[tokio::test]
    async fn files_are_streamed() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = l.local_addr().unwrap().to_string();
        let clamd = tokio::spawn(async move {
            let (mut io, _) = l.accept().await.unwrap();
            let mut cmd = [0u8; 10];
            io.read_exact(&mut cmd).await.unwrap();
            assert_eq!(&cmd, b"zINSTREAM\0");
            let (mut total, mut largest) = (0usize, 0usize);
            loop {
                let mut len = [0u8; 4];
                io.read_exact(&mut len).await.unwrap();
                let n = u32::from_be_bytes(len) as usize;
                if n == 0 {
                    break;
                }
                let mut chunk = vec![0u8; n];
                io.read_exact(&mut chunk).await.unwrap();
                total += n;
                largest = largest.max(n);
            }
            io.write_all(b"stream: OK\0").await.unwrap();
            (total, largest)
        });
        let dir = std::env::temp_dir().join(format!("xenify-scan-{}", crate::random(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, vec![1u8; 3 << 20]).unwrap();
        std::fs::write(&b, vec![2u8; 1000]).unwrap();
        let mut stream = Stream::start(&address).await.unwrap();
        feed_files(&mut stream, &[a, b]).await.unwrap();
        assert_eq!(stream.finish().await, Ok(Verdict::Clean));
        assert_eq!(clamd.await.unwrap(), ((3 << 20) + 1000, CHUNK));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
