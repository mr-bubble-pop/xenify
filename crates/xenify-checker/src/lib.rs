//! The shared checker: one background process that loads the mistake-spotter
//! model once for every Xenify app (Write, the picture editor), instead of
//! each window holding its own copy (about 135 MB each).
//!
//! Apps talk to it over a private local socket with one JSON line per
//! request and answer. The first app to need it starts it; it exits by
//! itself once no app has been connected for [`GRACE`]. Each app keeps one
//! connection open while it uses the model, so the connection is its lease.
//!
//! On systems without Unix sockets [`Client::connect`] fails and the app
//! loads the model itself.

use xenify_grammar_spotter::{Finding, Fix, Spotter};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the checker stays after the last app disconnects, so reopening
/// a window soon after closing one doesn't load the model again.
pub const GRACE: Duration = Duration::from_secs(60);

/// What an app asks.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Load the model folder `model` (if it isn't already).
    Load { model: PathBuf },
    /// The spots in `text`, each with up to `fixes` suggestions, leaving out
    /// the kinds of mistake in `hidden`.
    Spot {
        model: PathBuf,
        text: String,
        threshold: f32,
        fixes: usize,
        #[serde(default)]
        hidden: Vec<String>,
    },
}

/// What the checker answers.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    /// The model is loaded and knows `codes` kinds of mistake.
    Loaded {
        codes: usize,
    },
    Spots {
        spots: Vec<(Finding, Vec<Fix>)>,
    },
    Error {
        message: String,
    },
}

/// The spots in `text`, each with up to `fixes` suggestions (what
/// [`Request::Spot`] answers; also used when the model is loaded in-app).
pub fn spot_with_fixes(
    spotter: &Spotter,
    text: &str,
    threshold: f32,
    fixes: usize,
    hidden: &[String],
) -> Result<Vec<(Finding, Vec<Fix>)>, String> {
    Ok(spotter
        .spot_hiding(text, threshold, hidden)?
        .into_iter()
        .map(|f| {
            let s = spotter.suggest(text, &f, fixes).unwrap_or_default();
            (f, s)
        })
        .collect())
}

/// Where the checker listens: `$XDG_RUNTIME_DIR/xenify/checker.sock`, or a
/// private folder in the temp folder. Refuses a folder that isn't the
/// user's own and private, so other users can't read the text.
#[cfg(unix)]
pub fn socket_path() -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    // Safety: getuid has no preconditions and can't fail.
    let uid = unsafe { libc::getuid() };
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d).join("xenify"),
        None => std::env::temp_dir().join(format!("xenify-{uid}")),
    };
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err(format!("{} isn't a private folder", dir.display()));
    }
    Ok(dir.join("checker.sock"))
}

#[cfg(not(unix))]
pub fn socket_path() -> Result<PathBuf, String> {
    Err("the shared checker needs Unix sockets".into())
}

/// The `xenify-checker` program: `XENIFY_CHECKER`, else next to this
/// program, else on the `PATH`.
#[cfg_attr(not(unix), allow(dead_code))]
fn checker_exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("XENIFY_CHECKER") {
        return Some(p.into());
    }
    let name = format!("xenify-checker{}", std::env::consts::EXE_SUFFIX);
    let here = std::env::current_exe().ok()?.parent()?.join(&name);
    if here.is_file() {
        return Some(here);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&name)).find(|p| p.is_file())
}

/// An app's connection to the shared checker. It reconnects (starting the
/// checker again) if the checker went away.
#[cfg_attr(not(unix), allow(dead_code))]
pub struct Client {
    path: PathBuf,
    start: bool,
    #[cfg(unix)]
    conn: std::sync::Mutex<Option<unix::Conn>>,
}

impl Client {
    /// Connects to the running checker, starting it if needed.
    pub fn connect() -> Result<Self, String> {
        let c = Self::new(socket_path()?, true);
        c.ensure()?;
        Ok(c)
    }

    /// A client of the checker at `path` that never starts one (for tests).
    pub fn at(path: &Path) -> Result<Self, String> {
        let c = Self::new(path.to_path_buf(), false);
        c.ensure()?;
        Ok(c)
    }

