//! Xenify: a spreadsheet number, kept as an exact decimal.
//!
//! Upstream keeps every number as an `f64`, which holds about 15 to 17
//! significant digits and no decimal fractions exactly (0.1 + 0.2 isn't
//! 0.3; a 16 digit number may come back changed). `Num` holds a
//! `rust_decimal::Decimal` instead: 28 significant digits, exact for typed
//! and pasted numbers and for + − × and most ÷.
//!
//! - Functions that are worked out with floating point (SQRT, LN, EXP,
//!   trigonometry, statistics, finance) still are, exactly as upstream:
//!   their answers stay `f64` (whole numbers become exact), and anything
//!   worked out from one is floating point too. Showing such a number
//!   rounds it to 15 significant digits, as Excel does.
//! - ROUND and its kin turn a floating point number into an exact one.
//! - Numbers a decimal can't hold (beyond ±7.9e28, or non-zero and smaller
//!   than 1e-28, or not finite) stay `f64`.
//! - Division that doesn't end within 28 digits (1/3) is rounded there;
//!   `=` and the comparisons treat numbers that agree to 27 significant
//!   digits as equal, so =1/3*3=1 is TRUE.
//!
//! Function code that works with `f64` reads a number with [`Num::f`] and
//! hands a result back with `Num::from(f64)`; the operators below mix `Num`
//! and `f64` so most of it reads as before.

use bitcode::{Decode, Encode};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, Sub, SubAssign};

/// A number in a cell or a formula: 16 bytes, so cells don't grow.
///
/// The bytes are `Decimal::serialize`, whose first two (flags) are always
/// zero; floating point sets the first to 1 and keeps the `f64` in the last
/// eight.
#[derive(Clone, Copy, Encode, Decode)]
pub struct Num([u8; 16]);

#[derive(Clone, Copy)]
enum Repr {
    D(Decimal),
    F(f64),
}

impl Num {
    #[inline]
    fn r(self) -> Repr {
        if self.0[0] == 1 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&self.0[8..]);
            Repr::F(f64::from_le_bytes(b))
        } else {
            Repr::D(Decimal::deserialize(self.0))
        }
    }

    #[inline]
    fn float(f: f64) -> Num {
        let mut b = [0u8; 16];
        b[0] = 1;
        b[8..].copy_from_slice(&f.to_le_bytes());
        Num(b)
    }
}

/// Digits two numbers must agree to for `=`.
const EQUAL_DIGITS: u32 = 27;

impl Num {
    pub const ZERO: Num = Num([0; 16]);

    /// An exact decimal.
    pub fn dec(d: Decimal) -> Num {
        Num(d.normalize().serialize())
    }

    /// The decimal, when the number is one.
    pub fn decimal(self) -> Option<Decimal> {
        match self.r() {
            Repr::D(d) => Some(d),
            Repr::F(_) => None,
        }
    }

    /// Whether the number is exact (a decimal, not floating point).
    pub fn is_exact(self) -> bool {
        matches!(self.r(), Repr::D(_))
    }

    /// As floating point (the nearest one), for function code that works
    /// with `f64`.
    #[inline]
    pub fn f(self) -> f64 {
        match self.r() {
            Repr::D(d) => {

                if d.scale() == 0 {
                    if let Some(i) = d.to_i64() {
                        return i as f64;
                    }
                }
                // Through the digits: correctly rounded.
                d.to_string().parse().unwrap_or(f64::NAN)
            }
            Repr::F(f) => f,
        }
    }

