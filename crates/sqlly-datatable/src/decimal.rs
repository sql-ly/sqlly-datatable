//! Exact decimal values.
//!
//! An `f64` holds 15–17 significant decimal digits. Database `decimal` and
//! `numeric` columns hold up to 38, and integers past `i64` turn up in
//! `numeric(38,0)` keys. [`ExactDecimal`] keeps the digits the source sent,
//! so the grid paints and orders them exactly, alongside the nearest `f64`
//! for the work that is approximate anyway: filters, conditional formatting,
//! aggregation, and charts.
//!
//! Build cells with [`crate::CellValue::from_decimal_text`], which only
//! reaches for [`crate::CellValue::ExactDecimal`] when an `i64` or `f64`
//! would change the value.

use std::cmp::Ordering;
use std::sync::Arc;

/// Decimals with at most this many significant digits survive a round trip
/// through `f64` unchanged (`DBL_DIG`).
const F64_EXACT_DIGITS: usize = 15;

/// A plain decimal number held as its canonical digits.
///
/// The canonical text is an optional `-`, an integer part with no leading
/// zeros (`0` when it is zero), and an optional `.` plus the fraction digits
/// as given, trailing zeros included, so the source's scale survives. Zero is
/// never negative.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactDecimal {
    text: Arc<str>,
    value: f64,
}

impl ExactDecimal {
    /// Parse a plain decimal: optional sign, digits, optional `.` and digits.
    /// Surrounding whitespace is ignored. Exponents, separators, and anything
    /// else return `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        let (negative, unsigned) = match trimmed.as_bytes().first()? {
            b'-' => (true, &trimmed[1..]),
            b'+' => (false, &trimmed[1..]),
            _ => (false, trimmed),
        };
        let (int_part, frac_part) = match unsigned.split_once('.') {
            Some((int_part, frac_part)) => (int_part, Some(frac_part)),
            None => (unsigned, None),
        };
        let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if !all_digits(int_part) || !frac_part.is_none_or(all_digits) {
            return None;
        }
        if int_part.is_empty() && frac_part.is_none_or(str::is_empty) {
            return None;
        }
        let int_part = int_part.trim_start_matches('0');
        let int_part = if int_part.is_empty() { "0" } else { int_part };
        let is_zero = int_part == "0" && frac_part.is_none_or(|f| f.bytes().all(|b| b == b'0'));
        let mut canonical = String::with_capacity(trimmed.len() + 1);
        if negative && !is_zero {
            canonical.push('-');
        }
        canonical.push_str(int_part);
        if let Some(frac) = frac_part.filter(|f| !f.is_empty()) {
            canonical.push('.');
            canonical.push_str(frac);
        }
        let value = canonical.parse::<f64>().ok()?;
        Some(Self {
            text: canonical.into(),
            value,
        })
    }

    /// The canonical digits, e.g. `-12345678901234567890.1200`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The nearest `f64`. Infinite when the value is beyond `f64` range.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        self.value
    }

    /// Whether the value is below zero.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.text.starts_with('-')
    }

    /// Whether converting to `f64` and back would keep every digit.
    #[must_use]
    pub fn fits_f64(&self) -> bool {
        self.significant_digits() <= F64_EXACT_DIGITS
    }

    /// Exact total order on the decimal values themselves (scale ignored, so
    /// `1.50` equals `1.5`).
    #[must_use]
    pub fn cmp_exact(&self, other: &Self) -> Ordering {
        match (self.is_negative(), other.is_negative()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => cmp_magnitude(self.parts(), other.parts()),
            (true, true) => cmp_magnitude(other.parts(), self.parts()),
        }
    }

    /// The unsigned value rounded half away from zero to `decimals` fraction
    /// digits (padded with zeros when the source has fewer), as plain digits
    /// with a `.` separator.
    #[must_use]
    pub fn rounded_magnitude(&self, decimals: usize) -> String {
        let (int_part, frac_part) = self.parts();
        if frac_part.len() <= decimals {
            let mut out = String::from(int_part);
            if decimals > 0 {
                out.push('.');
                out.push_str(frac_part);
                out.extend(std::iter::repeat_n('0', decimals - frac_part.len()));
            }
            return out;
        }
        let mut digits: Vec<u8> = int_part
            .bytes()
            .chain(frac_part[..decimals].bytes())
            .collect();
        let mut int_len = int_part.len();
        if frac_part.as_bytes()[decimals] >= b'5' {
            let mut carry = true;
            for digit in digits.iter_mut().rev() {
                if *digit == b'9' {
                    *digit = b'0';
                } else {
                    *digit += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                digits.insert(0, b'1');
                int_len += 1;
            }
        }
        let mut out = String::with_capacity(digits.len() + 1);
        out.extend(digits[..int_len].iter().map(|&b| char::from(b)));
        if decimals > 0 {
            out.push('.');
            out.extend(digits[int_len..].iter().map(|&b| char::from(b)));
        }
        out
    }

    /// Integer and fraction digits of the magnitude.
    fn parts(&self) -> (&str, &str) {
        let unsigned = self.text.strip_prefix('-').unwrap_or(&self.text);
        unsigned.split_once('.').unwrap_or((unsigned, ""))
    }

    fn significant_digits(&self) -> usize {
        let (int_part, frac_part) = self.parts();
        let digits = format!("{int_part}{frac_part}");
        digits.trim_matches('0').len()
    }
}

