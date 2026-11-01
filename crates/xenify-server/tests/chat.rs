//! Chat on the Xenify server: the same accounts as files.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use xenify_chat_proto::{ToApp, ToServer};

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
    async fn wait(&mut self, ok: impl Fn(&ToApp) -> bool) -> ToApp {
        loop {
            let f = tokio::time::timeout(Duration::from_secs(10), self.ws.next()).await.unwrap().unwrap().unwrap();
            if let Ws::Text(t) = f {
                let m: ToApp = serde_json::from_str(&t).unwrap();
                if ok(&m) {
                    return m;
                }
            }
        }
    }
}

#[tokio::test]
async fn people_on_the_server_chat_with_their_accounts() {
    let Some(db) = fresh_db("chat").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let priya = add_person(&addr, &admin, "priya", false).await;

    let mut a = Chat::connect(&addr, &admin).await;
    let ToApp::Welcome { user, .. } = a.wait(|m| matches!(m, ToApp::Welcome { .. })).await else { unreachable!() };
    assert_eq!(user.username, "alex");
    let mut p = Chat::connect(&addr, &priya).await;
    p.wait(|m| matches!(m, ToApp::Welcome { .. })).await;

    a.send(ToServer::OpenDirect { user: "priya".into() }).await;
    let ToApp::Conversation { conv } = a.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    a.send(ToServer::Send { conv: conv.id, client_id: "a1".into(), text: "Brief is ready".into() }).await;
    let ToApp::Message { msg } = p.wait(|m| matches!(m, ToApp::Message { .. })).await else { unreachable!() };
    assert_eq!((msg.text.as_str(), msg.sender.username.as_str()), ("Brief is ready", "alex"));

    // Someone added later sees what was said before.
    a.send(ToServer::CreateGroup { title: "Launch".into(), members: vec!["priya".into()], purpose: String::new() })
        .await;
    let ToApp::Conversation { conv: g } =
        a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Launch")).await
    else {
        unreachable!()
    };
    a.send(ToServer::Send { conv: g.id, client_id: "g1".into(), text: "Kick-off on Monday".into() }).await;
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.conv == g.id)).await;
    let sam = add_person(&addr, &admin, "sam", false).await;
    a.send(ToServer::AddPeople { conv: g.id, users: vec!["sam".into()] }).await;
    let mut s = Chat::connect(&addr, &sam).await;
    s.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    s.send(ToServer::History { conv: g.id, before: None, limit: 50, tag: String::new() }).await;
    let ToApp::History { messages, .. } = s.wait(|m| matches!(m, ToApp::History { .. })).await else { unreachable!() };
    assert_eq!(messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Kick-off on Monday"]);

    // Leaving: the others see one fewer.
    p.send(ToServer::Leave { conv: g.id }).await;
    let id = g.id;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == id && conv.member_count == 2)).await;

    // Chat has no sign-in of its own: the Xenify server's is the one.
    let (status, _) = post(&addr, "/v1/register", None, json!({ "username": "x", "password": "y" })).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn everyone_finds_everyone_by_name_admins_or_not() {
    let Some(db) = fresh_db("chat_find").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let fred = add_person(&addr, &admin, "fred", false).await;
    add_person(&addr, &admin, "luke", true).await;
    let mut f = Chat::connect(&addr, &fred).await;
    f.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    let mut find = async |q: &str| {
        f.send(ToServer::FindPeople { q: q.into() }).await;
        let want = q.to_string();
        let ToApp::People { users, .. } = f.wait(|m| matches!(m, ToApp::People { q, .. } if *q == want)).await else {
            unreachable!()
        };
        users.into_iter().map(|u| u.username).collect::<Vec<_>>()
    };
    // Admins (alex, luke) are found by someone who isn't one, by user name,
    // by any word of their name, or by both words.
    assert_eq!(find("lu").await, ["luke"]);
    assert_eq!(find("Morgan").await, ["alex"]);
    assert_eq!(find("alex m").await, ["alex"]);
    assert!(find("alex x").await.is_empty());
    // Everyone, without yourself.
    assert_eq!(find("").await, ["alex", "luke"]);
}

/// Raw bytes in and out, as the chat app sends file parts.
fn bytes_call(method: &str, url: &str, token: &str, body: Option<Vec<u8>>) -> (u16, Vec<u8>) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let auth = format!("Bearer {token}");
    let mut r = match (method, body) {
        ("PUT", Some(b)) => agent.put(url).header("Authorization", &auth).send(&b[..]).unwrap(),
        _ => agent.get(url).header("Authorization", &auth).call().unwrap(),
    };
    (r.status().as_u16(), r.body_mut().read_to_vec().unwrap_or_default())
}

