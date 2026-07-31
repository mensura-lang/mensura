//! The type-level table model `Table<Qs, C>`.
//!
//! `C` (content) is the pure structure of the data; `Qs` is the four tracked
//! properties as scoped qualifiers. See `docs/language/09-typing-reference.md`
//! section 1 and `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`.

use std::collections::BTreeSet;

use mensura_syntax::StoreKind;

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
    /// wherever an operation may raise the bound (a non-functional `lookup`,
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
/// table carries. `union` unions tag-sets; `split` adds a sibling pair;
/// `demote` / key `pivot` drop them.
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
    /// (`demote`, key `pivot`). Same value as `root`, named for the
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

    /// `union` (`09` section 6.5): union the tag-sets.
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
/// non-key columns may be missing. A column is total (always known) by
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

    /// Make a column optional (e.g. a `lookup` added it as possibly missing).
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
/// bag holds all its possible rows. A `registry` source is `Complete` by
/// mechanism (ADR 0033); a bare `store` is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Incomplete,
}

/// Whether tie-freedom of a scan's order key has been claimed by fiat
/// (ADR 0029 Decision 11's tier 3, `assume { arranged }`).
///
/// A scan is deterministic only when its order key is injective on each fiber
/// (`Mensura.IsArrangement.unique`).  The checker discharges that from a
/// grading where it can (`Mensura.keyInjOn_demote_tag`); where it cannot, the
/// obligation is admitted locally and visibly, exactly as `assume { complete }`
/// admits completeness (ADR 0017, whose block form was written to generalize
/// this way).
///
/// Not a fifth qualifier axis in spirit: it records a *claim*, not a derived
/// fact, and it is consumed by the ordered primitives only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arranged {
    /// No claim: an order key must be discharged by a grading.
    Unclaimed,
    /// The pipeline claimed its keys are tie-free.
    Assumed,
}

/// The domain-relative grade of the completeness entry (`09` section 3.4,
/// ADR 0020), not a fifth qualifier axis: the enum-domained key columns `A`
/// for which every residual key present in the table carries its `(k, v)` row
/// for **every** variant `v`. Established by `unpivot` when every folded
/// column is total; consumed by `pivot`'s totality upgrade.
pub type Exhaustive = BTreeSet<String>;

/// The key-graded cardinality facts (ADR 0024,
/// `docs/decisions/0024-key-moves-as-a-true-inverse-pair.md`): column sets
/// over the flat table, key and non-key columns alike, over which the
/// table is known **functional** (grouping by the set yields at most one
/// row, `Mensura.Functional` in `formal/`). The scalar [`Cardinality`] is
/// the derived, `S = key` instance: a table is `Singletons` exactly when
/// some grading is a subset of the current key (functionality is monotone
/// upward, so a finer key stays functional). A grading is a fact about the
/// flat table, indifferent to which of its columns currently form the key:
/// the key moves re-derive cardinality and never touch the gradings, the
/// content-identity stages carry them, and every other operation resets
/// them to match its own output cardinality (the conservative rule until
/// the per-op transport table is mechanized).
pub type Functional = BTreeSet<BTreeSet<String>>;

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
    pub key: Vec<Column>,
    pub columns: Vec<Column>,
}

