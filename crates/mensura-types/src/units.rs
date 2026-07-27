//! Physical dimensions: integer exponent vectors over the seven SI base
//! dimensions (ADR 0026, `docs/language/11-physical-units.md`).
//!
//! A [`Dimension`] is an element of the free abelian group of rank seven:
//! dimensions multiply while their exponent vectors add, and the group
//! identity (all exponents zero) is the dimensionless quantity.  Equality is
//! vector equality, so the dimension-mismatch check is a decision procedure.
//! The group is mechanized in `formal/Mensura/Units/Dimension.lean`; this
//! module is its executable mirror.

use std::fmt;
use std::ops::{Div, Mul};

use crate::model::ColumnType;

/// The seven base dimensions, in the fixed axis order of the exponent
/// vector.  The names are lowercase built-in type names
/// (`docs/language/05-naming-and-casing.md`).
pub const BASE_DIMENSIONS: [&str; 7] = [
    "time",
    "length",
    "mass",
    "current",
    "temperature",
    "amount",
    "luminosity",
];

/// The seven intrinsic base units, paired with their base dimension by
/// index (ADR 0026, Decision 6).  Each is an ambient value binding of
/// magnitude one.
pub const BASE_UNITS: [&str; 7] = [
    "second", "meter", "kilogram", "ampere", "kelvin", "mole", "candela",
];

/// A physical dimension: an integer exponent per base dimension, indexed by
/// [`BASE_DIMENSIONS`] order.  Arithmetic saturates rather than wraps, so a
/// pathological exponent chain degrades into a wrong-but-finite diagnostic
/// instead of undefined behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dimension([i32; 7]);

impl Dimension {
    /// The group identity: the dimensionless quantity.
    pub const DIMENSIONLESS: Dimension = Dimension([0; 7]);

    /// The base dimension named `name` (exponent one on its own axis), or
    /// `None` if `name` is not one of the seven.
    pub fn base(name: &str) -> Option<Dimension> {
        let idx = BASE_DIMENSIONS.iter().position(|d| *d == name)?;
        let mut exps = [0; 7];
        exps[idx] = 1;
        Some(Dimension(exps))
    }

    /// The dimension of the intrinsic base unit `name` (`meter`,
    /// `second`, ...), or `None` if `name` is not one of the seven.
    pub fn of_base_unit(name: &str) -> Option<Dimension> {
        let idx = BASE_UNITS.iter().position(|u| *u == name)?;
        let mut exps = [0; 7];
        exps[idx] = 1;
        Some(Dimension(exps))
    }

    /// Whether this is the group identity (all exponents zero).
    pub fn is_dimensionless(&self) -> bool {
        self.0 == [0; 7]
    }

    /// Integer power: exponents scale.
    #[must_use]
    pub fn pow(self, n: i32) -> Dimension {
        Dimension(std::array::from_fn(|i| self.0[i].saturating_mul(n)))
    }

    /// This dimension applied to the `real` backing, as a column domain:
    /// the smart constructor that collapses the group identity to bare
    /// `real`, so [`ColumnType::Quantity`] never holds the dimensionless
    /// vector (ADR 0026, Decision 7: a dimensionless result is `real`).
    pub fn applied(self) -> ColumnType {
        if self.is_dimensionless() {
            ColumnType::Real
        } else {
            ColumnType::Quantity(self)
        }
    }

    /// The exponent vector, in [`BASE_DIMENSIONS`] order.
    pub fn exponents(&self) -> [i32; 7] {
        self.0
    }

    /// Render the applied type for diagnostics: `temperature[real]` for a
    /// single base dimension, `(length / time^2)[real]` otherwise.
    pub fn type_name(&self) -> String {
        let s = self.to_string();
        if BASE_DIMENSIONS.contains(&s.as_str()) {
            format!("{s}[real]")
        } else {
            format!("({s})[real]")
        }
    }
}

/// Dimension product: exponent vectors add.
impl Mul for Dimension {
    type Output = Dimension;

    fn mul(self, other: Dimension) -> Dimension {
        Dimension(std::array::from_fn(|i| {
            self.0[i].saturating_add(other.0[i])
        }))
    }
}

