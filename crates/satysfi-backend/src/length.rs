use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

/// A physical length in PDF points (1/72 inch), the `length.ml` unit.
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

impl AddAssign for Length {
    fn add_assign(&mut self, rhs: Length) {
        self.0 += rhs.0;
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
