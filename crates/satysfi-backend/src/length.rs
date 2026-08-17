use newer_type::implement;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// A physical length in PDF points (1/72 inch), the `length.ml` unit.
///
/// `Add`, `Sub`, `Neg` and `Mul<f64>` stay hand-written: `newer-type`'s
/// generic `ops` markers forward the *inner* type's `Output` verbatim (so
/// `newer_type_std::ops::Add` on an `f64` newtype yields `Output = f64`, not
/// `Output = Self` — confirmed empirically, and the macro has no syntax for
/// pinning an associated type: `newer-type-macro`'s `Implementor::parse`
/// explicitly rejects `AssocType` arguments in the `#[implement(...)]` list).
/// Using it here would silently change `Length + Length` from `Length` to
/// `f64`, so those four impls keep their hand-written `Output = Length`
/// bodies. `AddAssign` has no `Output` to lose, so it converts cleanly.
#[implement(newer_type_std::ops::AddAssign)]
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Length(pub f64);

impl Length {
    pub const ZERO: Length = Length(0.0);

    pub fn pt(v: f64) -> Length {
        Length(v)
    }

    /// Interpret a `LENGTHCONST` unit suffix (the v0.0.6 unit set).
    pub fn from_unit(value: f64, unit: &str) -> Option<Length> {
        let pt = match unit {
            "pt" => value,
            "mm" => value * 72.0 / 25.4,
            "cm" => value * 72.0 / 2.54,
            "inch" => value * 72.0,
            _ => return None,
        };
        Some(Length(pt))
    }

    pub fn max(self, other: Length) -> Length {
        Length(self.0.max(other.0))
    }

    pub fn min(self, other: Length) -> Length {
        Length(self.0.min(other.0))
    }

    pub fn is_positive(self) -> bool {
        self.0 > 0.0
    }
}

impl Add for Length {
    type Output = Length;
    fn add(self, rhs: Length) -> Length {
        Length(self.0 + rhs.0)
    }
}

impl Sub for Length {
    type Output = Length;
    fn sub(self, rhs: Length) -> Length {
        Length(self.0 - rhs.0)
    }
}

impl Neg for Length {
    type Output = Length;
    fn neg(self) -> Length {
        Length(-self.0)
    }
}

impl Mul<f64> for Length {
    type Output = Length;
    fn mul(self, rhs: f64) -> Length {
        Length(self.0 * rhs)
    }
}

impl Div<Length> for Length {
    type Output = f64;
    fn div(self, rhs: Length) -> f64 {
        self.0 / rhs.0
    }
}

impl std::fmt::Display for Length {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}pt", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        assert_eq!(Length(2.0) + Length(3.0), Length(5.0));
        assert_eq!(-Length(1.0), Length(-1.0));
        assert!(Length::pt(3.0) < Length::pt(4.0));
        assert_eq!(Length(6.0) / Length(3.0), 2.0);
        assert_eq!(Length(2.0) * 1.5, Length(3.0));
        let mut acc = Length(1.0);
        acc += Length(2.5);
        assert_eq!(acc, Length(3.5));
    }
}