    /// From a floating point answer: whole numbers that `f64` holds exactly
    /// become exact; anything else stays floating point.
    pub fn from_f64(f: f64) -> Num {
        if f == 0.0 {
            return Num::ZERO;
        }
        if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
            return Num::dec(Decimal::from(f as i64));
        }
        Num::float(f)
    }

    /// From a floating point number, as the shortest digits that read back
    /// as it (0.1 + 0.2 gives 0.30000000000000004), exact from then on.
    pub fn from_f64_digits(f: f64) -> Num {
        if !f.is_finite() {
            return Num::float(f);
        }
        Num::parse(&format!("{f}")).unwrap_or(Num::float(f))
    }

    /// Reads a plain number: an optional sign, digits with an optional
    /// decimal point, and an optional exponent (`1.5E-3`). More than 28
    /// significant digits are rounded; beyond what a decimal holds, it's
    /// floating point.
    pub fn parse(s: &str) -> Option<Num> {
        let t = s.trim();
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b)) {
            return None;
        }
        let (mantissa, exp) = match t.find(['e', 'E']) {
            Some(i) => (&t[..i], t[i + 1..].parse::<i32>().ok()?),
            None => (t, 0),
        };
        let (neg, digits) = match mantissa.as_bytes().first() {
            Some(b'-') => (true, &mantissa[1..]),
            Some(b'+') => (false, &mantissa[1..]),
            _ => (false, mantissa),
        };
        if digits.is_empty() || digits == "." || digits.matches('.').count() > 1 || digits.contains(['+', '-']) {
            return None;
        }
        let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
        let f = || t.parse::<f64>().ok().map(Num::from_f64);
        // The value is sig × 10^-scale.
        let all = format!("{int}{frac}");
        let all = all.trim_start_matches('0');
        if all.is_empty() {
            return Some(Num::ZERO);
        }
        let mut scale = frac.len() as i64 - exp as i64;
        let mut sig = all.trim_end_matches('0').to_string();
        scale -= (all.len() - sig.len()) as i64;
        // Round to 28 significant digits.
        if sig.len() > 28 {
            let up = sig.as_bytes()[28] >= b'5';
            scale -= (sig.len() - 28) as i64;
            sig.truncate(28);
            if up {
                match bump(&sig) {
                    Some(s) => sig = s,
                    None => {
                        sig = "1".into();
                        scale -= 28;
                    }
                }
            }
            let kept = sig.trim_end_matches('0').len();
            scale -= (sig.len() - kept) as i64;
            sig.truncate(kept);
        }
        if !(-28..=28).contains(&scale) {
            return f();
        }
        let mantissa: i128 = sig.parse().ok()?;
        let d = if scale >= 0 {
            Decimal::try_from_i128_with_scale(mantissa, scale as u32).ok()
        } else {
            Decimal::try_from_i128_with_scale(mantissa, 0).ok().and_then(|m| m.checked_mul(Decimal::from_i128_with_scale(10i128.pow((-scale) as u32), 0)))
        };
        match d {
            Some(d) => Some(Num::dec(if neg { -d } else { d })),
            None => f(),
        }
    }

    pub fn is_nan(self) -> bool {
        matches!(self.r(), Repr::F(f) if f.is_nan())
    }

    pub fn is_finite(self) -> bool {
        !matches!(self.r(), Repr::F(f) if !f.is_finite())
    }

    pub fn is_zero(self) -> bool {
        match self.r() {
            Repr::D(d) => d.is_zero(),
            Repr::F(f) => f == 0.0,
        }
    }

    pub fn is_sign_negative(self) -> bool {
        match self.r() {
            Repr::D(d) => d.is_sign_negative() && !d.is_zero(),
            Repr::F(f) => f < 0.0,
        }
    }

    pub fn abs(self) -> Num {
        match self.r() {
            Repr::D(d) => Num::dec(d.abs()),
            Repr::F(f) => Num::float(f.abs()),
        }
    }

    /// Toward zero.
    pub fn trunc(self) -> Num {
        match self.r() {
            Repr::D(d) => Num::dec(d.trunc()),
            Repr::F(f) => Num::from_f64(f.trunc()),
        }
    }

    pub fn floor(self) -> Num {
        match self.r() {
            Repr::D(d) => Num::dec(d.floor()),
            Repr::F(f) => Num::from_f64(f.floor()),
        }
    }

    pub fn ceil(self) -> Num {
        match self.r() {
            Repr::D(d) => Num::dec(d.ceil()),
            Repr::F(f) => Num::from_f64(f.ceil()),
        }
    }

    pub fn fract(self) -> Num {
        self - self.trunc()
    }

    pub fn is_integer(self) -> bool {
        match self.r() {
            Repr::D(d) => d.fract().is_zero(),
            Repr::F(f) => f.fract() == 0.0,
        }
    }

    /// To `digits` places after the point (negative: before it), halves
    /// away from zero, as ROUND does.
    pub fn round_places(self, digits: i32) -> Num {
        self.round_with(digits, RoundingStrategy::MidpointAwayFromZero)
    }

    /// The same, always away from zero (ROUNDUP).
    pub fn round_up_places(self, digits: i32) -> Num {
        self.round_with(digits, RoundingStrategy::AwayFromZero)
    }

    /// The same, always toward zero (ROUNDDOWN, TRUNC).
    pub fn round_down_places(self, digits: i32) -> Num {
        self.round_with(digits, RoundingStrategy::ToZero)
    }

    fn round_with(self, digits: i32, how: RoundingStrategy) -> Num {
        // Floating point: rounded as its shortest digits, which is what
        // it shows (2.675 rounds to 2.68, as in Excel).
        let d = match self.decimal() {
            Some(d) => d,
            None => match Num::from_f64_digits(self.f()).decimal() {
                Some(d) => d,
                None => {
                    let f = self.f();
                    let p = 10f64.powi(digits);
                    return Num::from_f64(match how {
                        RoundingStrategy::AwayFromZero => (f * p).abs().ceil().copysign(f) / p,
                        RoundingStrategy::ToZero => (f * p).trunc() / p,
                        _ => (f * p).round() / p,
                    });
                }
            },
        };
        if digits >= 0 {
            Num::dec(d.round_dp_with_strategy(digits.min(28) as u32, how))
        } else {
            let p = match Decimal::from_i128_with_scale(10, 0).checked_powi(-digits as i64) {
                Some(p) => p,
                None => return Num::ZERO,
            };
            match (d / p).round_dp_with_strategy(0, how).checked_mul(p) {
                Some(r) => Num::dec(r),
                None => Num::from_f64(self.f()),
            }
        }
    }

    /// `self` to the power `e`: exact for whole powers that fit, else
    /// floating point.
    pub fn pow(self, e: Num) -> Num {
        if let (Some(d), Some(x)) = (self.decimal(), e.decimal()) {
            if x.fract().is_zero() && x.abs() <= Decimal::from(96) {
                let n = x.to_i64().unwrap_or(0);
                if n >= 0 {
                    if let Some(r) = d.checked_powi(n) {
                        return Num::dec(r);
                    }
                } else if !d.is_zero() {
                    if let Some(r) = d.checked_powi(-n).and_then(|p| Decimal::ONE.checked_div(p)) {
                        return Num::dec(r);
                    }
                }
            }
        }
        Num::from_f64(self.f().powf(e.f()))
    }

    /// Ordering for sorting and comparisons; numbers that agree to 27
    /// significant digits are equal.
    pub fn compare(self, other: Num) -> Ordering {
        match (self.decimal(), other.decimal()) {
            (Some(a), Some(b)) => {
                if a == b || near(a, b) {
                    Ordering::Equal
                } else {
                    a.cmp(&b)
                }
            }
            _ => self.f().partial_cmp(&other.f()).unwrap_or(Ordering::Equal),
        }
    }

    pub fn is_infinite(self) -> bool {
        matches!(self.r(), Repr::F(f) if f.is_infinite())
    }

    pub fn max(self, o: Num) -> Num {
        if o > self { o } else { self }
    }

    pub fn min(self, o: Num) -> Num {
        if o < self { o } else { self }
    }

    /// The same for equal numbers (to tell values apart, as UNIQUE does).
    pub fn key(self) -> u128 {
        match self.r() {
            Repr::D(d) => u128::from_le_bytes(d.normalize().serialize()),
            Repr::F(f) => (1u128 << 127) | f.to_bits() as u128,
        }
    }

    /// The number written out in full, without trailing zeros (and with a
    /// point, an exponent and up to 17 digits for a floating point one).
    pub fn to_plain_string(self) -> String {
        match self.r() {
            Repr::D(d) => d.normalize().to_string(),
            Repr::F(f) => format!("{f}"),
        }
    }

    /// How an xlsx file holds the number (`<v>`). Floating point is
    /// written as Excel writes it, with 17 significant digits
    /// (-2.2999999999999998), so it reads back as floating point; an exact
    /// fraction that would have 16 or 17 is given trailing zeros, so it
    /// reads back exact. See [`Num::from_xlsx_str`].
    pub fn to_xlsx_string(self) -> String {
        match self.r() {
            Repr::F(f) if f.is_finite() => {
                let e = format!("{f:.16e}");
                let (m, x) = e.split_once('e').unwrap_or((&e, "0"));
                let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
                if x == "0" { m.to_string() } else { format!("{m}E{x}") }
            }
            Repr::F(f) => format!("{f}"),
            Repr::D(d) => {
                let mut text = d.normalize().to_string();
                while xlsx_float_digits(&text) {
                    text.push('0');
                }
                text
            }
        }
    }

    /// The number in an xlsx file's `<v>`: what Excel writes for its
    /// floating point numbers (a fraction with 16 or 17 significant digits)
    /// is floating point; anything else is exact.
    pub fn from_xlsx_str(text: &str) -> Option<Num> {
        let text = text.trim();
        let f = text.parse::<f64>().ok().filter(|f| f.is_finite())?;
        if xlsx_float_digits(text) {
            return Some(Num::from(f));
        }
        Num::parse(text).or(Some(Num::from(f)))
    }

    /// The number as it's edited and written in formulas: an exact number
    /// with every digit, floating point to 15 significant digits as
    /// upstream (0.1 + 0.2 reads 0.3).
    pub fn to_input_string(self) -> String {
        match self.r() {
            Repr::D(d) => d.normalize().to_string(),
            Repr::F(f) => crate::number_format::to_excel_precision_str(f),
        }
    }

    /// Rounded to `digits` significant digits, for showing.
    pub fn to_significant(self, digits: u32) -> Num {
        match self.decimal() {
            Some(d) => round_sig(d, digits).map(Num::dec).unwrap_or(self),
            None => self,
        }
    }
}

