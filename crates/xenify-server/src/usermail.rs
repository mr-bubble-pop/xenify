//! People's own email, kept by the server, so Chat has email in the browser
//! as well as in the desktop app, the same inbox on every computer, and
//! invitations to people outside go out from any of them.
//!
//! Where the mail comes from, without anyone typing a password into Xenify:
//!
//! - **The company's mailboxes, connected once by an admin** (Admin › Email):
//!   - Microsoft 365: the company's admin lets the server use its mailboxes
//!     (an app registration with Mail.ReadWrite and Mail.Send, consented
//!     for the organisation); mail goes through Microsoft Graph.
//!   - Google Workspace: a service account the company's admin lets act for
//!     its people (domain-wide delegation, scope https://mail.google.com/);
//!     mail goes through IMAP and SMTP with its tokens.
//!   - The company's own mail server: one master login (Dovecot's
//!     `person*master`, say) the server signs in with for each person.
//!   Everyone whose email address is at the company's domain (and, if the
//!   admin chose, in one group) then has their mailbox in Chat.
//! - **A person connects their own Microsoft or Google mail** on the
//!   provider's own sign-in page (OAuth with PKCE); the server keeps the
//!   token the provider gives, which they can take back there.
//! - Last, an email account with a password (or an app password), when the
//!   admin allows it: for mail servers that have nothing better.
//!
//! Tokens, passwords and keys are kept encrypted in the database (AES-GCM,
//! with a key made from the server's own secret).
//!
//! The work is xenify-mail-core's, as in the desktop app: one background
//! thread per mailbox, and a SQLite copy of the mail in the data folder (the
//! encrypted one when the server has it), `mail/<person>/<address>.sqlite`.
//! The screens call `POST /v1/mail/<call>` with the desktop app's command
//! names and arguments, and hear what changes on `GET /v1/mail/events`.

use crate::{Server, admin_power, db, err, link_base, signed_in, unavailable};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use deadpool_postgres::GenericClient;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use xenify_mail_core::account::{self, Account, Lookup};
use xenify_mail_core::auth::{Secret, TokenSource};
use xenify_mail_core::graph;
use xenify_mail_core::imap::{Client, Role, Security, Server as MailServer};
use xenify_mail_core::message::{self, Attachment};
use xenify_mail_core::send::{self, Outgoing};
use xenify_mail_core::store::{Row, Store};
use xenify_mail_core::sync::{self, Cmd, Event, Handle};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS mail_accounts (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'password',
    account TEXT NOT NULL,
    secret TEXT NOT NULL,
    added TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, email)
);
CREATE TABLE IF NOT EXISTS mail_drafts (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    value TEXT NOT NULL,
    saved BIGINT NOT NULL,
    PRIMARY KEY (user_id, id)
);
";

/// The most that can be attached to one email, in bytes.
const MAX_ATTACHED: usize = 25 * 1024 * 1024;
/// Accounts one person may add.
const MOST_ACCOUNTS: i64 = 10;
/// Unfinished emails one person may keep, and how big they may be in all.
const MOST_DRAFTS: i64 = 200;
const MOST_DRAFT_BYTES: usize = 150 * 1024 * 1024;
/// The settings key for Admin › Email.
const SETTING: &str = "user_mail";
/// Where the providers send people back after they sign in.
const CONNECTED: &str = "/v1/mail/connected";

pub fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/mail/events", get(events))
        .route("/v1/mail/attachment", get(attachment))
        .route(CONNECTED, get(connected).post(connected_finish))
        .route("/v1/admin/usermail", get(admin_get).post(admin_set))
        .route("/v1/admin/usermail/test", post(admin_test))
        // An email with its attachments, as base64.
        .route("/v1/mail/{call}", post(call).layer(DefaultBodyLimit::max(MAX_ATTACHED * 4 / 3 + (1 << 20))))
}

// ---- Keeping secrets ---------------------------------------------------

const DB_DOWN: &str = "The server can't reach its database.";

/// The key the mail secrets are sealed with: made from a key file in the
/// server's (encrypted) data folder and the server's secret in the
/// database, so a copy of the database alone opens nothing.
#[derive(Clone)]
struct Key {
    k: [u8; 32],
}

async fn key(s: &Server, c: &impl GenericClient) -> Result<Key, String> {
    use sha2::{Digest, Sha256};
    let secret = db::setting(c, "secret")
        .await
        .map_err(|_| DB_DOWN.to_string())?
        .filter(|x| !x.is_empty())
        .ok_or("The server has no secret to lock email passwords with.")?;
    let file = file_key(&s.secure)?;
    let k = Sha256::digest([b"xenify mail key 2\0".as_slice(), &file, secret.as_bytes()].concat()).into();
    Ok(Key { k })
}

/// `mail.key` in the data folder (32 random bytes, made the first time).
fn file_key(dir: &std::path::Path) -> Result<[u8; 32], String> {
    static KEYS: LazyLock<Mutex<HashMap<PathBuf, [u8; 32]>>> = LazyLock::new(Default::default);
    let mut keys = KEYS.lock().unwrap();
    if let Some(k) = keys.get(dir) {
        return Ok(*k);
    }
    let path = dir.join("mail.key");
    let broken = || "The server's mail key file (mail.key) can't be read.".to_string();
    let read = |p: &std::path::Path| -> Result<[u8; 32], String> {
        std::fs::read(p).map_err(|_| broken())?.try_into().map_err(|_| broken())
    };
    let k = match read(&path) {
        Ok(k) => k,
        Err(_) if !path.exists() => {
            use std::io::Write;
            let _ = std::fs::create_dir_all(dir);
            let new: [u8; 32] = xenify_sync::random(32).try_into().expect("32 bytes");
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
            match o.open(&path) {
                Ok(mut f) => {
                    f.write_all(&new).and_then(|_| f.sync_all()).map_err(|_| broken())?;
                    new
                }
                // Another copy of the server made it just now.
                Err(_) => read(&path)?,
            }
        }
        Err(e) => return Err(e),
    };
    keys.insert(dir.to_path_buf(), k);
    Ok(k)
}

/// Seals `plain` for the place `aad` names (a row or a setting), so a sealed
/// value copied elsewhere doesn't open.
fn seal(key: &Key, aad: &str, plain: &str) -> String {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
    if plain.is_empty() {
        return String::new();
    }
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key.k).expect("key"));
    // A random nonce each time (96 bits; a key seals few values).
    let n: [u8; 12] = xenify_sync::random(12).try_into().expect("nonce");
    let mut buf = plain.as_bytes().to_vec();
    k.seal_in_place_append_tag(Nonce::assume_unique_for_key(n), Aad::from(aad.as_bytes()), &mut buf).expect("seal");
    let mut out = n.to_vec();
    out.extend(buf);
    base64::engine::general_purpose::STANDARD.encode(out)
}

fn unseal(key: &Key, aad: &str, sealed: &str) -> String {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
    let Ok(b) = base64::engine::general_purpose::STANDARD.decode(sealed) else { return String::new() };
    if b.len() < 12 + 16 {
        return String::new();
    }
    let Ok(n) = <[u8; 12]>::try_from(&b[..12]) else { return String::new() };
    let open = |k: &[u8; 32], aad: &[u8]| {
        let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, k).expect("key"));
        let mut rest = b[12..].to_vec();
        k.open_in_place(Nonce::assume_unique_for_key(n), Aad::from(aad), &mut rest)
            .ok()
            .and_then(|p| String::from_utf8(p.to_vec()).ok())
    };
    open(&key.k, aad.as_bytes()).unwrap_or_default()
}

/// What a person's account row's secret is sealed for.
fn row_aad(user: i64, email: &str) -> String {
    format!("mail_accounts\0{user}\0{email}")
}

// ---- Admin › Email -------------------------------------------------------

/// What the admin set. The secret fields are kept sealed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    /// The company's mailboxes: "" (none), "microsoft", "google" or "imap".
    org: String,
    /// Its email domains ("example.com, example.org").
    domains: String,
    /// Only people in this group (0: everyone at the domains).
    group: i64,
    ms_tenant: String,
    ms_client_id: String,
    ms_secret: String,
    /// The Google service account's key file (JSON).
    google_key: String,
    imap: Option<ServerIn>,
    smtp: Option<ServerIn>,
    master_user: String,
    master_password: String,
    /// How the master login is written for someone ("{email}*{master}").
    login_as: String,
    /// People may connect their own Microsoft mail (this app registration).
    personal_ms: bool,
    pm_client_id: String,
    pm_secret: String,
    /// People may connect their own Google mail (this OAuth client).
    personal_google: bool,
    pg_client_id: String,
    pg_secret: String,
    /// People may not add accounts with a password.
    no_passwords: bool,
    /// Accounts people add may use mail servers on the server's own network
    /// (or this computer). Off, those addresses are refused, so nobody can
    /// make the server knock on the doors of what's behind it.
    private_hosts: bool,
}

const SECRETS: [&str; 5] = ["ms_secret", "google_key", "master_password", "pm_secret", "pg_secret"];

impl Settings {
    fn secrets(&mut self) -> [(&'static str, &mut String); 5] {
        [
            ("ms_secret", &mut self.ms_secret),
            ("google_key", &mut self.google_key),
            ("master_password", &mut self.master_password),
            ("pm_secret", &mut self.pm_secret),
            ("pg_secret", &mut self.pg_secret),
        ]
    }
}

async fn settings(s: &Server, c: &impl GenericClient) -> Result<(Settings, Key), String> {
    let k = key(s, c).await?;
    let mut st: Settings = db::setting(c, SETTING)
        .await
        .map_err(|_| DB_DOWN.to_string())?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    for (name, f) in st.secrets() {
        *f = unseal(&k, &format!("{SETTING}\0{name}"), f);
    }
    Ok((st, k))
}

fn no_key(e: String) -> Response {
    err(StatusCode::SERVICE_UNAVAILABLE, "mail_key", &e)
}

impl Settings {
    fn domains(&self) -> Vec<String> {
        self.domains
            .split([',', ' ', ';', '\n'])
            .map(|d| d.trim().trim_start_matches('@').to_ascii_lowercase())
            .filter(|d| d.contains('.'))
            .collect()
    }

    /// The Microsoft app people connect their own mail with (the company's
    /// own registration when no other is set).
    fn personal_ms_app(&self) -> Option<(String, String, String)> {
        if !self.personal_ms {
            return None;
        }
        let (id, secret) = if self.pm_client_id.is_empty() {
            (self.ms_client_id.clone(), self.ms_secret.clone())
        } else {
            (self.pm_client_id.clone(), self.pm_secret.clone())
        };
        // Any Microsoft account with its own app; the company's people only
        // with the company's.
        let tenant = if self.pm_client_id.is_empty() && !self.ms_tenant.is_empty() {
            self.ms_tenant.clone()
        } else {
            "common".into()
        };
        (!id.is_empty()).then_some((id, secret, tenant))
    }

