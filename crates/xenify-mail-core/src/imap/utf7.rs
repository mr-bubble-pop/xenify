//! IMAP's "modified UTF-7" for folder names (RFC 3501 §5.1.3):
//! "Entwürfe" travels as "Entw&APw-rfe".

use base64::Engine;
use base64::alphabet::Alphabet;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};

fn engine() -> GeneralPurpose {
    // Standard base64 with ',' for '/' and no padding.
    let alphabet = Alphabet::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,").expect("valid");
    GeneralPurpose::new(
        &alphabet,
        GeneralPurposeConfig::new()
            .with_encode_padding(false)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    )
}

/// A folder name for the wire.
pub fn encode(name: &str) -> String {
    let mut out = String::new();
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        out.push('&');
        out.push_str(&engine().encode(bytes));
        out.push('-');
        pending.clear();
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// A folder name from the wire (left as is if it isn't valid modified UTF-7).
pub fn decode(wire: &str) -> String {
    let mut out = String::new();
    let mut rest = wire;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('-') else { return wire.to_string() };
        let chunk = &rest[i + 1..i + j];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let Ok(bytes) = engine().decode(chunk) else { return wire.to_string() };
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b)).collect();
            match String::from_utf16(&units) {
                Ok(s) => out.push_str(&s),
                Err(_) => return wire.to_string(),
            }
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_rfc_examples() {
        assert_eq!(encode("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(decode("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(encode("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(decode("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode("~peter/mail/&U,BTFw-/&ZeVnLIqe-"), "~peter/mail/台北/日本語");
        assert_eq!(encode("~peter/mail/台北/日本語"), "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
        assert_eq!(decode("&Jjo-!"), "☺!");
        assert_eq!(decode("broken&"), "broken&");
    }
}
