//! Mail accounts: who you are, which servers, and finding the settings from
//! just an email address. Passwords live in the system keyring, never in the
//! settings file.

use crate::imap::{Security, Server};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    /// Your name, as recipients see it.
    pub name: String,
    /// The sign-in name (usually the email address).
    pub user: String,
    pub imap: Server,
    pub smtp: Server,
}

impl Account {
    fn secret_name(&self) -> String {
        format!("mail:{}", self.email.to_lowercase())
    }

    pub fn password(&self) -> Option<String> {
        xenify_openrouter::secrets::load(&self.secret_name()).map(|(p, _)| p)
    }

    /// Stores the password; says where ("system keyring" or a private file).
    pub fn save_password(&self, password: &str) -> Result<&'static str, String> {
        xenify_openrouter::secrets::save(&self.secret_name(), password).map(|l| l.describe())
    }

    /// Where the password is kept ("system keyring" or a private file), if
    /// it's kept at all.
    pub fn password_place(&self) -> Option<&'static str> {
        xenify_openrouter::secrets::load(&self.secret_name()).map(|(_, l)| l.describe())
    }

    pub fn forget_password(&self) {
        let _ = xenify_openrouter::secrets::delete(&self.secret_name());
    }

    /// The account's local mail copy.
    pub fn store_path(&self) -> PathBuf {
        let safe: String = self
            .email
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || "@.-_".contains(c) { c } else { '_' })
            .collect();
        xenify_wp_check::data_dir().join("mail").join(format!("{safe}.sqlite"))
    }
}

/// Server settings found for an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub imap: Server,
    pub smtp: Server,
    pub user: String,
    /// Where they came from ("built in", "Thunderbird's list", "a guess").
    pub source: &'static str,
    /// How to get an app password, if the provider needs one.
    pub help: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(Found),
    /// The provider only allows sign-in through its own login page (OAuth),
    /// which Xenify Mail doesn't do yet.
    NeedsProviderSignIn(String),
}

fn tls(host: &str, port: u16) -> Server {
    Server { host: host.into(), port, security: Security::Tls }
}

fn starttls(host: &str, port: u16) -> Server {
    Server { host: host.into(), port, security: Security::StartTls }
}

fn split(email: &str) -> Option<(&str, String)> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    (!local.is_empty() && domain.contains('.')).then(|| (local, domain.to_ascii_lowercase()))
}

/// Settings we know for big providers (no network needed).
pub fn known(email: &str) -> Option<Lookup> {
    let (local, domain) = split(email)?;
    let email = email.trim().to_string();
    let found = |imap, smtp, user: String, help| Lookup::Found(Found { imap, smtp, user, source: "built in", help });
    Some(match domain.as_str() {
        "gmail.com" | "googlemail.com" => found(
            tls("imap.gmail.com", 993),
            tls("smtp.gmail.com", 465),
            email,
            Some(
                "Gmail needs an app password: turn on 2-Step Verification in your Google Account, then create one at myaccount.google.com/apppasswords.",
            ),
        ),
        "yahoo.com" | "ymail.com" | "rocketmail.com" => found(
            tls("imap.mail.yahoo.com", 993),
            tls("smtp.mail.yahoo.com", 465),
            email,
            Some("Yahoo needs an app password: Account Info › Account Security › Generate app password."),
        ),
        "aol.com" => found(
            tls("imap.aol.com", 993),
            tls("smtp.aol.com", 465),
            email,
            Some("AOL needs an app password: Account Info › Account Security › Generate app password."),
        ),
        "icloud.com" | "me.com" | "mac.com" => found(
            tls("imap.mail.me.com", 993),
            starttls("smtp.mail.me.com", 587),
            local.to_string(),
            Some(
                "iCloud needs an app-specific password: sign in at account.apple.com › Sign-In and Security › App-Specific Passwords.",
            ),
        ),
        "fastmail.com" | "fastmail.fm" | "messagingengine.com" => found(
            tls("imap.fastmail.com", 993),
            tls("smtp.fastmail.com", 465),
            email,
            Some("Fastmail needs an app password: Settings › Privacy & Security › Manage app passwords."),
        ),
        d if d.starts_with("outlook.") || d.starts_with("hotmail.") || d.starts_with("live.") || d == "msn.com" => {
            Lookup::NeedsProviderSignIn("Microsoft".into())
        }
        _ => return None,
    })
}