    fn personal_google_app(&self) -> Option<(String, String)> {
        (self.personal_google && !self.pg_client_id.is_empty())
            .then(|| (self.pg_client_id.clone(), self.pg_secret.clone()))
    }
}

async fn admin_get(State(s): State<Server>, h: HeaderMap) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let (st, _) = match settings(&s, &c).await {
        Ok(x) => x,
        Err(e) => return no_key(e),
    };
    let mut v = serde_json::to_value(&st).unwrap_or_default();
    for f in SECRETS {
        let has = v[f].as_str().is_some_and(|x| !x.is_empty());
        v[f] = json!("");
        v[format!("has_{f}")] = json!(has);
    }
    // The address to register with Microsoft and Google for people's own sign-in.
    v["redirect"] = json!(format!("{}{CONNECTED}", crate::base_url(&c, &h).await));
    Json(v).into_response()
}

/// Saves Admin › Email (an empty secret keeps the one there).
async fn admin_set(State(s): State<Server>, h: HeaderMap, Json(mut new): Json<Settings>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let (mut old, k) = match settings(&s, &c).await {
        Ok(x) => x,
        Err(e) => return no_key(e),
    };
    if !["", "microsoft", "google", "imap"].contains(&new.org.as_str()) {
        return err(StatusCode::BAD_REQUEST, "bad", "unknown kind of mailboxes");
    }
    for ((_, n), (_, o)) in new.secrets().into_iter().zip(old.secrets()) {
        if n.is_empty() {
            *n = o.clone();
        }
    }
    if new.org == "google" && !new.google_key.is_empty() && GoogleKey::read(&new.google_key).is_err() {
        return err(StatusCode::BAD_REQUEST, "bad", "that isn't a Google service account key file (JSON)");
    }
    if new.org == "imap"
        && let (Some(i), Some(m)) = (&new.imap, &new.smtp)
        && let Err(e) = i.server().and(m.server())
    {
        return err(StatusCode::BAD_REQUEST, "bad", &e);
    }
    if new.org != "" && new.domains().is_empty() {
        return err(StatusCode::BAD_REQUEST, "bad", "type the company's email domain (example.com)");
    }
    let mut keep = new.clone();
    for (name, f) in keep.secrets() {
        *f = seal(&k, &format!("{SETTING}\0{name}"), f);
    }
    if db::set_setting(&c, SETTING, &serde_json::to_string(&keep).unwrap_or_default()).await.is_err() {
        return unavailable();
    }
    drop(c);
    // Everyone's mailboxes follow the new settings.
    restart_all(&s).await;
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct TestIn {
    email: String,
}

/// Tries the company's settings with one person's mailbox.
async fn admin_test(State(s): State<Server>, h: HeaderMap, Json(t): Json<TestIn>) -> Response {
    if let Err(e) = admin_power(&s, &h, "admin.network").await {
        return e;
    }
    let Ok(c) = s.pool.get().await else { return unavailable() };
    let (st, _) = match settings(&s, &c).await {
        Ok(x) => x,
        Err(e) => return no_key(e),
    };
    drop(c);
    let email = t.email.trim().to_string();
    let Some(link) = org_link(&st, &email, "") else {
        return err(
            StatusCode::BAD_REQUEST,
            "bad",
            "that address isn't at the company's domain, or the settings aren't complete",
        );
    };
    let r = tokio::task::spawn_blocking(move || match link {
        Link::Graph(m) => graph::check(&m),
        Link::Imap { account, secret } => {
            let mut c =
                Client::connect(&account.imap).map_err(|e| format!("Couldn't reach {}: {e}", account.imap.host))?;
            c.sign_in(&account.user, &secret).map_err(|e| format!("The mail server didn't let the server in: {e}"))?;
            c.logout();
            Ok(())
        }
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, "mail", &e),
    }
}

// ---- Tokens from the providers ------------------------------------------

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// POSTs a form to a token endpoint; the answer's JSON, or why not.
fn token_post(url: &str, form: &[(&str, &str)]) -> Result<Value, String> {
    let mut r = agent().post(url).send_form(form.iter().copied()).map_err(|e| format!("Couldn't reach {url}: {e}"))?;
    let ok = r.status().is_success();
    let v: Value =
        r.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if ok && v["access_token"].is_string() {
        return Ok(v);
    }
    let why = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("it said no").to_string();
    Err(format!("The mail provider didn't give the server a token: {why}"))
}

/// A token kept until a minute before it runs out.
#[derive(Default)]
struct Cached(Mutex<Option<(String, Instant)>>);

impl Cached {
    fn get(&self, fetch: impl FnOnce() -> Result<(String, u64), String>) -> Result<String, String> {
        let mut c = self.0.lock().unwrap();
        if let Some((t, until)) = &*c
            && Instant::now() < *until
        {
            return Ok(t.clone());
        }
        let (t, secs) = fetch()?;
        *c = Some((t.clone(), Instant::now() + Duration::from_secs(secs.saturating_sub(60).max(30))));
        Ok(t)
    }
}

fn expires(v: &Value) -> u64 {
    v["expires_in"].as_u64().or_else(|| v["expires_in"].as_str().and_then(|s| s.parse().ok())).unwrap_or(3600)
}

/// Microsoft 365, the company's consent: the app's own token (client credentials).
struct MsApp {
    tenant: String,
    client_id: String,
    secret: String,
    cache: Cached,
}

impl TokenSource for MsApp {
    fn token(&self) -> Result<String, String> {
        self.cache.get(|| {
            let url = format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", self.tenant);
            let v = token_post(
                &url,
                &[
                    ("client_id", &self.client_id),
                    ("client_secret", &self.secret),
                    ("scope", "https://graph.microsoft.com/.default"),
                    ("grant_type", "client_credentials"),
                ],
            )?;
            Ok((v["access_token"].as_str().unwrap_or("").to_string(), expires(&v)))
        })
    }
}

/// A Google service account's key file.
struct GoogleKey {
    email: String,
    der: Vec<u8>,
    token_uri: String,
}

impl GoogleKey {
    fn read(json_text: &str) -> Result<GoogleKey, String> {
        let v: Value = serde_json::from_str(json_text).map_err(|_| "not JSON".to_string())?;
        let email = v["client_email"].as_str().ok_or("no client_email")?.to_string();
        let pem = v["private_key"].as_str().ok_or("no private_key")?;
        let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| "a broken private_key")?;
        ring::signature::RsaKeyPair::from_pkcs8(&der).map_err(|_| "a private_key that isn't an RSA key")?;
        // Always Google's own: the signed request lets whoever gets it act
        // as everyone at the company.
        let token_uri = "https://oauth2.googleapis.com/token".to_string();
        Ok(GoogleKey { email, der, token_uri })
    }
}

/// Google Workspace, the company's delegation: a token to act as `subject`.
struct GoogleAs {
    key: Arc<GoogleKey>,
    subject: String,
    cache: Cached,
}

impl TokenSource for GoogleAs {
    fn token(&self) -> Result<String, String> {
        self.cache.get(|| {
            let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
            let now =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let claims = json!({
                "iss": self.key.email, "sub": self.subject, "scope": "https://mail.google.com/",
                "aud": self.key.token_uri, "iat": now, "exp": now + 3600,
            });
            let msg = format!("{}.{}", b64(br#"{"alg":"RS256","typ":"JWT"}"#), b64(claims.to_string().as_bytes()));
            let kp =
                ring::signature::RsaKeyPair::from_pkcs8(&self.key.der).map_err(|_| "The Google key can't sign.")?;
            let mut sig = vec![0; kp.public().modulus_len()];
            kp.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), msg.as_bytes(), &mut sig)
                .map_err(|_| "The Google key can't sign.")?;
            let jwt = format!("{msg}.{}", b64(&sig));
            let v = token_post(
                &self.key.token_uri,
                &[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", &jwt)],
            )?;
            Ok((v["access_token"].as_str().unwrap_or("").to_string(), expires(&v)))
        })
    }
}

/// Someone's own sign-in: tokens from their refresh token (a new refresh
/// token, when the provider hands one, is kept).
struct Refresh {
    url: String,
    client_id: String,
    secret: String,
    scope: String,
    refresh: Mutex<String>,
    keep: Box<dyn Fn(String) + Send + Sync>,
    cache: Cached,
}

impl TokenSource for Refresh {
    fn token(&self) -> Result<String, String> {
        self.cache.get(|| {
            let rt = self.refresh.lock().unwrap().clone();
            let mut form = vec![
                ("client_id", self.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", rt.as_str()),
            ];
            if !self.secret.is_empty() {
                form.push(("client_secret", &self.secret));
            }
            if !self.scope.is_empty() {
                form.push(("scope", &self.scope));
            }
            let v = token_post(&self.url, &form)?;
            if let Some(new) = v["refresh_token"].as_str().filter(|n| *n != rt) {
                *self.refresh.lock().unwrap() = new.to_string();
                (self.keep)(new.to_string());
            }
            Ok((v["access_token"].as_str().unwrap_or("").to_string(), expires(&v)))
        })
    }
}

/// The providers people can connect their own mail at.
struct Provider {
    auth: String,
    token: String,
    scope: &'static str,
    imap: MailServer,
    smtp: MailServer,
}

fn provider(st: &Settings, which: &str) -> Option<(Provider, String, String)> {
    let tls = |h: &str, p| MailServer { host: h.into(), port: p, security: Security::Tls };
    match which {
        "microsoft" => {
            let (id, secret, tenant) = st.personal_ms_app()?;
            Some((
                Provider {
                    auth: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
                    token: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
                    scope: "openid email offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send",
                    imap: tls("outlook.office365.com", 993),
                    smtp: MailServer { host: "smtp.office365.com".into(), port: 587, security: Security::StartTls },
                },
                id,
                secret,
            ))
        }
        "google" => {
            let (id, secret) = st.personal_google_app()?;
            Some((
                Provider {
                    auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                    token: "https://oauth2.googleapis.com/token".into(),
                    scope: "openid email https://mail.google.com/",
                    imap: tls("imap.gmail.com", 993),
                    smtp: tls("smtp.gmail.com", 465),
                },
                id,
                secret,
            ))
        }
        _ => None,
    }
}

// ---- Guarding the server -------------------------------------------------

/// An address on the internet (not this computer, the server's own network,
/// a cloud's metadata service, or a reserved one).
fn public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    fn v4(v: Ipv4Addr) -> bool {
        let o = v.octets();
        !(v.is_private()
            || v.is_loopback()
            || v.is_link_local()
            || v.is_broadcast()
            || v.is_documentation()
            || v.is_unspecified()
            || v.is_multicast()
            || o[0] == 0
            // Shared (carrier) addresses, 100.64.0.0/10.
            || (o[0] == 100 && (o[1] & 0xc0) == 64)
            // IETF protocol assignments, 192.0.0.0/24; benchmarking, 198.18.0.0/15.
            || (o[0] == 192 && o[1] == 0 && o[2] == 0)
            || (o[0] == 198 && (o[1] & 0xfe) == 18)
            // Reserved, 240.0.0.0/4.
            || o[0] >= 240)
    }
    match ip {
        IpAddr::V4(v) => v4(v),
        IpAddr::V6(v) => {
            let s = v.segments();
            if let Some(m) = v.to_ipv4_mapped() {
                return v4(m);
            }
            // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) carry an IPv4 address.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                return v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
            }
            if s[0] == 0x2002 {
                return v4(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8));
            }
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                // IPv4-compatible (::a.b.c.d), unique local (fc00::/7),
                // link-local (fe80::/10), documentation (2001:db8::/32).
                || s[..6] == [0, 0, 0, 0, 0, 0]
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// Whether the server may connect to `server` for a person's own account:
/// every address its name has must be on the internet, unless the admin
/// allows the server's own network. Gives the checked address to connect
/// to (`None` when nothing was checked), so the connection goes exactly
/// there and not to wherever the name points a moment later.
async fn may_reach(server: &MailServer, private_ok: bool) -> Result<Option<std::net::SocketAddr>, String> {
    if private_ok {
        return Ok(None);
    }
    let host = server.host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return checked(server, vec![std::net::SocketAddr::new(ip, server.port)]);
    }
    let addrs: Vec<_> = tokio::time::timeout(Duration::from_secs(10), tokio::net::lookup_host((host, server.port)))
        .await
        .map_err(|_| format!("Couldn't find the mail server {host}."))?
        .map_err(|_| format!("Couldn't find the mail server {host}."))?
        .collect();
    checked(server, addrs)
}

/// [`may_reach`] for a mail worker's thread (blocking): asked again before
/// every connection it makes.
fn may_reach_now(server: &MailServer) -> Result<Option<std::net::SocketAddr>, String> {
    use std::net::ToSocketAddrs;
    let host = server.host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return checked(server, vec![std::net::SocketAddr::new(ip, server.port)]);
    }
    let addrs: Vec<_> = (host, server.port)
        .to_socket_addrs()
        .map_err(|_| format!("Couldn't find the mail server {host}."))?
        .collect();
    checked(server, addrs)
}

