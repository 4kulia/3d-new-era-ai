//! Amounts as a shop writes them, kept exact.
//!
//! A price is money: it is never a float. Dutch pages write `24,54`,
//! `1.299,00` and `135,-`; `AnySite`'s adapters and accessibility labels write
//! `17.59`; a pack is `1.395 m²` on one page and `1,62 m²` on the next. One
//! reading covers them all, and the value goes out as the canonical string
//! `24.54`.

use std::fmt;

/// An exact decimal: `units / 10^scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal {
    units: i64,
    scale: u32,
}

impl Decimal {
    /// Reads a number as a Dutch page or an adapter writes it.
    ///
    /// - both `.` and `,`: the last one is the decimal mark (`1.299,00`);
    /// - only `,`: it is the decimal mark (`24,54`); `135,-` is whole;
    /// - only one `.`: it is the decimal mark (`17.59`, `1.395`) — a pack of
    ///   1.395 m² is what shops mean, and no measured page groups thousands
    ///   without cents;
    /// - several `.` and no `,`: thousands (`1.234.567`).
    ///
    /// A currency sign or spaces around it are dropped; anything else is not
    /// a number.
    pub fn parse(text: &str) -> Option<Self> {
        let cleaned: String = text
            .trim()
            .trim_start_matches('€')
            .trim_end_matches('€')
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let whole = cleaned
            .strip_suffix(",-")
            .or_else(|| cleaned.strip_suffix(",–"));
        if let Some(whole) = whole {
            return Self::digits(&whole.replace('.', ""), "");
        }
        let (negative, body) = match cleaned.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, cleaned.as_str()),
        };
        let commas = body.matches(',').count();
        let dots = body.matches('.').count();
        let (int, frac) = match (commas, dots) {
            (0, 0) => (body.to_owned(), String::new()),
            (1, 0) => split(body, ','),
            (0, 1) => split(body, '.'),
            (0, _) => (body.replace('.', ""), String::new()),
            (_, 0) => return None,
            _ => {
                let (mark, other, count) = if body.rfind(',') > body.rfind('.') {
                    (',', '.', commas)
                } else {
                    ('.', ',', dots)
                };
                if count != 1 {
                    return None;
                }
                let (int, frac) = split(body, mark);
                (int.replace(other, ""), frac)
            }
        };
        let value = Self::digits(&int, &frac)?;
        Some(if negative {
            Self {
                units: -value.units,
                ..value
            }
        } else {
            value
        })
    }

    fn digits(int: &str, frac: &str) -> Option<Self> {
        let all_digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
        if int.is_empty() || !all_digits(int) || !all_digits(frac) || frac.len() > 6 {
            return None;
        }
        let scale = u32::try_from(frac.len()).ok()?;
        let units = format!("{int}{frac}").parse::<i64>().ok()?;
        Some(Self { units, scale })
    }

    /// The value as a float, for measures that are not money.
    #[allow(clippy::cast_precision_loss, reason = "measures, never money")]
    pub fn to_f64(self) -> f64 {
        self.units as f64 / 10f64.powi(i32::try_from(self.scale).unwrap_or(0))
    }

    /// Whether it is above zero.
    pub fn is_positive(self) -> bool {
        self.units > 0
    }
}

fn split(text: &str, mark: char) -> (String, String) {
    match text.rsplit_once(mark) {
        Some((int, frac)) => (int.to_owned(), frac.to_owned()),
        None => (text.to_owned(), String::new()),
    }
}

/// Canonical: `24.54`, `1.395`, `135` — the digits the shop wrote, with a
/// dot.
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.units < 0 { "-" } else { "" };
        let digits = self.units.unsigned_abs().to_string();
        let scale = self.scale as usize;
        if scale == 0 {
            return write!(f, "{sign}{digits}");
        }
        let padded = format!("{digits:0>width$}", width = scale + 1);
        let (int, frac) = padded.split_at(padded.len() - scale);
        write!(f, "{sign}{int}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::Decimal;

    fn read(text: &str) -> Option<String> {
        Decimal::parse(text).map(|d| d.to_string())
    }

    /// Every way a measured page or adapter writes an amount.
    #[test]
    fn reads_what_shops_write() {
        let cases = [
            ("24,54", Some("24.54")),
            ("17.59", Some("17.59")),
            ("1.395", Some("1.395")),
            ("1,62", Some("1.62")),
            ("1.299,00", Some("1299.00")),
            ("1,299.00", Some("1299.00")),
            ("135,-", Some("135")),
            ("€ 14,99", Some("14.99")),
            ("18,13 €", Some("18.13")),
            ("1.234.567", Some("1234567")),
            ("0,05", Some("0.05")),
            ("42", Some("42")),
            ("-3,5", Some("-3.5")),
            ("", None),
            ("abc", None),
            ("1,2,3", None),
            ("12 x 3", None),
        ];
        for (text, want) in cases {
            assert_eq!(read(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn floats_only_for_measures() {
        assert!((Decimal::parse("1,62").unwrap().to_f64() - 1.62).abs() < 1e-12);
        assert!(Decimal::parse("0,01").unwrap().is_positive());
        assert!(!Decimal::parse("0").unwrap().is_positive());
    }
}
