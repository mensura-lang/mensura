//! The type-level table model `Table<Qs, C>`.
//!
//! `C` (content) is the pure structure of the data; `Qs` is the four tracked
//! properties as scoped qualifiers. See `docs/language/09-typing-reference.md`
//! section 1 and `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`.

/// Table-scoped cardinality qualifier: the two-value chain
/// `Singletons` (card <= 1) <= `Bag` (card 0..many) (`09` section 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    Singletons,
    Bag,
}

impl Cardinality {
    /// Least upper bound on the chain. Any `Bag` input yields `Bag`. Used
    /// wherever an operation may raise the bound (a non-functional `left_join`,
    /// binding overlapping inputs).
    pub fn join(self, other: Cardinality) -> Cardinality {
        match (self, other) {
            (Cardinality::Singletons, Cardinality::Singletons) => Cardinality::Singletons,
            _ => Cardinality::Bag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinality_join_is_least_upper_bound() {
        use Cardinality::{Bag, Singletons};
        assert_eq!(Singletons.join(Singletons), Singletons);
        assert_eq!(Singletons.join(Bag), Bag);
        assert_eq!(Bag.join(Singletons), Bag);
        assert_eq!(Bag.join(Bag), Bag);
    }
}