/// Whether `text` (a number as an xlsx file writes it) is a fraction with
/// 16 or 17 significant digits as written: how Excel writes floating point.
fn xlsx_float_digits(text: &str) -> bool {
    let (mantissa, exp) = match text.split_once(['e', 'E']) {
        Some((m, x)) => (m, x.parse::<i64>().unwrap_or(0)),
        None => (text, 0),
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let significant = digits.trim_start_matches('0').len();
    let decimals = mantissa.split_once('.').map_or(0, |(_, f)| f.len()) as i64 - exp;
    (16..=17).contains(&significant) && decimals > 0
}

/// `d` to `digits` significant digits (halves away from zero).
fn round_sig(d: Decimal, digits: u32) -> Option<Decimal> {
    if d.is_zero() {
        return Some(d);
    }
    let m = d.mantissa().unsigned_abs();
    let len = m.checked_ilog10().unwrap_or(0) + 1;
    if len <= digits {
        return Some(d);
    }
    let drop = len - digits;
    let scale = d.scale();
    if drop <= scale {
        return Some(d.round_dp_with_strategy(scale - drop, RoundingStrategy::MidpointAwayFromZero));
    }
    // Rounding to the left of the point.
    let p = Decimal::from_i128_with_scale(10i128.checked_pow(drop - scale)?, 0);
    let t = d.trunc();
    (t / p).round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero).checked_mul(p)
}

