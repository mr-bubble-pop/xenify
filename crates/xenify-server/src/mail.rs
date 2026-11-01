//! Email the server sends: welcome and reset links, and the admin's
//! test. Admin › Settings › Outgoing mail holds the mail server (asked for
//! the first time something needs it); without one, admins pass links on
//! themselves.
//!
//! Every email is one template in Xenify's look (light and dark), with a
//! plain-text part beside the HTML.

use crate::{Server, admin_power, db, err, failed, unavailable};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deadpool_postgres::GenericClient;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub fn routes() -> Router<Server> {
    Router::new().route("/v1/admin/mail", get(get_mail).post(set_mail)).route("/v1/admin/mail/test", post(test_mail))
}

/// The mail server, as the `mail` setting keeps it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Mail {
    pub host: String,
    pub port: u16,
    /// "starttls" (port 587), "tls" (465) or "none" (this computer only).
    pub security: String,
    pub username: String,
    pub password: String,
    /// Who the email says it's from ("Office server").
    pub from_name: String,
    pub from_address: String,
}

impl Mail {
    fn ready(&self) -> bool {
        !self.host.is_empty() && !self.from_address.is_empty()
    }
}

/// The mail server, when one is set up.
pub(crate) async fn mail(c: &impl GenericClient) -> Option<Mail> {
    let m: Mail = db::setting(c, "mail").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok())?;
    m.ready().then_some(m)
}

/// One email: a heading, what it says, and at most one button.
pub struct Email {
    pub subject: String,
    pub heading: String,
    pub lines: Vec<String>,
    pub button: Option<(String, String)>,
    /// Smaller print under the button (how long a link works, what to do if
    /// it wasn't you).
    pub small: Vec<String>,
}

/// Sends `e` to `to`. The server's name signs it.
pub(crate) async fn send(c: &impl GenericClient, to: &str, e: Email) -> Result<(), String> {
    let Some(m) = mail(c).await else { return Err("no mail server is set up".into()) };
    let server = db::setting(c, "name").await.ok().flatten().unwrap_or_else(|| "Xenify server".into());
    let address = crate::server_address(c).await;
    let (html, text) = (html(&e, &server, &address), text(&e, &server, &address));
    let to = to.to_string();
    tokio::task::spawn_blocking(move || deliver(&m, to, &e.subject, &html, &text)).await.map_err(|e| e.to_string())?
}

fn deliver(m: &Mail, to: String, subject: &str, html: &str, text: &str) -> Result<(), String> {
    use lettre::message::{Mailbox, MultiPart, SinglePart, header::ContentType};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};
    let from_name = if m.from_name.is_empty() { None } else { Some(m.from_name.clone()) };
    let from = Mailbox::new(from_name, m.from_address.parse().map_err(|_| "the From address isn't an email address")?);
    let to: Mailbox = to.parse().map_err(|_| "that isn't an email address")?;
    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::builder().header(ContentType::TEXT_PLAIN).body(text.to_string()))
                .singlepart(SinglePart::builder().header(ContentType::TEXT_HTML).body(html.to_string())),
        )
        .map_err(|e| e.to_string())?;
    let builder = match m.security.as_str() {
        "tls" => SmtpTransport::relay(&m.host).map_err(|e| e.to_string())?,
        "none" => {
            if !matches!(m.host.as_str(), "127.0.0.1" | "localhost" | "::1") {
                return Err("an unencrypted connection is only allowed to a mail server on this computer".into());
            }
            SmtpTransport::builder_dangerous(&m.host)
        }
        _ => SmtpTransport::starttls_relay(&m.host).map_err(|e| e.to_string())?,
    };
    let port = if m.port != 0 {
        m.port
    } else {
        match m.security.as_str() {
            "tls" => 465,
            "none" => 25,
            _ => 587,
        }
    };
    let mut builder = builder.port(port).timeout(Some(std::time::Duration::from_secs(30)));
    if !m.username.is_empty() {
        builder = builder.credentials(Credentials::new(m.username.clone(), m.password.clone()));
    }
    builder.build().send(&message).map(|_| ()).map_err(|e| {
        let text = e.to_string();
        if text.contains("535") || text.to_ascii_lowercase().contains("authentication") {
            format!("the mail server didn't accept the user name or password ({text})")
        } else {
            format!("couldn't send: {text}")
        }
    })
}

use xenify_util::esc;

