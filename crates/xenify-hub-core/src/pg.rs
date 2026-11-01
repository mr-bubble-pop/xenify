//! The server's own PostgreSQL, for a server run on this computer: the Hub
//! installs it as a part of the server and makes its database once (with a
//! random password, reachable from this computer only); the server's
//! service starts it before the server.

use std::path::{Path, PathBuf};

/// The port it listens on (this computer only).
pub const PORT: u16 = 55433;

pub struct Pg {
    pub bin: PathBuf,
    pub data: PathBuf,
}

fn exe(bin: &Path, name: &str) -> PathBuf {
    bin.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
}

/// Writes a file only its owner can read, from the moment it exists (not
/// made first and narrowed after). A file left from before is replaced.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(bytes)
}

fn run(cmd: &mut std::process::Command, what: &str) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("couldn't {what}: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("couldn't {what}: {}", text.trim().lines().last().unwrap_or("")))
    }
}

impl Pg {
    /// `program`: the installed `bin/postgres`.
    pub fn new(program: &Path, data: PathBuf) -> Option<Self> {
        Some(Self { bin: program.parent()?.to_path_buf(), data })
    }

    /// Makes the database folder once; returns the password (made then).
    pub fn init(&self, password: &str) -> Result<(), String> {
        if self.data.join("PG_VERSION").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.data).map_err(|e| e.to_string())?;
        let pwfile = self.data.with_extension("pw");
        write_private(&pwfile, password.as_bytes()).map_err(|e| e.to_string())?;
        let r = run(
            crate::command(exe(&self.bin, "initdb"))
                .arg("-D")
                .arg(&self.data)
                .args(["-U", "xenify", "-A", "scram-sha-256", "-E", "UTF8", "--no-instructions"])
                .arg(format!("--pwfile={}", pwfile.display())),
            "set up the database",
        );
        let _ = std::fs::remove_file(&pwfile);
        r.map(|_| ())
    }

    pub fn running(&self) -> bool {
        crate::command(exe(&self.bin, "pg_ctl"))
            .arg("-D")
            .arg(&self.data)
            .arg("status")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Starts it (this computer only, no socket files) and waits until it answers.
    pub fn start(&self) -> Result<(), String> {
        if self.running() {
            return Ok(());
        }
        // Its output goes nowhere, and only pg_ctl is waited for: on Windows
        // the database it starts inherits whatever pg_ctl was given, so
        // reading pg_ctl's output waited for as long as the database ran
        // (the Hub never got past starting it). What went wrong is in its log.
        let log = self.data.join("server.log");
        let status = crate::command(exe(&self.bin, "pg_ctl"))
            .arg("-D")
            .arg(&self.data)
            .arg("-l")
            .arg(&log)
            .args(["-w", "-t", "60", "-o", &format!("-p {PORT} -c listen_addresses=127.0.0.1 -k \"\""), "start"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("couldn't start the database: {e}"))?;
        if status.success() {
            return Ok(());
        }
        let said = std::fs::read_to_string(&log).unwrap_or_default();
        let last = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it didn't say why");
        Err(format!("couldn't start the database: {}", last.trim()))
    }

    pub fn stop(&self) {
        let _ = crate::command(exe(&self.bin, "pg_ctl")).arg("-D").arg(&self.data).args(["-m", "fast", "stop"]).output();
    }

    /// Makes the server's database (once). Just after starting, PostgreSQL
    /// can still be getting ready (seen on Windows the first time), so it
    /// tries again for up to 30 seconds.
    pub fn create_db(&self, password: &str) -> Result<(), String> {
        let started = std::time::Instant::now();
        loop {
            let r = run(
                crate::command(exe(&self.bin, "createdb"))
                    .args(["-h", "127.0.0.1", "-p", &PORT.to_string(), "-U", "xenify", "xenify"])
                    .env("PGPASSWORD", password),
                "make the server's database",
            );
            match r {
                Ok(_) => return Ok(()),
                Err(e) if e.contains("already exists") => return Ok(()),
                // A wrong password won't come right by waiting.
                Err(e) if e.contains("password authentication failed") => return Err(e),
                Err(e) if started.elapsed() >= std::time::Duration::from_secs(30) => return Err(e),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
            }
        }
    }

    pub fn url(password: &str) -> String {
        format!("postgres://xenify:{password}@127.0.0.1:{PORT}/xenify")
    }
}

#[cfg(all(test, target_os = "linux"))]
mod start_tests {
    /// A real PostgreSQL (when this computer has one): set up, started,
    /// running, stopped. The start returns once it's up (it once waited
    /// for the database's output to end, which on Windows was never).
    #[test]
    fn starts_and_stops() {
        let program = std::path::Path::new("/usr/lib/postgresql/17/bin/postgres");
        if !program.exists() {
            return;
        }
        let data = std::env::temp_dir().join(format!("xenify-pg-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let pg = super::Pg::new(program, data.clone()).unwrap();
        pg.init("test-password").unwrap();
        let t = std::time::Instant::now();
        pg.start().unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(30), "took {:?}", t.elapsed());
        assert!(pg.running());
        pg.stop();
        assert!(!pg.running());
        let _ = std::fs::remove_dir_all(&data);
    }
}
