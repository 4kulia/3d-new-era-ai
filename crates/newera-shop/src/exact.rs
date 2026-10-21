//! Exact fractions, for the arithmetic of an estimate.
//!
//! An estimate divides — m² by m² per pack, litres by litres per can — and
//! then rounds up to whole packs. In floating point, 3.24 m² over 1.62 m² per
//! pack is 2.0000000000000004, which rounds up to 3 packs; as a fraction it
//! is 2. Every amount here is an integer over an integer, kept reduced, and
//! an operation that would overflow says so (`None`) instead of wrapping.

use std::fmt;

use crate::decimal::Decimal;

/// `num / den`, reduced, with `den > 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exact {
    num: i128,
    den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

impl Exact {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const ONE: Self = Self { num: 1, den: 1 };

    /// `num / den`; `None` for a zero denominator.
    pub fn ratio(num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let sign = if den < 0 { -1 } else { 1 };
        let g = gcd(num, den);
        Some(Self {
            num: sign * num / g,
            den: sign * den / g,
        })
    }

    pub fn int(n: i128) -> Self {
        Self { num: n, den: 1 }
    }

    /// The decimal, exactly.
    pub fn of(decimal: Decimal) -> Self {
        let (units, scale) = decimal.parts();
        Self::ratio(i128::from(units), 10i128.pow(scale)).unwrap_or(Self::ZERO)
    }

    /// A number as JSON writes it — `1.62`, `80.0`, `10` — read as the
    /// decimal its text is, not as the binary float nearest to it.
    pub fn of_json(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::Number(n) => Decimal::parse(&n.to_string()).map(Self::of),
            serde_json::Value::String(s) => Decimal::parse(s).map(Self::of),
            _ => None,
        }
    }

    pub fn plus(self, other: Self) -> Option<Self> {
        let num = self
            .num
            .checked_mul(other.den)?
            .checked_add(other.num.checked_mul(self.den)?)?;
        Self::ratio(num, self.den.checked_mul(other.den)?)
    }

    pub fn times(self, other: Self) -> Option<Self> {
        // Cross-reduce first, so the products stay as small as they can.
        let a = gcd(self.num, other.den);
        let b = gcd(other.num, self.den);
        Self::ratio(
            (self.num / a).checked_mul(other.num / b)?,
            (self.den / b).checked_mul(other.den / a)?,
        )
    }

    /// `None` for a division by zero, or an overflow.
    pub fn over(self, other: Self) -> Option<Self> {
        if other.num == 0 {
            return None;
        }
        self.times(Self::ratio(other.den, other.num)?)
    }

    /// The smallest whole number not below it.
    pub fn ceil(self) -> i128 {
        let q = self.num.div_euclid(self.den);
        if self.num.rem_euclid(self.den) == 0 {
            q
        } else {
            q + 1
        }
    }

    /// The largest whole number not above it.
    pub fn floor(self) -> i128 {
        self.num.div_euclid(self.den)
    }

    pub fn is_positive(self) -> bool {
        self.num > 0
    }

    /// Up to `scale` decimals, rounded up: a quantity to buy is never less
    /// than what is needed.
    pub fn round_up(self, scale: u32) -> Option<Decimal> {
        let shift = Self::int(10i128.pow(scale));
        let units = i64::try_from(self.times(shift)?.ceil()).ok()?;
        Some(Decimal::new(units, scale))
    }

    /// To `scale` decimals, half away from zero: money, to the cent.
    pub fn round_half(self, scale: u32) -> Option<Decimal> {
        let shifted = self.times(Self::int(10i128.pow(scale)))?;
        let twice = shifted.times(Self::int(2))?;
        // round(x) = floor(x + 1/2) for x ≥ 0, mirrored below zero.
        let units = if twice.num >= 0 {
            (twice.num + twice.den).div_euclid(2 * twice.den)
        } else {
            -((-twice.num + twice.den).div_euclid(2 * twice.den))
        };
        Some(Decimal::new(i64::try_from(units).ok()?, scale))
    }

    /// The decimal it is, when it is one of at most six decimals — what an
    /// amount made of decimals by `add` and `mul` always is.
    pub fn decimal(self) -> Option<Decimal> {
        (0..=6).find_map(|scale| {
            let shifted = self.times(Self::int(10i128.pow(scale)))?;
            (shifted.den == 1)
                .then(|| {
                    i64::try_from(shifted.num)
                        .ok()
                        .map(|u| Decimal::new(u, scale))
                })
                .flatten()
        })
    }
}

