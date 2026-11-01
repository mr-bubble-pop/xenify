//! What Xenify Chat does between its screens and the chat server, shared by
//! the desktop app and the browser: signing in, meetings, your calendar and
//! address books on the server, busy times and out of office.
//!
//! The two differ only in a [`Host`]: how the screens are told things, how
//! messages reach the server, where the calendar and address books are kept
//! on this device, and email (desktop only).

pub mod account;
pub mod calendar;
pub mod contacts;
pub mod meetings;
pub mod sharedcals;

use chrono_tz::Tz;
use serde::Serialize;
use std::future::Future;
use std::sync::Mutex;
use xenify_chat_proto::ToServer;

pub use calendar::{Calendar, Event};
pub use contacts::{Contact, Contacts};
pub use meetings::Details;

/// The device Xenify Chat runs on.
pub trait Host: 'static {
    /// Tells the screens (`calendar`, `meetings`, `busy`, `to_app`…).
    fn emit(&self, event: &str, payload: serde_json::Value);
    /// Sends to the server the usual way.
    fn tell(&self, m: ToServer);
    /// Keeps a store on this device: `what` is "calendar" or "contacts".
    fn save(&self, what: &str, json: String);
    /// Unix milliseconds now.
    fn now(&self) -> i64;
    /// This device's time zone.
    fn tz(&self) -> Tz;
    /// The server signed in to (for keeping per-server notes apart).
    fn server(&self) -> String;
    /// Email accounts set up here, for inviting people by email.
    fn mail_accounts(&self) -> Vec<String> {
        Vec::new()
    }
    /// Emails a meeting's guests its invitation (or `cancel`: that it's off),
    /// in the background; a failure is for the host to show.
    fn email_guests(&self, _d: &Details, _organiser: &str, _people: &[String], _cancel: bool) {}
}

/// Talking to the server over HTTP as someone (`token`, or "" for no one).
pub trait Fetch {
    /// GET (`body` None) or POST JSON; the answer, or what went wrong in words.
    fn request(
        &self,
        token: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> impl Future<Output = Result<serde_json::Value, String>>;
}

/// The shared state: the calendar, address books and meetings.
pub struct Core<H: Host> {
    pub host: H,
    pub cal: Calendar,
    pub contacts: Contacts,
    pub(crate) inner: Mutex<meetings::Inner>,
}

impl<H: Host> Core<H> {
    /// `calendar` and `contacts`: what [`Host::save`] last kept (or "").
    ///
    /// What can't be read (damaged, or kept by a newer version whose changes
    /// this one doesn't understand) is first kept aside as
    /// "<what>-unreadable-<time>", so starting empty never loses it when the
    /// next change is saved.
    pub fn new(host: H, calendar: &str, contacts: &str) -> Core<H> {
        for (what, json, ok) in
            [("calendar", calendar, Calendar::readable(calendar)), ("contacts", contacts, Contacts::readable(contacts))]
        {
            if !ok {
                host.save(&format!("{what}-unreadable-{}", host.now()), json.to_string());
            }
        }
        Core {
            host,
            cal: Calendar::load(calendar),
            contacts: Contacts::load(contacts),
            inner: Mutex::new(Default::default()),
        }
    }

    pub(crate) fn emit(&self, event: &str, payload: impl Serialize) {
        self.host.emit(event, serde_json::to_value(payload).unwrap_or_default());
    }

    pub(crate) fn tell(&self, m: ToServer) {
        self.host.tell(m);
    }

    /// Keeps whatever changed (call after each change from outside), and
    /// sends the server your reminders when they changed.
    pub fn keep(&self) {
        if let Some(json) = self.cal.changed() {
            self.host.save("calendar", json);
        }
        self.send_reminders();
        if let Some(json) = self.contacts.changed() {
            self.host.save("contacts", json);
        }
    }

