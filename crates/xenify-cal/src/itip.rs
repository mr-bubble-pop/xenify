//! Meeting invitations by email (iTIP, RFC 5546; iMIP, RFC 6047): what an
//! invitation asks, answering it, and sending invitations and
//! cancellations. Outlook, Google, Apple and Zoom all speak this.

use crate::event::{Answer, Event, Person, calendar};
use crate::ics;

/// What an invitation email is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// "Please come" (new, or an update).
    Request,
    /// The meeting is off.
    Cancel,
    /// Someone's answer to your invitation.
    Reply,
    /// Just an event to add (no answer wanted).
    Publish,
    /// A suggested new time.
    Counter,
    Other,
}

impl Method {
    fn parse(s: &str) -> Method {
        match s.trim().to_ascii_uppercase().as_str() {
            "REQUEST" => Method::Request,
            "CANCEL" => Method::Cancel,
            "REPLY" => Method::Reply,
            "PUBLISH" => Method::Publish,
            "COUNTER" => Method::Counter,
            _ => Method::Other,
        }
    }

    pub fn ics(self) -> &'static str {
        match self {
            Method::Request => "REQUEST",
            Method::Cancel => "CANCEL",
            Method::Reply => "REPLY",
            Method::Publish => "PUBLISH",
            Method::Counter => "COUNTER",
            Method::Other => "PUBLISH",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    pub method: Method,
    pub events: Vec<Event>,
}

impl Invite {
    /// Reads a `text/calendar` part. None if there's no event in it.
    pub fn read(text: &str) -> Option<Invite> {
        let cals = ics::parse(text).ok()?;
        let method = cals.iter().find_map(|c| c.text("METHOD")).map(|m| Method::parse(&m)).unwrap_or(Method::Publish);
        let events: Vec<Event> =
            cals.iter().flat_map(|c| c.children_named("VEVENT")).filter_map(Event::from_component).collect();
        (!events.is_empty()).then_some(Invite { method, events })
    }

    /// The main event (not a changed occurrence), else the first.
    pub fn event(&self) -> &Event {
        self.events.iter().find(|e| e.recurrence_id.is_none()).unwrap_or(&self.events[0])
    }

    /// The attendee who is you (any of `my_addresses`).
    pub fn me<'a>(&'a self, my_addresses: &[String]) -> Option<&'a Person> {
        self.event().attendees.iter().find(|a| my_addresses.iter().any(|m| m.eq_ignore_ascii_case(&a.email)))
    }
}

/// Your answer to `event`, as an iCalendar REPLY for the organizer. Only
/// your own attendee line is included, as RFC 5546 asks.
pub fn reply(event: &Event, me: &Person, answer: Answer) -> String {
    let mut e = event.clone();
    e.attendees = vec![Person { answer, rsvp: false, ..me.clone() }];
    e.alarms.clear();
    e.description.clear();
    calendar(&[e], Some(Method::Reply.ics()))
}

/// An invitation to send (new or updated).
pub fn request(event: &Event) -> String {
    calendar(std::slice::from_ref(event), Some(Method::Request.ics()))
}

/// A cancellation to send to everyone invited.
pub fn cancel(event: &Event) -> String {
    let mut e = event.clone();
    e.status = "CANCELLED".into();
    e.sequence += 1;
    calendar(&[e], Some(Method::Cancel.ics()))
}

/// What to do with an incoming invitation given the copy you have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// You don't have it yet.
    New,
    /// It changes the one you have.
    Update,
    /// You already have this version (or a newer one).
    Same,
    Older,
}

/// Compares an incoming copy with the stored one (SEQUENCE, then the time
/// it was stamped).
pub fn freshness(incoming: &Event, stored: Option<&Event>) -> Freshness {
    let Some(s) = stored else { return Freshness::New };
    match incoming.sequence.cmp(&s.sequence) {
        std::cmp::Ordering::Greater => Freshness::Update,
        std::cmp::Ordering::Less => Freshness::Older,
        std::cmp::Ordering::Equal if incoming.stamp > s.stamp && !s.stamp.is_empty() => Freshness::Update,
        std::cmp::Ordering::Equal => Freshness::Same,
    }
}