/// The qualifier row `Qs` (`09` section 1): the four tracked properties, each at
/// its scope. Concrete and closed in the M0 freeze.
#[derive(Clone, Debug, PartialEq)]
pub struct Qualifiers {
    pub cardinality: Cardinality,
    pub totality: Totality,
    pub completeness: Completeness,
    /// The second grade of the completeness entry (ADR 0020); see
    /// [`Exhaustive`].
    pub exhaustive: Exhaustive,
    /// The key-graded cardinality facts (ADR 0024); see [`Functional`].
    pub functional: Functional,
    /// Whether tie-freedom was claimed by fiat; see [`Arranged`].
    pub arranged: Arranged,
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
    /// (`docs/language/10-views.md`, "Sources resolve by name"). Cardinality
    /// is the store's declared one (ADR 0022): `Singletons` for a plain
    /// `attr` store (the ADR 0001 discipline), `Bag` for an `attr*` store
    /// (many observations per entity). Index columns are total; non-key
    /// columns optional per their declared `?`. A bare store is `Incomplete`;
    /// a `registry` is `Complete` by mechanism (`09` section 8, ADR 0033),
    /// at its declared boundary whatever its cardinality. Both are untagged.
    pub fn from_store(schema: &Schema) -> TableType {
        let mut key = Vec::new();
        let mut columns = Vec::new();
        let mut totality = Totality::all_total();
        for col in &schema.columns {
            let structural = Column {
                name: col.name.clone(),
                domain: col.ty.clone(),
            };
            match col.role {
                ColumnRole::Key => key.push(structural),
                ColumnRole::Attr => {
                    if col.optional {
                        totality.mark_optional(col.name.clone());
                    }
                    columns.push(structural);
                }
            }
        }
        // Completeness by mechanism (ADR 0033 decision 2): a registry's
        // declaration is the sole intake for its observations and the intake
        // only appends, so what it holds for a key is everything there is.
        // The fact is trivially true at `Singletons` (the
        // `fiberCompleteWrt_of_functional` corollary) and contentful at
        // `Bag`, where it pins the reference population per entity; one rule
        // covers both so the keyword means one thing wherever it appears.
        let completeness = match schema.kind {
            StoreKind::Registry => Completeness::Complete,
            StoreKind::Store => Completeness::Incomplete,
        };
        let mut table = TableType {
            content: Content { key, columns },
            qualifiers: Qualifiers {
                cardinality: schema.cardinality,
                totality,
                completeness,
                exhaustive: Exhaustive::new(),
                functional: Functional::new(),
                arranged: Arranged::Unclaimed,
                lineage: Lineage::root(),
            },
        };
        // A `singletons` store seeds its key as a grading (ADR 0024): the
        // flat table is functional over the declared key by the ADR 0001
        // discipline. A `bag` store starts with no grading.
        table.sync_functional();
        table
    }

    /// The current key as a name set, the right-hand side of the grading
    /// subset check (ADR 0024).
    pub fn key_names(&self) -> BTreeSet<String> {
        self.content.key.iter().map(|c| c.name.clone()).collect()
    }

    /// Re-derive the scalar cardinality from the gradings (ADR 0024):
    /// `Singletons` exactly when some grading is a subset of the current
    /// key. Called by the key moves, which change the key and never the
    /// gradings.
    pub fn derive_cardinality(&mut self) {
        let key = self.key_names();
        self.qualifiers.cardinality = if self
            .qualifiers
            .functional
            .iter()
            .any(|grading| grading.is_subset(&key))
        {
            Cardinality::Singletons
        } else {
            Cardinality::Bag
        };
    }

    /// Reset the gradings to match the scalar cardinality (ADR 0024): the
    /// key itself if `Singletons`, nothing if `Bag`. The conservative rule
    /// for every operation without a mechanized transport witness; the key
    /// moves derive instead of reset, and the content-identity stages
    /// (`assume`, `completeness_check`) carry the input's gradings.
    pub fn sync_functional(&mut self) {
        self.qualifiers.functional = match self.qualifiers.cardinality {
            Cardinality::Singletons => {
                let mut gradings = Functional::new();
                gradings.insert(self.key_names());
                gradings
            }
            Cardinality::Bag => Functional::new(),
        };
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
        // union reconstructs both tags.
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
                key: vec![Column {
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
                exhaustive: Exhaustive::new(),
                functional: Functional::new(),
                arranged: Arranged::Unclaimed,
                lineage: Lineage::root(),
            },
        };
        assert_eq!(t.content.key.len(), 1);
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
            kind: StoreKind::Store,
            unit: "Machine".to_string(),
            columns: vec![
                col("machine", ColumnType::String, ColumnRole::Key, false),
                col("temperature", ColumnType::Real, ColumnRole::Attr, false),
                col("note", ColumnType::String, ColumnRole::Attr, true),
            ],
            cardinality: Cardinality::Singletons,
            foreign_keys: Vec::new(),
            span: Span::new(0, 0),
        };

