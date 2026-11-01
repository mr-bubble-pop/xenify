//! The server against a real PostgreSQL. Each test gets a fresh database.
//! `XENIFY_TEST_PG` is an admin address (default: a local test cluster on
//! port 55432); without a reachable PostgreSQL the tests say so and pass.

mod common;

use common::*;
use xenify_chat_proto::{AppMode, Kind, Notify, Presence, Role, ToApp, ToServer};

fn conv_of(m: ToApp) -> xenify_chat_proto::Conversation {
    match m {
        ToApp::Conversation { conv } => conv,
        other => panic!("not a conversation: {other:?}"),
    }
}

fn is_conv(id: i64) -> impl Fn(&ToApp) -> bool {
    move |m| matches!(m, ToApp::Conversation { conv } if conv.id == id)
}

fn is_error(code: &'static str) -> impl Fn(&ToApp) -> bool {
    move |m| matches!(m, ToApp::Error { code: c, .. } if c == code)
}

#[tokio::test]
async fn direct_chats_messages_and_read_markers() {
    let Some(db) = fresh_db("direct").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    let mut b2 = App::connect(&addr, &bo.token).await; // Bo's phone

    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    let conv = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    assert_eq!((conv.kind, conv.member_count, conv.can_post), (Kind::Direct, 2, true));
    // Opening it from Bo's side finds the same chat.
    b.send(ToServer::OpenDirect { user: "ana".into() }).await;
    assert_eq!(conv_of(b.wait(is_conv(conv.id)).await).id, conv.id);

    a.send(ToServer::Send { conv: conv.id, client_id: "a1".into(), text: "Lunch at noon?".into() }).await;
    // Sent twice (a flaky connection): stored once.
    a.send(ToServer::Send { conv: conv.id, client_id: "a1".into(), text: "Lunch at noon?".into() }).await;
    let ToApp::Message { msg } = b.wait(|m| matches!(m, ToApp::Message { .. })).await else { unreachable!() };
    assert_eq!((msg.seq, msg.sender.username.as_str(), msg.text.as_str()), (1, "ana", "Lunch at noon?"));
    a.send(ToServer::Send { conv: conv.id, client_id: "a2".into(), text: "The usual place".into() }).await;
    b.wait(|m| matches!(m, ToApp::Message { msg } if msg.seq == 2)).await;

    // Bo hasn't read it: bold in his list, with the last message.
    b.send(ToServer::List).await;
    let ToApp::Conversations { list } = b.wait(|m| matches!(m, ToApp::Conversations { .. })).await else {
        unreachable!()
    };
    let c = list.iter().find(|c| c.id == conv.id).unwrap();
    assert!(c.unread());
    assert_eq!(c.last.as_ref().unwrap().text, "The usual place");
    // Reading on one device clears it on the other.
    b.send(ToServer::Read { conv: conv.id, seq: 2 }).await;
    assert!(matches!(b2.wait(|m| matches!(m, ToApp::Read { .. })).await, ToApp::Read { seq: 2, .. }));
    // "Mark unread from here" moves it back, everywhere; reading again moves it forward.
    b.send(ToServer::Unread { conv: conv.id, seq: 1 }).await;
    assert!(matches!(b2.wait(|m| matches!(m, ToApp::Read { .. })).await, ToApp::Read { seq: 1, .. }));
    b.send(ToServer::Read { conv: conv.id, seq: 2 }).await;
    assert!(matches!(b2.wait(|m| matches!(m, ToApp::Read { .. })).await, ToApp::Read { seq: 2, .. }));
    // Ana's own messages count as read for her.
    a.send(ToServer::List).await;
    let ToApp::Conversations { list } = a.wait(|m| matches!(m, ToApp::Conversations { .. })).await else {
        unreachable!()
    };
    assert!(!list[0].unread());

    b.send(ToServer::History { conv: conv.id, before: None, limit: 50, tag: String::new() }).await;
    let ToApp::History { messages, .. } = b.wait(|m| matches!(m, ToApp::History { .. })).await else { unreachable!() };
    assert_eq!(messages.iter().map(|m| m.seq).collect::<Vec<_>>(), [1, 2], "once each, oldest first");

    // Edits and deletes: your own messages only.
    b.send(ToServer::Edit { conv: conv.id, seq: 1, text: "hacked".into() }).await;
    b.wait(is_error("not_yours")).await;
    a.send(ToServer::Delete { conv: conv.id, seq: 1 }).await;
    let ToApp::Message { msg } = b.wait(|m| matches!(m, ToApp::Message { msg } if msg.deleted)).await else {
        unreachable!()
    };
    assert_eq!(msg.text, "");

    // Reactions: everyone sees who reacted with what; taking yours back removes it.
    b.send(ToServer::React { conv: conv.id, seq: 2, emoji: "👍".into(), on: true }).await;
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.reactions.len() == 1)).await;
    a.send(ToServer::React { conv: conv.id, seq: 2, emoji: "👍".into(), on: true }).await;
    let ToApp::Message { msg } = b
        .wait(|m| matches!(m, ToApp::Message { msg } if msg.reactions.first().is_some_and(|r| r.users.len() == 2)))
        .await
    else {
        unreachable!()
    };
    assert_eq!(msg.reactions[0].emoji, "👍");
    assert_eq!(
        msg.reactions[0].users.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(),
        ["bo", "ana"],
        "in the order they reacted"
    );
    b.send(ToServer::React { conv: conv.id, seq: 2, emoji: "👍".into(), on: false }).await;
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.reactions.first().is_some_and(|r| r.users.len() == 1))).await;
    b.send(ToServer::React { conv: conv.id, seq: 2, emoji: "great".into(), on: true }).await;
    b.wait(is_error("bad_request")).await;
    // History carries them too.
    b.send(ToServer::History { conv: conv.id, before: None, limit: 50, tag: String::new() }).await;
    let ToApp::History { messages, .. } = b.wait(|m| matches!(m, ToApp::History { .. })).await else { unreachable!() };
    assert_eq!(messages.iter().find(|m| m.seq == 2).unwrap().reactions[0].users[0].username, "ana");

    // Someone else can't read it or react to it.
    let cy = register(&addr, &s, "cy").await;
    let mut c = App::connect(&addr, &cy.token).await;
    c.send(ToServer::History { conv: conv.id, before: None, limit: 50, tag: String::new() }).await;
    c.wait(is_error("not_member")).await;
    c.send(ToServer::React { conv: conv.id, seq: 2, emoji: "😂".into(), on: true }).await;
    c.wait(is_error("not_member")).await;

    // Notes to self: a direct chat with yourself.
    a.send(ToServer::OpenDirect { user: "ana".into() }).await;
    let notes = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.member_count == 1)).await);
    assert_eq!(notes.kind, Kind::Direct);
}

#[tokio::test]
async fn groups_add_people_and_survive_their_owner_leaving() {
    let Some(db) = fresh_db("groups").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let cy = register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    let mut c = App::connect(&addr, &cy.token).await;

    a.send(ToServer::CreateGroup {
        title: "Design team".into(),
        members: vec!["bo".into(), "nobody".into()],
        purpose: String::new(),
    })
    .await;
    a.wait(is_error("no_user")).await;
    a.send(ToServer::CreateGroup { title: "Design team".into(), members: vec!["bo".into()], purpose: String::new() })
        .await;
    let g = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    assert_eq!((g.kind, g.role, g.member_count), (Kind::Group, Role::Owner, 2));
    b.wait(is_conv(g.id)).await;
    a.send(ToServer::Send { conv: g.id, client_id: "1".into(), text: "Welcome".into() }).await;
    b.wait(|m| matches!(m, ToApp::Message { .. })).await;

    // Anyone in a group can add people; newcomers see the history.
    b.send(ToServer::AddPeople { conv: g.id, users: vec!["cy".into()] }).await;
    let seen = conv_of(c.wait(is_conv(g.id)).await);
    assert_eq!(seen.member_count, 3);
    c.send(ToServer::History { conv: g.id, before: None, limit: 50, tag: String::new() }).await;
    let ToApp::History { messages, .. } = c.wait(|m| matches!(m, ToApp::History { .. })).await else { unreachable!() };
    assert_eq!(messages.len(), 1);

    // Only owners take others out.
    b.send(ToServer::RemovePerson { conv: g.id, user: "cy".into() }).await;
    b.wait(is_error("not_owner")).await;

    // The owner leaves: whoever's been in longest (Bo) takes over.
    a.send(ToServer::Leave { conv: g.id }).await;
    assert!(matches!(a.wait(|m| matches!(m, ToApp::Removed { .. })).await, ToApp::Removed { conv } if conv == g.id));
    let now = conv_of(b.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.member_count == 2)).await);
    assert_eq!(now.role, Role::Owner);
    b.send(ToServer::RemovePerson { conv: g.id, user: "cy".into() }).await;
    c.wait(|m| matches!(m, ToApp::Removed { .. })).await;
    // The last one out: it's gone.
    b.send(ToServer::Leave { conv: g.id }).await;
    b.wait(|m| matches!(m, ToApp::Removed { .. })).await;
    let left: i64 =
        s.pool().get().await.unwrap().query_one("SELECT count(*) FROM conversations", &[]).await.unwrap().get(0);
    assert_eq!(left, 0);
}

