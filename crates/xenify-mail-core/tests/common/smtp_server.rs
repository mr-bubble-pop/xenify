//! A tiny SMTP server for tests (plain TCP on 127.0.0.1): accepts
//! AUTH PLAIN/LOGIN for one user and keeps what it receives.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct Received {
    pub from: String,
    pub to: Vec<String>,
    pub data: String,
}

pub type Inbox = Arc<Mutex<Vec<Received>>>;

pub fn start(user: &str, password: &str) -> (u16, Inbox) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let inbox: Inbox = Arc::default();
    let (user, password) = (user.to_string(), password.to_string());
    let got = inbox.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (got, user, password) = (got.clone(), user.clone(), password.clone());
            std::thread::spawn(move || {
                let mut out = conn.try_clone().unwrap();
                let mut rd = BufReader::new(conn);
                let _ = write!(out, "220 test ESMTP\r\n");
                let mut msg = Received::default();
                let mut authed = false;
                loop {
                    let mut line = String::new();
                    if rd.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let upper = line.to_ascii_uppercase();
                    let reply = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        "250-test\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME".to_string()
                    } else if upper.starts_with("AUTH PLAIN") {
                        use base64::Engine;
                        let token = line.split_whitespace().nth(2).unwrap_or("");
                        let d = base64::engine::general_purpose::STANDARD.decode(token).unwrap_or_default();
                        let parts: Vec<&[u8]> = d.split(|b| *b == 0).collect();
                        authed = parts.len() == 3 && parts[1] == user.as_bytes() && parts[2] == password.as_bytes();
                        if authed { "235 ok".into() } else { "535 5.7.8 Authentication failed".into() }
                    } else if upper.starts_with("MAIL FROM") {
                        if !authed {
                            "530 authenticate first".into()
                        } else {
                            msg.from = line[10..].trim().to_string();
                            "250 ok".into()
                        }
                    } else if upper.starts_with("RCPT TO") {
                        msg.to.push(line[8..].trim().to_string());
                        "250 ok".into()
                    } else if upper.starts_with("DATA") {
                        let _ = write!(out, "354 go\r\n");
                        let mut data = String::new();
                        loop {
                            let mut l = String::new();
                            if rd.read_line(&mut l).unwrap_or(0) == 0 {
                                return;
                            }
                            if l == ".\r\n" {
                                break;
                            }
                            data.push_str(l.strip_prefix('.').filter(|_| l.starts_with("..")).unwrap_or(&l));
                        }
                        msg.data = data;
                        got.lock().unwrap().push(std::mem::take(&mut msg));
                        "250 queued".into()
                    } else if upper.starts_with("QUIT") {
                        let _ = write!(out, "221 bye\r\n");
                        return;
                    } else {
                        "250 ok".into()
                    };
                    let _ = write!(out, "{reply}\r\n");
                }
            });
        }
    });
    (port, inbox)
}