    fn new(path: PathBuf, start: bool) -> Self {
        Self {
            path,
            start,
            #[cfg(unix)]
            conn: std::sync::Mutex::new(None),
        }
    }

    /// Loads `model` in the checker; the number of kinds of mistake it knows.
    pub fn load(&self, model: &Path) -> Result<usize, String> {
        match self.request(&Request::Load { model: model.to_path_buf() })? {
            Response::Loaded { codes } => Ok(codes),
            Response::Error { message } => Err(message),
            r => Err(format!("unexpected answer {r:?}")),
        }
    }

    /// Like [`spot_with_fixes`], run by the checker.
    pub fn spot(
        &self,
        model: &Path,
        text: &str,
        threshold: f32,
        fixes: usize,
        hidden: &[String],
    ) -> Result<Vec<(Finding, Vec<Fix>)>, String> {
        let req = Request::Spot {
            model: model.to_path_buf(),
            text: text.to_string(),
            threshold,
            fixes,
            hidden: hidden.to_vec(),
        };
        match self.request(&req)? {
            Response::Spots { spots } => Ok(spots),
            Response::Error { message } => Err(message),
            r => Err(format!("unexpected answer {r:?}")),
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    pub(super) struct Conn {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    impl Conn {
        fn open(stream: UnixStream) -> Result<Self, String> {
            let writer = stream.try_clone().map_err(|e| e.to_string())?;
            Ok(Self { reader: BufReader::new(stream), writer })
        }

        fn exchange(&mut self, req: &Request) -> Result<Response, String> {
            let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
            line.push('\n');
            self.writer.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            let mut answer = String::new();
            if self.reader.read_line(&mut answer).map_err(|e| e.to_string())? == 0 {
                return Err("the checker closed the connection".into());
            }
            serde_json::from_str(&answer).map_err(|e| e.to_string())
        }
    }

    impl Client {
        /// Opens the connection if it isn't open.
        pub(super) fn ensure(&self) -> Result<(), String> {
            let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            if conn.is_none() {
                *conn = Some(self.open()?);
            }
            Ok(())
        }

        fn open(&self) -> Result<Conn, String> {
            if let Ok(s) = UnixStream::connect(&self.path) {
                return Conn::open(s);
            }
            if !self.start {
                return Err(format!("no checker at {}", self.path.display()));
            }
            let exe = checker_exe().ok_or("the xenify-checker program isn't installed")?;
            let mut cmd = std::process::Command::new(&exe);
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            // Its own process group, so closing the app's terminal doesn't stop
            // it while other apps use it.
            std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
            let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
            // Reap it when it exits, so it doesn't linger as a zombie.
            std::thread::spawn(move || child.wait());
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(s) = UnixStream::connect(&self.path) {
                    return Conn::open(s);
                }
                if Instant::now() > until {
                    return Err("the checker didn't start".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        /// Sends `req`, reconnecting once if the checker went away.
        pub(super) fn request(&self, req: &Request) -> Result<Response, String> {
            let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let mut last = String::new();
            for _ in 0..2 {
                if conn.is_none() {
                    *conn = Some(self.open()?);
                }
                match conn.as_mut().expect("just opened").exchange(req) {
                    Ok(r) => return Ok(r),
                    Err(e) => {
                        *conn = None;
                        last = e;
                    }
                }
            }
            Err(last)
        }
    }

    #[derive(Default)]
    struct State {
        clients: AtomicUsize,
        /// The loaded model and its folder. Requests take turns with it, so
        /// two apps checking at once don't fight over the CPU.
        model: Mutex<Option<(PathBuf, Spotter)>>,
    }

    /// Serves apps at `path` until none has been connected for `grace`.
    /// Returns at once if another checker is already serving there.
    pub fn serve(path: &Path, grace: Duration) -> std::io::Result<()> {
        if UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        // A leftover socket from a checker that didn't exit cleanly.
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        let state = Arc::new(State::default());
        let mut idle_since = Some(Instant::now());
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    state.clients.fetch_add(1, Ordering::SeqCst);
                    let state = state.clone();
                    std::thread::spawn(move || {
                        handle(stream, &state);
                        state.clients.fetch_sub(1, Ordering::SeqCst);
                    });
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            if state.clients.load(Ordering::SeqCst) == 0 {
                if idle_since.get_or_insert_with(Instant::now).elapsed() >= grace {
                    break;
                }
            } else {
                idle_since = None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    fn handle(stream: UnixStream, state: &State) {
        let Ok(mut out) = stream.try_clone() else { return };
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let answer = match serde_json::from_str::<Request>(&line) {
                Ok(req) => answer(req, state),
                Err(e) => Response::Error { message: e.to_string() },
            };
            let mut line = serde_json::to_string(&answer).unwrap_or_default();
            line.push('\n');
            if out.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
    }

    fn answer(req: Request, state: &State) -> Response {
        let mut model = state.model.lock().unwrap_or_else(|e| e.into_inner());
        let (Request::Load { model: dir } | Request::Spot { model: dir, .. }) = &req;
        if model.as_ref().is_none_or(|(d, _)| d != dir) {
            match Spotter::load(dir) {
                Ok(s) => *model = Some((dir.clone(), s)),
                Err(e) => return Response::Error { message: e },
            }
        }
        let spotter = &model.as_ref().expect("just loaded").1;
        match req {
            Request::Load { .. } => Response::Loaded { codes: spotter.codes().len() },
            Request::Spot { text, threshold, fixes, hidden, .. } => {
                match spot_with_fixes(spotter, &text, threshold, fixes, &hidden) {
                    Ok(spots) => Response::Spots { spots },
                    Err(message) => Response::Error { message },
                }
            }
        }
    }
}

#[cfg(unix)]
pub use unix::serve;

#[cfg(not(unix))]
impl Client {
    fn ensure(&self) -> Result<(), String> {
        Err("the shared checker needs Unix sockets".into())
    }

    fn request(&self, _: &Request) -> Result<Response, String> {
        self.ensure().map(|_| unreachable!())
    }
}

#[cfg(not(unix))]
pub fn serve(_: &Path, _: Duration) -> std::io::Result<()> {
    Err(std::io::Error::other("the shared checker needs Unix sockets"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xenify-checker-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("checker.sock")
    }

    #[test]
    fn serves_then_exits_when_no_app_is_left() {
        let path = temp_socket("idle");
        let p = path.clone();
        let server = std::thread::spawn(move || serve(&p, Duration::from_millis(300)));
        let until = std::time::Instant::now() + Duration::from_secs(5);
        let client = loop {
            if let Ok(c) = Client::at(&path) {
                break c;
            }
            assert!(std::time::Instant::now() < until, "checker didn't start");
            std::thread::sleep(Duration::from_millis(20));
        };
        // A missing model is an error, not a dropped connection.
        assert!(client.load(Path::new("/no/such/model")).is_err());
        // A second checker at the same place leaves the first one serving.
        serve(&path, Duration::ZERO).unwrap();
        assert!(client.load(Path::new("/no/such/model")).is_err());
        // While an app is connected the checker stays, even past its grace.
        std::thread::sleep(Duration::from_millis(600));
        assert!(!server.is_finished());
        drop(client);
        server.join().unwrap().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn the_shared_model_answers_like_a_local_one() {
        // Set XENIFY_SPOTTER to a model folder to run this.
        let Some(dir) = std::env::var_os("XENIFY_SPOTTER").map(PathBuf::from) else { return };
        let path = temp_socket("model");
        let p = path.clone();
        std::thread::spawn(move || serve(&p, Duration::from_millis(300)));
        std::thread::sleep(Duration::from_millis(300));
        let client = Client::at(&path).unwrap();
        let text = "She go to school every days.";
        let local = spot_with_fixes(&Spotter::load(&dir).unwrap(), text, 0.5, 3, &[]).unwrap();
        assert!(!local.is_empty());
        assert_eq!(client.spot(&dir, text, 0.5, 3, &[]).unwrap(), local);
        assert!(client.load(&dir).unwrap() > 0);
    }
}