#[tokio::test]
async fn channels_are_followed_owners_post_and_closed_ones_stay_hidden() {
    let Some(db) = fresh_db("channels").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;

    a.send(ToServer::CreateChannel { title: "Build alerts".into(), open: true, purpose: String::new() }).await;
    let ch = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::CreateChannel { title: "Board notes".into(), open: false, purpose: String::new() }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id != ch.id)).await;

    // Bo finds the open one (not the closed one) and subscribes.
    b.send(ToServer::FindConversations { q: "b".into() }).await;
    let ToApp::Found { conversations, .. } = b.wait(|m| matches!(m, ToApp::Found { .. })).await else { unreachable!() };
    assert_eq!(conversations.iter().map(|f| f.title.as_str()).collect::<Vec<_>>(), ["Build alerts"]);
    assert!(!conversations[0].joined);
    // An empty search lists every channel Bo could follow.
    b.send(ToServer::FindConversations { q: "".into() }).await;
    let ToApp::Found { conversations, .. } = b.wait(|m| matches!(m, ToApp::Found { .. })).await else { unreachable!() };
    assert_eq!(conversations.iter().map(|f| f.title.as_str()).collect::<Vec<_>>(), ["Build alerts"]);
    assert_eq!(ch.owners, [ana.user.id], "who posts there");
    // Bo can read it before following (but not the closed one).
    a.send(ToServer::Send { conv: ch.id, client_id: "0".into(), text: "Build #481 passed".into() }).await;
    a.wait(|m| matches!(m, ToApp::Message { .. })).await;
    b.send(ToServer::History { conv: ch.id, before: None, limit: 20, tag: String::new() }).await;
    let ToApp::History { messages, .. } = b.wait(|m| matches!(m, ToApp::History { .. })).await else { unreachable!() };
    assert_eq!(messages[0].text, "Build #481 passed");
    b.send(ToServer::History { conv: ch.id + 1, before: None, limit: 20, tag: String::new() }).await;
    b.wait(is_error("not_member")).await;
    b.send(ToServer::Subscribe { conv: ch.id }).await;
    let mine = conv_of(b.wait(is_conv(ch.id)).await);
    assert_eq!((mine.role, mine.can_post), (Role::Member, false));

    // Only owners post; followers get the posts.
    b.send(ToServer::Send { conv: ch.id, client_id: "x".into(), text: "me too".into() }).await;
    b.wait(is_error("not_allowed")).await;
    a.send(ToServer::Send { conv: ch.id, client_id: "1".into(), text: "Build #482 failed".into() }).await;
    b.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "Build #482 failed")).await;

    // Unsubscribing is leaving.
    b.send(ToServer::Leave { conv: ch.id }).await;
    b.wait(|m| matches!(m, ToApp::Removed { .. })).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == ch.id && conv.member_count == 1)).await;
}

#[tokio::test]
async fn notification_settings_follow_you_between_devices() {
    let Some(db) = fresh_db("notify").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut a2 = App::connect(&addr, &ana.token).await;
    a.send(ToServer::CreateGroup {
        title: "Weekly planning".into(),
        members: vec!["bo".into()],
        purpose: String::new(),
    })
    .await;
    let g = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    assert_eq!(g.notify, Notify::All);

    let in_an_hour = 4_102_444_800_000; // far enough ahead
    a.send(ToServer::SetNotify { conv: g.id, notify: Notify::Mentions, muted_until: Some(in_an_hour) }).await;
    let on_phone =
        conv_of(a2.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.notify == Notify::Mentions)).await);
    assert_eq!(on_phone.muted_until, Some(in_an_hour));
    // A mute that has run out no longer shows.
    a.send(ToServer::SetNotify { conv: g.id, notify: Notify::All, muted_until: Some(1_000) }).await;
    let now = conv_of(a2.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.notify == Notify::All)).await);
    assert_eq!(now.muted_until, None);
}

#[tokio::test]
async fn finding_people() {
    let Some(db) = fresh_db("people").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    register(&addr, &s, "morgan").await;
    register(&addr, &s, "moira").await;
    let c = s.pool().get().await.unwrap();
    c.execute("UPDATE users SET display_name = 'Sam Morrow' WHERE username = 'moira'", &[]).await.unwrap();
    let mut a = App::connect(&addr, &ana.token).await;
    a.send(ToServer::FindPeople { q: "Mor".into() }).await;
    let ToApp::People { users, .. } = a.wait(|m| matches!(m, ToApp::People { .. })).await else { unreachable!() };
    let names: Vec<&str> = users.iter().map(|u| u.name()).collect();
    assert_eq!(names, ["Sam Morrow", "morgan"], "by user name, or any word of the display name");
    a.send(ToServer::FindPeople { q: "an".into() }).await;
    let ToApp::People { users, .. } = a.wait(|m| matches!(m, ToApp::People { .. })).await else { unreachable!() };
    assert!(users.is_empty(), "not yourself");
    // A display name given when the account is made, and changed later.
    let cy = register_as(&addr, &s, "cy", "  Cy   Young ").await;
    assert_eq!(cy.user.display_name, "Cy Young");
    a.send(ToServer::SetName { display_name: "Ana Bell".into() }).await;
    let ToApp::Welcome { user, .. } = a.wait(|m| matches!(m, ToApp::Welcome { .. })).await else { unreachable!() };
    assert_eq!(user.display_name, "Ana Bell");
    a.send(ToServer::FindPeople { q: "cy".into() }).await;
    let ToApp::People { users, .. } = a.wait(|m| matches!(m, ToApp::People { .. })).await else { unreachable!() };
    assert_eq!(users[0].name(), "Cy Young");
}

#[tokio::test]
async fn two_servers_share_the_conversation() {
    let Some(db) = fresh_db("twoservers").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (one, s1) = serve(&db).await;
    let (two, _s2) = serve(&db).await;
    let ana = register(&one, &s1, "ana").await;
    let bo = register(&one, &s1, "bo").await;
    let mut a = App::connect(&one, &ana.token).await;
    let mut b = App::connect(&two, &bo.token).await;
    a.send(ToServer::CreateGroup { title: "Team".into(), members: vec!["bo".into()], purpose: String::new() }).await;
    let ToApp::Conversation { conv } = b.wait(|m| matches!(m, ToApp::Conversation { .. })).await else {
        unreachable!()
    };
    a.send(ToServer::Send { conv: conv.id, client_id: "x".into(), text: "Across servers".into() }).await;
    let ToApp::Message { msg } = b.wait(|m| matches!(m, ToApp::Message { .. })).await else { unreachable!() };
    assert_eq!(msg.text, "Across servers");
    b.send(ToServer::Typing { conv: conv.id }).await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Typing { .. })).await, ToApp::Typing { user, .. } if user.username == "bo")
    );
}