        let t = TableType::from_store(&schema);

        // Structure: key vs non-key split by role.
        assert_eq!(t.content.key.len(), 1);
        assert_eq!(t.content.key[0].name, "machine");
        assert_eq!(t.content.columns.len(), 2);

        // Qualifiers: store boundary is singletons, incomplete, untagged.
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        assert_eq!(t.qualifiers.lineage, Lineage::root());

        // Totality: the optional column is recorded; the key never is.
        assert!(t.qualifiers.totality.is_optional("note"));
        assert!(t.qualifiers.totality.is_total("temperature"));
        assert!(t.qualifiers.totality.is_total("machine"));
    }

    #[test]
    fn from_store_lifts_the_declared_bag_cardinality() {
        // An `attr*` store enters the pipeline as a `Bag` (ADR 0022); it is
        // still `Incomplete` (establishment is an annotation, a check, or a
        // `registry` mechanism).
        let schema = Schema {
            store: "readings".to_string(),
            kind: StoreKind::Store,
            unit: "Machine".to_string(),
            columns: vec![
                col("machine", ColumnType::String, ColumnRole::Key, false),
                col("kelvin", ColumnType::Real, ColumnRole::Attr, false),
            ],
            cardinality: Cardinality::Bag,
            foreign_keys: Vec::new(),
            span: Span::new(0, 0),
        };
        let t = TableType::from_store(&schema);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        // The storage shape follows: a bag store is unkeyed.
        assert!(!schema.shape().keyed);
    }

    // --- completeness by mechanism (ADR 0033 decision 2) ------------------

    fn kinded(kind: StoreKind, cardinality: Cardinality) -> TableType {
        TableType::from_store(&Schema {
            store: "readings".to_string(),
            kind,
            unit: "Machine".to_string(),
            columns: vec![
                col("machine", ColumnType::String, ColumnRole::Key, false),
                col("kelvin", ColumnType::Real, ColumnRole::Attr, false),
            ],
            cardinality,
            foreign_keys: Vec::new(),
            span: Span::new(0, 0),
        })
    }

    #[test]
    fn a_registry_is_complete_at_either_cardinality() {
        // The uniform rule: trivially true at `Singletons` (the
        // `fiberCompleteWrt_of_functional` corollary), contentful at `Bag`,
        // where it pins the reference population per entity.
        for cardinality in [Cardinality::Singletons, Cardinality::Bag] {
            let t = kinded(StoreKind::Registry, cardinality);
            assert_eq!(t.qualifiers.completeness, Completeness::Complete);
            assert_eq!(t.qualifiers.cardinality, cardinality);
        }
    }

    #[test]
    fn a_store_is_incomplete_at_either_cardinality() {
        for cardinality in [Cardinality::Singletons, Cardinality::Bag] {
            let t = kinded(StoreKind::Store, cardinality);
            assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        }
    }

    #[test]
    fn a_registry_is_otherwise_lifted_exactly_as_a_store() {
        // Everything but the completeness qualifier is kind-independent:
        // a registry materializes as the same table (ADR 0033 decision 3).
        let store = kinded(StoreKind::Store, Cardinality::Singletons);
        let registry = kinded(StoreKind::Registry, Cardinality::Singletons);
        assert_eq!(store.content, registry.content);
        assert_eq!(store.qualifiers.totality, registry.qualifiers.totality);
        assert_eq!(store.qualifiers.functional, registry.qualifiers.functional);
        assert_eq!(store.qualifiers.lineage, registry.qualifiers.lineage);
    }
}