    /// The screens heard a store changed on this device: tell the server.
    pub fn calendar_changed(&self) {
        meetings::calendar_changed(self);
        self.keep();
    }
}

impl<H: Host> Core<H> {
    /// Signed in: the server keeps your reminders and shows them on all
    /// your devices when they're due (see [`Calendar::reminders`]).
    fn send_reminders(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.me.is_none() {
            return;
        }
        // In the server's time.
        let skew = inner.skew;
        let list: Vec<_> = self
            .cal
            .reminders(self.host.now())
            .into_iter()
            .map(|r| xenify_chat_proto::Reminder { at: r.at + skew, ..r })
            .collect();
        if inner.reminders_sent.as_ref() != Some(&list) {
            inner.reminders_sent = Some(list.clone());
            drop(inner);
            self.tell(ToServer::SetReminders { list });
        }
    }
}

impl<H: Host> Core<H> {
    /// What the server says, before the screens hear it: meetings, your
    /// calendar, address books and busy times are taken here. Returns what
    /// the screens should still see.
    pub fn incoming(&self, m: xenify_chat_proto::ToApp) -> Option<xenify_chat_proto::ToApp> {
        let out = meetings::incoming(self, m);
        self.keep();
        out
    }

    /// A call from the screens this core answers (None: not one of its).
    pub fn call(&self, name: &str, a: &serde_json::Value) -> Option<Result<serde_json::Value, String>> {
        use serde_json::Value;
        fn arg<T: serde::de::DeserializeOwned>(a: &Value, k: &str) -> Result<T, String> {
            serde_json::from_value(a.get(k).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{k}: {e}"))
        }
        fn out<T: Serialize>(r: Result<T, String>) -> Result<Value, String> {
            r.map(|v| serde_json::to_value(v).unwrap_or_default())
        }
        let r = (|| -> Option<Result<Value, String>> {
            Some(match name {
                "cal_between" => out((|| self.cal.between(arg(a, "from")?, arg(a, "to")?))()),
                "cal_due" => out((|| self.cal.due(arg(a, "now")?))()),
                "cal_snoozed" => out((|| self.cal.snoozed(arg(a, "now")?))()),
                "cal_reminded" => out((|| self.cal.mark_reminded(arg(a, "id")?, arg(a, "at")?))()),
                "meeting_remind" => {
                    out((|| Ok::<_, String>(self.cal.meeting(arg(a, "conv")?)?.and_then(|e| e.remind)))())
                }
                "meeting_remind_set" => out((|| self.cal.set_meeting_remind(arg(a, "conv")?, arg(a, "minutes")?))()),
                // How long before a meeting new ones remind you (Settings).
                "cal_meeting_remind" => out((|| {
                    let m: i64 = arg(a, "minutes")?;
                    self.cal.set_meta("meeting_remind", &m.to_string());
                    Ok(())
                })()),
                "cal_save" => out((|| {
                    let saved = self.cal.save(&arg::<Event>(a, "event")?)?;
                    meetings::calendar_changed(self);
                    Ok(saved)
                })()),
                "cal_delete" => out((|| {
                    self.cal.delete(arg(a, "id")?)?;
                    meetings::calendar_changed(self);
                    Ok(())
                })()),
                "meetings_list" => out(Ok(meetings::meetings_list(self))),
                "meeting_new" => out((|| meetings::meeting_new(self, arg(a, "details")?, arg(a, "people")?))()),
                "meeting_change" => out((|| {
                    meetings::meeting_change(
                        self,
                        arg(a, "conv")?,
                        arg(a, "details")?,
                        arg(a, "add")?,
                        arg(a, "remove")?,
                        arg(a, "bookings").unwrap_or_default(),
                    )
                })()),
                "meeting_cancel" => {
                    out((|| meetings::meeting_cancel(self, arg(a, "conv")?, arg(a, "bookings").unwrap_or_default()))())
                }
                "meeting_answer" => out((|| meetings::meeting_answer(self, arg(a, "conv")?, arg(a, "answer")?))()),
                "meeting_busy" => out((|| {
                    meetings::meeting_busy(self, arg(a, "users")?);
                    Ok(())
                })()),
                "calendar_settings" => out(Ok(meetings::calendar_settings(self))),
                "calendar_settings_save" => out((|| meetings::calendar_settings_save(self, arg(a, "settings")?))()),
                "books_list" => out(Ok(meetings::books_list(self))),
                "book_new" => out((|| {
                    meetings::book_new(self, arg(a, "name")?, arg(a, "people")?, arg(a, "groups").unwrap_or_default())
                })()),
                "book_share" => out((|| {
                    meetings::book_share(
                        self,
                        arg(a, "conv")?,
                        arg(a, "add")?,
                        arg(a, "remove")?,
                        arg(a, "roles")?,
                        arg(a, "groups").unwrap_or_default(),
                    )
                })()),
                "book_rename" => out((|| meetings::book_rename(self, arg(a, "conv")?, arg(a, "name")?))()),
                "book_leave" => out((|| {
                    meetings::book_leave(self, arg(a, "conv")?);
                    Ok(())
                })()),
                "shared_cals" => out(Ok(sharedcals::list(self))),
                "shared_cal_between" => out((|| sharedcals::between(self, arg(a, "from")?, arg(a, "to")?))()),
                "shared_cal_new" => out((|| sharedcals::new(self, arg(a, "name")?, arg(a, "people")?))()),
                "shared_cal_share" => out((|| {
                    sharedcals::share(self, arg(a, "conv")?, arg(a, "add")?, arg(a, "remove")?, arg(a, "roles")?)
                })()),
                "shared_cal_rename" => out((|| sharedcals::rename(self, arg(a, "conv")?, arg(a, "name")?))()),
                "shared_cal_leave" => out((|| {
                    sharedcals::leave(self, arg(a, "conv")?);
                    Ok(())
                })()),
                "shared_cal_remove" => out((|| sharedcals::remove(self, arg(a, "conv")?))()),
                "shared_cal_save" => out((|| sharedcals::save(self, arg(a, "conv")?, arg(a, "event")?))()),
                "shared_cal_delete" => out((|| sharedcals::delete(self, arg(a, "conv")?, arg(a, "uid")?))()),
                "away_get" => out(Ok(meetings::away_get(self))),
                "away_save" => out((|| meetings::away_save(self, arg(a, "away")?))()),
                "contacts_list" => out((|| self.contacts.list(arg(a, "book")?))()),
                "contacts_find" => out((|| self.contacts.find(&arg::<String>(a, "q")?))()),
                "contact_save" => out((|| {
                    let c: Contact = arg(a, "contact")?;
                    if c.book != 0 && !meetings::can_change_book(self, c.book) {
                        return Err("Only people who can change this contact list can do that.".to_string());
                    }
                    let saved = self.contacts.save(&c)?;
                    meetings::contacts_changed(self);
                    Ok(saved)
                })()),
                "contact_delete" => out((|| {
                    let book: i64 = arg(a, "book")?;
                    if book != 0 && !meetings::can_change_book(self, book) {
                        return Err("Only people who can change this contact list can do that.".to_string());
                    }
                    self.contacts.delete(book, &arg::<String>(a, "uid")?)?;
                    meetings::contacts_changed(self);
                    Ok(())
                })()),
                "contacts_vcard_text" => {
                    out((|| Ok::<_, String>(contacts::vcards(&self.contacts.list(arg(a, "book")?)?)))())
                }
                _ => return None,
            })
        })();
        self.keep();
        r
    }

    /// People you just emailed (their addresses and names) go to My contacts.
    pub fn emailed(&self, people: &[(String, String)]) {
        let day = meetings::today(self);
        let mut added = false;
        for (name, email) in people {
            added |= self.contacts.add_if_new(name, email, "", "emailed them", &day);
        }
        if added {
            meetings::contacts_changed(self);
        }
        self.keep();
    }

    /// Someone invited by email answered (an iCalendar reply to meeting `uid`).
    pub fn guest_answered(&self, uid: &str, email: &str, answer: &str) {
        meetings::guest_answered(self, uid, email, answer);
        self.keep();
    }

    /// Your out of office, for the mail side's automatic replies.
    pub fn my_away(&self) -> Option<xenify_chat_proto::Away> {
        self.inner.lock().unwrap().away.clone()
    }
}
