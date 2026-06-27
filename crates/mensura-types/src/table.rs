//! The type-level table model `Table<Qs, C>`.
//!
//! `C` (content) is the pure structure of the data; `Qs` is the four tracked
//! properties as scoped qualifiers. See `docs/language/09-typing-reference.md`
//! section 1 and `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`.

use std::collections::BTreeSet;

use crate::model::{ColumnRole, ColumnType, Schema};

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
            match (self.uniform_side(id), other.uniform_side(id)) {
                (Some(a), Some(b)) if a != b => return true,
                _ => {}
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

/// Column-scoped totality qualifier (`09` section 3.3, ADR 0010): which
/// non-index columns may be missing. A column is total (always known) by
/// default; `optional` lists the exceptions. Index columns are always total and
/// never appear here.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Totality {
    optional: BTreeSet<String>,
}

impl Totality {
    /// Every column total (the default).
    pub fn all_total() -> Totality {
        Totality::default()
    }

    pub fn is_optional(&self, column: &str) -> bool {
        self.optional.contains(column)
    }

    pub fn is_total(&self, column: &str) -> bool {
        !self.is_optional(column)
    }

    /// Make a column optional (e.g. a `left_join` added it as possibly missing).
    pub fn mark_optional(&mut self, column: impl Into<String>) {
        self.optional.insert(column.into());
    }

    /// Narrow an optional column back to total (`is known`, a default, or a
    /// missingness-aware aggregate; ADR 0010).
    pub fn narrow(&mut self, column: &str) {
        self.optional.remove(column);
    }
}

/// Table-scoped completeness qualifier (`09` section 3.4): whether each key's
/// bag holds all its possible rows. A `collect` source is `Complete` by
/// mechanism; a bare `store` is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Incomplete,
}

/// A type-level structural column: a name and its domain. This is `C`-side
/// structure only; totality lives in the column-scoped [`Totality`] qualifier,
/// not here.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub domain: ColumnType,
}

/// The content `C` (`09` section 3.1): pure structure, no propagated facts.
#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub index: Vec<Column>,
    pub columns: Vec<Column>,
}

/// The qualifier row `Qs` (`09` section 1): the four tracked properties, each at
/// its scope. Concrete and closed in the M0 freeze.
#[derive(Clone, Debug, PartialEq)]
pub struct Qualifiers {
    pub cardinality: Cardinality,
    pub totality: Totality,
    pub completeness: Completeness,
    pub lineage: Lineage,
}

/// `Table<Qs, C>`: structure plus scoped qualifiers (ADR 0013).
#[derive(Clone, Debug, PartialEq)]
pub struct TableType {
    pub content: Content,
    pub qualifiers: Qualifiers,
}

impl TableType {
    /// Present a resolved store schema to the pipeline as a table value
    /// (`docs/language/10-views.md`, "Sources resolve by name"). A store is a
    /// unit tabulation (ADR 0001): `Singletons`, index columns total, non-index
    /// columns optional per their declared `?`. A bare store is `Incomplete`
    /// (only a `collect` is complete by mechanism, `09` section 8) and untagged.
    pub fn from_store(schema: &Schema) -> TableType {
        let mut index = Vec::new();
        let mut columns = Vec::new();
        let mut totality = Totality::all_total();
        for col in &schema.columns {
            let structural = Column {
                name: col.name.clone(),
                domain: col.ty.clone(),
            };
            match col.role {
                ColumnRole::Index => index.push(structural),
                ColumnRole::Const | ColumnRole::Var => {
                    if col.optional {
                        totality.mark_optional(col.name.clone());
                    }
                    columns.push(structural);
                }
            }
        }
        TableType {
            content: Content { index, columns },
            qualifiers: Qualifiers {
                cardinality: Cardinality::Singletons,
                totality,
                completeness: Completeness::Incomplete,
                lineage: Lineage::root(),
            },
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

    #[test]
    fn totality_defaults_total_and_narrows() {
        let mut t = Totality::all_total();
        assert!(t.is_total("temp"));

        t.mark_optional("temp");
        assert!(t.is_optional("temp"));
        assert!(!t.is_total("temp"));

        t.narrow("temp");
        assert!(t.is_total("temp"));
    }

    #[test]
    fn table_type_is_structure_plus_qualifiers() {
        let t = TableType {
            content: Content {
                index: vec![Column {
                    name: "id".to_string(),
                    domain: ColumnType::Int,
                }],
                columns: vec![Column {
                    name: "name".to_string(),
                    domain: ColumnType::String,
                }],
            },
            qualifiers: Qualifiers {
                cardinality: Cardinality::Singletons,
                totality: Totality::all_total(),
                completeness: Completeness::Incomplete,
                lineage: Lineage::root(),
            },
        };
        assert_eq!(t.content.index.len(), 1);
        assert_eq!(t.content.columns[0].domain, ColumnType::String);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    use crate::model::Column as StorageColumn;
    use mensura_syntax::Span;

    fn col(name: &str, ty: ColumnType, role: ColumnRole, optional: bool) -> StorageColumn {
        StorageColumn {
            name: name.to_string(),
            ty,
            role,
            optional,
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn from_store_lifts_structure_and_qualifiers() {
        let schema = Schema {
            store: "readings".to_string(),
            unit: "Machine".to_string(),
            columns: vec![
                col("machine", ColumnType::String, ColumnRole::Index, false),
                col("temperature", ColumnType::Real, ColumnRole::Var, false),
                col("note", ColumnType::String, ColumnRole::Var, true),
            ],
            span: Span::new(0, 0),
        };

        let t = TableType::from_store(&schema);

        // Structure: index vs non-index split by role.
        assert_eq!(t.content.index.len(), 1);
        assert_eq!(t.content.index[0].name, "machine");
        assert_eq!(t.content.columns.len(), 2);

        // Qualifiers: store boundary is singletons, incomplete, untagged.
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        assert_eq!(t.qualifiers.lineage, Lineage::root());

        // Totality: the optional column is recorded; the index never is.
        assert!(t.qualifiers.totality.is_optional("note"));
        assert!(t.qualifiers.totality.is_total("temperature"));
        assert!(t.qualifiers.totality.is_total("machine"));
    }
}