/// The address to connect to when all of `addrs` are on the internet.
fn checked(server: &MailServer, addrs: Vec<std::net::SocketAddr>) -> Result<Option<std::net::SocketAddr>, String> {
    let host = server.host.trim_start_matches('[').trim_end_matches(']');
    let Some(first) = addrs.first() else {
        return Err(format!("Couldn't find the mail server {host}."));
    };
    if addrs.iter().any(|a| !public_ip(a.ip())) {
        return Err(format!(
            "{} is on the server's own network, which email accounts can't use here. Ask your admin.",
            server.host
        ));
    }
    Ok(Some(*first))
}

/// Settings for an address: built in, then the provider's autoconfig file
/// and Mozilla's list (over HTTPS, never following a redirect, and only from
/// addresses on the internet unless the admin allows more), then a guess.
/// (Not mail-core's look_up: that one follows redirects anywhere.)
async fn look_up(email: &str, private_ok: bool) -> Option<Lookup> {
    if let Some(l) = account::known(email) {
        return Some(l);
    }
    let email = email.trim().to_string();
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    if !domain.contains('.') || !domain.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)) {
        return None;
    }
    let q: String = url::form_urlencoded::byte_serialize(email.as_bytes()).collect();
    let tls = |h: String, p| MailServer { host: h, port: p, security: Security::Tls };
    for (host, url) in [
        (format!("autoconfig.{domain}"), format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={q}")),
        ("autoconfig.thunderbird.net".to_string(), format!("https://autoconfig.thunderbird.net/v1.1/{domain}")),
    ] {
        if may_reach(&tls(host, 443), private_ok).await.is_err() {
            continue;
        }
        let e = email.clone();
        let found = tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(10)))
                .max_redirects(0)
                .https_only(true)
                .build()
                .into();
            let xml = agent.get(&url).call().ok()?.body_mut().with_config().limit(1 << 20).read_to_string().ok()?;
            account::parse_autoconfig(&xml, &e)
        })
        .await
        .ok()
        .flatten();
        if found.is_some() {
            return found;
        }
    }
    Some(Lookup::Found(account::Found {
        imap: tls(format!("imap.{domain}"), 993),
        smtp: tls(format!("smtp.{domain}"), 465),
        user: email,
        source: "a guess: check these with your provider",
        help: None,
    }))
}

/// At most `most` of `what` by one person in `per` (counting `n` now).
fn within(user: i64, what: &'static str, n: usize, most: usize, per: Duration) -> Result<(), String> {
    static DONE: LazyLock<Mutex<HashMap<(i64, &'static str), Vec<Instant>>>> = LazyLock::new(Default::default);
    let mut done = DONE.lock().unwrap();
    let now = Instant::now();
    done.retain(|_, l| {
        l.retain(|t| now.duration_since(*t) < Duration::from_secs(24 * 3600));
        !l.is_empty()
    });
    let mine = done.entry((user, what)).or_default();
    let recent = mine.iter().filter(|t| now.duration_since(**t) < per).count();
    if recent + n > most {
        return Err(match what {
            "send" => "You've sent a lot of email in the last hour. Wait a while, then try again.".into(),
            _ => "That's a lot of tries. Wait a few minutes, then try again.".into(),
        });
    }
    mine.extend(std::iter::repeat_n(now, n));
    Ok(())
}

/// Recipients one person may send to in an hour, and in one email.
const MOST_SENT: usize = 500;
const MOST_TO: usize = 100;

/// Counts an email to `lists` (To, Cc) against the hour's limit.
fn sending(user: i64, lists: &[&str]) -> Result<(), String> {
    let n: usize = lists.iter().map(|l| send::parse_list(l).map(|v| v.len()).unwrap_or(0)).sum();
    if n > MOST_TO {
        return Err(format!("Up to {MOST_TO} people in one email."));
    }
    within(user, "send", n.max(1), MOST_SENT, Duration::from_secs(3600))
}

/// A header's text on one line.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

/// A plain address a company mailbox can be opened for: nothing a mail
/// server's master login or Graph's paths could read as more than a name.
fn plain_address(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else { return false };
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && local.chars().all(|c| c.is_ascii_alphanumeric() || "._+-'".contains(c))
        && domain.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

// ---- The mailboxes being kept in sync ------------------------------------

/// How a mailbox is reached.
#[derive(Clone)]
enum Link {
    Imap { account: Account, secret: Secret },
    Graph(graph::Mailbox),
}

/// A mailbox someone should have.
struct Want {
    email: String,
    name: String,
    /// "company", "password", "microsoft" or "google".
    kind: &'static str,
    /// Changes when its settings do (the worker then starts again).
    stamp: String,
    link: Link,
    /// Its mail servers must be on the internet (checked before each connection is made).
    guard: bool,
}

struct Running {
    email: String,
    name: String,
    kind: &'static str,
    stamp: String,
    link: Link,
    guard: bool,
    handle: Handle,
}

/// Whose news goes where: a person on a server (its data folder).
type Who = (PathBuf, i64);

static RUNNING: LazyLock<Mutex<HashMap<Who, Vec<Running>>>> = LazyLock::new(Default::default);
static NEWS: LazyLock<Mutex<HashMap<Who, broadcast::Sender<String>>>> = LazyLock::new(Default::default);

fn news(who: &Who) -> broadcast::Sender<String> {
    NEWS.lock().unwrap().entry(who.clone()).or_insert_with(|| broadcast::channel(256).0).clone()
}

fn tell_screens(who: &Who, v: Value) {
    let _ = news(who).send(v.to_string());
}

fn safe(email: &str) -> String {
    email.to_lowercase().chars().map(|c| if c.is_alphanumeric() || "@.-_".contains(c) { c } else { '_' }).collect()
}

fn store_path(s: &Server, user: i64, email: &str) -> PathBuf {
    s.secure.join("mail").join(user.to_string()).join(format!("{}.sqlite", safe(email)))
}

fn stamp_of(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    xenify_sync::b64(&Sha256::digest(parts.join("\0").as_bytes()))
}

/// The company's mailbox for `email`, if the settings cover it.
fn org_link(st: &Settings, email: &str, name: &str) -> Option<Link> {
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    if !plain_address(email) || !st.domains().contains(&domain) {
        return None;
    }
    let tls = |h: &str, p| MailServer { host: h.into(), port: p, security: Security::Tls };
    match st.org.as_str() {
        "microsoft" if !st.ms_client_id.is_empty() && !st.ms_secret.is_empty() && !st.ms_tenant.is_empty() => {
            let tokens = org_ms_tokens(st);
            Some(Link::Graph(graph::Mailbox { base: graph::GRAPH.into(), user: email.into(), tokens }))
        }
        "google" => {
            let key = Arc::new(GoogleKey::read(&st.google_key).ok()?);
            let account = Account {
                email: email.into(),
                name: name.into(),
                user: email.into(),
                imap: tls("imap.gmail.com", 993),
                smtp: tls("smtp.gmail.com", 465),
            };
            let secret = Secret::Token(Arc::new(GoogleAs { key, subject: email.into(), cache: Cached::default() }));
            Some(Link::Imap { account, secret })
        }
        "imap" if !st.master_user.is_empty() => {
            let (imap, smtp) = (st.imap.as_ref()?.server().ok()?, st.smtp.as_ref()?.server().ok()?);
            let how = if st.login_as.is_empty() { "{email}*{master}" } else { st.login_as.as_str() };
            let local = email.rsplit_once('@').map(|x| x.0).unwrap_or(email);
            let user = how.replace("{email}", email).replace("{user}", local).replace("{master}", &st.master_user);
            let account = Account { email: email.into(), name: name.into(), user, imap, smtp };
            Some(Link::Imap { account, secret: Secret::Password(st.master_password.clone()) })
        }
        _ => None,
    }
}

/// One Microsoft app token for every company mailbox (it's the same app).
fn org_ms_tokens(st: &Settings) -> Arc<dyn TokenSource> {
    static ONE: LazyLock<Mutex<Option<(String, Arc<MsApp>)>>> = LazyLock::new(Default::default);
    let stamp = stamp_of(&[&st.ms_tenant, &st.ms_client_id, &st.ms_secret]);
    let mut one = ONE.lock().unwrap();
    if let Some((s, t)) = &*one
        && *s == stamp
    {
        return t.clone();
    }
    let t = Arc::new(MsApp {
        tenant: st.ms_tenant.clone(),
        client_id: st.ms_client_id.clone(),
        secret: st.ms_secret.clone(),
        cache: Cached::default(),
    });
    *one = Some((stamp, t.clone()));
    t
}

/// The mailboxes `user` should have: the company's, then their own.
async fn wanted(s: &Server, user: i64) -> Result<Vec<Want>, Response> {
    let Ok(c) = s.pool.get().await else { return Err(unavailable()) };
    let (st, k) = settings(s, &c).await.map_err(no_key)?;
    let mut out = Vec::new();
    let me = c
        .query_opt("SELECT email, display_name FROM users WHERE id = $1 AND NOT disabled", &[&user])
        .await
        .map_err(|_| unavailable())?;
    let Some(me) = me else { return Ok(out) };
    let (email, name): (Option<String>, Option<String>) = (me.get(0), me.get(1));
    let email = email.unwrap_or_default();
    let name = name.unwrap_or_default();
    let in_group = st.group == 0
        || c.query_opt("SELECT 1 FROM group_members WHERE group_id = $1 AND user_id = $2", &[&st.group, &user])
            .await
            .ok()
            .flatten()
            .is_some();
    if !email.is_empty()
        && in_group
        && let Some(link) = org_link(&st, &email, &name)
    {
        let stamp = stamp_of(&[
            &st.org,
            &st.ms_tenant,
            &st.ms_client_id,
            &st.ms_secret,
            &st.google_key,
            &st.master_user,
            &st.master_password,
            &st.login_as,
            &serde_json::to_string(&(&st.imap, &st.smtp)).unwrap_or_default(),
            &name,
        ]);
        out.push(Want { email: email.clone(), name: name.clone(), kind: "company", stamp, link, guard: false });
    }
    let rows = c
        .query("SELECT email, kind, account, secret FROM mail_accounts WHERE user_id = $1 ORDER BY added", &[&user])
        .await
        .map_err(|_| unavailable())?;
    for r in rows {
        let (addr, kind, acct, sealed): (String, String, String, String) = (r.get(0), r.get(1), r.get(2), r.get(3));
        if out.iter().any(|w| w.email.eq_ignore_ascii_case(&addr)) {
            continue;
        }
        let Ok(account) = serde_json::from_str::<Account>(&acct) else { continue };
        let secret_text = unseal(&k, &row_aad(user, &addr), &sealed);
        let stamp = stamp_of(&[&kind, &acct, &sealed, if st.private_hosts { "private" } else { "" }]);
        let (kind, secret): (&'static str, Secret) = match kind.as_str() {
            "microsoft" | "google" => {
                let Some((p, id, client_secret)) = provider(&st, &kind) else { continue };
                let (pool, k2, addr2, rt) = (s.pool.clone(), k.clone(), addr.clone(), tokio::runtime::Handle::current());
                let keep = Box::new(move |new: String| {
                    let (pool, addr) = (pool.clone(), addr2.clone());
                    let sealed = seal(&k2, &row_aad(user, &addr), &new);
                    rt.spawn(async move {
                        if let Ok(c) = pool.get().await {
                            let _ = c
                                .execute(
                                    "UPDATE mail_accounts SET secret = $3 WHERE user_id = $1 AND email = $2",
                                    &[&user, &addr, &sealed],
                                )
                                .await;
                        }
                    });
                });
                let src = Refresh {
                    url: p.token,
                    client_id: id,
                    secret: client_secret,
                    scope: if kind == "microsoft" { p.scope.into() } else { String::new() },
                    refresh: Mutex::new(secret_text),
                    keep,
                    cache: Cached::default(),
                };
                (if kind == "microsoft" { "microsoft" } else { "google" }, Secret::Token(Arc::new(src)))
            }
            _ => ("password", Secret::Password(secret_text)),
        };
        // A person's own mail server: only on the internet, unless the admin allows more.
        let guard = kind == "password" && !st.private_hosts;
        out.push(Want { email: addr, name: account.name.clone(), kind, stamp, link: Link::Imap { account, secret }, guard });
    }
    Ok(out)
}

/// Starts the workers of `user`'s mailboxes that aren't running (and stops
/// any for mailboxes gone or changed).
async fn ensure(s: &Server, user: i64) -> Result<(), Response> {
    let mut list = wanted(s, user).await?;
    let who: Who = (s.secure.clone(), user);
    // New mailboxes on mail servers that aren't on the internet aren't started.
    let running = emails(s, user);
    let mut refused = Vec::new();
    for w in &list {
        if w.guard
            && !running.contains(&w.email)
            && let Link::Imap { account, .. } = &w.link
            && let Err(e) = async { may_reach(&account.imap, false).await.and(may_reach(&account.smtp, false).await) }.await
        {
            refused.push((w.email.clone(), e));
        }
    }
    list.retain(|w| !refused.iter().any(|r| r.0 == w.email));
    for (email, e) in refused {
        tell_screens(&who, json!({ "account": email, "kind": "problem", "text": e, "folder": "", "uid": 0 }));
    }
    let mut run = RUNNING.lock().unwrap();
    let mine = run.entry(who.clone()).or_default();
    let before = mine.len();
    mine.retain(|r| list.iter().any(|w| w.email == r.email && w.stamp == r.stamp));
    let mut changed = mine.len() != before;
    for w in list {
        if mine.iter().any(|r| r.email == w.email) {
            continue;
        }
        changed = true;
        let path = store_path(s, user, &w.email);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut handle = match &w.link {
            // Guarded mailboxes are checked again before every connection,
            // and connected to exactly the address checked.
            Link::Imap { account, secret } => {
                let reach: Option<sync::Reach> = w.guard.then(|| Arc::new(may_reach_now) as sync::Reach);
                sync::start_reaching(account.clone(), secret.clone(), path.clone(), reach)
            }
            Link::Graph(m) => graph::start(m.clone(), path.clone()),
        };
        let (_, dummy) = std::sync::mpsc::channel();
        let events = std::mem::replace(&mut handle.events, dummy);
        let (who, email) = (who.clone(), w.email.clone());
        std::thread::spawn(move || {
            while let Ok(e) = events.recv() {
                if let Event::Downloaded { folder, uid } = &e
                    && let Some(answer) = invite_answer(&path, folder, *uid)
                {
                    tell_screens(&who, answer);
                }
                if e == Event::Changed {
                    for answer in invite_answers(&path) {
                        tell_screens(&who, answer);
                    }
                }
                let (kind, text, folder, uid) = match e {
                    Event::Status(t) => ("status", t, String::new(), 0),
                    Event::Changed => ("changed", String::new(), String::new(), 0),
                    Event::Downloaded { folder, uid } => ("downloaded", String::new(), folder, uid),
                    Event::Problem(t) => ("problem", t, String::new(), 0),
                    Event::SignInFailed(t) => ("signin_failed", t, String::new(), 0),
                };
                tell_screens(
                    &who,
                    json!({ "account": email, "kind": kind, "text": text, "folder": folder, "uid": uid }),
                );
            }
        });
        mine.push(Running {
            email: w.email,
            name: w.name,
            kind: w.kind,
            stamp: w.stamp,
            link: w.link,
            guard: w.guard,
            handle,
        });
    }
    drop(run);
    if changed {
        tell_screens(&who, json!({ "kind": "accounts", "account": "" }));
    }
    Ok(())
}

/// Starts everyone's mailboxes, so mail comes in while nobody has Chat open.
/// Then every ten minutes: the mailboxes of people turned off or removed
/// stop, and those whose mail servers moved are checked again.
pub(crate) fn start(s: Server) {
    tokio::spawn(async move {
        let mut stopping = s.stopping.subscribe();
        loop {
            restart_all(&s).await;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(600)) => {}
                _ = stopping.changed() => break,
            }
        }
    });
}