#[tokio::test]
async fn people_see_who_is_around() {
    let Some(db) = fresh_db("presence").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let cy = register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { .. })).await;

    let is = |user: i64, state: Presence| move |m: &ToApp| matches!(m, ToApp::Presence { list } if list.iter().any(|p| p.user == user && p.state == state));
    // Bo connects: Ana hears, and Bo is told Ana is around.
    let mut b = App::connect(&addr, &bo.token).await;
    b.wait(is(ana.user.id, Presence::Online)).await;
    a.wait(is(bo.user.id, Presence::Online)).await;
    b.send(ToServer::SetStatus { status: Presence::Busy }).await;
    a.wait(is(bo.user.id, Presence::Busy)).await;
    drop(b);
    a.wait(is(bo.user.id, Presence::Offline)).await;
    // Someone you don't chat with: not told.
    a.send(ToServer::GetPresence { users: vec![cy.user.id, bo.user.id] }).await;
    let ToApp::Presence { list } = a.wait(|m| matches!(m, ToApp::Presence { .. })).await else { unreachable!() };
    assert_eq!(list.iter().map(|p| p.user).collect::<Vec<_>>(), [bo.user.id]);
}

#[tokio::test]
async fn people_make_apps_that_post_stream_and_answer_by_webhook() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Some(db) = fresh_db("apps").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;

    // A tiny bot: records what it's sent and answers.
    let hook = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_url = format!("http://127.0.0.1:{}/hook", hook.local_addr().unwrap().port());
    let (got_tx, mut got) = tokio::sync::mpsc::unbounded_channel::<(String, String, String)>();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = hook.accept().await.unwrap();
            let mut buf = vec![0; 65536];
            let mut n = 0;
            // Read the headers and the whole body.
            let req = loop {
                n += sock.read(&mut buf[n..]).await.unwrap();
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        break (head.to_string(), body.to_string());
                    }
                }
            };
            let header = |name: &str| {
                req.0
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix(name).map(|_| l[name.len()..].trim().to_string()))
                    .unwrap_or_default()
            };
            let reply = r#"{"text":"You get 25 days a year."}"#;
            let _ = got_tx.send((header("x-xenify-timestamp:"), header("x-xenify-signature:"), req.1.clone()));
            let _ = sock
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}",
                        reply.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });

    // Ana makes a personal app with a webhook: its key and secret, once.
    a.send(ToServer::CreateApp {
        name: "helper".into(),
        display_name: "Policy helper".into(),
        description: "Answers questions about policies".into(),
        webhook: hook_url.clone(),
        mode: AppMode::Personal,
    })
    .await;
    let ToApp::AppKey { app, key, secret } = a.wait(|m| matches!(m, ToApp::AppKey { .. })).await else {
        unreachable!()
    };
    assert_eq!((app.user.username.as_str(), app.owner, app.user.app), ("helper", ana.user.id, true));
    // Only its owner manages it; apps can't make apps.
    b.send(ToServer::NewAppKey { app: app.user.id }).await;
    b.wait(is_error("not_owner")).await;
    let mut as_app = App::connect(&addr, &key).await;
    as_app.send(ToServer::ListApps).await;
    as_app
        .send(ToServer::CreateApp {
            name: "x2".into(),
            display_name: String::new(),
            description: String::new(),
            webhook: String::new(),
            mode: AppMode::Personal,
        })
        .await;
    as_app.wait(is_error("not_allowed")).await;

    // Bo talks to it 1:1: the webhook gets the message, signed; the answer is posted.
    b.send(ToServer::OpenDirect { user: "helper".into() }).await;
    let dm = conv_of(b.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    b.send(ToServer::Send { conv: dm.id, client_id: "q".into(), text: "How much leave do I get?".into() }).await;
    let (at, signature, body) =
        tokio::time::timeout(std::time::Duration::from_secs(10), got.recv()).await.unwrap().unwrap();
    assert_eq!(signature, format!("sha256={}", xenify_chat_server::sign(&secret, &format!("{at}.{body}"))));
    let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (sent["kind"].as_str(), sent["message"]["text"].as_str()),
        (Some("direct"), Some("How much leave do I get?"))
    );
    b.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "You get 25 days a year." && msg.sender.app)).await;

    // In a channel, a personal app doesn't listen; posting and streaming work with its key.
    a.send(ToServer::CreateChannel { title: "Build alerts".into(), open: true, purpose: String::new() }).await;
    let ch = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.kind == Kind::Channel)).await);
    let post = serde_json::json!({ "channel": "build alerts", "text": "Build 12 passed" });
    assert_eq!(post_as(&addr, "/v1/post", &key, &post).await.0, 404, "not in it yet");
    a.send(ToServer::AddPeople { conv: ch.id, users: vec!["helper".into()] }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == ch.id && conv.member_count == 2)).await;
    let (status, out) = post_as(&addr, "/v1/post", &key, &post).await;
    assert_eq!(status, 200, "{out}");
    let seq = serde_json::from_str::<serde_json::Value>(&out).unwrap()["seq"].as_i64().unwrap();
    let edit = serde_json::json!({ "conv": ch.id, "seq": seq, "text": "Build 12 passed: 42 tests" });
    assert_eq!(post_as(&addr, "/v1/edit", &key, &edit).await.0, 200);
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "Build 12 passed: 42 tests")).await;
    a.send(ToServer::Send { conv: ch.id, client_id: "m".into(), text: "@helper are you there?".into() }).await;
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "@helper are you there?")).await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(800), got.recv()).await.is_err(),
        "personal apps don't listen in channels"
    );
    a.send(ToServer::AppsIn { conv: ch.id }).await;
    let ToApp::Apps { list, .. } = a.wait(|m| matches!(m, ToApp::Apps { conv, .. } if *conv == ch.id)).await else {
        unreachable!()
    };
    assert_eq!((list[0].mode, list[0].webhook.as_str()), (AppMode::Personal, ""), "webhooks aren't shown to everyone");

    // A shared-only app can't be messaged 1:1.
    a.send(ToServer::UpdateApp {
        app: app.user.id,
        display_name: "Policy helper".into(),
        description: String::new(),
        webhook: hook_url,
        mode: AppMode::Shared,
    })
    .await;
    a.wait(|m| matches!(m, ToApp::Apps { list, .. } if list[0].mode == AppMode::Shared)).await;
    b.send(ToServer::OpenDirect { user: "helper".into() }).await;
    b.wait(is_error("not_allowed")).await;
    // …but answers when mentioned in a channel it's in.
    a.send(ToServer::Send { conv: ch.id, client_id: "m2".into(), text: "@helper how much leave?".into() }).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), got.recv()).await.unwrap().unwrap();

    // A new key: the old one stops working.
    a.send(ToServer::NewAppKey { app: app.user.id }).await;
    let ToApp::AppKey { key: fresh, .. } = a.wait(|m| matches!(m, ToApp::AppKey { .. })).await else { unreachable!() };
    assert_eq!(post_as(&addr, "/v1/post", &key, &post).await.0, 401);
    assert_eq!(post_as(&addr, "/v1/post", &fresh, &post).await.0, 200);

    // The same through the HTTP API, with Ana's own key for scripts.
    a.send(ToServer::NewApiKey).await;
    let ToApp::ApiKey { key: mine } = a.wait(|m| matches!(m, ToApp::ApiKey { .. })).await else { unreachable!() };
    let made = serde_json::json!({ "name": "alerts", "display_name": "Alerts", "mode": "shared" });
    let (status, out) = post_as(&addr, "/v1/apps", &mine, &made).await;
    assert_eq!(status, 200, "{out}");
    assert!(out.contains("\"key\""));
    let local = serde_json::json!({ "app": app.user.id, "display_name": "x", "description": "", "webhook": "https://localhost/x", "mode": "shared" });
    assert_eq!(post_as(&addr, "/v1/apps/update", &bo.token, &local).await.0, 403);
    // Deleted: it leaves its chats.
    let gone = serde_json::json!({ "app": app.user.id });
    assert_eq!(post_as(&addr, "/v1/apps/delete", &mine, &gone).await.0, 200);
    assert_eq!(post_as(&addr, "/v1/post", &fresh, &post).await.0, 401);
}

