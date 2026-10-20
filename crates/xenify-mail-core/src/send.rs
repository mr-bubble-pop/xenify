//! Sending: turning a written message into MIME (plain text plus HTML,
//! pictures as inline parts, attachments) and handing it to the account's
//! SMTP server. A copy goes into the Sent folder over IMAP.

use crate::account::Account;
use crate::imap::{Client, Role, Security};
use crate::message::Attachment;
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use std::time::Duration;

/// A message ready to send.
#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub from_name: String,
    pub from_email: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub text: String,
    /// A full HTML document (from the editor).
    pub html: String,
    /// The Message-ID of the message replied to.
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<Attachment>,
    /// A meeting invitation or answer: (method, iCalendar text).
    pub calendar: Option<(String, String)>,
}

/// Splits "Sam Lee <sam@x.org>, \"Lee, Ana\" <ana@x.org>; bo@x.org" into
/// mailboxes, saying which one is wrong.
pub fn parse_list(list: &str) -> Result<Vec<Mailbox>, String> {
    let mut out = Vec::new();
    for part in split_addresses(list) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mb: Mailbox =
            part.parse().map_err(|_| format!("\u{201c}{part}\u{201d} isn't a complete email address."))?;
        out.push(mb);
    }
    Ok(out)
}

/// Splits on commas and semicolons outside quotes and angle brackets.
fn split_addresses(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut angle) = (Vec::new(), String::new(), false, false);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

/// The editor's HTML made ready for mail programs: style rules applied
/// inline (many ignore `<style>`), and embedded pictures turned into inline
/// parts (`cid:`), which Gmail and others show while they block `data:`.
pub fn email_html(editor_html: &str) -> (String, Vec<(String, String, Vec<u8>)>) {
    use base64::Engine;
    let opts = xenify_safe_html::Options { remote_images: true, max_inline_image: 25 * 1024 * 1024 };
    let clean = xenify_safe_html::clean(editor_html, &opts, &|src| Some(src.to_string()));
    let mut html = String::new();
    let mut parts = Vec::new();
    let mut rest = clean.html.as_str();
    while let Some(i) = rest.find("src=\"data:") {
        html.push_str(&rest[..i]);
        let after = &rest[i + 5..];
        let end = after.find('"').unwrap_or(after.len());
        let data_url = &after[..end];
        let mime = data_url.trim_start_matches("data:").split(';').next().unwrap_or("image/png").to_string();
        let bytes = data_url
            .split_once("base64,")
            .and_then(|(_, b)| base64::engine::general_purpose::STANDARD.decode(b).ok())
            .unwrap_or_default();
        let cid = format!("picture{}@xenify", parts.len() + 1);
        html.push_str(&format!("src=\"cid:{cid}\""));
        parts.push((cid, mime, bytes));
        rest = &after[(end + 1).min(after.len())..];
    }
    html.push_str(rest);
    let doc = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"></head><body>{html}</body></html>"
    );
    (doc, parts)
}

