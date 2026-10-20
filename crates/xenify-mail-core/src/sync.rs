//! Keeping an account's local copy up to date: one background thread per
//! account with its own IMAP connection. It brings in new mail and flag
//! changes, downloads messages when you open them, does what you ask
//! (read/unread, flag, move, delete), and then waits for news (IDLE).

use crate::account::Account;
use crate::imap::{Client, ImapError, Role};
use crate::store::Store;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// What the app asks the worker to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Check everything now.
    SyncNow,
    /// Download a message (it's shown when `Event::Downloaded` comes).
    Download {
        folder: String,
        uid: u32,
    },
    SetFlag {
        folder: String,
        uid: u32,
        flag: String,
        on: bool,
    },
    Move {
        folder: String,
        uid: u32,
        to: String,
    },
    /// Delete: move to Trash (or delete for good when already in Trash).
    Delete {
        folder: String,
        uid: u32,
    },
    Stop,
}

/// What the worker tells the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Short progress text ("Checking Inbox…"), or empty when idle.
    Status(String),
    /// Folders or messages changed: read the store again.
    Changed,
    Downloaded {
        folder: String,
        uid: u32,
    },
    /// Couldn't connect or a request failed (it will retry).
    Problem(String),
    /// The password was refused: stopped until the user fixes it.
    SignInFailed(String),
}

/// The app's end of a worker.
pub struct Handle {
    tx: Sender<Cmd>,
    pub events: Receiver<Event>,
    wake: Arc<AtomicBool>,
}

impl Handle {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
        // Interrupt IDLE so the command runs now.
        self.wake.store(true, Ordering::Relaxed);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.send(Cmd::Stop);
    }
}

/// How many of a folder's newest messages are listed at first.
pub const FIRST_SYNC: usize = 2000;
/// How long to wait for news before checking everything again.
const IDLE_FOR: Duration = Duration::from_secs(9 * 60);
/// Other folders are checked this often.
const OTHERS_EVERY: Duration = Duration::from_secs(10 * 60);

/// Starts syncing `account` into the store at `store_path`.
pub fn start(account: Account, password: String, store_path: PathBuf) -> Handle {
    let (tx, rx) = channel();
    let (etx, events) = channel();
    let wake = Arc::new(AtomicBool::new(false));
    let w = wake.clone();
    std::thread::Builder::new()
        .name(format!("mail-sync {}", account.email))
        .spawn(move || {
            let store = match Store::open(&store_path) {
                Ok(s) => s,
                Err(e) => {
                    let _ = etx.send(Event::Problem(format!("Couldn't open the mail store: {e}")));
                    return;
                }
            };
            Worker { account, password, store, rx, events: etx, wake: w, last_others: None }.run();
        })
        .expect("thread");
    Handle { tx, events, wake }
}

struct Worker {
    account: Account,
    password: String,
    store: Store,
    rx: Receiver<Cmd>,
    events: Sender<Event>,
    wake: Arc<AtomicBool>,
    last_others: Option<Instant>,
}

enum Next {
    Continue,
    Stop,
}