/// As a decimal when it is one, else `num/den`.
impl fmt::Display for Exact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.decimal() {
            Some(d) => write!(f, "{d}"),
            None => write!(f, "{}/{}", self.num, self.den),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Exact;
    use crate::decimal::Decimal;

    fn x(text: &str) -> Exact {
        Exact::of(Decimal::parse(text).unwrap())
    }

    #[test]
    fn divides_without_the_float_error() {
        assert_eq!(x("3.24").over(x("1.62")), Some(Exact::int(2)));
        assert_eq!(x("3.24").over(x("1.62")).unwrap().ceil(), 2);
        assert_eq!(x("3.25").over(x("1.62")).unwrap().ceil(), 3);
        assert_eq!(x("1").over(Exact::ZERO), None);
        assert_eq!(Exact::ratio(1, 0), None);
    }

    #[test]
    fn rounds_whole_numbers_both_ways() {
        let cases = [
            ("2", 2, 2),
            ("2.001", 3, 2),
            ("1.999", 2, 1),
            ("0", 0, 0),
            ("-1.5", -1, -2),
        ];
        for (text, ceil, floor) in cases {
            assert_eq!(x(text).ceil(), ceil, "ceil {text}");
            assert_eq!(x(text).floor(), floor, "floor {text}");
        }
    }

    #[test]
    fn rounds_to_decimals() {
        let up = |e: Exact, s| e.round_up(s).unwrap().to_string();
        let half = |e: Exact, s| e.round_half(s).unwrap().to_string();
        assert_eq!(up(x("9.4501"), 2), "9.46");
        assert_eq!(up(x("9.45"), 2), "9.45");
        assert_eq!(up(Exact::ratio(1, 3).unwrap(), 2), "0.34");
        assert_eq!(half(x("15.61005"), 2), "15.61");
        assert_eq!(half(x("15.615"), 2), "15.62");
        assert_eq!(half(x("15.6149"), 2), "15.61");
        assert_eq!(half(x("-15.615"), 2), "-15.62");
        assert_eq!(half(Exact::ratio(2, 3).unwrap(), 2), "0.67");
        assert_eq!(half(x("7"), 2), "7.00");
    }

    #[test]
    fn adds_and_multiplies_exactly() {
        assert_eq!(x("0.1").plus(x("0.2")), Some(x("0.3")));
        assert_eq!(
            x("24.54").times(Exact::int(6)).unwrap().to_string(),
            "147.24"
        );
        assert_eq!(
            x("1.395").times(x("11.19")).unwrap().to_string(),
            "15.61005"
        );
        assert_eq!(Exact::ratio(1, 3).unwrap().to_string(), "1/3");
        assert_eq!(Exact::int(i128::MAX).plus(Exact::ONE), None);
        assert_eq!(Exact::int(i128::MAX).times(Exact::int(2)), None);
    }

    #[test]
    fn reads_json_numbers_as_their_text() {
        use serde_json::json;
        assert_eq!(Exact::of_json(&json!(1.62)), Some(x("1.62")));
        assert_eq!(Exact::of_json(&json!(80.0)), Some(Exact::int(80)));
        assert_eq!(Exact::of_json(&json!("24.54")), Some(x("24.54")));
        assert_eq!(Exact::of_json(&json!(true)), None);
    }
}
