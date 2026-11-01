//! Chat at office scale, against a running server (plain HTTP, e.g. through
//! an SSH tunnel):
//!
//!   cargo run --example chat_load -- seed  ADDR STATE SETUP_TOKEN [PEOPLE]
//!   cargo run --example chat_load -- check ADDR STATE
//!
//! `seed` sets the server up (first admin "qa-admin"), adds PEOPLE people
//! (default 300) in teams of 25, a channel everyone follows, a group per
//! team and some direct chats, and saves their sign-ins in STATE (JSON).
//! `check` connects everyone at once, then times the conversation list,
//! sending to the big channel until everyone has it, history and search,
//! and checks search answers: every hit is from a chat the person is in,
//! and a word said in one team's group is found by that team only.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use xenify_chat_proto::{ToApp, ToServer};

const TEAM: usize = 25;
const WORDS: &[&str] = &[
    "budget", "launch", "meeting", "invoice", "customer", "design", "review", "deadline", "server", "report",
    "holiday", "printer", "contract", "release", "office", "lunch", "travel", "training", "update", "draft",
];

fn http(addr: &str, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let url = format!("http://{addr}{path}");
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut r = match (method, body) {
        ("POST", Some(b)) => {
            let mut req = agent.post(&url);
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            req.header("Content-Type", "application/json").send(b.to_string()).unwrap()
        }
        _ => {
            let mut req = agent.get(&url);
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            req.call().unwrap()
        }
    };
    let status = r.status().as_u16();
    (status, serde_json::from_str(&r.body_mut().read_to_string().unwrap_or_default()).unwrap_or(Value::Null))
}

struct Chat {
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Chat {
    async fn connect(addr: &str, token: &str) -> Chat {
        let mut req = format!("ws://{addr}/v1/ws").into_client_request().unwrap();
        req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        Chat { ws }
    }
    async fn send(&mut self, m: ToServer) {
        self.ws.send(Ws::Text(serde_json::to_string(&m).unwrap().into())).await.unwrap();
    }
    async fn wait(&mut self, secs: u64, ok: impl Fn(&ToApp) -> bool) -> Option<ToApp> {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            let f = tokio::time::timeout(left, self.ws.next()).await.ok()??.ok()?;
            if let Ws::Text(t) = f {
                match serde_json::from_str::<ToApp>(&t) {
                    Ok(ToApp::Error { code, message }) => eprintln!("error from the server: {code}: {message}"),
                    Ok(m) if ok(&m) => return Some(m),
                    Ok(_) => {}
                    Err(e) => eprintln!("can't read {t:.120}: {e}"),
                }
            }
        }
    }
}

fn name(i: usize) -> String {
    format!("qa{i:03}")
}

fn team_of(i: usize) -> usize {
    i / TEAM
}

fn stats(label: &str, mut ms: Vec<f64>) {
    if ms.is_empty() {
        return println!("{label}: nothing measured");
    }
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| ms[((ms.len() - 1) as f64 * p) as usize];
    println!("{label}: n={} median {:.0} ms, p95 {:.0} ms, max {:.0} ms", ms.len(), at(0.5), at(0.95), at(1.0));
}