impl Worker {
    fn tell(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn run(mut self) {
        let mut backoff = Duration::from_secs(5);
        loop {
            match self.session() {
                Ok(Next::Stop) => return,
                Ok(Next::Continue) => backoff = Duration::from_secs(5),
                Err(ImapError::Auth(m)) => {
                    self.tell(Event::SignInFailed(m));
                    // Wait for Stop (the app restarts the worker with a new password).
                    while let Ok(c) = self.rx.recv() {
                        if c == Cmd::Stop {
                            return;
                        }
                    }
                    return;
                }
                Err(e) => {
                    self.tell(Event::Problem(e.to_string()));
                    self.tell(Event::Status(String::new()));
                    // Wait before reconnecting, but still stop or retry on request.
                    let until = Instant::now() + backoff;
                    while Instant::now() < until {
                        match self.rx.recv_timeout(Duration::from_millis(250)) {
                            Ok(Cmd::Stop) => return,
                            Ok(Cmd::SyncNow) => break,
                            _ => {}
                        }
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(300));
                }
            }
        }
    }

    /// One connection's life: sync, then idle and run commands until an error.
    fn session(&mut self) -> Result<Next, ImapError> {
        self.tell(Event::Status(format!("Connecting to {}…", self.account.imap.host)));
        let mut c = Client::connect(&self.account.imap)?;
        c.login(&self.account.user, &self.password)?;
        let folders = c.list()?;
        self.store.set_folders(&folders).map_err(db)?;
        self.tell(Event::Changed);
        self.sync_all(&mut c, true)?;
        loop {
            // Cleared before looking at the queue: a command sent after this
            // wakes IDLE straight away, so none waits for the next check.
            self.wake.store(false, Ordering::Relaxed);
            while let Ok(cmd) = self.rx.try_recv() {
                if let Next::Stop = self.command(&mut c, cmd)? {
                    c.logout();
                    return Ok(Next::Stop);
                }
            }
            self.tell(Event::Status(String::new()));
            let inbox = self.inbox();
            let sel = c.select(&inbox, false)?;
            // Mail that came since the last check (before IDLE starts) is
            // only in SELECT's counts: catch up first.
            let known = self.store.folders().map_err(db)?.into_iter().find(|f| f.wire == inbox);
            if known.is_none_or(|f| f.uid_next != sel.uid_next || f.total != sel.exists) {
                self.sync_folder(&mut c, &inbox)?;
                self.tell(Event::Changed);
                continue;
            }
            let news = c.idle(IDLE_FOR, &self.wake)?;
            if news {
                self.sync_folder(&mut c, &inbox)?;
                self.tell(Event::Changed);
            } else if !self.wake.load(Ordering::Relaxed) {
                // Quiet for a while: check everything.
                let others = self.last_others.is_none_or(|t| t.elapsed() > OTHERS_EVERY);
                self.sync_all(&mut c, others)?;
            }
        }
    }

    fn inbox(&self) -> String {
        self.store.folder_with(Role::Inbox).ok().flatten().unwrap_or_else(|| "INBOX".into())
    }

    fn command(&mut self, c: &mut Client, cmd: Cmd) -> Result<Next, ImapError> {
        match cmd {
            Cmd::Stop => return Ok(Next::Stop),
            Cmd::SyncNow => self.sync_all(c, true)?,
            Cmd::Download { folder, uid } => {
                c.select(&folder, true)?;
                if let Some(raw) = c.fetch_message(uid)? {
                    self.store.set_raw(&folder, uid, &raw).map_err(db)?;
                }
                self.tell(Event::Downloaded { folder, uid });
            }
            Cmd::SetFlag { folder, uid, flag, on } => {
                c.select(&folder, false)?;
                c.store_flags(&uid.to_string(), on, &[&flag])?;
                self.store.toggle_flag(&folder, uid, &flag, on).map_err(db)?;
                self.tell(Event::Changed);
            }
            Cmd::Move { folder, uid, to } => {
                c.select(&folder, false)?;
                c.move_to(&uid.to_string(), &to)?;
                self.store.remove(&folder, &[uid]).map_err(db)?;
                self.sync_folder(c, &to)?;
                self.tell(Event::Changed);
            }
            Cmd::Delete { folder, uid } => {
                let trash = self.store.folder_with(Role::Trash).map_err(db)?;
                c.select(&folder, false)?;
                match trash.filter(|t| *t != folder) {
                    Some(t) => {
                        c.move_to(&uid.to_string(), &t)?;
                        self.store.remove(&folder, &[uid]).map_err(db)?;
                        self.sync_folder(c, &t)?;
                    }
                    None => {
                        c.store_flags(&uid.to_string(), true, &["\\Deleted"])?;
                        if c.has("UIDPLUS") {
                            c.uid_expunge(&uid.to_string())?;
                        } else {
                            c.expunge()?;
                        }
                    }
                }
                self.store.remove(&folder, &[uid]).map_err(db)?;
                self.tell(Event::Changed);
            }
        }
        Ok(Next::Continue)
    }

    /// The Inbox, then (when `others`) every other folder.
    fn sync_all(&mut self, c: &mut Client, others: bool) -> Result<(), ImapError> {
        let inbox = self.inbox();
        self.sync_folder(c, &inbox)?;
        self.tell(Event::Changed);
        if others {
            let folders = self.store.folders().map_err(db)?;
            // Sent and Drafts first, then the rest in the server's order.
            let mut order: Vec<_> = folders.into_iter().filter(|f| f.wire != inbox).collect();
            order.sort_by_key(|f| match f.role {
                Some(Role::Sent) => 0,
                Some(Role::Drafts) => 1,
                Some(Role::All) => 9, // Gmail's All Mail repeats everything
                _ => 5,
            });
            for f in order {
                // Commands wait at most one folder.
                if self.wake.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = self.sync_folder(c, &f.wire) {
                    match e {
                        ImapError::No(_) => continue, // e.g. a folder we may not open
                        e => return Err(e),
                    }
                }
                self.tell(Event::Changed);
            }
            self.last_others = Some(Instant::now());
        }
        Ok(())
    }

    /// New messages, changed flags and deletions in one folder.
    fn sync_folder(&mut self, c: &mut Client, wire: &str) -> Result<(), ImapError> {
        let name =
            self.store.folders().map_err(db)?.into_iter().find(|f| f.wire == wire).map(|f| f.name).unwrap_or_default();
        self.tell(Event::Status(format!("Checking {name}…")));
        let sel = c.select(wire, true)?;
        self.store.set_folder_state(wire, sel.uid_validity, sel.uid_next).map_err(db)?;
        if sel.exists == 0 {
            self.store.keep_only(wire, &[]).map_err(db)?;
            return Ok(());
        }
        let server = c.uid_search("ALL")?;
        let last = self.store.last_uid(wire).map_err(db)?;
        let mine = self.store.uids(wire).map_err(db)?;
        // New: above what we have; on the first sync only the newest ones.
        let mut new: Vec<u32> = server.iter().copied().filter(|u| *u > last).collect();
        if mine.is_empty() && new.len() > FIRST_SYNC {
            new.drain(..new.len() - FIRST_SYNC);
        }
        for chunk in new.chunks(250) {
            let list = c.fetch_summaries(&uid_set(chunk))?;
            self.store.add_summaries(wire, &list).map_err(db)?;
        }
        // Gone: deleted or moved elsewhere.
        self.store.keep_only(wire, &server).map_err(db)?;
        // Flags changed elsewhere (read on the phone, flagged on the web).
        let known: Vec<u32> = mine.into_iter().filter(|u| server.binary_search(u).is_ok()).collect();
        for chunk in known.chunks(1000) {
            for (uid, flags) in c.fetch_flags(&uid_set(chunk))? {
                self.store.set_flags(wire, uid, &flags).map_err(db)?;
            }
        }
        Ok(())
    }
}

fn db(e: rusqlite::Error) -> ImapError {
    ImapError::Io(format!("mail store: {e}"))
}

/// `[1,2,3,7,9,10]` → `1:3,7,9:10`.
pub fn uid_set(uids: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < uids.len() {
        let start = uids[i];
        let mut end = start;
        while i + 1 < uids.len() && uids[i + 1] == end + 1 {
            i += 1;
            end = uids[i];
        }
        out.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        i += 1;
    }
    out.join(",")
}

#[cfg(test)]
mod tests {
    #[test]
    fn uid_sets_are_compact() {
        assert_eq!(super::uid_set(&[1, 2, 3, 7, 9, 10]), "1:3,7,9:10");
        assert_eq!(super::uid_set(&[5]), "5");
        assert_eq!(super::uid_set(&[]), "");
    }
}
