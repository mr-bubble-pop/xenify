//! Chat's jobs on a timer:
//!
//! - every hour, messages older than the server's or the chat's limit go,
//!   with the files sent in those chats then (recordings are chat files);
//! - reminders that are due are shown on people's devices and sent to
//!   their browsers (Web Push);
//! - new messages go to the browsers of people with no Chat open anywhere;
//! - messages still unread after ten minutes (direct chats and mentions, or
//!   everything) are emailed, when the server can send email.
//!
//! Each follows the person's mutes, Do not disturb and quiet hours. With
//! several servers on one database, a lock keeps each job to one of them.

use crate::{Server, db, mail, push};
use deadpool_postgres::{Client, GenericClient};
use serde_json::{Value, json};

/// How long a message waits unread before it's emailed.
const MISSED_AFTER: &str = "10 minutes";

pub(crate) fn start(s: Server) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            if *s.stopping().borrow() {
                break;
            }
            n += 1;
            refresh_limit(&s).await;
            reminders(&s).await;
            if n % 4 == 1 {
                messages(&s).await;
                digests(&s).await;
            }
            // Every hour, the first a minute after starting.
            if n % 240 == 4 {
                let _ = expire(&s).await;
            }
        }
    });
}

/// How many days the server keeps messages (Admin › Network › Chat; None:
/// for ever).
pub(crate) async fn keep_days(c: &impl GenericClient) -> Option<i64> {
    db::setting(c, "chat_keep_days").await.ok().flatten().and_then(|v| v.parse().ok()).filter(|d: &i64| *d > 0)
}

async fn refresh_limit(s: &Server) {
    if let Ok(c) = s.pool.get().await {
        s.chat.set_keep_days(keep_days(&c).await);
    }
}

/// Runs `f` while holding lock `key` (skipped when another server has it).
async fn alone<F: std::future::Future<Output = ()>>(c: &Client, key: i64, f: F) {
    let Ok(r) = c.query_one("SELECT pg_try_advisory_lock($1)", &[&key]).await else { return };
    if !r.get::<_, bool>(0) {
        return;
    }
    f.await;
    let _ = c.execute("SELECT pg_advisory_unlock($1)", &[&key]).await;
}

/// Deletes old messages, and the files sent in those chats before the
/// same time. Returns how many files went.
pub async fn expire(s: &Server) -> Result<usize, String> {
    refresh_limit(s).await;
    let gone = s.chat.expire().await?;
    let c = s.pool.get().await.map_err(|e| e.to_string())?;
    let mut files = 0;
    for (conv, before) in gone {
        let rows = c
            .query(
                "DELETE FROM chat_files WHERE conv = $1 AND created < to_timestamp($2::bigint / 1000.0) RETURNING id",
                &[&conv, &before],
            )
            .await
            .map_err(|e| e.to_string())?;
        for r in rows {
            let id: String = r.get(0);
            let _ = tokio::fs::remove_dir_all(s.secure.join("chat-files").join(&id)).await;
            files += 1;
        }
    }
    Ok(files)
}

async fn reminders(s: &Server) {
    let Ok(due) = s.chat.due_reminders().await else { return };
    if due.is_empty() {
        return;
    }
    let Ok(c) = s.pool.get().await else { return };
    for (user, r) in due {
        let act = push::token_for(&c, user, &r.key).await;
        let payload = json!({
            "kind": "reminder",
            "title": if r.starts > 0 { r.title.clone() } else { "Reminder".to_string() },
            "body": if r.starts > 0 { r.body.clone() } else { r.title.clone() },
            "key": r.key,
            "starts": r.starts,
            "link": r.link,
            "act": act,
        });
        push::send(s, user, payload).await;
    }
}

/// What the server reads of someone's Chat settings.
struct Prefs {
    desktop: bool,
    preview: bool,
    mentions: bool,
    /// "off", "mentions" (and direct chats) or "all".
    digest: String,
    quiet: String,
    tz: chrono_tz::Tz,
}