async fn seed(addr: &str, state: &str, setup: &str, people: usize) {
    let (status, s) = http(
        addr,
        "POST",
        "/v1/setup",
        None,
        Some(json!({"token": setup, "server_name": "QA office", "username": "qa-admin", "display_name": "QA Admin",
            "email": "qa-admin@example.org", "password": "qa admin password 1", "device": "qa"})),
    );
    assert_eq!(status, 200, "setup: {s}");
    let admin = s["token"].as_str().unwrap().to_string();
    let mut tokens = Vec::new();
    let t0 = Instant::now();
    for i in 0..people {
        let (status, made) = http(
            addr,
            "POST",
            "/v1/admin/people",
            Some(&admin),
            Some(
                json!({"username": name(i), "display_name": format!("Person {i} Team{}", team_of(i)), "admin": false}),
            ),
        );
        assert_eq!(status, 200, "add {i}: {made}");
        let link = made["link"]["link"].as_str().unwrap();
        let part = link.rsplit_once('#').unwrap().1;
        let (status, session) = http(
            addr,
            "POST",
            "/v1/link/use",
            None,
            Some(json!({"link": part, "password": format!("{} password 1", name(i)), "username": "", "device": "qa"})),
        );
        assert_eq!(status, 200, "link {i}: {session}");
        tokens.push(session["token"].as_str().unwrap().to_string());
        if i % 50 == 49 {
            println!("{} people in {:.0} s", i + 1, t0.elapsed().as_secs_f64());
        }
    }
    // A channel for everyone, a group per team, direct chats in each team.
    let mut a = Chat::connect(addr, &admin).await;
    a.wait(10, |m| matches!(m, ToApp::Welcome { .. })).await.unwrap();
    a.send(ToServer::CreateChannel { title: "Everyone".into(), open: true, purpose: String::new() }).await;
    let Some(ToApp::Conversation { conv: everyone }) =
        a.wait(10, |m| matches!(m, ToApp::Conversation { conv } if conv.title == "Everyone")).await
    else {
        panic!("no channel")
    };
    for chunk in (0..people).collect::<Vec<_>>().chunks(100) {
        a.send(ToServer::AddPeople { conv: everyone.id, users: chunk.iter().map(|i| name(*i)).collect() }).await;
        a.wait(30, |m| matches!(m, ToApp::Conversation { conv } if conv.id == everyone.id)).await;
    }
    let mut groups = Vec::new();
    for t in 0..people.div_ceil(TEAM) {
        let lead = t * TEAM;
        let mut c = Chat::connect(addr, &tokens[lead]).await;
        c.wait(10, |m| matches!(m, ToApp::Welcome { .. })).await.unwrap();
        let members = (lead..(lead + TEAM).min(people)).map(name).collect();
        let title = format!("Team {t}");
        c.send(ToServer::CreateGroup { title: title.clone(), members, purpose: String::new() }).await;
        let Some(ToApp::Conversation { conv }) =
            c.wait(10, |m| matches!(m, ToApp::Conversation { conv } if conv.title == title)).await
        else {
            panic!("no group")
        };
        // A word only this team says (for checking search), and some talk.
        c.send(ToServer::Send {
            conv: conv.id,
            client_id: "secret".into(),
            text: format!("Codename zq{t}x for the project"),
        })
        .await;
        for k in 0..40 {
            let text = format!(
                "{} {} {} number {k}",
                WORDS[k % WORDS.len()],
                WORDS[(k * 7) % WORDS.len()],
                WORDS[(k * 3 + t) % WORDS.len()]
            );
            c.send(ToServer::Send { conv: conv.id, client_id: format!("s{k}"), text }).await;
        }
        for other in lead + 1..(lead + 6).min(people) {
            c.send(ToServer::OpenDirect { user: name(other) }).await;
        }
        c.wait(5, |m| matches!(m, ToApp::Message { msg } if msg.client_id == "s39")).await;
        groups.push(conv.id);
    }
    std::fs::write(
        state,
        serde_json::to_string_pretty(
            &json!({"admin": admin, "tokens": tokens, "everyone": everyone.id, "groups": groups}),
        )
        .unwrap(),
    )
    .unwrap();
    println!("seeded {people} people, {} teams, in {:.0} s", groups.len(), t0.elapsed().as_secs_f64());
}

