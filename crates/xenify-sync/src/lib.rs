//! Small helpers the Xenify apps and server share, with no I/O so they
//! build for the browser too.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("bad data: {0}")]
    Bad(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// URL-safe base64 without padding, used for everything sent as text.
pub fn b64(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

pub fn unb64(text: &str) -> Result<Vec<u8>> {
    base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, text.trim())
        .map_err(|_| Error::Bad("base64"))
}

/// `n` random bytes.
pub fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("random");
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_round_trips() {
        let b = super::random(20);
        assert_eq!(super::unb64(&super::b64(&b)).unwrap(), b);
    }
}