#[test]
fn webhooks_only_go_to_public_addresses() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use xenify_chat_server::check_webhook;
        for bad in [
            "http://example.org/x",
            "https://127.0.0.1/x",
            "https://10.0.0.5/x",
            "https://[::1]/x",
            "https://192.168.1.2/x",
            "ftp://x.org",
        ] {
            assert!(check_webhook(bad, false).await.is_err(), "{bad}");
        }
        assert_eq!(check_webhook("", false).await.unwrap(), "");
        assert!(check_webhook("http://127.0.0.1:9000/x", true).await.is_ok(), "allowed for tests");
    });
}

#[tokio::test]
async fn meetings_calendars_and_busy_times_stay_out_of_chats() {
    use xenify_chat_proto::{BUSY, CALENDAR, MEETING};
    let Some(db) = fresh_db("purposes").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;

    // One calendar each, just you: asking again gives the same one.
    a.send(ToServer::CreateGroup { title: "Calendar".into(), members: vec!["bo".into()], purpose: CALENDAR.into() })
        .await;
    let cal = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    assert_eq!((cal.purpose.as_str(), cal.member_count), (CALENDAR, 1));
    a.send(ToServer::CreateGroup { title: "Calendar".into(), members: vec![], purpose: CALENDAR.into() }).await;
    assert_eq!(conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await).id, cal.id);
    a.send(ToServer::AddPeople { conv: cal.id, users: vec!["bo".into()] }).await;
    a.wait(is_error("not_allowed")).await;

    // Busy times: one channel each; others find it by person.
    a.send(ToServer::CreateChannel { title: "Busy times".into(), open: false, purpose: BUSY.into() }).await;
    let busy = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.purpose == BUSY)).await);
    b.send(ToServer::BusyOf { users: vec![ana.user.id, bo.user.id] }).await;
    let ToApp::BusyOf { list } = b.wait(|m| matches!(m, ToApp::BusyOf { .. })).await else { unreachable!() };
    assert_eq!(list.len(), 1, "Bo has none");
    assert_eq!((list[0].user, list[0].conv, list[0].open, list[0].member), (ana.user.id, busy.id, false, false));
    // Closed: Bo can't follow it until Ana opens it to everyone.
    b.send(ToServer::Subscribe { conv: busy.id }).await;
    b.wait(is_error("not_open")).await;
    a.send(ToServer::SetOpen { conv: busy.id, open: true }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == busy.id && conv.open)).await;
    b.send(ToServer::Subscribe { conv: busy.id }).await;
    b.wait(is_conv(busy.id)).await;
    b.send(ToServer::SetOpen { conv: busy.id, open: false }).await;
    b.wait(is_error("not_owner")).await;

    // A meeting: only its organiser invites more people.
    a.send(ToServer::CreateGroup { title: "meeting-1".into(), members: vec!["bo".into()], purpose: MEETING.into() })
        .await;
    let meet = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.purpose == MEETING)).await);
    b.wait(is_conv(meet.id)).await;
    b.send(ToServer::AddPeople { conv: meet.id, users: vec!["cy".into()] }).await;
    b.wait(is_error("not_owner")).await;
    a.send(ToServer::AddPeople { conv: meet.id, users: vec!["cy".into()] }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == meet.id && conv.member_count == 3)).await;

    // None of them turn up in searches.
    a.send(ToServer::FindConversations { q: "".into() }).await;
    let ToApp::Found { conversations, .. } = a.wait(|m| matches!(m, ToApp::Found { .. })).await else { unreachable!() };
    assert!(conversations.is_empty(), "{conversations:?}");
    a.send(ToServer::CreateGroup { title: "Chat".into(), members: vec![], purpose: "secret".into() }).await;
    a.wait(is_error("bad_request")).await;
}

#[tokio::test]
async fn rooms_and_things_are_booked_once_and_away_is_told() {
    use xenify_chat_proto::{Away, Thing, User};
    let Some(db) = fresh_db("things").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    let room = Thing {
        id: 0,
        name: "Room 2".into(),
        kind: "room".into(),
        place: "2nd floor".into(),
        seats: 8,
        by_day: false,
        approver: None,
    };

    // Only admins add things.
    b.send(ToServer::SaveThing { thing: room.clone() }).await;
    b.wait(is_error("not_admin")).await;
    s.pool().get().await.unwrap().execute("UPDATE users SET admin = TRUE WHERE username = 'ana'", &[]).await.unwrap();
    a.send(ToServer::SaveThing { thing: room }).await;
    let book_thing = Thing {
        id: 0,
        name: "Rust in Action".into(),
        kind: "book".into(),
        place: "Library".into(),
        seats: 0,
        by_day: true,
        approver: Some(User { id: ana.user.id, username: "ana".into(), ..Default::default() }),
    };
    a.send(ToServer::SaveThing { thing: book_thing }).await;
    let ToApp::Things { list, admin } = a.wait(|m| matches!(m, ToApp::Things { list, .. } if list.len() == 2)).await
    else {
        unreachable!()
    };
    assert!(admin);
    let room_id = list.iter().find(|t| t.kind == "room").unwrap().id;
    let book_id = list.iter().find(|t| t.kind == "book").unwrap().id;

    // A free room says yes by itself; the same time again is taken.
    const H: i64 = 3_600_000;
    let t0 = 2_000_000_000_000;
    b.send(ToServer::Book { thing: room_id, start: t0, end: t0 + H, meeting: Some(7), tag: "r1".into() }).await;
    let ToApp::Booked { booking, .. } = b.wait(|m| matches!(m, ToApp::Booked { tag, .. } if tag == "r1")).await else {
        unreachable!()
    };
    assert_eq!((booking.status.as_str(), booking.meeting), ("yes", Some(7)));
    a.send(ToServer::Book { thing: room_id, start: t0 + H / 2, end: t0 + 2 * H, meeting: None, tag: "r2".into() })
        .await;
    a.wait(is_error("taken")).await;
    // Others see who booked it, not what for.
    a.send(ToServer::Bookings { things: vec![room_id], from: t0 - H, to: t0 + 3 * H }).await;
    let ToApp::Bookings { list } = a.wait(|m| matches!(m, ToApp::Bookings { .. })).await else { unreachable!() };
    assert_eq!((list.len(), list[0].user.username.as_str(), list[0].meeting), (1, "bo", None));

    // A book someone looks after: asked, then answered.
    b.send(ToServer::Book { thing: book_id, start: t0, end: t0 + 48 * H, meeting: None, tag: "b1".into() }).await;
    let ToApp::Booked { booking, .. } = b.wait(|m| matches!(m, ToApp::Booked { tag, .. } if tag == "b1")).await else {
        unreachable!()
    };
    assert_eq!(booking.status, "asked");
    a.wait(|m| matches!(m, ToApp::Booked { booking: x, .. } if x.id == booking.id)).await;
    b.send(ToServer::AnswerBooking { booking: booking.id, yes: true }).await;
    b.wait(is_error("not_allowed")).await;
    a.send(ToServer::AnswerBooking { booking: booking.id, yes: true }).await;
    b.wait(|m| matches!(m, ToApp::Booked { booking: x, .. } if x.id == booking.id && x.status == "yes")).await;
    a.send(ToServer::MyBookings).await;
    let ToApp::Bookings { list } = a.wait(|m| matches!(m, ToApp::Bookings { .. })).await else { unreachable!() };
    assert!(list.is_empty(), "nothing waiting for Ana now: {list:?}");
    b.send(ToServer::CancelBooking { booking: booking.id }).await;
    b.wait(|m| matches!(m, ToApp::Booked { booking: x, .. } if x.status == "cancelled")).await;

    // Out of office: others are told, past times drop off.
    let away = Away {
        user: 0,
        times: vec![(1, 2), (t0, t0 + 24 * H)],
        text: "On holiday".into(),
        notice: true,
        ..Away::default()
    };
    a.send(ToServer::SetAway { away }).await;
    a.wait(|m| matches!(m, ToApp::AwayOf { .. })).await;
    b.send(ToServer::AwayOf { users: vec![ana.user.id, bo.user.id] }).await;
    let ToApp::AwayOf { list } = b.wait(|m| matches!(m, ToApp::AwayOf { .. })).await else { unreachable!() };
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].user, list[0].times.clone(), list[0].text.as_str()),
        (ana.user.id, vec![(t0, t0 + 24 * H)], "On holiday")
    );
}

