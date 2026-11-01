//! Calls: who's in which call, and passing their sound and pictures along.
//!
//! The apps encode (WebCodecs: VP8 pictures, Opus sound) each frame
//! themselves; this server only forwards the frames to everyone else in the
//! call, each prefixed with the sender's user id. Calls live in this server's memory (a server that
//! restarts ends its calls; the apps call again).
//!
//! Recording works the same way: the app of whoever records keeps the
//! sound and pictures it already gets, of the people who turned something
//! back on after everyone's was turned off. This server keeps track of who
//! that is, and passes it on.

use crate::{Server, db, err, hash_token};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use xenify_chat_proto::{CallAction, CallInfo, CallRecording, Kind, ToApp, User};

/// The largest frame passed along (a key frame of a shared screen).
pub const FRAME: usize = 2 << 20;

pub struct Call {
    pub info: CallInfo,
    /// Media connections: connection number → (user, where to send, check
    /// the session still works).
    media: HashMap<u64, (i64, mpsc::Sender<Vec<u8>>, Arc<tokio::sync::Notify>)>,
    /// Recordings made in it so far.
    rounds: i64,
}

#[derive(Default)]
pub struct Calls {
    by_conv: HashMap<i64, Call>,
    next: u64,
}

use xenify_util::now_ms;

impl Server {
    /// Tells everyone in the conversation (who's connected here) about the
    /// call.
    async fn tell_call(&self, info: &CallInfo) {
        let Ok(c) = self.pool.get().await else { return };
        let members = db::member_ids(&c, info.conv).await.unwrap_or_default();
        self.tell(&members, ToApp::Call { call: info.clone() });
    }

    /// The calls you can see on connecting (so a call already going rings
    /// or shows "Join").
    pub(crate) async fn calls_for(&self, user: i64) -> Vec<CallInfo> {
        let convs: Vec<i64> = self.calls.lock().unwrap().by_conv.keys().copied().collect();
        let Ok(c) = self.pool.get().await else { return Vec::new() };
        let mut out = Vec::new();
        for conv in convs {
            if db::role_of(&c, conv, user).await.ok().flatten().is_some()
                && let Some(call) = self.calls.lock().unwrap().by_conv.get(&conv)
            {
                out.push(call.info.clone());
            }
        }
        out
    }

    pub(crate) async fn call(&self, me: &User, conv: i64, action: CallAction, video: bool) -> Option<ToApp> {
        let Ok(c) = self.pool.get().await else { return Some(crate::failed("database unavailable")) };
        if db::role_of(&c, conv, me.id).await.ok().flatten().is_none() {
            return Some(crate::not_member());
        }
        // Between two people, one hanging up or saying no ends it.
        let direct = c
            .query_opt("SELECT kind FROM conversations WHERE id = $1", &[&conv])
            .await
            .ok()
            .flatten()
            .is_some_and(|r| db::kind_of(r.get(0)) == Kind::Direct);
        // Admins can turn recording off (Admin › Network › Chat).
        let can_record = c
            .query_opt("SELECT value FROM settings WHERE key = 'call_recording'", &[])
            .await
            .ok()
            .flatten()
            .is_none_or(|r| r.get::<_, String>(0) != "off");
        drop(c);
        let info = {
            let mut calls = self.calls.lock().unwrap();
            match action {
                CallAction::Start | CallAction::Join => {
                    let call = calls.by_conv.entry(conv).or_insert_with(|| Call {
                        info: CallInfo {
                            conv,
                            id: crate::random(12),
                            by: me.clone(),
                            people: Vec::new(),
                            video,
                            at: now_ms(),
                            declined: false,
                            recording: None,
                            can_record,
                        },
                        media: HashMap::new(),
                        rounds: 0,
                    });
                    if !call.info.people.iter().any(|u| u.id == me.id) {
                        call.info.people.push(me.clone());
                    }
                    call.info.clone()
                }
                CallAction::Leave | CallAction::Decline => {
                    if !direct && action == CallAction::Decline {
                        // A group call rings on: only this person said no.
                        return None;
                    }
                    leave(&mut calls, conv, me.id, direct, action == CallAction::Decline)?
                }
                CallAction::Record | CallAction::StopRecording | CallAction::RecordingOff | CallAction::InRecording => {
                    let Some(call) =
                        calls.by_conv.get_mut(&conv).filter(|c| c.info.people.iter().any(|u| u.id == me.id))
                    else {
                        return Some(crate::error("not_in_call", "you're not in that call"));
                    };
                    call.info.can_record = can_record;
                    let r = &mut call.info.recording;
                    match action {
                        CallAction::Record if !can_record => {
                            return Some(crate::error(
                                "no_recording",
                                "this server's admins have turned recording off",
                            ));
                        }
                        CallAction::Record if r.is_some() => {
                            return Some(crate::error("recording", "someone is recording this call already"));
                        }
                        CallAction::Record => {
                            call.rounds += 1;
                            *r = Some(CallRecording {
                                by: me.clone(),
                                round: call.rounds,
                                since: now_ms(),
                                off: Vec::new(),
                                in_it: Vec::new(),
                            });
                        }
                        CallAction::StopRecording if r.as_ref().is_some_and(|r| r.by.id == me.id) => *r = None,
                        CallAction::RecordingOff => {
                            if let Some(r) = r.as_mut().filter(|r| !r.off.contains(&me.id)) {
                                r.off.push(me.id);
                            }
                        }
                        CallAction::InRecording => {
                            if let Some(r) = r.as_mut().filter(|r| !r.in_it.contains(&me.id)) {
                                r.in_it.push(me.id);
                            }
                        }
                        _ => return None,
                    }
                    call.info.clone()
                }
            }
        };
        self.tell_call(&info).await;
        None
    }