/// Builds the MIME message.
pub fn build(o: &Outgoing) -> Result<Message, String> {
    let from: Mailbox = format!("{} <{}>", quote_name(&o.from_name), o.from_email.trim())
        .parse()
        .or_else(|_| o.from_email.trim().parse())
        .map_err(|_| format!("Your address \u{201c}{}\u{201d} isn't valid.", o.from_email))?;
    let to = parse_list(&o.to)?;
    let cc = parse_list(&o.cc)?;
    let bcc = parse_list(&o.bcc)?;
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err("Add at least one recipient in To.".into());
    }
    let mut b = Message::builder().from(from).subject(o.subject.trim()).date_now();
    for m in to {
        b = b.to(m);
    }
    for m in cc {
        b = b.cc(m);
    }
    for m in bcc {
        b = b.bcc(m);
    }
    let domain = o.from_email.rsplit('@').next().unwrap_or("xenify.invalid");
    b = b.message_id(Some(format!("<{}@{domain}>", unique())));
    if let Some(id) = &o.in_reply_to {
        b = b.in_reply_to(angle(id));
    }
    if !o.references.is_empty() {
        b = b.references(o.references.iter().map(|r| angle(r)).collect::<Vec<_>>().join(" "));
    }
    b = b.user_agent("Xenify Mail".into());

    let (html, pictures) = email_html(&o.html);
    let mut alternative =
        MultiPart::alternative().singlepart(SinglePart::plain(o.text.clone())).singlepart(SinglePart::html(html));
    // Invitations: the calendar as a third version of the body (what Outlook
    // and Google act on) and as an .ics attachment (for everyone else).
    let mut ics_file = None;
    if let Some((method, ics)) = &o.calendar {
        let ct = ContentType::parse(&format!("text/calendar; method={method}; charset=UTF-8"))
            .map_err(|e| format!("Couldn't build the invitation: {e}"))?;
        alternative = alternative.singlepart(SinglePart::builder().header(ct).body(ics.clone()));
        ics_file = Some(ics.clone());
    }
    let octet = || ContentType::parse("application/octet-stream").expect("valid");
    let body = if pictures.is_empty() {
        alternative
    } else {
        let mut related = MultiPart::related().multipart(alternative);
        for (cid, mime, bytes) in pictures {
            let ct = ContentType::parse(&mime).unwrap_or_else(|_| octet());
            related = related.singlepart(lettre::message::Attachment::new_inline(cid).body(bytes, ct));
        }
        related
    };
    let body = if o.attachments.is_empty() && ics_file.is_none() {
        body
    } else {
        let mut mixed = MultiPart::mixed().multipart(body);
        for a in &o.attachments {
            let ct = ContentType::parse(&a.mime).unwrap_or_else(|_| octet());
            mixed = mixed.singlepart(lettre::message::Attachment::new(a.name.clone()).body(a.data.clone(), ct));
        }
        if let Some(ics) = ics_file {
            let ct = ContentType::parse("application/ics").expect("valid");
            mixed = mixed.singlepart(lettre::message::Attachment::new("invite.ics".into()).body(ics.into_bytes(), ct));
        }
        mixed
    };
    b.multipart(body).map_err(|e| format!("Couldn't build the message: {e}"))
}

fn quote_name(name: &str) -> String {
    let n = name.trim().replace(['"', '\\', '\r', '\n'], "");
    if n.is_empty() { String::new() } else { format!("\"{n}\"") }
}

fn angle(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') { id.to_string() } else { format!("<{id}>") }
}

/// A random Message-ID part. (lettre's own would include this computer's
/// name, which recipients don't need to see.)
fn unique() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{t:x}.{:016x}", fastrand::u64(..))
}

/// Sends `m` through the account's SMTP server.
pub fn send(account: &Account, password: &str, m: &Message) -> Result<(), String> {
    let s = &account.smtp;
    let creds = Credentials::new(account.user.clone(), password.to_string());
    let builder = match s.security {
        Security::Tls => SmtpTransport::relay(&s.host).map_err(|e| e.to_string())?,
        Security::StartTls => SmtpTransport::starttls_relay(&s.host).map_err(|e| e.to_string())?,
        Security::None => {
            if !matches!(s.host.as_str(), "127.0.0.1" | "localhost" | "::1") {
                return Err("An unencrypted connection is only allowed to a server on this computer.".into());
            }
            SmtpTransport::builder_dangerous(&s.host)
        }
    };
    let transport = builder.port(s.port).credentials(creds).timeout(Some(Duration::from_secs(60))).build();
    transport.send(m).map(|_| ()).map_err(|e| {
        let text = e.to_string();
        if text.contains("535") || text.to_ascii_lowercase().contains("authentication") {
            format!("The mail server didn't accept the password for sending ({text}).")
        } else {
            format!("Couldn't send: {text}")
        }
    })
}