impl Prefs {
    fn of(v: &Value) -> Prefs {
        let flag = |k: &str, or: bool| v.get(k).and_then(Value::as_bool).unwrap_or(or);
        let text = |k: &str, or: &str| v.get(k).and_then(Value::as_str).unwrap_or(or).to_string();
        Prefs {
            desktop: flag("desktop", true),
            preview: flag("preview", true),
            mentions: flag("mentions", true),
            digest: text("digest", "mentions"),
            quiet: text("quiet", "Off"),
            tz: text("tz", "UTC").parse().unwrap_or(chrono_tz::UTC),
        }
    }

    /// Quiet hours, as the app has them (Settings › Notifications).
    fn quiet_now(&self) -> bool {
        use chrono::{Datelike, Timelike};
        let now = chrono::Utc::now().with_timezone(&self.tz);
        let h = now.hour();
        let weekend = matches!(now.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
        let night = !(8..18).contains(&h);
        match self.quiet.as_str() {
            "Evenings and weekends" => weekend || night,
            "18:00 to 08:00" => night,
            "Weekends only" => weekend,
            _ => false,
        }
    }
}

/// A message waiting for someone, with what's needed to decide whether to
/// tell them.
struct Waiting {
    conv: i64,
    seq: i64,
    kind: String,
    title: String,
    sender: String,
    text: String,
    notify: String,
    muted: bool,
    /// Not a reply, or a reply in a thread this person is in (wrote its
    /// first message or replied); otherwise only a mention tells them.
    in_thread: bool,
}

impl Waiting {
    fn of(r: &tokio_postgres::Row) -> Waiting {
        Waiting {
            conv: r.get("conv"),
            seq: r.get("seq"),
            kind: r.get("kind"),
            title: r.get("title"),
            sender: r.get("sender"),
            text: r.get("body"),
            notify: r.get("notify"),
            muted: r.get("muted"),
            in_thread: r.get("in_thread"),
        }
    }

    /// "@" and one of their names (the name others see, or the one
    /// underneath).
    fn mentions(&self, names: &[&str]) -> bool {
        let text = self.text.to_lowercase();
        names.iter().filter(|n| !n.trim().is_empty()).any(|n| text.contains(&format!("@{}", n.trim().to_lowercase())))
    }

    /// Whether someone with `p` wants to hear of it (`all`: everything they
    /// get notifications for, else direct chats and mentions).
    fn wanted(&self, p: &Prefs, names: &[&str], all: bool) -> bool {
        let mention = self.mentions(names);
        if !self.in_thread && !mention {
            return false;
        }
        let chat_wants = !self.muted && (self.notify == "all" || (self.notify == "mentions" && mention));
        if !chat_wants && !(mention && p.mentions) {
            return false;
        }
        all || self.kind == "direct" || mention
    }

    fn line(&self, preview: bool) -> String {
        let place = if self.kind == "direct" { String::new() } else { format!(" in {}", self.place()) };
        let text: String = self.text.chars().take(200).collect();
        if preview { format!("{}{place}: {text}", self.sender) } else { format!("{}{place}", self.sender) }
    }