#[tokio::test]
async fn files_in_chats_go_in_parts_to_members_only() {
    let Some(db) = fresh_db("chatfiles").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let kim = add_person(&addr, &admin, "kim", false).await;

    // Alex makes a group without Kim and sends a file of 5 MiB (two parts).
    let mut a = Chat::connect(&addr, &admin).await;
    a.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    a.send(ToServer::CreateGroup { title: "Plans".into(), members: vec![], purpose: String::new() }).await;
    let ToApp::Conversation { conv } = a.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    let size = 5 << 20;
    let (status, started) = post(&addr, "/v1/chat/files", Some(&admin), json!({ "conv": conv.id, "size": size })).await;
    assert_eq!(status, 200, "{started}");
    let (id, part) = (started["id"].as_str().unwrap().to_string(), started["part"].as_u64().unwrap() as usize);
    assert_eq!(part, 4 << 20);
    // Kim isn't in the chat: she can't start a file there.
    assert_eq!(post(&addr, "/v1/chat/files", Some(&kim), json!({ "conv": conv.id, "size": 10 })).await.0, 403);
    let base = format!("http://{addr}/v1/chat/files/{id}");
    let (adm, k) = (admin.clone(), kim.clone());
    let b2 = base.clone();
    let got = tokio::task::spawn_blocking(move || {
        let first = vec![1u8; part];
        let second = vec![2u8; 1 << 20];
        let too_big = bytes_call("PUT", &format!("{b2}/0"), &adm, Some(vec![1u8; part + 1])).0;
        let put0 = bytes_call("PUT", &format!("{b2}/0"), &adm, Some(first.clone())).0;
        // Only the sender adds parts.
        let kim_put = bytes_call("PUT", &format!("{b2}/1"), &k, Some(second.clone())).0;
        let early = bytes_call("GET", &format!("{b2}/0"), &adm, None).0;
        let put1 = bytes_call("PUT", &format!("{b2}/1"), &adm, Some(second.clone())).0;
        let too_far = bytes_call("PUT", &format!("{b2}/2"), &adm, Some(second)).0;
        (put0, kim_put, early, put1, too_far, first, too_big)
    })
    .await
    .unwrap();
    assert_eq!((got.6, got.0, got.1, got.2, got.3, got.4), (413, 204, 403, 404, 204, 400));
    assert_eq!(post(&addr, &format!("/v1/chat/files/{id}/done"), Some(&admin), json!({})).await.0, 204);
    let first = got.5;
    let (adm, k) = (admin.clone(), kim.clone());
    let (mine, hers) = tokio::task::spawn_blocking(move || {
        (bytes_call("GET", &format!("{base}/0"), &adm, None), bytes_call("GET", &format!("{base}/0"), &k, None).0)
    })
    .await
    .unwrap();
    assert_eq!(mine, (200, first));
    assert_eq!(hers, 404, "only people in the chat get the parts");

    // The admins' limit.
    post(&addr, "/v1/admin/settings", Some(&admin), json!({ "chat_file_limit_mb": 1 })).await;
    assert_eq!(post(&addr, "/v1/chat/files", Some(&admin), json!({ "conv": conv.id, "size": 2 << 20 })).await.0, 413);
}

#[tokio::test]
async fn whiteboards_are_for_the_chats_members_only() {
    let Some(db) = fresh_db("boards").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let kim = add_person(&addr, &admin, "kim", false).await;

    // Kim makes a group of her own and a board in it.
    let mut k = Chat::connect(&addr, &kim).await;
    k.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    k.send(ToServer::CreateGroup { title: "Mine".into(), members: vec![], purpose: String::new() }).await;
    let ToApp::Conversation { conv } = k.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    let (status, sp) = post(&addr, "/v1/chat/boards", Some(&kim), json!({ "conv": conv.id })).await;
    assert_eq!(status, 200, "{sp}");
    let space = sp["space"].as_i64().unwrap();
    // The same space each time.
    assert_eq!(post(&addr, "/v1/chat/boards", Some(&kim), json!({ "conv": conv.id })).await.1["space"], json!(space));
    let (status, n) =
        post(&addr, "/v1/nodes", Some(&kim), json!({ "space": space, "parent": null, "kind": "file", "meta": "x" }))
            .await;
    assert_eq!(status, 200, "{n}");
    let node = n["id"].as_i64().unwrap();
    assert_eq!(get(&addr, &format!("/v1/nodes/{node}"), Some(&kim)).await.0, 200);

    // Alex, an admin but not in the chat, gets nowhere; it's not in the spaces list.
    assert_eq!(post(&addr, "/v1/chat/boards", Some(&admin), json!({ "conv": conv.id })).await.0, 403);
    assert_eq!(get(&addr, &format!("/v1/nodes/{node}"), Some(&admin)).await.0, 404);
    let (_, spaces) = get(&addr, "/v1/spaces", Some(&kim)).await;
    assert!(spaces.as_array().unwrap().iter().all(|s| s["id"] != json!(space)));
}