    /// Someone is no longer in `conv`: they're out of its call (if it's
    /// here), and their media connections to it close.
    pub(crate) async fn out_of_call(&self, user: i64, conv: i64) {
        let info = {
            let mut calls = self.calls.lock().unwrap();
            if !calls.by_conv.get(&conv).is_some_and(|c| c.info.people.iter().any(|u| u.id == user)
                || c.media.values().any(|(u, _, _)| *u == user))
            {
                return;
            }
            leave(&mut calls, conv, user, false, false)
        };
        if let Some(info) = info {
            self.tell_call(&info).await;
        }
    }

    /// Asks these people's media connections here to check their sessions.
    pub(crate) fn recheck_media(&self, users: &[i64]) {
        let calls = self.calls.lock().unwrap();
        for call in calls.by_conv.values() {
            for (u, _, check) in call.media.values() {
                if users.contains(u) {
                    check.notify_one();
                }
            }
        }
    }

    /// Someone's last connection went: they leave their calls.
    pub(crate) async fn gone(&self, user: &User) {
        let convs: Vec<i64> = self
            .calls
            .lock()
            .unwrap()
            .by_conv
            .iter()
            .filter(|(_, c)| c.info.people.iter().any(|u| u.id == user.id))
            .map(|(k, _)| *k)
            .collect();
        for conv in convs {
            self.call(user, conv, CallAction::Leave, false).await;
        }
    }
}

/// Takes `user` out of the call in `conv` (`direct`: it ends for both;
/// `declined`: they said no). Their media connections close. None: no call.
fn leave(calls: &mut Calls, conv: i64, user: i64, direct: bool, declined: bool) -> Option<CallInfo> {
    let call = calls.by_conv.get_mut(&conv)?;
    if direct {
        call.info.people.clear();
        call.info.declined = declined;
    } else {
        call.info.people.retain(|u| u.id != user);
        // Their media connections close with the call.
        call.media.retain(|_, (u, _, _)| *u != user);
        // The recording stops with the person making it.
        if call.info.recording.as_ref().is_some_and(|r| r.by.id == user) {
            call.info.recording = None;
        }
        if let Some(r) = call.info.recording.as_mut() {
            r.off.retain(|u| *u != user);
            r.in_it.retain(|u| *u != user);
        }
    }
    let info = call.info.clone();
    // Nobody left: it ends (its media connections with it).
    if info.people.is_empty() {
        calls.by_conv.remove(&conv);
    }
    Some(info)
}