impl From<i64> for ExactDecimal {
    fn from(value: i64) -> Self {
        Self {
            text: value.to_string().into(),
            value: value as f64,
        }
    }
}

impl std::fmt::Display for ExactDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// Compare two unsigned magnitudes given as (integer, fraction) digits with
/// no leading integer zeros.
fn cmp_magnitude(a: (&str, &str), b: (&str, &str)) -> Ordering {
    a.0.len()
        .cmp(&b.0.len())
        .then_with(|| a.0.cmp(b.0))
        .then_with(|| {
            let len = a.1.len().max(b.1.len());
            let digit = |s: &str, i: usize| s.as_bytes().get(i).copied().unwrap_or(b'0');
            (0..len)
                .map(|i| digit(a.1, i).cmp(&digit(b.1, i)))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn d(text: &str) -> ExactDecimal {
        ExactDecimal::parse(text).expect("valid decimal")
    }

    #[test]
    fn parse_canonicalizes_sign_and_leading_zeros() {
        assert_eq!(d(" +007.50 ").as_str(), "7.50");
        assert_eq!(d("-0.000").as_str(), "0.000");
        assert_eq!(d(".5").as_str(), "0.5");
        assert_eq!(d("12.").as_str(), "12");
        assert_eq!(
            d("-12345678901234567890123").as_str(),
            "-12345678901234567890123"
        );
    }

    #[test]
    fn parse_rejects_non_plain_decimals() {
        for bad in ["", "-", ".", "1e5", "1,000", "1.2.3", "abc", "--1", "NaN"] {
            assert!(ExactDecimal::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn fits_f64_counts_significant_digits() {
        assert!(d("123456789012345").fits_f64());
        assert!(d("0.000123456789012345").fits_f64());
        assert!(d("1000000000000000000000").fits_f64());
        assert!(d("1234.5000").fits_f64());
        assert!(!d("1234567890123456").fits_f64());
        assert!(!d("12345678901234567890123").fits_f64());
        assert!(!d("0.1234567890123456789").fits_f64());
    }

    #[test]
    fn cmp_exact_orders_values_f64_cannot_tell_apart() {
        let a = d("12345678901234567890001");
        let b = d("12345678901234567890002");
        assert_eq!(a.to_f64(), b.to_f64());
        assert_eq!(a.cmp_exact(&b), Ordering::Less);
        assert_eq!(d("-5").cmp_exact(&d("-40")), Ordering::Greater);
        assert_eq!(d("-1").cmp_exact(&d("0")), Ordering::Less);
        assert_eq!(d("1.50").cmp_exact(&d("1.5")), Ordering::Equal);
        assert_eq!(d("0.05").cmp_exact(&d("0.1")), Ordering::Less);
        assert_eq!(d("9.99").cmp_exact(&d("10")), Ordering::Less);
    }

    #[test]
    fn rounded_magnitude_rounds_half_away_and_pads() {
        assert_eq!(d("1.005").rounded_magnitude(2), "1.01");
        assert_eq!(d("-1.005").rounded_magnitude(2), "1.01");
        assert_eq!(d("9.995").rounded_magnitude(2), "10.00");
        assert_eq!(d("99.5").rounded_magnitude(0), "100");
        assert_eq!(d("1.2").rounded_magnitude(4), "1.2000");
        assert_eq!(
            d("12345678901234567890.4").rounded_magnitude(0),
            "12345678901234567890"
        );
        assert_eq!(
            d("0.123456789012345678").rounded_magnitude(17),
            "0.12345678901234568"
        );
    }
}