#[tokio::test]
async fn calls_ring_and_pass_frames_along() {
    use xenify_chat_proto::CallAction;
    let Some(db) = fresh_db("calls").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let kim = add_person(&addr, &admin, "kim", false).await;
    let kim_id = sign_in(&addr, "kim", "kim password 1").await.1["account"]["id"].as_i64().unwrap();

    let mut a = Chat::connect(&addr, &admin).await;
    a.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    let mut k = Chat::connect(&addr, &kim).await;
    k.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    a.send(ToServer::OpenDirect { user: "kim".into() }).await;
    let ToApp::Conversation { conv } = a.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };

    // Alex calls; Kim's app rings.
    a.send(ToServer::Call { conv: conv.id, action: CallAction::Start, video: true }).await;
    let ToApp::Call { call } = k.wait(|m| matches!(m, ToApp::Call { .. })).await else { unreachable!() };
    assert_eq!((call.people.len(), call.video), (1, true));
    // Only people in the call reach its frames.
    let media = |token: &str| {
        // As browsers sign in: the key as a subprotocol, not in the address.
        let mut req = format!("ws://{addr}/v1/call/{}/media", call.id).into_client_request().unwrap();
        req.headers_mut().remove("Authorization");
        req.headers_mut().insert("Sec-WebSocket-Protocol", format!("xenify-chat, {token}").parse().unwrap());
        tokio_tungstenite::connect_async(req)
    };
    assert!(media(&kim).await.is_err(), "not in the call yet");
    k.send(ToServer::Call { conv: conv.id, action: CallAction::Join, video: false }).await;
    k.wait(|m| matches!(m, ToApp::Call { call } if call.people.len() == 2)).await;
    let (mut am, _) = media(&admin).await.unwrap();
    let (mut km, _) = media(&kim).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // A frame from Kim reaches Alex, stamped with who sent it.
    km.send(Ws::Binary(vec![9, 8, 7].into())).await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), am.next()).await.unwrap().unwrap().unwrap();
    let Ws::Binary(b) = got else { panic!("expected a frame") };
    assert_eq!(&b[..8], &kim_id.to_be_bytes());
    assert_eq!(&b[8..], &[9, 8, 7]);

    // Kim hangs up: between two people, that ends it for both.
    k.send(ToServer::Call { conv: conv.id, action: CallAction::Leave, video: false }).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.people.is_empty() && !call.declined)).await;

    // Alex calls again and Kim says no: Alex's app hears so, and it ends.
    a.send(ToServer::Call { conv: conv.id, action: CallAction::Start, video: false }).await;
    k.wait(|m| matches!(m, ToApp::Call { call } if call.people.len() == 1)).await;
    k.send(ToServer::Call { conv: conv.id, action: CallAction::Decline, video: false }).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.people.is_empty() && call.declined)).await;
    k.wait(|m| matches!(m, ToApp::Call { call } if call.people.is_empty())).await;
}

#[tokio::test]
async fn recording_a_call_counts_only_who_turns_back_on() {
    use xenify_chat_proto::CallAction;
    let Some(db) = fresh_db("recording").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let mut tokens = Vec::new();
    for who in ["kim", "lee"] {
        let token = add_person(&addr, &admin, who, false).await;
        let id = sign_in(&addr, who, &format!("{who} password 1")).await.1["account"]["id"].as_i64().unwrap();
        tokens.push((token, id));
    }
    let [(kim, kim_id), (lee, lee_id)] = [tokens[0].clone(), tokens[1].clone()];
    let mut a = Chat::connect(&addr, &admin).await;
    a.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    let mut k = Chat::connect(&addr, &kim).await;
    k.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    let mut l = Chat::connect(&addr, &lee).await;
    l.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    a.send(ToServer::CreateGroup {
        title: "Planning".into(),
        members: vec!["kim".into(), "lee".into()],
        purpose: String::new(),
    })
    .await;
    let ToApp::Conversation { conv } = a.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    act(&mut a, conv.id, CallAction::Start).await;
    act(&mut k, conv.id, CallAction::Join).await;
    let ToApp::Call { call: info } = k.wait(|m| matches!(m, ToApp::Call { call } if call.people.len() == 2)).await
    else {
        unreachable!()
    };
    assert!(info.can_record && info.recording.is_none());

    // Lee isn't in the call, so can't record it.
    act(&mut l, conv.id, CallAction::Record).await;
    l.wait(|m| matches!(m, ToApp::Error { code, .. } if code == "not_in_call")).await;

    // Kim records: everyone hears of it, then whose apps went quiet and who
    // turned something back on.
    act(&mut k, conv.id, CallAction::Record).await;
    let ToApp::Call { call: info } = a.wait(|m| matches!(m, ToApp::Call { call } if call.recording.is_some())).await
    else {
        unreachable!()
    };
    let r = info.recording.unwrap();
    assert_eq!((r.by.id, r.round), (kim_id, 1));
    act(&mut a, conv.id, CallAction::RecordingOff).await;
    act(&mut k, conv.id, CallAction::RecordingOff).await;
    act(&mut k, conv.id, CallAction::InRecording).await;
    let ToApp::Call { call: info } = a
        .wait(|m| matches!(m, ToApp::Call { call } if call.recording.as_ref().is_some_and(|r| !r.in_it.is_empty())))
        .await
    else {
        unreachable!()
    };
    let r = info.recording.unwrap();
    assert_eq!(r.in_it, [kim_id]);
    assert_eq!(r.off.len(), 2);
    // A second recording at the same time isn't allowed; only Kim stops hers.
    act(&mut a, conv.id, CallAction::Record).await;
    a.wait(|m| matches!(m, ToApp::Error { code, .. } if code == "recording")).await;
    act(&mut a, conv.id, CallAction::StopRecording).await;
    act(&mut k, conv.id, CallAction::StopRecording).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.recording.is_none() && call.people.len() == 2)).await;

    // Recording again starts a new round; Kim leaving stops it.
    act(&mut k, conv.id, CallAction::Record).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.recording.as_ref().is_some_and(|r| r.round == 2))).await;
    act(&mut k, conv.id, CallAction::Leave).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.recording.is_none() && call.people.len() == 1)).await;

    // An admin turns recording off.
    let (status, _) = post(&addr, "/v1/admin/settings", Some(&admin), json!({ "call_recording": false })).await;
    assert_eq!(status, 204);
    let (_, set) = get(&addr, "/v1/admin/settings", Some(&admin)).await;
    assert_eq!(set["call_recording"], false);
    act(&mut a, conv.id, CallAction::Record).await;
    a.wait(|m| matches!(m, ToApp::Error { code, .. } if code == "no_recording")).await;
    let _ = lee_id;
}