/// Everyone who has (or may have) a mailbox, brought up to date.
async fn restart_all(s: &Server) {
    let Ok(c) = s.pool.get().await else { return };
    let Ok((st, _)) = settings(s, &c).await else { return };
    let mut users: Vec<i64> = c
        .query("SELECT DISTINCT user_id FROM mail_accounts", &[])
        .await
        .map(|r| r.iter().map(|r| r.get(0)).collect())
        .unwrap_or_default();
    for d in st.domains() {
        if st.org.is_empty() {
            break;
        }
        let like = format!("%@{d}");
        if let Ok(rows) = c.query("SELECT id FROM users WHERE lower(email) LIKE $1 AND NOT disabled", &[&like]).await {
            users.extend(rows.iter().map(|r| r.get::<_, i64>(0)));
        }
    }
    // Those running now whose mailboxes may have gone.
    users.extend(RUNNING.lock().unwrap().keys().filter(|w| w.0 == s.secure).map(|w| w.1));
    users.sort_unstable();
    users.dedup();
    drop(c);
    for u in users {
        let _ = ensure(s, u).await;
    }
}

fn with<T>(s: &Server, user: i64, email: &str, f: impl FnOnce(&Running) -> T) -> Result<T, String> {
    let run = RUNNING.lock().unwrap();
    run.get(&(s.secure.clone(), user))
        .and_then(|l| l.iter().find(|r| r.email == email))
        .map(f)
        .ok_or_else(|| format!("{email} isn't set up."))
}

fn tell(s: &Server, user: i64, email: &str, cmd: Cmd) {
    let _ = with(s, user, email, |r| r.handle.send(cmd));
}

fn emails(s: &Server, user: i64) -> Vec<String> {
    RUNNING
        .lock()
        .unwrap()
        .get(&(s.secure.clone(), user))
        .map(|l| l.iter().map(|r| r.email.clone()).collect())
        .unwrap_or_default()
}

fn store(s: &Server, user: i64, email: &str) -> Result<Store, String> {
    with(s, user, email, |_| ())?;
    Store::open(&store_path(s, user, email)).map_err(|e| e.to_string())
}