/// `GET /v1/call/{id}/media`: the frames of a call you're in. Browsers
/// can't set headers on WebSockets: they sign in as on `/v1/ws`, offering
/// the subprotocols `["xenify-chat", <token>]` (never in the address, where
/// it would end up in logs).
pub async fn media(
    State(s): State<Server>,
    Path(id): Path<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let offered: Vec<String> = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').map(|p| p.trim().to_string()).collect())
        .unwrap_or_default();
    let from_browser = offered.first().map(String::as_str) == Some(crate::BROWSER_PROTOCOL);
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| from_browser.then(|| offered.get(1).cloned()).flatten())
        .unwrap_or_default();
    let hash = hash_token(&token);
    let Ok(c) = s.pool.get().await else { return err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable") };
    let Some((user, device)) = db::session(&c, &hash).await.ok().flatten() else {
        return err(StatusCode::UNAUTHORIZED, "sign in again");
    };
    drop(c);
    // Calls are for people in Chat, not apps or keys for scripts.
    if user.app || device == "api" {
        return err(StatusCode::FORBIDDEN, "calls are made from Chat");
    }
    let in_call = s
        .calls
        .lock()
        .unwrap()
        .by_conv
        .values()
        .any(|c| c.info.id == id && c.info.people.iter().any(|u| u.id == user.id));
    if !in_call {
        return err(StatusCode::NOT_FOUND, "that call isn't going, or you're not in it");
    }
    let upgrade = if from_browser { upgrade.protocols([crate::BROWSER_PROTOCOL]) } else { upgrade };
    upgrade.max_message_size(FRAME).on_upgrade(move |socket| relay(s, user, hash, id, socket))
}

async fn relay(s: Server, user: User, token_hash: String, id: String, socket: WebSocket) {
    let (mut out, mut inp) = socket.split();
    // A little room, then frames are dropped (better than falling behind).
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
    let check: Arc<tokio::sync::Notify> = Arc::default();
    let me = {
        let mut calls = s.calls.lock().unwrap();
        calls.next += 1;
        let n = calls.next;
        match calls.by_conv.values_mut().find(|c| c.info.id == id) {
            Some(call) => {
                call.media.insert(n, (user.id, tx, check.clone()));
                n
            }
            None => return,
        }
    };
    // Ends when this connection is taken out of the call (all its senders
    // gone), and then the connection closes.
    let mut writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if out.send(WsMessage::Binary(frame.into())).await.is_err() {
                break;
            }
        }
        let _ = out.send(WsMessage::Close(None)).await;
    });
    let prefix = user.id.to_be_bytes();
    let mut every = tokio::time::interval_at(tokio::time::Instant::now() + crate::RECHECK, crate::RECHECK);
    loop {
        let frame = tokio::select! {
            f = inp.next() => f,
            _ = &mut writer => break,
            _ = check.notified() => {
                if !crate::still_signed_in(&s, &token_hash).await {
                    break;
                }
                continue;
            }
            _ = every.tick() => {
                if !crate::still_signed_in(&s, &token_hash).await {
                    break;
                }
                continue;
            }
        };
        let Some(Ok(frame)) = frame else { break };
        let data = match frame {
            WsMessage::Binary(b) => b,
            WsMessage::Close(_) => break,
            _ => continue,
        };
        let mut stamped = Vec::with_capacity(8 + data.len());
        stamped.extend_from_slice(&prefix);
        stamped.extend_from_slice(&data);
        let targets: Vec<mpsc::Sender<Vec<u8>>> = {
            let calls = s.calls.lock().unwrap();
            match calls.by_conv.values().find(|c| c.info.id == id) {
                Some(call) if call.media.contains_key(&me) => {
                    // Everyone else: not back to this person's own connections.
                    call.media
                        .iter()
                        .filter(|(n, (u, _, _))| **n != me && *u != user.id)
                        .map(|(_, (_, t, _))| t.clone())
                        .collect()
                }
                // Left the call, or it ended.
                _ => break,
            }
        };
        for t in targets {
            let _ = t.try_send(stamped.clone());
        }
    }
    if let Some(call) = s.calls.lock().unwrap().by_conv.values_mut().find(|c| c.info.id == id) {
        call.media.remove(&me);
    }
    writer.abort();
}