#[tokio::test]
async fn search_finds_words_and_their_starts_in_what_you_may_read() {
    let Some(db) = fresh_db("search").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    let d = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::OpenDirect { user: "cy".into() }).await;
    let private = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id != d.id)).await);
    let texts = [
        (d.id, "The quarterly budget is due on Friday"),
        (d.id, "Budget meeting moved; the budget itself is fine"),
        (d.id, "Lunch?"),
        (private.id, "Secret budget plans for Cy"),
    ];
    for (i, (conv, text)) in texts.iter().enumerate() {
        a.send(ToServer::Send { conv: *conv, client_id: format!("s{i}"), text: text.to_string() }).await;
    }
    b.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "Lunch?")).await;

    // The start of a word finds it; Bo doesn't find what's in Ana's chat with Cy.
    b.send(ToServer::Search {
        q: "budg".into(),
        conv: None,
        sort: Default::default(),
        after: String::new(),
        tag: "t1".into(),
    })
    .await;
    let ToApp::Hits { hits, tag, .. } = b.wait(|m| matches!(m, ToApp::Hits { .. })).await else { unreachable!() };
    assert_eq!(tag, "t1");
    assert_eq!(hits.len(), 2);
    assert!(hits[0].msg.text.starts_with("Budget meeting"), "the one saying it twice first");
    assert!(hits[0].snippet.contains("\u{1}Budget\u{2}"), "{}", hits[0].snippet);
    // Every word must be there.
    b.send(ToServer::Search {
        q: "budget friday".into(),
        conv: None,
        sort: Default::default(),
        after: String::new(),
        tag: String::new(),
    })
    .await;
    let ToApp::Hits { hits, .. } = b.wait(|m| matches!(m, ToApp::Hits { .. })).await else { unreachable!() };
    assert_eq!(hits.len(), 1);
    // Ana finds all three; in one chat, just that chat's.
    a.send(ToServer::Search {
        q: "budget".into(),
        conv: None,
        sort: Default::default(),
        after: String::new(),
        tag: String::new(),
    })
    .await;
    let ToApp::Hits { hits, .. } = a.wait(|m| matches!(m, ToApp::Hits { .. })).await else { unreachable!() };
    assert_eq!(hits.len(), 3);
    a.send(ToServer::Search {
        q: "budget".into(),
        conv: Some(private.id),
        sort: Default::default(),
        after: String::new(),
        tag: String::new(),
    })
    .await;
    let ToApp::Hits { hits, .. } = a.wait(|m| matches!(m, ToApp::Hits { .. })).await else { unreachable!() };
    assert_eq!(hits.len(), 1);
    // Nothing to look for: nothing found (and no error).
    a.send(ToServer::Search {
        q: " ?! ".into(),
        conv: None,
        sort: Default::default(),
        after: String::new(),
        tag: String::new(),
    })
    .await;
    assert!(matches!(a.wait(|m| matches!(m, ToApp::Hits { .. })).await, ToApp::Hits { hits, .. } if hits.is_empty()));

    // Addresses, links and file names are found by any of their parts;
    // a file by its name, never by the details the app keeps with it.
    let more = [
        "Write to sam.lee@example.org about it",
        "Notes at https://wiki.acme.net/launch-plan",
        r#"~file:{"id":"f1","name":"report_final-v2.pdf","size":1200,"mime":"application/pdf"}"#,
        r#"~board:{"node":7,"title":"Sprint planning"}"#,
    ];
    for (i, text) in more.iter().enumerate() {
        a.send(ToServer::Send { conv: d.id, client_id: format!("m{i}"), text: text.to_string() }).await;
    }
    b.wait(|m| matches!(m, ToApp::Message { msg } if msg.text.starts_with("~board:"))).await;
    for (q, want) in [
        ("sam.lee@example.org", Some("Write to")),
        ("example", Some("Write to")),
        ("launch-plan", Some("Notes at")),
        ("final", Some("~file:")),
        ("report_final", Some("~file:")),
        ("v2.pdf", Some("~file:")),
        ("mime", None),
        ("application", None),
        ("sprint", Some("~board:")),
        ("node", None),
    ] {
        b.send(ToServer::Search {
            q: q.into(),
            conv: Some(d.id),
            sort: Default::default(),
            after: String::new(),
            tag: q.into(),
        })
        .await;
        let ToApp::Hits { hits, .. } = b.wait(|m| matches!(m, ToApp::Hits { tag, .. } if tag == q)).await else {
            unreachable!()
        };
        let found: Vec<&str> = hits.iter().map(|h| h.msg.text.as_str()).collect();
        match want {
            Some(start) => assert!(found.len() == 1 && found[0].starts_with(start), "{q}: {found:?}"),
            None => assert!(found.is_empty(), "{q}: {found:?}"),
        }
        if q == "final" {
            assert!(
                hits[0].snippet.contains("report_final-v2.pdf") && !hits[0].snippet.contains("mime"),
                "{}",
                hits[0].snippet
            );
        }
    }
    // One letter is a word of its own, not the start of every word.
    b.send(ToServer::Search {
        q: "t".into(),
        conv: None,
        sort: Default::default(),
        after: String::new(),
        tag: "t".into(),
    })
    .await;
    let ToApp::Hits { hits, .. } = b.wait(|m| matches!(m, ToApp::Hits { tag, .. } if tag == "t")).await else {
        unreachable!()
    };
    assert!(hits.is_empty(), "{:?}", hits.iter().map(|h| &h.msg.text).collect::<Vec<_>>());
}

#[tokio::test]
async fn settings_follow_you_and_reminders_show_everywhere_once() {
    let Some(db) = fresh_db("settings").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut a2 = App::connect(&addr, &ana.token).await;
    a.send(ToServer::SaveSettings { settings: serde_json::json!({ "sound": false, "digest": "mentions" }) }).await;
    let ToApp::Settings { settings } = a2.wait(|m| matches!(m, ToApp::Settings { .. })).await else { unreachable!() };
    assert_eq!(settings["sound"], false);
    // A device connecting later gets them with its welcome.
    let mut req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!("ws://{addr}/v1/ws"))
            .unwrap();
    req.headers_mut().insert("Authorization", format!("Bearer {}", ana.token).parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let mut a3 = App { ws };
    let ToApp::Welcome { settings, keep_days, .. } = a3.next().await else { panic!("welcome first") };
    assert_eq!((settings["digest"].as_str(), keep_days), (Some("mentions"), None));
    a.send(ToServer::SaveSettings { settings: serde_json::json!([1]) }).await;
    a.wait(is_error("bad_request")).await;

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let soon = xenify_chat_proto::Reminder {
        key: "ev:1".into(),
        at: now - 1000,
        title: "Call the bank".into(),
        starts: now + 60_000,
        ..Default::default()
    };
    let later = xenify_chat_proto::Reminder {
        key: "ev:2".into(),
        at: now + 3_600_000,
        title: "Later".into(),
        ..Default::default()
    };
    a.send(ToServer::SetReminders { list: vec![soon.clone(), later.clone()] }).await;
    // Due: on both devices.
    for app in [&mut a, &mut a2] {
        let ToApp::Reminder { reminder } = app.wait(|m| matches!(m, ToApp::Reminder { .. })).await else {
            unreachable!()
        };
        assert_eq!(reminder.title, "Call the bank");
    }
    assert!(s.due_reminders().await.unwrap().is_empty(), "shown once");
    // Snoozed on one device: gone from the other until it's due again.
    a.send(ToServer::Snooze { key: "ev:1".into(), until: now + 500 }).await;
    assert!(
        matches!(a2.wait(|m| matches!(m, ToApp::ReminderGone { .. })).await, ToApp::ReminderGone { key } if key == "ev:1")
    );
    // Sending the same list again keeps the snooze.
    a.send(ToServer::SetReminders { list: vec![soon.clone(), later.clone()] }).await;
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let due = s.due_reminders().await.unwrap();
    assert_eq!(due.len(), 1);
    a2.wait(|m| matches!(m, ToApp::Reminder { reminder } if reminder.key == "ev:1")).await;
    // A device connecting now shows it too, until it's dismissed.
    let mut a4 = App::connect(&addr, &ana.token).await;
    a4.wait(|m| matches!(m, ToApp::Reminder { reminder } if reminder.key == "ev:1")).await;
    // Dismissed: sending the list again doesn't bring it back.
    a.send(ToServer::Dismiss { key: "ev:1".into() }).await;
    a2.wait(|m| matches!(m, ToApp::ReminderGone { .. })).await;
    let mut a5 = App::connect(&addr, &ana.token).await;
    a5.send(ToServer::Ping).await;
    assert!(matches!(a5.wait(|m| matches!(m, ToApp::Pong | ToApp::Reminder { .. })).await, ToApp::Pong));
    a.send(ToServer::SetReminders { list: vec![soon, later] }).await;
    a.send(ToServer::Ping).await;
    a.wait(|m| matches!(m, ToApp::Pong)).await;
    assert!(s.due_reminders().await.unwrap().is_empty());
}

