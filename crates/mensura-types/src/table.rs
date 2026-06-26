//! The type-level table model `Table<Qs, C>`.
//!
//! `C` (content) is the pure structure of the data; `Qs` is the four tracked
//! properties as scoped qualifiers. See `docs/language/09-typing-reference.md`
//! section 1 and `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`.

use std::collections::BTreeSet;

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

/// A fresh identity per `split` site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SplitId(pub u32);

/// Which side of a split a branch descends into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Left,
    Right,
}

/// One step of a tag: a split and the side taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Branch {
    pub split: SplitId,
    pub side: Side,
}

/// The path of branches from the root to a table's position.
pub type Tag = Vec<Branch>;

/// Table-scoped lineage qualifier (`09` sections 3.5, 9): the set of tags a
/// table carries. `bind` unions tag-sets; `split` adds a sibling pair;
/// `shrink_key` / index `pivot` drop them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lineage {
    pub tags: BTreeSet<Tag>,
}

impl Lineage {
    /// A table with no split ancestry.
    pub fn root() -> Lineage {
        Lineage {
            tags: BTreeSet::new(),
        }
    }

    /// The lineage after a key change drops the branch structure
    /// (`shrink_key`, index `pivot`). Same value as `root`, named for the
    /// call site.
    pub fn dropped() -> Lineage {
        Lineage::root()
    }

    /// `split` (`09` section 6.5): extend every tag with one sibling branch per
    /// side under a fresh split id. A root table gets a single one-branch tag.
    pub fn split(&self, id: SplitId) -> (Lineage, Lineage) {
        (self.extend(id, Side::Left), self.extend(id, Side::Right))
    }

    fn extend(&self, id: SplitId, side: Side) -> Lineage {
        let branch = Branch { split: id, side };
        if self.tags.is_empty() {
            let mut tags = BTreeSet::new();
            tags.insert(vec![branch]);
            return Lineage { tags };
        }
        let tags = self
            .tags
            .iter()
            .map(|tag| {
                let mut tag = tag.clone();
                tag.push(branch);
                tag
            })
            .collect();
        Lineage { tags }
    }

    /// `bind` (`09` section 6.5): union the tag-sets.
    pub fn union(&self, other: &Lineage) -> Lineage {
        Lineage {
            tags: self.tags.union(&other.tags).cloned().collect(),
        }
    }

    /// Structural disjointness (`09` section 9, ADR 0013): disjoint when some
    /// split has this table entirely on one side and the other entirely on the
    /// opposite side. Sound because a split's sides are disjoint
    /// (`split_disjoint`); decidable as a tree-position test, no solver.
    pub fn disjoint(&self, other: &Lineage) -> bool {
        if self.tags.is_empty() || other.tags.is_empty() {
            return false;
        }
        let Some(first) = self.tags.iter().next() else {
            return false;
        };
        for branch in first {
            let id = branch.split;
            if let (Some(a), Some(b)) = (self.uniform_side(id), other.uniform_side(id)) {
                if a != b {
                    return true;
                }
            }
        }
        false
    }

    /// The side this table takes at split `id`, if *every* tag passes through
    /// `id` on the *same* side; otherwise `None`.
    fn uniform_side(&self, id: SplitId) -> Option<Side> {
        let mut seen: Option<Side> = None;
        for tag in &self.tags {
            let side = tag.iter().find(|b| b.split == id).map(|b| b.side)?;
            match seen {
                None => seen = Some(side),
                Some(s) if s == side => {}
                Some(_) => return None,
            }
        }
        seen
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

    #[test]
    fn split_halves_are_disjoint_and_bind_unions() {
        let data = Lineage::root();
        let (train, test) = data.split(SplitId(0));
        assert!(train.disjoint(&test));
        assert!(test.disjoint(&train));

        let merged = train.union(&test);
        // The whole is not disjoint from either half.
        assert!(!merged.disjoint(&train));
        // bind reconstructs both tags.
        assert_eq!(merged.tags.len(), 2);
    }

    #[test]
    fn unrelated_tables_are_not_disjoint() {
        assert!(!Lineage::root().disjoint(&Lineage::root()));
        let (a, _) = Lineage::root().split(SplitId(1));
        let (b, _) = Lineage::root().split(SplitId(2));
        // No common split: cannot be decided disjoint structurally.
        assert!(!a.disjoint(&b));
    }
}
