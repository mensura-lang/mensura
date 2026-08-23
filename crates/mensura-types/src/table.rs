//! The type-level table model `Table<Qs, C>`.
//!
//! `C` (content) is the pure structure of the data; `Qs` is the four tracked
//! properties as scoped qualifiers. See `docs/language/09-typing-reference.md`
//! section 1 and `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`.

use std::collections::{BTreeMap, BTreeSet};

use mensura_syntax::{BinOp, StoreKind};

use crate::model::{ColumnRole, ColumnType, Lateness, Schema};

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
/// bag holds all its possible rows, at the **current** key against the fixed
/// intended population (`Mensura.FiberCompleteWrt`). Held trivially by any
/// `Singletons` table (a present key's single row is its whole fiber,
/// `fiberCompleteWrt_of_functional`); at `Bag` a `registry` source carries it
/// by mechanism at its own declared key (ADR 0033) and a bare `store` does
/// not. The fact does not survive a genuine key coarsening: the key moves
/// re-derive it from the gradings, and `demote` to a `bag` clears it
/// (ADR 0035).
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

/// Which window columns carry a **rectangularity** fact (ADR 0038
/// decision 4): per residual key, the column's values are exactly the grid
/// between that entity's declared lower bound and the closed upper bound.
///
/// The sibling of [`Exhaustive`], and deliberately the same shape, because
/// it plays the same part: these are the two facts a genuinely coarsening
/// `demote` consumes instead of clearing the completeness qualifier
/// (ADR 0035's rule, with its two exceptions).  Established by `dense`,
/// which enumerates the grid, and cleared by every other operation, since
/// anything that drops or adds rows can put a hole back.
pub type Rectangles = BTreeSet<String>;

/// Which value columns are a **single fold**, and at which combiner
/// (ADR 0038 decision 2).
///
/// Recorded by a reducing `map_bags`, where the field's defining expression
/// is in hand, and read by `dense`, which fills such a column from the
/// combiner's identity and pushes every other column onto the value axis.
/// The map holds only the columns the recognizer accepted: a compound
/// expression has no entry, which is what makes "no single combiner
/// produced it" the default rather than a special case.
///
/// It is *not* a provenance model.  The pipeline ADR 0038 specifies puts
/// `dense` directly after the reduction, so the fact needs to survive
/// exactly one step and every other operation clears it.
pub type Reductions = BTreeMap<String, BinOp>;

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
    /// The live window facts (ADR 0037 decision 2); see [`Windows`].
    pub windows: Windows,
    /// The completed window grids (ADR 0038 decision 4); see [`Rectangles`].
    pub rectangles: Rectangles,
    /// The single-fold value columns (ADR 0038 decision 2); see
    /// [`Reductions`].
    pub reductions: Reductions,
    /// The source's intake contracts, carried so a windowing operation can
    /// inherit the one on its point column (ADR 0037 decision 4).  Seeded
    /// by [`TableType::from_store`] and cleared by anything that is not
    /// content-identity, since a contract is a statement about a
    /// registry's intake rather than about a derived table.
    pub contracts: Vec<Lateness>,
    pub lineage: Lineage,
}

/// `Table<Qs, C>`: structure plus scoped qualifiers (ADR 0013).
#[derive(Clone, Debug, PartialEq)]
pub struct TableType {
    pub content: Content,
    pub qualifiers: Qualifiers,
}

/// What `window` records about one window column (ADR 0037 decision 2).
///
/// The sibling of [`Exhaustive`], with a payload: `w` windows `point` at
/// this extent and stride, over a source whose intake contract it
/// inherits.  Established by the operation's construction, consumed
/// downstream by `closed`, and reset conservatively by any operation that
/// touches `w` or `point` or is not content-identity in ADR 0024's sense.
///
/// `size` and `stride` are held in the point column's **storage grain**:
/// whole milliseconds for an `instant`, a plain count for an `int`.  That
/// is the convention [`crate::model::Lateness::bound`] already uses, so
/// `closed` can compare `w + size + lateness` in one unit with no
/// conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowFact {
    /// The point column the window is over.  It stays where it was, key
    /// or attribute (ADR 0037 decision 2).
    pub point: String,
    pub size: i64,
    pub stride: i64,
    /// The source's intake contract on `point`, when it declared one.
    /// `closed` needs it and rejects the stage without it, since the
    /// establishment is mechanism-grade or nothing (decision 4).
    pub contract: Option<Lateness>,
    /// Whether `closed` has run on this grid.  `dense` demands it, because
    /// closedness is the grid's upper bound (ADR 0038 decision 3): filling
    /// past the watermark would declare a future that has not happened
    /// confirmed empty, which is the one error the stage must not make.
    pub closed: bool,
}

/// The live window facts, keyed by window column (ADR 0037 decision 2).
///
/// Encapsulated like [`Totality`] rather than exposed like [`Exhaustive`],
/// because this is the first fact carrying a payload: the clearing rule is
/// "any operation that touches the window column or its point", which is
/// easier to get right as [`Windows::clear_touching`] than as raw set
/// surgery at each of the sites that reset a fact.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Windows {
    facts: BTreeMap<String, WindowFact>,
}

