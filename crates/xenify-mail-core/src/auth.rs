//! How an account signs in: a password, or a token from the mail provider
//! (Microsoft, Google) that's fetched again when it runs out (OAuth 2:
//! IMAP and SMTP's XOAUTH2). A token can be taken back by whoever gave it;
//! no password is kept.

use std::sync::Arc;

/// Gives a current access token (fetching a new one when it's run out).
pub trait TokenSource: Send + Sync {
    fn token(&self) -> Result<String, String>;
}

#[derive(Clone)]
pub enum Secret {
    Password(String),
    Token(Arc<dyn TokenSource>),
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Secret::Password(_) => "Secret::Password(…)",
            Secret::Token(_) => "Secret::Token(…)",
        })
    }
}

impl From<String> for Secret {
    fn from(p: String) -> Secret {
        Secret::Password(p)
    }
}

/// XOAUTH2's initial response (before base64).
pub fn xoauth2(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}