#[tokio::test]
async fn old_messages_go_after_the_server_or_chat_limit() {
    let Some(db) = fresh_db("keep").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::CreateGroup { title: "Short".into(), members: vec!["bo".into()], purpose: String::new() }).await;
    let g = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    let d = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id != g.id)).await);
    a.send(ToServer::CreateGroup { title: String::new(), members: vec![], purpose: "calendar".into() }).await;
    let cal = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.purpose == "calendar")).await);
    for (i, conv) in [g.id, g.id, d.id, cal.id].iter().enumerate() {
        a.send(ToServer::Send { conv: *conv, client_id: format!("k{i}"), text: format!("message {i}") }).await;
    }
    a.wait(|m| matches!(m, ToApp::Message { msg } if msg.text == "message 3")).await;
    // Make them old: 40 days, apart from the group's second.
    let c = s.pool().get().await.unwrap();
    c.execute("UPDATE messages SET sent_at = now() - interval '40 days' WHERE body <> 'message 1'", &[]).await.unwrap();

    // Only owners choose, and only for groups and channels.
    b.send(ToServer::SetKeep { conv: g.id, days: Some(30) }).await;
    b.wait(is_error("not_owner")).await;
    a.send(ToServer::SetKeep { conv: d.id, days: Some(30) }).await;
    a.wait(is_error("not_group")).await;
    // Kept for ever: nothing goes.
    assert!(s.expire().await.unwrap().is_empty());
    a.send(ToServer::SetKeep { conv: g.id, days: Some(30) }).await;
    assert_eq!(
        conv_of(b.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.keep_days.is_some())).await).keep_days,
        Some(30)
    );
    let gone = s.expire().await.unwrap();
    assert_eq!(gone.iter().map(|g| g.0).collect::<Vec<_>>(), [g.id]);
    // The server's own limit covers every chat, but not the calendar.
    s.set_keep_days(Some(90));
    assert!(s.expire().await.unwrap().is_empty());
    s.set_keep_days(Some(35));
    assert_eq!(s.expire().await.unwrap().iter().map(|g| g.0).collect::<Vec<_>>(), [d.id]);
    let left: Vec<String> =
        c.query("SELECT body FROM messages ORDER BY id", &[]).await.unwrap().iter().map(|r| r.get(0)).collect();
    assert_eq!(left, ["message 1", "message 3"]);
}

#[tokio::test]
async fn owners_share_the_job_rename_and_take_down_messages() {
    let Some(db) = fresh_db("owners").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::CreateChannel { title: "News".into(), open: true, purpose: String::new() }).await;
    let ch = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "News")).await);
    a.send(ToServer::AddPeople { conv: ch.id, users: vec!["bo".into(), "cy".into()] }).await;
    b.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == ch.id)).await;

    // Bo can't rename or post yet; once a co-owner, Bo can do both.
    b.send(ToServer::Rename { conv: ch.id, title: "Bo's news".into() }).await;
    assert!(
        matches!(b.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_owner")
    );
    a.send(ToServer::SetRole { conv: ch.id, user: "bo".into(), owner: true }).await;
    let v = conv_of(b.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == ch.id && conv.can_post)).await);
    assert_eq!(v.owners.len(), 2);
    b.send(ToServer::Rename { conv: ch.id, title: "  Team news ".into() }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Team news")).await;
    b.send(ToServer::Send { conv: ch.id, client_id: "b1".into(), text: "Office closed Friday".into() }).await;
    let ToApp::Message { msg } = a.wait(|m| matches!(m, ToApp::Message { msg } if msg.client_id == "b1")).await else {
        unreachable!()
    };
    // An owner takes down someone else's post; nobody edits it but its sender.
    a.send(ToServer::Edit { conv: ch.id, seq: msg.seq, text: "changed".into() }).await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_yours")
    );
    a.send(ToServer::Delete { conv: ch.id, seq: msg.seq }).await;
    b.wait(|m| matches!(m, ToApp::Message { msg } if msg.client_id == "b1" && msg.deleted)).await;

    // In a group anyone renames it, but only owners take down others' messages.
    a.send(ToServer::CreateGroup { title: "Lunch".into(), members: vec!["bo".into()], purpose: String::new() }).await;
    let g = conv_of(b.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Lunch")).await);
    b.send(ToServer::Rename { conv: g.id, title: "Lunch club".into() }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.title == "Lunch club")).await;
    a.send(ToServer::Send { conv: g.id, client_id: "a1".into(), text: "Pizza?".into() }).await;
    let ToApp::Message { msg } = b.wait(|m| matches!(m, ToApp::Message { msg } if msg.client_id == "a1")).await else {
        unreachable!()
    };
    b.send(ToServer::Delete { conv: g.id, seq: msg.seq }).await;
    assert!(
        matches!(b.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_yours")
    );
    // Without the server's roles, nobody moderates: Bo can't take Ana out.
    b.send(ToServer::RemovePerson { conv: g.id, user: "ana".into() }).await;
    assert!(
        matches!(b.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_owner")
    );
    // Direct chats have no owners and no new names.
    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    let d = conv_of(
        a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.kind == xenify_chat_proto::Kind::Direct)).await,
    );
    a.send(ToServer::Rename { conv: d.id, title: "x".into() }).await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_group")
    );
}

#[tokio::test]
async fn replies_go_in_threads_and_tell_whoever_is_in_them() {
    let Some(db) = fresh_db("threads").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let cy = register(&addr, &s, "cy").await;
    let dee = register(&addr, &s, "dee").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    let mut d = App::connect(&addr, &dee.token).await;
    a.send(ToServer::CreateGroup {
        title: "Launch".into(),
        members: vec!["bo".into(), "cy".into()],
        purpose: String::new(),
    })
    .await;
    let g = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::Send { conv: g.id, client_id: "r".into(), text: "Which day for the launch?".into() }).await;
    let ToApp::Message { msg: root } = b.wait(|m| matches!(m, ToApp::Message { .. })).await else { unreachable!() };

    // Bo answers in its thread: everyone gets the reply (for the counts),
    // with who's in the thread, and the first message with its count.
    b.send(ToServer::Reply { conv: g.id, thread: root.seq, client_id: "t1".into(), text: "Tuesday".into() }).await;
    let ToApp::Message { msg: reply } = a.wait(|m| matches!(m, ToApp::Message { msg } if msg.thread.is_some())).await
    else {
        unreachable!()
    };
    assert_eq!(reply.thread, Some(root.seq));
    let mut people = reply.thread_people.clone();
    people.sort();
    assert_eq!(people, [ana.user.id, bo.user.id]);
    let ToApp::Message { msg: counted } =
        a.wait(|m| matches!(m, ToApp::Message { msg } if msg.id == root.id && msg.replies == 1)).await
    else {
        unreachable!()
    };
    assert_eq!(counted.last_reply_seq, reply.seq);

    // The chat itself leaves replies out; the thread has both.
    a.send(ToServer::History { conv: g.id, before: None, limit: 50, tag: String::new() }).await;
    let ToApp::History { messages, thread, .. } = a.wait(|m| matches!(m, ToApp::History { .. })).await else {
        unreachable!()
    };
    assert_eq!(
        (messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), thread),
        (vec!["Which day for the launch?"], None)
    );
    a.send(ToServer::Thread { conv: g.id, thread: root.seq, tag: "th".into() }).await;
    let ToApp::History { messages, thread, .. } =
        a.wait(|m| matches!(m, ToApp::History { tag, .. } if tag == "th")).await
    else {
        unreachable!()
    };
    assert_eq!(thread, Some(root.seq));
    assert_eq!(messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Which day for the launch?", "Tuesday"]);
    // The reply counts as unread for Ana until she reads it.
    a.send(ToServer::List).await;
    let ToApp::Conversations { list } = a.wait(|m| matches!(m, ToApp::Conversations { .. })).await else {
        unreachable!()
    };
    assert!(list.iter().find(|c| c.id == g.id).unwrap().unread());

    // No threads in threads, and none for people outside the chat.
    a.send(ToServer::Reply { conv: g.id, thread: reply.seq, client_id: "t2".into(), text: "Nested".into() }).await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "no_thread")
    );
    d.send(ToServer::Thread { conv: g.id, thread: root.seq, tag: String::new() }).await;
    assert!(
        matches!(d.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_member")
    );
    d.send(ToServer::Reply { conv: g.id, thread: root.seq, client_id: "t3".into(), text: "Hi".into() }).await;
    assert!(
        matches!(d.wait(|m| matches!(m, ToApp::Error { .. })).await, ToApp::Error { code, .. } if code == "not_member")
    );
    drop(cy);
}