/// The HTML part: tables and inline styles, since mail programs ignore
/// most of CSS; the dark colours for the ones that follow the system.
fn html(e: &Email, server: &str, address: &str) -> String {
    let lines: String = e
        .lines
        .iter()
        .map(|l| {
            format!(
                r#"<p style="margin:0 0 14px;font-size:15px;line-height:1.55;color:#44403c" class="x-ink2">{}</p>"#,
                esc(l)
            )
        })
        .collect();
    let button = e
        .button
        .as_ref()
        .map(|(label, url)| {
            format!(
                r#"<table role="presentation" cellspacing="0" cellpadding="0" border="0" style="margin:8px 0 18px"><tr><td style="border-radius:10px;background:#c2410c" class="x-button"><a href="{url}" style="display:inline-block;padding:12px 22px;font-size:15px;font-weight:600;color:#ffffff;text-decoration:none;border-radius:10px">{label}</a></td></tr></table>
<p style="margin:0 0 14px;font-size:12.5px;line-height:1.5;color:#78716c;word-break:break-all" class="x-faint">Or open this address: <a href="{url}" style="color:#c2410c" class="x-link">{url}</a></p>"#,
                url = esc(url),
                label = esc(label)
            )
        })
        .unwrap_or_default();
    let small: String = e
        .small
        .iter()
        .map(|l| {
            format!(
                r#"<p style="margin:0 0 8px;font-size:13px;line-height:1.5;color:#57534e" class="x-dim">{}</p>"#,
                esc(l)
            )
        })
        .collect();
    let from = if address.is_empty() { esc(server) } else { format!("{} · {}", esc(server), esc(address)) };
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark"><meta name="supported-color-schemes" content="light dark">
<title>{subject}</title>
<style>
@media (prefers-color-scheme: dark) {{
  .x-ground {{ background:#1c1917 !important; }}
  .x-sheet {{ background:#292524 !important; }}
  .x-ink {{ color:#fafaf9 !important; }}
  .x-ink2 {{ color:#e7e5e4 !important; }}
  .x-dim, .x-faint {{ color:#a8a29e !important; }}
  .x-link {{ color:#fb923c !important; }}
  .x-rule {{ border-color:#44403c !important; }}
}}
</style></head>
<body style="margin:0;padding:0;background:#efece8" class="x-ground">
<table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="background:#efece8;font-family:Figtree,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif" class="x-ground"><tr><td align="center" style="padding:32px 16px">
<table role="presentation" width="100%" cellspacing="0" cellpadding="0" border="0" style="max-width:520px">
<tr><td style="padding:0 4px 16px"><table role="presentation" cellspacing="0" cellpadding="0" border="0"><tr>
<td style="width:28px;height:28px;border-radius:8px;background:#c2410c;color:#ffffff;font-weight:800;font-size:15px;text-align:center;vertical-align:middle">X</td>
<td style="padding-left:10px;font-size:15px;font-weight:700;color:#1c1917" class="x-ink">{server}</td></tr></table></td></tr>
<tr><td style="background:#ffffff;border-radius:16px;padding:32px 32px 24px" class="x-sheet">
<h1 style="margin:0 0 16px;font-size:23px;line-height:1.25;font-weight:700;color:#1c1917" class="x-ink">{heading}</h1>
{lines}{button}<div style="border-top:1px solid #f0ede9;margin:8px 0 14px" class="x-rule"></div>{small}
</td></tr>
<tr><td style="padding:16px 8px 0;font-size:12px;line-height:1.5;color:#78716c;text-align:center" class="x-faint">Sent by {from}, a Xenify server.</td></tr>
</table></td></tr></table></body></html>"#,
        subject = esc(&e.subject),
        server = esc(server),
        heading = esc(&e.heading),
    )
}

fn text(e: &Email, server: &str, address: &str) -> String {
    let mut out = format!("{}\n\n", e.heading);
    for l in &e.lines {
        out.push_str(l);
        out.push_str("\n\n");
    }
    if let Some((label, url)) = &e.button {
        out.push_str(&format!("{label}:\n{url}\n\n"));
    }
    for l in &e.small {
        out.push_str(l);
        out.push_str("\n\n");
    }
    out.push_str(&format!(
        "-- \nSent by {server}{}, a Xenify server.\n",
        if address.is_empty() { String::new() } else { format!(" ({address})") }
    ));
    out
}

/// Sent when an admin adds someone: who added them, and how to start (the
/// email address, then a code by email, then their password).
pub(crate) fn invitation(server: &str, by: &str, name: &str, email: &str, sign_in: &str) -> Email {
    Email {
        subject: format!("You're invited to {server}"),
        heading: format!("Welcome, {name}"),
        lines: vec![
            if by.is_empty() {
                format!("You've been invited to {server}, where your team shares files, writes and draws together, and chats.")
            } else {
                format!("{by} invited you to {server}, where your team shares files, writes and draws together, and chats.")
            },
            format!("Sign in with this email address, {email}. We'll email you a code to check it's you, then you pick your password."),
        ],
        button: Some(("Sign in".into(), sign_in.into())),
        small: vec!["The Xenify apps (Write, Draw, Calc, Plan, Chat) sign in the same way.".into()],
    }
}

/// An admin reset their password: they pick a new one the same way.
pub(crate) fn reset(server: &str, name: &str, email: &str, sign_in: &str) -> Email {
    Email {
        subject: format!("New password for {server}"),
        heading: format!("Pick a new password, {name}"),
        lines: vec![
            format!("An admin reset your password for {server}."),
            format!("Sign in with this email address, {email}. We'll email you a code, then you pick a new password."),
        ],
        button: Some(("Sign in".into(), sign_in.into())),
        small: Vec::new(),
    }
}

/// A one-time code to pick a password: a new account's first, or a
/// forgotten one.
pub(crate) fn code(server: &str, name: &str, code: &str, new: bool) -> Email {
    Email {
        subject: format!("Your code for {server}: {code}"),
        heading: format!("Your code is {code}"),
        lines: vec![if new {
            format!("Hi {name}, type it where you're signing in to {server}, then pick your password.")
        } else {
            format!("Hi {name}, someone asked to pick a new password for you on {server}. Type the code where you're signing in, then pick one.")
        }],
        button: None,
        small: vec![
            "It works for 30 minutes.".into(),
            if new {
                "If you weren't expecting this, ignore this email.".into()
            } else {
                "If it wasn't you, ignore this email: your password stays as it is.".into()
            },
        ],
    }
}

/// Where an email's "Sign in" goes: the server's home, with the email
/// address filled in.
pub(crate) fn sign_in_link(base: &str, email: &str) -> String {
    let email: String = email
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b".-_@".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("{}/files/?user={email}", base.trim_end_matches('/'))
}

#[derive(Serialize)]
struct MailOut {
    #[serde(flatten)]
    mail: Mail,
    password_set: bool,
}

async fn get_mail(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let mut m: Mail =
        db::setting(&c, "mail").await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
    let password_set = !m.password.is_empty();
    m.password.clear();
    Json(MailOut { mail: m, password_set }).into_response()
}

/// Saves the mail server; an empty password keeps the one kept.
async fn set_mail(State(s): State<Server>, h: HeaderMap, Json(mut m): Json<Mail>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    m.host = m.host.trim().to_string();
    m.from_address = m.from_address.trim().to_string();
    if !["starttls", "tls", "none"].contains(&m.security.as_str()) {
        m.security = "starttls".into();
    }
    if !m.from_address.is_empty() && crate::admin::clean_email(&m.from_address).is_none_or(|e| e.is_empty()) {
        return err(StatusCode::BAD_REQUEST, "bad_email", "the From address isn't an email address");
    }
    if m.security == "none" && !m.host.is_empty() && !matches!(m.host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return err(
            StatusCode::BAD_REQUEST,
            "insecure",
            "an unencrypted connection is only allowed to a mail server on this computer",
        );
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    if m.password.is_empty() {
        let old: Mail = db::setting(&c, "mail")
            .await
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        m.password = old.password;
    }
    match db::set_setting(&c, "mail", &serde_json::to_string(&m).expect("serialises")).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => failed(),
    }
}

#[derive(Deserialize)]
struct Test {
    to: String,
}

async fn test_mail(State(s): State<Server>, h: HeaderMap, Json(t): Json<Test>) -> Response {
    let me = match admin_power(&s, &h, "admin.network").await {
        Ok(a) => a,
        Err(e) => return e,
    };
    let Ok(c) = s.pool.get().await else { return unavailable() };
    crate::base_url(&c, &h).await;
    let server = db::setting(&c, "name").await.ok().flatten().unwrap_or_default();
    let e = Email {
        subject: format!("Test email from {server}"),
        heading: "Email works".into(),
        lines: vec![format!(
            "{} sent this from the admin pages to check {server} can send email. Welcome links and password resets will look like this.",
            if me.display_name.is_empty() { &me.username } else { &me.display_name }
        )],
        button: None,
        small: vec![],
    };
    match send(&c, t.to.trim(), e).await {
        Ok(()) => Json(json!({ "sent": true })).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, "mail_failed", &e),
    }
}

#[cfg(test)]
mod tests {
    /// With XENIFY_EMAIL_SAMPLES set to a folder, writes each kind of email
    /// there as HTML, for looking at.
    #[test]
    fn emails_render() {
        let link = super::sign_in_link("https://files.example.org", "priya@example.org");
        let samples = [
            ("invitation", super::invitation("Office server", "Alex Morgan", "Priya Shah", "priya@example.org", &link), true),
            ("reset", super::reset("Office server", "Priya Shah", "priya@example.org", &link), true),
            ("code", super::code("Office server", "Priya Shah", "K7P2-9QXM", true), false),
        ];
        for (name, e, linked) in samples {
            let html = super::html(&e, "Office server", "https://files.example.org");
            assert!(html.contains(&e.heading) && (!linked || html.contains(&link)));
            let text = super::text(&e, "Office server", "https://files.example.org");
            assert!(!linked || text.contains(&link));
            if let Some(dir) = std::env::var_os("XENIFY_EMAIL_SAMPLES") {
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.html")), html).unwrap();
            }
        }
    }
}
