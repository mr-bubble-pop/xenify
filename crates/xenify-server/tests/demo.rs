//! A demo world for the site's pictures and films: a server with a small
//! company ("Riverside Studio") in it — people and a team, chats, a meeting
//! and bookings, email, a handbook wiki with documents, a Calc book and a
//! Draw canvas, templates and two plans. It keeps running until killed:
//!
//! `XENIFY_DEMO_APPS=<dir with chat/ plan/ write/ calc/ draw/ shared/> \
//!  cargo test -p xenify-server --test demo world -- --ignored --nocapture`
//!
//! prints one line `DEMO {"addr": …, "people": {<user>: {"token", "account"}}}`
//! once everything is in (sign in a browser by putting a person's
//! {token, account} into localStorage "xenify-session" on the address).

mod common;

#[allow(dead_code)]
#[path = "../../xenify-mail-core/tests/common/imap_server.rs"]
mod imap_server;
#[allow(dead_code)]
#[path = "../../xenify-mail-core/tests/common/smtp_server.rs"]
mod smtp_server;

use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::io::Write as _;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use xenify_chat_proto::{ToApp, ToServer};

const DOMAIN: &str = "riverside.example";
const HOUR: i64 = 3_600_000;
const DAY: i64 = 86_400_000;

// ---------------------------------------------------------------- times

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The calendar date of `days` since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Days since 1970 today (UTC).
fn today() -> i64 {
    now_ms().div_euclid(DAY)
}

/// "2026-11-04" for `days` since 1970.
fn ymd(days: i64) -> String {
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Monday 0 … Sunday 6.
fn weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

/// The working day `n` working days from `days` (n may be negative).
fn work_day(days: i64, n: i64) -> i64 {
    let mut d = days;
    let mut left = n.abs();
    while weekday(d) >= 5 {
        d += 1;
    }
    while left > 0 {
        d += n.signum();
        if weekday(d) < 5 {
            left -= 1;
        }
    }
    d
}

/// An email's Date header for `ms`.
fn mail_date(ms: i64) -> String {
    let days = ms.div_euclid(DAY);
    let (y, m, d) = civil(days);
    let secs = ms.rem_euclid(DAY) / 1000;
    let wd = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][weekday(days) as usize];
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m as usize - 1];
    format!("{wd}, {d:02} {mon} {y} {:02}:{:02}:{:02} +0000", secs / 3600, secs / 60 % 60, secs % 60)
}

// ---------------------------------------------------------------- chat

struct Chat {
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    n: u32,
}