async fn act(c: &mut Chat, conv: i64, action: xenify_chat_proto::CallAction) {
    c.send(ToServer::Call { conv, action, video: false }).await;
}

#[tokio::test]
async fn old_messages_and_their_files_go_after_the_admins_limit() {
    let Some(db) = fresh_db("chatkeep").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let mut a = Chat::connect(&addr, &admin).await;
    let ToApp::Welcome { keep_days, .. } = a.wait(|m| matches!(m, ToApp::Welcome { .. })).await else { unreachable!() };
    assert_eq!(keep_days, None, "for ever by default");
    a.send(ToServer::CreateGroup { title: "Plans".into(), members: vec![], purpose: String::new() }).await;
    let ToApp::Conversation { conv } = a.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    // A file and its message, then a newer message.
    let (_, started) = post(&addr, "/v1/chat/files", Some(&admin), json!({ "conv": conv.id, "size": 3 })).await;
    let id = started["id"].as_str().unwrap().to_string();
    let url = format!("http://{addr}/v1/chat/files/{id}/0");
    let adm = admin.clone();
    assert_eq!(
        tokio::task::spawn_blocking(move || bytes_call("PUT", &url, &adm, Some(vec![1, 2, 3])).0).await.unwrap(),
        204
    );
    a.send(ToServer::Send { conv: conv.id, client_id: "f".into(), text: format!("file {id}") }).await;
    a.send(ToServer::Send { conv: conv.id, client_id: "n".into(), text: "newer".into() }).await;
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "newer")).await;
    let c = server.pool().get().await.unwrap();
    c.execute("UPDATE messages SET sent_at = now() - interval '40 days' WHERE body <> 'newer'", &[]).await.unwrap();
    c.execute("UPDATE chat_files SET created = now() - interval '40 days'", &[]).await.unwrap();
    assert_eq!(xenify_server::chatjobs::expire(&server).await.unwrap(), 0, "kept for ever");

    assert_eq!(post(&addr, "/v1/admin/settings", Some(&admin), json!({ "chat_keep_days": -1 })).await.0, 400);
    assert_eq!(post(&addr, "/v1/admin/settings", Some(&admin), json!({ "chat_keep_days": 30 })).await.0, 204);
    assert_eq!(get(&addr, "/v1/admin/settings", Some(&admin)).await.1["chat_keep_days"], 30);
    assert_eq!(xenify_server::chatjobs::expire(&server).await.unwrap(), 1);
    let left: Vec<String> = c.query("SELECT body FROM messages", &[]).await.unwrap().iter().map(|r| r.get(0)).collect();
    assert_eq!(left, ["newer"]);
    assert_eq!(c.query("SELECT 1 FROM chat_files", &[]).await.unwrap().len(), 0);
    let data = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(db.rsplit('/').next().unwrap());
    assert!(!data.join("chat-files").join(&id).exists());
    // Apps hear the chat changed, and new connections the server's limit.
    a.wait(|m| matches!(m, ToApp::Conversation { conv: c } if c.last.as_ref().is_some_and(|l| l.text == "newer")))
        .await;
    let mut a2 = Chat::connect(&addr, &admin).await;
    let ToApp::Welcome { keep_days, .. } = a2.wait(|m| matches!(m, ToApp::Welcome { .. })).await else {
        unreachable!()
    };
    assert_eq!(keep_days, Some(30));
}

#[tokio::test]
async fn browsers_subscribe_to_push_and_reminder_buttons_need_their_token() {
    let Some(db) = fresh_db("chatpush").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let (status, k) = get(&addr, "/v1/push/key", None).await;
    assert_eq!(status, 200);
    assert_eq!(k["key"].as_str().unwrap().len(), 87, "an uncompressed P-256 point");
    assert_eq!(get(&addr, "/v1/push/key", None).await.1, k, "the same key each time");
    // A browser's subscription (a real point; any 16 bytes of auth).
    let point = k["key"].as_str().unwrap();
    let sub = json!({ "endpoint": "https://push.example.org/send/abc", "keys": { "p256dh": point, "auth": "AAAAAAAAAAAAAAAAAAAAAA" } });
    assert_eq!(post(&addr, "/v1/push/subscribe", None, sub.clone()).await.0, 401);
    assert_eq!(post(&addr, "/v1/push/subscribe", Some(&admin), sub.clone()).await.0, 204);
    let bad =
        json!({ "endpoint": "http://127.0.0.1/x", "keys": { "p256dh": point, "auth": "AAAAAAAAAAAAAAAAAAAAAA" } });
    assert_eq!(post(&addr, "/v1/push/subscribe", Some(&admin), bad).await.0, 400);
    let c = server.pool().get().await.unwrap();
    assert_eq!(c.query("SELECT 1 FROM push_subs", &[]).await.unwrap().len(), 1);
    assert_eq!(post(&addr, "/v1/push/act", None, json!({ "token": "made.up" })).await.0, 403);
    assert_eq!(
        post(&addr, "/v1/push/unsubscribe", Some(&admin), json!({ "endpoint": "https://push.example.org/send/abc" }))
            .await
            .0,
        204
    );
    assert_eq!(c.query("SELECT 1 FROM push_subs", &[]).await.unwrap().len(), 0);
}

