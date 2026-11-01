//! All integration tests in one test program. Each file used to be its own
//! program linking the whole server (~240 MB each, 31 of them per build); one
//! program links once. Files stay where they are; run one file or test with
//! `cargo test -p xenify-server --test server web::` (module path filter).

#[path = "access.rs"]
mod access;
#[path = "accounts.rs"]
mod accounts;
#[path = "ai.rs"]
mod ai;
#[path = "backup.rs"]
mod backup;
#[path = "chat.rs"]
mod chat;
#[path = "comfy.rs"]
mod comfy;
#[path = "comments.rs"]
mod comments;
#[path = "datasources.rs"]
mod datasources;
#[path = "datasources_safety.rs"]
mod datasources_safety;
#[path = "demo.rs"]
mod demo;
#[path = "files_page.rs"]
mod files_page;
#[path = "files.rs"]
mod files;
#[path = "hub.rs"]
mod hub;
#[path = "live.rs"]
mod live;
#[path = "mail.rs"]
mod mail;
#[path = "mcp.rs"]
mod mcp;
#[path = "media.rs"]
mod media;
#[path = "mfa.rs"]
mod mfa;
#[path = "net.rs"]
mod net;
#[path = "pages.rs"]
mod pages;
#[path = "plan.rs"]
mod plan;
#[path = "publinks.rs"]
mod publinks;
#[path = "push_subs.rs"]
mod push_subs;
#[path = "roles.rs"]
mod roles;
#[path = "scan.rs"]
mod scan;
#[path = "secrets.rs"]
mod secrets;
#[path = "security.rs"]
mod security;
#[path = "service.rs"]
mod service;
#[path = "tls.rs"]
mod tls;
#[path = "usermail.rs"]
mod usermail;
#[path = "web.rs"]
mod web;

/// How many tests use PostgreSQL at once. When every file was its own program
/// they ran one file at a time; in one program all ~100 tests would start
/// together and each server keeps a pool of connections, which runs past
/// PostgreSQL's 100-connection default ("too many clients"). A test takes a
/// slot in `common::fresh_db` and keeps it until its thread ends (the test
/// harness runs every test on its own thread). `XENIFY_TEST_PG_SLOTS` changes
/// the number (default 6).
mod pg_slots {
    use std::cell::RefCell;
    use std::sync::{Condvar, Mutex};

    static USED: Mutex<usize> = Mutex::new(0);
    static FREED: Condvar = Condvar::new();

    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            *USED.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
            FREED.notify_one();
        }
    }

    thread_local! {
        static HELD: RefCell<Option<Slot>> = const { RefCell::new(None) };
    }

    fn limit() -> usize {
        std::env::var("XENIFY_TEST_PG_SLOTS").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0).unwrap_or(6)
    }

    /// Waits for a slot unless this test's thread already holds one.
    pub fn take() {
        HELD.with(|h| {
            if h.borrow().is_some() {
                return;
            }
            let mut used = USED.lock().unwrap_or_else(|e| e.into_inner());
            while *used >= limit() {
                used = FREED.wait(used).unwrap_or_else(|e| e.into_inner());
            }
            *used += 1;
            *h.borrow_mut() = Some(Slot);
        });
    }
}