/// Sends `m` from mailbox `email` (and keeps a copy in Sent).
async fn send_from(s: &Server, user: i64, email: &str, m: lettre::Message) -> Result<(), String> {
    let (link, guard) = with(s, user, email, |r| (r.link.clone(), r.guard))?;
    // Guarded: connect exactly to the addresses checked.
    let (mut smtp_pin, mut imap_pin) = (None, None);
    if guard && let Link::Imap { account, .. } = &link {
        smtp_pin = may_reach(&account.smtp, false).await?;
        // No checked address for the copy in Sent: no copy is kept.
        imap_pin = may_reach(&account.imap, false).await.ok().flatten();
    }
    let copy_ok = !guard || imap_pin.is_some();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        match link {
            Link::Graph(mb) => graph::send(&mb, &m.formatted()),
            Link::Imap { account, secret } => {
                send::send_pinned(&account, &secret, &m, smtp_pin)?;
                // Not keeping a copy isn't a reason to say sending failed.
                if copy_ok {
                    let _ = send::save_sent_copy_pinned(&account, &secret, &m.formatted(), imap_pin);
                }
                Ok(())
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Connecting one's own Microsoft or Google mail -------------------------

struct Pending {
    user: i64,
    which: String,
    verifier: String,
    redirect: String,
    at: Instant,
}

static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Default::default);

/// How long a sign-in at the provider may take.
const SIGN_IN_TIME: Duration = Duration::from_secs(10 * 60);
/// Sign-ins one person may have started at once.
const MOST_PENDING: usize = 5;

/// The provider's sign-in page for someone connecting their mail there.
async fn connect_url(s: &Server, h: &HeaderMap, user: i64, which: &str) -> Result<Value, String> {
    use sha2::{Digest, Sha256};
    within(user, "connect", 1, 20, Duration::from_secs(3600))?;
    let c = s.pool.get().await.map_err(|_| DB_DOWN.to_string())?;
    let (st, _) = settings(s, &c).await?;
    let (p, client_id, _) = provider(&st, which).ok_or("The admin hasn't set that up.")?;
    let redirect = format!("{}{CONNECTED}", link_base(&c, h).await);
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let verifier = b64(&xenify_sync::random(32));
    let challenge = b64(&Sha256::digest(verifier.as_bytes()));
    let state = b64(&xenify_sync::random(24));
    let mut url = url::Url::parse(&p.auth).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", p.scope)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", if which == "google" { "consent" } else { "select_account" });
    if which == "google" {
        url.query_pairs_mut().append_pair("access_type", "offline");
    }
    let mut pend = PENDING.lock().unwrap();
    pend.retain(|_, p| p.at.elapsed() < SIGN_IN_TIME);
    // Only the newest few of one person's (each new one pushes out the oldest).
    let mut mine: Vec<(String, Instant)> =
        pend.iter().filter(|(_, p)| p.user == user).map(|(k, p)| (k.clone(), p.at)).collect();
    mine.sort_by_key(|m| m.1);
    while mine.len() >= MOST_PENDING {
        pend.remove(&mine.remove(0).0);
    }
    pend.insert(state, Pending { user, which: which.into(), verifier, redirect, at: Instant::now() });
    Ok(json!({ "url": url.to_string() }))
}

#[derive(Deserialize)]
struct Back {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

/// The pending sign-in `state` names, while it hasn't run out.
fn pending_user(state: &str) -> Option<i64> {
    let pend = PENDING.lock().unwrap();
    pend.get(state).filter(|p| p.at.elapsed() < SIGN_IN_TIME).map(|p| p.user)
}

/// A small page of the server's own, for the end of a sign-in.
fn page(title: &str, body: &str, script: Option<(&str, &Value)>) -> Response {
    let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let nonce = xenify_sync::b64(&xenify_sync::random(18));
    let script = match script {
        Some((code, data)) => format!(
            r#"<script type="application/json" id="d">{}</script><script nonce="{nonce}">{code}</script>"#,
            data.to_string().replace('<', "\\u003c")
        ),
        None => String::new(),
    };
    let html = format!(
        r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{t}</title>
<style nonce="{nonce}">:root{{color-scheme:light dark}}body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#EFECE8;font:16px/1.5 system-ui,sans-serif;color:#1c1917}}main{{background:#fff;border-radius:20px;padding:32px 36px;max-width:420px;margin:16px}}h1{{font-size:22px;margin:0 0 8px}}p{{margin:0 0 12px;color:#57534e}}button{{font:inherit;padding:8px 18px;border-radius:10px;border:0;background:#B23A0A;color:#fff;cursor:pointer}}[hidden]{{display:none}}@media(prefers-color-scheme:dark){{body{{background:#1c1917;color:#f5f5f4}}main{{background:#292524}}p{{color:#d6d3d1}}}}</style>
<main><h1 id="t">{t}</h1><div id="b">{body}</div></main>{script}</html>"#,
        t = esc(title),
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                format!("default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
            ),
            (header::REFERRER_POLICY, "no-referrer".into()),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        html,
    )
        .into_response()
}

fn esc_text(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// What the page does: as whoever is signed in to Xenify in this browser it
/// finishes at once (the server checks it's the person who started); with
/// nobody signed in, it asks first, naming the Xenify account the mail would
/// go to, so a link someone else started can't quietly connect your mail to
/// their account.
const FINISH_JS: &str = r#"
const d = JSON.parse(document.getElementById('d').textContent);
const $ = (id) => document.getElementById(id);
let token = '';
try { token = JSON.parse(localStorage.getItem('xenify-session') || 'null')?.token || ''; } catch (e) {}
function say(title, text) { $('t').textContent = title; $('b').replaceChildren(Object.assign(document.createElement('p'), { textContent: text })); }
async function finish(confirm) {
  say('Connecting your email…', 'One moment.');
  let r, v = {};
  try {
    r = await fetch(location.pathname, { method: 'POST', headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: 'Bearer ' + token } : {}) }, body: JSON.stringify({ state: d.state, code: d.code, confirm }) });
    v = await r.json().catch(() => ({}));
  } catch (e) { return say("Your email isn't connected", "The server couldn't be reached. Start again from Chat."); }
  if (r.ok) {
    say('Your email is connected', v.email + ' is now in Xenify Chat. You can close this tab.');
    try { window.opener && window.opener.postMessage({ xenifyMail: true }, location.origin); } catch (e) {}
  } else if (v.code === 'confirm') {
    token = '';
    ask();
  } else say("Your email isn't connected", v.error || 'Start again from Chat.');
}
function ask() {
  $('t').textContent = 'Connect your email?';
  const p = document.createElement('p');
  p.textContent = 'Your email will be in Xenify Chat for ' + d.who + '. If that isn’t you, close this tab.';
  const b = Object.assign(document.createElement('button'), { textContent: 'Connect' });
  b.onclick = () => finish(true);
  $('b').replaceChildren(p, b);
}
if (token) finish(false); else ask();
"#;

/// The page the provider sends people back to. Nothing is done on the GET:
/// the page finishes it (see FINISH_JS).
async fn connected(State(s): State<Server>, Query(b): Query<Back>) -> Response {
    let gone = || page("Your email isn't connected", "<p>This sign-in has run out. Start again from Chat.</p>", None);
    let Some(user) = pending_user(&b.state) else { return gone() };
    if !b.error.is_empty() || b.code.is_empty() {
        PENDING.lock().unwrap().remove(&b.state);
        let why = if b.error == "access_denied" || b.code.is_empty() {
            "You didn't allow Xenify to use your mail.".to_string()
        } else {
            format!(
                "The sign-in didn't work: {}",
                one_line(if b.error_description.is_empty() { &b.error } else { &b.error_description })
            )
        };
        return page("Your email isn't connected", &format!("<p>{}</p>", esc_text(&why)), None);
    }
    let Ok(c) = s.pool.get().await else { return gone() };
    let who = c
        .query_opt("SELECT display_name, email, username FROM users WHERE id = $1", &[&user])
        .await
        .ok()
        .flatten()
        .map(|r| {
            let (name, email, username): (Option<String>, Option<String>, String) = (r.get(0), r.get(1), r.get(2));
            let name = name.filter(|n| !n.is_empty()).unwrap_or(username);
            match email.filter(|e| !e.is_empty()) {
                Some(e) => format!("{name} ({e})"),
                None => name,
            }
        })
        .unwrap_or_default();
    page(
        "Connecting your email…",
        "<p>One moment.</p>",
        Some((FINISH_JS, &json!({ "state": b.state, "code": b.code, "who": who }))),
    )
}

#[derive(Deserialize)]
struct FinishIn {
    #[serde(default)]
    state: String,
    #[serde(default)]
    code: String,
    /// Nobody signed in in this browser, and the person said yes to the
    /// account the page named.
    #[serde(default)]
    confirm: bool,
}

/// The page's POST: the sign-in is checked and finished.
async fn connected_finish(State(s): State<Server>, h: HeaderMap, Json(f): Json<FinishIn>) -> Response {
    let me = signed_in(&s, &h).await.ok();
    let p = {
        let mut pend = PENDING.lock().unwrap();
        let Some(p) = pend.get(&f.state).filter(|p| p.at.elapsed() < SIGN_IN_TIME) else {
            return err(StatusCode::BAD_REQUEST, "gone", "This sign-in has run out. Start again from Chat.");
        };
        match &me {
            Some(a) if a.id != p.user => {
                pend.remove(&f.state);
                return err(
                    StatusCode::FORBIDDEN,
                    "someone_else",
                    "Someone else started this sign-in, so your email wasn't connected. Start again from Chat.",
                );
            }
            None if !f.confirm => return err(StatusCode::BAD_REQUEST, "confirm", "Say yes first."),
            _ => {}
        }
        pend.remove(&f.state).expect("there")
    };
    match finish_connect(&s, p, f.code).await {
        Ok(email) => Json(json!({ "email": email })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, "mail", &e),
    }
}

async fn finish_connect(s: &Server, p: Pending, code: String) -> Result<String, String> {
    if code.is_empty() || code.len() > 4096 {
        return Err("The sign-in didn't work. Start again from Chat.".into());
    }
    let c = s.pool.get().await.map_err(|_| DB_DOWN.to_string())?;
    let (st, k) = settings(s, &c).await?;
    let (prov, client_id, secret) = provider(&st, &p.which).ok_or("The admin has turned this off.")?;
    let (url, verifier, redirect) = (prov.token.clone(), p.verifier.clone(), p.redirect.clone());
    let v = tokio::task::spawn_blocking(move || {
        let mut form = vec![
            ("client_id", client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
        ];
        if !secret.is_empty() {
            form.push(("client_secret", secret.as_str()));
        }
        token_post(&url, &form)
    })
    .await
    .map_err(|_| "The sign-in didn't work. Try again.".to_string())??;
    let refresh =
        v["refresh_token"].as_str().ok_or("The provider didn't let the server stay signed in. Try again.")?.to_string();
    // Who signed in: the ID token the provider just handed over (straight
    // from its token address over TLS, so its signature needn't be checked).
    let id: Value = v["id_token"]
        .as_str()
        .and_then(|t| t.split('.').nth(1))
        .and_then(|p| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if id["email_verified"] == json!(false) || id["email_verified"] == json!("false") {
        return Err("The provider hasn't checked that email address yet.".into());
    }
    let email = id["email"].as_str().or(id["preferred_username"].as_str()).unwrap_or("").trim().to_string();
    if !email.contains('@') || email.len() > 254 || email.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("The provider didn't say which email address you signed in with.".into());
    }
    let name = c
        .query_opt("SELECT display_name FROM users WHERE id = $1 AND NOT disabled", &[&p.user])
        .await
        .map_err(|_| DB_DOWN.to_string())?
        .ok_or("This Xenify account is turned off.")?
        .get::<_, Option<String>>(0)
        .unwrap_or_default();
    let others: i64 = c
        .query_one("SELECT count(*) FROM mail_accounts WHERE user_id = $1 AND email <> $2", &[&p.user, &email])
        .await
        .map_err(|_| DB_DOWN.to_string())?
        .get(0);
    if others >= MOST_ACCOUNTS {
        return Err(format!("Up to {MOST_ACCOUNTS} email accounts each."));
    }
    let account = Account { email: email.clone(), name, user: email.clone(), imap: prov.imap, smtp: prov.smtp };
    c.execute(
        "INSERT INTO mail_accounts (user_id, email, kind, account, secret) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, email) DO UPDATE SET kind = $3, account = $4, secret = $5",
        &[
            &p.user,
            &email,
            &p.which,
            &serde_json::to_string(&account).unwrap_or_default(),
            &seal(&k, &row_aad(p.user, &email), &refresh),
        ],
    )
    .await
    .map_err(|_| DB_DOWN.to_string())?;
    drop(c);
    let _ = ensure(s, p.user).await;
    Ok(email)
}

// ---- What the screens see ---------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ServerIn {
    host: String,
    port: u16,
    /// "tls", "starttls" (or "none", only for a mail server on this computer).
    security: String,
}

impl ServerIn {
    fn server(&self) -> Result<MailServer, String> {
        let host = self.host.trim().to_string();
        let security = match self.security.as_str() {
            "starttls" => Security::StartTls,
            "none" => {
                // A password over the network without encryption: only to
                // a mail server on the same computer.
                if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
                    return Err("Mail servers on other computers need TLS or STARTTLS.".into());
                }
                Security::None
            }
            _ => Security::Tls,
        };
        if host.is_empty() || self.port == 0 {
            return Err("Type the mail server's name and port.".into());
        }
        Ok(MailServer { host, port: self.port, security })
    }
    fn of(s: &MailServer) -> ServerIn {
        let security = match s.security {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::None => "none",
        };
        ServerIn { host: s.host.clone(), port: s.port, security: security.into() }
    }
}