/// Puts a copy of a sent message in the account's Sent folder. Gmail does
/// this by itself, so it's skipped there.
pub fn save_sent_copy(account: &Account, password: &str, raw: &[u8]) -> Result<(), String> {
    if account.smtp.host.ends_with("gmail.com") || account.smtp.host.ends_with("googlemail.com") {
        return Ok(());
    }
    let mut c = Client::connect(&account.imap).map_err(|e| e.to_string())?;
    c.login(&account.user, password).map_err(|e| e.to_string())?;
    let folders = c.list().map_err(|e| e.to_string())?;
    let Some(sent) = folders.iter().find(|f| f.role == Some(Role::Sent)) else {
        return Err("This account has no Sent folder, so no copy was kept.".into());
    };
    c.append(&sent.wire, &["\\Seen"], raw).map_err(|e| e.to_string())?;
    c.logout();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitations_carry_the_calendar_twice() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:m1\r\nSUMMARY:Lunch\r\nDTSTART:20261022T120000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let o = Outgoing {
            from_email: "sam@x.org".into(),
            to: "ana@x.org".into(),
            subject: "Invitation: Lunch".into(),
            text: "Lunch on Thursday".into(),
            html: "<p>Lunch on Thursday</p>".into(),
            calendar: Some(("REQUEST".into(), ics.into())),
            ..Default::default()
        };
        let raw = String::from_utf8(build(&o).unwrap().formatted()).unwrap();
        assert!(raw.contains("text/calendar; method=REQUEST"), "{raw}");
        assert!(raw.contains("filename=\"invite.ics\""), "{raw}");
        let m = crate::message::parse(raw.as_bytes()).unwrap();
        assert!(m.calendar.unwrap().contains("UID:m1"));
    }

    #[test]
    fn address_lists_split_and_explain_mistakes() {
        let l = parse_list("Sam Lee <sam@x.org>, \"Lee, Ana\" <ana@x.org>; bo@x.org").unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[1].name.as_deref(), Some("Lee, Ana"));
        assert_eq!(parse_list("bo@").unwrap_err(), "\u{201c}bo@\u{201d} isn't a complete email address.");
        assert!(parse_list("  ").unwrap().is_empty());
    }

    #[test]
    fn messages_have_text_html_threading_and_inline_pictures() {
        let o = Outgoing {
            from_name: "Sam Lee".into(),
            from_email: "sam@x.org".into(),
            to: "Ana <ana@x.org>".into(),
            subject: "Re: Lunch".into(),
            text: "Yes please.".into(),
            html: "<html><head><style>.big { font-weight: bold }</style></head><body><p class=\"big\">Yes please.</p><img src=\"data:image/png;base64,iVBORw0KGgo=\"></body></html>".into(),
            in_reply_to: Some("lunch-1@x.org".into()),
            references: vec!["<lunch-1@x.org>".into()],
            attachments: vec![Attachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"hi".to_vec() }],
            ..Default::default()
        };
        let raw = String::from_utf8(build(&o).unwrap().formatted()).unwrap();
        for want in [
            "From: \"Sam Lee\" <sam@x.org>",
            "To: Ana <ana@x.org>",
            "Subject: Re: Lunch",
            "In-Reply-To: <lunch-1@x.org>",
            "References: <lunch-1@x.org>",
            "multipart/alternative",
            "multipart/related",
            "multipart/mixed",
            "Content-ID: <picture1@xenify>",
            "filename=\"notes.txt\"",
        ] {
            assert!(raw.contains(want), "{want} missing in:\n{raw}");
        }
        let parsed = crate::message::parse(raw.as_bytes()).unwrap();
        let html = parsed.html.unwrap();
        assert!(html.contains("font-weight:bold"), "styles inlined: {html}");
        assert!(html.contains("cid:picture1@xenify") && !html.contains("data:image"), "{html}");
        assert_eq!(parsed.text.trim(), "Yes please.");
        assert_eq!(parsed.attachments.len(), 1);
        assert!(
            build(&Outgoing { from_email: "sam@x.org".into(), ..Default::default() })
                .unwrap_err()
                .contains("recipient")
        );
    }
}
