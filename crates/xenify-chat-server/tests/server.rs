//! The server against a real PostgreSQL. Each test gets a fresh database.
//! `XENIFY_TEST_PG` is an admin address (default: a local test cluster on
//! port 55432); without a reachable PostgreSQL the tests say so and pass.

mod common;

use common::*;
use xenify_chat_proto::{AppMode, Kind, Notify, Presence, Role, Session, ToApp, ToServer};

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
async fn accounts_need_invites_and_right_passwords() {
    let Some(db) = fresh_db("accounts").await else { return eprintln!("no PostgreSQL to test with: skipped") };
    let (addr, s) = serve(&db).await;
    let (st, _) = post(&addr, "/v1/register", &login("ana", "correct horse battery", "")).await;
    assert_eq!(st, 403, "invite needed");
    let code = xenify_chat_server::invite(s.pool(), 1).await.unwrap();
    assert_eq!(post(&addr, "/v1/register", &login("ana", "short", &code)).await.0, 400, "a real password");
    assert_eq!(post(&addr, "/v1/register", &login("ana", "correct horse battery", &code)).await.0, 200);
    assert_eq!(
        post(&addr, "/v1/register", &login("bo", "correct horse battery", &code)).await.0,
        403,
        "codes work once"
    );
    assert_eq!(post(&addr, "/v1/login", &login("ana", "wrong password!!", "")).await.0, 401);
    let (st, body) = post(&addr, "/v1/login", &login("Ana", "correct horse battery", "")).await;
    assert_eq!(st, 200);
    let session: Session = serde_json::from_str(&body).unwrap();
    assert_eq!(session.user.username, "ana");
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

    a.send(ToServer::CreateGroup { title: "Design team".into(), members: vec!["bo".into(), "nobody".into()] }).await;
    a.wait(is_error("no_user")).await;
    a.send(ToServer::CreateGroup { title: "Design team".into(), members: vec!["bo".into()] }).await;
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

    a.send(ToServer::CreateChannel { title: "Build alerts".into(), open: true }).await;
    let ch = conv_of(a.wait(|m| matches!(m, ToApp::Conversation { .. })).await);
    a.send(ToServer::CreateChannel { title: "Board notes".into(), open: false }).await;
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
    a.send(ToServer::CreateGroup { title: "Weekly planning".into(), members: vec!["bo".into()] }).await;
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
    let code = xenify_chat_server::invite(s.pool(), 1).await.unwrap();
    let mut l = login("cy", "correct horse battery", &code);
    l.display_name = "  Cy   Young ".into();
    let (status, body) = post(&addr, "/v1/register", &l).await;
    assert_eq!(status, 200, "{body}");
    let cy: xenify_chat_proto::Session = serde_json::from_str(&body).unwrap();
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
    a.send(ToServer::CreateGroup { title: "Team".into(), members: vec!["bo".into()] }).await;
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
    a.send(ToServer::CreateChannel { title: "Build alerts".into(), open: true }).await;
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