async fn check(addr: &str, state: &str) {
    let s: Value = serde_json::from_str(&std::fs::read_to_string(state).unwrap()).unwrap();
    let tokens: Vec<String> = serde_json::from_value(s["tokens"].clone()).unwrap();
    let everyone = s["everyone"].as_i64().unwrap();
    let admin = s["admin"].as_str().unwrap().to_string();
    let people = tokens.len();

    // Everyone connects at once and lists their chats.
    let t0 = Instant::now();
    let mut tasks = Vec::new();
    for t in tokens.clone() {
        let addr = addr.to_string();
        tasks.push(tokio::spawn(async move {
            let start = Instant::now();
            let mut c = Chat::connect(&addr, &t).await;
            c.wait(30, |m| matches!(m, ToApp::Welcome { .. })).await.expect("welcome");
            let welcome = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            c.send(ToServer::List).await;
            c.wait(60, |m| matches!(m, ToApp::Conversations { .. })).await.expect("list");
            (c, welcome, start.elapsed().as_secs_f64() * 1000.0)
        }));
    }
    let mut chats = Vec::new();
    let (mut welcome, mut list) = (Vec::new(), Vec::new());
    for t in tasks {
        let (c, w, l) = t.await.unwrap();
        chats.push(c);
        welcome.push(w);
        list.push(l);
    }
    println!("{people} connected in {:.1} s", t0.elapsed().as_secs_f64());
    stats("connect until welcome", welcome);
    stats("conversation list", list);

    // A post to the big channel, until everyone has it.
    let mut a = Chat::connect(addr, &admin).await;
    a.wait(10, |m| matches!(m, ToApp::Welcome { .. })).await.unwrap();
    for round in 0..3 {
        let id = format!("fan{round}-{}", std::process::id());
        let start = Instant::now();
        a.send(ToServer::Send { conv: everyone, client_id: id.clone(), text: format!("All hands at {round} pm") })
            .await;
        let mut waits = Vec::new();
        for mut c in chats.drain(..) {
            let id = id.clone();
            waits.push(tokio::spawn(async move {
                let got = c.wait(30, |m| matches!(m, ToApp::Message { msg } if msg.client_id == id)).await;
                (c, got.is_some(), start.elapsed().as_secs_f64() * 1000.0)
            }));
        }
        let mut missed = 0;
        let mut ms = Vec::new();
        for w in waits {
            let (c, got, t) = w.await.unwrap();
            chats.push(c);
            if got { ms.push(t) } else { missed += 1 }
        }
        if missed > 0 {
            println!("PROBLEM: {missed} of {people} never got the channel post");
        }
        stats(&format!("channel post reaching everyone (round {round})"), ms);
    }

    // History and search, by a few people, measured one at a time.
    let (mut hist, mut search, mut leaks, mut wrong) = (Vec::new(), Vec::new(), 0, 0);
    for (i, c) in chats.iter_mut().enumerate().step_by((people / 30).max(1)) {
        let start = Instant::now();
        c.send(ToServer::History { conv: everyone, before: None, limit: 50, tag: "h".into() }).await;
        c.wait(30, |m| matches!(m, ToApp::History { .. })).await.expect("history");
        hist.push(start.elapsed().as_secs_f64() * 1000.0);
        for q in [
            "budget",
            "budg",
            "zq",
            &format!("zq{}x", team_of(i)),
            "nothingmatchesthis",
            "meeting deadline",
            "number",
            "nu",
            "codename",
        ] {
            let start = Instant::now();
            c.send(ToServer::Search {
                q: q.into(),
                conv: None,
                sort: Default::default(),
                after: String::new(),
                tag: q.into(),
            })
            .await;
            let Some(ToApp::Hits { hits, .. }) = c.wait(60, |m| matches!(m, ToApp::Hits { tag, .. } if tag == q)).await
            else {
                println!("PROBLEM: no answer searching {q:?}");
                continue;
            };
            search.push(start.elapsed().as_secs_f64() * 1000.0);
            let mine = s["groups"].as_array().unwrap()[team_of(i)].as_i64().unwrap();
            for h in &hits {
                let ok = h.msg.conv == mine || h.msg.conv == everyone || h.msg.sender.username == name(i);
                if !ok && !h.msg.text.is_empty() {
                    // Direct chats are fine too; anything in another team's group isn't.
                    if s["groups"].as_array().unwrap().iter().any(|g| g.as_i64() == Some(h.msg.conv)) {
                        leaks += 1;
                    }
                }
            }
            if q.starts_with("zq") && q.len() > 2 && hits.len() != 1 {
                wrong += 1;
                println!("PROBLEM: {q:?} by {} gave {} hits (expected 1)", name(i), hits.len());
            }
            if q == "zq" && hits.len() != 1 {
                println!("note: prefix \"zq\" gave {} hits for {}", hits.len(), name(i));
            }
        }
    }
    stats("history (50 newest)", hist);
    stats("search", search);
    println!("search leaks from others' groups: {leaks}; wrong counts: {wrong}");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("seed") => {
            let people = args.get(5).and_then(|p| p.parse().ok()).unwrap_or(300);
            seed(&args[2], &args[3], &args[4], people).await
        }
        Some("check") => check(&args[2], &args[3]).await,
        _ => eprintln!("chat_load seed ADDR STATE SETUP_TOKEN [PEOPLE] | check ADDR STATE"),
    }
}