/// The conversation called `title` (or, for a direct chat, with `title` in
/// it; for calendars and busy times, with that purpose), once it's made.
async fn made(c: &mut Chat, title: &str) -> i64 {
    let m = c
        .wait(|m| match m {
            ToApp::Conversation { conv } => {
                conv.title == title || conv.purpose == title || conv.members.iter().any(|u| u.username == title)
            }
            _ => false,
        })
        .await;
    match m {
        ToApp::Conversation { conv } => conv.id,
        _ => unreachable!(),
    }
}

/// A server with the people, chats, calendar, busy times and rooms the
/// site's Chat pictures show, kept running for a headless browser:
/// `XENIFY_DEMO_APPS=<dir with chat/> cargo test -p xenify-server --test chat screens -- --ignored --nocapture`
/// prints the address and Alex's session.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn screens() {
    let Some(db) = fresh_db("chat_screens").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    if let Ok(apps) = std::env::var("XENIFY_DEMO_APPS") {
        server.host_apps(apps.into());
    }
    let admin = set_up(&addr, &server).await;
    let person = |user: &'static str, name: &'static str| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move {
            let body = json!({ "username": user, "display_name": name });
            let (_, p) = post(&addr, "/v1/admin/people", Some(&admin), body).await;
            use_link(&addr, p["code"].as_str().unwrap(), user, &format!("{user} password 1")).await.1["token"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let (priya, sam) = (person("priya", "Priya Shah").await, person("sam", "Sam Lee").await);
    let mut a = Chat::connect(&addr, &admin).await;
    let mut p = Chat::connect(&addr, &priya).await;
    let mut s = Chat::connect(&addr, &sam).await;
    let say =
        |conv: i64, text: &str| ToServer::Send { conv, client_id: format!("demo{}", text.len()), text: text.into() };
    let pause = || tokio::time::sleep(Duration::from_millis(150));

    // Announcements, a few days ago in the list (sent first).
    a.send(ToServer::CreateChannel { title: "Announcements".into(), open: true, purpose: String::new() }).await;
    let ann = made(&mut a, "Announcements").await;
    a.send(say(ann, "The office is closed on Friday afternoon for the summer party.")).await;
    pause().await;
    // Priya, directly.
    a.send(ToServer::OpenDirect { user: "priya".into() }).await;
    let dm = made(&mut a, "priya").await;
    p.send(say(dm, "No problem! I'll send the final versions after lunch.")).await;
    pause().await;
    // The Website launch group, with a picture.
    a.send(ToServer::CreateGroup {
        title: "Website launch".into(),
        members: vec!["priya".into(), "sam".into()],
        purpose: String::new(),
    })
    .await;
    let group = made(&mut a, "Website launch").await;
    p.send(say(group, "And here's the picture for the launch post:")).await;
    pause().await;
    let picture =
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/media/draw-film.webp"))
            .unwrap();
    let (_, started) =
        post(&addr, "/v1/chat/files", Some(&priya), json!({ "conv": group, "size": picture.len() })).await;
    let id = started["id"].as_str().unwrap().to_string();
    let part = started["part"].as_u64().unwrap() as usize;
    for (n, chunk) in picture.chunks(part).enumerate() {
        bytes_call("PUT", &format!("http://{addr}/v1/chat/files/{id}/{n}"), &priya, Some(chunk.to_vec()));
    }
    post(&addr, &format!("/v1/chat/files/{id}/done"), Some(&priya), json!({})).await;
    let file =
        json!({ "id": id, "name": "launch-post.webp", "size": picture.len(), "mime": "image/webp", "part": part });
    p.send(say(group, &format!("~file:{file}"))).await;
    pause().await;
    s.send(say(group, "Love it. The colours work really well.")).await;
    pause().await;
    a.send(say(group, "Shall we go through everything together tomorrow at 10? I'll book Room 2.")).await;
    pause().await;
    p.send(say(group, "Works for me.")).await;

    // Times: today at 16:00 here, and busy times for Find a time.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let (hour, day) = (3_600_000, 86_400_000);
    let today = now - now % day;
    // Alex's calendar: the launch review.
    a.send(ToServer::CreateGroup { title: "Calendar".into(), members: vec![], purpose: "calendar".into() }).await;
    let cal = made(&mut a, "calendar").await;
    let event = json!({ "op": "put", "uid": "demo-launch-review", "event": {
        "id": 0, "title": "Launch review", "start": today + 16 * hour, "end": today + 17 * hour, "kind": "event",
    }});
    a.send(say(cal, &format!("~cal1:{event}"))).await;
    // Priya's and Sam's busy times, shared with everyone.
    for (c, busy) in [(&mut p, vec![(9, 10), (13, 14), (15, 16)]), (&mut s, vec![(8, 9), (14, 16)])] {
        c.send(ToServer::CreateChannel { title: "Busy".into(), open: true, purpose: "busy".into() }).await;
        let ch = made(c, "busy").await;
        let mut times = Vec::new();
        let mut work = Vec::new();
        for d in 0..14 {
            let base = today + d * day;
            work.push((base + 8 * hour, base + 18 * hour));
            times.extend(busy.iter().map(|(f, t)| (base + f * hour, base + t * hour)));
        }
        let post_ = json!({ "from": today, "to": today + 60 * day, "busy": times, "work": work, "away": [] });
        c.send(say(ch, &format!("~busy1:{post_}"))).await;
    }
    // Rooms and things, and Room 2 booked for the review.
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
        thing("Board room", "room", "Ground floor", 16, false),
        thing("Room 2", "room", "2nd floor", 8, false),
        thing("Projector", "equipment", "Front desk", 0, false),
        thing("The Design of Everyday Things", "book", "Library, shelf B", 0, true),
    ] {
        a.send(t).await;
        pause().await;
    }
    a.send(ToServer::Things).await;
    let things = a.wait(|m| matches!(m, ToApp::Things { list, .. } if list.iter().any(|t| t.name == "Room 2"))).await;
    if let ToApp::Things { list, .. } = things
        && let Some(room) = list.iter().find(|t| t.name == "Room 2")
    {
        a.send(ToServer::Book {
            thing: room.id,
            start: today + 16 * hour,
            end: today + 17 * hour,
            meeting: None,
            tag: String::new(),
        })
        .await;
    }
    pause().await;
    // Alex has read everything.
    for conv in [ann, dm, group] {
        a.send(ToServer::Read { conv, seq: 1000 }).await;
    }
    pause().await;
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    println!("SCREENS addr=http://{addr}");
    println!("SCREENS session={}", json!({ "token": admin, "account": me }));
    tokio::time::sleep(Duration::from_secs(3600)).await;
}