impl Chat {
    async fn connect(addr: &str, token: &str) -> Chat {
        let mut req = format!("ws://{addr}/v1/ws").into_client_request().unwrap();
        req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let mut c = Chat { ws, n: 0 };
        c.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
        c
    }
    async fn send(&mut self, m: ToServer) {
        self.ws.send(Ws::Text(serde_json::to_string(&m).unwrap().into())).await.unwrap();
    }
    async fn wait(&mut self, ok: impl Fn(&ToApp) -> bool) -> ToApp {
        loop {
            let f = tokio::time::timeout(Duration::from_secs(15), self.ws.next())
                .await
                .expect("an answer from Chat")
                .unwrap()
                .unwrap();
            if let Ws::Text(t) = f
                && let Ok(m) = serde_json::from_str::<ToApp>(&t)
            {
                if let ToApp::Error { .. } = &m {
                    eprintln!("chat said: {t}");
                }
                if ok(&m) {
                    return m;
                }
            }
        }
    }
    /// Sends `text` in `conv`; its seq.
    async fn say(&mut self, conv: i64, text: &str) -> i64 {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        self.n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let client_id = format!("demo-{}", self.n);
        self.send(ToServer::Send { conv, client_id: client_id.clone(), text: text.into() }).await;
        let m =
            self.wait(|m| matches!(m, ToApp::Message { msg } if msg.conv == conv && msg.client_id == client_id)).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        match m {
            ToApp::Message { msg } => msg.seq,
            _ => unreachable!(),
        }
    }
    /// The conversation just made (by title, purpose or a member).
    async fn made(&mut self, title: &str) -> i64 {
        let m = self
            .wait(|m| match m {
                ToApp::Conversation { conv } => {
                    conv.title == title
                        || conv.purpose == title
                        || (conv.kind == xenify_chat_proto::Kind::Direct
                            && conv.members.iter().any(|u| u.username == title))
                }
                _ => false,
            })
            .await;
        match m {
            ToApp::Conversation { conv } => conv.id,
            _ => unreachable!(),
        }
    }
    async fn react(&mut self, conv: i64, seq: i64, emoji: &str) {
        self.send(ToServer::React { conv, seq, emoji: emoji.into(), on: true }).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

// ---------------------------------------------------------------- files

async fn raw(addr: &str, method: &str, path: &str, token: &str, body: &[u8]) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let out = String::from_utf8_lossy(&out).to_string();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    let text = out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("");
    (status, serde_json::from_str(text).unwrap_or(Value::Null))
}

/// A new file (or folder) with details `meta`; its id.
async fn node(addr: &str, token: &str, space: i64, parent: Option<i64>, kind: &str, meta: Value) -> i64 {
    let body = json!({ "space": space, "parent": parent, "kind": kind, "meta": meta.to_string() });
    let (s, n) = post(addr, "/v1/nodes", Some(token), body).await;
    assert_eq!(s, 200, "{n}");
    n["id"].as_i64().unwrap()
}

/// Saves `bytes` as the file's newest version.
async fn save(addr: &str, token: &str, id: i64, bytes: &[u8]) {
    let (s, v) = raw(addr, "POST", &format!("/v1/nodes/{id}/content"), token, bytes).await;
    assert_eq!(s, 200, "saving {id}: {v}");
}

/// A Write document from Markdown.
fn write_doc(md: &str) -> Vec<u8> {
    let loaded = xenify_wp_io::load(md.trim_start().as_bytes(), xenify_wp_io::Format::Markdown).unwrap();
    xenify_wp_io::save(&loaded.doc, xenify_wp_io::Format::Igdoc).unwrap()
}

/// A Write document (or a page of a wiki, at `order`) made and saved.
#[allow(clippy::too_many_arguments)]
async fn document(
    addr: &str,
    token: &str,
    space: i64,
    parent: Option<i64>,
    name: &str,
    page: Option<i64>,
    md: &str,
) -> i64 {
    let mut meta = json!({ "name": name, "app": "write", "ext": "xwrite" });
    if let Some(order) = page {
        meta["page"] = json!(true);
        meta["order"] = json!(order);
    }
    let id = node(addr, token, space, parent, "file", meta).await;
    save(addr, token, id, &write_doc(md)).await;
    // The words, for search (as the apps send them).
    raw(addr, "PUT", &format!("/v1/nodes/{id}/text"), token, md.as_bytes()).await;
    id
}

fn zip_of(parts: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes, squeeze) in parts {
        let method = if *squeeze { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored };
        z.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(method)).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// "Sales 2026": a table of months with totals, as a .xcalc file, and
/// what other documents show of it (link.json, also the file's picture).
fn sales_book() -> (Vec<u8>, Vec<u8>) {
    let months = [
        ("January", 18_400, 6_000, 2),
        ("February", 16_900, 6_000, 1),
        ("March", 22_750, 6_500, 3),
        ("April", 24_300, 6_500, 2),
        ("May", 21_800, 7_200, 2),
        ("June", 27_650, 7_200, 4),
        ("July", 19_200, 7_200, 1),
        ("August", 15_400, 7_200, 1),
        ("September", 29_900, 8_100, 3),
        ("October", 31_250, 8_100, 3),
    ];
    let mut cells = Vec::new();
    let head = ["Month", "Projects", "Retainers", "Total", "New clients"];
    for (c, h) in head.iter().enumerate() {
        cells.push(json!({ "r": 1, "c": c + 1, "s": h }));
    }
    let mut rows = vec![head.iter().map(|s| s.to_string()).collect::<Vec<_>>()];
    let money = |n: i64| {
        let s = n.to_string();
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        format!("£{out}")
    };
    let (mut tp, mut tr, mut tn) = (0, 0, 0);
    for (i, (m, p, r, n)) in months.iter().enumerate() {
        let row = i + 2;
        cells.push(json!({ "r": row, "c": 1, "s": m }));
        cells.push(json!({ "r": row, "c": 2, "n": *p as f64, "st": 1 }));
        cells.push(json!({ "r": row, "c": 3, "n": *r as f64, "st": 1 }));
        cells.push(json!({ "r": row, "c": 4, "f": format!("=B{row}+C{row}"), "st": 1 }));
        cells.push(json!({ "r": row, "c": 5, "n": *n as f64 }));
        rows.push(vec![m.to_string(), money(*p), money(*r), money(p + r), n.to_string()]);
        (tp, tr, tn) = (tp + p, tr + r, tn + n);
    }
    let last = months.len() + 2;
    cells.push(json!({ "r": last, "c": 1, "s": "Total" }));
    for (c, col, f) in [(2, "Projects", 109), (3, "Retainers", 109), (4, "Total", 109), (5, "New clients", 109)] {
        let st = if c == 5 { 0 } else { 1 };
        cells.push(json!({ "r": last, "c": c, "f": format!("=SUBTOTAL({f},Sales[{col}])"), "st": st }));
    }
    rows.push(vec!["Total".into(), money(tp), money(tr), money(tp + tr), tn.to_string()]);
    let font = json!({ "sz": 12, "name": "Inter", "family": 2, "scheme": "minor" });
    let style = |fmt: &str| json!({ "num_fmt": fmt, "fill": {}, "font": font, "border": {}, "quote_prefix": false });
    let book = json!({
        "format": "xenify-calc", "version": 1, "name": "Sales 2026", "timezone": "UTC",
        "sheets": [{
            "id": 1, "name": "Sales", "info": { "kind": "normal", "working": false }, "frozen": [1, 0], "grid": true,
            "rows": [], "columns": [{ "at": 1, "size": 120.0, "hidden": false }, { "at": 2, "size": 110.0, "hidden": false },
                { "at": 3, "size": 110.0, "hidden": false }, { "at": 4, "size": 110.0, "hidden": false }, { "at": 5, "size": 110.0, "hidden": false }],
            "cells": cells, "merged": [],
        }],
        "styles": [style("general"), style("\"£\"#,##0")],
        "tables": [{
            "name": "Sales", "sheet": 1, "top": 1, "left": 1, "bottom": last, "right": 5, "header": true, "totals": true,
            "columns": [{ "name": "Month" }, { "name": "Projects", "total": "sum" }, { "name": "Retainers", "total": "sum" },
                { "name": "Total", "total": "sum" }, { "name": "New clients", "total": "sum" }],
            "stripes": true,
        }],
        "names": [], "notes": [], "definitions": [],
    });
    let link = json!({ "format": "xenify-calc-link", "version": 1, "items": [{
        "id": "table:Sales", "kind": "table", "name": "Sales", "sheet": "Sales", "rows": rows,
        "header": true, "footer": true, "numbers": [false, true, true, true, true],
    }]});
    let link = link.to_string().into_bytes();
    let file = zip_of(&[
        ("mimetype", b"application/vnd.xenify.calc", false),
        ("calc.json", book.to_string().as_bytes(), true),
        ("link.json", &link, true),
    ]);
    (file, link)
}

/// A picture of the poster (its SVG export at 3 s, drawn by a browser).
const POSTER_PNG: &[u8] = include_bytes!("demo/open-studio-poster.png");

/// The open studio poster (Draw's example poster, reworded) as a .xdraw file.
fn poster() -> Vec<u8> {
    let doc = include_str!("demo/open-studio-poster.json");
    zip_of(&[
        ("document.json", doc.as_bytes(), true),
        ("preview.png", POSTER_PNG, false),
        ("info.json", br#"{"animated":true}"#, true),
    ])
}

// ---------------------------------------------------------------- plans

fn plan_session(title: &str, prefix: &str, start: i64, me: &str, me_res: &str) -> xenify_plan_core::Session {
    let mut p = xenify_plan_core::Session::new();
    p.call("set_clock", &json!({ "today": ymd(start), "now": (start * DAY + 9 * HOUR), "seed": 7 })).unwrap();
    p.call("set_me", &json!({ "name": me, "resource": me_res })).unwrap();
    p.fresh(title);
    p.call("set_item", &json!({ "list": "projects", "id": "p1", "value": { "name": title, "prefix": prefix } }))
        .unwrap();
    p
}

struct Issue<'a> {
    title: &'a str,
    kind: &'a str,
    epic: Option<&'a str>,
    who: &'a str,
    days: f64,
    status: &'a str,
    /// Working days from the plan's start (held there), or None (as soon as it can).
    at: Option<i64>,
    waits: &'a [&'a str],
    priority: &'a str,
    description: &'a str,
}