impl Windows {
    /// No window facts, the state every source and every non-windowing
    /// operation starts from.
    pub fn none() -> Windows {
        Windows::default()
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// The fact for a window column, if it is live.
    pub fn get(&self, window: &str) -> Option<&WindowFact> {
        self.facts.get(window)
    }

    /// The live window columns, in name order.
    pub fn columns(&self) -> impl Iterator<Item = &String> {
        self.facts.keys()
    }

    /// Record what `window` just built.
    pub fn record(&mut self, window: impl Into<String>, fact: WindowFact) {
        self.facts.insert(window.into(), fact);
    }

    /// Record that `closed` has run on a grid (ADR 0038 decision 3).
    pub fn mark_closed(&mut self, window: &str) {
        if let Some(fact) = self.facts.get_mut(window) {
            fact.closed = true;
        }
    }

    /// Drop every fact that mentions `column`, as its window column or as
    /// its point.  The conservative rule: once either end of the relation
    /// has been touched, the checker no longer knows the grid holds.
    pub fn clear_touching(&mut self, column: &str) {
        self.facts
            .retain(|w, fact| w != column && fact.point != column);
    }

    /// Drop every fact, for an operation with no transport witness.
    pub fn clear(&mut self) {
        self.facts.clear();
    }

    /// The facts both sides agree on, for `union` (which must not invent a
    /// grid the other branch does not have).
    pub fn intersect(&self, other: &Windows) -> Windows {
        Windows {
            facts: self
                .facts
                .iter()
                .filter(|(w, fact)| other.facts.get(*w) == Some(fact))
                .map(|(w, fact)| (w.clone(), fact.clone()))
                .collect(),
        }
    }
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
        // Completeness at the source (ADR 0033 decision 2, as amended by
        // ADR 0035). A registry's declaration is the sole intake for its
        // observations and the intake only appends, so what it holds for a
        // key of its own declared boundary is everything there is: `Complete`
        // at either cardinality, trivially at `Singletons` (the
        // `fiberCompleteWrt_of_functional` corollary) and contentfully at
        // `Bag`, where it pins the reference population per entity. Any
        // `Singletons` source carries the same trivial fact (a present key's
        // single row is its whole fiber), which is what keeps the qualifier
        // consistent with the key moves' re-derivation and the reducer's
        // trivial discharge (ADR 0035). The kinds therefore differ exactly
        // at `Bag`, where the registry mechanism is the only source-level
        // establishment. The fact stops at the declared boundary either
        // way: a genuine `demote` clears it, because recording every
        // observation received is not receiving every observation that
        // happened (ADR 0035).
        let completeness = match (schema.kind, schema.cardinality) {
            (StoreKind::Registry, _) | (_, Cardinality::Singletons) => Completeness::Complete,
            (StoreKind::Store, Cardinality::Bag) => Completeness::Incomplete,
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
                windows: Windows::none(),
                rectangles: Rectangles::new(),
                reductions: Reductions::new(),
                // The intake contracts travel with the source, so `window`
                // can inherit the one on its point column and `closed` can
                // demand it (ADR 0037 decision 4).  A plain store declares
                // none, which is exactly why `closed` is unavailable over
                // one.
                contracts: schema.lateness.clone(),
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
                windows: Windows::none(),
                rectangles: Rectangles::new(),
                reductions: Reductions::new(),
                contracts: Vec::new(),
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
            lateness: Vec::new(),
            span: Span::new(0, 0),
        };

        let t = TableType::from_store(&schema);

        // Structure: key vs non-key split by role.
        assert_eq!(t.content.key.len(), 1);
        assert_eq!(t.content.key[0].name, "machine");
        assert_eq!(t.content.columns.len(), 2);

        // Qualifiers: store boundary is singletons and untagged.  At
        // `Singletons` the completeness qualifier is the trivial corollary
        // (a present key's single row is its whole fiber, ADR 0035), so it
        // is derived from the cardinality, kind notwithstanding.
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
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
            lateness: Vec::new(),
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
            lateness: Vec::new(),
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
    fn a_store_is_incomplete_exactly_at_bag() {
        // ADR 0035: the kinds differ exactly at `Bag`.  A `singletons` store
        // enters `Complete` on the trivial corollary (a present key's single
        // row is its whole fiber), so the registry's type-level content
        // lives entirely at `Bag`, where its mechanism is the only
        // source-level establishment.
        let t = kinded(StoreKind::Store, Cardinality::Bag);
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        let t = kinded(StoreKind::Store, Cardinality::Singletons);
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
    }

    #[test]
    fn a_registry_is_otherwise_lifted_exactly_as_a_store() {
        // Everything but the completeness qualifier is kind-independent:
        // a registry materializes as the same table (ADR 0033 decision 3).
        // `Bag` is where the qualifier actually differs (ADR 0035), so that
        // is where the "otherwise" is contentful.
        let store = kinded(StoreKind::Store, Cardinality::Bag);
        let registry = kinded(StoreKind::Registry, Cardinality::Bag);
        assert_ne!(
            store.qualifiers.completeness,
            registry.qualifiers.completeness
        );
        assert_eq!(store.content, registry.content);
        assert_eq!(store.qualifiers.totality, registry.qualifiers.totality);
        assert_eq!(store.qualifiers.functional, registry.qualifiers.functional);
        assert_eq!(store.qualifiers.lineage, registry.qualifiers.lineage);
    }
}