#[tokio::test]
async fn search_pages_through_matches_in_the_order_asked() {
    use xenify_chat_proto::{SEARCH_PAGE, SearchSort};
    let Some(db) = fresh_db("search_pages").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let cy = register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    let ab = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::OpenDirect { user: "cy".into() }).await;
    let ac = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id != ab.id)).await);
    // Lots of matches, one a minute (some at the same moment), straight
    // into the database: more than are ranked all together, in two chats.
    let c = s.pool().get().await.unwrap();
    let n = 1300i64;
    c.execute(
        "INSERT INTO messages (conv, seq, sender, body, sent_at)
         SELECT $1, g, $2, 'report number ' || g || CASE WHEN g % 7 = 0 THEN ' report again' ELSE '' END,
                timestamptz '2026-01-01' + ((g / 2) * interval '1 minute')
         FROM generate_series(1, $3::bigint) g",
        &[&ab.id, &ana.user.id, &n],
    )
    .await
    .unwrap();
    c.execute("UPDATE conversations SET next_seq = $2 WHERE id = $1", &[&ab.id, &n]).await.unwrap();
    c.execute(
        "INSERT INTO messages (conv, seq, sender, body) VALUES ($1, 1, $2, 'secret report for Cy')",
        &[&ac.id, &ana.user.id],
    )
    .await
    .unwrap();
    drop(c);

    // Bo pages newest first: each page in order, none twice, never Cy's.
    for (sort, newest_first) in [(SearchSort::Newest, true), (SearchSort::Oldest, false)] {
        let mut after = String::new();
        let mut seen: Vec<(i64, i64)> = Vec::new();
        for page in 0..3 {
            let tag = format!("{sort:?}{page}");
            b.send(ToServer::Search { q: "report".into(), conv: None, sort, after: after.clone(), tag: tag.clone() })
                .await;
            let ToApp::Hits { hits, total, more, .. } =
                b.wait(|m| matches!(m, ToApp::Hits { tag: t, .. } if *t == tag)).await
            else {
                unreachable!()
            };
            assert_eq!(total, 1001, "counted up to 1,001");
            assert_eq!(hits.len(), SEARCH_PAGE);
            assert!(!more.is_empty());
            seen.extend(hits.iter().map(|h| (h.msg.at, h.msg.id)));
            assert!(hits.iter().all(|h| h.msg.conv == ab.id));
            after = more;
        }
        let mut sorted = seen.clone();
        sorted.sort_by(|x, y| if newest_first { y.cmp(x) } else { x.cmp(y) });
        assert_eq!(seen, sorted, "{sort:?}: in order");
        sorted.dedup();
        assert_eq!(sorted.len(), 3 * SEARCH_PAGE, "{sort:?}: none twice");
        let ids: Vec<i64> = seen.iter().map(|(_, id)| *id).collect();
        let first = if newest_first { n } else { 1 };
        b.send(ToServer::History { conv: ab.id, before: Some(first + 1), limit: 1, tag: "h".into() }).await;
        let ToApp::History { messages, .. } = b.wait(|m| matches!(m, ToApp::History { tag, .. } if tag == "h")).await
        else {
            unreachable!()
        };
        assert_eq!(ids[0], messages[0].id, "{sort:?}: starts at the very first");
    }
    // Best first: those saying it twice, a page at a time.
    b.send(ToServer::Search {
        q: "report".into(),
        conv: None,
        sort: SearchSort::Best,
        after: String::new(),
        tag: "best".into(),
    })
    .await;
    let ToApp::Hits { hits, more, .. } = b.wait(|m| matches!(m, ToApp::Hits { tag, .. } if tag == "best")).await else {
        unreachable!()
    };
    assert!(
        hits.iter().all(|h| h.msg.text.contains("again")),
        "{:?}",
        hits.iter().map(|h| &h.msg.text).collect::<Vec<_>>()
    );
    assert_eq!(more, format!("o:{SEARCH_PAGE}"));
    b.send(ToServer::Search {
        q: "report".into(),
        conv: None,
        sort: SearchSort::Best,
        after: more,
        tag: "best2".into(),
    })
    .await;
    let ToApp::Hits { hits: next, .. } = b.wait(|m| matches!(m, ToApp::Hits { tag, .. } if tag == "best2")).await
    else {
        unreachable!()
    };
    assert_eq!(next.len(), SEARCH_PAGE);
    assert!(next.iter().all(|x| hits.iter().all(|h| h.msg.id != x.msg.id)));
    // A few matches: all on one page, nothing more.
    a.send(ToServer::Search {
        q: "secret".into(),
        conv: None,
        sort: SearchSort::Newest,
        after: String::new(),
        tag: "few".into(),
    })
    .await;
    let ToApp::Hits { hits, more, total, .. } = a.wait(|m| matches!(m, ToApp::Hits { tag, .. } if tag == "few")).await
    else {
        unreachable!()
    };
    assert_eq!((hits.len(), more.as_str(), total), (1, "", 1));
    drop(cy);
}

/// A pretend AI that keeps what it was given.
struct FakeAi(std::sync::Mutex<Vec<String>>);

impl xenify_chat_server::Assistant for FakeAi {
    fn ready(&self, _: i64) -> futures_util::future::BoxFuture<'_, bool> {
        Box::pin(async { true })
    }
    fn answer(
        &self,
        _: i64,
        messages: Vec<serde_json::Value>,
    ) -> futures_util::future::BoxFuture<'static, Result<String, String>> {
        let given =
            messages.iter().map(|m| m["content"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("\n");
        self.0.lock().unwrap().push(given.clone());
        Box::pin(async move {
            Ok(if given.contains("Question:") { "Priya said Tuesday [1].".into() } else { "launch day tuesday".into() })
        })
    }
}

#[tokio::test]
async fn the_ai_answers_from_your_messages_only() {
    let Some(db) = fresh_db("ask").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    register(&addr, &s, "cy").await;
    let mut a = App::connect(&addr, &ana.token).await;
    // No AI yet: Ask says so.
    a.send(ToServer::Ask { q: "When is the launch?".into(), conv: None, tag: "q0".into() }).await;
    assert!(
        matches!(a.wait(|m| matches!(m, ToApp::Answer { .. })).await, ToApp::Answer { error, .. } if !error.is_empty())
    );
    let ai = std::sync::Arc::new(FakeAi(Default::default()));
    s.set_assistant(ai.clone());
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::OpenDirect { user: "bo".into() }).await;
    let ab = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::OpenDirect { user: "cy".into() }).await;
    let ac = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id != ab.id)).await);
    a.send(ToServer::Send { conv: ab.id, client_id: "1".into(), text: "The launch day is Tuesday".into() }).await;
    a.send(ToServer::Send { conv: ac.id, client_id: "2".into(), text: "Secret: the launch moves to Friday".into() })
        .await;
    b.wait(|m| matches!(m, ToApp::Message { .. })).await;

    b.send(ToServer::Ask { q: "When is the launch?".into(), conv: None, tag: "q1".into() }).await;
    let ToApp::Answer { text, sources, error, tag, .. } = b.wait(|m| matches!(m, ToApp::Answer { .. })).await else {
        unreachable!()
    };
    assert_eq!((error.as_str(), tag.as_str(), text.as_str()), ("", "q1", "Priya said Tuesday [1]."));
    assert_eq!(sources.iter().map(|h| h.msg.text.as_str()).collect::<Vec<_>>(), ["The launch day is Tuesday"]);
    // The AI never saw Ana's chat with Cy.
    let given = ai.0.lock().unwrap().join("\n");
    assert!(given.contains("The launch day is Tuesday") && !given.contains("Friday"), "{given}");
}

