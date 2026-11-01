//! Address books on this device: yours (My contacts) and shared ones, as
//! copies of what's on the server (see `meetings`, which keeps them in step:
//! My contacts is a group of one, a shared book a channel whose owners can
//! change it). People you email or invite are added to My contacts by
//! themselves; people who only write to you aren't.
//!
//! Kept in memory and saved whole by the host.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    /// 0 for My contacts, else the shared book's conversation.
    #[serde(default)]
    pub book: i64,
    /// The same contact on every device (empty: a new one).
    #[serde(default)]
    pub uid: String,
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub company: String,
    /// Their job ("Product lead").
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub notes: String,
    /// Their user name on this server, if they're on it.
    #[serde(default)]
    pub username: String,
    /// How they got here, when it was by themselves ("Added when you
    /// emailed them on 12 October").
    #[serde(default)]
    pub added: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Kept {
    #[serde(flatten)]
    pub contact: Contact,
    /// Changes not yet on the server.
    #[serde(default)]
    pub dirty: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Book {
    pub name: String,
    /// Who it's shared with, as its owners said (JSON: groups and people).
    #[serde(default)]
    pub sharing: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Data {
    #[serde(default)]
    pub contacts: Vec<Kept>,
    /// Deleted here, not yet on the server: (book, uid).
    #[serde(default)]
    pub gone: BTreeSet<(i64, String)>,
    /// Shared books' names and sharing.
    #[serde(default)]
    pub books: BTreeMap<i64, Book>,
}

pub struct Contacts {
    data: Mutex<Data>,
    changed: Mutex<bool>,
}

impl Contacts {
    /// Whether `json` (what was kept, "" for none) reads as kept data.
    pub fn readable(json: &str) -> bool {
        json.trim().is_empty() || serde_json::from_str::<Data>(json).is_ok()
    }

    pub fn load(json: &str) -> Contacts {
        Contacts::from_data(serde_json::from_str(json).unwrap_or_default())
    }

    pub fn from_data(data: Data) -> Contacts {
        Contacts { data: Mutex::new(data), changed: Mutex::new(false) }
    }

    /// What to keep, if anything changed since last asked.
    pub fn changed(&self) -> Option<String> {
        let mut c = self.changed.lock().unwrap();
        if !*c {
            return None;
        }
        *c = false;
        serde_json::to_string(&*self.data.lock().unwrap()).ok()
    }

    fn change<T>(&self, f: impl FnOnce(&mut Data) -> T) -> T {
        let out = f(&mut self.data.lock().unwrap());
        *self.changed.lock().unwrap() = true;
        out
    }

    fn read<T>(&self, f: impl FnOnce(&Data) -> T) -> T {
        f(&self.data.lock().unwrap())
    }

    pub fn list(&self, book: i64) -> Result<Vec<Contact>, String> {
        let mut out: Vec<Contact> =
            self.read(|d| d.contacts.iter().filter(|k| k.contact.book == book).map(|k| k.contact.clone()).collect());
        out.sort_by_key(|c| (c.name.to_lowercase(), c.email.to_lowercase()));
        Ok(out)
    }

    /// Contacts in any book whose name or email address starts a word with `q`.
    pub fn find(&self, q: &str) -> Result<Vec<Contact>, String> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let hit = |c: &Contact| {
            let name = c.name.to_lowercase();
            name.starts_with(&q) || name.contains(&format!(" {q}")) || c.email.to_lowercase().starts_with(&q)
        };
        let mut out: Vec<Contact> =
            self.read(|d| d.contacts.iter().map(|k| &k.contact).filter(|c| hit(c)).cloned().collect());
        out.sort_by_key(|c| (c.book, c.name.to_lowercase()));
        out.truncate(20);
        Ok(out)
    }

    pub fn count(&self, book: i64) -> i64 {
        self.read(|d| d.contacts.iter().filter(|k| k.contact.book == book).count() as i64)
    }

    /// Whether anyone in any book has this email address or user name.
    pub fn knows(&self, email: &str, username: &str) -> bool {
        self.read(|d| {
            d.contacts.iter().any(|k| {
                (!email.is_empty() && k.contact.email.eq_ignore_ascii_case(email))
                    || (!username.is_empty() && k.contact.username == username)
            })
        })
    }

    /// Saves one of yours (or in a book you can change); due to go to the server.
    pub fn save(&self, e: &Contact) -> Result<Contact, String> {
        if e.name.trim().is_empty() && e.email.trim().is_empty() {
            return Err("Give them a name or an email address.".into());
        }
        let uid = if e.uid.is_empty() {
            xenify_sync::random(16).iter().map(|b| format!("{b:02x}")).collect()
        } else {
            e.uid.clone()
        };
        let saved = Contact {
            uid,
            name: if e.name.trim().is_empty() { e.email.trim().to_string() } else { e.name.trim().to_string() },
            email: e.email.trim().to_string(),
            ..e.clone()
        };
        self.store(&saved, true);
        Ok(saved)
    }

    fn store(&self, e: &Contact, mine: bool) {
        self.change(|d| match d.contacts.iter_mut().find(|k| k.contact.book == e.book && k.contact.uid == e.uid) {
            Some(k) => {
                k.contact = e.clone();
                if mine {
                    k.dirty += 1;
                }
            }
            None => d.contacts.push(Kept { contact: e.clone(), dirty: i64::from(mine) }),
        });
    }

    pub fn delete(&self, book: i64, uid: &str) -> Result<(), String> {
        self.change(|d| {
            d.gone.insert((book, uid.to_string()));
            d.contacts.retain(|k| !(k.contact.book == book && k.contact.uid == uid));
        });
        Ok(())
    }

    // Syncing (see `meetings`).

    pub fn unsent(&self) -> Result<Vec<(Contact, i64)>, String> {
        Ok(self.read(|d| d.contacts.iter().filter(|k| k.dirty > 0).map(|k| (k.contact.clone(), k.dirty)).collect()))
    }

    pub fn sent(&self, book: i64, uid: &str, changes: i64) {
        self.change(|d| {
            if let Some(k) =
                d.contacts.iter_mut().find(|k| k.contact.book == book && k.contact.uid == uid && k.dirty == changes)
            {
                k.dirty = 0;
            }
        });
    }

    pub fn gone(&self) -> Result<Vec<(i64, String)>, String> {
        Ok(self.read(|d| d.gone.iter().cloned().collect()))
    }

    pub fn gone_sent(&self, book: i64, uid: &str) {
        self.change(|d| d.gone.remove(&(book, uid.to_string())));
    }

    /// A contact as the server has it (unless it changed here meanwhile).
    pub fn put_from_server(&self, e: &Contact) {
        let busy = self.read(|d| {
            d.gone.contains(&(e.book, e.uid.clone()))
                || d.contacts.iter().any(|k| k.contact.book == e.book && k.contact.uid == e.uid && k.dirty > 0)
        });
        if !busy {
            self.store(e, false);
        }
    }

    pub fn delete_from_server(&self, book: i64, uid: &str) {
        self.change(|d| d.contacts.retain(|k| !(k.contact.book == book && k.contact.uid == uid)));
    }

    pub fn set_book_name(&self, conv: i64, name: &str) {
        self.change(|d| d.books.entry(conv).or_default().name = name.to_string());
    }

    pub fn book_name(&self, conv: i64) -> Option<String> {
        self.read(|d| d.books.get(&conv).map(|b| b.name.clone()).filter(|n| !n.is_empty()))
    }

    pub fn set_book_sharing(&self, conv: i64, sharing: &str) {
        self.change(|d| d.books.entry(conv).or_default().sharing = sharing.to_string());
    }

    pub fn book_sharing(&self, conv: i64) -> String {
        self.read(|d| d.books.get(&conv).map(|b| b.sharing.clone()).unwrap_or_default())
    }

    pub fn book_convs(&self) -> Result<Vec<i64>, String> {
        Ok(self.read(|d| {
            let mut all: BTreeSet<i64> = d.books.keys().copied().collect();
            all.extend(d.contacts.iter().map(|k| k.contact.book).filter(|b| *b != 0));
            all.into_iter().collect()
        }))
    }

    /// A shared book you're no longer in.
    pub fn drop_book(&self, conv: i64) {
        self.change(|d| {
            d.contacts.retain(|k| k.contact.book != conv);
            d.gone.retain(|(b, _)| *b != conv);
            d.books.remove(&conv);
        });
    }

    /// Adds someone you wrote to or invited, unless they're in a book
    /// already; `day` is today as people say it ("12 October").
    pub fn add_if_new(&self, name: &str, email: &str, username: &str, how: &str, day: &str) -> bool {
        if (email.is_empty() && username.is_empty()) || self.knows(email, username) {
            return false;
        }
        let c = Contact {
            name: if name.trim().is_empty() { email.to_string() } else { name.trim().to_string() },
            email: email.trim().to_string(),
            username: username.to_string(),
            added: format!("Added when you {how} on {day}."),
            ..Contact::default()
        };
        self.save(&c).is_ok()
    }
}

/// One line of a vCard, escaped.
fn vline(out: &mut String, key: &str, v: &str) {
    if !v.is_empty() {
        let v = v.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;");
        out.push_str(&format!("{key}:{v}\r\n"));
    }
}

/// Contacts as vCards (Outlook, Gmail and Apple Contacts read them).
pub fn vcards(list: &[Contact]) -> String {
    let mut out = String::new();
    for c in list {
        out.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        vline(&mut out, "FN", &c.name);
        vline(&mut out, "EMAIL", &c.email);
        vline(&mut out, "TEL", &c.phone);
        vline(&mut out, "ORG", &c.company);
        vline(&mut out, "TITLE", &c.role);
        vline(&mut out, "NOTE", &c.notes);
        out.push_str("END:VCARD\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_you_write_to_are_added_once() {
        let st = Contacts::load("");
        assert!(st.add_if_new("Dana Okafor", "dana@client.example", "", "emailed them", "12 October"));
        assert!(!st.add_if_new("", "DANA@client.example", "", "emailed them", "12 October"), "already there");
        let list = st.list(0).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].added, "Added when you emailed them on 12 October.");
        assert_eq!(st.find("oka").unwrap().len(), 1, "a later word of the name");
        assert_eq!(st.find("dana@").unwrap().len(), 1);
        // From the server: not over a change made here that hasn't gone yet.
        let mine = &list[0];
        st.put_from_server(&Contact { name: "Someone else".into(), ..mine.clone() });
        assert_eq!(st.list(0).unwrap()[0].name, "Dana Okafor");
        let (_, n) = st.unsent().unwrap()[0].clone();
        st.sent(0, &mine.uid, n);
        st.put_from_server(&Contact { name: "Dana O.".into(), ..mine.clone() });
        assert_eq!(st.list(0).unwrap()[0].name, "Dana O.");
        assert!(vcards(&st.list(0).unwrap()).contains("FN:Dana O.\r\nEMAIL:dana@client.example"));
        let again = Contacts::load(&st.changed().unwrap());
        assert_eq!(again.count(0), 1);
    }
}
