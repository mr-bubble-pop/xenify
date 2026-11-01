//! Email in the same window as chat: any number of accounts (IMAP and
//! SMTP), each kept in a local copy that a background thread keeps in sync.
//! Passwords live in the system keyring; the account settings in
//! `mail-accounts.json`; unfinished emails in `mail-drafts.json`.
//!
//! The screens list and read from the local copies, and hear `mail` events
//! ("changed", "downloaded", "status", "problem", "signin_failed"), each
//! naming its account.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use xenify_mail_core::account::{self, Account, Lookup};
use xenify_mail_core::imap::{Client, Role, Security, Server};
use xenify_mail_core::message::{self, Attachment};
use xenify_mail_core::send::{self, Outgoing};
use xenify_mail_core::store::{Row, Store};
use xenify_mail_core::sync::{self, Cmd, Event, Handle};

/// The most that can be attached to one email, in bytes.
const MAX_ATTACHED: usize = 25 * 1024 * 1024;

#[derive(Default)]
pub struct Mail {
    on: Mutex<Vec<(Account, Handle)>>,
}

fn accounts_path() -> PathBuf {
    xenify_wp_check::config_dir().join("mail-accounts.json")
}

/// The saved accounts (moving the one-account file of earlier versions over).
fn saved_accounts() -> Vec<Account> {
    if let Ok(b) = std::fs::read(accounts_path()) {
        // One by one: an account this version can't read is skipped, not
        // all of them. A file that can't be read at all is kept aside
        // before it could be written over.
        return match serde_json::from_slice::<Vec<serde_json::Value>>(&b) {
            Ok(list) => list.into_iter().filter_map(|a| serde_json::from_value(a).ok()).collect(),
            Err(_) => {
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let _ = std::fs::rename(accounts_path(), accounts_path().with_extension(format!("json.unreadable-{at}")));
                Vec::new()
            }
        };
    }
    let old = xenify_wp_check::config_dir().join("mail-account.json");
    let list: Vec<Account> =
        std::fs::read(&old).ok().and_then(|b| serde_json::from_slice(&b).ok()).into_iter().collect();
    if !list.is_empty() && write_accounts(&list).is_ok() {
        let _ = std::fs::remove_file(old);
    }
    list
}