/// Records someone's REPLY on your copy of the event (you organized it).
/// Returns the updated event and who answered, if they're on the list.
pub fn apply_reply(stored: &Event, reply: &Event) -> Option<(Event, Person)> {
    let answered = reply.attendees.first()?;
    let mut e = stored.clone();
    let a = e.attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(&answered.email))?;
    a.answer = answered.answer;
    Some((e.clone(), answered.clone()))
}

/// "Accepted: Q4 planning" and the like, for the reply email's subject.
pub fn reply_subject(answer: Answer, summary: &str) -> String {
    let word = match answer {
        Answer::Accepted => "Accepted",
        Answer::Tentative => "Tentative",
        Answer::Declined => "Declined",
        _ => "Reply",
    };
    format!("{word}: {summary}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:m1\r\nSEQUENCE:2\r\nDTSTAMP:20261015T091500Z\r\nSUMMARY:Q4 planning\r\nDTSTART:20261022T130000Z\r\nDTEND:20261022T140000Z\r\nORGANIZER;CN=Pat:mailto:pat@example.com\r\nATTENDEE;CN=Sam Lee;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:sam@example.org\r\nATTENDEE;CN=Ana;PARTSTAT=ACCEPTED:mailto:ana@example.org\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn answering_an_invitation() {
        let inv = Invite::read(REQUEST).unwrap();
        assert_eq!(inv.method, Method::Request);
        let me = inv.me(&["SAM@example.org".into()]).unwrap().clone();
        let text = reply(inv.event(), &me, Answer::Accepted);
        let back = Invite::read(&text).unwrap();
        assert_eq!(back.method, Method::Reply);
        let e = back.event();
        assert_eq!(e.uid, "m1");
        assert_eq!(e.sequence, 2, "replies carry the sequence they answer");
        assert_eq!(e.attendees.len(), 1, "only my own line");
        assert_eq!(e.attendees[0].answer, Answer::Accepted);
        assert_eq!(e.organizer.as_ref().unwrap().email, "pat@example.com");
        assert!(e.alarms.is_empty());
        assert_eq!(reply_subject(Answer::Accepted, &e.summary), "Accepted: Q4 planning");
    }

    #[test]
    fn organizer_records_answers_and_cancels() {
        let mine = Invite::read(REQUEST).unwrap().event().clone();
        let me = mine.attendees[0].clone();
        let answer = Invite::read(&reply(&mine, &me, Answer::Declined)).unwrap();
        let (updated, who) = apply_reply(&mine, answer.event()).unwrap();
        assert_eq!(who.email, "sam@example.org");
        assert_eq!(updated.attendees[0].answer, Answer::Declined);
        assert_eq!(updated.attendees[1].answer, Answer::Accepted, "others unchanged");

        let c = Invite::read(&cancel(&mine)).unwrap();
        assert_eq!(c.method, Method::Cancel);
        assert!(c.event().cancelled());
        assert_eq!(c.event().sequence, 3);
    }

    #[test]
    fn updates_are_told_from_old_copies() {
        let e = Invite::read(REQUEST).unwrap().event().clone();
        assert_eq!(freshness(&e, None), Freshness::New);
        assert_eq!(freshness(&e, Some(&e)), Freshness::Same);
        let mut newer = e.clone();
        newer.sequence = 3;
        assert_eq!(freshness(&newer, Some(&e)), Freshness::Update);
        assert_eq!(freshness(&e, Some(&newer)), Freshness::Older);
        let mut restamped = e.clone();
        restamped.stamp = "20261016T080000Z".into();
        assert_eq!(freshness(&restamped, Some(&e)), Freshness::Update);
    }
}