#[tokio::test]
async fn apps_post_in_channels_only_when_an_owner_added_them() {
    let Some(db) = fresh_db("appchan").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::CreateChannel { title: "Announcements".into(), open: true, purpose: String::new() }).await;
    let ch = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);

    // Bo's bot can't follow Ana's channel and post there.
    b.send(ToServer::CreateApp {
        name: "itdept".into(),
        display_name: "IT Department".into(),
        description: String::new(),
        webhook: String::new(),
        mode: AppMode::Shared,
    })
    .await;
    let ToApp::AppKey { key, .. } = b.wait(|m| matches!(m, ToApp::AppKey { .. })).await else { unreachable!() };
    let mut bot = App::connect(&addr, &key).await;
    bot.send(ToServer::Subscribe { conv: ch.id }).await;
    bot.wait(is_error("not_allowed")).await;
    // Even if it got in on its own (an older server let it), it can't post.
    let c = s.pool().get().await.unwrap();
    let bot_id: i64 = c.query_one("SELECT id FROM users WHERE username = 'itdept'", &[]).await.unwrap().get(0);
    xenify_chat_server::db::add_member(&c, ch.id, bot_id, None).await.unwrap();
    bot.send(ToServer::Send { conv: ch.id, client_id: "1".into(), text: "Reset your password here".into() }).await;
    bot.wait(is_error("not_allowed")).await;
    let post = serde_json::json!({ "conv": ch.id, "text": "Reset your password here" });
    assert_eq!(post_as(&addr, "/v1/post", &key, &post).await.0, 403);

    // Added by an owner, it posts.
    c.execute("DELETE FROM members WHERE user_id = $1", &[&bot_id]).await.unwrap();
    a.send(ToServer::AddPeople { conv: ch.id, users: vec!["itdept".into()] }).await;
    a.wait(|m| matches!(m, ToApp::Conversation { conv } if conv.id == ch.id && conv.member_count == 2)).await;
    assert_eq!(post_as(&addr, "/v1/post", &key, &post).await.0, 200);

    // When its owner is disabled, its key stops working.
    c.execute("UPDATE users SET disabled = TRUE WHERE id = $1", &[&bo.user.id]).await.unwrap();
    assert_eq!(post_as(&addr, "/v1/post", &key, &post).await.0, 401);
}

#[tokio::test]
async fn idle_sessions_stop_and_signed_out_sockets_close() {
    let Some(db) = fresh_db("sessions").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let c = s.pool().get().await.unwrap();
    // An app's key keeps working however long it sat unused; a person's doesn't.
    let mut a = App::connect(&addr, &ana.token).await;
    a.send(ToServer::CreateApp {
        name: "alerts".into(),
        display_name: String::new(),
        description: String::new(),
        webhook: String::new(),
        mode: AppMode::Personal,
    })
    .await;
    let ToApp::AppKey { key, .. } = a.wait(|m| matches!(m, ToApp::AppKey { .. })).await else { unreachable!() };
    a.send(ToServer::NewApiKey).await;
    let ToApp::ApiKey { key: script } = a.wait(|m| matches!(m, ToApp::ApiKey { .. })).await else { unreachable!() };
    let devices: Vec<String> = c
        .query("SELECT device FROM sessions ORDER BY created", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(devices, ["test", "app", "api"]);
    c.execute("UPDATE sessions SET last_seen = now() - interval '100 days'", &[]).await.unwrap();
    let list = serde_json::json!({});
    assert_eq!(post_as(&addr, "/v1/apps/update", &script, &list).await.0, 401, "idle key for scripts");
    let post = serde_json::json!({ "channel": "nowhere", "text": "hi" });
    assert_eq!(post_as(&addr, "/v1/post", &key, &post).await.0, 404, "app keys don't go idle");

    // Signed out on another server: the open socket closes.
    c.execute("UPDATE sessions SET last_seen = now()", &[]).await.unwrap();
    let mut b = App::connect(&addr, &ana.token).await;
    c.execute("DELETE FROM sessions WHERE device = 'test'", &[]).await.unwrap();
    s.sessions_changed(ana.user.id).await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        use futures_util::StreamExt;
        loop {
            match b.ws.next().await {
                None | Some(Err(_)) | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the socket closes");
}

#[tokio::test]
async fn people_taken_out_leave_the_call() {
    use xenify_chat_proto::CallAction;
    let Some(db) = fresh_db("calls").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let bo = register(&addr, &s, "bo").await;
    let mut a = App::connect(&addr, &ana.token).await;
    let mut b = App::connect(&addr, &bo.token).await;
    a.send(ToServer::CreateGroup { title: "Team".into(), members: vec!["bo".into()], purpose: String::new() }).await;
    let g = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::Call { conv: g.id, action: CallAction::Start, video: false }).await;
    b.wait(|m| matches!(m, ToApp::Call { .. })).await;
    b.send(ToServer::Call { conv: g.id, action: CallAction::Join, video: false }).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.people.len() == 2)).await;
    a.send(ToServer::RemovePerson { conv: g.id, user: "bo".into() }).await;
    a.wait(|m| matches!(m, ToApp::Call { call } if call.people.iter().all(|u| u.id != bo.user.id))).await;
}

#[tokio::test]
async fn call_media_takes_the_key_in_the_subprotocol_not_the_address() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let Some(db) = fresh_db("media").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let ana = register(&addr, &s, "ana").await;
    let mut a = App::connect(&addr, &ana.token).await;
    a.send(ToServer::CreateGroup { title: "Me".into(), members: vec![], purpose: String::new() }).await;
    let g = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::Call { conv: g.id, action: xenify_chat_proto::CallAction::Start, video: false }).await;
    let ToApp::Call { call } = a.wait(|m| matches!(m, ToApp::Call { .. })).await else { unreachable!() };
    let old = format!("ws://{addr}/v1/call/{}/media?token={}", call.id, ana.token);
    assert!(tokio_tungstenite::connect_async(old).await.is_err(), "not in the address");
    let mut req = format!("ws://{addr}/v1/call/{}/media", call.id).into_client_request().unwrap();
    req.headers_mut().insert("Sec-WebSocket-Protocol", format!("xenify-chat, {}", ana.token).parse().unwrap());
    let (_ws, resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    assert_eq!(resp.headers().get("sec-websocket-protocol").unwrap(), "xenify-chat");
}

#[test]
fn pinned_webhooks_go_only_where_they_were_checked() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use xenify_chat_server::pin_webhook;
        assert!(pin_webhook("https://127.0.0.1/x", false).await.is_err());
        assert!(pin_webhook("https://[::ffff:10.0.0.1]/x", false).await.is_err());
        assert!(pin_webhook("https://[64:ff9b::a00:1]/x", false).await.is_err(), "NAT64 of 10.0.0.1");
        let p = pin_webhook("http://127.0.0.1:9/x", true).await.unwrap();
        assert_eq!(p.addrs, ["127.0.0.1:9".parse().unwrap()]);
        // Another host than the one pinned isn't looked up.
        let agent = p.agent(std::time::Duration::from_secs(2));
        assert!(tokio::task::spawn_blocking(move || agent.get("http://localhost:9/x").call()).await.unwrap().is_err());
    });
}