fn write_accounts(list: &[Account]) -> Result<(), String> {
    let p = accounts_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written whole, then swapped in at once.
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
struct MailEvent {
    account: String,
    kind: &'static str,
    text: String,
    folder: String,
    uid: u32,
}

impl Mail {
    /// Starts syncing `account` and passes its news to the screens.
    fn start(&self, app: &AppHandle, account: Account, password: String) {
        let mut handle = sync::start(account.clone(), password, account.store_path());
        let (_, dummy) = std::sync::mpsc::channel();
        let events = std::mem::replace(&mut handle.events, dummy);
        let (app, email) = (app.clone(), account.email.clone());
        let store_path = account.store_path();
        std::thread::spawn(move || {
            // Answers to meetings you invited people to by email.
            let mut looked = std::collections::HashSet::new();
            while let Ok(e) = events.recv() {
                match &e {
                    Event::Changed => {
                        find_meeting_replies(&app, &email, &store_path, &mut looked);
                        find_away_replies(&app, &email, &store_path);
                    }
                    Event::Downloaded { folder, uid } => {
                        meeting_reply(&app, &store_path, folder, *uid);
                        away_reply(&app, &email, &store_path, folder, *uid);
                    }
                    _ => {}
                }
                let (kind, text, folder, uid) = match e {
                    Event::Status(t) => ("status", t, String::new(), 0),
                    Event::Changed => ("changed", String::new(), String::new(), 0),
                    Event::Downloaded { folder, uid } => ("downloaded", String::new(), folder, uid),
                    Event::Problem(t) => ("problem", t, String::new(), 0),
                    Event::SignInFailed(t) => ("signin_failed", t, String::new(), 0),
                };
                let _ = app.emit("mail", MailEvent { account: email.clone(), kind, text, folder, uid });
            }
        });
        let mut on = self.on.lock().unwrap();
        on.retain(|(a, _)| a.email != account.email);
        on.push((account, handle));
    }

    fn account(&self, email: &str) -> Result<Account, String> {
        self.on
            .lock()
            .unwrap()
            .iter()
            .find(|(a, _)| a.email == email)
            .map(|(a, _)| a.clone())
            .ok_or_else(|| format!("{email} isn't set up on this computer."))
    }

    fn tell(&self, email: &str, cmd: Cmd) {
        if let Some((_, h)) = self.on.lock().unwrap().iter().find(|(a, _)| a.email == email) {
            h.send(cmd);
        }
    }

    fn store(&self, email: &str) -> Result<Store, String> {
        Store::open(&self.account(email)?.store_path()).map_err(|e| e.to_string())
    }

    fn emails(&self) -> Vec<String> {
        self.on.lock().unwrap().iter().map(|(a, _)| a.email.clone()).collect()
    }
}

/// The newest emails in your inbox that look like answers to invitations
/// ("Accepted: …"): download them (then [`meeting_reply`] reads them).
fn find_meeting_replies(
    app: &AppHandle,
    email: &str,
    store_path: &std::path::Path,
    looked: &mut std::collections::HashSet<u32>,
) {
    let Ok(s) = Store::open(store_path) else { return };
    let Ok(Some(inbox)) = s.folder_with(Role::Inbox) else { return };
    for r in s.list(&inbox, 30).unwrap_or_default() {
        let answer_like = ["Accepted:", "Tentative:", "Declined:"].iter().any(|w| r.subject.starts_with(w));
        if !answer_like || !looked.insert(r.uid) {
            continue;
        }
        match s.raw(&inbox, r.uid) {
            Ok(Some(_)) => meeting_reply(app, store_path, &inbox, r.uid),
            _ => tauri::Manager::state::<Mail>(app).tell(email, Cmd::Download { folder: inbox.clone(), uid: r.uid }),
        }
    }
}

/// If this email answers an invitation you sent, the meeting hears it.
fn meeting_reply(app: &AppHandle, store_path: &std::path::Path, folder: &str, uid: u32) {
    use xenify_cal::event::Answer;
    use xenify_cal::itip::{Invite, Method};
    let Some(m) = Store::open(store_path)
        .ok()
        .and_then(|s| s.raw(folder, uid).ok().flatten())
        .and_then(|raw| message::parse(&raw))
    else {
        return;
    };
    let Some(invite) = m.calendar.as_deref().and_then(Invite::read).filter(|i| i.method == Method::Reply) else {
        return;
    };
    let e = invite.event();
    // Only the guest can answer for themselves (anyone can write a reply
    // naming someone else).
    let Some(who) = e.attendees.first().filter(|a| a.email.trim().eq_ignore_ascii_case(m.from_address.trim())) else {
        return;
    };
    let answer = match who.answer {
        Answer::Accepted => "yes",
        Answer::Tentative => "maybe",
        Answer::Declined => "no",
        _ => return,
    };
    tauri::Manager::state::<crate::host::Chat>(app).guest_answered(&e.uid, &who.email, answer);
}

/// While you're away: new emails in your inbox that may get your automatic
/// reply (downloaded first, to see whether they're from a list or a machine).
fn find_away_replies(app: &AppHandle, email: &str, store_path: &std::path::Path) {
    let Some(away) = tauri::Manager::state::<crate::host::Chat>(app)
        .my_away()
        .filter(|a| a.email_to != "nobody" && !a.text.trim().is_empty())
    else {
        return;
    };
    let now = chrono::Utc::now().timestamp_millis();
    let Some(&(from, _)) = away.times.iter().find(|(s, e)| *s <= now && now < *e) else { return };
    let Ok(s) = Store::open(store_path) else { return };
    let Ok(Some(inbox)) = s.folder_with(Role::Inbox) else { return };
    for r in s.list(&inbox, 30).unwrap_or_default() {
        if r.is_read() || r.date * 1000 < from {
            continue;
        }
        match s.raw(&inbox, r.uid) {
            Ok(Some(_)) => away_reply(app, email, store_path, &inbox, r.uid),
            _ => tauri::Manager::state::<Mail>(app).tell(email, Cmd::Download { folder: inbox.clone(), uid: r.uid }),
        }
    }
}

/// Sends your automatic reply to this email's sender, once per time away,
/// unless it's from a list or a machine (or, if you chose, someone not in
/// your address books).
fn away_reply(app: &AppHandle, email: &str, store_path: &std::path::Path, folder: &str, uid: u32) {
    let Some(away) = tauri::Manager::state::<crate::host::Chat>(app)
        .my_away()
        .filter(|a| a.email_to != "nobody" && !a.text.trim().is_empty())
    else {
        return;
    };
    let now = chrono::Utc::now().timestamp_millis();
    let Some(&(from, _)) = away.times.iter().find(|(s, e)| *s <= now && now < *e) else { return };
    let Ok(s) = Store::open(store_path) else { return };
    if s.folder_with(Role::Inbox).ok().flatten().as_deref() != Some(folder) {
        return;
    }
    let Some(row) = s.row(folder, uid).ok().flatten().filter(|r| r.date * 1000 >= from) else { return };
    let Some(m) = s.raw(folder, uid).ok().flatten().and_then(|raw| message::parse(&raw)) else { return };
    let to = m.from_address.clone();
    let me = saved_accounts().into_iter().find(|a| a.email == email);
    if m.automatic || to.is_empty() || me.as_ref().is_none_or(|a| a.email.eq_ignore_ascii_case(&to)) {
        return;
    }
    let chat = tauri::Manager::state::<crate::host::Chat>(app);
    if away.email_to == "contacts" && !chat.contacts.knows(&to, "") {
        return;
    }
    // Once per person per time away.
    let cal = &chat.cal;
    let once = format!("autoreply:{email}:{from}:{}", to.to_lowercase());
    if cal.get_meta(&once).is_some() {
        return;
    }
    cal.set_meta(&once, &row.uid.to_string());
    chat.keep();
    let Some(acct) = me else { return };
    let Some(password) = acct.password() else { return };
    let mut references = m.references.clone();
    if !m.message_id.is_empty() {
        references.push(m.message_id.clone());
    }
    let out = Outgoing {
        from_name: acct.name.clone(),
        from_email: acct.email.clone(),
        // Only the one address checked above (a From line can name several).
        to: to.clone(),
        subject: format!("Automatic reply: {}", m.subject),
        html: html_of(&away.text),
        text: away.text.clone(),
        in_reply_to: (!m.message_id.is_empty()).then(|| m.message_id.clone()),
        references,
        auto_reply: true,
        ..Outgoing::default()
    };
    if let Ok(msg) = send::build(&out) {
        std::thread::spawn(move || {
            let _ = send::send(&acct, &password, &msg);
        });
    }
}

/// The email addresses set up on this computer.
pub fn account_emails(_app: &AppHandle) -> Vec<String> {
    saved_accounts().into_iter().map(|a| a.email).collect()
}

/// Emails a meeting's guests its invitation (or `cancel`: that it's off),
/// from the meeting's email account. `people`: who on the server is invited.
pub async fn send_invitation(
    _app: &AppHandle,
    d: &xenify_chat_core::Details,
    organiser: &str,
    people: &[String],
    cancel: bool,
) -> Result<(), String> {
    use chrono::TimeZone;
    use xenify_cal::event::{Answer, Event as CalEvent, Person, When};
    let acct = saved_accounts()
        .into_iter()
        .find(|a| a.email == d.mail_from)
        .ok_or_else(|| format!("{} isn't set up on this computer.", d.mail_from))?;
    let password = acct.password().ok_or("The email password isn't saved on this computer; add the account again.")?;
    let at = |ms: i64| chrono::Utc.timestamp_millis_opt(ms).single().unwrap_or_default();
    let mut e = CalEvent::new(&d.uid, &d.title, When::Utc(at(d.start)), Some(When::Utc(at(d.end.max(d.start)))));
    let me = if acct.name.is_empty() { organiser.to_string() } else { acct.name.clone() };
    let call_note = format!(
        "This meeting is a call in Xenify Chat, for people on {me}'s server. Reply to {me} for another way to join."
    );
    let rooms: Vec<&str> = d.things.iter().map(|t| t.name.as_str()).collect();
    let place = [rooms.join(", ").as_str(), d.place.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    e.location = if place.is_empty() { "Xenify Chat call".into() } else { place.clone() };
    e.description = [d.notes.as_str(), if d.place.is_empty() { call_note.as_str() } else { "" }]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    e.sequence = if cancel { d.sequence.saturating_sub(1) } else { d.sequence };
    e.organizer = Some(Person { email: acct.email.clone(), name: me.clone(), ..Person::default() });
    e.attendees = d
        .guests
        .iter()
        .map(|g| Person {
            email: g.email.clone(),
            name: g.name.clone(),
            answer: match g.answer.as_str() {
                "yes" => Answer::Accepted,
                "maybe" => Answer::Tentative,
                "no" => Answer::Declined,
                _ => Answer::NeedsAction,
            },
            rsvp: true,
            ..Person::default()
        })
        .collect();
    let (method, ics) =
        if cancel { ("CANCEL", xenify_cal::itip::cancel(&e)) } else { ("REQUEST", xenify_cal::itip::request(&e)) };
    let tz = crate::calendar::local_tz();
    let (s, end) = (at(d.start).with_timezone(&tz), at(d.end.max(d.start)).with_timezone(&tz));
    let when = format!("{}–{} ({})", s.format("%A %-d %B %Y, %H:%M"), end.format("%H:%M"), tz.name());
    let short = format!("{}–{}", s.format("%a %-d %b %H:%M"), end.format("%H:%M"));
    let mut with: Vec<String> = people.to_vec();
    with.extend(d.guests.iter().map(|g| if g.name.is_empty() { g.email.clone() } else { g.name.clone() }));
    let text = if cancel {
        format!("{me} called off {}, {when}.", d.title)
    } else {
        let mut t = format!("{me} invites you to {}.\n\n{when}\nWith {}", d.title, with.join(", "));
        if !place.is_empty() {
            t.push_str(&format!("\nWhere: {place}"));
        }
        for part in [d.notes.as_str(), if d.place.is_empty() { call_note.as_str() } else { "" }] {
            if !part.is_empty() {
                t.push_str("\n\n");
                t.push_str(part);
            }
        }
        t
    };
    let out = Outgoing {
        from_name: me,
        from_email: acct.email.clone(),
        to: d.guests.iter().map(|g| g.email.clone()).collect::<Vec<_>>().join(", "),
        subject: if cancel { format!("Cancelled: {}", d.title) } else { format!("Invitation: {} · {short}", d.title) },
        html: html_of(&text),
        text,
        calendar: Some((method.into(), ics)),
        ..Outgoing::default()
    };
    let m = send::build(&out)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        send::send(&acct, &password, &m)?;
        let _ = send::save_sent_copy(&acct, &password, &m.formatted());
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct AccountOut {
    email: String,
    name: String,
    /// Where the password is kept, or empty when it isn't.
    kept: String,
}

impl AccountOut {
    fn of(a: &Account) -> AccountOut {
        AccountOut { email: a.email.clone(), name: a.name.clone(), kept: a.password_place().unwrap_or_default().into() }
    }
}

/// The email accounts (starting to sync any that aren't yet).
#[tauri::command]
pub fn mail_accounts(app: AppHandle, mail: State<'_, Mail>) -> Vec<AccountOut> {
    let running = mail.emails();
    saved_accounts()
        .into_iter()
        .map(|a| {
            let out = AccountOut::of(&a);
            if !running.contains(&a.email)
                && let Some(p) = a.password()
            {
                mail.start(&app, a, p);
            }
            out
        })
        .collect()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ServerIn {
    host: String,
    port: u16,
    /// "tls", "starttls" (or "none", only for this computer).
    security: String,
}

impl ServerIn {
    fn server(&self) -> Server {
        let security = match self.security.as_str() {
            "starttls" => Security::StartTls,
            "none" => Security::None,
            _ => Security::Tls,
        };
        Server { host: self.host.trim().into(), port: self.port, security }
    }
    fn of(s: &Server) -> ServerIn {
        let security = match s.security {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::None => "none",
        };
        ServerIn { host: s.host.clone(), port: s.port, security: security.into() }
    }
}

#[derive(Serialize)]
pub struct Settings {
    imap: ServerIn,
    smtp: ServerIn,
    user: String,
    source: String,
    help: Option<String>,
}

/// The servers for an address: built in for big providers, else the
/// provider's own settings file, else a guess. Err when the provider needs
/// its own sign-in page.
#[tauri::command]
pub async fn mail_look_up(email: String) -> Result<Option<Settings>, String> {
    let found =
        tauri::async_runtime::spawn_blocking(move || account::look_up(&email)).await.map_err(|e| e.to_string())?;
    match found {
        Some(Lookup::Found(f)) => Ok(Some(Settings {
            imap: ServerIn::of(&f.imap),
            smtp: ServerIn::of(&f.smtp),
            user: f.user,
            source: f.source.into(),
            help: f.help.map(Into::into),
        })),
        Some(Lookup::NeedsProviderSignIn(who)) => Err(format!(
            "{who} only lets apps sign in through its own web page, which Xenify Chat can't do yet. Use an app password if {who} offers one."
        )),
        None => Ok(None),
    }
}

/// Checks the sign-in, then keeps the account and starts syncing it.
#[allow(clippy::too_many_arguments)] // The fields of the screen's form.
#[tauri::command]
pub async fn mail_add(
    app: AppHandle,
    mail: State<'_, Mail>,
    email: String,
    name: String,
    user: String,
    password: String,
    imap: ServerIn,
    smtp: ServerIn,
) -> Result<AccountOut, String> {
    if saved_accounts().iter().any(|a| a.email.eq_ignore_ascii_case(email.trim())) {
        return Err(format!("{} is already set up.", email.trim()));
    }
    let account = Account {
        email: email.trim().into(),
        name: name.trim().into(),
        user: user.trim().into(),
        imap: imap.server(),
        smtp: smtp.server(),
    };
    let (a, p) = (account.clone(), password.clone());
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut c = Client::connect(&a.imap).map_err(|e| format!("Couldn't reach {}: {e}", a.imap.host))?;
        c.login(&a.user, &p).map_err(|_| "The mail server didn't accept that user name and password.".to_string())?;
        c.logout();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    account.save_password(&password)?;
    let mut list = saved_accounts();
    list.push(account.clone());
    write_accounts(&list)?;
    let out = AccountOut::of(&account);
    mail.start(&app, account, password);
    Ok(out)
}

/// Removes an account from this computer: settings, password, mail copy
/// and its drafts.
#[tauri::command]
pub fn mail_remove(mail: State<'_, Mail>, account: String) -> Result<(), String> {
    mail.on.lock().unwrap().retain(|(a, _)| a.email != account);
    let mut list = saved_accounts();
    if let Some(a) = list.iter().find(|a| a.email == account) {
        a.forget_password();
        let _ = std::fs::remove_file(a.store_path());
    }
    list.retain(|a| a.email != account);
    write_accounts(&list)?;
    let mut drafts = read_drafts();
    drafts.retain(|d| d.account != account);
    write_drafts(&drafts)
}

#[derive(Serialize)]
pub struct FolderOut {
    wire: String,
    name: String,
    role: Option<String>,
    unread: u32,
    total: u32,
}

#[tauri::command]
pub fn mail_folders(mail: State<'_, Mail>, account: String) -> Result<Vec<FolderOut>, String> {
    let s = mail.store(&account)?;
    Ok(s.folders()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|f| FolderOut {
            wire: f.wire,
            name: f.name,
            role: f.role.map(|r| format!("{r:?}").to_lowercase()),
            unread: f.unread,
            total: f.total,
        })
        .collect())
}

#[derive(Serialize)]
pub struct RowOut {
    account: String,
    folder: String,
    uid: u32,
    from: String,
    from_address: String,
    to: String,
    subject: String,
    /// Unix milliseconds, like chat messages.
    at: i64,
    read: bool,
    flagged: bool,
}

fn row_out(account: &str, r: Row) -> RowOut {
    RowOut {
        account: account.into(),
        read: r.is_read(),
        flagged: r.is_flagged(),
        folder: r.folder,
        uid: r.uid,
        from: r.from,
        from_address: r.from_address,
        to: r.to,
        subject: r.subject,
        at: r.date * 1000,
    }
}

fn list_of(mail: &Mail, account: &str, folder: &str, limit: usize) -> Result<Vec<RowOut>, String> {
    let s = mail.store(account)?;
    let wire = if folder.is_empty() {
        match s.folder_with(Role::Inbox).map_err(|e| e.to_string())? {
            Some(w) => w,
            None => return Ok(Vec::new()),
        }
    } else {
        folder.to_string()
    };
    Ok(s.list(&wire, limit).map_err(|e| e.to_string())?.into_iter().map(|r| row_out(account, r)).collect())
}

/// The newest messages in one account's `folder` ("" for its inbox), or,
/// with no account, in every inbox together.
#[tauri::command]
pub fn mail_list(mail: State<'_, Mail>, account: String, folder: String, limit: usize) -> Result<Vec<RowOut>, String> {
    if !account.is_empty() {
        return list_of(&mail, &account, &folder, limit);
    }
    let mut all: Vec<RowOut> =
        mail.emails().iter().flat_map(|a| list_of(&mail, a, "", limit).unwrap_or_default()).collect();
    all.sort_by_key(|r| std::cmp::Reverse(r.at));
    all.truncate(limit);
    Ok(all)
}

/// Every account's mail containing `q`.
#[tauri::command]
pub fn mail_search(mail: State<'_, Mail>, q: String) -> Result<Vec<RowOut>, String> {
    let mut all = Vec::new();
    for a in mail.emails() {
        if let Ok(s) = mail.store(&a) {
            all.extend(s.search(&q, 100).map_err(|e| e.to_string())?.into_iter().map(|r| row_out(&a, r)));
        }
    }
    all.sort_by_key(|r| std::cmp::Reverse(r.at));
    Ok(all)
}

#[derive(Serialize)]
pub struct Opened {
    account: String,
    folder: String,
    uid: u32,
    from: String,
    from_address: String,
    to: String,
    cc: String,
    subject: String,
    date: String,
    /// Cleaned HTML (no scripts, no internet pictures), if the email has it.
    html: Option<String>,
    text: String,
    blocked_images: usize,
    attachments: Vec<(String, String, usize)>,
    message_id: String,
    /// A meeting invitation in the email, if there is one.
    invite: Option<InviteOut>,
}

/// A meeting invitation as the reader shows it.
#[derive(Serialize)]
pub struct InviteOut {
    /// "request" (please come), "cancel", "reply" (someone's answer) or
    /// "publish" (just an event).
    method: &'static str,
    uid: String,
    title: String,
    /// Unix milliseconds.
    start: i64,
    end: i64,
    all_day: bool,
    location: String,
    url: String,
    organizer: String,
    repeats: bool,
    /// Your answer so far ("Going", "Maybe"…), if you're invited.
    answer: String,
    /// It's in your calendar already.
    in_calendar: bool,
}

fn invite_of(
    m: &message::Message,
    mine: &[String],
    cal: &xenify_chat_core::Calendar,
) -> Option<(xenify_cal::itip::Invite, InviteOut)> {
    use xenify_cal::itip::{Invite, Method};
    let invite = Invite::read(m.calendar.as_deref()?)?;
    let e = invite.event();
    let tz = crate::calendar::local_tz();
    let start = e.start.instant(tz);
    let out = InviteOut {
        method: match invite.method {
            Method::Request => "request",
            Method::Cancel => "cancel",
            Method::Reply => "reply",
            _ => "publish",
        },
        uid: e.uid.clone(),
        title: e.summary.clone(),
        start: start.timestamp_millis(),
        end: (start + e.duration(tz)).timestamp_millis(),
        all_day: e.all_day(),
        location: e.location.clone(),
        url: e.url.clone(),
        organizer: e.organizer.as_ref().map(|o| o.display()).unwrap_or_default(),
        repeats: e.rrule.is_some(),
        answer: invite.me(mine).map(|p| p.answer.label().to_string()).unwrap_or_default(),
        in_calendar: cal.by_uid(&e.uid).ok().flatten().is_some(),
    };
    Some((invite, out))
}

/// The whole message, or None while it's being downloaded (a "downloaded"
/// event follows). Opening marks it read.
#[tauri::command]
pub fn mail_open(
    app: AppHandle,
    mail: State<'_, Mail>,
    chat: State<'_, crate::host::Chat>,
    account: String,
    folder: String,
    uid: u32,
) -> Result<Option<Opened>, String> {
    let cal = &chat.cal;
    let s = mail.store(&account)?;
    let Some(raw) = s.raw(&folder, uid).map_err(|e| e.to_string())? else {
        mail.tell(&account, Cmd::Download { folder, uid });
        return Ok(None);
    };
    let m = message::parse(&raw).ok_or("This email couldn't be read.")?;
    if s.row(&folder, uid).ok().flatten().is_some_and(|r| !r.is_read()) {
        mail.tell(&account, Cmd::SetFlag { folder: folder.clone(), uid, flag: "\\Seen".into(), on: true });
    }
    let (html, blocked) = match &m.html {
        Some(h) => {
            let opts = xenify_safe_html::Options { remote_images: false, ..Default::default() };
            let clean = xenify_safe_html::clean(h, &opts, &|src| m.inline_image(src));
            (Some(clean.html), clean.blocked_images)
        }
        None => (None, 0),
    };
    let invite = invite_of(&m, &mail.emails(), cal).map(|(_, out)| out);
    if invite.as_ref().is_some_and(|i| i.method == "reply") {
        let wire = folder.clone();
        if let Ok(a) = mail.account(&account) {
            meeting_reply(&app, &a.store_path(), &wire, uid);
        }
    }
    Ok(Some(Opened {
        invite,
        account,
        folder,
        uid,
        from: m.from.clone(),
        from_address: m.from_address.clone(),
        to: m.to.clone(),
        cc: m.cc.clone(),
        subject: m.subject.clone(),
        date: message::nice_date(&m.date),
        html,
        text: m.text.clone(),
        blocked_images: blocked,
        attachments: m.attachments.iter().map(|a| (a.name.clone(), a.mime.clone(), a.data.len())).collect(),
        message_id: m.message_id.clone(),
    }))
}

#[tauri::command]
pub fn mail_flag(mail: State<'_, Mail>, account: String, folder: String, uid: u32, flag: String, on: bool) {
    mail.tell(&account, Cmd::SetFlag { folder, uid, flag, on });
}

#[tauri::command]
pub fn mail_delete(mail: State<'_, Mail>, account: String, folder: String, uid: u32) {
    mail.tell(&account, Cmd::Delete { folder, uid });
}

#[tauri::command]
pub fn mail_move(mail: State<'_, Mail>, account: String, folder: String, uid: u32, to: String) {
    mail.tell(&account, Cmd::Move { folder, uid, to });
}

/// Checks every account now.
#[tauri::command]
pub fn mail_sync_now(mail: State<'_, Mail>) {
    for a in mail.emails() {
        mail.tell(&a, Cmd::SyncNow);
    }
}

/// A file attached in the screens (its contents as base64).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AttachIn {
    name: String,
    #[serde(default)]
    mime: String,
    data: String,
}

fn attachments_of(list: &[AttachIn]) -> Result<Vec<Attachment>, String> {
    let mut total = 0;
    list.iter()
        .map(|a| {
            let data = base64::engine::general_purpose::STANDARD
                .decode(&a.data)
                .map_err(|_| format!("{} couldn't be attached.", a.name))?;
            total += data.len();
            if total > MAX_ATTACHED {
                return Err("Attachments can be 25 MB in all.".to_string());
            }
            let mime = if a.mime.is_empty() { "application/octet-stream".into() } else { a.mime.clone() };
            Ok(Attachment { name: a.name.clone(), mime, data })
        })
        .collect()
}

fn html_of(text: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let body: Vec<String> = text.split("\n\n").map(|p| format!("<p>{}</p>", esc(p).replace('\n', "<br>"))).collect();
    format!("<!doctype html><html><body style=\"font-family:sans-serif\">{}</body></html>", body.join(""))
}

/// Sends an email (a reply when `reply_to` names the message answered), and
/// keeps a copy in Sent.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // The fields of the screen's form.
pub async fn mail_send(
    app: AppHandle,
    mail: State<'_, Mail>,
    account: String,
    to: String,
    cc: String,
    subject: String,
    text: String,
    reply_to: Option<(String, u32)>,
    attachments: Vec<AttachIn>,
) -> Result<(), String> {
    let email = account;
    let account = mail.account(&email)?;
    let password =
        account.password().ok_or("The email password isn't saved on this computer; add the account again.")?;
    let (mut in_reply_to, mut references) = (None, Vec::new());
    if let Some((folder, uid)) = reply_to
        && let Ok(Some(raw)) = mail.store(&email).and_then(|s| s.raw(&folder, uid).map_err(|e| e.to_string()))
        && let Some(m) = message::parse(&raw)
    {
        references = m.references.clone();
        if !m.message_id.is_empty() {
            references.push(m.message_id.clone());
            in_reply_to = Some(m.message_id);
        }
    }
    let out = Outgoing {
        from_name: account.name.clone(),
        from_email: account.email.clone(),
        to,
        cc,
        subject,
        html: html_of(&text),
        text,
        in_reply_to,
        references,
        attachments: attachments_of(&attachments)?,
        ..Outgoing::default()
    };
    let m = send::build(&out)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        send::send(&account, &password, &m)?;
        // Not keeping a copy isn't a reason to say sending failed.
        let _ = send::save_sent_copy(&account, &password, &m.formatted());
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    // Whoever you write to goes in My contacts.
    let people: Vec<(String, String)> = [&out.to, &out.cc]
        .into_iter()
        .flat_map(|list| send::parse_list(list).unwrap_or_default())
        .map(|m| (m.name.clone().unwrap_or_default(), m.email.to_string()))
        .collect();
    tauri::Manager::state::<crate::host::Chat>(&app).emailed(&people);
    mail.tell(&email, Cmd::SyncNow);
    Ok(())
}

/// Saves attachment `index` of a message to the Downloads folder (a new name
/// if the file is there already); says where.
#[tauri::command]
pub fn mail_save_attachment(
    mail: State<'_, Mail>,
    account: String,
    folder: String,
    uid: u32,
    index: usize,
) -> Result<String, String> {
    let raw = mail.store(&account)?.raw(&folder, uid).map_err(|e| e.to_string())?.ok_or("Open the email first.")?;
    let m = message::parse(&raw).ok_or("This email couldn't be read.")?;
    let a = m.attachments.get(index).ok_or("That attachment isn't there.")?;
    let dir = xenify_wp_check::home_dir().join("Downloads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = message::safe_file_name(&a.name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&path, &a.data).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Opens a web or email link in the computer's own browser or mail program.
#[tauri::command]
pub fn open_link(url: String) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")) {
        return Err("Only web and email links are opened.".into());
    }
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    std::process::Command::new(program).arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// An email being written, kept on this computer until it's sent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Draft {
    /// Empty for a new one.
    #[serde(default)]
    pub id: String,
    pub account: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub quote: String,
    #[serde(default)]
    pub reply_to: Option<(String, u32)>,
    #[serde(default)]
    pub attachments: Vec<AttachIn>,
    /// Unix milliseconds.
    #[serde(default)]
    pub saved: i64,
}

fn drafts_path() -> PathBuf {
    xenify_wp_check::data_dir().join("mail-drafts.json")
}

fn read_drafts() -> Vec<Draft> {
    std::fs::read(drafts_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Saves where only you can read it (drafts can be private).
fn write_drafts(list: &[Draft]) -> Result<(), String> {
    let p = drafts_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    use std::io::Write;
    o.open(&p).and_then(|mut f| f.write_all(&serde_json::to_vec(list).unwrap_or_default())).map_err(|e| e.to_string())
}

/// Unfinished emails, newest first.
#[tauri::command]
pub fn mail_drafts() -> Vec<Draft> {
    let mut list = read_drafts();
    list.sort_by_key(|d| std::cmp::Reverse(d.saved));
    list
}

/// Keeps (or updates) a draft; returns its id.
#[tauri::command]
pub fn mail_draft_save(mut draft: Draft) -> Result<String, String> {
    attachments_of(&draft.attachments)?;
    let mut list = read_drafts();
    if draft.id.is_empty() {
        draft.id = format!(
            "{:x}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        );
    }
    draft.saved =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    list.retain(|d| d.id != draft.id);
    let id = draft.id.clone();
    list.push(draft);
    write_drafts(&list)?;
    Ok(id)
}

#[tauri::command]
pub fn mail_draft_delete(id: String) -> Result<(), String> {
    let mut list = read_drafts();
    list.retain(|d| d.id != id);
    write_drafts(&list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_are_decoded_and_limited() {
        let ok = AttachIn { name: "a.txt".into(), mime: String::new(), data: "aGVsbG8=".into() };
        let got = attachments_of(std::slice::from_ref(&ok)).unwrap();
        assert_eq!((got[0].data.as_slice(), got[0].mime.as_str()), (&b"hello"[..], "application/octet-stream"));
        let bad = AttachIn { data: "not base64!".into(), ..ok.clone() };
        assert!(attachments_of(&[bad]).is_err());
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 13 * 1024 * 1024]);
        let two = AttachIn { data: big, ..ok };
        assert!(attachments_of(&[two.clone(), two]).unwrap_err().contains("25 MB"));
    }
}

/// Answers the invitation in an email ("accept", "maybe" or "decline"):
/// tells the organizer (an iCalendar reply, which Outlook, Google and Apple
/// calendars read), and puts the meeting in your calendar, or takes it out.
#[tauri::command]
pub async fn mail_answer_invite(
    mail: State<'_, Mail>,
    chat: State<'_, crate::host::Chat>,
    account: String,
    folder: String,
    uid: u32,
    answer: String,
) -> Result<(), String> {
    let cal = &chat.cal;
    use xenify_cal::event::{Answer, Person};
    let acct = mail.account(&account)?;
    let raw = mail.store(&account)?.raw(&folder, uid).map_err(|e| e.to_string())?.ok_or("Open the email first.")?;
    let m = message::parse(&raw).ok_or("This email couldn't be read.")?;
    let (invite, out) = invite_of(&m, &mail.emails(), cal).ok_or("There's no invitation in this email.")?;
    let answer = match answer.as_str() {
        "accept" => Answer::Accepted,
        "maybe" => Answer::Tentative,
        _ => Answer::Declined,
    };
    let event = invite.event().clone();
    // Tell the organizer (when there's one to tell).
    if let Some(org) = &event.organizer {
        let me = invite.me(&mail.emails()).cloned().unwrap_or(Person {
            email: acct.email.clone(),
            name: acct.name.clone(),
            ..Person::default()
        });
        let ics = xenify_cal::itip::reply(&event, &me, answer);
        let text = format!(
            "{} {}.",
            if acct.name.is_empty() { &acct.email } else { &acct.name },
            match answer {
                Answer::Accepted => "will come",
                Answer::Tentative => "might come",
                _ => "won't come",
            }
        );
        let reply = Outgoing {
            from_name: acct.name.clone(),
            from_email: acct.email.clone(),
            to: org.email.clone(),
            subject: xenify_cal::itip::reply_subject(answer, &event.summary),
            html: html_of(&text),
            text,
            calendar: Some(("REPLY".into(), ics)),
            ..Outgoing::default()
        };
        let msg = send::build(&reply)?;
        let password = acct.password().ok_or("The email password isn't saved on this computer.")?;
        let a = acct.clone();
        tauri::async_runtime::spawn_blocking(move || send::send(&a, &password, &msg))
            .await
            .map_err(|e| e.to_string())??;
    }
    // Your calendar.
    if answer == Answer::Declined {
        cal.delete_uid(&event.uid)?;
        chat.calendar_changed();
        return Ok(());
    }
    let link = serde_json::json!({ "type": "mail", "account": account, "folder": folder, "uid": uid, "label": format!("the invitation “{}”", event.summary) });
    let notes = [event.location.as_str(), event.url.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let saved = xenify_chat_core::Event {
        id: cal.by_uid(&event.uid)?.map(|e| e.id).unwrap_or(0),
        title: event.summary.clone(),
        start: out.start,
        end: out.end.max(out.start),
        all_day: out.all_day,
        notes,
        kind: "event".into(),
        link: link.to_string(),
        remind: Some(event.alarms.first().copied().unwrap_or(10)),
        repeat: event.rrule.clone().unwrap_or_default(),
        ical_uid: event.uid.clone(),
        ..xenify_chat_core::Event::default()
    };
    cal.save(&saved)?;
    chat.calendar_changed();
    Ok(())
}

/// Takes a cancelled meeting out of your calendar.
#[tauri::command]
pub fn mail_forget_invite(chat: State<'_, crate::host::Chat>, uid: String) -> Result<(), String> {
    chat.cal.delete_uid(&uid)?;
    chat.calendar_changed();
    Ok(())
}