#[tokio::test]
async fn the_servers_roles_say_who_makes_channels_and_apps_and_who_moderates() {
    let Some(db) = fresh_db("chatroles").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let priya = add_person(&addr, &admin, "priya", false).await;
    add_person(&addr, &admin, "sam", false).await;
    let mut a = Chat::connect(&addr, &admin).await;
    a.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    let mut p = Chat::connect(&addr, &priya).await;
    p.wait(|m| matches!(m, ToApp::Welcome { .. })).await;

    // Everyone may make channels and apps until the admin says otherwise.
    p.send(ToServer::CreateChannel { title: "Priya's".into(), open: true, purpose: String::new() }).await;
    p.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Priya's")).await;
    let (_, roles) = get(&addr, "/v1/roles", Some(&admin)).await;
    let everyone =
        roles["roles"].as_array().unwrap().iter().find(|r| r["name"] == "Everyone").unwrap()["id"].as_i64().unwrap();
    let (status, body) =
        post(&addr, &format!("/v1/admin/roles/{everyone}"), Some(&admin), json!({ "perms": ["chat.create_channel"] }))
            .await;
    assert_eq!(status, 200, "{body}");
    p.send(ToServer::CreateChannel { title: "Open".into(), open: true, purpose: String::new() }).await;
    let ToApp::Error { code, .. } = p.wait(|m| matches!(m, ToApp::Error { .. })).await else { unreachable!() };
    assert_eq!(code, "not_allowed");
    p.send(ToServer::CreateApp {
        name: "bot".into(),
        display_name: String::new(),
        description: String::new(),
        webhook: String::new(),
        mode: Default::default(),
    })
    .await;
    p.wait(|m| matches!(m, ToApp::Error { code, .. } if code == "not_allowed")).await;
    // Private channels still, and the calendar's busy-times channel is never refused.
    p.send(ToServer::CreateChannel { title: "Closed".into(), open: false, purpose: String::new() }).await;
    p.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Closed")).await;
    p.send(ToServer::CreateChannel { title: "busy".into(), open: true, purpose: xenify_chat_proto::BUSY.into() }).await;
    p.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.purpose == xenify_chat_proto::BUSY)).await;

    // Admins moderate: take down Priya's message and take Sam out of her group.
    p.send(ToServer::CreateGroup {
        title: "Team".into(),
        members: vec!["alex".into(), "sam".into()],
        purpose: String::new(),
    })
    .await;
    let ToApp::Conversation { conv: g } =
        a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Team")).await
    else {
        unreachable!()
    };
    p.send(ToServer::Send { conv: g.id, client_id: "p1".into(), text: "Something rude".into() }).await;
    let ToApp::Message { msg } = a.wait(|m| matches!(m, ToApp::Message { msg } if msg.client_id == "p1")).await else {
        unreachable!()
    };
    a.send(ToServer::Delete { conv: g.id, seq: msg.seq }).await;
    p.wait(|m| matches!(m, ToApp::Message { msg } if msg.client_id == "p1" && msg.deleted)).await;
    a.send(ToServer::RemovePerson { conv: g.id, user: "sam".into() }).await;
    let id = g.id;
    p.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == id && conv.member_count == 2)).await;
}

