//! Rooms and things people book, and out of office. Everyone may see who
//! booked what and when, never what for, and the server stops two bookings
//! overlapping. Out of office is kept with your account so people are told
//! while your computer is off.

use crate::{Server, error, failed};
use deadpool_postgres::Client;
use tokio_postgres::Row;
use xenify_chat_proto::{Away, Booking, Thing, ToApp, User};

pub(crate) const SCHEMA: &str = "
ALTER TABLE users ADD COLUMN IF NOT EXISTS admin BOOLEAN NOT NULL DEFAULT FALSE;
-- Out of office (xenify_chat_proto::Away as JSON; empty: none).
ALTER TABLE users ADD COLUMN IF NOT EXISTS away TEXT NOT NULL DEFAULT '';
CREATE TABLE IF NOT EXISTS things (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    place TEXT NOT NULL DEFAULT '',
    seats BIGINT NOT NULL DEFAULT 0,
    by_day BOOLEAN NOT NULL DEFAULT FALSE,
    approver BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS bookings (
    id BIGSERIAL PRIMARY KEY,
    thing BIGINT NOT NULL REFERENCES things(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    start_at BIGINT NOT NULL,
    end_at BIGINT NOT NULL,
    status TEXT NOT NULL,
    meeting BIGINT,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS bookings_by_thing ON bookings (thing, start_at);
";

const KINDS: [&str; 5] = ["room", "equipment", "book", "vehicle", "other"];

fn thing_of(r: &Row) -> Thing {
    let approver: Option<i64> = r.get("approver");
    Thing {
        id: r.get("id"),
        name: r.get("name"),
        kind: r.get("kind"),
        place: r.get("place"),
        seats: r.get("seats"),
        by_day: r.get("by_day"),
        approver: approver.map(|id| User {
            id,
            username: r.get("a_username"),
            display_name: r.get("a_display"),
            ..Default::default()
        }),
    }
}

const THING_COLUMNS: &str = "t.id, t.name, t.kind, t.place, t.seats, t.by_day, t.approver,
    COALESCE(a.username, '') AS a_username, COALESCE(a.display_name, '') AS a_display";

fn booking_of(r: &Row, me: i64) -> Booking {
    let user =
        User { id: r.get("user_id"), username: r.get("username"), display_name: r.get("display_name"), ..Default::default() };
    Booking {
        id: r.get("id"),
        thing: r.get("thing"),
        start: r.get("start_at"),
        end: r.get("end_at"),
        status: r.get("status"),
        // What it's for is the booker's business.
        meeting: if user.id == me { r.get("meeting") } else { None },
        user,
    }
}

const BOOKING_COLUMNS: &str =
    "b.id, b.thing, b.user_id, b.start_at, b.end_at, b.status, b.meeting, u.username, u.display_name";

async fn is_admin(c: &Client, me: i64) -> bool {
    c.query_opt("SELECT admin FROM users WHERE id = $1", &[&me])
        .await
        .ok()
        .flatten()
        .is_some_and(|r| r.get::<_, bool>(0))
}

pub(crate) async fn things(c: &Client, me: i64) -> ToApp {
    match c
        .query(
            &format!("SELECT {THING_COLUMNS} FROM things t LEFT JOIN users a ON a.id = t.approver ORDER BY t.kind, lower(t.name)"),
            &[],
        )
        .await
    {
        Ok(rows) => ToApp::Things { list: rows.iter().map(thing_of).collect(), admin: is_admin(c, me).await },
        Err(e) => failed(e),
    }
}

pub(crate) async fn save_thing(c: &Client, me: i64, t: Thing) -> ToApp {
    if !is_admin(c, me).await {
        return error("not_admin", "only admins add rooms and things");
    }
    let name = t.name.trim();
    if name.is_empty() || name.len() > 200 {
        return error("bad_request", "give it a name");
    }
    if !KINDS.contains(&t.kind.as_str()) {
        return error("bad_request", "a room, equipment, a book, a vehicle or something else");
    }
    let approver = t.approver.as_ref().map(|u| u.id);
    let done = if t.id == 0 {
        c.execute(
            "INSERT INTO things (name, kind, place, seats, by_day, approver) VALUES ($1, $2, $3, $4, $5, $6)",
            &[&name, &t.kind, &t.place.trim(), &t.seats.max(0), &t.by_day, &approver],
        )
        .await
    } else {
        c.execute(
            "UPDATE things SET name = $2, kind = $3, place = $4, seats = $5, by_day = $6, approver = $7 WHERE id = $1",
            &[&t.id, &name, &t.kind, &t.place.trim(), &t.seats.max(0), &t.by_day, &approver],
        )
        .await
    };
    if let Err(e) = done {
        return failed(e);
    }
    things(c, me).await
}

pub(crate) async fn remove_thing(c: &Client, me: i64, id: i64) -> ToApp {
    if !is_admin(c, me).await {
        return error("not_admin", "only admins take rooms and things away");
    }
    if let Err(e) = c.execute("DELETE FROM things WHERE id = $1", &[&id]).await {
        return failed(e);
    }
    things(c, me).await
}

pub(crate) async fn bookings(c: &Client, me: i64, things: &[i64], from: i64, to: i64) -> ToApp {
    match c
        .query(
            &format!(
                "SELECT {BOOKING_COLUMNS} FROM bookings b JOIN users u ON u.id = b.user_id
                 WHERE b.thing = ANY($1) AND b.status IN ('yes', 'asked') AND b.start_at < $3 AND b.end_at > $2
                 ORDER BY b.start_at LIMIT 2000"
            ),
            &[&things, &from, &to],
        )
        .await
    {
        Ok(rows) => ToApp::Bookings { list: rows.iter().map(|r| booking_of(r, me)).collect() },
        Err(e) => failed(e),
    }
}

/// Your bookings still to come (and recent answers), and requests you answer.
pub(crate) async fn my_bookings(c: &Client, me: i64) -> ToApp {
    let now = now();
    match c
        .query(
            &format!(
                "SELECT {BOOKING_COLUMNS} FROM bookings b JOIN users u ON u.id = b.user_id JOIN things t ON t.id = b.thing
                 WHERE ((b.user_id = $1 AND b.status IN ('yes', 'asked', 'no')) OR (t.approver = $1 AND b.status = 'asked'))
                   AND b.end_at > $2
                 ORDER BY b.start_at LIMIT 500"
            ),
            &[&me, &(now - 86_400_000)],
        )
        .await
    {
        Ok(rows) => ToApp::Bookings { list: rows.iter().map(|r| booking_of(r, me)).collect() },
        Err(e) => failed(e),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

async fn booking(c: &Client, id: i64, me: i64) -> Option<(Booking, Option<i64>)> {
    let r = c
        .query_opt(
            &format!(
                "SELECT {BOOKING_COLUMNS}, t.approver FROM bookings b JOIN users u ON u.id = b.user_id JOIN things t ON t.id = b.thing WHERE b.id = $1"
            ),
            &[&id],
        )
        .await
        .ok()??;
    Some((booking_of(&r, me), r.get("approver")))
}

#[allow(clippy::too_many_arguments)] // The booking as it's asked for.
pub(crate) async fn book(
    s: &Server,
    c: &mut Client,
    me: i64,
    thing: i64,
    start: i64,
    end: i64,
    meeting: Option<i64>,
    tag: String,
) -> ToApp {
    if end <= start || end - start > 366 * 86_400_000 {
        return error("bad_request", "a booking ends after it starts, within a year");
    }
    let tx = match c.transaction().await {
        Ok(t) => t,
        Err(e) => return failed(e),
    };
    // One booking at a time per thing, whichever server takes it.
    let t = match tx.query_opt("SELECT name, approver FROM things WHERE id = $1 FOR UPDATE", &[&thing]).await {
        Ok(Some(r)) => r,
        Ok(None) => return error("no_thing", "that isn't there to book any more"),
        Err(e) => return failed(e),
    };
    let name: String = t.get(0);
    let approver: Option<i64> = t.get(1);
    let clash = tx
        .query_opt(
            "SELECT u.display_name, u.username FROM bookings b JOIN users u ON u.id = b.user_id
             WHERE b.thing = $1 AND b.status IN ('yes', 'asked') AND b.start_at < $3 AND b.end_at > $2 LIMIT 1",
            &[&thing, &start, &end],
        )
        .await;
    match clash {
        Ok(Some(r)) => {
            let who: String = r.get(0);
            let who = if who.is_empty() { r.get::<_, String>(1) } else { who };
            return error("taken", &format!("{name} is booked then (by {who})"));
        }
        Ok(None) => {}
        Err(e) => return failed(e),
    }
    let status = if approver.is_none_or(|a| a == me) { "yes" } else { "asked" };
    let id: i64 = match tx
        .query_one(
            "INSERT INTO bookings (thing, user_id, start_at, end_at, status, meeting) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
            &[&thing, &me, &start, &end, &status, &meeting],
        )
        .await
    {
        Ok(r) => r.get(0),
        Err(e) => return failed(e),
    };
    if let Err(e) = tx.commit().await {
        return failed(e);
    }
    s.notify(format!("b:{id}")).await;
    match booking(c, id, me).await {
        Some((booking, _)) => ToApp::Booked { booking, tag },
        None => failed("the booking vanished"),
    }
}

pub(crate) async fn answer(s: &Server, c: &Client, me: i64, id: i64, yes: bool) -> Option<ToApp> {
    let Some((b, approver)) = booking(c, id, me).await else {
        return Some(error("no_booking", "that booking isn't there"));
    };
    if approver != Some(me) {
        return Some(error("not_allowed", "only whoever looks after it answers"));
    }
    if b.status != "asked" {
        return Some(error("answered", "that's been answered already"));
    }
    let status = if yes { "yes" } else { "no" };
    if let Err(e) = c.execute("UPDATE bookings SET status = $2 WHERE id = $1", &[&id, &status]).await {
        return Some(failed(e));
    }
    s.notify(format!("b:{id}")).await;
    None
}

pub(crate) async fn cancel(s: &Server, c: &Client, me: i64, id: i64) -> Option<ToApp> {
    let Some((b, approver)) = booking(c, id, me).await else {
        return Some(error("no_booking", "that booking isn't there"));
    };
    if b.user.id != me && approver != Some(me) {
        return Some(error("not_allowed", "only whoever booked it can call it off"));
    }
    if let Err(e) = c.execute("UPDATE bookings SET status = 'cancelled' WHERE id = $1", &[&id]).await {
        return Some(failed(e));
    }
    s.notify(format!("b:{id}")).await;
    None
}

/// For `b:id`: the booker and whoever looks after the thing hear of it.
pub(crate) async fn changed(c: &Client, id: i64) -> Option<(Vec<i64>, Booking, Booking)> {
    let (b, approver) = booking(c, id, 0).await?;
    let mut mine = b.clone();
    mine.meeting =
        c.query_opt("SELECT meeting FROM bookings WHERE id = $1", &[&id]).await.ok().flatten().and_then(|r| r.get(0));
    let mut to = vec![b.user.id];
    to.extend(approver.filter(|a| *a != b.user.id));
    Some((to, mine, b))
}

pub(crate) async fn set_away(c: &Client, me: i64, mut away: Away) -> Option<ToApp> {
    away.user = me;
    away.since = now();
    away.times.retain(|(a, b)| b > a);
    away.times.sort();
    if away.times.len() > 100 || away.text.len() > 4000 {
        return Some(error("bad_request", "that's too much out of office"));
    }
    let json = if away.times.is_empty() { String::new() } else { serde_json::to_string(&away).unwrap_or_default() };
    match c.execute("UPDATE users SET away = $2 WHERE id = $1", &[&me, &json]).await {
        Ok(_) => Some(ToApp::AwayOf { list: if json.is_empty() { Vec::new() } else { vec![away] } }),
        Err(e) => Some(failed(e)),
    }
}

/// Who of `users` is away now or later.
pub(crate) async fn away_of(c: &Client, users: &[i64]) -> ToApp {
    let now = now();
    match c.query("SELECT id, away FROM users WHERE id = ANY($1) AND away <> ''", &[&users]).await {
        Ok(rows) => ToApp::AwayOf {
            list: rows
                .iter()
                .filter_map(|r| {
                    let mut a: Away = serde_json::from_str(r.get::<_, &str>(1)).ok()?;
                    a.user = r.get(0);
                    a.times.retain(|(_, end)| *end > now);
                    (!a.times.is_empty()).then_some(a)
                })
                .collect(),
        },
        Err(e) => failed(e),
    }
}