/// Dimension quotient: exponent vectors subtract.
impl Div for Dimension {
    type Output = Dimension;

    fn div(self, other: Dimension) -> Dimension {
        Dimension(std::array::from_fn(|i| {
            self.0[i].saturating_sub(other.0[i])
        }))
    }
}

/// The canonical human form: positive-exponent factors, then `/` and the
/// negative-exponent factors (`length / time^2`, parenthesized when the
/// denominator has several factors).  A dimension with no positive part
/// renders its negative exponents explicitly (`time^-1`).
impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "dimensionless");
        }
        let factor = |name: &str, exp: i32| {
            if exp == 1 {
                name.to_string()
            } else {
                format!("{name}^{exp}")
            }
        };
        let pos: Vec<String> = BASE_DIMENSIONS
            .iter()
            .zip(self.0)
            .filter(|(_, e)| *e > 0)
            .map(|(n, e)| factor(n, e))
            .collect();
        let neg: Vec<String> = BASE_DIMENSIONS
            .iter()
            .zip(self.0)
            .filter(|(_, e)| *e < 0)
            .map(|(n, e)| factor(n, -e))
            .collect();
        if pos.is_empty() {
            // No positive part: render the negative exponents explicitly.
            let all: Vec<String> = BASE_DIMENSIONS
                .iter()
                .zip(self.0)
                .filter(|(_, e)| *e != 0)
                .map(|(n, e)| factor(n, e))
                .collect();
            return write!(f, "{}", all.join(" * "));
        }
        write!(f, "{}", pos.join(" * "))?;
        match neg.len() {
            0 => Ok(()),
            1 => write!(f, " / {}", neg[0]),
            _ => write!(f, " / ({})", neg.join(" * ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_operations_add_exponents() {
        let length = Dimension::base("length").unwrap();
        let time = Dimension::base("time").unwrap();
        let accel = length / time.pow(2);
        assert_eq!(accel.exponents(), [-2, 1, 0, 0, 0, 0, 0]);
        // A same-dimension ratio cancels to the identity.
        assert!((accel / accel).is_dimensionless());
        assert_eq!((accel / accel).applied(), ColumnType::Real);
        // `^0` collapses too.
        assert!(length.pow(0).is_dimensionless());
    }

    #[test]
    fn base_units_map_to_their_dimensions() {
        assert_eq!(Dimension::of_base_unit("meter"), Dimension::base("length"));
        assert_eq!(Dimension::of_base_unit("kilogram"), Dimension::base("mass"));
        assert_eq!(
            Dimension::of_base_unit("candela"),
            Dimension::base("luminosity")
        );
        assert_eq!(Dimension::of_base_unit("length"), None);
    }

    #[test]
    fn display_uses_the_quotient_form() {
        let length = Dimension::base("length").unwrap();
        let mass = Dimension::base("mass").unwrap();
        let time = Dimension::base("time").unwrap();
        assert_eq!(length.to_string(), "length");
        assert_eq!((length / time.pow(2)).to_string(), "length / time^2");
        // Energy (mass * length^2 / time^2): factors render in the fixed
        // axis order of `BASE_DIMENSIONS`, the canonical form.
        let energy = mass * length.pow(2) / time.pow(2);
        assert_eq!(energy.to_string(), "length^2 * mass / time^2");
        // Frequency has no positive part.
        assert_eq!(time.pow(-1).to_string(), "time^-1");
        // A many-factor denominator is parenthesized.
        let odd = length / (time.pow(2) * mass);
        assert_eq!(odd.to_string(), "length / (time^2 * mass)");
        assert_eq!(Dimension::DIMENSIONLESS.to_string(), "dimensionless");
    }

    #[test]
    fn type_name_parenthesizes_compound_dimensions() {
        let temp = Dimension::base("temperature").unwrap();
        assert_eq!(temp.type_name(), "temperature[real]");
        let time = Dimension::base("time").unwrap();
        let speed = Dimension::base("length").unwrap() / time;
        assert_eq!(speed.type_name(), "(length / time)[real]");
    }
}
