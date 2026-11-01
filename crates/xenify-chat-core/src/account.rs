//! Signing in to the Xenify server, the same on every device: the user
//! name (or email address) first ([`start`]), then the password; or, for an
//! account an admin added that has no password yet, the code or link the
//! server made (a welcome or a password reset), which picks the password. A server may ask for a second step after the password (a
//! code from an authenticator app, an email or a backup code), or for one
//! to be set up first.

use crate::Fetch;
use serde::Serialize;
use serde_json::json;
use xenify_chat_proto::{Session, User};
/// The second step's types, for the apps.
pub use xenify_server_proto::{AppSecret, SecondStep, SecondStepState};
use xenify_server_proto::{LinkCheck, LinkInfo, LinkUse, Login, LoginStart};
pub use xenify_server_proto::{LoginNext, LoginStep};

/// What signing in answered.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum SignedIn {
    /// `remember` (after "Remember this computer"): send it with the next
    /// sign-in to skip the second step for 30 days.
    Session { session: Session, remember: Option<String> },
    /// The password was right; the second step next (see [`second_code`]),
    /// or with `setup`, setting one up ([`setup_app`], [`setup_email`]).
    Step(SecondStep),
}

fn unexpected() -> String {
    "The server sent something unexpected.".into()
}

fn signed_in(v: serde_json::Value) -> Result<(Session, Option<String>), String> {
    let s: xenify_server_proto::Session = serde_json::from_value(v).map_err(|_| unexpected())?;
    let a = s.account;
    let session = Session {
        token: s.token,
        user: User { id: a.id, username: a.username, display_name: a.display_name, app: false, email: a.email },
    };
    Ok((session, s.remember))
}

/// The first step: what comes after the user name. A server from before
/// this step asks for the password, as always.
pub async fn start(fetch: &impl Fetch, username: &str) -> Result<LoginStep, String> {
    let body = serde_json::to_value(LoginStart { username: username.trim().into() }).expect("a name serialises");
    match fetch.request("", "/v1/login/start", Some(body)).await {
        Ok(v) => serde_json::from_value(v).map_err(|_| unexpected()),
        Err(e) if e.contains("404") || e.contains("405") => {
            Ok(LoginStep { next: LoginNext::Password, can_email: false })
        }
        Err(e) => Err(e),
    }
}

/// "Forgot password?", or "Email me a link" for an account without one
/// yet: the server emails a link if the account has an address.
pub async fn forgot(fetch: &impl Fetch, username: &str) -> Result<(), String> {
    fetch.request("", "/v1/forgot", Some(json!({ "username": username.trim() }))).await.map(|_| ())
}

/// Signs in. With `link` (`username`'s code, emailed or from an admin, or
/// an older link's `<id>.<secret>`, see [`parse_link`]) it picks the
/// password instead. `remember`: the token from an earlier second step.
pub async fn account(
    fetch: &impl Fetch,
    username: &str,
    password: &str,
    link: &str,
    device: &str,
    remember: &str,
) -> Result<SignedIn, String> {
    let body = if link.is_empty() {
        let login = Login {
            username: username.trim().into(),
            password: password.into(),
            device: device.into(),
            remember: remember.into(),
        };
        serde_json::to_value(login)
    } else {
        let use_link = LinkUse {
            link: link.into(),
            username: username.trim().to_lowercase(),
            password: password.into(),
            device: device.into(),
        };
        serde_json::to_value(use_link)
    };
    let path = if link.is_empty() { "/v1/login" } else { "/v1/link/use" };
    let v = fetch.request("", path, Some(body.expect("a sign-in serialises"))).await?;
    if v.get("ticket").is_some() {
        return serde_json::from_value(v).map(SignedIn::Step).map_err(|_| unexpected());
    }
    let (session, remember) = signed_in(v)?;
    Ok(SignedIn::Session { session, remember })
}

/// The second step's code (`method` "app", "email" or "backup"); with
/// `remember`, the session comes with a token that skips it next time.
pub async fn second_code(
    fetch: &impl Fetch,
    ticket: &str,
    method: &str,
    code: &str,
    remember: bool,
    device: &str,
) -> Result<(Session, Option<String>), String> {
    let body =
        json!({ "ticket": ticket, "method": method, "code": code.trim(), "remember": remember, "device": device });
    signed_in(fetch.request("", "/v1/login/code", Some(body)).await?)
}

/// "Email me a code" during the second step.
pub async fn email_code(fetch: &impl Fetch, ticket: &str) -> Result<(), String> {
    fetch.request("", "/v1/login/email", Some(json!({ "ticket": ticket }))).await.map(|_| ())
}

// Setting a second step up: while signing in (`ticket`, when the server
// requires one), or signed in (`ticket` "": the fetch's session).