#[derive(Serialize)]
struct RowOut {
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

fn list_of(s: &Server, user: i64, account: &str, folder: &str, limit: usize) -> Result<Vec<RowOut>, String> {
    let st = store(s, user, account)?;
    let wire = if folder.is_empty() {
        match st.folder_with(Role::Inbox).map_err(|e| e.to_string())? {
            Some(w) => w,
            None => return Ok(Vec::new()),
        }
    } else {
        folder.to_string()
    };
    Ok(st.list(&wire, limit).map_err(|e| e.to_string())?.into_iter().map(|r| row_out(account, r)).collect())
}

/// A file attached in the screens (its contents as base64).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AttachIn {
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

fn tz_of(a: &Value) -> chrono_tz::Tz {
    a["tz"].as_str().and_then(|t| t.parse().ok()).unwrap_or(chrono_tz::UTC)
}

/// A meeting invitation as the reader shows it (as the desktop app's).
fn invite_out(m: &message::Message, mine: &[String], tz: chrono_tz::Tz) -> Option<(xenify_cal::itip::Invite, Value)> {
    use xenify_cal::itip::{Invite, Method};
    let invite = Invite::read(m.calendar.as_deref()?)?;
    let e = invite.event();
    let start = e.start.instant(tz);
    let out = json!({
        "method": match invite.method { Method::Request => "request", Method::Cancel => "cancel", Method::Reply => "reply", _ => "publish" },
        "uid": e.uid,
        "title": e.summary,
        "start": start.timestamp_millis(),
        "end": (start + e.duration(tz)).timestamp_millis(),
        "all_day": e.all_day(),
        "location": e.location,
        "url": e.url,
        "organizer": e.organizer.as_ref().map(|o| o.display()).unwrap_or_default(),
        "repeats": e.rrule.is_some(),
        "answer": invite.me(mine).map(|p| p.answer.label().to_string()).unwrap_or_default(),
        // The screens know (the calendar is theirs).
        "in_calendar": false,
    });
    Some((invite, out))
}

/// If this email answers an invitation (an iCalendar reply), what the
/// screens pass to the meeting: `{ kind: "invite_answer", uid, email, answer }`.
fn invite_answer(path: &std::path::Path, folder: &str, uid: u32) -> Option<Value> {
    use xenify_cal::event::Answer;
    use xenify_cal::itip::{Invite, Method};
    let raw = Store::open(path).ok()?.raw(folder, uid).ok()??;
    let m = message::parse(&raw)?;
    let invite = m.calendar.as_deref().and_then(Invite::read).filter(|i| i.method == Method::Reply)?;
    let e = invite.event();
    let who = e.attendees.first()?;
    // Only the guest can answer for themselves (anyone can write a reply
    // naming someone else).
    if !who.email.trim().eq_ignore_ascii_case(m.from_address.trim()) {
        return None;
    }
    let answer = match who.answer {
        Answer::Accepted => "yes",
        Answer::Tentative => "maybe",
        Answer::Declined => "no",
        _ => return None,
    };
    Some(json!({ "kind": "invite_answer", "uid": e.uid, "email": who.email, "answer": answer }))
}

/// Answers already downloaded among the newest in the inbox (the rest are
/// heard as they're downloaded, which opening them does).
fn invite_answers(path: &std::path::Path) -> Vec<Value> {
    let Ok(s) = Store::open(path) else { return Vec::new() };
    let Ok(Some(inbox)) = s.folder_with(Role::Inbox) else { return Vec::new() };
    s.list(&inbox, 30)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| ["Accepted:", "Tentative:", "Declined:"].iter().any(|w| r.subject.starts_with(w)) && r.downloaded)
        .filter_map(|r| invite_answer(path, &inbox, r.uid))
        .collect()
}

fn arg<T: serde::de::DeserializeOwned>(a: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(a.get(k).cloned().unwrap_or(Value::Null)).map_err(|_| format!("{k} is missing"))
}

use xenify_util::now_ms;

async fn call(State(s): State<Server>, h: HeaderMap, Path(name): Path<String>, Json(a): Json<Value>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if let Err(e) = ensure(&s, me.id).await {
        return e;
    }
    match run(&s, &h, me.id, &name, &a).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, "mail", &e),
    }
}