/// Adds the issues (epics by title); the ids by title.
fn fill_plan(
    p: &mut xenify_plan_core::Session,
    start: i64,
    issues: &[Issue],
) -> std::collections::HashMap<String, String> {
    let mut ids = std::collections::HashMap::<String, String>::new();
    for i in issues {
        let mut args =
            json!({ "title": i.title, "kind": i.kind, "priority": i.priority, "description": i.description });
        if i.kind != "epic" && i.kind != "milestone" {
            args["work"] = json!((i.days * 450.0).round() as u64);
        }
        if !i.who.is_empty() {
            args["who"] = json!(i.who);
        }
        if let Some(e) = i.epic {
            args["parent"] = json!(ids[e]);
        }
        let waits: Vec<&String> = i.waits.iter().map(|w| &ids[*w]).collect();
        if !waits.is_empty() {
            args["waits_for"] = json!(waits);
        }
        let id = p.call("add_task", &args).unwrap()["id"].as_str().unwrap().to_string();
        if let Some(at) = i.at {
            let day = work_day(start, at);
            p.call("hold", &json!({ "id": id, "start": ymd(day) })).unwrap();
        }
        if i.status != "todo" {
            p.call("set_task", &json!({ "id": id, "field": "status", "value": i.status })).unwrap();
            if let Some(at) = i.at {
                let s = work_day(start, at);
                p.call("set_task", &json!({ "id": id, "field": "started", "value": ymd(s) })).unwrap();
                if i.status == "done" {
                    let f = work_day(s, (i.days.ceil() as i64 - 1).max(0));
                    p.call("set_task", &json!({ "id": id, "field": "finished", "value": ymd(f) })).unwrap();
                }
            }
        }
        ids.insert(i.title.to_string(), id);
    }
    ids
}

// ---------------------------------------------------------------- mail

