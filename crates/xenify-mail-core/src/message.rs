//! A mail message read from MIME (an `.eml` file now, IMAP later): who it's
//! from, the body, the pictures inside it and its attachments.

use base64::Engine;
use mail_parser::{MessageParser, MimeHeaders, PartType};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Message {
    /// "Sam Lee <sam@example.com>".
    pub from: String,
    /// Just the address, for "always load pictures from this sender".
    pub from_address: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    /// As written in the message ("2026-10-16T09:41:00+01:00").
    pub date: String,
    /// The HTML body as sent (cleaned before it's shown), if there is one.
    pub html: Option<String>,
    /// The plain text body (or text made from the HTML).
    pub text: String,
    /// Pictures referred to as `cid:…`, as `data:` URLs.
    pub inline: HashMap<String, String>,
    pub attachments: Vec<Attachment>,
    /// A meeting invitation (`text/calendar`), for later.
    pub calendar: Option<String>,
    pub message_id: String,
    /// The thread so far (References), for replies.
    pub references: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// Biggest picture embedded as `data:` for display (bigger ones show as
/// attachments only).
const MAX_INLINE: usize = 8 * 1024 * 1024;

pub fn parse(raw: &[u8]) -> Option<Message> {
    let m = MessageParser::default().parse(raw)?;
    let list = |a: Option<&mail_parser::Address>| {
        a.map(|a| a.iter().map(show_addr).collect::<Vec<_>>().join(", ")).unwrap_or_default()
    };
    let mut msg = Message {
        from: list(m.from()),
        from_address: m.from().and_then(|a| a.first()).and_then(|a| a.address()).unwrap_or("").to_ascii_lowercase(),
        to: list(m.to()),
        cc: list(m.cc()),
        subject: m.subject().unwrap_or("").to_string(),
        date: m.date().map(|d| d.to_rfc3339()).unwrap_or_default(),
        message_id: m.message_id().unwrap_or("").to_string(),
        references: match m.references() {
            mail_parser::HeaderValue::Text(t) => vec![t.to_string()],
            mail_parser::HeaderValue::TextList(v) => v.iter().map(|t| t.to_string()).collect(),
            _ => Vec::new(),
        },
        ..Default::default()
    };
    msg.html = m.html_bodies().find(|p| p.is_text_html()).and_then(|p| p.text_contents()).map(str::to_string);
    msg.text = m.body_text(0).map(|t| t.into_owned()).unwrap_or_default();

    let mut used_cids = Vec::new();
    for (i, part) in m.parts.iter().enumerate() {
        let mime = part
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("")).to_ascii_lowercase())
            .unwrap_or_default();
        if mime == "text/calendar" && msg.calendar.is_none() {
            msg.calendar = part.text_contents().map(str::to_string);
        }
        if let Some(cid) = part.content_id()
            && mime.starts_with("image/")
            && part.contents().len() <= MAX_INLINE
        {
            let cid = cid.trim_matches(['<', '>']).to_string();
            let data = base64::engine::general_purpose::STANDARD.encode(part.contents());
            msg.inline.insert(format!("cid:{cid}"), format!("data:{mime};base64,{data}"));
            if msg.html.as_ref().is_some_and(|h| h.contains(&cid)) {
                used_cids.push(i as u32);
            }
        }
    }
    for &i in &m.attachments {
        let part = &m.parts[i as usize];
        // Pictures shown in the body aren't listed again as attachments.
        if used_cids.contains(&i) || matches!(part.body, PartType::Multipart(_)) {
            continue;
        }
        let mime = part
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("")).to_ascii_lowercase())
            .unwrap_or_else(|| "application/octet-stream".into());
        let name = part
            .attachment_name()
            .map(safe_file_name)
            .unwrap_or_else(|| if mime == "text/calendar" { "invite.ics".into() } else { "attachment".into() });
        msg.attachments.push(Attachment { name, mime, data: part.contents().to_vec() });
    }
    Some(msg)
}