/// Whether `a` and `b` agree to 27 significant digits.
fn near(a: Decimal, b: Decimal) -> bool {
    if a.is_sign_negative() != b.is_sign_negative() {
        return false;
    }
    match (round_sig(a, EQUAL_DIGITS), round_sig(b, EQUAL_DIGITS)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Adds one to a string of digits ("199" → "200"); None when it carries
/// past the front ("999").
fn bump(s: &str) -> Option<String> {
    let mut b = s.as_bytes().to_vec();
    for i in (0..b.len()).rev() {
        if b[i] == b'9' {
            b[i] = b'0';
        } else {
            b[i] += 1;
            return String::from_utf8(b).ok();
        }
    }
    None
}

impl Default for Num {
    fn default() -> Self {
        Num::ZERO
    }
}

impl fmt::Debug for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_string())
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_string())
    }
}

impl From<f64> for Num {
    fn from(f: f64) -> Self {
        Num::from_f64(f)
    }
}

impl From<Num> for f64 {
    fn from(n: Num) -> Self {
        n.f()
    }
}

impl From<Decimal> for Num {
    fn from(d: Decimal) -> Self {
        Num::dec(d)
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Num {
            fn from(i: $t) -> Self {
                Num::dec(Decimal::from(i))
            }
        }
    )*};
}
from_int!(i32, i64, u32, u64, usize, i16, u16, u8, i8);

impl PartialEq for Num {
    fn eq(&self, other: &Num) -> bool {
        self.compare(*other) == Ordering::Equal
    }
}

impl PartialOrd for Num {
    fn partial_cmp(&self, other: &Num) -> Option<Ordering> {
        if self.is_nan() || other.is_nan() {
            return None;
        }
        Some(self.compare(*other))
    }
}

impl PartialEq<f64> for Num {
    fn eq(&self, other: &f64) -> bool {
        *self == Num::from_f64(*other)
    }
}

impl PartialOrd<f64> for Num {
    fn partial_cmp(&self, other: &f64) -> Option<Ordering> {
        self.partial_cmp(&Num::from_f64(*other))
    }
}

impl PartialEq<Num> for f64 {
    fn eq(&self, other: &Num) -> bool {
        Num::from_f64(*self) == *other
    }
}