/// Calls Mail's `call` until `ok` holds for its answer.
async fn until(addr: &str, token: &str, call: &str, args: Value, ok: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let (s, v) = post(addr, &format!("/v1/mail/{call}"), Some(token), args.clone()).await;
        if s == 200 && ok(&v) {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {call}: {s} {v}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn plain(from: &str, to: &str, subject: &str, at: i64, id: &str, body: &str) -> String {
    format!(
        "From: {from}\nTo: {to}\nSubject: {subject}\nDate: {}\nMessage-ID: <{id}@demo>\nMIME-Version: 1.0\nContent-Type: text/plain; charset=utf-8\n\n{body}\n",
        mail_date(at)
    )
}

// ---------------------------------------------------------------- the world

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn world() {
    let Some(db) = fresh_db("demo_world").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    if let Ok(apps) = std::env::var("XENIFY_DEMO_APPS") {
        server.host_apps(apps.into());
    }
    let now = now_ms();
    let day0 = today();
    let midnight = day0 * DAY;

    // ---------------------------------------------------------- people
    let sam_email = format!("sam@{DOMAIN}");
    let (s, session) = post(
        &addr,
        "/v1/setup",
        None,
        json!({
            "token": server.setup_token().unwrap(),
            "server_name": "Riverside Studio",
            "username": "sam",
            "display_name": "Sam Okafor",
            "email": sam_email,
            "password": "Riverside sam 2026",
            "device": "demo",
        }),
    )
    .await;
    assert_eq!(s, 200, "{session}");
    let sam = session["token"].as_str().unwrap().to_string();
    let (_, g) = post(&addr, "/v1/admin/groups", Some(&sam), json!({ "name": "Studio team" })).await;
    let team = g["id"].as_i64().unwrap();
    let (_, me) = get(&addr, "/v1/me", Some(&sam)).await;
    let sam_id = me["id"].as_i64().unwrap();
    post(&addr, &format!("/v1/admin/people/{sam_id}"), Some(&sam), json!({ "groups": [team] })).await;

    // (user, name, role in the studio)
    let staff = [
        ("priya", "Priya Shah"),
        ("maya", "Maya Lindqvist"),
        ("tom", "Tom Becker"),
        ("leo", "Leo Martins"),
        ("hannah", "Hannah Cole"),
    ];
    let mut tokens = vec![("sam".to_string(), sam.clone())];
    let mut ids = vec![("sam".to_string(), sam_id)];
    for (user, name) in staff {
        let email = format!("{user}@{DOMAIN}");
        let body = json!({ "username": user, "display_name": name, "email": email, "groups": [team] });
        let (s, p) = post(&addr, "/v1/admin/people", Some(&sam), body).await;
        assert_eq!(s, 200, "{p}");
        let (s, sess) = use_link(&addr, p["code"].as_str().unwrap(), &email, &format!("Riverside {user} 2026")).await;
        assert_eq!(s, 200, "{sess}");
        tokens.push((user.to_string(), sess["token"].as_str().unwrap().to_string()));
        ids.push((user.to_string(), p["id"].as_i64().unwrap()));
    }
    let token = |u: &str| tokens.iter().find(|t| t.0 == u).unwrap().1.clone();
    let res = |u: &str| format!("u{}", ids.iter().find(|t| t.0 == u).unwrap().1);

    // ---------------------------------------------------------- chat
    let mut c_sam = Chat::connect(&addr, &sam).await;
    let mut c_priya = Chat::connect(&addr, &token("priya")).await;
    let mut c_maya = Chat::connect(&addr, &token("maya")).await;
    let mut c_tom = Chat::connect(&addr, &token("tom")).await;
    let mut c_leo = Chat::connect(&addr, &token("leo")).await;
    let mut c_hannah = Chat::connect(&addr, &token("hannah")).await;

    // Rooms and things (the oldest activity).
    let thing = |name: &str, kind: &str, place: &str, seats: i64, by_day: bool| ToServer::SaveThing {
        thing: xenify_chat_proto::Thing {
            id: 0,
            name: name.into(),
            kind: kind.into(),
            place: place.into(),
            seats,
            by_day,
            approver: None,
        },
    };
    for t in [
        thing("Meeting room", "room", "Ground floor", 8, false),
        thing("Quiet room", "room", "First floor", 3, false),
        thing("Pool car", "vehicle", "Car park, space 4", 5, false),
        thing("Projector", "equipment", "Front desk", 0, false),
        thing("Camera kit", "equipment", "Studio cupboard", 0, true),
    ] {
        c_sam.send(t).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    c_sam.send(ToServer::Things).await;
    let things = match c_sam
        .wait(|m| matches!(m, ToApp::Things { list, .. } if list.iter().any(|t| t.name == "Camera kit")))
        .await
    {
        ToApp::Things { list, .. } => list,
        _ => unreachable!(),
    };
    let thing_id = |n: &str| things.iter().find(|t| t.name == n).unwrap().id;

    // New website: Sam, Tom and Priya.
    c_tom
        .send(ToServer::CreateGroup {
            title: "New website".into(),
            members: vec!["sam".into(), "priya".into()],
            purpose: String::new(),
        })
        .await;
    let web = c_tom.made("New website").await;
    c_tom.say(web, "I've put the sitemap in the plan. Twelve pages in the end, down from twenty.").await;
    c_priya.say(web, "Nice. I'll start the wireframes with the home page and the case studies.").await;
    let s1 = c_sam.say(web, "Great work, both. Let's keep the contact form as simple as we can.").await;
    c_tom.react(web, s1, "👍").await;

    // Office: Hannah's announcements.
    c_hannah.send(ToServer::CreateChannel { title: "Office".into(), open: true, purpose: String::new() }).await;
    let office = c_hannah.made("Office").await;
    for c in [&mut c_sam, &mut c_priya, &mut c_maya, &mut c_tom, &mut c_leo] {
        c.send(ToServer::Subscribe { conv: office }).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    c_hannah
        .say(office, "Good morning everyone! The window cleaners are in on Thursday, so please clear the sills by Wednesday evening.")
        .await;
    let ann = c_hannah
        .say(office, "Also: the new coffee machine has arrived 🎉 Instructions are taped to the side. Please don't descale it yourselves.")
        .await;
    c_leo.react(office, ann, "🎉").await;
    c_priya.react(office, ann, "🎉").await;
    c_tom.react(office, ann, "☕").await;

    // Priya and Sam, directly.
    c_sam.send(ToServer::OpenDirect { user: "priya".into() }).await;
    let dm = c_sam.made("priya").await;
    c_sam.say(dm, "Hi Priya, have you got ten minutes before the check-in? I'd like a quick look at the poster.").await;
    c_priya.say(dm, "Sure! I'll bring the laptop over at half one.").await;
    c_priya.say(dm, "The harbour shots came out really well, by the way.").await;

    // Spring campaign: the busiest, last.
    c_maya
        .send(ToServer::CreateGroup {
            title: "Spring campaign".into(),
            members: vec!["sam".into(), "priya".into(), "leo".into()],
            purpose: String::new(),
        })
        .await;
    let spring = c_maya.made("Spring campaign").await;
    c_maya
        .say(
            spring,
            "Harbour Foods loved the second route! They'd like to see it on a poster and in a short social clip.",
        )
        .await;
    let good = c_sam.say(spring, "That's brilliant news. Well done, Maya.").await;
    c_maya.react(spring, good, "❤️").await;
    c_leo
        .say(
            spring,
            "I'll have the headline options ready by Wednesday. Thinking \"Fresh from the harbour\" as the main line.",
        )
        .await;
    c_priya.say(spring, "Love that. I'll set it on the poster so we can see it in place.").await;
    c_maya.say(spring, "Could we go through everything at 2 today? I've booked the meeting room.").await;
    let last = c_leo.say(spring, "Works for me 👍").await;

    // Today's check-in: a meeting with the meeting room, organised by Maya.
    let start = midnight + 14 * HOUR;
    c_maya
        .send(ToServer::CreateGroup {
            title: "Spring campaign check-in".into(),
            members: vec!["sam".into(), "priya".into(), "leo".into()],
            purpose: xenify_chat_proto::MEETING.into(),
        })
        .await;
    let meeting = c_maya.made("Spring campaign check-in").await;
    let room = thing_id("Meeting room");
    let details = json!({
        "title": "Spring campaign check-in", "start": start, "end": start + HOUR, "place": "Meeting room",
        "notes": "Poster drafts, headline options and the plan for the social clip.",
        "guests": [], "uid": format!("demo-checkin-{day0}@{DOMAIN}"), "sequence": 0,
        "things": [{ "id": room, "name": "Meeting room" }],
    });
    c_maya.say(meeting, &format!("~meet1:{details}")).await;
    c_maya
        .send(ToServer::Book { thing: room, start, end: start + HOUR, meeting: Some(meeting), tag: String::new() })
        .await;
    for c in [&mut c_sam, &mut c_priya, &mut c_leo] {
        c.say(meeting, "~answer1:yes").await;
    }
    // Leo has the pool car tomorrow morning for the print shop; Priya the camera kit.
    let tomorrow = midnight + DAY;
    c_leo
        .send(ToServer::Book {
            thing: thing_id("Pool car"),
            start: tomorrow + 9 * HOUR,
            end: tomorrow + 12 * HOUR,
            meeting: None,
            tag: String::new(),
        })
        .await;
    c_priya
        .send(ToServer::Book {
            thing: thing_id("Camera kit"),
            start: tomorrow + 2 * DAY,
            end: tomorrow + 3 * DAY,
            meeting: None,
            tag: String::new(),
        })
        .await;

    // Sam's own calendar.
    c_sam.send(ToServer::CreateGroup { title: "Calendar".into(), members: vec![], purpose: "calendar".into() }).await;
    let cal = c_sam.made("calendar").await;
    for (uid, title, from, to, notes) in [
        (
            "call",
            "Call with Harbour Foods",
            midnight + 10 * HOUR,
            midnight + 10 * HOUR + HOUR / 2,
            "Daniel Price, about the quote.",
        ),
        ("lunch", "Lunch with Tom", midnight + 12 * HOUR + HOUR / 2, midnight + 13 * HOUR + HOUR / 2, ""),
        ("payroll", "Approve payroll", midnight + 16 * HOUR, midnight + 16 * HOUR + HOUR / 2, ""),
        ("print", "Print proofs at Northside", tomorrow + 9 * HOUR + HOUR / 2, tomorrow + 11 * HOUR, ""),
        ("web", "Website wireframe review", tomorrow + 15 * HOUR, tomorrow + 16 * HOUR, ""),
    ] {
        let ev = json!({ "op": "put", "uid": format!("demo-{uid}"), "event": {
            "id": 0, "title": title, "start": from, "end": to, "kind": "event", "notes": notes,
        }});
        c_sam.say(cal, &format!("~cal1:{ev}")).await;
    }
    // The rest of this week and next (so the week view looks lived in,
    // whichever day it is): the stand-up every working day and a few more.
    let monday = (day0 - weekday(day0)) * DAY;
    for w in 0..2 {
        for d in 0..5 {
            let base = monday + (w * 7 + d) * DAY;
            if base == midnight || base == tomorrow {
                continue;
            }
            let ev = json!({ "op": "put", "uid": format!("demo-standup-{w}-{d}"), "event": {
                "id": 0, "title": "Studio stand-up", "start": base + 9 * HOUR + HOUR / 2, "end": base + 9 * HOUR + 3 * HOUR / 4, "kind": "event",
            }});
            c_sam.say(cal, &format!("~cal1:{ev}")).await;
        }
    }
    for (uid, title, d, from, len) in [
        ("kickoff", "Harbour Foods kick-off", 0, 10.0, 2.0),
        ("oak", "Oak & Ash: menu proofs", 1, 14.0, 1.0),
        ("interview", "Interview: junior designer", 2, 11.0, 1.0),
        ("routes", "Concept routes review", 3, 15.0, 1.5),
        ("drinks", "Studio drinks", 4, 17.0, 2.0),
        ("lumen", "Lumen Architects intro call", 7, 11.0, 0.5),
        ("budget", "Budget planning with Hannah", 8, 14.0, 1.0),
        ("shoot", "Harbour photo shoot", 9, 7.0, 3.0),
        ("copy", "Campaign copy review", 10, 13.5, 1.0),
        ("drinks2", "Studio drinks", 11, 17.0, 2.0),
    ] {
        let base = monday + d * DAY;
        if base == midnight || base == tomorrow {
            continue;
        }
        let start = base + (from * HOUR as f64) as i64;
        let ev = json!({ "op": "put", "uid": format!("demo-{uid}"), "event": {
            "id": 0, "title": title, "start": start, "end": start + (len * HOUR as f64) as i64, "kind": "event",
        }});
        c_sam.say(cal, &format!("~cal1:{ev}")).await;
    }
    // Everyone else's busy times, for Find a time.
    for (c, busy) in [
        (&mut c_priya, vec![(9, 10), (13, 14), (15, 16)]),
        (&mut c_maya, vec![(10, 11), (14, 15)]),
        (&mut c_leo, vec![(9, 12), (14, 15)]),
        (&mut c_tom, vec![(11, 12), (12, 13)]),
    ] {
        c.send(ToServer::CreateChannel { title: "Busy".into(), open: true, purpose: "busy".into() }).await;
        let ch = c.made("busy").await;
        let mut times = Vec::new();
        let mut work = Vec::new();
        for d in 0..14 {
            let base = midnight + d * DAY;
            work.push((base + 9 * HOUR, base + 17 * HOUR + HOUR / 2));
            times.extend(busy.iter().map(|(f, t)| (base + f * HOUR, base + t * HOUR)));
        }
        let b = json!({ "from": midnight, "to": midnight + 60 * DAY, "busy": times, "work": work, "away": [] });
        c.say(ch, &format!("~busy1:{b}")).await;
    }
    // Sam has read everything but Leo's last line.
    for conv in [web, office, dm, meeting] {
        c_sam.send(ToServer::Read { conv, seq: 1000 }).await;
    }
    c_sam.send(ToServer::Read { conv: spring, seq: last - 1 }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ---------------------------------------------------------- email
    let (iport, imap) = imap_server::start(&sam_email, "master-pw");
    let (sport, _sent) = smtp_server::start(&sam_email, "master-pw");
    {
        let to = format!("Sam Okafor <{sam_email}>");
        let mut m = imap.lock().unwrap();
        m.add(
            "INBOX",
            &plain(
                "Hannah Cole <hannah@riverside.example>",
                &to,
                "October expenses",
                now - 2 * DAY - 3 * HOUR,
                "expenses",
                "Hi Sam,\n\nOctober's expenses are in the shared folder. Two receipts are still missing from the photo shoot; I've asked Priya for them.\n\nHannah",
            ),
            &["\\Seen"],
        );
        m.add(
            "INBOX",
            &plain(
                "Olivia Grant <olivia@oakandash.example>",
                &to,
                "Thank you!",
                now - DAY - 6 * HOUR,
                "thanks",
                "Dear Sam,\n\nJust a quick note to say thank you to the whole team. The new menus look wonderful and our regulars keep asking who designed them.\n\nWe'll be in touch about the summer refresh.\n\nWarm regards,\nOlivia Grant\nOak & Ash Kitchen",
            ),
            &["\\Seen"],
        );
        m.add(
            "INBOX",
            &format!(
                "From: Design Weekly <hello@designweekly.example>\nTo: {to}\nSubject: Five colour trends for spring\nDate: {}\nMessage-ID: <dw-212@demo>\nMIME-Version: 1.0\nContent-Type: text/html; charset=utf-8\n\n\
                 <html><body style=\"font-family:Georgia,serif\"><h2 style=\"color:#0f766e\">Five colour trends for spring</h2>\
                 <p>Soft greens are back, warm terracotta is everywhere and <b>sunny yellow</b> is the accent of the season.</p>\
                 <ol><li>Sage and moss</li><li>Terracotta</li><li>Butter yellow</li><li>Sea glass</li><li>Deep ink blue</li></ol>\
                 <p><a href=\"https://designweekly.example/spring\">Read the full article</a></p>\
                 <p style=\"color:#78716c;font-size:12px\">You get this because you signed up at designweekly.example.</p></body></html>\n",
                mail_date(now - 20 * HOUR)
            ),
            &[],
        );
        m.add(
            "INBOX",
            &plain(
                "Northside Printworks <accounts@northsideprint.example>",
                &to,
                "Invoice INV-20417",
                now - 5 * HOUR,
                "inv-20417",
                "Hello,\n\nPlease find our invoice INV-20417 for £1,240.00 (500 A2 posters and 2,000 flyers for Oak & Ash Kitchen), due within 30 days.\n\nBank details are on the invoice. Thank you for your business.\n\nNorthside Printworks Accounts",
            ),
            &[],
        );
        // An invitation (tomorrow at 15:00 UTC), as email calendars send them.
        let (y, mo, d) = civil(day0 + 2);
        let ics = format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Demo//EN\nMETHOD:REQUEST\nBEGIN:VEVENT\nUID:lumen-site-review@demo\nDTSTAMP:20261018T090000Z\n\
             DTSTART:{y:04}{mo:02}{d:02}T150000Z\nDTEND:{y:04}{mo:02}{d:02}T160000Z\nSUMMARY:Lumen Architects: brand workshop\nLOCATION:Lumen Architects, 14 Mill Lane\n\
             ORGANIZER;CN=Grace Mensah:mailto:grace@lumenarchitects.example\nATTENDEE;CN=Sam Okafor;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{sam_email}\nSEQUENCE:0\nEND:VEVENT\nEND:VCALENDAR\n"
        );
        m.add(
            "INBOX",
            &format!(
                "From: Grace Mensah <grace@lumenarchitects.example>\nTo: {to}\nSubject: Invitation: Brand workshop\nDate: {}\nMessage-ID: <lumen-invite@demo>\nMIME-Version: 1.0\n\
                 Content-Type: multipart/alternative; boundary=b1\n\n--b1\nContent-Type: text/plain; charset=utf-8\n\nHi Sam, here's the invitation for our brand workshop. Looking forward to it!\n\nGrace\n\
                 --b1\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\n\n{ics}--b1--\n",
                mail_date(now - 3 * HOUR)
            ),
            &[],
        );
        m.add(
            "INBOX",
            &plain(
                "Daniel Price <daniel.price@harbourfoods.example>",
                &to,
                "Quote for the spring campaign",
                now - 40 * 60_000,
                "harbour-quote",
                "Hi Sam,\n\nThanks again for the concepts, the team is really excited about \"Fresh from the harbour\".\n\nCould you send us a quote for the poster, the social clip and three weeks of media booking? If possible we'd like to sign it off before the end of the month.\n\nBest wishes,\nDaniel Price\nMarketing Manager, Harbour Foods",
            ),
            &[],
        );
    }
    let local = |p: u16| json!({ "host": "127.0.0.1", "port": p, "security": "none" });
    let set = json!({
        "org": "imap", "domains": DOMAIN, "imap": local(iport), "smtp": local(sport),
        "master_user": "office", "master_password": "master-pw", "login_as": "{email}",
    });
    let (s, v) = post(&addr, "/v1/admin/usermail", Some(&sam), set).await;
    assert_eq!(s, 204, "{v}");

    // ---------------------------------------------------------- files
    // Templates (the server's own space).
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&sam)).await;
    let space_of =
        |kind: &str| spaces.as_array().unwrap().iter().find(|s| s["kind"] == kind).map(|s| s["id"].as_i64().unwrap());
    if let Some(templates) = space_of("templates") {
        document(&addr, &sam, templates, None, "Meeting notes", None, MEETING_NOTES).await;
        document(&addr, &sam, templates, None, "Project brief", None, PROJECT_BRIEF).await;
        document(&addr, &sam, templates, None, "Client letter", None, CLIENT_LETTER).await;
    }
    let shared_space = |name: &str| {
        let (addr, sam) = (addr.clone(), sam.clone());
        let name = name.to_string();
        async move {
            let body = json!({ "name": json!({ "name": name }).to_string(),
                "grants": [{ "who": { "group": team }, "role": "editor" }] });
            let (s, sp) = post(&addr, "/v1/spaces", Some(&sam), body).await;
            assert_eq!(s, 200, "{sp}");
            sp["id"].as_i64().unwrap()
        }
    };
    // The Studio handbook: a wiki with its overview, pages and a few files.
    let handbook = shared_space("Studio handbook").await;
    let welcome = document(&addr, &sam, handbook, None, "Welcome to Riverside", Some(-1), WELCOME).await;
    post(
        &addr,
        &format!("/v1/spaces/{handbook}"),
        Some(&sam),
        json!({ "name": json!({ "name": "Studio handbook", "home": welcome }).to_string() }),
    )
    .await;
    let hannah = token("hannah");
    document(&addr, &hannah, handbook, None, "Holiday policy", Some(1), HOLIDAYS).await;
    let brand = document(&addr, &token("priya"), handbook, None, "Brand guide", Some(2), BRAND).await;
    document(&addr, &token("priya"), handbook, Some(brand), "Logo and colours", Some(0), LOGO).await;
    document(&addr, &token("maya"), handbook, None, "Client onboarding checklist", Some(3), ONBOARDING).await;
    document(&addr, &hannah, handbook, None, "Office and IT", Some(4), OFFICE).await;
    let (book, link) = sales_book();
    let sales =
        node(&addr, &hannah, handbook, None, "file", json!({ "name": "Sales 2026", "app": "calc", "ext": "xcalc" }))
            .await;
    save(&addr, &hannah, sales, &book).await;
    raw(&addr, "POST", &format!("/v1/nodes/{sales}/preview"), &hannah, &link).await;
    let drawing = node(
        &addr,
        &token("priya"),
        handbook,
        None,
        "file",
        json!({ "name": "Open studio poster", "app": "draw", "ext": "xdraw" }),
    )
    .await;
    save(&addr, &token("priya"), drawing, &poster()).await;
    raw(&addr, "POST", &format!("/v1/nodes/{drawing}/preview"), &token("priya"), POSTER_PNG).await;

    // Projects: the plans, and the campaign's documents.
    let projects = shared_space("Client projects").await;
    let harbour = node(&addr, &sam, projects, None, "folder", json!({ "name": "Harbour Foods" })).await;
    document(&addr, &token("maya"), projects, Some(harbour), "Spring campaign brief", None, CAMPAIGN_BRIEF).await;
    document(&addr, &token("leo"), projects, Some(harbour), "Headline options", None, HEADLINES).await;
    node(&addr, &sam, projects, None, "folder", json!({ "name": "Oak & Ash Kitchen" })).await;
    node(&addr, &sam, projects, None, "folder", json!({ "name": "Lumen Architects" })).await;

    // Sam's own files.
    if let Some(mine) = space_of("personal") {
        document(&addr, &sam, mine, None, "Quote for Harbour Foods", None, QUOTE).await;
        document(&addr, &sam, mine, None, "One-to-one notes", None, ONE_TO_ONE).await;
    }

    // ---------------------------------------------------------- plans
    let plan_start = work_day(day0, -9);
    let mut spring_plan = plan_session("Spring campaign", "SPR", plan_start, "Maya Lindqvist", &res("maya"));
    let e = |t| Some(t);
    let (maya, priya, leo, tom, sam_r) = (res("maya"), res("priya"), res("leo"), res("tom"), res("sam"));
    fill_plan(
        &mut spring_plan,
        plan_start,
        &[
            Issue {
                title: "Campaign concept",
                kind: "epic",
                epic: None,
                who: "",
                days: 0.0,
                status: "doing",
                at: None,
                waits: &[],
                priority: "high",
                description: "",
            },
            Issue {
                title: "Kick-off workshop with Harbour Foods",
                kind: "task",
                epic: e("Campaign concept"),
                who: &maya,
                days: 1.0,
                status: "done",
                at: Some(0),
                waits: &[],
                priority: "high",
                description: "Half a day at their offices: goals, audience, budget and dates.",
            },
            Issue {
                title: "Audience research summary",
                kind: "task",
                epic: e("Campaign concept"),
                who: &leo,
                days: 2.0,
                status: "done",
                at: Some(1),
                waits: &[],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Three concept routes",
                kind: "story",
                epic: e("Campaign concept"),
                who: &priya,
                days: 3.0,
                status: "done",
                at: Some(3),
                waits: &[],
                priority: "high",
                description: "Mood boards and one rough poster for each route.",
            },
            Issue {
                title: "Client feedback on the routes",
                kind: "task",
                epic: e("Campaign concept"),
                who: &maya,
                days: 1.0,
                status: "review",
                at: Some(6),
                waits: &[],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Production",
                kind: "epic",
                epic: None,
                who: "",
                days: 0.0,
                status: "doing",
                at: None,
                waits: &[],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Photo shoot at the harbour",
                kind: "task",
                epic: e("Production"),
                who: &priya,
                days: 2.0,
                status: "doing",
                at: Some(8),
                waits: &[],
                priority: "high",
                description: "Early morning light; the fish market has agreed to let us in from 6am.",
            },
            Issue {
                title: "Write the campaign copy",
                kind: "task",
                epic: e("Production"),
                who: &leo,
                days: 3.0,
                status: "doing",
                at: Some(8),
                waits: &[],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Poster and social artwork",
                kind: "story",
                epic: e("Production"),
                who: &priya,
                days: 4.0,
                status: "todo",
                at: None,
                waits: &["Photo shoot at the harbour", "Write the campaign copy"],
                priority: "high",
                description: "",
            },
            Issue {
                title: "Fix colour shift in the print proofs",
                kind: "bug",
                epic: e("Production"),
                who: &priya,
                days: 1.0,
                status: "todo",
                at: None,
                waits: &["Poster and social artwork"],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Launch",
                kind: "epic",
                epic: None,
                who: "",
                days: 0.0,
                status: "todo",
                at: None,
                waits: &[],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Media plan and booking",
                kind: "task",
                epic: e("Launch"),
                who: &maya,
                days: 2.0,
                status: "todo",
                at: None,
                waits: &["Client feedback on the routes"],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Campaign landing page",
                kind: "task",
                epic: e("Launch"),
                who: &tom,
                days: 3.0,
                status: "todo",
                at: None,
                waits: &["Poster and social artwork"],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Campaign goes live",
                kind: "milestone",
                epic: e("Launch"),
                who: &maya,
                days: 0.0,
                status: "todo",
                at: None,
                waits: &["Campaign landing page", "Media plan and booking"],
                priority: "high",
                description: "",
            },
        ],
    );
    let mut web_plan = plan_session("New website", "WEB", plan_start, "Tom Becker", &tom);
    let web_ids = fill_plan(
        &mut web_plan,
        plan_start,
        &[
            Issue {
                title: "Content audit and sitemap",
                kind: "task",
                epic: None,
                who: &tom,
                days: 2.0,
                status: "done",
                at: Some(2),
                waits: &[],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "Wireframes",
                kind: "story",
                epic: None,
                who: &priya,
                days: 3.0,
                status: "doing",
                at: Some(8),
                waits: &[],
                priority: "high",
                description: "",
            },
            Issue {
                title: "Case study write-ups",
                kind: "task",
                epic: None,
                who: &leo,
                days: 3.0,
                status: "todo",
                at: Some(10),
                waits: &[],
                priority: "medium",
                description: "Oak & Ash, Lumen Architects and the Harbour Foods campaign.",
            },
            Issue {
                title: "Build the page templates",
                kind: "task",
                epic: None,
                who: &tom,
                days: 5.0,
                status: "todo",
                at: None,
                waits: &["Wireframes"],
                priority: "high",
                description: "",
            },
            Issue {
                title: "Contact form",
                kind: "task",
                epic: None,
                who: &tom,
                days: 1.0,
                status: "todo",
                at: None,
                waits: &["Build the page templates"],
                priority: "low",
                description: "",
            },
            Issue {
                title: "Sign-off with Sam",
                kind: "task",
                epic: None,
                who: &sam_r,
                days: 1.0,
                status: "todo",
                at: None,
                waits: &["Build the page templates", "Case study write-ups"],
                priority: "medium",
                description: "",
            },
            Issue {
                title: "New site live",
                kind: "milestone",
                epic: None,
                who: &tom,
                days: 0.0,
                status: "todo",
                at: None,
                waits: &["Sign-off with Sam", "Contact form"],
                priority: "high",
                description: "",
            },
        ],
    );
    // Priya does the wireframes alongside the shoot (not split around her campaign work).
    web_plan.call("set_task", &json!({ "id": web_ids["Wireframes"], "field": "overbook", "value": true })).unwrap();
    for (title, plan, by) in [("Spring campaign", &spring_plan, "maya"), ("New website", &web_plan, "tom")] {
        let t = token(by);
        let n = node(&addr, &t, projects, None, "file", json!({ "name": title, "app": "plan", "ext": "xplan" })).await;
        save(&addr, &t, n, &plan.save_file().unwrap()).await;
    }

    // ---------------------------------------------------------- ready
    until(&addr, &sam, "mail_list", json!({ "account": "", "folder": "", "limit": 50 }), |v| {
        v.as_array().is_some_and(|l| l.len() >= 6)
    })
    .await;
    let mut people = serde_json::Map::new();
    for (user, t) in &tokens {
        let (_, account) = get(&addr, "/v1/me", Some(t)).await;
        people.insert(user.clone(), json!({ "token": t, "account": account }));
    }
    println!("DEMO {}", json!({ "addr": format!("http://{addr}"), "people": people }));
    // Keep the chats' connections open (people online) and the server up.
    let _keep = (c_sam, c_priya, c_maya, c_tom, c_leo, c_hannah, imap);
    std::future::pending::<()>().await;
}

// ---------------------------------------------------------------- words

const WELCOME: &str = r#"
Welcome to Riverside Studio! We're a small design studio by the river, making brands, campaigns and websites for independent businesses.

This handbook has everything you need for your first weeks. If something's missing, add it — every page can be changed by everyone in the team.

## Your first week

- **Monday:** coffee with Sam, your laptop and accounts, and a tour of the building.
- **Tuesday:** sit in on a client call with Maya.
- **Wednesday to Friday:** join a live project and get to know the team.

## Who's who

| Name | Role | Ask them about |
| --- | --- | --- |
| Sam Okafor | Studio director | Clients, quotes and anything else |
| Maya Lindqvist | Account manager | Client projects and schedules |
| Priya Shah | Senior designer | Brand, artwork and print |
| Tom Becker | Developer | Websites and IT |
| Leo Martins | Copywriter | Words, names and headlines |
| Hannah Cole | Office manager | Holidays, expenses and the building |

## How we work

We plan our projects in **Plan**, talk in **Chat** and keep our documents here in **Files**. Client work lives in the *Client projects* space; everything about the studio itself lives in this handbook.
"#;

const HOLIDAYS: &str = r#"
Everyone at Riverside gets **27 days of holiday** a year plus bank holidays, and the studio is closed between Christmas and New Year.

## Booking time off

1. Check the team calendar in Chat for clashes with launches and shoots.
2. Ask Hannah at least **two weeks** ahead for anything longer than two days.
3. Once it's agreed, set your out of office in Chat so clients know who to contact.

## Carrying days over

You can carry up to **five days** into the next year. Please use them by the end of March.

## Sick days

Let Hannah and your project lead know by 9:30 if you're unwell. You don't need a note for the first week.
"#;

const BRAND: &str = r#"
Our own brand should feel like our work: calm, clear and a little bit playful.

## Voice

- Write like we talk: short sentences, everyday words.
- Be warm, never cute. One exclamation mark per page is plenty.
- Say what we did for a client, not how clever we were.

## Typefaces

Headlines are set in **Fraunces**, everything else in **Inter**. Both are free, so clients can use them too.

## Photography

Real people, real places, natural light. No stock photos of handshakes.
"#;

const LOGO: &str = r#"
The logo is the word *riverside* with a wave under the "i". Always leave space around it at least as wide as the wave.

| Colour | Use | Hex |
| --- | --- | --- |
| River teal | Logo, headings, links | #0F766E |
| Sand | Backgrounds | #F5EBDD |
| Ink | Body text | #1C1917 |
| Coral | One highlight per page | #F97362 |

Don't stretch, rotate or outline the logo, and don't put it on busy photos.
"#;

const ONBOARDING: &str = r#"
Everything we do when a new client signs, in order. Tick items off in the client's copy of this list.

## Before the kick-off

- [ ] Signed quote and purchase order received
- [ ] Client folder made in *Client projects*
- [ ] Plan set up with the agreed dates
- [ ] Kick-off workshop booked (meeting room or at their office)

## At the kick-off

- [ ] Goals, audience and budget agreed
- [ ] Who signs off, and how quickly
- [ ] Brand files and photos handed over

## After the kick-off

- [ ] Notes shared with the client within a day
- [ ] First invoice sent (30% of the quote)
- [ ] Weekly check-in added to everyone's calendar
"#;

const OFFICE: &str = r#"
## Getting in

The front door code changes every month; Hannah posts it in the *Office* chat. The last one out locks the side gate.

## Wi-Fi and printing

Use the *Riverside* network (the password is on the kitchen noticeboard). The big printer is on the first floor and prints up to A3.

## Booking rooms and things

Book the meeting room, the quiet room, the pool car and the camera kit in **Chat › Bookings**. Please return the pool car with at least a quarter of a tank.

## Kitchen

Tea and coffee are on us. Fruit arrives on Mondays. Please empty the dishwasher if it's clean.
"#;

const CAMPAIGN_BRIEF: &str = r#"
**Client:** Harbour Foods · **Contact:** Daniel Price · **Launch:** early spring

## What they want

Harbour Foods sells fresh fish and seafood to restaurants and, since last year, to home cooks online. They want more people in the city to try ordering from them at home.

## Audience

Home cooks aged 30 to 55 who already shop at farmers' markets and care where their food comes from.

## What we'll make

1. A poster for bus stops and the fish market
2. A 20-second social clip
3. A landing page with the spring offer

## Budget and dates

£24,000 including media. First drafts in three weeks; launch on the first Monday of spring.
"#;

const HEADLINES: &str = r#"
Shortlist for the poster, favourite first.

1. **Fresh from the harbour.** Simple, true, and it works with the photo.
2. *Tonight's catch, on your doorstep.*
3. *The sea, delivered.*
4. *Caught this morning. Dinner tonight.*

Notes from the check-in: the client liked 1 and 4. We could use 4 as the line under the headline.
"#;

const QUOTE: &str = r#"
**For:** Daniel Price, Harbour Foods
**From:** Sam Okafor, Riverside Studio

## Spring campaign

| Item | Days | Price |
| --- | --- | --- |
| Poster design and artwork | 4 | £3,200 |
| Social clip (20 seconds) | 5 | £4,500 |
| Landing page | 3 | £2,700 |
| Media planning and booking | 2 | £1,600 |
| **Total** | **14** | **£12,000** |

Printing and media costs are charged as they come, without a mark-up. The quote is valid for 30 days.
"#;

const ONE_TO_ONE: &str = r#"
## Priya

- Loved the harbour shoot; wants to do more photography.
- Would like to lead the website redesign.
- Training: a motion design course in the new year.

## Leo

- Busy month, but happy. Wants clearer briefs earlier on.
- Ask Maya to share client feedback with him directly.
"#;

const MEETING_NOTES: &str = r#"
**Date:** · **Who:** · **Where:**

## What we talked about

## What we decided

## Who does what next

| What | Who | By when |
| --- | --- | --- |
|  |  |  |
"#;

const PROJECT_BRIEF: &str = r#"
**Client:** · **Contact:** · **Launch:**

## What they want

## Audience

## What we'll make

## Budget and dates
"#;

const CLIENT_LETTER: &str = r#"
Riverside Studio · 3 Wharf Street · Riverside

Dear ,

Thank you for

With best wishes,

Sam Okafor
Studio director
"#;