async fn run(s: &Server, h: &HeaderMap, user: i64, name: &str, a: &Value) -> Result<Value, String> {
    let account = || arg::<String>(a, "account");
    let folder = || arg::<String>(a, "folder");
    let uid = || arg::<u32>(a, "uid");
    let db = || async { s.pool.get().await.map_err(|_| DB_DOWN.to_string()) };
    // A database error says only that (never the query's details).
    let dbe = |_: tokio_postgres::Error| DB_DOWN.to_string();
    Ok(match name {
        "mail_accounts" => {
            let mut list: Vec<Value> = RUNNING
                .lock()
                .unwrap()
                .get(&(s.secure.clone(), user))
                .map(|l| {
                    l.iter()
                        .map(|r| {
                            let kept = match r.kind {
                                "company" => "your company's mailbox",
                                "microsoft" => "signed in with Microsoft",
                                "google" => "signed in with Google",
                                _ => "on the server",
                            };
                            json!({ "email": r.email, "name": r.name, "kind": r.kind, "kept": kept })
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Accounts kept but not running (a mail server that moved off the
            // internet, say), so they can still be removed.
            let rows = db().await?
                .query("SELECT email, account FROM mail_accounts WHERE user_id = $1 ORDER BY added", &[&user])
                .await
                .map_err(dbe)?;
            for r in rows {
                let email: String = r.get(0);
                if !list.iter().any(|a| a["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(&email))) {
                    let name = serde_json::from_str::<Account>(r.get::<_, &str>(1)).map(|a| a.name).unwrap_or_default();
                    list.push(json!({ "email": email, "name": name, "kind": "password", "kept": "on the server" }));
                }
            }
            json!(list)
        }
        // What this server offers for adding email (Settings › Email).
        "mail_options" => {
            let c = db().await?;
            let (st, _) = settings(s, &c).await?;
            json!({
                "microsoft": st.personal_ms_app().is_some(),
                "google": st.personal_google_app().is_some(),
                "passwords": !st.no_passwords,
                "company": !st.org.is_empty(),
            })
        }
        "mail_connect" => {
            let which: String = arg(a, "provider")?;
            return connect_url(s, h, user, &which).await;
        }
        "mail_look_up" => {
            let email: String = arg(a, "email")?;
            within(user, "add", 1, 30, Duration::from_secs(3600))?;
            let c = db().await?;
            let (st, _) = settings(s, &c).await?;
            drop(c);
            let found = look_up(&email, st.private_hosts).await;
            match found {
                Some(Lookup::Found(f)) => json!({
                    "imap": ServerIn::of(&f.imap), "smtp": ServerIn::of(&f.smtp), "user": f.user,
                    "source": f.source, "help": f.help,
                }),
                Some(Lookup::NeedsProviderSignIn(who)) => {
                    return Err(format!(
                        "{who} only lets apps sign in through its own web page, which Xenify can't do yet. Use an app password if {who} offers one."
                    ));
                }
                None => Value::Null,
            }
        }
        "mail_add" => {
            let email = arg::<String>(a, "email")?.trim().to_string();
            let password: String = arg(a, "password")?;
            let imap: ServerIn = arg(a, "imap")?;
            let smtp: ServerIn = arg(a, "smtp")?;
            if !email.contains('@') || email.len() > 254 || email.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err("Type the email address.".into());
            }
            if password.len() > 1024 || password.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
                return Err("The mail server didn't accept that user name and password.".into());
            }
            let user_name = arg::<String>(a, "user").unwrap_or_default();
            let acct = Account {
                email: email.clone(),
                name: arg::<String>(a, "name").unwrap_or_default().trim().into(),
                user: if user_name.trim().is_empty() { email.clone() } else { user_name.trim().into() },
                imap: imap.server()?,
                smtp: smtp.server()?,
            };
            let c = db().await?;
            let (st, k) = settings(s, &c).await?;
            if st.no_passwords {
                return Err("This server doesn't take email passwords. Connect your mail with Microsoft or Google instead, or ask your admin.".into());
            }
            if emails(s, user).iter().any(|e| e.eq_ignore_ascii_case(&email)) {
                return Err(format!("{email} is already set up."));
            }
            let n: i64 = c
                .query_one("SELECT count(*) FROM mail_accounts WHERE user_id = $1", &[&user])
                .await
                .map_err(dbe)?
                .get(0);
            if n >= MOST_ACCOUNTS {
                return Err(format!("Up to {MOST_ACCOUNTS} email accounts each."));
            }
            let taken = c
                .query_opt(
                    "SELECT 1 FROM mail_accounts WHERE user_id = $1 AND lower(email) = lower($2)",
                    &[&user, &email],
                )
                .await
                .map_err(dbe)?;
            if taken.is_some() {
                return Err(format!("{email} is already set up."));
            }
            // Each try reaches a mail server for the person: not too many,
            // and only on the internet unless the admin allows more.
            within(user, "add", 1, 30, Duration::from_secs(3600))?;
            let mut imap_pin = None;
            if !st.private_hosts {
                imap_pin = may_reach(&acct.imap, false).await?;
                may_reach(&acct.smtp, false).await?;
            }
            let (ac, p) = (acct.clone(), password.clone());
            tokio::task::spawn_blocking(move || -> Result<(), String> {
                let mut c = Client::connect_pinned(&ac.imap, imap_pin)
                    .map_err(|e| format!("Couldn't reach {}: {e}", ac.imap.host))?;
                c.login(&ac.user, &p)
                    .map_err(|_| "The mail server didn't accept that user name and password.".to_string())?;
                c.logout();
                Ok(())
            })
            .await
            .map_err(|_| "The mail server couldn't be checked.".to_string())??;
            c.execute(
                "INSERT INTO mail_accounts (user_id, email, kind, account, secret) VALUES ($1, $2, 'password', $3, $4)",
                &[
                    &user,
                    &email,
                    &serde_json::to_string(&acct).unwrap_or_default(),
                    &seal(&k, &row_aad(user, &email), &password),
                ],
            )
            .await
            .map_err(dbe)?;
            drop(c);
            ensure(s, user).await.map_err(|_| "The server can't reach its database.".to_string())?;
            json!({ "email": acct.email, "name": acct.name, "kept": "on the server" })
        }
        "mail_remove" => {
            let email = account()?;
            let company = RUNNING.lock().unwrap().get(&(s.secure.clone(), user)).is_some_and(|l| {
                l.iter().any(|r| r.kind == "company" && r.email.eq_ignore_ascii_case(&email))
            });
            if company {
                return Err("This is your company's mailbox: it stays while your admin keeps it connected.".into());
            }
            let c = db().await?;
            let gone = c
                .execute("DELETE FROM mail_accounts WHERE user_id = $1 AND email = $2", &[&user, &email])
                .await
                .map_err(dbe)?;
            if gone == 0 {
                return Err(format!("{email} isn't set up."));
            }
            let drafts = c
                .query("SELECT id, value FROM mail_drafts WHERE user_id = $1", &[&user])
                .await
                .map_err(dbe)?;
            for r in drafts {
                let v: Value = serde_json::from_str(r.get::<_, &str>(1)).unwrap_or_default();
                if v["account"] == email.as_str() {
                    let id: String = r.get(0);
                    let _ = c.execute("DELETE FROM mail_drafts WHERE user_id = $1 AND id = $2", &[&user, &id]).await;
                }
            }
            drop(c);
            // Stopped (ensure drops its worker), then its copy goes.
            let _ = ensure(s, user).await;
            let _ = std::fs::remove_file(store_path(s, user, &email));
            Value::Null
        }
        "mail_folders" => {
            let st = store(s, user, &account()?)?;
            json!(st
                .folders()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|f| json!({ "wire": f.wire, "name": f.name, "role": f.role.map(|r| format!("{r:?}").to_lowercase()), "unread": f.unread, "total": f.total }))
                .collect::<Vec<_>>())
        }
        "mail_list" => {
            let (acct, f) = (account()?, folder().unwrap_or_default());
            let limit = arg::<usize>(a, "limit").unwrap_or(200).min(2000);
            if !acct.is_empty() {
                json!(list_of(s, user, &acct, &f, limit)?)
            } else {
                let mut all: Vec<RowOut> =
                    emails(s, user).iter().flat_map(|e| list_of(s, user, e, "", limit).unwrap_or_default()).collect();
                all.sort_by_key(|r| std::cmp::Reverse(r.at));
                all.truncate(limit);
                json!(all)
            }
        }
        "mail_search" => {
            let q: String = arg(a, "q")?;
            let mut all = Vec::new();
            for e in emails(s, user) {
                if let Ok(st) = store(s, user, &e) {
                    all.extend(st.search(&q, 100).unwrap_or_default().into_iter().map(|r| row_out(&e, r)));
                }
            }
            all.sort_by_key(|r| std::cmp::Reverse(r.at));
            json!(all)
        }
        "mail_open" => {
            let (acct, f, u) = (account()?, folder()?, uid()?);
            let st = store(s, user, &acct)?;
            let Some(raw) = st.raw(&f, u).map_err(|e| e.to_string())? else {
                tell(s, user, &acct, Cmd::Download { folder: f, uid: u });
                return Ok(Value::Null);
            };
            let m = message::parse(&raw).ok_or("This email couldn't be read.")?;
            if st.row(&f, u).ok().flatten().is_some_and(|r| !r.is_read()) {
                tell(s, user, &acct, Cmd::SetFlag { folder: f.clone(), uid: u, flag: "\\Seen".into(), on: true });
            }
            let (html, blocked) = match &m.html {
                Some(h) => {
                    let opts = xenify_safe_html::Options { remote_images: false, ..Default::default() };
                    let clean = xenify_safe_html::clean(h, &opts, &|src| m.inline_image(src));
                    (Some(clean.html), clean.blocked_images)
                }
                None => (None, 0),
            };
            let invite = invite_out(&m, &emails(s, user), tz_of(a)).map(|(_, v)| v);
            if invite.as_ref().is_some_and(|i| i["method"] == "reply")
                && let Some(answer) = invite_answer(&store_path(s, user, &acct), &f, u)
            {
                tell_screens(&(s.secure.clone(), user), answer);
            }
            json!({
                "invite": invite, "account": acct, "folder": f, "uid": u,
                "from": m.from, "from_address": m.from_address, "to": m.to, "cc": m.cc, "subject": m.subject,
                "date": message::nice_date(&m.date), "html": html, "text": m.text, "blocked_images": blocked,
                "attachments": m.attachments.iter().map(|x| (x.name.clone(), x.mime.clone(), x.data.len())).collect::<Vec<_>>(),
                "message_id": m.message_id,
            })
        }
        "mail_flag" => {
            tell(
                s,
                user,
                &account()?,
                Cmd::SetFlag { folder: folder()?, uid: uid()?, flag: arg(a, "flag")?, on: arg(a, "on")? },
            );
            Value::Null
        }
        "mail_delete" => {
            tell(s, user, &account()?, Cmd::Delete { folder: folder()?, uid: uid()? });
            Value::Null
        }
        "mail_move" => {
            tell(s, user, &account()?, Cmd::Move { folder: folder()?, uid: uid()?, to: arg(a, "to")? });
            Value::Null
        }
        "mail_sync_now" => {
            for e in emails(s, user) {
                tell(s, user, &e, Cmd::SyncNow);
            }
            Value::Null
        }
        "mail_send" => {
            let acct_email = account()?;
            let (acct_name, acct_addr) = with(s, user, &acct_email, |r| (r.name.clone(), r.email.clone()))?;
            let (mut in_reply_to, mut references) = (None, Vec::new());
            if let Ok(Some((f, u))) = arg::<Option<(String, u32)>>(a, "reply_to")
                && let Ok(Some(raw)) =
                    store(s, user, &acct_email).and_then(|st| st.raw(&f, u).map_err(|e| e.to_string()))
                && let Some(m) = message::parse(&raw)
            {
                references = m.references.clone();
                if !m.message_id.is_empty() {
                    references.push(m.message_id.clone());
                    in_reply_to = Some(m.message_id);
                }
            }
            let text: String = arg(a, "text").unwrap_or_default();
            let (to, cc): (String, String) = (arg(a, "to").unwrap_or_default(), arg(a, "cc").unwrap_or_default());
            sending(user, &[&to, &cc])?;
            let out = Outgoing {
                from_name: acct_name,
                from_email: acct_addr,
                to,
                cc,
                subject: one_line(&arg::<String>(a, "subject").unwrap_or_default()),
                html: html_of(&text),
                text,
                in_reply_to,
                references,
                attachments: attachments_of(&arg::<Vec<AttachIn>>(a, "attachments").unwrap_or_default())?,
                ..Outgoing::default()
            };
            let m = send::build(&out)?;
            send_from(s, user, &acct_email, m).await?;
            tell(s, user, &acct_email, Cmd::SyncNow);
            // Whoever you wrote to, for the screens to put in My contacts.
            let people: Vec<(String, String)> = [&out.to, &out.cc]
                .into_iter()
                .flat_map(|list| send::parse_list(list).unwrap_or_default())
                .map(|m| (m.name.clone().unwrap_or_default(), m.email.to_string()))
                .collect();
            json!({ "people": people })
        }
        "mail_drafts" => {
            let c = db().await?;
            let rows = c
                .query("SELECT value FROM mail_drafts WHERE user_id = $1 ORDER BY saved DESC", &[&user])
                .await
                .map_err(dbe)?;
            json!(
                rows.iter().filter_map(|r| serde_json::from_str::<Value>(r.get::<_, &str>(0)).ok()).collect::<Vec<_>>()
            )
        }
        "mail_draft_save" => {
            let mut d: Value = arg(a, "draft")?;
            attachments_of(&serde_json::from_value::<Vec<AttachIn>>(d["attachments"].clone()).unwrap_or_default())?;
            let mut id = d["id"].as_str().unwrap_or("").to_string();
            if id.is_empty() || id.len() > 40 {
                id = format!("{:x}", now_ms() as u128 * 1000 + fastrand_u16() as u128);
            }
            let saved = now_ms();
            d["id"] = json!(id);
            d["saved"] = json!(saved);
            let value = d.to_string();
            let c = db().await?;
            let r = c
                .query_one(
                    "SELECT count(*), COALESCE(sum(length(value)), 0)::BIGINT FROM mail_drafts WHERE user_id = $1 AND id <> $2",
                    &[&user, &id],
                )
                .await
                .map_err(dbe)?;
            let (n, size): (i64, i64) = (r.get(0), r.get(1));
            if n >= MOST_DRAFTS {
                return Err(format!("Up to {MOST_DRAFTS} unfinished emails: send or delete some first."));
            }
            if size as usize + value.len() > MOST_DRAFT_BYTES {
                return Err("Your unfinished emails are too big in all: send or delete some first.".into());
            }
            c.execute(
                "INSERT INTO mail_drafts (user_id, id, value, saved) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (user_id, id) DO UPDATE SET value = $3, saved = $4",
                &[&user, &id, &value, &saved],
            )
            .await
            .map_err(dbe)?;
            json!(id)
        }
        "mail_draft_delete" => {
            let id: String = arg(a, "id")?;
            db().await?
                .execute("DELETE FROM mail_drafts WHERE user_id = $1 AND id = $2", &[&user, &id])
                .await
                .map_err(dbe)?;
            Value::Null
        }
        "mail_answer_invite" => answer_invite(s, user, a).await?,
        "mail_invite" => invite(s, user, a).await?,
        _ => return Err(format!("unknown call “{name}”")),
    })
}

fn fastrand_u16() -> u16 {
    let b = xenify_sync::random(2);
    u16::from_le_bytes([b[0], b[1]])
}

/// Answers the invitation in an email ("accept", "maybe" or "decline"): the
/// organizer is told (an iCalendar reply, which Outlook, Google and Apple
/// calendars read). The calendar is the screens': they get the event to keep
/// (`{ event }`), or to take out (`{ declined: uid }`).
async fn answer_invite(s: &Server, user: i64, a: &Value) -> Result<Value, String> {
    use xenify_cal::event::{Answer, Person};
    let (account, folder, uid): (String, String, u32) = (arg(a, "account")?, arg(a, "folder")?, arg(a, "uid")?);
    let (acct_name, acct_email) = with(s, user, &account, |r| (r.name.clone(), r.email.clone()))?;
    let raw = store(s, user, &account)?.raw(&folder, uid).map_err(|e| e.to_string())?.ok_or("Open the email first.")?;
    let m = message::parse(&raw).ok_or("This email couldn't be read.")?;
    let tz = tz_of(a);
    let mine = emails(s, user);
    let (invite, out) = invite_out(&m, &mine, tz).ok_or("There's no invitation in this email.")?;
    let answer = match arg::<String>(a, "answer")?.as_str() {
        "accept" => Answer::Accepted,
        "maybe" => Answer::Tentative,
        _ => Answer::Declined,
    };
    let event = invite.event().clone();
    if let Some(org) = &event.organizer {
        let me = invite.me(&mine).cloned().unwrap_or(Person {
            email: acct_email.clone(),
            name: acct_name.clone(),
            ..Person::default()
        });
        sending(user, &[&org.email])?;
        let ics = xenify_cal::itip::reply(&event, &me, answer);
        let text = format!(
            "{} {}.",
            if acct_name.is_empty() { &acct_email } else { &acct_name },
            match answer {
                Answer::Accepted => "will come",
                Answer::Tentative => "might come",
                _ => "won't come",
            }
        );
        let reply = Outgoing {
            from_name: acct_name.clone(),
            from_email: acct_email.clone(),
            to: org.email.clone(),
            subject: one_line(&xenify_cal::itip::reply_subject(answer, &event.summary)),
            html: html_of(&text),
            text,
            calendar: Some(("REPLY".into(), ics)),
            ..Outgoing::default()
        };
        let msg = send::build(&reply)?;
        send_from(s, user, &account, msg).await?;
    }
    if answer == Answer::Declined {
        return Ok(json!({ "declined": event.uid }));
    }
    let link = json!({ "type": "mail", "account": account, "folder": folder, "uid": uid, "label": format!("the invitation “{}”", event.summary) });
    let notes = [event.location.as_str(), event.url.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(json!({ "event": {
        "id": 0, "title": event.summary, "start": out["start"], "end": out["end"].as_i64().unwrap_or(0).max(out["start"].as_i64().unwrap_or(0)),
        "all_day": out["all_day"], "notes": notes, "kind": "event", "link": link.to_string(),
        "remind": event.alarms.first().copied().unwrap_or(10), "repeat": event.rrule.clone().unwrap_or_default(), "ical_uid": event.uid,
    }}))
}

/// A meeting's invitation for its guests outside the server (or, with
/// `cancel`, that it's off), from the meeting's email account (`details`
/// as Chat keeps them; `people`: who on the server is invited).
async fn invite(s: &Server, user: i64, a: &Value) -> Result<Value, String> {
    use chrono::TimeZone;
    use xenify_cal::event::{Answer, Event as CalEvent, Person, When};
    let d: Value = arg(a, "details")?;
    let organiser: String = arg(a, "organiser").unwrap_or_default();
    let people: Vec<String> = arg(a, "people").unwrap_or_default();
    let cancel: bool = arg(a, "cancel").unwrap_or(false);
    let tz = tz_of(a);
    let st = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let from = st("mail_from");
    let (acct_name, acct_email) = with(s, user, &from, |r| (r.name.clone(), r.email.clone()))?;
    let (start, end) = (d["start"].as_i64().unwrap_or(0), d["end"].as_i64().unwrap_or(0));
    let at = |ms: i64| chrono::Utc.timestamp_millis_opt(ms).single().unwrap_or_default();
    let (title, uid, notes, place) = (one_line(&st("title")), st("uid"), st("notes"), st("place"));
    let mut e = CalEvent::new(&uid, &title, When::Utc(at(start)), Some(When::Utc(at(end.max(start)))));
    let me = if acct_name.is_empty() { organiser.clone() } else { acct_name.clone() };
    let call_note = format!(
        "This meeting is a call in Xenify Chat, for people on {me}'s server. Reply to {me} for another way to join."
    );
    let rooms: Vec<String> = d["things"]
        .as_array()
        .map(|l| l.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    let place_all =
        [rooms.join(", "), place.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
    e.location = if place_all.is_empty() { "Xenify Chat call".into() } else { place_all.clone() };
    e.description = [notes.as_str(), if place.is_empty() { call_note.as_str() } else { "" }]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let sequence = d["sequence"].as_u64().unwrap_or(0) as u32;
    e.sequence = if cancel { sequence.saturating_sub(1) } else { sequence };
    e.organizer = Some(Person { email: acct_email.clone(), name: me.clone(), ..Person::default() });
    let guests: Vec<(String, String, String)> = d["guests"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|g| {
                    (
                        g["email"].as_str().unwrap_or("").into(),
                        g["name"].as_str().unwrap_or("").into(),
                        g["answer"].as_str().unwrap_or("").into(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if guests.is_empty() {
        return Ok(Value::Null);
    }
    e.attendees = guests
        .iter()
        .map(|(email, name, answer)| Person {
            email: email.clone(),
            name: name.clone(),
            answer: match answer.as_str() {
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
    let (s0, e0) = (at(start).with_timezone(&tz), at(end.max(start)).with_timezone(&tz));
    let when = format!("{}–{} ({})", s0.format("%A %-d %B %Y, %H:%M"), e0.format("%H:%M"), tz.name());
    let short = format!("{}–{}", s0.format("%a %-d %b %H:%M"), e0.format("%H:%M"));
    let mut with_list = people;
    with_list.extend(guests.iter().map(|(email, name, _)| if name.is_empty() { email.clone() } else { name.clone() }));
    let text = if cancel {
        format!("{me} called off {title}, {when}.")
    } else {
        let mut t = format!("{me} invites you to {title}.\n\n{when}\nWith {}", with_list.join(", "));
        if !place_all.is_empty() {
            t.push_str(&format!("\nWhere: {place_all}"));
        }
        for part in [notes.as_str(), if place.is_empty() { call_note.as_str() } else { "" }] {
            if !part.is_empty() {
                t.push_str("\n\n");
                t.push_str(part);
            }
        }
        t
    };
    let to = guests.iter().map(|g| g.0.clone()).collect::<Vec<_>>().join(", ");
    sending(user, &[&to])?;
    let out = Outgoing {
        from_name: me,
        from_email: acct_email.clone(),
        to,
        subject: if cancel { format!("Cancelled: {title}") } else { format!("Invitation: {title} · {short}") },
        html: html_of(&text),
        text,
        calendar: Some((method.into(), ics)),
        ..Outgoing::default()
    };
    let m = send::build(&out)?;
    send_from(s, user, &from, m).await?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
struct AttachmentQuery {
    account: String,
    folder: String,
    uid: u32,
    index: usize,
}

/// One attachment of a downloaded email, as a file.
async fn attachment(State(s): State<Server>, h: HeaderMap, Query(q): Query<AttachmentQuery>) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if let Err(e) = ensure(&s, me.id).await {
        return e;
    }
    let got = (|| -> Result<Attachment, String> {
        let raw = store(&s, me.id, &q.account)?
            .raw(&q.folder, q.uid)
            .map_err(|e| e.to_string())?
            .ok_or("Open the email first.")?;
        let m = message::parse(&raw).ok_or("This email couldn't be read.")?;
        m.attachments.into_iter().nth(q.index).ok_or_else(|| "That attachment isn't there.".to_string())
    })();
    match got {
        Ok(a) => {
            let name = message::safe_file_name(&a.name);
            let encoded: String = name
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b".-_".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            (
                [
                    // Never shown as a page from the server's own address.
                    (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                    (header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{encoded}")),
                    (header::HeaderName::from_static("x-content-type-options"), "nosniff".into()),
                ],
                Body::from(a.data),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", &e),
    }
}

/// What changes, as server-sent events (one JSON object each): the desktop
/// app's `mail` events, and `invite_answer`s for the meetings.
async fn events(State(s): State<Server>, h: HeaderMap) -> Response {
    let me = match signed_in(&s, &h).await {
        Ok(a) => a,
        Err(e) => return e,
    };
    if let Err(e) = ensure(&s, me.id).await {
        return e;
    }
    let Some(slot) = Listening::take((s.secure.clone(), me.id)) else {
        return err(StatusCode::TOO_MANY_REQUESTS, "busy", "Too many windows are listening for email. Close some.");
    };
    let mut rx = news(&(s.secure.clone(), me.id)).subscribe();
    let mut stopping = s.stopping.subscribe();
    let stream = async_stream(move |tx| async move {
        let _slot = slot;
        // A comment now and then keeps proxies from closing the connection.
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(25));
        tick.tick().await;
        let _ = tx.send(": hello\n\n".to_string()).await;
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Ok(m) => { if tx.send(format!("data: {m}\n\n")).await.is_err() { break } }
                    Err(broadcast::error::RecvError::Lagged(_)) => { if tx.send("data: {\"kind\":\"changed\",\"account\":\"\"}\n\n".into()).await.is_err() { break } }
                    Err(_) => break,
                },
                _ = tick.tick() => {
                    // Signed out (or turned off) since: the stream ends.
                    if signed_in(&s, &h).await.is_err() || tx.send(": .\n\n".to_string()).await.is_err() { break }
                }
                _ = stopping.changed() => break,
            }
        }
    });
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

/// Event streams one person may have open at once.
const MOST_LISTENING: usize = 8;

/// One of a person's open event streams (given back when dropped), by
/// server (its data folder: tests run several in one program) and person.
struct Listening(Who);

static LISTENING: LazyLock<Mutex<HashMap<Who, usize>>> = LazyLock::new(Default::default);

impl Listening {
    fn take(who: Who) -> Option<Listening> {
        let mut l = LISTENING.lock().unwrap();
        let n = l.entry(who.clone()).or_default();
        if *n >= MOST_LISTENING {
            return None;
        }
        *n += 1;
        Some(Listening(who))
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        let mut l = LISTENING.lock().unwrap();
        if let Some(n) = l.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                l.remove(&self.0);
            }
        }
    }
}

/// A stream fed by a task (`f` gets the sending end).
fn async_stream<F, Fut>(f: F) -> impl futures_util::Stream<Item = Result<String, std::convert::Infallible>>
where
    F: FnOnce(tokio::sync::mpsc::Sender<String>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    tokio::spawn(f(tx));
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx).map(|o| o.map(Ok)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Key {
        Key { k: [7; 32] }
    }

    #[test]
    fn sealed_secrets_open_only_where_they_were_sealed() {
        let k = key();
        let a = seal(&k, &row_aad(1, "a@example.com"), "pw");
        assert_ne!(seal(&k, &row_aad(1, "a@example.com"), "pw"), a, "a new nonce each time");
        assert_eq!(unseal(&k, &row_aad(1, "a@example.com"), &a), "pw");
        // Copied to someone else's row, or opened with another key: nothing.
        assert_eq!(unseal(&k, &row_aad(2, "a@example.com"), &a), "");
        assert_eq!(unseal(&Key { k: [8; 32] }, &row_aad(1, "a@example.com"), &a), "");
    }

    #[test]
    fn the_key_file_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("xenify-mail-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = file_key(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join("mail.key")).unwrap(), a.to_vec());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("mail.key")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(file_key(&dir).unwrap(), a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_internet_addresses_are_public() {
        for ip in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0",
            "255.255.255.255", "::1", "::", "fc00::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe", "2002:7f00:1::", "::7f00:1", "198.18.0.1", "240.0.0.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn own_mail_servers_must_be_on_the_internet() {
        let at = |h: &str| MailServer { host: h.into(), port: 993, security: Security::Tls };
        for h in ["127.0.0.1", "localhost", "169.254.169.254", "[::1]", "10.0.0.5"] {
            assert!(may_reach(&at(h), false).await.is_err(), "{h}");
            assert!(may_reach(&at(h), true).await.is_ok(), "{h}: the admin allows it");
        }
        assert!(may_reach(&at("8.8.8.8"), false).await.is_ok());
        // The checked address is the one connected to.
        assert_eq!(may_reach(&at("8.8.8.8"), false).await.unwrap(), Some("8.8.8.8:993".parse().unwrap()));
        let v6 = may_reach_now(&at("[2001:4860:4860::8888]")).unwrap();
        assert_eq!(v6.map(|a| a.ip().to_string()).as_deref(), Some("2001:4860:4860::8888"));
        assert!(may_reach_now(&at("127.0.0.1")).is_err());
    }

    #[test]
    fn company_mailboxes_only_for_plain_addresses() {
        assert!(plain_address("sam.o'neil+x@example.org"));
        for e in ["ceo@example.org*master", "a/../ceo@example.org", "a*b@example.org", "a b@example.org", "@example.org"] {
            assert!(!plain_address(e), "{e}");
        }
    }

    #[test]
    fn headers_stay_on_one_line() {
        assert_eq!(one_line("Hi\r\nBcc: x@example.com"), "Hi  Bcc: x@example.com");
    }

    #[test]
    fn sending_is_limited() {
        let me = -42;
        assert!(sending(me, &["a@example.com, b@example.com"]).is_ok());
        let many = (0..=MOST_TO).map(|i| format!("p{i}@example.com")).collect::<Vec<_>>().join(", ");
        assert!(sending(me, &[&many]).is_err(), "too many in one email");
        for _ in 0..(MOST_SENT - 2) {
            sending(me, &["a@example.com"]).unwrap();
        }
        assert!(sending(me, &["a@example.com"]).is_err(), "too many in an hour");
    }
}