fn show_addr(a: &mail_parser::Addr) -> String {
    match (a.name(), a.address()) {
        (Some(n), Some(e)) if !n.is_empty() => format!("{n} <{e}>"),
        (_, Some(e)) => e.to_string(),
        (Some(n), None) => n.to_string(),
        _ => String::new(),
    }
}

/// An attachment's name made safe to save: no folders, no control characters.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() { "attachment".into() } else { clean.chars().take(120).collect() }
}

impl Message {
    /// "On Fri 16 Oct 2026, 09:41 (+01:00), Sam Lee <sam@…> wrote:" for replies.
    pub fn quote_header(&self) -> String {
        if self.date.is_empty() {
            format!("{} wrote:", self.from)
        } else {
            format!("On {}, {} wrote:", nice_date(&self.date), self.from)
        }
    }

    /// "Re: …" without piling up "Re: Re:".
    pub fn reply_subject(&self) -> String {
        prefixed("Re", &self.subject)
    }

    pub fn forward_subject(&self) -> String {
        prefixed("Fwd", &self.subject)
    }
}

fn prefixed(p: &str, subject: &str) -> String {
    let lower = subject.trim_start().to_ascii_lowercase();
    if lower.starts_with(&format!("{}:", p.to_ascii_lowercase())) {
        subject.trim().to_string()
    } else {
        format!("{p}: {}", subject.trim())
    }
}

/// "2026-10-15T09:41:00+01:00" → "Thu 15 Oct 2026, 09:41 (+01:00)".
pub fn nice_date(rfc3339: &str) -> String {
    let parse = || -> Option<String> {
        let (y, mo, d) = (
            rfc3339.get(0..4)?.parse::<i32>().ok()?,
            rfc3339.get(5..7)?.parse::<usize>().ok()?,
            rfc3339.get(8..10)?.parse::<u32>().ok()?,
        );
        let time = rfc3339.get(11..16)?;
        let zone =
            rfc3339.get(19..).filter(|z| !z.is_empty() && *z != "Z").map(|z| format!(" ({z})")).unwrap_or_default();
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        // Day of the week (Sakamoto's method).
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let yy = if mo < 3 { y - 1 } else { y };
        let dow = (yy + yy / 4 - yy / 100 + yy / 400 + T[mo.checked_sub(1)?] + d as i32).rem_euclid(7) as usize;
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        Some(format!("{} {d} {} {y}, {time}{zone}", DAYS[dow], MONTHS.get(mo - 1)?))
    };
    parse().unwrap_or_else(|| rfc3339.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EML: &str = "From: \"Sam Lee\" <Sam@Example.com>\r\n\
To: You <you@example.org>\r\n\
Subject: Totals\r\n\
Date: Thu, 15 Oct 2026 09:41:00 +0100\r\n\
Message-ID: <abc@example.com>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"MIX\"\r\n\
\r\n\
--MIX\r\n\
Content-Type: multipart/related; boundary=\"REL\"\r\n\
\r\n\
--REL\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hello <img src=\"cid:logo@x\"></p>\r\n\
--REL\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@x>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--REL--\r\n\
--MIX\r\n\
Content-Type: application/pdf; name=\"../../report.pdf\"\r\n\
Content-Disposition: attachment; filename=\"../../report.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0=\r\n\
--MIX--\r\n";

    #[test]
    fn reads_headers_body_inline_pictures_and_attachments() {
        let m = parse(EML.as_bytes()).unwrap();
        assert_eq!(m.from, "Sam Lee <Sam@Example.com>");
        assert_eq!(m.from_address, "sam@example.com");
        assert_eq!(m.subject, "Totals");
        assert!(m.html.as_deref().unwrap().contains("cid:logo@x"));
        assert!(m.inline["cid:logo@x"].starts_with("data:image/png;base64,"));
        assert_eq!(m.attachments.len(), 1, "the logo isn't listed as an attachment");
        assert_eq!(m.attachments[0].name, "report.pdf", "no folder tricks");
        assert_eq!(m.reply_subject(), "Re: Totals");
        assert_eq!(prefixed("Re", "RE: Totals"), "RE: Totals");
    }
}