    fn place(&self) -> String {
        if self.title.is_empty() { "a group".into() } else { self.title.clone() }
    }
}

const WAITING_COLUMNS: &str =
    "m.conv, m.seq, cv.kind, cv.title, COALESCE(NULLIF(u.display_name, ''), u.username) AS sender,
    m.body, me.notify, COALESCE(me.muted_until > now(), FALSE) AS muted,
    (m.thread IS NULL OR EXISTS (SELECT 1 FROM messages t WHERE t.conv = m.conv AND t.sender = me.user_id
        AND (t.seq = m.thread OR t.thread = m.thread) AND t.seq < m.seq)) AS in_thread";

/// New messages to the browsers of people with no Chat open: each server
/// takes the next ones in turn (the last one sent is a setting).
async fn messages(s: &Server) {
    let Ok(c) = s.pool.get().await else { return };
    alone(&c, 7420427, async {
        let last: i64 = match db::setting(&c, "chat_pushed").await.ok().flatten().and_then(|v| v.parse().ok()) {
            Some(l) => l,
            None => {
                // The first time: from now on.
                let top: i64 =
                    c.query_one("SELECT COALESCE(max(id), 0) FROM messages", &[]).await.map(|r| r.get(0)).unwrap_or(0);
                let _ = db::set_setting(&c, "chat_pushed", &top.to_string()).await;
                return;
            }
        };
        let Ok(top) =
            c.query_one("SELECT COALESCE(max(id), $1) FROM messages", &[&last]).await.map(|r| r.get::<_, i64>(0))
        else {
            return;
        };
        if top <= last {
            return;
        }
        let _ = db::set_setting(&c, "chat_pushed", &top.to_string()).await;
        let Ok(rows) = c
            .query(
                &format!(
                    "SELECT me.user_id, r.username, r.display_name AS my_name, r.status, COALESCE(cs.value, '{{}}') AS prefs, {WAITING_COLUMNS}
                     FROM messages m JOIN conversations cv ON cv.id = m.conv AND cv.purpose = ''
                     JOIN users u ON u.id = m.sender
                     JOIN members me ON me.conv = m.conv AND me.user_id <> m.sender
                     JOIN users r ON r.id = me.user_id
                     LEFT JOIN chat_settings cs ON cs.user_id = me.user_id
                     WHERE m.id > $1 AND m.id <= $2 AND NOT m.deleted AND me.read_seq < m.seq
                       AND EXISTS (SELECT 1 FROM push_subs p WHERE p.user_id = me.user_id)
                     ORDER BY m.id"
                ),
                &[&last, &top],
            )
            .await
        else {
            return;
        };
        for r in &rows {
            let user: i64 = r.get("user_id");
            let p = Prefs::of(&r.get::<_, Value>("prefs"));
            let w = Waiting::of(r);
            if !p.desktop
                || r.get::<_, &str>("status") == "busy"
                || p.quiet_now()
                || !w.wanted(&p, &[r.get("username"), r.get("my_name")], true)
            {
                continue;
            }
            if s.chat.connected(user).await {
                continue;
            }
            let title = if w.kind == "direct" { w.sender.clone() } else { format!("{} · {}", w.sender, w.place()) };
            let body = if p.preview { w.text.chars().take(300).collect() } else { "New message".to_string() };
            push::send(s, user, json!({ "kind": "message", "title": title, "body": body, "conv": w.conv })).await;
        }
    })
    .await;
}

/// Emails people what they've missed: messages unread for ten minutes they
/// haven't been emailed about, in one email each.
pub async fn digests(s: &Server) {
    let Ok(c) = s.pool.get().await else { return };
    if mail::mail(&c).await.is_none() {
        return;
    }
    alone(&c, 7420428, async {
        let Ok(rows) = c
            .query(
                &format!(
                    "SELECT me.user_id, r.username, r.display_name AS my_name, r.email, r.status, COALESCE(cs.value, '{{}}') AS prefs, {WAITING_COLUMNS}
                     FROM members me JOIN users r ON r.id = me.user_id AND r.email <> '' AND NOT r.disabled AND NOT r.app
                     JOIN conversations cv ON cv.id = me.conv AND cv.purpose = ''
                     JOIN messages m ON m.conv = me.conv AND m.seq > GREATEST(me.read_seq, me.mailed_seq)
                     JOIN users u ON u.id = m.sender
                     LEFT JOIN chat_settings cs ON cs.user_id = me.user_id
                     WHERE cv.next_seq > GREATEST(me.read_seq, me.mailed_seq) AND m.sender <> me.user_id AND NOT m.deleted
                       AND m.sent_at < now() - interval '{MISSED_AFTER}'
                       AND COALESCE(cs.value ->> 'digest', 'mentions') <> 'off'
                     ORDER BY me.user_id, m.sent_at LIMIT 5000"
                ),
                &[],
            )
            .await
        else {
            return;
        };
        let mut i = 0;
        while i < rows.len() {
            let user: i64 = rows[i].get("user_id");
            let mut j = i;
            while j < rows.len() && rows[j].get::<_, i64>("user_id") == user {
                j += 1;
            }
            digest(s, &c, &rows[i..j]).await;
            i = j;
        }
    })
    .await;
}

/// One person's email of missed messages (`rows` are theirs).
async fn digest(s: &Server, c: &Client, rows: &[tokio_postgres::Row]) {
    let first = &rows[0];
    let user: i64 = first.get("user_id");
    let names: [&str; 2] = [first.get("username"), first.get("my_name")];
    let p = Prefs::of(&first.get::<_, Value>("prefs"));
    // Do not disturb and quiet hours: later, when they're over.
    if first.get::<_, &str>("status") == "busy" || p.quiet_now() {
        return;
    }
    let all = p.digest == "all";
    let waiting: Vec<Waiting> = rows.iter().map(Waiting::of).collect();
    let wanted: Vec<&Waiting> = waiting.iter().filter(|w| w.wanted(&p, &names, all)).collect();
    // Whatever happens, these have been looked at.
    let mut upto: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for w in &waiting {
        let e = upto.entry(w.conv).or_default();
        *e = (*e).max(w.seq);
    }
    if !wanted.is_empty() {
        let server = db::setting(c, "name").await.ok().flatten().unwrap_or_else(|| "Xenify server".into());
        let address = crate::server_address(c).await;
        let chats: std::collections::BTreeSet<i64> = wanted.iter().map(|w| w.conv).collect();
        let mut lines: Vec<String> = wanted.iter().take(15).map(|w| w.line(p.preview)).collect();
        if wanted.len() > 15 {
            lines.push(format!("…and {} more.", wanted.len() - 15));
        }
        let n = wanted.len();
        let e = mail::Email {
            subject: if n == 1 {
                format!("A message you missed on {server}")
            } else {
                format!("{n} messages you missed on {server}")
            },
            heading: if n == 1 { "You have a message".into() } else { format!("You have {n} messages") },
            lines,
            button: (!address.is_empty() && s.hosted_apps().is_some())
                .then(|| ("Open Chat".to_string(), format!("{}/chat/", address.trim_end_matches('/')))),
            small: vec![
                if chats.len() == 1 { "From one chat.".into() } else { format!("From {} chats.", chats.len()) },
                "Change these emails in Chat, Settings › Notifications.".into(),
            ],
        };
        let to: String = first.get("email");
        if mail::send(c, &to, e).await.is_err() {
            return;
        }
    }
    for (conv, seq) in upto {
        let _ = c
            .execute(
                "UPDATE members SET mailed_seq = GREATEST(mailed_seq, $3) WHERE conv = $1 AND user_id = $2",
                &[&conv, &user, &seq],
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(kind: &str, text: &str, notify: &str, muted: bool) -> Waiting {
        Waiting {
            conv: 1,
            seq: 1,
            kind: kind.into(),
            title: "Team".into(),
            sender: "Bo".into(),
            text: text.into(),
            notify: notify.into(),
            muted,
            in_thread: true,
        }
    }

    #[test]
    fn digests_follow_mutes_and_choices() {
        let p = Prefs::of(&json!({}));
        assert_eq!(p.digest, "mentions");
        assert!(waiting("direct", "hi", "all", false).wanted(&p, &["ana"], false));
        assert!(!waiting("group", "hi", "all", false).wanted(&p, &["ana"], false));
        assert!(waiting("group", "hi", "all", false).wanted(&p, &["ana"], true));
        assert!(waiting("group", "hi @Ana", "all", false).wanted(&p, &["ana"], false));
        // Muted: only mentions, and only when they always want those.
        assert!(!waiting("direct", "hi", "all", true).wanted(&p, &["ana"], true));
        assert!(waiting("group", "hi @ana", "off", false).wanted(&p, &["ana"], false));
        let p = Prefs::of(&json!({ "mentions": false }));
        assert!(!waiting("group", "hi @ana", "off", false).wanted(&p, &["ana"], false));
        assert_eq!(waiting("group", "hi", "all", false).line(false), "Bo in Team");
        // A reply in a thread you're not in: only when it mentions you.
        let p = Prefs::of(&json!({}));
        let outside = |text: &str| Waiting { in_thread: false, ..waiting("group", text, "all", false) };
        assert!(!outside("Tuesday works").wanted(&p, &["ana"], true));
        assert!(outside("Tuesday works, @ana?").wanted(&p, &["ana"], true));
    }
}