/// Reads a Thunderbird autoconfig file (the format of Mozilla's ISP list and
/// of `autoconfig.<domain>`).
pub fn parse_autoconfig(xml: &str, email: &str) -> Option<Lookup> {
    let (local, domain) = split(email)?;
    let doc = roxmltree::Document::parse(xml).ok()?;
    let server = |kind: &str| {
        doc.descendants()
            .filter(|n| {
                n.has_tag_name(kind)
                    && n.attribute("type") == Some(if kind == "incomingServer" { "imap" } else { "smtp" })
            })
            .find_map(|n| {
                let get = |t: &str| n.children().find(|c| c.has_tag_name(t)).and_then(|c| c.text()).map(str::trim);
                let host = get("hostname")?.to_string();
                let port: u16 = get("port")?.parse().ok()?;
                let security = match get("socketType")? {
                    "SSL" => Security::Tls,
                    "STARTTLS" => Security::StartTls,
                    _ => return None, // never plain over the internet
                };
                let auth: Vec<&str> =
                    n.children().filter(|c| c.has_tag_name("authentication")).filter_map(|c| c.text()).collect();
                let user = get("username")
                    .unwrap_or("%EMAILADDRESS%")
                    .replace("%EMAILADDRESS%", email.trim())
                    .replace("%EMAILLOCALPART%", local)
                    .replace("%EMAILDOMAIN%", &domain);
                Some((Server { host, port, security }, user, auth.iter().all(|a| a.eq_ignore_ascii_case("OAuth2"))))
            })
    };
    let (imap, user, oauth_only) = server("incomingServer")?;
    if oauth_only {
        let name = doc
            .descendants()
            .find(|n| n.has_tag_name("displayShortName"))
            .and_then(|n| n.text())
            .unwrap_or("This provider");
        return Some(Lookup::NeedsProviderSignIn(name.trim().to_string()));
    }
    let (smtp, _, _) =
        server("outgoingServer").unwrap_or_else(|| (tls(&format!("smtp.{domain}"), 465), user.clone(), false));
    Some(Lookup::Found(Found { imap, smtp, user, source: "Thunderbird's list of providers", help: None }))
}

/// Finds settings: built in, then the provider's own autoconfig file, then
/// Mozilla's list (both over HTTPS), then a guess. Blocking (network).
pub fn look_up(email: &str) -> Option<Lookup> {
    if let Some(l) = known(email) {
        return Some(l);
    }
    let (_, domain) = split(email)?;
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(10))).build().into();
    let fetch = |url: &str| agent.get(url).call().ok()?.body_mut().read_to_string().ok();
    for url in [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={}", email.trim()),
        format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
    ] {
        if let Some(l) = fetch(&url).and_then(|xml| parse_autoconfig(&xml, email)) {
            return Some(l);
        }
    }
    Some(Lookup::Found(Found {
        imap: tls(&format!("imap.{domain}"), 993),
        smtp: tls(&format!("smtp.{domain}"), 465),
        user: email.trim().to_string(),
        source: "a guess: check these with your provider",
        help: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_providers_are_built_in() {
        let Some(Lookup::Found(g)) = known("Sam.Lee@gmail.com") else { panic!() };
        assert_eq!(g.imap, tls("imap.gmail.com", 993));
        assert_eq!(g.user, "Sam.Lee@gmail.com");
        assert!(g.help.unwrap().contains("app password"));
        let Some(Lookup::Found(i)) = known("emily@icloud.com") else { panic!() };
        assert_eq!(i.user, "emily");
        assert_eq!(i.smtp.security, Security::StartTls);
        assert_eq!(known("me@hotmail.co.uk"), Some(Lookup::NeedsProviderSignIn("Microsoft".into())));
        assert_eq!(known("me@example.org"), None);
        assert_eq!(known("not-an-address"), None);
    }

    #[test]
    fn autoconfig_files_are_read() {
        let xml = r#"<?xml version="1.0"?>
<clientConfig version="1.1"><emailProvider id="example.org"><domain>example.org</domain>
  <displayShortName>Example</displayShortName>
  <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
  <incomingServer type="imap"><hostname>mail.example.org</hostname><port>143</port><socketType>STARTTLS</socketType>
    <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication></incomingServer>
  <outgoingServer type="smtp"><hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType>
    <username>%EMAILADDRESS%</username></outgoingServer>
</emailProvider></clientConfig>"#;
        let Some(Lookup::Found(f)) = parse_autoconfig(xml, "pat@example.org") else { panic!() };
        assert_eq!(f.imap, starttls("mail.example.org", 143));
        assert_eq!(f.smtp, starttls("smtp.example.org", 587));
        assert_eq!(f.user, "pat");

        let oauth = r#"<clientConfig><emailProvider id="x"><displayShortName>BigCo</displayShortName>
  <incomingServer type="imap"><hostname>imap.bigco.test</hostname><port>993</port><socketType>SSL</socketType>
  <authentication>OAuth2</authentication></incomingServer></emailProvider></clientConfig>"#;
        assert_eq!(parse_autoconfig(oauth, "a@bigco.test"), Some(Lookup::NeedsProviderSignIn("BigCo".into())));
        // Plain (unencrypted) servers are never used.
        let plain = r#"<clientConfig><emailProvider id="x"><incomingServer type="imap"><hostname>h</hostname><port>143</port><socketType>plain</socketType></incomingServer></emailProvider></clientConfig>"#;
        assert_eq!(parse_autoconfig(plain, "a@b.test"), None);
    }
}
