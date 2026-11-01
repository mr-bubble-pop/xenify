//! Slows down password guessing: after too many failed sign-ins from one
//! address, further tries from it are refused for a while. One account
//! that many addresses get wrong is only slowed down (each address gets a
//! few tries at it), so nobody can keep someone else locked out.
//!
//! Keys are short: typed names are hashed, IPv6 addresses go by their /64
//! (one home or office network).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failed tries allowed per window.
pub const TRIES: u32 = 10;
pub const WINDOW: Duration = Duration::from_secs(5 * 60);
/// Once an account has had [`TRIES`] wrong tries lately, each address gets
/// this many at it per window.
pub const ACCOUNT_TRIES_EACH: u32 = 3;
/// Emails with a code to pick a new password, per account.
pub const FORGOT_EMAILS: u32 = 3;
pub const FORGOT_WINDOW: Duration = Duration::from_secs(3600);
/// Past this many keys, the oldest go first (a flood of made-up names
/// can't fill the memory).
const MAX_KEYS: usize = 100_000;

#[derive(Default)]
pub struct Limiter {
    failures: Mutex<HashMap<String, (u32, Instant, Duration)>>,
}

/// An address's key: "ip:192.0.2.1", or "ip:2001:db8:1:2::/64".
pub fn ip_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v) => format!("ip:{v}"),
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => format!("ip:{v4}"),
            None => {
                let s = v.segments();
                format!("ip:{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
    }
}

/// A typed user name or email address's key (hashed: short, whatever was typed).
pub fn name_key(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(name.trim().to_lowercase().as_bytes());
    format!("user:{}", h[..12].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// What signing in counts on: the address, and the account (or the name
/// typed, when there's no such account, so both look the same).
pub struct SignIn {
    pub ip: String,
    /// None: no name typed (an older link names its account itself).
    pub account: Option<String>,
}

impl SignIn {
    pub fn new(ip: IpAddr, account: Option<String>) -> SignIn {
        SignIn { ip: ip_key(ip), account }
    }

    fn pair(&self) -> Option<String> {
        self.account.as_ref().map(|a| format!("{a}@{}", self.ip))
    }

    fn keys(&self) -> Vec<String> {
        [Some(self.ip.clone()), self.account.clone(), self.pair()].into_iter().flatten().collect()
    }
}

impl Limiter {
    fn count(f: &HashMap<String, (u32, Instant, Duration)>, key: &str) -> u32 {
        f.get(key).map(|e| e.0).unwrap_or(0)
    }

    fn tidy(f: &mut HashMap<String, (u32, Instant, Duration)>, now: Instant) {
        f.retain(|_, (_, since, window)| now.duration_since(*since) < *window);
        if f.len() > MAX_KEYS {
            let mut by_age: Vec<(Instant, String)> = f.iter().map(|(k, e)| (e.1, k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(f.len() - MAX_KEYS / 2) {
                f.remove(&k);
            }
        }
    }

    /// Whether any of `keys` has failed too often lately.
    pub fn blocked(&self, keys: &[String]) -> bool {
        let now = Instant::now();
        let mut f = self.failures.lock().unwrap();
        Self::tidy(&mut f, now);
        keys.iter().any(|k| Self::count(&f, k) >= TRIES)
    }

    pub fn failed(&self, keys: &[String]) {
        for k in keys {
            self.hit(k, WINDOW);
        }
    }

    /// Counts one for `key` (its window starts with the first); the count now.
    pub fn hit(&self, key: &str, window: Duration) -> u32 {
        let now = Instant::now();
        let mut f = self.failures.lock().unwrap();
        Self::tidy(&mut f, now);
        let e = f.entry(key.to_string()).or_insert((0, now, window));
        e.0 += 1;
        e.0
    }

    /// Too many wrong tries from this address, or at this account from here
    /// while it's being guessed at from many.
    pub fn sign_in_blocked(&self, k: &SignIn) -> bool {
        let now = Instant::now();
        let mut f = self.failures.lock().unwrap();
        Self::tidy(&mut f, now);
        if Self::count(&f, &k.ip) >= TRIES {
            return true;
        }
        match (&k.account, k.pair()) {
            (Some(a), Some(p)) => Self::count(&f, a) >= TRIES && Self::count(&f, &p) >= ACCOUNT_TRIES_EACH,
            _ => false,
        }
    }

    pub fn sign_in_failed(&self, k: &SignIn) {
        self.failed(&k.keys());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_guessed_at_stays_open_to_others() {
        let l = Limiter::default();
        let ip = |s: &str| s.parse().unwrap();
        let acct = Some("acct:7".to_string());
        // Four addresses get it wrong ten times between them…
        for n in 0..12 {
            let k = SignIn::new(ip(&format!("203.0.113.{}", n % 4)), acct.clone());
            assert!(!l.sign_in_blocked(&k), "try {n}");
            l.sign_in_failed(&k);
        }
        // …so each of them now gets only a few, but someone elsewhere still can.
        assert!(l.sign_in_blocked(&SignIn::new(ip("203.0.113.0"), acct.clone())));
        assert!(!l.sign_in_blocked(&SignIn::new(ip("198.51.100.9"), acct.clone())));
        // One address that keeps trying is stopped whatever it tries.
        for _ in 0..TRIES {
            l.sign_in_failed(&SignIn::new(ip("192.0.2.5"), Some(name_key("x"))));
        }
        assert!(l.sign_in_blocked(&SignIn::new(ip("192.0.2.5"), Some("acct:8".into()))));
    }

    #[test]
    fn keys_are_short_and_ipv6_goes_by_network() {
        let ip = |s: &str| s.parse().unwrap();
        assert_eq!(ip_key(ip("2001:db8:1:2:aaaa::1")), ip_key(ip("2001:db8:1:2:bbbb::9")));
        assert_ne!(ip_key(ip("2001:db8:1:2::1")), ip_key(ip("2001:db8:1:3::1")));
        assert_eq!(ip_key(ip("::ffff:192.0.2.1")), "ip:192.0.2.1");
        assert_eq!(name_key(&"x".repeat(100_000)).len(), "user:".len() + 24);
        assert_eq!(name_key(" Sam@Example.org "), name_key("sam@example.org"));
    }
}