/// A new authenticator key to scan (or type).
pub async fn setup_app(fetch: &impl Fetch, ticket: &str) -> Result<AppSecret, String> {
    let v = fetch.request("", "/v1/mfa/app/start", Some(json!({ "ticket": ticket }))).await?;
    serde_json::from_value(v).map_err(|_| unexpected())
}

/// Emails a code for setting up codes by email.
pub async fn setup_email(fetch: &impl Fetch, ticket: &str) -> Result<(), String> {
    fetch.request("", "/v1/mfa/email", Some(json!({ "ticket": ticket }))).await.map(|_| ())
}

/// Turns the second step on with a code from `method` ("app" or "email"):
/// the backup codes (shown once) and, while signing in, the session.
pub async fn setup_finish(
    fetch: &impl Fetch,
    ticket: &str,
    method: &str,
    code: &str,
    remember: bool,
) -> Result<(Vec<String>, Option<(Session, Option<String>)>), String> {
    let path = if method == "app" { "/v1/mfa/app/finish" } else { "/v1/mfa/email" };
    let body = json!({ "ticket": ticket, "code": code.trim(), "remember": remember });
    let mut v = fetch.request("", path, Some(body)).await?;
    let codes = serde_json::from_value(v["backup_codes"].take()).map_err(|_| unexpected())?;
    let session = match v["session"].take() {
        serde_json::Value::Null => None,
        s => Some(signed_in(s)?),
    };
    Ok((codes, session))
}

/// Signed in: whether the second step is on, and what setting it up may use.
pub async fn second_step(fetch: &impl Fetch) -> Result<SecondStepState, String> {
    serde_json::from_value(fetch.request("", "/v1/me/mfa", None).await?).map_err(|_| unexpected())
}

/// Turns the second step off (refused when the server requires it).
pub async fn second_step_off(fetch: &impl Fetch, password: &str) -> Result<(), String> {
    fetch.request("", "/v1/me/mfa/off", Some(json!({ "password": password }))).await.map(|_| ())
}

/// New backup codes; the old ones stop working.
pub async fn new_backup_codes(fetch: &impl Fetch, password: &str) -> Result<Vec<String>, String> {
    let v = fetch.request("", "/v1/me/mfa/backup", Some(json!({ "password": password }))).await?;
    serde_json::from_value(v["backup_codes"].clone()).map_err(|_| unexpected())
}

/// What `username`'s code (or an older link) is for (welcome or reset)
/// and whose it is, before using it.
pub async fn link_info(fetch: &impl Fetch, link: &str, username: &str) -> Result<LinkInfo, String> {
    let body = serde_json::to_value(LinkCheck { link: link.into(), username: username.trim().to_lowercase() })
        .expect("a link serialises");
    let v = fetch.request("", "/v1/link", Some(body)).await?;
    serde_json::from_value(v).map_err(|_| "The server sent something unexpected.".to_string())
}

/// A link the server made, as pasted (`https://server/join/#<id>.<secret>`)
/// or opened (`xenify://join?link=<that, URL-encoded>`): (the server, the
/// `<id>.<secret>`).
pub fn parse_link(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let link = match text.strip_prefix("xenify://join") {
        Some(rest) => {
            let query = rest.trim_start_matches('/').strip_prefix('?')?;
            unescape(query.split('&').find_map(|p| p.strip_prefix("link="))?)
        }
        None => text.to_string(),
    };
    let (server, id) = link.split_once("/join/#").or_else(|| link.split_once("/join#"))?;
    let server = server.trim_end_matches('/');
    let ok = (server.starts_with("http://") || server.starts_with("https://")) && server.len() > 8;
    let (a, b) = id.trim().split_once('.')?;
    (ok && !a.is_empty() && !b.is_empty()).then(|| (server.to_string(), id.trim().to_string()))
}

/// `%xx` and `+` in a URL's query, decoded.
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let want = Some(("https://office.example.org".to_string(), "Ab3.xyz9".to_string()));
        assert_eq!(parse_link(" https://office.example.org/join/#Ab3.xyz9 "), want);
        assert_eq!(parse_link("xenify://join?link=https%3A%2F%2Foffice.example.org%2Fjoin%2F%23Ab3.xyz9"), want);
        assert_eq!(parse_link("xenify://join/?x=1&link=https://office.example.org/join/%23Ab3.xyz9"), want);
        assert_eq!(
            parse_link("http://10.0.0.2:7700/join/#a.b"),
            Some(("http://10.0.0.2:7700".to_string(), "a.b".to_string()))
        );
        assert_eq!(parse_link("https://office.example.org/join/#nodot"), None);
        assert_eq!(parse_link("ftp://x.org/join/#a.b"), None);
        assert_eq!(parse_link("https://example.org"), None);
        assert_eq!(unescape("a%2"), "a%2");
    }
}