#[tokio::test]
async fn admins_turn_threads_off_and_never_read_chats_they_are_not_in() {
    let Some(db) = fresh_db("chatthreads").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let priya = add_person(&addr, &admin, "priya", false).await;
    add_person(&addr, &admin, "sam", false).await;
    let mut p = Chat::connect(&addr, &priya).await;
    // Threads are on at first; no AI is set up.
    let ToApp::Welcome { threads, ai, .. } = p.wait(|m| matches!(m, ToApp::Welcome { .. })).await else {
        unreachable!()
    };
    assert!(threads && !ai);
    p.send(ToServer::OpenDirect { user: "sam".into() }).await;
    let ToApp::Conversation { conv: d } = p.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    p.send(ToServer::Send { conv: d.id, client_id: "1".into(), text: "Private salary numbers".into() }).await;
    let ToApp::Message { msg: root } = p.wait(|m| matches!(m, ToApp::Message { .. })).await else { unreachable!() };
    p.send(ToServer::Reply { conv: d.id, thread: root.seq, client_id: "2".into(), text: "More salary detail".into() })
        .await;
    p.wait(|m| matches!(m, ToApp::Message { msg } if msg.thread.is_some())).await;

    // The admin isn't in Priya's chat with Sam: nothing of it, however asked.
    let mut a = Chat::connect(&addr, &admin).await;
    a.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    a.send(ToServer::Search {
        q: "salary".into(),
        conv: None,
        sort: Default::default(),
        after: String::new(),
        tag: "s".into(),
    })
    .await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Hits { .. })).await, ToApp::Hits { hits, total: 0, .. } if hits.is_empty())
    );
    a.send(ToServer::Search {
        q: "salary".into(),
        conv: Some(d.id),
        sort: Default::default(),
        after: String::new(),
        tag: "s".into(),
    })
    .await;
    assert!(matches!(a.wait(|m| matches!(m, ToApp::Hits { .. })).await, ToApp::Hits { hits, .. } if hits.is_empty()));
    for m in [
        ToServer::History { conv: d.id, before: None, limit: 50, tag: String::new() },
        ToServer::Thread { conv: d.id, thread: root.seq, tag: String::new() },
        ToServer::Delete { conv: d.id, seq: root.seq },
        ToServer::RemovePerson { conv: d.id, user: "sam".into() },
    ] {
        a.send(m.clone()).await;
        let reply = a.wait(|m| matches!(m, ToApp::Error { .. } | ToApp::History { .. })).await;
        assert!(matches!(&reply, ToApp::Error { code, .. } if code == "not_member"), "{m:?}: {reply:?}");
    }
    a.send(ToServer::Ask { q: "salary?".into(), conv: Some(d.id), tag: String::new() }).await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Answer { .. })).await, ToApp::Answer { error, sources, .. } if !error.is_empty() && sources.is_empty())
    );

    // Threads off: replies show in the chat itself, and new ones aren't taken.
    let (status, body) = post(&addr, "/v1/admin/settings", Some(&admin), json!({ "chat_threads": false })).await;
    assert_eq!(status, 204, "{body}");
    let (_, set) = get(&addr, "/v1/admin/settings", Some(&admin)).await;
    assert_eq!(set["chat_threads"], false);
    let mut p2 = Chat::connect(&addr, &priya).await;
    let ToApp::Welcome { threads, .. } = p2.wait(|m| matches!(m, ToApp::Welcome { .. })).await else { unreachable!() };
    assert!(!threads);
    p2.send(ToServer::History { conv: d.id, before: None, limit: 50, tag: String::new() }).await;
    let ToApp::History { messages, .. } = p2.wait(|m| matches!(m, ToApp::History { .. })).await else { unreachable!() };
    assert_eq!(messages.len(), 2);
    p2.send(ToServer::Reply { conv: d.id, thread: root.seq, client_id: "3".into(), text: "Again".into() }).await;
    assert!(
        matches!(p2.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "no_threads")
    );
}