impl PartialOrd<Num> for f64 {
    fn partial_cmp(&self, other: &Num) -> Option<Ordering> {
        Num::from_f64(*self).partial_cmp(other)
    }
}

impl Neg for Num {
    type Output = Num;
    fn neg(self) -> Num {
        match self.r() {
            Repr::D(d) => Num::dec(-d),
            Repr::F(f) => Num::float(-f),
        }
    }
}

macro_rules! op {
    ($tr:ident, $m:ident, $as:ident, $am:ident, $checked:ident, $fop:tt) => {
        impl $tr for Num {
            type Output = Num;
            fn $m(self, o: Num) -> Num {
                if let (Some(a), Some(b)) = (self.decimal(), o.decimal()) {
                    if let Some(r) = a.$checked(b) {
                        return Num::dec(r);
                    }
                }
                Num::from_f64(self.f() $fop o.f())
            }
        }
        impl $tr<f64> for Num {
            type Output = Num;
            fn $m(self, o: f64) -> Num {
                self $fop Num::from_f64(o)
            }
        }
        impl $tr<Num> for f64 {
            type Output = Num;
            fn $m(self, o: Num) -> Num {
                Num::from_f64(self) $fop o
            }
        }
        impl $as for Num {
            fn $am(&mut self, o: Num) {
                *self = *self $fop o;
            }
        }
        impl $as<f64> for Num {
            fn $am(&mut self, o: f64) {
                *self = *self $fop Num::from_f64(o);
            }
        }
    };
}

op!(Add, add, AddAssign, add_assign, checked_add, +);

macro_rules! ref_op {
    ($tr:ident, $m:ident, $op:tt) => {
        impl $tr<&Num> for &Num {
            type Output = Num;
            fn $m(self, o: &Num) -> Num {
                *self $op *o
            }
        }
        impl $tr<f64> for &Num {
            type Output = Num;
            fn $m(self, o: f64) -> Num {
                *self $op o
            }
        }
    };
}
ref_op!(Add, add, +);
ref_op!(Sub, sub, -);
ref_op!(Mul, mul, *);
ref_op!(Div, div, /);
op!(Sub, sub, SubAssign, sub_assign, checked_sub, -);
op!(Mul, mul, MulAssign, mul_assign, checked_mul, *);
op!(Div, div, DivAssign, div_assign, checked_div, /);

impl Rem for Num {
    type Output = Num;
    fn rem(self, o: Num) -> Num {
        if let (Some(a), Some(b)) = (self.decimal(), o.decimal()) {
            if let Some(r) = a.checked_rem(b) {
                return Num::dec(r);
            }
        }
        Num::from_f64(self.f() % o.f())
    }
}

impl std::iter::Sum for Num {
    fn sum<I: Iterator<Item = Num>>(iter: I) -> Num {
        iter.fold(Num::ZERO, |a, b| a + b)
    }
}

/// Written as a JSON number when its shortest digits are the number's
/// (0.1, 12.5, floating point), else as a string of digits
/// ("1234567890123456789"). Read back exactly either way.
impl serde::Serialize for Num {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let f = self.f();
        if !self.is_exact() || Num::from_f64_digits(f).to_plain_string() == self.to_plain_string() {
            s.serialize_f64(f)
        } else {
            s.serialize_str(&self.to_plain_string())
        }
    }
}

