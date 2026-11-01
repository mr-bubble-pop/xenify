use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with alpha, stored as `#rrggbb` / `#rrggbbaa` in files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const BLACK: Rgba = Rgba([0, 0, 0, 255]);
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba([r, g, b, 255])
    }

    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn from_hex(s: &str) -> Option<Self> {
        let h = s.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
                Some(Rgba([d(0)?, d(1)?, d(2)?, 255]))
            }
            6 => Some(Rgba([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Rgba([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgba::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("bad colour {s:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for c in [Rgba::BLACK, Rgba::rgb(0x12, 0xab, 0xef), Rgba([1, 2, 3, 4])] {
            assert_eq!(Rgba::from_hex(&c.to_hex()), Some(c));
        }
        assert_eq!(Rgba::from_hex("#f00"), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(Rgba::from_hex("nope"), None);
    }
}