/// A pretend AI runner: picks words the first time, then answers citing
/// message 1, and keeps what it was asked.
async fn pretend_ai() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = asked.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |axum::Json(v): axum::Json<serde_json::Value>| {
            let kept = kept.clone();
            async move {
                let text = v["messages"].to_string();
                kept.lock().unwrap().push(text.clone());
                let answer = if text.contains("Question:") {
                    "The launch review is tomorrow at 10 in Room 2 [1]. Sam booked the room [2]."
                } else {
                    "<think>hm</think>launch review room tomorrow"
                };
                axum::Json(json!({ "choices": [{ "message": { "role": "assistant", "content": answer } }] }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (at, asked)
}

#[tokio::test]
async fn people_ask_the_servers_ai_about_their_own_messages() {
    let Some(db) = fresh_db("chatask").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, server) = serve(&db).await;
    let admin = set_up(&addr, &server).await;
    let priya = add_person(&addr, &admin, "priya", false).await;
    add_person(&addr, &admin, "sam", false).await;
    let (runner, asked) = pretend_ai().await;
    let (status, body) = post(
        &addr,
        "/v1/admin/ai",
        Some(&admin),
        json!({ "runner": "ollama", "url": format!("{runner}/v1"), "text_model": "qwen" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let mut p = Chat::connect(&addr, &priya).await;
    let ToApp::Welcome { ai, .. } = p.wait(|m| matches!(m, ToApp::Welcome { .. })).await else { unreachable!() };
    assert!(ai, "AI set up, and Everyone may write with it");
    p.send(ToServer::OpenDirect { user: "sam".into() }).await;
    let ToApp::Conversation { conv: d } = p.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    p.send(ToServer::Send { conv: d.id, client_id: "1".into(), text: "Launch review tomorrow at 10, Room 2".into() })
        .await;
    p.wait(|m| matches!(m, ToApp::Message { .. })).await;
    // The admin's own chat with Sam stays out of Priya's answer.
    let mut a = Chat::connect(&addr, &admin).await;
    a.wait(|m| matches!(m, ToApp::Welcome { .. })).await;
    a.send(ToServer::OpenDirect { user: "sam".into() }).await;
    let ToApp::Conversation { conv: ad } = a.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    a.send(ToServer::Send {
        conv: ad.id,
        client_id: "2".into(),
        text: "Confidential: the launch review is cancelled".into(),
    })
    .await;
    a.wait(|m| matches!(m, ToApp::Message { .. })).await;

    p.send(ToServer::Ask { q: "When is the launch review?".into(), conv: None, tag: "a".into() }).await;
    let ToApp::Answer { text, sources, error, .. } = p.wait(|m| matches!(m, ToApp::Answer { .. })).await else {
        unreachable!()
    };
    assert_eq!(error, "");
    assert!(text.starts_with("The launch review is tomorrow"), "{text}");
    assert_eq!(
        sources.iter().map(|h| h.msg.text.as_str()).collect::<Vec<_>>(),
        ["Launch review tomorrow at 10, Room 2"]
    );
    let all = asked.lock().unwrap().join("\n");
    assert!(all.contains("Launch review tomorrow") && !all.contains("Confidential"), "{all}");
}

/// A server with a thread, many matches to page through and a pretend AI,
/// for screenshots of threads, search and Ask AI in the browser:
/// `XENIFY_DEMO_APPS=<dir with chat/> cargo test -p xenify-server --test chat thread_screens -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn thread_screens() {
    let Some(db) = fresh_db("chat_thread_screens").await else {
        return eprintln!("no PostgreSQL to test with: skipped");
    };
    let (addr, server) = serve(&db).await;
    if let Ok(apps) = std::env::var("XENIFY_DEMO_APPS") {
        server.host_apps(apps.into());
    }
    let admin = set_up(&addr, &server).await;
    let person = |user: &'static str, name: &'static str| {
        let (addr, admin) = (addr.clone(), admin.clone());
        async move {
            let body = json!({ "username": user, "display_name": name });
            let (_, p) = post(&addr, "/v1/admin/people", Some(&admin), body).await;
            use_link(&addr, p["code"].as_str().unwrap(), user, &format!("{user} password 1")).await.1["token"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let (priya, sam) = (person("priya", "Priya Shah").await, person("sam", "Sam Lee").await);
    let (runner, _) = pretend_ai().await;
    post(
        &addr,
        "/v1/admin/ai",
        Some(&admin),
        json!({ "runner": "ollama", "url": format!("{runner}/v1"), "text_model": "qwen" }),
    )
    .await;
    let mut a = Chat::connect(&addr, &admin).await;
    let mut p = Chat::connect(&addr, &priya).await;
    let mut s = Chat::connect(&addr, &sam).await;
    let pause = || tokio::time::sleep(Duration::from_millis(150));
    let say = |conv: i64, n: &str, text: &str| ToServer::Send { conv, client_id: n.into(), text: text.into() };
    a.send(ToServer::CreateGroup {
        title: "Website launch".into(),
        members: vec!["priya".into(), "sam".into()],
        purpose: String::new(),
    })
    .await;
    let g = made(&mut a, "Website launch").await;
    p.send(say(g, "1", "The new pages are ready for a last look.")).await;
    pause().await;
    a.send(say(g, "2", "Shall we go through everything together tomorrow at 10? The launch review, in Room 2.")).await;
    let ToApp::Message { msg: root } = p.wait(|m| matches!(m, ToApp::Message { msg } if msg.client_id == "2")).await
    else {
        unreachable!()
    };
    pause().await;
    p.send(ToServer::Reply { conv: g, thread: root.seq, client_id: "t1".into(), text: "Works for me.".into() }).await;
    pause().await;
    s.send(ToServer::Reply {
        conv: g,
        thread: root.seq,
        client_id: "t2".into(),
        text: "I booked Room 2 and the projector.".into(),
    })
    .await;
    pause().await;
    p.send(ToServer::Reply {
        conv: g,
        thread: root.seq,
        client_id: "t3".into(),
        text: "I'll bring the printed proofs too.".into(),
    })
    .await;
    pause().await;
    s.send(say(g, "3", "Love the colours on the home page.")).await;
    // Plenty of "launch" to page through.
    a.send(ToServer::OpenDirect { user: "priya".into() }).await;
    let ToApp::Conversation { conv: dm } =
        a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.kind == xenify_chat_proto::Kind::Direct)).await
    else {
        unreachable!()
    };
    let dm = dm.id;
    for i in 0..45 {
        let text = format!(
            "Launch checklist item {}: {}",
            i + 1,
            ["copy", "pictures", "links", "translations", "forms"][i % 5]
        );
        a.send(say(dm, &format!("d{i}"), &text)).await;
    }
    pause().await;
    a.send(ToServer::Read { conv: dm, seq: 1000 }).await;
    let (_, me) = get(&addr, "/v1/me", Some(&admin)).await;
    println!("SCREENS addr=http://{addr}");
    println!(
        "SCREENS session={}",
        json!({ "server": format!("http://{addr}"), "token": admin, "user": me["account"].clone() })
    );
    println!("SCREENS me={me}");
    tokio::time::sleep(Duration::from_secs(3600)).await;
}