impl<'de> serde::Deserialize<'de> for Num {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Num, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Num;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a number")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Num, E> {
                Ok(Num::from_f64_digits(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Num, E> {
                Ok(Num::from(v))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Num, E> {
                Ok(Num::from(v))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Num, E> {
                Num::parse(v).ok_or_else(|| E::custom("not a number"))
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Num {
        Num::parse(s).unwrap()
    }

    #[test]
    fn exact() {
        assert_eq!((n("0.1") + n("0.2")).to_plain_string(), "0.3");
        assert_eq!(n("1234567890123456789").to_plain_string(), "1234567890123456789");
        assert_eq!(n("9007199254740993").to_plain_string(), "9007199254740993");
        assert_eq!((n("9007199254740993") + n("1")).to_plain_string(), "9007199254740994");
        assert_eq!(n("1.5E-3").to_plain_string(), "0.0015");
        assert_eq!(n("-12.50").to_plain_string(), "-12.5");
        assert_eq!(n("1e5").to_plain_string(), "100000");
        assert_eq!(n("000123").to_plain_string(), "123");
        assert_eq!(n(".5").to_plain_string(), "0.5");
    }

    #[test]
    fn limits() {
        // 30 significant digits round to 28.
        assert_eq!(n("1.23456789012345678901234567891").to_plain_string(), "1.234567890123456789012345679");
        // Too big for a decimal: floating point.
        assert!(n("1e300").decimal().is_none());
        assert_eq!(n("1e300").f(), 1e300);
        assert!(Num::parse("1.2.3").is_none());
        assert!(Num::parse("abc").is_none());
        assert!(Num::parse("-").is_none());
    }

    #[test]
    fn floats() {
        assert!(!Num::from_f64(0.1 + 0.2).is_exact());
        assert_eq!(Num::from_f64(0.1 + 0.2).f(), 0.1 + 0.2);
        assert_eq!(Num::from_f64(2f64.sqrt()).f(), 2f64.sqrt());
        assert!(Num::from_f64(-3.0).is_exact());
        assert_eq!(Num::from_f64(-3.0).to_plain_string(), "-3");
        // Exact meets float: float.
        assert!(!(n("1") + Num::from_f64(0.5_f64.sqrt())).is_exact());
        // ROUND makes it exact.
        assert_eq!(Num::from_f64(2f64.sqrt()).round_places(2).to_plain_string(), "1.41");
        assert!(Num::from_f64(2f64.sqrt()).round_places(2).is_exact());
        assert_eq!(n("0.3").f(), 0.3);
        assert_eq!(n("0.1").f(), 0.1);
    }

    #[test]
    fn division_and_equality() {
        let third = n("1") / n("3");
        assert!(third * n("3") == n("1"));
        assert_eq!((n("10") / n("4")).to_plain_string(), "2.5");
        assert!(n("2") > n("1.9999999"));
        assert!(n("1") != n("1.000000000000000000001"));
    }

    #[test]
    fn xlsx() {
        // Excel's floating point stays floating point, both ways.
        let f = Num::from_xlsx_str("-2.2999999999999998").unwrap();
        assert!(!f.is_exact());
        assert_eq!(f.f(), -2.3);
        assert_eq!(f.to_xlsx_string(), "-2.2999999999999998");
        let g = Num::from(0.1 + 0.2);
        assert_eq!(Num::from_xlsx_str(&g.to_xlsx_string()).unwrap().f(), 0.1 + 0.2);
        assert!(!Num::from_xlsx_str(&g.to_xlsx_string()).unwrap().is_exact());
        // Exact numbers stay exact, every digit.
        for t in ["0.1", "1234567890123456789", "0.1234567890123456", "0.12345678901234567", "-7.8", "12345678901234567890.123"] {
            let a = n(t);
            let b = Num::from_xlsx_str(&a.to_xlsx_string()).unwrap();
            assert!(b.is_exact(), "{t}");
            assert_eq!(a.to_plain_string(), b.to_plain_string(), "{t}");
        }
        // Floating point that is a short decimal comes back exact and equal.
        assert_eq!(Num::from(0.5).to_xlsx_string(), "5E-1");
        assert_eq!(Num::from_xlsx_str("5E-1").unwrap().to_plain_string(), "0.5");
    }

    #[test]
    fn json() {
        for t in ["0.1", "12.5", "1234567890123456789", "-0.000123", "1e-27"] {
            let a = n(t);
            let text = serde_json::to_string(&a).unwrap();
            let b: Num = serde_json::from_str(&text).unwrap();
            assert!(b.is_exact(), "{t} via {text}");
            assert_eq!(a.to_plain_string(), b.to_plain_string(), "{t} via {text}");
        }
        assert_eq!(serde_json::to_string(&n("0.1")).unwrap(), "0.1");
        assert_eq!(serde_json::to_string(&n("1234567890123456789")).unwrap(), "\"1234567890123456789\"");
    }

    #[test]
    fn rounding() {
        assert_eq!(n("2.675").round_places(2).to_plain_string(), "2.68");
        assert_eq!(n("-2.5").round_places(0).to_plain_string(), "-3");
        assert_eq!(n("1234.5").round_places(-2).to_plain_string(), "1200");
        assert_eq!(n("1.231").round_up_places(2).to_plain_string(), "1.24");
        assert_eq!(n("1.239").round_down_places(2).to_plain_string(), "1.23");
        assert_eq!(n("2").pow(n("10")).to_plain_string(), "1024");
        assert_eq!(n("2").pow(n("-2")).to_plain_string(), "0.25");
    }
}
