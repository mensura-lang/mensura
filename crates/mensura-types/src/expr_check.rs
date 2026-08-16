//! Value-expression type checking (`docs/language/09-typing-reference.md`
//! section 5, `06-expressions.md`, ADR 0014).
//!
//! Types an expression against a row/group context derived from a [`TableType`].
//! Mirrors `resolve`'s contract: it collects all diagnostics rather than failing
//! on the first. Operators are gated by the scalar domain's properties
//! (equatable / orderable / numeric); typing is strict, with no `int`/`real`
//! coercion. The `|>` pipe routes to the same application path as juxtaposition
//! (ADR 0018, `docs/toolkit/01-application-checking.md`), over the intrinsics
//! (`fold` and `map`, the reduction primitives of ADR 0031; `to_real`) and
//! const functions (ADR 0030), whether named bare or qualified by a module: a
//! function value applies by the saturated-or-error rule, and a const
//! function's body re-types at each call site.  The derived reduction
//! vocabulary (`sum`, `min`, ...) is *not* here: it is const bindings in the
//! bundled `bag` module, imported like any other (ADR 0031, Decision 8).
//! Lambdas as *values in view bodies*, record/tuple literals, and `is known`
//! narrowing remain deferred.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mensura_syntax::{BinOp, Expr, ExprKind, Ident, Span, UnOp};

use crate::model::ColumnType;
use crate::table::{Arranged, Cardinality, Completeness, TableType};
use crate::units::{BASE_UNITS, Dimension};

/// The optional axis of a single value (`09` section 3.3 / 5.3). Distinct from
/// the table-scoped `table::Totality`; here it is a per-value flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optionality {
    Total,
    Optional,
}

/// The type of an expression value (`09` section 5).
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// A card-1 value carrying its domain and optionality (section 5.3).
    Value {
        domain: ColumnType,
        opt: Optionality,
    },
    /// A bag at one key, consumable only by combinators (section 5.4). It
    /// carries a totality so an aggregate can demand a total bag (ADR 0014).
    Bag {
        domain: ColumnType,
        opt: Optionality,
    },
    /// The **fiber**: the bag of rows at one key (ADR 0031, Decision 1).  This
    /// is the type of the `b` in `map_bags |k, b| ...`, and it matches
    /// `formal/Mensura/Core/Defs.lean`'s `rows : K -> Multiset (Row H σ)`
    /// exactly; the columnar record-of-bags of ADR 0015 is the *presentation*
    /// (see [`Ty::project`]), not the model.
    ///
    /// It carries each field's domain and optionality, so member access can
    /// hand back today's [`Ty::Bag`] unchanged.  Deliberately *not* a `Bag` of
    /// `Record`: nested collections do not arrive by the back door (Decision
    /// 10).  A rows value never enters a column and is not user-writable in
    /// type position; it is constructible only where groups are.
    Rows(BTreeMap<String, Ty>),
    /// A row (fields are `Value`) or a group (fields are `Bag`).
    Record(BTreeMap<String, Ty>),
    /// A const function (ADR 0030).  It carries its closure, so a saturated
    /// application is typed by binding the parameters to the arguments'
    /// types and re-typing the body at each call site: exact, per-site, no
    /// inference.  A function never enters a column and cannot be ascribed.
    Fn(Arc<TyClosure>),
    /// An orderable value marked **descending** (ADR 0031, Decision 7): the
    /// order dual, `desc e`.  Orderable in, orderable out.
    ///
    /// A variant rather than a flag on [`Ty::Value`], for the same reason
    /// [`Ty::Rows`] is one: a descending marker must never reach a column, and
    /// making it a distinct inhabitant gets that exclusion *by construction*,
    /// since `column_of` is the only door to storage and matches exhaustively.
    /// A flag would instead be silently *accepted* everywhere a `Ty::Value` is
    /// destructured, which is the wrong default for a checker-internal notion.
    /// It is not a `ColumnType` either: that is the storable vocabulary, and
    /// this is precisely a thing that cannot be stored.
    ///
    /// Never ascribable, and that falls out free: `desc` has no spelling in
    /// type position.  Because the marker sits on *values*, direction is
    /// per-component, which is what lets `|r| (r.date, desc r.priority)` mean
    /// `ORDER BY date ASC, priority DESC` (once tuple keys exist).
    Desc(Box<Ty>),
    /// A **builtin** function value (ADR 0031, Decision 11).  `fold`, `scan`,
    /// and `map` have function types but no lambda bodies: the language has no
    /// recursion and cannot express bag iteration, so they are primitives
    /// whose *types* are function-shaped.
    ///
    /// A builtin cannot be a [`Ty::Fn`], because a `TyClosure` is a lambda
    /// body rather than an arrow: there is nothing to re-type per call site.
    /// Nor is it a uniform arrow, since each slot has its own rule (a combiner
    /// token, then functions, then the bag).  So it carries the primitive's
    /// identity plus the arguments already applied, and each application step
    /// consults the primitive's own table.  A partial application is an
    /// ordinary value, which is what lets a bundled module write
    /// `let sum { fold `+` (|v| v) }`.
    Builtin(Arc<PartialBuiltin>),
}

/// Which primitive a [`Ty::Builtin`] is.  The set is closed and extends by
/// decision record, exactly as the combiner table does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    /// `fold : combiner -> (element -> value) -> bag -> value`
    /// (ADR 0031, Decision 4).  Gated by ADR 0029's Stage 1, proved in
    /// `formal/Mensura/Fold.lean`.
    Fold,
    /// `scan : combiner -> (row -> value) -> (row -> key) -> rows -> bag`
    /// (ADR 0031, Decision 7).  The ordered sibling: same combiner table, same
    /// mapper, plus an order key, emitting every intermediate where `fold`
    /// keeps the last.
    ///
    /// Both lambdas take a **row**, and the trailing argument is the fiber
    /// rather than a projected bag.  That is forced rather than chosen: sorting
    /// values by a sibling column requires seeing the row that carries both, so
    /// a projected bag (a bag of bare scalars) has nothing left to order by.
    /// ADR 0031 Decision 8 writes the derived bindings with a `(|v| v)` mapper,
    /// which cannot typecheck for this reason; the value extractor comes from
    /// the call site instead.  Gated by ADR 0029's Stage 2, proved in
    /// `formal/Mensura/Arranged.lean`.
    ///
    /// The key is an ordinary *argument* rather than a `by` clause, because a
    /// clause cannot be partially applied and the derived vocabulary is
    /// partial applications (`let cumsum { scan `+` (|v| v) }`).
    Scan,
    /// `prescan`, the exclusive scan (Blelloch's naming): position `i` carries
    /// the fold of the proper prefix `1..i-1`, so its first output is the
    /// combiner's identity, or *missing* for an identity-free row.  That is
    /// where `lag`'s missing first row comes from, and it falls out of the
    /// identity-and-emptiness rule rather than being designed
    /// (`Mensura.prescanBag`).
    Prescan,
    /// `map : (element -> value) -> bag -> bag` (ADR 0031, Decision 3): the
    /// projection functor, the explicit form of `b.x`.  Order-free, so it is
    /// not derivable from `fold` or `scan`.
    Map,
}

impl Builtin {
    /// The surface spelling, for diagnostics and for the initial environment.
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Fold => "fold",
            Builtin::Scan => "scan",
            Builtin::Prescan => "prescan",
            Builtin::Map => "map",
        }
    }

    /// How many arguments saturate this primitive.
    pub fn arity(self) -> usize {
        match self {
            // combiner, mapper, bag
            Builtin::Fold => 3,
            // combiner, mapper, key, rows.  The bag stays last so a scan pipes
            // (ADR 0018) and so the derived bindings are partial applications.
            Builtin::Scan | Builtin::Prescan => 4,
            // mapper, bag
            Builtin::Map => 2,
        }
    }

    /// Which slot takes a backticked combiner rather than a value, if any.
    ///
    /// It is the first slot for every primitive that has one, but asking the
    /// question per primitive (rather than testing `applied.is_empty()` at the
    /// call site) keeps the slot layout beside [`Builtin::arity`], where a
    /// reader adding a primitive looks, so the two cannot drift.
    pub fn combiner_slot(self) -> Option<usize> {
        match self {
            Builtin::Fold | Builtin::Scan | Builtin::Prescan => Some(0),
            Builtin::Map => None,
        }
    }

    /// Whether this primitive orders its bag, hence takes a key.
    pub fn is_ordered(self) -> bool {
        matches!(self, Builtin::Scan | Builtin::Prescan)
    }

    /// The primitive a name denotes, if any.  The set is closed, so this is
    /// also the test for "is this an intrinsic reduction".
    pub fn from_name(name: &str) -> Option<Builtin> {
        [Builtin::Fold, Builtin::Scan, Builtin::Prescan, Builtin::Map]
            .into_iter()
            .find(|b| b.name() == name)
    }
}

/// A builtin with the arguments applied so far: the value form of a partial
/// application (ADR 0030's currying, at a primitive).  Saturating it types the
/// result; short of that it stays a function value.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialBuiltin {
    pub which: Builtin,
    /// The argument *types* bound so far, in order.  A combiner slot records
    /// the operator it quoted rather than a type, since a combiner is not a
    /// value.
    pub applied: Vec<BuiltinArg>,
}

/// One argument already applied to a builtin.
#[derive(Clone, Debug, PartialEq)]
pub enum BuiltinArg {
    /// A backticked operator from the closed combiner table.
    Combiner(BinOp),
    /// Any other argument, kept as its type.
    Ty(Ty),
}

/// The checker's view of a closure (ADR 0030): the lambda plus the
/// definition-site name environment, at the type level.  `names` holds the
/// bindings *local* to the definition (captured block locals, enclosing
/// lambda parameters); free top-level names resolve through the pristine
/// ambient at application time instead, which is what keeps top-level
/// bindings order-independent (`add1`'s body may reference `add` declared
/// later).
#[derive(Clone, Debug, PartialEq)]
pub struct TyClosure {
    /// One name, or n names for the tupled form `|a, b| e`, which binds a
    /// single n-tuple parameter (ADR 0030, Decision 2).
    pub params: Vec<Ident>,
    pub body: Expr,
    pub names: BTreeMap<String, Ty>,
    /// Whether the body was parsed from a **bundled module**'s source rather
    /// than from the file being checked (`crate::modules`).  Spans carry no
    /// file identity yet, so a body diagnostic from such a closure cannot be
    /// rendered against the importing file: it is re-anchored at the call site
    /// by [`apply_closure`].  Set where the module's members enter the ambient
    /// ([`Ty::from_module`]), the one place that knows the provenance.
    pub foreign: bool,
}

impl Ty {
    /// This type as it enters the ambient from a **bundled module**: a function
    /// is marked foreign, everything else is unchanged.
    ///
    /// A module's source is a different file, and spans carry no file identity
    /// yet (`crate::modules`), so a diagnostic from inside `bag.sum`'s or
    /// `series.lag`'s body would otherwise report at an offset into the
    /// *module*: a nonsense location in the importing file, and out of bounds
    /// whenever that file is the shorter of the two.  The importer already
    /// anchors a module's *load* diagnostics at the `import` item for the same
    /// reason; this is the same policy for the ones raised at a call.
    pub fn from_module(self) -> Ty {
        match self {
            Ty::Fn(c) => Ty::Fn(Arc::new(TyClosure {
                foreign: true,
                ..(*c).clone()
            })),
            other => other,
        }
    }

    /// Project a field out of the fiber: the sugar equation of ADR 0031,
    /// Decision 2, stated once.
    ///
    /// ```text
    /// b.x  ==  map (|r| r.x) b
    /// ```
    ///
    /// In `formal/` this is `Multiset.map` at a field projection.  It is
    /// well defined because the fiber's columns are jointly indexed by the
    /// group's rows; that alignment is *provenance*, not structure a type
    /// could carry, which is why there is no generic `zip` of two arbitrary
    /// bags (ADR 0031, Alternatives).  Do not generalize it.
    fn project(fields: &BTreeMap<String, Ty>, field: &str) -> Option<Ty> {
        fields.get(field).cloned()
    }

    /// `Some(domain)` iff this is a card-1, not-missing value: the gate the
    /// scalar rule checks (section 5.3).
    pub fn known_value_domain(&self) -> Option<&ColumnType> {
        match self {
            Ty::Value {
                domain,
                opt: Optionality::Total,
            } => Some(domain),
            _ => None,
        }
    }
}

/// **The closed combiner table** (ADR 0031, Decision 6).
///
/// A backticked operator is admitted as a combiner only if it appears here.
/// The set is closed on purpose: a fold over an *unordered* bag is
/// deterministic only for associative-commutative combiners, and those are
/// laws no checker can verify on a user lambda, so the algebra is compiler
/// knowledge rather than something a call site asserts.  The mapper, by
/// contrast, stays fully open, because its obligation is a type check.  That
/// asymmetry (a law versus a type) is the whole design.
///
/// Each row records what the checker needs: which primitives admit it, the
/// identity it fabricates (as the empty case's answer, so `None` means the
/// result is optional there), and the domain property both operands need.
/// The identity and absorber columns of the ADR's table play opposite roles:
/// an absorber is *read* (a licensed short-circuit, invisible to typing),
/// while an identity is *written*, so only the identity is modelled here.
struct CombinerRow {
    op: BinOp,
    /// Spelled as the surface writes it inside backticks.
    spelling: &'static str,
    /// Whether `fold` admits this row.  A fold reduces an *unordered* bag, so
    /// it needs commutativity as well as associativity: without it the answer
    /// would depend on how the executor happened to shard the bag.  Stated as
    /// an admission rather than derived from a `commutative` flag, because the
    /// call site's question is "does this row admit this primitive", which is
    /// what `09-typing-reference.md` section 5.4's "admitted under" column
    /// answers, and because the algebra behind an admission is this table's
    /// knowledge rather than a fact to be re-derived per use.
    folds: bool,
    /// Whether `scan` admits this row.  Every row does: a scan's key supplies
    /// the order a bag lacks, so associativity alone suffices, and that is
    /// what lets the tacks be combiners at all (ADR 0029 Decision 10's
    /// ordered-only column, ADR 0031 Decision 6).
    scans: bool,
    /// Whether the domain carries an identity for this operator, i.e. whether
    /// the empty bag has a true answer.  `<<`/`>>` have none ("there is no
    /// smallest element of nothing"), which is what the `Option` completion of
    /// `formal/Mensura/Fold.lean` exists for.
    has_identity: bool,
    /// The property both operands' shared domain must have.
    domain: CombinerDomain,
}

/// The domain restriction a combiner row carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CombinerDomain {
    /// Any numeric domain, dimension included: `+` requires equal dimensions
    /// and preserves them, so `sum` works at every dimension.
    Numeric,
    /// **Dimensionless** numerics only (`int`, bare `real`).  A fold's
    /// accumulator type must be invariant, and dimensioned `*` *adds* exponent
    /// vectors (ADR 0026), so folding it would give a product whose dimension
    /// depends on the bag's cardinality, which no static type can carry.
    DimensionlessNumeric,
    /// Any orderable domain, dimension included.
    Orderable,
    /// `bool` only.
    Boolean,
    /// No restriction: the tacks discard a value rather than inspect it.
    Any,
}

/// The table.  It extends by ADR, never by a user assertion.
const COMBINERS: [CombinerRow; 8] = [
    CombinerRow {
        op: BinOp::Add,
        spelling: "+",
        folds: true,
        scans: true,
        has_identity: true,
        domain: CombinerDomain::Numeric,
    },
    CombinerRow {
        op: BinOp::Mul,
        spelling: "*",
        folds: true,
        scans: true,
        has_identity: true,
        domain: CombinerDomain::DimensionlessNumeric,
    },
    CombinerRow {
        op: BinOp::Min,
        spelling: "<<",
        folds: true,
        scans: true,
        has_identity: false,
        domain: CombinerDomain::Orderable,
    },
    CombinerRow {
        op: BinOp::Max,
        spelling: ">>",
        folds: true,
        scans: true,
        has_identity: false,
        domain: CombinerDomain::Orderable,
    },
    CombinerRow {
        op: BinOp::Or,
        spelling: "or",
        folds: true,
        scans: true,
        has_identity: true,
        domain: CombinerDomain::Boolean,
    },
    CombinerRow {
        op: BinOp::And,
        spelling: "and",
        folds: true,
        scans: true,
        has_identity: true,
        domain: CombinerDomain::Boolean,
    },
    CombinerRow {
        op: BinOp::KeepLeft,
        spelling: "<:",
        folds: false,
        scans: true,
        has_identity: false,
        domain: CombinerDomain::Any,
    },
    CombinerRow {
        op: BinOp::KeepRight,
        spelling: ":>",
        folds: false,
        scans: true,
        has_identity: false,
        domain: CombinerDomain::Any,
    },
];

impl CombinerRow {
    /// Whether this row admits `which`.  The call site asks the admission
    /// question directly rather than re-deriving it from an algebraic
    /// property, so the reason a row is refused lives in the table (with the
    /// documented column it mirrors) instead of at each use.
    fn admits(&self, which: Builtin) -> bool {
        match which {
            Builtin::Fold => self.folds,
            Builtin::Scan | Builtin::Prescan => self.scans,
            // `map` has no combiner slot, so the question does not arise.
            Builtin::Map => false,
        }
    }
}

fn combiner_row(op: BinOp) -> Option<&'static CombinerRow> {
    COMBINERS.iter().find(|row| row.op == op)
}

/// Resolve a backticked token against the table.  An unknown combiner names
/// the table, since the set is closed and the writer cannot extend it.
fn resolve_combiner(raw: &str, span: Span) -> Result<BinOp, Vec<TypeError>> {
    match COMBINERS.iter().find(|row| row.spelling == raw) {
        Some(row) => Ok(row.op),
        None => {
            let known = COMBINERS
                .iter()
                .map(|row| format!("`{}`", row.spelling))
                .collect::<Vec<_>>()
                .join(", ");
            Err(vec![TypeError::new(
                format!("`{raw}` is not a combiner; the table is {known}"),
                span,
            )])
        }
    }
}

/// A type-checking diagnostic, located by span. Mirrors `ResolveError`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
}

impl TypeError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        TypeError {
            message: message.into(),
            span,
        }
    }
}

/// The ambient value environment (`12-modules-and-imports.md`): names in
/// scope at every expression site before a lambda binds its parameters.
/// It holds the seven intrinsic base units ([`intrinsics`]), and the
/// resolver extends it with top-level const bindings and imported modules
/// (each module a [`Ty::Record`] of its members, so `si.km` types through
/// ordinary member access).
pub type Ambient = BTreeMap<String, Ty>;

/// The intrinsic initial environment (ADR 0026, Decision 6; ADR 0027,
/// Decision 4): the seven base units, each a total dimensioned value of
/// its base dimension.
pub fn intrinsics() -> Ambient {
    let mut env: Ambient = BASE_UNITS
        .iter()
        .map(|unit| {
            let dim = Dimension::of_base_unit(unit).expect("a base unit has a base dimension");
            (
                unit.to_string(),
                Ty::Value {
                    domain: ColumnType::Quantity(dim),
                    opt: Optionality::Total,
                },
            )
        })
        .collect();
    // The primitives (ADR 0031, Decision 11).  They are *values* in the
    // initial environment, not a parser special form, which is what lets a
    // bundled module bind their partial applications by name.  The derived
    // vocabulary (`sum`, `min`, ...) is deliberately absent: it lives in the
    // `bag` and `series` modules and is imported (Decision 8).
    for which in [Builtin::Fold, Builtin::Scan, Builtin::Prescan, Builtin::Map] {
        env.insert(
            which.name().to_string(),
            Ty::Builtin(Arc::new(PartialBuiltin {
                which,
                applied: Vec::new(),
            })),
        );
    }
    env
}

/// The typing context `Gamma` (section 5.1): the named values in scope and the
/// in-scope builtins. Which builtins are in scope is a property of the context,
/// not the grammar.  Lambda parameters are bound after the ambient names, so a
/// parameter shadows an ambient name (ordinary lexical scoping; the top-level
/// collision rule is the resolver's, not this layer's).
pub struct Context {
    names: BTreeMap<String, Ty>,
    /// The pristine ambient (intrinsics, top-level bindings, modules),
    /// before any lambda parameters.  A const function's body types
    /// against this, never against the caller's lambda scope: the
    /// closure's free names are its captures and the top level, by
    /// lexical scoping (ADR 0030).
    ambient: BTreeMap<String, Ty>,
    /// Function-application nesting depth; see [`MAX_FN_DEPTH`].
    fn_depth: u32,
    /// What the enclosing table knows about its fibers, for a scan's two
    /// demands: tie-freedom of the order key (ADR 0029 Decision 11's tier 1)
    /// and completeness under a fold-admitting combiner (ADR 0037
    /// decision 5).  `None` outside a group lambda, since only there is a
    /// fiber in scope.
    fiber_facts: Option<FiberFacts>,
    /// Whether the expression being typed came from a bundled module's source
    /// rather than the file being checked; see [`TyClosure::foreign`].  Set
    /// while typing a foreign closure's body, so a lambda that body creates
    /// inherits the provenance and a curried module binding stays anchored.
    foreign: bool,
}

/// The evidence a scan's demands can be discharged against.
///
/// A scan is deterministic only when its key is injective *on each fiber*
/// (`Mensura.IsArrangement.unique`), and under a fold-admitting combiner it is
/// honest only when the fiber is whole (ADR 0037 decision 5).  Both are
/// properties of the data, so the checker cannot decide them in general; what
/// it can do is look up the facts the table already carries.
///
/// The tracked tie fact is a **grading** (ADR 0024): a column set over which
/// the flat table is known functional.  A projection key `|r| r.c` is
/// injective on the fiber exactly when `key + {c}` contains a grading, because
/// then two rows of one fiber agreeing on `c` agreed on a full grading and are
/// the same row.  `Mensura.keyInjOn_demote_tag` is that argument, and it is
/// why the common window shape (a history keyed by entity and time, projected
/// to the entity) needs no ceremony: `demote` carries gradings unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct FiberFacts {
    /// The current key's column names.
    pub key: BTreeSet<String>,
    /// The column sets over which the flat table is known functional.
    pub gradings: BTreeSet<BTreeSet<String>>,
    /// Whether the pipeline claimed tie-freedom by fiat, for keys no grading
    /// covers (`assume { arranged }`).  The escape hatch of ADR 0029
    /// Decision 11's tier 3, which had no home until this rule needed one.
    pub assumed: bool,
    /// Whether every fiber of the enclosing table is known whole: `Complete`
    /// at the current key, or a `singletons` cardinality (a present key's
    /// single row is its whole fiber, `fiberCompleteWrt_of_functional`).
    /// Exactly the fact a reducing `map_bags` consumes (ADR 0023); the
    /// fold-admitting scans consume it too (ADR 0037 decision 5).
    pub complete: bool,
}

impl FiberFacts {
    /// Whether adding `column` to the key completes a grading, i.e. whether a
    /// projection key on it is injective within each fiber.
    fn grades(&self, column: &str) -> bool {
        let mut with = self.key.clone();
        with.insert(column.to_string());
        // Functionality is monotone upward: a grading inside `key + {c}` means
        // grouping by that superset is also at most one row.
        self.gradings.iter().any(|g| g.is_subset(&with))
    }
}

impl Context {
    /// Bind a row lambda's key-first parameters `|k, r|` (ADR 0015): `kname` to
    /// the key (key columns as total values), `rname` to the value row
    /// (non-key columns as single values carrying their totality).
    pub fn row(ambient: &Ambient, kname: &str, rname: &str, table: &TableType) -> Context {
        Context {
            names: bind2(
                ambient,
                kname,
                key_record(table),
                rname,
                value_record(table),
            ),
            ambient: ambient.clone(),
            fn_depth: 0,
            fiber_facts: None,
            foreign: false,
        }
    }

    /// Bind a group lambda's key-first parameters `|k, b|`: `kname` to the key
    /// (key columns as total values, constant within a bag), `bname` to the
    /// **fiber**, the bag of rows at that key (ADR 0031, Decision 1).
    ///
    /// `b` was the columnar record-of-bags of ADR 0015; that is now the
    /// derived presentation, reached by member access ([`Ty::project`]).  Bare
    /// `b` in a scalar position was a type error before (a record is not a
    /// value) and remains one (a bag of rows is not a value), so no existing
    /// program reads differently.
    pub fn bag(ambient: &Ambient, kname: &str, bname: &str, table: &TableType) -> Context {
        Context {
            names: bind2(ambient, kname, key_record(table), bname, fiber(table)),
            ambient: ambient.clone(),
            fn_depth: 0,
            fiber_facts: Some(FiberFacts {
                key: table.key_names(),
                gradings: table.qualifiers.functional.clone(),
                assumed: table.qualifiers.arranged == Arranged::Assumed,
                complete: table.qualifiers.cardinality == Cardinality::Singletons
                    || table.qualifiers.completeness == Completeness::Complete,
            }),
            foreign: false,
        }
    }

    /// Bind a `split` predicate's parameter `|k|` to a record of the table's
    /// key columns as total values (the key, `09` section 6.5).
    pub fn key(ambient: &Ambient, param: &str, table: &TableType) -> Context {
        Context {
            names: bind(ambient, param, key_record(table)),
            ambient: ambient.clone(),
            fn_depth: 0,
            fiber_facts: None,
            foreign: false,
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Ty> {
        self.names.get(name)
    }

    /// The context a const function's body types in: the pristine ambient,
    /// the closure's definition-site names, then the bound parameters,
    /// innermost last (a parameter shadows a capture, both shadow the top
    /// level).  The caller's lambda scope is deliberately absent
    /// (ADR 0030): a body cannot read the caller's `r`.
    fn for_closure_body(&self, closure: &TyClosure, bound: Vec<(String, Ty)>) -> Context {
        let mut names = self.ambient.clone();
        for (name, ty) in &closure.names {
            names.insert(name.clone(), ty.clone());
        }
        for (name, ty) in bound {
            names.insert(name, ty);
        }
        Context {
            names,
            ambient: self.ambient.clone(),
            fn_depth: self.fn_depth + 1,
            // Fiber facts describe the *enclosing table*, not the name scope,
            // so they survive into a closure body: `series.cumsum f k b`
            // applies the module's lambda, and the key `k` it forwards must
            // still be checked against the fiber it will arrange (and the
            // fiber's completeness against the combiner it will fold).
            fiber_facts: self.fiber_facts.clone(),
            // A module's body stays foreign however deeply it curries.
            foreign: self.foreign || closure.foreign,
        }
    }
}

fn bind(ambient: &Ambient, param: &str, ty: Ty) -> BTreeMap<String, Ty> {
    let mut names = ambient.clone();
    names.insert(param.to_string(), ty);
    names
}

/// Bind two key-first parameters over the ambient names, skipping `_` (the
/// ignored key).  Parameters land last, so they shadow ambient names.
fn bind2(ambient: &Ambient, a: &str, aty: Ty, b: &str, bty: Ty) -> BTreeMap<String, Ty> {
    let mut names = ambient.clone();
    if a != "_" {
        names.insert(a.to_string(), aty);
    }
    if b != "_" {
        names.insert(b.to_string(), bty);
    }
    names
}

/// Group flat dotted column names into nested record groups (ADR 0032): the
/// column `course.department.code` presents as `course` holding
/// `department` holding `code`, so member access follows the unit hierarchy
/// exactly as `01-units.md` writes it.  The resolver reserves a group's
/// prefix, so a scalar column can never collide with a group name.
fn nest_fields(fields: BTreeMap<String, Ty>) -> BTreeMap<String, Ty> {
    let mut out: BTreeMap<String, Ty> = BTreeMap::new();
    let mut groups: BTreeMap<String, BTreeMap<String, Ty>> = BTreeMap::new();
    for (name, ty) in fields {
        match name.split_once('.') {
            None => {
                out.insert(name, ty);
            }
            Some((head, rest)) => {
                groups
                    .entry(head.to_string())
                    .or_default()
                    .insert(rest.to_string(), ty);
            }
        }
    }
    for (head, sub) in groups {
        out.insert(head, Ty::Record(nest_fields(sub)));
    }
    out
}

/// The value row `r` of a table (ADR 0015): the non-key columns as single
/// values carrying their totality. The key columns live in the key `k`.
fn value_record(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in &table.content.columns {
        fields.insert(
            col.name.clone(),
            Ty::Value {
                domain: col.domain.clone(),
                opt: column_opt(table, &col.name),
            },
        );
    }
    Ty::Record(nest_fields(fields))
}

/// The **fiber** `b` of a table (ADR 0031, Decision 1): the bag of rows at one
/// key.  The field types are the *projections* (`b.x` is a bag of `x`), which
/// is the columnar presentation of ADR 0015 kept as sugar; the key columns
/// live in `k`.  A unit-reference group nests as a record of bags
/// (ADR 0032), so `b.course.name` is still a projection.
fn fiber(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in &table.content.columns {
        fields.insert(
            col.name.clone(),
            Ty::Bag {
                domain: col.domain.clone(),
                opt: column_opt(table, &col.name),
            },
        );
    }
    Ty::Rows(nest_fields(fields))
}

/// A key view of a table: the key columns as total values.
fn key_record(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in &table.content.key {
        fields.insert(
            col.name.clone(),
            Ty::Value {
                domain: col.domain.clone(),
                opt: Optionality::Total,
            },
        );
    }
    Ty::Record(nest_fields(fields))
}

fn column_opt(table: &TableType, name: &str) -> Optionality {
    if table.qualifiers.totality.is_total(name) {
        Optionality::Total
    } else {
        Optionality::Optional
    }
}

fn total(domain: ColumnType) -> Ty {
    Ty::Value {
        domain,
        opt: Optionality::Total,
    }
}

/// Type an expression, collecting all diagnostics (parallels
/// `resolve(&Program) -> Result<Vec<Schema>, Vec<ResolveError>>`).
pub fn type_expr(ctx: &Context, expr: &Expr) -> Result<Ty, Vec<TypeError>> {
    match &expr.kind {
        ExprKind::Int(_) => Ok(total(ColumnType::Int)),
        ExprKind::Float(_) => Ok(total(ColumnType::Real)),
        ExprKind::Str(_) => Ok(total(ColumnType::String)),
        ExprKind::Bool(_) => Ok(total(ColumnType::Bool)),
        ExprKind::Name(name) => type_name(ctx, name, expr.span),
        ExprKind::Member(base, field) => type_member(ctx, base, field),
        ExprKind::Binary(op, lhs, rhs) => type_binary(ctx, *op, lhs, rhs),
        ExprKind::Unary(op, operand) => type_unary(ctx, *op, operand),
        ExprKind::App(..) => apply_value(ctx, expr, None),
        ExprKind::Presence(base, _) => type_presence(ctx, base, expr.span),
        ExprKind::If { cond, then, els } => type_if(ctx, cond, then, els, expr.span),
        // A lambda types as a function value closing over the current
        // names (ADR 0030).  This is what lets a *curried* const function
        // type: applying `|a| |b| a + b` types its body, which is this
        // lambda.  A lambda cannot reach a column (`column_of`) and only a
        // `Name` head applies, so a view body still cannot make use of one
        // it creates.
        ExprKind::Lambda { params, ret, body } => {
            if ret.is_some() {
                return Err(vec![TypeError::new(
                    "a return-type ascription on a lambda is not supported \
                     (the type grammar has no function type)",
                    expr.span,
                )]);
            }
            Ok(Ty::Fn(Arc::new(TyClosure {
                params: params.clone(),
                body: (**body).clone(),
                names: ctx.names.clone(),
                foreign: ctx.foreign,
            })))
        }
        // A combiner is not a value: it names an operator whose *algebra* is
        // compiler knowledge, and it is meaningful only in a reduction's
        // combiner slot, where `apply_builtin` consumes it (ADR 0031,
        // Decision 6).  Reaching here means it was written somewhere else.
        ExprKind::Combiner(raw) => {
            // Still resolve it, so a typo is reported as a typo rather than
            // hidden behind the position complaint.
            resolve_combiner(raw, expr.span)?;
            Err(vec![TypeError::new(
                format!(
                    "`` `{raw}` `` is a combiner, not a value; it belongs in a \
                     reduction's combiner slot, as in ``fold `{raw}` (|v| v) b``"
                ),
                expr.span,
            )])
        }
        _ => Err(vec![TypeError::new(
            "unsupported in this increment",
            expr.span,
        )]),
    }
}

fn type_name(ctx: &Context, name: &str, span: Span) -> Result<Ty, Vec<TypeError>> {
    match ctx.lookup(name) {
        Some(ty) => Ok(ty.clone()),
        None => Err(vec![TypeError::new(format!("unknown name `{name}`"), span)]),
    }
}

fn type_member(ctx: &Context, base: &Expr, field: &Ident) -> Result<Ty, Vec<TypeError>> {
    // A record projects its field; the fiber projects too, by the sugar
    // equation `b.x == map (|r| r.x) b` (ADR 0031, Decision 2).  Both spell
    // the same failure the same way, so `b.nope` and `r.nope` read alike.
    let fields = match type_expr(ctx, base)? {
        Ty::Record(fields) | Ty::Rows(fields) => fields,
        _ => {
            return Err(vec![TypeError::new(
                "member access on a non-record value",
                field.span,
            )]);
        }
    };
    match Ty::project(&fields, &field.name) {
        Some(ty) => Ok(ty),
        None => Err(vec![TypeError::new(
            format!("unknown column `{}`", field.name),
            field.span,
        )]),
    }
}

/// Decompose a curried application `f a b` into the head `f` and the argument
/// list `[a, b]`. A non-application returns `(expr, [])`. (Local to the value
/// layer; the pipe layer has its own copy, since the two type domains stay
/// decoupled, `docs/toolkit/01-application-checking.md`.)
fn flatten_app(expr: &Expr) -> (&Expr, Vec<&Expr>) {
    let mut args = Vec::new();
    let mut cur = expr;
    while let ExprKind::App(func, arg) = &cur.kind {
        args.push(arg.as_ref());
        cur = func;
    }
    args.reverse();
    (cur, args)
}

/// Apply a value operation, whether the argument arrived from the left of a
/// `|>` (`piped` is `Some`) or as a trailing argument in a bare application
/// `op arg` (`piped` is `None`). Both spellings converge here, so `x |> op` and
/// `op x` are checked identically (ADR 0018,
/// `docs/toolkit/01-application-checking.md`).
///
/// The head may be a const function, a reduction primitive (`fold`, `map`), or
/// `to_real`, and may be spelled bare or qualified by a module (`bag.max`,
/// ADR 0031 Decision 8).  Only `to_real` is still 1-ary: the primitives are
/// curried, so a partial application is an ordinary value.
fn apply_value(ctx: &Context, op_expr: &Expr, piped: Option<&Expr>) -> Result<Ty, Vec<TypeError>> {
    let (head, mut args) = flatten_app(op_expr);
    if let Some(input) = piped {
        args.push(input);
    }
    // Resolve the head to a function value, whichever way it is spelled: a
    // bare name (ADR 0030) or a module member (`bag.max`, ADR 0031
    // Decision 8).  A module is an ambient `Ty::Record`, so a qualified name
    // is ordinary member access; what was missing was routing the *result*
    // into the application path, which is why a qualified call used to report
    // "unsupported in this increment".
    let head_ty = match &head.kind {
        ExprKind::Name(name) => ctx.lookup(name).cloned(),
        // A malformed qualified head propagates its own diagnostic (unknown
        // module, unknown member) rather than the generic complaint below.
        ExprKind::Member(..) => Some(type_member_head(ctx, head)?),
        _ => {
            return Err(vec![TypeError::new(
                "unsupported in this increment",
                head.span,
            )]);
        }
    };
    // The resolver forbids a binding that collides with a builtin, so a
    // function head can never shadow `to_real` or an aggregate.
    match head_ty {
        Some(Ty::Fn(closure)) => {
            return apply_closure(ctx, &head_name(head), closure, &args, head.span);
        }
        // A builtin primitive (ADR 0031, Decision 11): same currying, but the
        // slots have their own rules rather than a lambda body to re-type.
        Some(Ty::Builtin(partial)) => {
            return apply_builtin(ctx, partial, &args, head.span);
        }
        _ => {}
    }
    let ExprKind::Name(name) = &head.kind else {
        return Err(vec![TypeError::new(
            "unsupported in this increment",
            head.span,
        )]);
    };
    let [arg] = args[..] else {
        return Err(vec![TypeError::new(
            "unsupported in this increment",
            head.span,
        )]);
    };
    if name == "to_real" {
        return type_to_real(ctx, arg);
    }
    // `desc` is 1-ary and has no slots, so it shares `to_real`'s shape rather
    // than becoming a `Builtin` (whose contract is a curried primitive with a
    // per-slot rule).
    if name == "desc" {
        return type_desc(ctx, arg);
    }
    let _ = arg;
    Err(vec![TypeError::new(
        retired_aggregate_hint(name).unwrap_or_else(|| "unsupported in this increment".to_string()),
        head.span,
    )])
}

/// The six aggregates left the initial environment (ADR 0031, Decision 8):
/// with `fold` a builtin there is no reason to keep so many *names* in the
/// language, and ADR 0027 Decision 4's "nothing else is in scope that you did
/// not import" now holds without the exception it used to carry.  The names
/// are ordinary again, so `unknown name` is the honest diagnostic; this hint
/// exists because an unqualified `sum b.x` is the single most likely thing an
/// existing program says.
fn retired_aggregate_hint(name: &str) -> Option<String> {
    let replacement = match name {
        "sum" | "min" | "max" | "any" | "all" => format!("`bag.{name}` after `import bag`"),
        "count" => "`#` (as in `#b` or `#b.column`)".to_string(),
        "mean" => "`bag.sum b.x / to_real (#b.x)`".to_string(),
        _ => return None,
    };
    Some(format!(
        "`{name}` is no longer a builtin: it is {replacement} (ADR 0031)"
    ))
}

/// Type an application head that is a member access (`bag.max`).  Separate
/// from [`type_member`] only so a failure here is not swallowed by the
/// generic "unsupported" arm of [`apply_value`].
fn type_member_head(ctx: &Context, head: &Expr) -> Result<Ty, Vec<TypeError>> {
    let ExprKind::Member(base, field) = &head.kind else {
        unreachable!("called only on a member head");
    };
    type_member(ctx, base, field)
}

/// How a head prints in a call-site note: `f` or `bag.max`.
fn head_name(head: &Expr) -> String {
    match &head.kind {
        ExprKind::Name(n) => n.clone(),
        ExprKind::Member(base, field) => match &base.kind {
            ExprKind::Name(m) => format!("{m}.{}", field.name),
            _ => field.name.clone(),
        },
        _ => "the function".to_string(),
    }
}

/// The maximum function-application nesting depth while typing.  Mutual
/// recursion between const functions escapes the const evaluator's own
/// guard (each body types fine in isolation) and would otherwise recurse
/// the checker; see the evaluator's `MAX_APPLY_DEPTH` for the rationale.
const MAX_FN_DEPTH: u32 = 64;

/// Apply a const function (ADR 0030, Decision 3): every application is
/// saturated or an error.  Arguments type in the caller's context; the
/// body re-types per call site in the function's own environment
/// ([`Context::for_closure_body`]), which is what makes the checking exact
/// with no inference.  Body diagnostics carry definition-site spans, so a
/// call-site note is appended to them (ADR 0030, Consequences).
///
/// **Unless the definition site is another file.**  A bundled module's body
/// spans do not address the file being checked ([`TyClosure::foreign`]), so
/// those diagnostics are *re-anchored* at the call site rather than merely
/// annotated.  Without this, `series.running_max`'s tie demand reports at an
/// offset into `stdlib/series.mensura`: a nonsense line in the user's file, and
/// out of bounds whenever the user's file is the shorter of the two, which the
/// language server cannot render at all.
fn apply_closure(
    ctx: &Context,
    name: &str,
    closure: Arc<TyClosure>,
    args: &[&Expr],
    head_span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let mut ty = Ty::Fn(closure);
    for (i, arg) in args.iter().enumerate() {
        // A closure whose body is a partially applied primitive returns a
        // builtin value, not another closure, so the remaining arguments belong
        // to the primitive's slot rules.  `series.lead value key b` is the case
        // that needs this: the outer lambda yields a 3-slot `prescan`.
        if let Ty::Builtin(partial) = &ty {
            return apply_builtin(ctx, partial.clone(), &args[i..], head_span);
        }
        let Ty::Fn(c) = ty else {
            return Err(vec![TypeError::new(
                format!(
                    "cannot apply a value of type `{}`: it is not a function",
                    ty_name(&ty)
                ),
                arg.span,
            )]);
        };
        // Bind the parameters to the arguments' types: a one-parameter
        // function binds any value; a tupled function requires its tuple.
        let mut bound = Vec::new();
        match c.params.len() {
            1 => bound.push((c.params[0].name.clone(), type_expr(ctx, arg)?)),
            n => match &arg.kind {
                ExprKind::Tuple(items) if items.len() == n => {
                    for (param, item) in c.params.iter().zip(items) {
                        bound.push((param.name.clone(), type_expr(ctx, item)?));
                    }
                }
                _ => {
                    return Err(vec![TypeError::new(
                        format!(
                            "`{name}` expects a tuple of {n} values (a \
                             multi-parameter lambda is tupled; currying is \
                             written `|a| |b| ...`)"
                        ),
                        arg.span,
                    )]);
                }
            },
        }
        if ctx.fn_depth >= MAX_FN_DEPTH {
            return Err(vec![TypeError::new(
                "const function applications nest too deeply: a const \
                 function may be recursive",
                head_span,
            )]);
        }
        let body_ctx = ctx.for_closure_body(&c, bound);
        let foreign = c.foreign;
        ty = type_expr(&body_ctx, &c.body).map_err(|mut errs| {
            if foreign {
                for err in &mut errs {
                    err.span = head_span;
                }
            }
            errs.push(TypeError::new(
                format!("while applying `{name}` here"),
                head_span,
            ));
            errs
        })?;
    }
    Ok(ty)
}

/// Apply arguments to a builtin (ADR 0031, Decision 11).  Each step consults
/// the primitive's own slot rule; short of saturation the result is another
/// builtin value, which is what makes `let sum { fold `+` (|v| v) }` a
/// binding rather than a special form.
fn apply_builtin(
    ctx: &Context,
    partial: Arc<PartialBuiltin>,
    args: &[&Expr],
    head_span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let which = partial.which;
    let mut applied = partial.applied.clone();
    for arg in args {
        if applied.len() >= which.arity() {
            return Err(vec![TypeError::new(
                format!(
                    "`{}` takes {} arguments and is already saturated",
                    which.name(),
                    which.arity()
                ),
                arg.span,
            )]);
        }
        // The combiner slot takes a backticked operator, never a value: a
        // combiner's algebra is compiler knowledge, so it cannot be computed.
        if which.combiner_slot() == Some(applied.len()) {
            let ExprKind::Combiner(raw) = &arg.kind else {
                return Err(vec![TypeError::new(
                    format!(
                        "`{}`'s first argument is a backticked combiner, \
                         like `` `+` ``",
                        which.name()
                    ),
                    arg.span,
                )]);
            };
            let op = resolve_combiner(raw, arg.span)?;
            let row = combiner_row(op).expect("resolved from the table");
            // Admission is the table's own column (Decision 6).  Only `fold`
            // refuses a row today: it reduces an *unordered* bag, so it needs
            // commutativity, while a scan's key supplies the order a bag lacks.
            if !row.admits(which) {
                return Err(vec![TypeError::new(
                    format!(
                        "`` `{}` `` is not commutative, so it is admitted under \
                         `scan` only; a fold over an unordered bag would depend \
                         on the order",
                        row.spelling
                    ),
                    arg.span,
                )]);
            }
            applied.push(BuiltinArg::Combiner(op));
            continue;
        }
        applied.push(BuiltinArg::Ty(type_expr(ctx, arg)?));
    }
    if applied.len() < which.arity() {
        // Still partial: an ordinary value (ADR 0030's currying).
        return Ok(Ty::Builtin(Arc::new(PartialBuiltin { which, applied })));
    }
    saturated_builtin(ctx, which, &applied, head_span)
}

/// Type a saturated builtin application.
fn saturated_builtin(
    ctx: &Context,
    which: Builtin,
    applied: &[BuiltinArg],
    head_span: Span,
) -> Result<Ty, Vec<TypeError>> {
    match which {
        Builtin::Fold => {
            let [
                BuiltinArg::Combiner(op),
                BuiltinArg::Ty(mapper),
                BuiltinArg::Ty(bag),
            ] = applied
            else {
                return Err(vec![TypeError::new(
                    "`fold` expects a combiner, a mapper, and a bag",
                    head_span,
                )]);
            };
            type_fold(ctx, *op, mapper, bag, head_span)
        }
        Builtin::Scan | Builtin::Prescan => {
            let [
                BuiltinArg::Combiner(op),
                BuiltinArg::Ty(mapper),
                BuiltinArg::Ty(key),
                BuiltinArg::Ty(bag),
            ] = applied
            else {
                return Err(vec![TypeError::new(
                    format!(
                        "`{}` expects a combiner, a mapper, an order key, and \
                         a bag",
                        which.name()
                    ),
                    head_span,
                )]);
            };
            type_scan(ctx, which, *op, mapper, key, bag, head_span)
        }
        Builtin::Map => {
            let [BuiltinArg::Ty(mapper), BuiltinArg::Ty(bag)] = applied else {
                return Err(vec![TypeError::new(
                    "`map` expects a mapper and a bag",
                    head_span,
                )]);
            };
            type_map(ctx, mapper, bag, head_span)
        }
    }
}

/// The element type a bag or fiber hands to a mapper.  Over a projected bag
/// the element is a *value*; over the fiber itself it is a *row*, which is how
/// a row-mapper fold like `fold `+` (|r| r.mass / r.height ^ 2) b` becomes
/// expressible (ADR 0031, Decision 4).
fn element_of(ty: &Ty, what: &str, span: Span) -> Result<Ty, Vec<TypeError>> {
    match ty {
        Ty::Bag { domain, opt } => Ok(Ty::Value {
            domain: domain.clone(),
            opt: *opt,
        }),
        // A row of the fiber: each field as a single value, which is exactly
        // the `flat_map` row view.  The fiber's fields are stored as their
        // projections, so unwrap one level.
        Ty::Rows(fields) => Ok(Ty::Record(
            fields
                .iter()
                .map(|(name, ty)| {
                    let field = match ty {
                        Ty::Bag { domain, opt } => Ty::Value {
                            domain: domain.clone(),
                            opt: *opt,
                        },
                        other => other.clone(),
                    };
                    (name.clone(), field)
                })
                .collect(),
        )),
        other => Err(vec![TypeError::new(
            format!("{what} expects a bag, found {}", describe_ty(other)),
            span,
        )]),
    }
}

/// Apply a mapper (a one-parameter function value) to one element type.
fn apply_mapper(
    ctx: &Context,
    mapper: &Ty,
    element: Ty,
    what: &str,
    span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let Ty::Fn(c) = mapper else {
        return Err(vec![TypeError::new(
            format!(
                "{what}'s mapper must be a function, found {}",
                describe_ty(mapper)
            ),
            span,
        )]);
    };
    if c.params.len() != 1 {
        return Err(vec![TypeError::new(
            format!(
                "{what}'s mapper takes one element, but this function takes {}",
                c.params.len()
            ),
            span,
        )]);
    }
    if ctx.fn_depth >= MAX_FN_DEPTH {
        return Err(vec![TypeError::new(
            "function applications nest too deeply: a const function may be \
             recursive",
            span,
        )]);
    }
    let bound = vec![(c.params[0].name.clone(), element)];
    let body_ctx = ctx.for_closure_body(c, bound);
    type_expr(&body_ctx, &c.body).map_err(|mut errs| {
        errs.push(TypeError::new(format!("while applying {what} here"), span));
        errs
    })
}

/// `fold : combiner -> (element -> value) -> bag -> value` (ADR 0031,
/// Decision 4).  Backed by ADR 0029's Stage 1, proved in
/// `formal/Mensura/Fold.lean`: `foldBag` over a commutative monoid, the shard
/// lemma, and the `Option` completion with its presence lemma.
fn type_fold(
    ctx: &Context,
    op: BinOp,
    mapper: &Ty,
    bag: &Ty,
    span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let element = element_of(bag, "`fold`", span)?;
    let mapped = apply_mapper(ctx, mapper, element, "`fold`", span)?;
    let row = combiner_row(op).expect("resolved from the table");
    let domain = accumulator_domain(&mapped, "`fold`", span)?;
    require_combiner_domain(row, &domain, span)?;
    // The result is total either way, but for two different reasons, and the
    // distinction is the whole content of ADR 0029 Decision 4:
    //
    // - A combiner *with* an identity has a true answer for the empty bag
    //   (`0` is the sum of nothing), so the fold is total unconditionally.
    // - A combiner *without* one has none ("there is no smallest element of
    //   nothing"), so it folds through the `Option` completion and is total
    //   only on a non-empty bag.  A group arises from rows and is never
    //   empty, which is exactly the hypothesis
    //   `Mensura.foldBagOpt_isSome_of_ne_zero` discharges.
    //
    // Both land on `total` here because a bag in this position is always a
    // group.  When a possibly-empty bag becomes expressible, the identity-free
    // rows must yield an optional value, and this is the branch that changes.
    //
    // The assertion below is deliberately *fold-local*: it holds because a
    // fold only ever sees the six commutative rows, among which exactly the
    // orderable pair lacks an identity.  It is not true of the table as a
    // whole, since the tacks are identity-free too and `scan` reaches them
    // (`type_scan`).  Do not lift it.
    debug_assert!(
        row.has_identity || matches!(row.domain, CombinerDomain::Orderable),
        "only the orderable rows (`<<`, `>>`) lack an identity among the \
         fold-admitted six"
    );
    Ok(total(domain))
}

/// The accumulator's domain: the mapper's result is what the combiner folds, so
/// a fold's (and a scan's) accumulator type is that result's domain.  The type
/// must be invariant under the combiner, which is what the per-row domain
/// restriction of [`require_combiner_domain`] enforces.
///
/// A mapper that may produce a missing value is rejected here: a reduction over
/// a bag with holes has no defined answer, and the fix is upstream (filter, or
/// coalesce the mapper).
fn accumulator_domain(mapped: &Ty, what: &str, span: Span) -> Result<ColumnType, Vec<TypeError>> {
    let (domain, opt) = match mapped {
        Ty::Value { domain, opt } => (domain.clone(), *opt),
        other => {
            return Err(vec![TypeError::new(
                format!(
                    "{what}'s mapper must produce a value, found {}",
                    describe_ty(other)
                ),
                span,
            )]);
        }
    };
    if opt == Optionality::Optional {
        return Err(vec![TypeError::new(
            format!(
                "{what} requires a total bag; this mapper may produce a missing value, \
                 so discharge it with `??` upstream (ADR 0039)"
            ),
            span,
        )]);
    }
    Ok(domain)
}

/// Check the combiner row's domain restriction against the accumulator's
/// domain.  Shared by `fold` and `scan` so the two primitives cannot drift on
/// it: the restrictions are properties of the *combiner*, not of the direction
/// the bag is reduced in.
fn require_combiner_domain(
    row: &'static CombinerRow,
    domain: &ColumnType,
    span: Span,
) -> Result<(), Vec<TypeError>> {
    let domain = domain.clone();
    let ok = match row.domain {
        CombinerDomain::Numeric => domain.is_numeric(),
        CombinerDomain::DimensionlessNumeric => {
            domain == ColumnType::Int || domain == ColumnType::Real
        }
        CombinerDomain::Orderable => domain.is_orderable(),
        CombinerDomain::Boolean => domain == ColumnType::Bool,
        CombinerDomain::Any => true,
    };
    if !ok {
        let expected = match row.domain {
            CombinerDomain::Numeric => "a numeric domain",
            // Name the reason: this is the one row whose restriction is not
            // obvious from the operator (ADR 0031, Decision 6).
            CombinerDomain::DimensionlessNumeric => {
                "a dimensionless numeric domain (`int` or bare `real`), since \
                 a dimensioned product's dimension would depend on the bag's \
                 size"
            }
            CombinerDomain::Orderable => "an orderable domain",
            CombinerDomain::Boolean => "`bool`",
            CombinerDomain::Any => unreachable!("`Any` always admits"),
        };
        return Err(vec![TypeError::new(
            format!(
                "`` `{}` `` folds {expected}, found {}",
                row.spelling,
                domain_name(&domain)
            ),
            span,
        )]);
    }
    Ok(())
}

/// `scan`/`prescan : combiner -> (row -> value) -> (row -> key) -> rows ->
/// bag` (ADR 0031, Decision 7).  Backed by ADR 0029's Stage 2, proved in
/// `formal/Mensura/Arranged.lean`: `IsArrangement` with its existence and
/// Tier 1 uniqueness theorems, `scanBag`/`prescanBag` as two slices of one
/// `List.scanl`, the coherence theorem that a scan's last element is the
/// corresponding fold, and `scanFiber_splitSafe`.
///
/// The result is a **bag**, not a value: a scan emits one row per input row, so
/// it is the window shape of `map_bags` rather than the reducing one.
///
/// # Completeness
///
/// The completeness demand is **per combiner row**, not per output shape
/// (ADR 0037 decision 5).  Under a fold-admitting combiner a scan *contains*
/// its reduction: the last entry of the inclusive scan is the fold
/// (`Mensura.scanl_getLast_eq_foldBag`), and every output row folds a prefix,
/// so a missing early row corrupts every later output.  It therefore demands
/// the same fiber-completeness fact a reducing `map_bags` demands (ADR 0023);
/// exempting it would make `` scan `+` `` a loophole recomputing `` fold `+` ``
/// without the obligation.  The keep combiners' outputs are claims about
/// adjacency among *present* rows ("the previous reading in this bag"), which
/// a partial bag represents honestly, so they demand nothing.  The demand axis
/// is "does any output row's claim quantify over absent rows", and the closed
/// table's `folds` column is that line.
///
/// # Optionality
///
/// | | identity (`+ * or and`) | identity-free (`<< >> <: :>`) |
/// | --- | --- | --- |
/// | `scan` | total | total |
/// | `prescan` | total | **optional** |
///
/// An inclusive scan is total at every row because position `i` folds elements
/// `1..i`, which is never empty, so the identity is never consulted.  An
/// exclusive scan's first position folds the *empty* prefix, so an
/// identity-free combiner has no answer there and the result carries a missing
/// value.  That is where `lag`'s missing first row comes from: it falls out of
/// the row's `has_identity` being false, not from any rule about `lag`.
///
/// `type_fold`'s comment on its own totality branch anticipated this change and
/// named a possibly-empty bag as the trigger.  `prescan` is a *second* trigger
/// it did not name: the bag is non-empty, but a proper prefix of it can be.
fn type_scan(
    ctx: &Context,
    which: Builtin,
    op: BinOp,
    mapper: &Ty,
    key: &Ty,
    bag: &Ty,
    span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let what = format!("`{}`", which.name());
    let element = element_of(bag, &what, span)?;
    let mapped = apply_mapper(ctx, mapper, element.clone(), &what, span)?;
    let row = combiner_row(op).expect("resolved from the table");
    let domain = accumulator_domain(&mapped, &what, span)?;
    require_combiner_domain(row, &domain, span)?;
    // The key orders the fiber, so its obligation is decidable (an orderable
    // domain) and it contributes nothing to the result type.
    let key_ty = apply_mapper(ctx, key, element, &format!("{what}'s order key"), span)?;
    require_orderable_key(&key_ty, &what, span)?;
    require_tie_free_key(ctx, key, &what, span)?;
    require_complete_fiber(ctx, row, &what, span)?;
    let opt = if which == Builtin::Prescan && !row.has_identity {
        Optionality::Optional
    } else {
        Optionality::Total
    };
    Ok(Ty::Bag { domain, opt })
}

/// The order key's obligation: a single value in an orderable domain, present
/// at every row.
///
/// Totality is required for a reason the ADR does not name: a missing key has
/// no position in the order, so there is no arrangement to scan over.  The fix
/// is upstream, which the diagnostic says.
///
/// A `desc`-marked key is accepted and its direction discarded here: direction
/// changes the arrangement, not the obligation, since the dual of a total order
/// is total (`Mensura.arrangeList` at `OrderDual`).
fn require_orderable_key(key_ty: &Ty, what: &str, span: Span) -> Result<(), Vec<TypeError>> {
    let inner = match key_ty {
        Ty::Desc(inner) => inner.as_ref(),
        other => other,
    };
    match inner {
        Ty::Value { domain, opt } => {
            if *opt == Optionality::Optional {
                return Err(vec![TypeError::new(
                    format!(
                        "{what}'s order key may produce a missing value, which \
                         has no position in the order; filter the bag or \
                         discharge the key with `??` first (ADR 0039)"
                    ),
                    span,
                )]);
            }
            if !domain.is_orderable() {
                return Err(vec![TypeError::new(
                    format!(
                        "{what}'s order key must land in an orderable domain \
                         (`int`, `real`, a dimensioned real, or `date`), found \
                         {}",
                        domain_name(domain)
                    ),
                    span,
                )]);
            }
            Ok(())
        }
        // A tuple key would order lexicographically (ADR 0031, Decision 7's
        // tier 2), but a value tuple has no type yet, so say that rather than
        // reporting a generic "unsupported" from the tuple itself.
        Ty::Record(_) => Err(vec![TypeError::new(
            format!(
                "{what}'s order key must be a single orderable value; \
                 lexicographic tuple keys are not yet supported (ADR 0031, \
                 Decision 7)"
            ),
            span,
        )]),
        other => Err(vec![TypeError::new(
            format!(
                "{what}'s order key must produce an orderable value, found {}",
                describe_ty(other)
            ),
            span,
        )]),
    }
}

/// **The Tier 1 obligation**: a scan's order key must be tie-free on each fiber,
/// because otherwise the arrangement is not unique and the intermediate values
/// are not determined (`Mensura.IsArrangement.unique`).
///
/// This is the ordered counterpart of the reducing shape's completeness demand
/// (ADR 0023), and it is the same *kind* of obligation: a property of the data,
/// undecidable in general, decidable in a tracked special case, established
/// upstream or admitted by fiat.  ADR 0029 Decision 11 already says ties are
/// "structurally the same problem as completeness"; this is that, enforced.
///
/// Two ways to discharge it:
///
/// 1. **A grading.**  A projection key `|r| r.c` is injective on the fiber when
///    `key + {c}` contains a grading (ADR 0024), because two rows of one fiber
///    agreeing on `c` then agreed on a whole grading and are the same row.  This
///    is `Mensura.keyInjOn_demote_tag`, and it makes the common window shape
///    ceremony-free: a history keyed by `(entity, time)` and projected to
///    `entity` carries the grading through `demote` unchanged.
/// 2. **`assume { arranged }`**, for a computed key or an ungraded column: the
///    claim ADR 0029 Decision 11 calls tier 3, admitted locally and visibly the
///    way `assume { complete }` admits completeness (ADR 0017, whose block form
///    was written to generalize this way).
///
/// A `desc` marker is transparent: the dual of an injective key is injective.
fn require_tie_free_key(
    ctx: &Context,
    key: &Ty,
    what: &str,
    span: Span,
) -> Result<(), Vec<TypeError>> {
    let Some(facts) = &ctx.fiber_facts else {
        // No fiber in scope, so nothing to arrange.  Unreachable through
        // `type_scan` (its bag argument came from a group lambda), but the
        // conservative branch is to demand nothing rather than to invent a fact.
        return Ok(());
    };
    if facts.assumed {
        return Ok(());
    }
    match key_projection(key) {
        Some(column) if facts.grades(&column) => Ok(()),
        Some(column) => Err(vec![TypeError::new(
            format!(
                "{what}'s order key `{column}` may have ties, so the \
                 arrangement is not determined: nothing says at most one row \
                 per key shares it.  Order by a column projected out of the \
                 key (`demote` carries that fact), or claim it with \
                 `assume {{ arranged }}`"
            ),
            span,
        )]),
        None => Err(vec![TypeError::new(
            format!(
                "{what}'s order key is computed, so the checker cannot tell \
                 whether it has ties; claim it with `assume {{ arranged }}` or \
                 order by a column whose uniqueness is tracked"
            ),
            span,
        )]),
    }
}

/// **The per-combiner completeness demand** (ADR 0037 decision 5, settling
/// ADR 0029's flag): under a fold-admitting combiner a scan contains its
/// reduction, so it demands the fiber-completeness fact a reducing `map_bags`
/// demands (ADR 0023), discharged the same two ways (`Complete` at the current
/// key, or a `singletons` cardinality whose present fiber is trivially whole).
/// The keep combiners demand nothing: their outputs are claims about adjacency
/// among *present* rows, which a partial bag represents honestly.  The line is
/// the closed table's own admission column, so this is a lookup, not an
/// analysis.
fn require_complete_fiber(
    ctx: &Context,
    row: &CombinerRow,
    what: &str,
    span: Span,
) -> Result<(), Vec<TypeError>> {
    if !row.folds {
        return Ok(());
    }
    let Some(facts) = &ctx.fiber_facts else {
        // No fiber in scope, so no bag to be partial: the same conservative
        // branch as the tie demand above.
        return Ok(());
    };
    if facts.complete {
        return Ok(());
    }
    Err(vec![TypeError::new(
        format!(
            "{what} at `` `{}` `` contains its reduction (every output row \
             folds a prefix of the bag, and its last row is the fold itself), \
             so it needs completeness over the current key like any reducer \
             (ADR 0037 decision 5); establish it with \
             `completeness_check {{ ... }}` or `assume {{ complete }}` first, \
             after any `demote` (the fact does not survive a key coarsening)",
            row.spelling
        ),
        span,
    )])
}

/// The column a key lambda projects, if it is a bare projection (`|r| r.c`,
/// possibly under `desc`).  `None` for a computed key, whose injectivity is not
/// a question the checker can answer.
fn key_projection(key: &Ty) -> Option<String> {
    let Ty::Fn(c) = key else {
        return None;
    };
    projected_column(&c.body, &c.names)
}

/// The column an order-key body projects, looking through `desc` and through a
/// lambda applied to the row.
///
/// The second case is not an edge case: `series.lead` is defined as
/// `|value| |key| prescan `:>` value (|r| desc (key r))`, so once the caller's
/// key is substituted the body reads `desc ((|r| r.taken_at) r)`.  Refusing to
/// look through that application would make `lead` permanently unprovable while
/// `lag` at the same column is fine, which would be an artefact of how the
/// library happens to be written rather than a fact about the data.
fn projected_column(body: &Expr, env: &BTreeMap<String, Ty>) -> Option<String> {
    match &body.kind {
        // `desc` is transparent: the dual of an injective key is injective.
        ExprKind::App(head, arg) if matches!(&head.kind, ExprKind::Name(n) if n == "desc") => {
            projected_column(arg, env)
        }
        ExprKind::App(head, _) => match &head.kind {
            // A lambda applied to the row: its parameter is the row either way,
            // so the projection is the same column.
            ExprKind::Lambda { params, body, .. } if params.len() == 1 => {
                projected_column(body, env)
            }
            // A *captured* function applied to the row.  This is the `lead`
            // case: the caller's key arrives in the closure's environment rather
            // than inlined in the body, so follow it there.
            ExprKind::Name(n) => match env.get(n) {
                Some(Ty::Fn(inner)) => projected_column(&inner.body, &inner.names),
                _ => None,
            },
            _ => None,
        },
        ExprKind::Member(base, field) if matches!(&base.kind, ExprKind::Name(_)) => {
            Some(field.name.clone())
        }
        _ => None,
    }
}

/// `map : (element -> value) -> bag -> bag` (ADR 0031, Decision 3): the
/// projection functor and the explicit form of `b.x`.  It is not a reduction,
/// so it needs no combiner and no proof gate beyond `Multiset.map`.
fn type_map(ctx: &Context, mapper: &Ty, bag: &Ty, span: Span) -> Result<Ty, Vec<TypeError>> {
    let element = element_of(bag, "`map`", span)?;
    match apply_mapper(ctx, mapper, element, "`map`", span)? {
        Ty::Value { domain, opt } => Ok(Ty::Bag { domain, opt }),
        other => Err(vec![TypeError::new(
            format!(
                "`map`'s mapper must produce a value, found {}",
                describe_ty(&other)
            ),
            span,
        )]),
    }
}

/// A short name for a `Ty` in diagnostics.
fn ty_name(ty: &Ty) -> String {
    match ty {
        Ty::Value { domain, .. } => domain_name(domain),
        Ty::Bag { domain, .. } => format!("bag of {}", domain_name(domain)),
        Ty::Rows(_) => "bag of rows".to_string(),
        Ty::Record(_) => "record".to_string(),
        Ty::Desc(inner) => format!("descending {}", ty_name(inner)),
        Ty::Fn(_) | Ty::Builtin(_) => "function".to_string(),
    }
}

/// `desc` (ADR 0031, Decision 7): wrap an orderable value in its order dual.
/// Orderable in, orderable out, never storable and never ascribable.
///
/// A direction *marker* rather than a comparator, forced by the same epistemics
/// as the combiner table: a comparator's obligation is a law (a strict total
/// order), unverifiable on a user lambda, so keys stay values in orderable
/// domains and direction stays a marker.  In `formal/` the marker is Mathlib's
/// `OrderDual`, so `Mensura.arrangeList` absorbs it with no new structure and
/// Tier 1 survives it (the dual of a total order is total, and the dual of an
/// injective key is injective).
fn type_desc(ctx: &Context, arg: &Expr) -> Result<Ty, Vec<TypeError>> {
    let span = arg.span;
    match type_expr(ctx, arg)? {
        Ty::Value { domain, opt } if domain.is_orderable() => {
            Ok(Ty::Desc(Box::new(Ty::Value { domain, opt })))
        }
        Ty::Value { domain, .. } => Err(vec![TypeError::new(
            format!(
                "`desc` orders an orderable domain (`int`, `real`, a \
                 dimensioned real, or `date`), found {}",
                domain_name(&domain)
            ),
            span,
        )]),
        // The dual of the dual is the original order, so a double marker is
        // always a mistake rather than a no-op worth accepting silently.
        Ty::Desc(_) => Err(vec![TypeError::new(
            "`desc` is already applied here; the dual of the dual is the \
             original order",
            span,
        )]),
        other => Err(vec![TypeError::new(
            format!(
                "`desc` marks a single orderable value as descending, found {}",
                describe_ty(&other)
            ),
            span,
        )]),
    }
}

/// `to_real` (ADR 0014): `int -> real` on a value, lifted element-wise over a
/// bag (`bag<int> -> bag<real>`); totality is preserved.
fn type_to_real(ctx: &Context, arg: &Expr) -> Result<Ty, Vec<TypeError>> {
    match type_expr(ctx, arg)? {
        Ty::Value {
            domain: ColumnType::Int,
            opt,
        } => Ok(Ty::Value {
            domain: ColumnType::Real,
            opt,
        }),
        Ty::Bag {
            domain: ColumnType::Int,
            opt,
        } => Ok(Ty::Bag {
            domain: ColumnType::Real,
            opt,
        }),
        _ => Err(vec![TypeError::new(
            "`to_real` converts an int value or bag",
            arg.span,
        )]),
    }
}

fn type_binary(ctx: &Context, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    match op {
        // `+`/`-` require *equal* numeric domains, dimension included
        // (ADR 0026), plus the three torsor rows of ADR 0036 decision 4.
        BinOp::Add | BinOp::Sub => type_add_sub(ctx, op, lhs, rhs),
        BinOp::Mul => type_mul(ctx, lhs, rhs),
        BinOp::Div => type_div(ctx, lhs, rhs),
        BinOp::Pow => type_pow(ctx, lhs, rhs),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let (_, opt) = matching_operands(
                ctx,
                lhs,
                rhs,
                ColumnType::is_orderable,
                "a comparison",
                "an orderable value (int, real, or date)",
            )?;
            Ok(Ty::Value {
                domain: ColumnType::Bool,
                opt,
            })
        }
        // `<<`/`>>` (binary minimum and maximum, ADR 0031 Decision 6) take
        // both operands of *one* orderable domain, dimension included, and
        // return that domain: the earlier of two dates and the smaller of two
        // temperatures both work.  Same operand rule as the comparisons; only
        // the result differs (the domain, not `bool`).
        BinOp::Min | BinOp::Max => {
            let (domain, opt) = matching_operands(
                ctx,
                lhs,
                rhs,
                ColumnType::is_orderable,
                "a minimum or maximum",
                "an orderable value (int, real, or date)",
            )?;
            Ok(Ty::Value { domain, opt })
        }
        // `<:`/`:>` (keep-left and keep-right) take both operands of one
        // domain and return it.  They demand no property *of* the domain,
        // only that the two agree: the operation discards a value rather than
        // inspecting it, so there is nothing to require.  In particular
        // `real` is admissible here though it is not equatable (ADR 0014),
        // since keeping a value never compares it.  Lifting is uniform
        // (ADR 0039): an absent operand makes the result absent even on the
        // discarded side.
        BinOp::KeepLeft | BinOp::KeepRight => {
            let (domain, opt) = matching_operands(
                ctx,
                lhs,
                rhs,
                |_| true,
                "keep-left or keep-right",
                "a value",
            )?;
            Ok(Ty::Value { domain, opt })
        }
        BinOp::Coalesce => type_coalesce(ctx, lhs, rhs),
        BinOp::Eq | BinOp::Ne => type_equality(ctx, lhs, rhs),
        BinOp::And | BinOp::Or => {
            let (_, opt) = matching_operands(
                ctx,
                lhs,
                rhs,
                |d| *d == ColumnType::Bool,
                "a boolean operator",
                "a boolean",
            )?;
            Ok(Ty::Value {
                domain: ColumnType::Bool,
                opt,
            })
        }
        BinOp::In => type_membership(ctx, lhs, rhs),
        BinOp::Pipe => apply_value(ctx, rhs, Some(lhs)),
    }
}

/// `??` (ADR 0039 decision 2): the coalescing discharge, the one exit from
/// optionality.  The left operand may be optional (accepting one is this
/// operator's purpose); the default must be of the same scalar domain,
/// dimension included, so `r.peak ?? 0.0` on a `temperature[real]?` is the
/// same compile error as any other dimension mismatch.  The result is total
/// exactly when the (chained) default is; a total left operand is legal and
/// simply makes the default unreachable.
fn type_coalesce(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let lt = collect_ty(type_expr(ctx, lhs), &mut errs);
    let rt = collect_ty(type_expr(ctx, rhs), &mut errs);
    let (Some(lt), Some(rt)) = (lt, rt) else {
        return Err(errs);
    };
    let (ld, lo) = as_value(&lt, "`??`", lhs.span)?;
    let (rd, ro) = as_value(&rt, "`??`", rhs.span)?;
    if ld != rd {
        return Err(vec![TypeError::new(
            format!(
                "`??` expects a default of the same type, found {} and {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            rhs.span,
        )]);
    }
    let opt = match lo {
        Optionality::Total => Optionality::Total,
        Optionality::Optional => ro,
    };
    Ok(Ty::Value { domain: ld, opt })
}

/// `+`/`-` (ADR 0014, ADR 0026, ADR 0036).  Matching numeric domains,
/// dimension included, so `meter + second` and `meter + 1.0` are rejected;
/// plus the three torsor rows of ADR 0036 decision 4, gated by
/// `formal/Mensura/Units/Torsor.lean`:
///
/// * `instant - instant : time[real]` (difference),
/// * `instant + time[real] : instant` and `instant - time[real] : instant`
///   (translation; the whole-millisecond exactness of decision 6 is checked
///   at evaluation for data-dependent durations).
///
/// Nothing else: points do not add, a point never scales, and the temporal
/// families never mix (decision 1: `instant - date` is a category error no
/// conversion silently repairs).  `diff(date)` is deferred, so `date`
/// supports no arithmetic at all.  The rejections below are spelled per
/// case, because each has a different fix and the generic "expects a
/// number" would send the author the wrong way.
fn type_add_sub(ctx: &Context, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    // Type the operands with no domain predicate up front: which pairs are
    // admissible is decided by the match below, so a torsor operand is not
    // prematurely rejected as "not a number".
    let mut errs = Vec::new();
    let ld = operand_domain(ctx, lhs, |_| true, "arithmetic", "a value", &mut errs);
    let rd = operand_domain(ctx, rhs, |_| true, "arithmetic", "a value", &mut errs);
    let (Some((ld, lo)), Some((rd, ro))) = (ld, rd) else {
        return Err(errs);
    };
    // The result's optionality is the join of the operands' (ADR 0039):
    // absence flows through the torsor rows exactly as through the numeric
    // ones.
    let opt = join_opt(lo, ro);
    let lifted = |domain: ColumnType| Ok(Ty::Value { domain, opt });
    let duration = Dimension::base("time")
        .expect("time is a base dimension")
        .applied();
    let reject = |message: String| Err(vec![TypeError::new(message, lhs.span)]);
    match (op, &ld, &rd) {
        // Difference: `instant - instant : time[real]`.
        (BinOp::Sub, ColumnType::Instant, ColumnType::Instant) => lifted(duration),
        // Points do not add (ADR 0036 decision 3).
        (BinOp::Add, ColumnType::Instant, ColumnType::Instant) => reject(
            "two instants do not add: points are not quantities (ADR 0036); \
             subtract them for a `time[real]` duration, or translate one by a \
             duration"
                .into(),
        ),
        // Translation: `instant +/- time[real] : instant`.
        (_, ColumnType::Instant, rd) if *rd == duration => lifted(ColumnType::Instant),
        // Translation writes the point on the left; a duration does not
        // left-translate (decision 4 has exactly two operation families).
        (BinOp::Add, rd, ColumnType::Instant) if *rd == duration => reject(
            "translation writes the point first: `instant + duration`, not \
             `duration + instant` (ADR 0036)"
                .into(),
        ),
        // The families never mix (decision 1).
        (_, ColumnType::Instant, ColumnType::Date) | (_, ColumnType::Date, ColumnType::Instant) => {
            reject(
                "`instant` and `date` are different temporal families (absolute \
                 versus civil, ADR 0036): no conversion relates them without a \
                 zone, so they never share an operator"
                    .into(),
            )
        }
        // `diff(date)` is deferred (ADR 0036 decision 4).
        (_, ColumnType::Date, _) | (_, _, ColumnType::Date) => reject(
            "`date` supports no arithmetic: its difference type is deferred \
             (ADR 0036 decision 4) until a consumer settles it"
                .into(),
        ),
        // Any other instant pairing: name what the domain does support.
        (_, ColumnType::Instant, other) | (_, other, ColumnType::Instant) => reject(format!(
            "`instant` subtracts another instant (giving `time[real]`) or \
             translates by a `time[real]` duration (ADR 0036); found {}",
            domain_name(other)
        )),
        // The numeric rule (ADR 0026): equal domains, dimension included.
        _ => {
            for (domain, operand) in [(&ld, lhs), (&rd, rhs)] {
                if !domain.is_numeric() {
                    errs.push(TypeError::new(
                        format!(
                            "arithmetic expects a number, found a {}",
                            domain_name(domain)
                        ),
                        operand.span,
                    ));
                }
            }
            if !errs.is_empty() {
                return Err(errs);
            }
            if ld == rd {
                lifted(ld)
            } else {
                reject(format!(
                    "arithmetic expects operands of the same type, found {} and {}",
                    domain_name(&ld),
                    domain_name(&rd)
                ))
            }
        }
    }
}

/// `*` (ADR 0026, `11-physical-units.md`): matching `int`s stay `int`; two
/// `real`-backed operands multiply their dimensions (bare `real` is the
/// group identity, and an identity result collapses back to `real`); `int`
/// never mixes with a `real`-backed domain (no coercion, ADR 0014).
fn type_mul(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let ld = operand_domain(
        ctx,
        lhs,
        ColumnType::is_numeric,
        "arithmetic",
        "a number",
        &mut errs,
    );
    let rd = operand_domain(
        ctx,
        rhs,
        ColumnType::is_numeric,
        "arithmetic",
        "a number",
        &mut errs,
    );
    let (Some((ld, lo)), Some((rd, ro))) = (ld, rd) else {
        return Err(errs);
    };
    let opt = join_opt(lo, ro);
    match (ld.dimension(), rd.dimension()) {
        (Some(a), Some(b)) => Ok(Ty::Value {
            domain: (a * b).applied(),
            opt,
        }),
        _ if ld == rd => Ok(Ty::Value { domain: ld, opt }),
        _ => Err(vec![TypeError::new(
            format!(
                "arithmetic expects operands of the same type, found {} and {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )]),
    }
}

/// `/` (ADR 0014, ADR 0026): `real`-backed operands only; the result divides
/// the dimensions, so a same-dimension ratio cancels to bare `real`.
fn type_div(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let ok = |d: &ColumnType| matches!(d, ColumnType::Real | ColumnType::Quantity(_));
    let ld = operand_domain(ctx, lhs, ok, "`/`", "a real (`/` is real only)", &mut errs);
    let rd = operand_domain(ctx, rhs, ok, "`/`", "a real (`/` is real only)", &mut errs);
    let (Some((ld, lo)), Some((rd, ro))) = (ld, rd) else {
        return Err(errs);
    };
    let (Some(a), Some(b)) = (ld.dimension(), rd.dimension()) else {
        unreachable!("`/` operands are real-backed, so both carry a dimension");
    };
    Ok(Ty::Value {
        domain: (a / b).applied(),
        opt: join_opt(lo, ro),
    })
}

/// `^` (ADR 0026, `11-physical-units.md`): on a dimensionless base the
/// status quo (matching numeric operands, so `real ^ real` and `int ^ int`);
/// on a dimensioned base the exponent must be an integer literal (optionally
/// negated), because the result dimension is computed at compile time.
/// `^ 0` cancels to bare `real`.
fn type_pow(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let ld = operand_domain(
        ctx,
        lhs,
        ColumnType::is_numeric,
        "arithmetic",
        "a number",
        &mut errs,
    );
    let Some((ld, lo)) = ld else {
        return Err(errs);
    };
    if let ColumnType::Quantity(dim) = ld {
        let Some(n) = int_literal(rhs) else {
            return Err(vec![TypeError::new(
                "the exponent of a dimensioned value must be an integer literal \
                 (the result dimension is computed at compile time)",
                rhs.span,
            )]);
        };
        let Ok(n) = i32::try_from(n) else {
            return Err(vec![TypeError::new(
                "the exponent of a dimensioned value is out of range",
                rhs.span,
            )]);
        };
        return Ok(Ty::Value {
            domain: dim.pow(n).applied(),
            opt: lo,
        });
    }
    let rd = operand_domain(
        ctx,
        rhs,
        ColumnType::is_numeric,
        "arithmetic",
        "a number",
        &mut errs,
    );
    let Some((rd, ro)) = rd else {
        return Err(errs);
    };
    if ld == rd {
        Ok(Ty::Value {
            domain: ld,
            opt: join_opt(lo, ro),
        })
    } else {
        Err(vec![TypeError::new(
            format!(
                "arithmetic expects operands of the same type, found {} and {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )])
    }
}

/// The value of an integer-literal expression, seeing through one unary
/// negation (`x ^ -2` parses as `x ^ (-2)`).
fn int_literal(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Int(n) => Some(*n),
        ExprKind::Unary(UnOp::Neg, inner) => match &inner.kind {
            ExprKind::Int(n) => n.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

/// Type both operands as values whose domain satisfies `ok` and that match
/// each other; return the common domain and the joined optionality
/// (ADR 0039: an optional operand is accepted and its absence propagates,
/// so the result is optional when either operand is).  `label`/`expected`
/// shape the messages.
fn matching_operands(
    ctx: &Context,
    lhs: &Expr,
    rhs: &Expr,
    ok: fn(&ColumnType) -> bool,
    label: &str,
    expected: &str,
) -> Result<(ColumnType, Optionality), Vec<TypeError>> {
    let mut errs = Vec::new();
    let ld = operand_domain(ctx, lhs, ok, label, expected, &mut errs);
    let rd = operand_domain(ctx, rhs, ok, label, expected, &mut errs);
    let (Some((ld, lo)), Some((rd, ro))) = (ld, rd) else {
        return Err(errs);
    };
    if ld == rd {
        Ok((ld, join_opt(lo, ro)))
    } else {
        Err(vec![TypeError::new(
            format!(
                "{label} expects operands of the same type, found {} and {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )])
    }
}

fn operand_domain(
    ctx: &Context,
    operand: &Expr,
    ok: fn(&ColumnType) -> bool,
    label: &str,
    expected: &str,
    errs: &mut Vec<TypeError>,
) -> Option<(ColumnType, Optionality)> {
    match type_expr(ctx, operand) {
        Err(e) => {
            errs.extend(e);
            None
        }
        Ok(ty) => match as_value(&ty, label, operand.span) {
            Err(e) => {
                errs.extend(e);
                None
            }
            Ok((domain, opt)) if ok(&domain) => Some((domain, opt)),
            Ok((domain, _)) => {
                errs.push(TypeError::new(
                    format!(
                        "{label} expects {expected}, found a {}",
                        domain_name(&domain)
                    ),
                    operand.span,
                ));
                None
            }
        },
    }
}

/// `value in bag` (section 5.4): the left side a value matching the bag's
/// element domain; the right side a bag of *total* elements, since
/// membership quantifies over the bag's values the way a fold does
/// (ADR 0039 decision 3).  An optional left side lifts, so the result is
/// then an optional boolean.
fn type_membership(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let lt = collect_ty(type_expr(ctx, lhs), &mut errs);
    let rt = collect_ty(type_expr(ctx, rhs), &mut errs);
    let (Some(lt), Some(rt)) = (lt, rt) else {
        return Err(errs);
    };
    let elem = match rt {
        Ty::Bag {
            domain,
            opt: Optionality::Total,
        } => domain,
        Ty::Bag { .. } => {
            errs.push(TypeError::new(
                "`in` quantifies over the bag's values, so it requires a total \
                 bag; discharge the elements with `??` upstream (ADR 0039)",
                rhs.span,
            ));
            return Err(errs);
        }
        _ => {
            errs.push(TypeError::new("`in` expects a bag on the right", rhs.span));
            return Err(errs);
        }
    };
    match as_value(&lt, "`in`", lhs.span) {
        Ok((domain, opt)) if domain == elem => Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt,
        }),
        Ok((domain, _)) => Err(vec![TypeError::new(
            format!(
                "`in` expects a {} value to match the bag, found a {}",
                domain_name(&elem),
                domain_name(&domain)
            ),
            lhs.span,
        )]),
        Err(e) => {
            errs.extend(e);
            Err(errs)
        }
    }
}

/// `is known` / `is missing` (section 5.6): apply to a value, yield a total
/// boolean.  Narrowing is deferred (ADR 0039 alternative 4), so `is known`
/// does not change the value's totality.
fn type_presence(ctx: &Context, base: &Expr, span: Span) -> Result<Ty, Vec<TypeError>> {
    match type_expr(ctx, base)? {
        Ty::Value { .. } => Ok(total(ColumnType::Bool)),
        _ => Err(vec![TypeError::new(
            "`is known` / `is missing` apply to a value",
            span,
        )]),
    }
}

fn type_unary(ctx: &Context, op: UnOp, operand: &Expr) -> Result<Ty, Vec<TypeError>> {
    match op {
        UnOp::Neg => {
            let mut errs = Vec::new();
            let d = operand_domain(
                ctx,
                operand,
                ColumnType::is_numeric,
                "negation",
                "a number",
                &mut errs,
            );
            match d {
                Some((domain, opt)) if errs.is_empty() => Ok(Ty::Value { domain, opt }),
                _ => Err(errs),
            }
        }
        // `not` lifts like the binary boolean operators (ADR 0039): the
        // negation of an absent boolean is absent.
        UnOp::Not => {
            let mut errs = Vec::new();
            let d = operand_domain(
                ctx,
                operand,
                |d| *d == ColumnType::Bool,
                "`not`",
                "a boolean",
                &mut errs,
            );
            match d {
                Some((domain, opt)) if errs.is_empty() => Ok(Ty::Value { domain, opt }),
                _ => Err(errs),
            }
        }
        // `#` (cardinality, ADR 0031 Decision 9): `#e == fold `+` (|_| 1) e`,
        // so it consumes a bag and yields an `int`.  Unlike the value
        // reductions it does *not* require a total bag: the mapper discards
        // the element, so a missing value still counts its row.  Backed by
        // Stage 1 (`Mensura.foldBag`, the additive-monoid instance).
        //
        // Both bag shapes count.  `#b.x` counts a projected bag, and `#b`
        // counts the fiber itself, which is the group's row count and the
        // point of Decision 1: today one writes `#b.x` and arbitrarily
        // picks a column.  They agree, since projection preserves cardinality.
        UnOp::Card => match type_expr(ctx, operand)? {
            Ty::Bag { .. } | Ty::Rows(_) => Ok(total(ColumnType::Int)),
            other => Err(vec![TypeError::new(
                format!("`#` counts a bag or a group, found {}", describe_ty(&other)),
                operand.span,
            )]),
        },
    }
}

/// `==` / `!=` (equatable, ADR 0014): both sides values of the same
/// equatable domain (so `real` is rejected), with the enum-vs-string-literal
/// exception of section 5.7.  Optional operands lift (ADR 0039): the
/// comparison is then an optional boolean, absent when either side is,
/// never a three-valued "missing equals missing".
fn type_equality(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let lt = collect_ty(type_expr(ctx, lhs), &mut errs);
    let rt = collect_ty(type_expr(ctx, rhs), &mut errs);
    let (Some(lt), Some(rt)) = (lt, rt) else {
        return Err(errs);
    };

    if let Some(res) = enum_vs_literal(&lt, rhs) {
        return res;
    }
    if let Some(res) = enum_vs_literal(&rt, lhs) {
        return res;
    }

    let ld = equatable_value(&lt, lhs.span, &mut errs);
    let rd = equatable_value(&rt, rhs.span, &mut errs);
    let (Some((ld, lo)), Some((rd, ro))) = (ld, rd) else {
        return Err(errs);
    };
    if ld == rd {
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: join_opt(lo, ro),
        })
    } else {
        Err(vec![TypeError::new(
            format!(
                "`==`/`!=` expects matching types, found a {} and a {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )])
    }
}

fn equatable_value(
    ty: &Ty,
    span: Span,
    errs: &mut Vec<TypeError>,
) -> Option<(ColumnType, Optionality)> {
    match as_value(ty, "`==`/`!=`", span) {
        Err(e) => {
            errs.extend(e);
            None
        }
        Ok((domain, opt)) if domain.is_equatable() => Some((domain, opt)),
        Ok((domain, _)) => {
            errs.push(TypeError::new(
                format!("`==`/`!=` is not defined on {}", domain_name(&domain)),
                span,
            ));
            None
        }
    }
}

fn collect_ty(result: Result<Ty, Vec<TypeError>>, errs: &mut Vec<TypeError>) -> Option<Ty> {
    match result {
        Ok(ty) => Some(ty),
        Err(e) => {
            errs.extend(e);
            None
        }
    }
}

/// The section 5.7 exception: an enum value compared to a string literal,
/// validating the literal against the variant set. `None` if `value` is not an
/// enum or `other` is not a string literal.  An optional enum lifts like any
/// other operand (ADR 0039): the comparison is then an optional boolean.
fn enum_vs_literal(value: &Ty, other: &Expr) -> Option<Result<Ty, Vec<TypeError>>> {
    let Ty::Value {
        domain: ColumnType::Enum { name, variants },
        opt,
    } = value
    else {
        return None;
    };
    let ExprKind::Str(lit) = &other.kind else {
        return None;
    };
    if variants.iter().any(|v| v == lit) {
        Some(Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: *opt,
        }))
    } else {
        Some(Err(vec![TypeError::new(
            format!("`{lit}` is not a variant of `{name}`"),
            other.span,
        )]))
    }
}

/// The domain and optionality of `ty` as a single value, else a located
/// error (the scalar rule, section 5.3, under ADR 0039's lifting: the
/// cardinality axis is still hard, but an optional value is accepted and
/// its absence propagates through the operator).
fn as_value(ty: &Ty, what: &str, span: Span) -> Result<(ColumnType, Optionality), Vec<TypeError>> {
    match ty {
        Ty::Value { domain, opt } => Ok((domain.clone(), *opt)),
        Ty::Bag { .. } => Err(vec![TypeError::new(
            format!("{what} expects a single value, found a bag"),
            span,
        )]),
        // Bare `b` was a type error before ADR 0031 (a record is not a value)
        // and remains one (a bag of rows is not a value), so no existing
        // program reads differently; only the noun changes.  The hint names
        // the projection, since that is what the writer almost always wanted.
        Ty::Rows(_) => Err(vec![TypeError::new(
            format!("{what} expects a value, found a bag of rows (project a column, `b.name`)"),
            span,
        )]),
        Ty::Record(_) => Err(vec![TypeError::new(
            format!("{what} expects a value, found a row"),
            span,
        )]),
        // A descending marker orders a key; it is not itself a value one can
        // compute with or store (ADR 0031, Decision 7).
        Ty::Desc(_) => Err(vec![TypeError::new(
            format!(
                "{what} expects a value, found a descending marker; `desc` \
                 belongs in a scan's order key, not in a value position"
            ),
            span,
        )]),
        Ty::Fn(_) | Ty::Builtin(_) => Err(vec![TypeError::new(
            format!("{what} expects a value, found a function (apply it)"),
            span,
        )]),
    }
}

/// `if c then a else b` (section 5, ADR 0015): `c` is a total boolean and the
/// two branches unify to one value type, which is the result.
fn type_if(
    ctx: &Context,
    cond: &Expr,
    then: &Expr,
    els: &Expr,
    span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let mut errs = require_bool(ctx, cond, "an `if` condition");
    let then_ty = collect_ty(type_expr(ctx, then), &mut errs);
    let els_ty = collect_ty(type_expr(ctx, els), &mut errs);
    let (Some(then_ty), Some(els_ty)) = (then_ty, els_ty) else {
        return Err(errs);
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    unify_branches(&then_ty, &els_ty, span)
}

/// Unify the two branch types of a conditional. Two values of the same domain
/// merge (the result is optional if either branch is); otherwise the branches
/// must be identical. A mismatch is a located error.
fn unify_branches(then_ty: &Ty, els_ty: &Ty, span: Span) -> Result<Ty, Vec<TypeError>> {
    match (then_ty, els_ty) {
        (
            Ty::Value {
                domain: da,
                opt: oa,
            },
            Ty::Value {
                domain: db,
                opt: ob,
            },
        ) if da == db => Ok(Ty::Value {
            domain: da.clone(),
            opt: join_opt(*oa, *ob),
        }),
        _ if then_ty == els_ty => Ok(then_ty.clone()),
        _ => Err(vec![TypeError::new(
            format!(
                "the `if` branches must have the same type, found {} and {}",
                describe_ty(then_ty),
                describe_ty(els_ty)
            ),
            span,
        )]),
    }
}

/// The optional axis of a merge: optional if either input is.
fn join_opt(a: Optionality, b: Optionality) -> Optionality {
    if a == Optionality::Optional || b == Optionality::Optional {
        Optionality::Optional
    } else {
        Optionality::Total
    }
}

/// A short human description of a `Ty` for diagnostics.
fn describe_ty(ty: &Ty) -> String {
    match ty {
        Ty::Value { domain, .. } => format!("a {}", domain_name(domain)),
        Ty::Bag { domain, .. } => format!("a bag of {}", domain_name(domain)),
        Ty::Rows(_) => "a bag of rows".to_string(),
        Ty::Record(_) => "a record".to_string(),
        Ty::Desc(inner) => format!("a descending {}", ty_name(inner)),
        Ty::Fn(_) | Ty::Builtin(_) => "a function".to_string(),
    }
}

/// Require `operand` to be a *total* boolean: the branching boundary of
/// ADR 0039 decision 3.  Absence may flow through values, never past a
/// decision unconsulted, so an optional boolean is rejected with the
/// policy the author must state.
fn require_bool(ctx: &Context, operand: &Expr, what: &str) -> Vec<TypeError> {
    match type_expr(ctx, operand) {
        Err(errs) => errs,
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Total,
        }) => Vec::new(),
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Optional,
        }) => vec![TypeError::new(
            format!(
                "{what} may be missing; state the absent-row policy with \
                 `?? true` or `?? false` (ADR 0039)"
            ),
            operand.span,
        )],
        Ok(_) => vec![TypeError::new(
            format!("{what} expects a boolean"),
            operand.span,
        )],
    }
}

fn domain_name(domain: &ColumnType) -> String {
    match domain {
        ColumnType::String => "string".into(),
        ColumnType::Int => "int".into(),
        ColumnType::Real => "real".into(),
        ColumnType::Quantity(dim) => dim.type_name(),
        ColumnType::Bool => "bool".into(),
        ColumnType::Date => "date".into(),
        ColumnType::Instant => "instant".into(),
        ColumnType::Enum { .. } => "enum".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column as StorageColumn, ColumnRole, Schema};
    use mensura_syntax::StoreKind;

    fn scol(name: &str, ty: ColumnType, role: ColumnRole, optional: bool) -> StorageColumn {
        StorageColumn {
            name: name.to_string(),
            ty,
            role,
            optional,
            span: Span::new(0, 0),
        }
    }

    fn sample_table() -> TableType {
        let schema = Schema {
            store: "readings".to_string(),
            kind: StoreKind::Store,
            unit: "Machine".to_string(),
            columns: vec![
                scol("machine", ColumnType::String, ColumnRole::Key, false),
                scol("size", ColumnType::Int, ColumnRole::Attr, false),
                scol("temperature", ColumnType::Real, ColumnRole::Attr, false),
                scol("peak", ColumnType::Real, ColumnRole::Attr, true),
                scol("note", ColumnType::String, ColumnRole::Attr, true),
                scol("at", ColumnType::Date, ColumnRole::Attr, false),
                scol(
                    "status",
                    ColumnType::Enum {
                        name: "Status".to_string(),
                        variants: vec!["active".to_string(), "closed".to_string()],
                    },
                    ColumnRole::Attr,
                    false,
                ),
                scol("flag", ColumnType::Bool, ColumnRole::Attr, false),
                scol(
                    "kelvin_reading",
                    ColumnType::Quantity(Dimension::base("temperature").unwrap()),
                    ColumnRole::Attr,
                    false,
                ),
                scol("started", ColumnType::Instant, ColumnRole::Attr, false),
                scol("ended", ColumnType::Instant, ColumnRole::Attr, false),
                scol("confirmed", ColumnType::Instant, ColumnRole::Attr, true),
            ],
            cardinality: crate::table::Cardinality::Singletons,
            foreign_keys: Vec::new(),
            lateness: Vec::new(),
            span: Span::new(0, 0),
        };
        TableType::from_store(&schema)
    }

    fn ty_of(ctx: &Context, src: &str) -> Result<Ty, Vec<TypeError>> {
        let toks = mensura_syntax::tokenize(src).expect("lex");
        let expr = mensura_syntax::parse_expr(&toks).expect("parse");
        type_expr(ctx, &expr)
    }

    fn row_ctx() -> Context {
        Context::row(&test_ambient(), "k", "r", &sample_table())
    }

    fn bag_ctx() -> Context {
        Context::bag(&test_ambient(), "k", "b", &sample_table())
    }

    /// The ambient a program gets from `import bag` (ADR 0031, Decision 8).
    /// These tests type bare expressions, so there is no `import` item to
    /// resolve; injecting the module's env is the equivalent, and keeps the
    /// fixtures reading as a user would write them.
    fn test_ambient() -> Ambient {
        let mut ambient = intrinsics();
        for name in ["bag", "series"] {
            let env = crate::modules::bundled(name)
                .expect("bundled")
                .as_ref()
                .expect("resolves cleanly");
            let members = env
                .values
                .iter()
                .filter_map(|(n, v)| Some((n.clone(), v.ty()?.from_module())))
                .collect();
            ambient.insert(name.to_string(), Ty::Record(members));
        }
        ambient
    }

    #[test]
    fn operators_lift_over_optional_operands() {
        // ADR 0039 decision 1: any optional operand makes the result
        // optional; the domain and dimension rules are unchanged, applied
        // under the `?`.
        let ctx = row_ctx();
        let opt = |domain: ColumnType| Ty::Value {
            domain,
            opt: Optionality::Optional,
        };
        // Comparisons and equality yield an optional boolean.
        assert_eq!(ty_of(&ctx, "r.peak > 1.0"), Ok(opt(ColumnType::Bool)));
        assert_eq!(ty_of(&ctx, "r.note == \"x\""), Ok(opt(ColumnType::Bool)));
        // The torsor rows lift: an optional instant's difference is an
        // optional duration, and translating by it stays optional.
        let duration = Dimension::base("time").unwrap().applied();
        assert_eq!(
            ty_of(&ctx, "r.ended - r.confirmed"),
            Ok(opt(duration.clone()))
        );
        assert_eq!(
            ty_of(&ctx, "r.started + (r.ended - r.confirmed)"),
            Ok(opt(ColumnType::Instant))
        );
        // `not` and `and` lift through an optional boolean.
        assert_eq!(ty_of(&ctx, "not (r.peak > 1.0)"), Ok(opt(ColumnType::Bool)));
        assert_eq!(
            ty_of(&ctx, "r.flag and (r.peak > 1.0)"),
            Ok(opt(ColumnType::Bool))
        );
        // Dimension arithmetic under the `?`: optional real times a kelvin
        // quantity is an optional quantity.
        assert_eq!(
            ty_of(&ctx, "r.peak * r.kelvin_reading"),
            Ok(opt(ColumnType::Quantity(
                Dimension::base("temperature").unwrap()
            )))
        );
        // The bool representations are one (the Ty::Bool collapse): a bool
        // column compares against a literal.
        assert_eq!(ty_of(&ctx, "r.flag == true"), Ok(total(ColumnType::Bool)));
    }

    #[test]
    fn decision_boundaries_demand_total() {
        // ADR 0039 decision 3: absence flows through values, never past a
        // decision unconsulted.
        let ctx = row_ctx();
        // An optional condition names the policy in its rejection.
        let errs = ty_of(&ctx, "if r.peak > 1.0 then 1 else 0").expect_err("optional cond");
        assert!(errs[0].message.contains("?? true"), "{}", errs[0].message);
        // Discharged, it types; the branches still unify.
        assert_eq!(
            ty_of(&ctx, "if (r.peak > 1.0) ?? false then 1 else 0"),
            Ok(total(ColumnType::Int))
        );
        // A presence test applies to a lifted result and stays total.
        assert_eq!(
            ty_of(&ctx, "(r.peak + 1.0) is known"),
            Ok(total(ColumnType::Bool))
        );
    }

    #[test]
    fn coalesce_discharges_optionality() {
        // ADR 0039 decision 2: `??` is the exit from optionality.  A total
        // default makes the result total; an optional default keeps the
        // chain optional; a total left operand is legal.
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.peak ?? 0.0"), Ok(total(ColumnType::Real)));
        assert_eq!(
            ty_of(&ctx, "r.peak ?? r.temperature"),
            Ok(total(ColumnType::Real))
        );
        assert_eq!(
            ty_of(&ctx, "r.peak ?? r.peak"),
            Ok(Ty::Value {
                domain: ColumnType::Real,
                opt: Optionality::Optional
            })
        );
        assert_eq!(
            ty_of(&ctx, "r.peak ?? r.peak ?? 0.0"),
            Ok(total(ColumnType::Real))
        );
        assert_eq!(
            ty_of(&ctx, "r.note ?? \"unnamed\""),
            Ok(total(ColumnType::String))
        );
        assert_eq!(
            ty_of(&ctx, "r.temperature ?? 0.0"),
            Ok(total(ColumnType::Real))
        );
        // A parenthesized comparison discharges to a total boolean.
        assert_eq!(
            ty_of(&ctx, "(r.temperature > 1.0) ?? true"),
            Ok(total(ColumnType::Bool))
        );
    }

    #[test]
    fn coalesce_demands_a_matching_default() {
        let ctx = row_ctx();
        let msg = |src: &str| ty_of(&ctx, src).expect_err(src)[0].message.clone();
        // The default's domain must match, dimension included (the `0.0`
        // here is a bare real against a temperature).
        assert!(msg("r.peak ?? 1").contains("same type"));
        assert!(msg("r.kelvin_reading ?? 0.0").contains("same type"));
        // A bag is not a value; the cardinality axis is untouched.
        let bag = bag_ctx();
        let e = ty_of(&bag, "b.peak ?? 0.0").expect_err("bag operand");
        assert!(e[0].message.contains("value"), "{}", e[0].message);
    }

    #[test]
    fn torsor_rows_type_instant_arithmetic() {
        // ADR 0036 decision 4, gated by `formal/Mensura/Units/Torsor.lean`:
        // difference is a `time[real]` duration, translation returns the
        // point, and nothing else types.
        let ctx = row_ctx();
        let duration = total(Dimension::base("time").unwrap().applied());
        assert_eq!(ty_of(&ctx, "r.ended - r.started"), Ok(duration));
        assert_eq!(
            ty_of(&ctx, "r.started + (r.ended - r.started)"),
            Ok(total(ColumnType::Instant))
        );
        assert_eq!(
            ty_of(&ctx, "r.ended - (r.ended - r.started)"),
            Ok(total(ColumnType::Instant))
        );
    }

    #[test]
    fn torsor_rejections_name_their_rule() {
        let ctx = row_ctx();
        let msg = |src: &str| ty_of(&ctx, src).expect_err(src)[0].message.clone();
        // Points do not add (decision 3).
        assert!(msg("r.started + r.ended").contains("points are not quantities"));
        // The families never mix (decision 1); `r.at` is a `date`.
        assert!(msg("r.started - r.at").contains("different temporal families"));
        // `diff(date)` is deferred (decision 4).
        assert!(msg("r.at - r.at").contains("deferred"));
        // Translation is by a `time[real]` only, and writes the point first.
        assert!(msg("r.started + r.kelvin_reading").contains("time[real]"));
        assert!(msg("r.started + 1.0").contains("time[real]"));
        assert!(msg("(r.ended - r.started) + r.started").contains("point first"));
    }

    #[test]
    fn literals_split_int_and_real() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "42"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "3.5"), Ok(total(ColumnType::Real)));
        assert_eq!(ty_of(&ctx, "\"hi\""), Ok(total(ColumnType::String)));
        assert_eq!(ty_of(&ctx, "true"), Ok(total(ColumnType::Bool)));
    }

    #[test]
    fn member_reads_columns_with_totality() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.temperature"), Ok(total(ColumnType::Real)));
        assert_eq!(
            ty_of(&ctx, "r.note"),
            Ok(Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Optional
            })
        );
        assert!(ty_of(&ctx, "r.missing").is_err());
        // Key-first split (ADR 0015): the key column lives on `k`, not `r`.
        assert_eq!(ty_of(&ctx, "k.machine"), Ok(total(ColumnType::String)));
        let errs = ty_of(&ctx, "r.machine").expect_err("key column is not on r");
        assert!(errs[0].message.contains("unknown column"));
    }

    #[test]
    fn arithmetic_requires_matching_numeric() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.size + 1"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "2 ^ 3"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "-r.size"), Ok(total(ColumnType::Int)));
        assert_eq!(
            ty_of(&ctx, "r.temperature + 1.0"),
            Ok(total(ColumnType::Real))
        );
        // int mixed with real is a type error (no coercion).
        let errs = ty_of(&ctx, "r.size + 1.0").expect_err("mixed");
        assert!(errs[0].message.contains("same type"));
        // arithmetic on a non-number.
        assert!(ty_of(&ctx, "k.machine + 1").is_err());
        // An optional operand lifts (ADR 0039): the sum is optional, absent
        // when `peak` is.
        assert_eq!(
            ty_of(&ctx, "r.peak + 1.0"),
            Ok(Ty::Value {
                domain: ColumnType::Real,
                opt: Optionality::Optional
            })
        );
    }

    #[test]
    fn division_is_real_only() {
        let ctx = row_ctx();
        assert_eq!(
            ty_of(&ctx, "r.temperature / 2.0"),
            Ok(total(ColumnType::Real))
        );
        let errs = ty_of(&ctx, "r.size / 2").expect_err("int division");
        assert!(errs[0].message.contains("real"));
    }

    #[test]
    fn ordering_is_orderable_including_date() {
        let ctx = row_ctx();
        assert_eq!(
            ty_of(&ctx, "r.temperature > 30.0"),
            Ok(total(ColumnType::Bool))
        );
        assert_eq!(ty_of(&ctx, "r.size < 2"), Ok(total(ColumnType::Bool)));
        assert_eq!(ty_of(&ctx, "r.at < r.at"), Ok(total(ColumnType::Bool))); // date is orderable
        let errs = ty_of(&ctx, "k.machine < \"z\"").expect_err("string not orderable");
        assert!(errs[0].message.contains("orderable"));
    }

    #[test]
    fn equality_excludes_real_and_validates_enum() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.size == 1"), Ok(total(ColumnType::Bool)));
        assert_eq!(
            ty_of(&ctx, "k.machine == \"m1\""),
            Ok(total(ColumnType::Bool))
        );
        assert_eq!(
            ty_of(&ctx, "r.status == \"active\""),
            Ok(total(ColumnType::Bool))
        );
        assert_eq!(ty_of(&ctx, "r.at == r.at"), Ok(total(ColumnType::Bool)));
        let errs = ty_of(&ctx, "r.temperature == 30.0").expect_err("real equality");
        assert!(errs[0].message.contains("not defined on real"));
        let errs = ty_of(&ctx, "r.status == \"activ\"").expect_err("bad variant");
        assert!(errs[0].message.contains("not a variant of `Status`"));
    }

    #[test]
    fn conditional_unifies_branches() {
        let ctx = row_ctx();
        // Both branches a total int.
        assert_eq!(
            ty_of(&ctx, "if true then 1 else 2"),
            Ok(total(ColumnType::Int))
        );
        // A row predicate as the condition.
        assert_eq!(
            ty_of(&ctx, "if r.flag then 1 else 2"),
            Ok(total(ColumnType::Int))
        );
        // An optional branch makes the whole conditional optional.
        assert_eq!(
            ty_of(&ctx, "if r.flag then r.note else \"x\""),
            Ok(Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Optional
            })
        );
        // Mismatched branch domains.
        let errs = ty_of(&ctx, "if true then 1 else \"x\"").expect_err("branch mismatch");
        assert!(errs[0].message.contains("same type"));
        // A non-boolean condition.
        let errs = ty_of(&ctx, "if 1 then 1 else 2").expect_err("non-bool condition");
        assert!(errs[0].message.contains("boolean"));
    }

    #[test]
    fn aggregates_have_per_domain_signatures() {
        let ctx = bag_ctx();
        assert_eq!(ty_of(&ctx, "#b.size"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "bag.sum b.size"), Ok(total(ColumnType::Int)));
        assert_eq!(
            ty_of(&ctx, "bag.sum b.temperature"),
            Ok(total(ColumnType::Real))
        );
        assert_eq!(ty_of(&ctx, "bag.min b.at"), Ok(total(ColumnType::Date))); // date is orderable
        assert_eq!(
            ty_of(&ctx, "bag.max b.temperature"),
            Ok(total(ColumnType::Real))
        );
        assert_eq!(ty_of(&ctx, "bag.any b.flag"), Ok(total(ColumnType::Bool)));
        // sum on a non-numeric bag, min on a non-orderable bag.
        assert!(ty_of(&ctx, "bag.sum b.note").is_err());
        assert!(ty_of(&ctx, "bag.min b.note").is_err());
    }

    #[test]
    fn mean_is_not_a_primitive() {
        let ctx = bag_ctx();
        // `mean` is gone; it is recovered from sum/count/to_real.
        assert!(ty_of(&ctx, "mean b.temperature").is_err());
        assert_eq!(
            ty_of(&ctx, "bag.sum b.temperature / to_real (#b.temperature)"),
            Ok(total(ColumnType::Real))
        );
    }

    #[test]
    fn aggregates_require_a_total_bag() {
        let ctx = bag_ctx();
        let errs = ty_of(&ctx, "bag.sum b.peak").expect_err("optional bag");
        assert!(errs[0].message.contains("total bag"));
    }

    #[test]
    fn the_fiber_projects_to_a_bag() {
        // ADR 0031, Decisions 1 and 2: `b` is the bag of rows, and `b.x` is
        // the projection `map (|r| r.x) b`, which is today's `bag<T>`.  Every
        // existing aggregate site therefore keeps its exact spelling and
        // meaning; this test is the statement that the sugar is faithful.
        let ctx = bag_ctx();
        assert_eq!(
            ty_of(&ctx, "b.size"),
            Ok(Ty::Bag {
                domain: ColumnType::Int,
                opt: Optionality::Total,
            })
        );
        // Optionality survives the projection, so a partial column still
        // fails the total-bag demand above.
        assert_eq!(
            ty_of(&ctx, "b.peak"),
            Ok(Ty::Bag {
                domain: ColumnType::Real,
                opt: Optionality::Optional,
            })
        );
    }

    #[test]
    fn a_bare_fiber_is_not_a_value() {
        // Unchanged behaviour, new noun: bare `b` was an error before the rows
        // model (a record is not a value) and remains one (a bag of rows is
        // not a value), so no existing program reads differently.
        let ctx = bag_ctx();
        let errs = ty_of(&ctx, "b + 1").expect_err("the fiber is not a value");
        assert!(
            errs[0].message.contains("bag of rows"),
            "diagnostic should name the fiber: {}",
            errs[0].message
        );
    }

    #[test]
    fn an_unknown_projection_reads_like_an_unknown_column() {
        let ctx = bag_ctx();
        let errs = ty_of(&ctx, "b.nope").expect_err("no such column");
        assert!(errs[0].message.contains("unknown column `nope`"));
    }

    #[test]
    fn cardinality_counts_both_bag_shapes() {
        // `#b` is the group's row count (Decision 1's headline) and `#b.x` a
        // projected bag's size; both are `int`.  `#b.peak` is admitted despite
        // `peak` being optional, since counting never reads a value.
        let ctx = bag_ctx();
        assert_eq!(ty_of(&ctx, "#b"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "#b.size"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "#b.peak"), Ok(total(ColumnType::Int)));
        // A value has no cardinality to take.
        let row = row_ctx();
        let errs = ty_of(&row, "#r.size").expect_err("a value is not a bag");
        assert!(errs[0].message.contains("counts a bag or a group"));
    }

    #[test]
    fn fold_types_over_a_projected_bag() {
        // ADR 0031, Decision 4: combiner, mapper, trailing bag.  Over a
        // projected bag the element is a value.
        let ctx = bag_ctx();
        assert_eq!(
            ty_of(&ctx, "fold `+` (|v| v) b.size"),
            Ok(total(ColumnType::Int))
        );
        // The mapper is open: any well-typed expression over the element.
        assert_eq!(
            ty_of(&ctx, "fold `+` (|v| v * v) b.size"),
            Ok(total(ColumnType::Int))
        );
        // The pipe is the same application path (ADR 0018).
        assert_eq!(
            ty_of(&ctx, "b.size |> fold `+` (|v| v)"),
            Ok(total(ColumnType::Int))
        );
    }

    #[test]
    fn fold_over_the_fiber_takes_a_row_mapper() {
        // ADR 0029's headline example, expressible only once `b` became the
        // bag of rows (ADR 0031, Decisions 1 and 4): the element is a *row*.
        let ctx = bag_ctx();
        assert_eq!(
            ty_of(&ctx, "fold `+` (|r| r.size) b"),
            Ok(total(ColumnType::Int))
        );
    }

    #[test]
    fn map_is_the_explicit_projection() {
        // Decision 3: `map (|r| r.x) b` is the explicit spelling of `b.x`, and
        // it also expresses a computed bag no projection sigil could.
        let ctx = bag_ctx();
        assert_eq!(ty_of(&ctx, "map (|r| r.size) b"), ty_of(&ctx, "b.size"));
        assert_eq!(
            ty_of(&ctx, "map (|v| v * 2) b.size"),
            Ok(Ty::Bag {
                domain: ColumnType::Int,
                opt: Optionality::Total,
            })
        );
    }

    #[test]
    fn a_partial_reduction_is_a_value() {
        // ADR 0031, Decision 11: short of saturation a builtin is an ordinary
        // function value, which is what lets `bag` bind `fold `+` (|v| v)` to
        // a name (Decision 8).
        let ctx = bag_ctx();
        assert!(matches!(
            ty_of(&ctx, "fold `+` (|v| v)"),
            Ok(Ty::Builtin(_))
        ));
        assert!(matches!(ty_of(&ctx, "fold `+`"), Ok(Ty::Builtin(_))));
    }

    #[test]
    fn the_combiner_table_is_closed_and_per_primitive() {
        let ctx = bag_ctx();
        // An unknown combiner names the table; the set extends by ADR, never
        // by a call site.
        let errs = ty_of(&ctx, "fold `%` (|v| v) b.size").expect_err("not a row");
        assert!(errs[0].message.contains("is not a combiner"));
        assert!(errs[0].message.contains("`+`"));
        // The tacks are associative but not commutative, so they are
        // `scan`-only (Decision 6's ordered-only column).
        let errs = ty_of(&ctx, "fold `<:` (|v| v) b.size").expect_err("scan only");
        assert!(errs[0].message.contains("not commutative"));
        // A combiner is not a value.
        let errs = ty_of(&ctx, "`+`").expect_err("not a value");
        assert!(errs[0].message.contains("not a value"));
        // The combiner slot takes an operator, never a computed function.
        let errs = ty_of(&ctx, "fold (|a| a) (|v| v) b.size").expect_err("needs a combiner");
        assert!(errs[0].message.contains("backticked combiner"));
    }

    /// The table's admission column mirrors `09-typing-reference.md` section
    /// 5.4 row by row.  Pinned as data rather than derived from `commutative`,
    /// so a future row cannot drift from the documented table silently, and so
    /// the doc and the code fail together (the spirit of the module oracles).
    #[test]
    fn the_admission_column_mirrors_the_documented_table() {
        // (spelling, admitted under `fold`, admitted under `scan`)
        let documented = [
            ("+", true, true),
            ("*", true, true),
            ("<<", true, true),
            (">>", true, true),
            ("or", true, true),
            ("and", true, true),
            // The ordered-only column: associative but not commutative, so a
            // key must supply the order a bag lacks.
            ("<:", false, true),
            (":>", false, true),
        ];
        assert_eq!(
            documented.len(),
            COMBINERS.len(),
            "the table gained or lost a row without the doc column following"
        );
        for (spelling, folds, scans) in documented {
            let row = COMBINERS
                .iter()
                .find(|r| r.spelling == spelling)
                .unwrap_or_else(|| panic!("no row spelled `{spelling}`"));
            assert_eq!(row.folds, folds, "`{spelling}`'s `fold` admission");
            assert_eq!(row.scans, scans, "`{spelling}`'s `scan` admission");
        }
        // Every row is scan-admitted, which is what makes the tacks combiners
        // at all; the fold-admitted set is strictly smaller.
        assert!(COMBINERS.iter().all(|r| r.scans));
        assert!(COMBINERS.iter().filter(|r| r.folds).count() < COMBINERS.len());
    }

    /// The optionality matrix of ADR 0031 Decision 7, all four cells.  An
    /// inclusive scan is total at every row because position `i` folds a
    /// non-empty prefix; an exclusive scan's first position folds the *empty*
    /// prefix, so an identity-free combiner has no answer there.
    #[test]
    fn the_scan_optionality_matrix() {
        let ctx = bag_ctx();
        let bag = |domain, opt| Ty::Bag { domain, opt };
        // scan, identity row: total.
        assert_eq!(
            ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b"),
            Ok(bag(ColumnType::Real, Optionality::Total))
        );
        // scan, identity-free row: still total, because no prefix is empty.
        assert_eq!(
            ty_of(&ctx, "scan `>>` (|r| r.temperature) (|r| r.at) b"),
            Ok(bag(ColumnType::Real, Optionality::Total))
        );
        // prescan, identity row: total, the identity answers the empty prefix.
        assert_eq!(
            ty_of(&ctx, "prescan `+` (|r| r.temperature) (|r| r.at) b"),
            Ok(bag(ColumnType::Real, Optionality::Total))
        );
        // prescan, identity-free row: OPTIONAL.  This is `lag`'s missing first
        // row, and it falls out of the row's `has_identity` rather than from
        // any rule about `lag` (ADR 0031, Decision 7).
        assert_eq!(
            ty_of(&ctx, "prescan `:>` (|r| r.temperature) (|r| r.at) b"),
            Ok(bag(ColumnType::Real, Optionality::Optional))
        );
    }

    #[test]
    fn a_scan_admits_the_tacks_but_a_fold_still_does_not() {
        let ctx = bag_ctx();
        // The ordered-only column is now reachable: a key supplies the order a
        // bag lacks, so associativity alone suffices.
        assert!(ty_of(&ctx, "scan `<:` (|r| r.temperature) (|r| r.at) b").is_ok());
        assert!(ty_of(&ctx, "scan `:>` (|r| r.temperature) (|r| r.at) b").is_ok());
        // A fold over an unordered bag still refuses them.
        let errs = ty_of(&ctx, "fold `<:` (|v| v) b.temperature").expect_err("scan only");
        assert!(errs[0].message.contains("not commutative"));
    }

    #[test]
    fn a_scans_order_key_must_be_orderable_and_total() {
        let ctx = bag_ctx();
        // An enum is equatable but not orderable, so it cannot order a scan.
        let errs =
            ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.status) b").expect_err("not ordered");
        assert!(
            errs[0].message.contains("orderable domain"),
            "unexpected: {}",
            errs[0].message
        );
        // A missing key has no position in the order.
        let errs =
            ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.peak) b").expect_err("optional key");
        assert!(
            errs[0].message.contains("no position in the order"),
            "unexpected: {}",
            errs[0].message
        );
        // A dimensioned real is orderable, so it may order a scan.
        assert!(ty_of(&ctx, "scan `+` (|r| r.size) (|r| r.kelvin_reading) b").is_ok());
    }

    #[test]
    fn desc_marks_a_key_and_is_not_a_value() {
        let ctx = bag_ctx();
        // A descending key orders the dual, and the obligation is unchanged:
        // the dual of a total order is total.
        assert!(ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| desc r.at) b").is_ok());
        // `desc` of a non-orderable domain is an error naming the set.
        let errs = ty_of(&ctx, "desc \"x\"").expect_err("not orderable");
        assert!(errs[0].message.contains("orderable domain"));
        // The dual of the dual is the original order, so a double marker is a
        // mistake rather than a silent no-op.
        let errs = ty_of(&ctx, "desc (desc 1)").expect_err("already descending");
        assert!(errs[0].message.contains("already applied"));
        // A descending marker is not a value: it cannot be computed with.
        let errs = ty_of(&ctx, "desc 1 + 2").expect_err("not a value");
        assert!(!errs.is_empty());
    }

    #[test]
    fn a_scan_is_curried_like_a_fold() {
        let ctx = bag_ctx();
        // Short of saturation each step is an ordinary value, which is what
        // lets `series` bind partial applications by name (Decision 8).
        for src in [
            "scan `+`",
            "scan `+` (|r| r.temperature)",
            "scan `+` (|r| r.temperature) (|r| r.at)",
        ] {
            assert!(
                matches!(ty_of(&ctx, src), Ok(Ty::Builtin(_))),
                "`{src}` should still be a function value"
            );
        }
        // Over-application names the arity.
        let errs =
            ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b b.size").expect_err("saturated");
        assert!(errs[0].message.contains("already saturated"));
    }

    /// A const function whose body is a *partially applied primitive* returns a
    /// builtin value rather than another closure, so the remaining arguments
    /// must reach the primitive's slot rules.  `series.lead` is the case that
    /// needs it: its outer lambda yields a 3-slot `prescan`.
    ///
    /// This was a real gap, not a hypothetical: nothing before `series` had a
    /// const function returning a partial builtin, so `apply_closure`'s loop
    /// only ever followed `Ty::Fn` and reported "cannot apply a value of type
    /// `function`" on the next argument.
    #[test]
    fn a_closure_returning_a_partial_builtin_keeps_applying() {
        let ctx = bag_ctx();
        // A local stand-in for `series.lead`'s shape: curried, and the inner
        // body is a partial `prescan` awaiting its key and bag.
        // One argument in, `lead` is still a function (its inner `|key|`
        // lambda); two in, the inner body has yielded a partial `prescan`,
        // which is a *builtin* value.  It is the third argument that used to
        // report "cannot apply a value of type `function`".
        assert!(matches!(
            ty_of(&ctx, "series.lead (|r| r.temperature)"),
            Ok(Ty::Fn(_))
        ));
        assert!(matches!(
            ty_of(&ctx, "series.lead (|r| r.temperature) (|r| r.at)"),
            Ok(Ty::Builtin(_))
        ));
        // Applying through the closure to saturation reaches `type_scan`, so
        // the result is the window's bag.  `lead` is a `prescan` at keep-right,
        // so it is optional: the *last* row under the order has no successor.
        assert_eq!(
            ty_of(&ctx, "series.lead (|r| r.temperature) (|r| r.at) b"),
            Ok(Ty::Bag {
                domain: ColumnType::Real,
                opt: Optionality::Optional
            })
        );
    }

    /// A bag-cardinality fiber whose only grading is `{machine, at}`: the shape
    /// `demote` leaves behind, and the one a real window has.  Completeness is
    /// pinned `Complete` so these fixtures isolate the *tie* axis; the
    /// completeness axis has its own fixtures below (ADR 0037 decision 5).
    fn bag_ctx_graded_by(gradings: &[&[&str]]) -> Context {
        let mut table = sample_table();
        table.qualifiers.cardinality = crate::table::Cardinality::Bag;
        table.qualifiers.completeness = crate::table::Completeness::Complete;
        table.qualifiers.functional = gradings
            .iter()
            .map(|g| g.iter().map(|c| c.to_string()).collect())
            .collect();
        Context::bag(&test_ambient(), "k", "b", &table)
    }

    /// **The Tier 1 obligation.**  A scan's order key must be tie-free, and the
    /// checker discharges that from a grading: `key + {c}` containing one means
    /// two rows of a fiber agreeing on `c` agreed on a whole grading and are the
    /// same row (`Mensura.keyInjOn_demote_tag`).
    #[test]
    fn a_graded_order_key_needs_no_claim() {
        // `machine` is the key, so the grading `{machine, at}` says at most one
        // row per machine shares an `at`: exactly the demoted-key shape.
        let ctx = bag_ctx_graded_by(&[&["machine", "at"]]);
        assert!(ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b").is_ok());
        // `desc` is transparent: the dual of an injective key is injective.
        assert!(ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| desc r.at) b").is_ok());
        // A *different* column is not covered by that grading.
        let errs = ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.size) b")
            .expect_err("size is not graded");
        assert!(
            errs[0].message.contains("may have ties"),
            "unexpected: {}",
            errs[0].message
        );
    }

    #[test]
    fn an_ungraded_or_computed_order_key_is_rejected() {
        let ctx = bag_ctx_graded_by(&[]);
        // Nothing says `at` is unique per machine.
        let errs =
            ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b").expect_err("no grading");
        assert!(errs[0].message.contains("may have ties"));
        // The diagnostic names both fixes.
        assert!(errs[0].message.contains("demote"));
        assert!(errs[0].message.contains("assume { arranged }"));
        // A computed key cannot be analysed at all, so it says so instead.
        let errs = ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.size + 1) b")
            .expect_err("computed key");
        assert!(
            errs[0].message.contains("is computed"),
            "unexpected: {}",
            errs[0].message
        );
    }

    /// The demoted-bag shape with **no completeness fact**: graded, so the tie
    /// demand discharges and what remains is ADR 0037 decision 5's axis.
    fn incomplete_bag_ctx() -> Context {
        let mut table = sample_table();
        table.qualifiers.cardinality = crate::table::Cardinality::Bag;
        table.qualifiers.completeness = crate::table::Completeness::Incomplete;
        table.qualifiers.functional = [["machine", "at"]]
            .iter()
            .map(|g| g.iter().map(|c| c.to_string()).collect())
            .collect();
        Context::bag(&test_ambient(), "k", "b", &table)
    }

    /// **The completeness demand is per combiner row, not per output shape**
    /// (ADR 0037 decision 5, settling ADR 0029's flag).  A fold-admitting scan
    /// contains its reduction (`Mensura.scanl_getLast_eq_foldBag`), so over a
    /// bag with no completeness fact it is rejected exactly as the fold it
    /// contains would be; the keep combiners' outputs are claims about
    /// adjacency among *present* rows, which a partial bag represents
    /// honestly, so they stay ceremony-free at the very same fiber.
    #[test]
    fn a_fold_admitting_scan_demands_completeness() {
        let ctx = incomplete_bag_ctx();
        let errs = ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b")
            .expect_err("scan `+` contains the fold");
        assert!(
            errs[0].message.contains("contains its reduction"),
            "unexpected: {}",
            errs[0].message
        );
        // The diagnostic names both establishments and the placement rule.
        assert!(errs[0].message.contains("assume { complete }"));
        assert!(errs[0].message.contains("demote"));
        // The exclusive form reads the same prefixes.
        assert!(ty_of(&ctx, "prescan `+` (|r| r.temperature) (|r| r.at) b").is_err());
        // The orderable pair is fold-admitting too, identity or not.
        assert!(ty_of(&ctx, "scan `>>` (|r| r.temperature) (|r| r.at) b").is_err());
        // The tacks demand nothing at the very same fiber.
        assert!(ty_of(&ctx, "prescan `:>` (|r| r.temperature) (|r| r.at) b").is_ok());
        assert!(ty_of(&ctx, "scan `<:` (|r| r.temperature) (|r| r.at) b").is_ok());
    }

    /// The two discharges mirror the reducing `map_bags` (ADR 0023): a
    /// completeness fact at the current key, or a `singletons` cardinality
    /// whose present fiber is trivially whole.
    #[test]
    fn the_scan_demand_discharges_like_the_reducers() {
        // `bag_ctx_graded_by` pins `Complete` on a bag cardinality.
        let ctx = bag_ctx_graded_by(&[&["machine", "at"]]);
        assert!(ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b").is_ok());
        // `bag_ctx` is the singletons store: trivially whole fibers.
        assert!(ty_of(&bag_ctx(), "scan `+` (|r| r.temperature) (|r| r.at) b").is_ok());
    }

    /// The bundled `series` bindings ripple with no per-binding exemption
    /// (ADR 0037 decision 5): the scans at `+`/`>>` demand the fact, the
    /// prescan/scan at the tacks do not.
    #[test]
    fn series_bindings_ripple_per_combiner() {
        let ctx = incomplete_bag_ctx();
        for rejected in [
            "series.cumsum (|r| r.temperature) (|r| r.at) b",
            "series.running_max (|r| r.temperature) (|r| r.at) b",
            "series.rank (|r| r.at) b",
        ] {
            let errs = ty_of(&ctx, rejected).expect_err("fold-admitting");
            assert!(
                errs[0].message.contains("contains its reduction"),
                "unexpected for `{rejected}`: {}",
                errs[0].message
            );
        }
        for accepted in [
            "series.lag (|r| r.temperature) (|r| r.at) b",
            "series.lead (|r| r.temperature) (|r| r.at) b",
            "series.first_value (|r| r.temperature) (|r| r.at) b",
        ] {
            assert!(ty_of(&ctx, accepted).is_ok(), "`{accepted}` should pass");
        }
    }

    #[test]
    fn a_module_binding_reports_at_the_call_site() {
        // A `series` binding's body lives in `stdlib/series.mensura`, whose spans
        // do not address the file being checked.  Every diagnostic from inside
        // one is re-anchored at the call site, so no offset can escape the
        // source: unrendered, such a span crashed the language server's line
        // index, and printed a nonsense line in the CLI.
        let mut table = sample_table();
        table.qualifiers.cardinality = crate::table::Cardinality::Bag;
        table.qualifiers.functional = crate::table::Functional::new();
        let ctx = Context::bag(&test_ambient(), "k", "b", &table);
        let src = "series.running_max (|r| r.temperature) (|r| r.at) b";
        let errs = ty_of(&ctx, src).expect_err("no grading covers `at`");
        assert!(errs[0].message.contains("may have ties"));
        for err in &errs {
            assert!(
                err.span.end <= src.len(),
                "span {:?} escapes the source: {}",
                err.span,
                err.message
            );
        }
        // The note still names the binding the demand arrived through.
        assert!(errs.iter().any(|e| {
            e.message
                .contains("while applying `series.running_max` here")
        }));
    }

    #[test]
    fn assume_arranged_discharges_the_tie_obligation() {
        let mut table = sample_table();
        table.qualifiers.cardinality = crate::table::Cardinality::Bag;
        table.qualifiers.functional = crate::table::Functional::new();
        table.qualifiers.arranged = Arranged::Assumed;
        let ctx = Context::bag(&test_ambient(), "k", "b", &table);
        // The claim covers what no grading could, including a computed key: it
        // is an assertion about the data, not about the expression's shape.
        assert!(ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.at) b").is_ok());
        assert!(ty_of(&ctx, "scan `+` (|r| r.temperature) (|r| r.size + 1) b").is_ok());
    }

    #[test]
    fn folding_a_dimensioned_product_is_rejected() {
        // The one row with a domain restriction (Decision 6): a fold's
        // accumulator type must be invariant, but dimensioned `*` *adds*
        // exponent vectors (ADR 0026), so the product's dimension would depend
        // on the bag's cardinality.  `+` preserves dimensions, so `sum` works
        // at every dimension while `prod` does not.
        let ctx = bag_ctx();
        let errs =
            ty_of(&ctx, "fold `*` (|v| v) b.kelvin_reading").expect_err("dimensioned product");
        assert!(
            errs[0].message.contains("dimensionless"),
            "should explain the restriction: {}",
            errs[0].message
        );
        // The same bag folds fine under `+`, which preserves the dimension.
        assert!(ty_of(&ctx, "fold `+` (|v| v) b.kelvin_reading").is_ok());
    }

    #[test]
    fn min_max_and_the_tacks_type_at_one_domain() {
        // ADR 0031, Decision 6.  `<<`/`>>` need an orderable domain and return
        // it; the tacks need only that the operands agree, so `real` works
        // even though it is not equatable (ADR 0014).
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.size << 3"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "r.size >> 3"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "1.5 <: 2.5"), Ok(total(ColumnType::Real)));
        assert_eq!(ty_of(&ctx, "1.5 :> 2.5"), Ok(total(ColumnType::Real)));
        // Mismatched domains have no shared order.
        let errs = ty_of(&ctx, "r.size << r.name").expect_err("int vs string");
        assert!(!errs.is_empty());
    }

    #[test]
    fn to_real_converts_int_value_and_bag() {
        let row = row_ctx();
        assert_eq!(ty_of(&row, "to_real r.size"), Ok(total(ColumnType::Real)));
        let bag = bag_ctx();
        assert_eq!(
            ty_of(&bag, "to_real b.size"),
            Ok(Ty::Bag {
                domain: ColumnType::Real,
                opt: Optionality::Total
            })
        );
        assert!(ty_of(&row, "to_real k.machine").is_err());
    }

    #[test]
    fn presence_is_bool_and_collects_errors() {
        let ctx = row_ctx();
        assert_eq!(
            ty_of(&ctx, "r.note is missing"),
            Ok(total(ColumnType::Bool))
        );
        assert_eq!(
            ty_of(&ctx, "r.temperature is known"),
            Ok(total(ColumnType::Bool))
        );
        let errs = ty_of(&ctx, "r.bogus + r.gone").expect_err("two errors");
        assert_eq!(errs.len(), 2);
    }

    // ADR 0018: in the value layer too, `x |> op` and `op x` are one application,
    // checked identically over the built-in operations
    // (`docs/toolkit/01-application-checking.md`).

    #[test]
    fn application_equals_pipe_for_aggregate() {
        // ADR 0018 holds for a *qualified* head too: the module member is the
        // operation and the piped input is its trailing argument.
        let ctx = bag_ctx();
        assert_eq!(
            ty_of(&ctx, "bag.sum b.temperature"),
            ty_of(&ctx, "b.temperature |> bag.sum"),
        );
        // And for the primitive it is derived from.
        assert_eq!(
            ty_of(&ctx, "fold `+` (|v| v) b.temperature"),
            ty_of(&ctx, "b.temperature |> fold `+` (|v| v)"),
        );
    }

    #[test]
    fn application_equals_pipe_for_to_real() {
        let ctx = row_ctx();
        assert_eq!(
            ty_of(&ctx, "to_real r.size"),
            ty_of(&ctx, "r.size |> to_real")
        );
    }

    #[test]
    fn pipe_to_a_non_builtin_is_rejected() {
        // The value layer does not (yet) admit general application; piping into a
        // non-builtin is an error, mirroring the bare form (ADR 0018 open
        // question 2).
        let ctx = bag_ctx();
        assert!(ty_of(&ctx, "b.temperature |> bogus").is_err());
        assert!(ty_of(&ctx, "bogus b.temperature").is_err());
    }

    // ADR 0026 (`11-physical-units.md`): dimensional arithmetic over the
    // intrinsic base units and dimensioned columns.

    fn quantity(dim: &str) -> ColumnType {
        ColumnType::Quantity(Dimension::base(dim).unwrap())
    }

    #[test]
    fn dimensional_arithmetic_follows_the_group() {
        let ctx = row_ctx();
        // The intrinsics are ambient dimensioned values of magnitude one.
        assert_eq!(ty_of(&ctx, "kelvin"), Ok(total(quantity("temperature"))));
        // Scaling by a bare real keeps the dimension (real is the identity).
        assert_eq!(
            ty_of(&ctx, "350.0 * kelvin"),
            Ok(total(quantity("temperature")))
        );
        // `9.8 * meter / second^2` is an acceleration.
        let accel = Dimension::base("length").unwrap() / Dimension::base("time").unwrap().pow(2);
        assert_eq!(
            ty_of(&ctx, "9.8 * meter / second^2"),
            Ok(total(ColumnType::Quantity(accel)))
        );
        // A same-dimension ratio cancels to bare `real`.
        assert_eq!(ty_of(&ctx, "meter / meter"), Ok(total(ColumnType::Real)));
        // `+` requires equal dimensions, and bare `real` is a different
        // dimension (the identity) from `length`.
        let errs = ty_of(&ctx, "meter + second").expect_err("dimension mismatch");
        assert!(errs[0].message.contains("same type"));
        assert!(ty_of(&ctx, "meter + 1.0").is_err());
        // `int` never mixes with a `real`-backed domain.
        assert!(ty_of(&ctx, "2 * meter").is_err());
        // Negation preserves the dimension.
        assert_eq!(ty_of(&ctx, "-kelvin"), Ok(total(quantity("temperature"))));
    }

    #[test]
    fn dimensioned_pow_takes_an_integer_literal() {
        let ctx = row_ctx();
        // `^ 0` cancels to bare `real`; a negative literal is allowed.
        assert_eq!(ty_of(&ctx, "second ^ 0"), Ok(total(ColumnType::Real)));
        let hz = Dimension::base("time").unwrap().pow(-1);
        assert_eq!(
            ty_of(&ctx, "second ^ -1"),
            Ok(total(ColumnType::Quantity(hz)))
        );
        let errs = ty_of(&ctx, "second ^ 2.0").expect_err("non-literal exponent");
        assert!(errs[0].message.contains("integer literal"));
        // Dimensionless bases keep the status quo: `real ^ int` stays
        // rejected (`11-physical-units.md`).
        assert!(ty_of(&ctx, "3.0 ^ 2").is_err());
        assert_eq!(ty_of(&ctx, "2 ^ 3"), Ok(total(ColumnType::Int)));
    }

    #[test]
    fn dimensions_gate_comparison_and_equality() {
        let ctx = row_ctx();
        // Same dimension: orderable.
        assert_eq!(
            ty_of(&ctx, "r.kelvin_reading > kelvin"),
            Ok(total(ColumnType::Bool))
        );
        // Cross-dimension comparison is a mismatch.
        assert!(ty_of(&ctx, "meter < second").is_err());
        assert!(ty_of(&ctx, "r.kelvin_reading > 30.0").is_err());
        // Equality stays undefined on any real-backed domain, and the
        // diagnostic renders the dimensioned type.
        let errs = ty_of(&ctx, "kelvin == kelvin").expect_err("real-backed equality");
        assert!(errs[0].message.contains("temperature[real]"));
    }

    #[test]
    fn aggregates_preserve_the_dimension() {
        let ctx = bag_ctx();
        assert_eq!(
            ty_of(&ctx, "bag.max b.kelvin_reading"),
            Ok(total(quantity("temperature")))
        );
        assert_eq!(
            ty_of(&ctx, "bag.sum b.kelvin_reading"),
            Ok(total(quantity("temperature")))
        );
        assert_eq!(ty_of(&ctx, "#b.kelvin_reading"), Ok(total(ColumnType::Int)));
        // A mean-style ratio: dimensioned sum over a dimensionless count.
        assert_eq!(
            ty_of(
                &ctx,
                "bag.sum b.kelvin_reading / to_real (#b.kelvin_reading)"
            ),
            Ok(total(quantity("temperature")))
        );
    }

    #[test]
    fn lambda_parameters_shadow_intrinsics() {
        // A pre-existing lambda parameter named like an intrinsic keeps
        // working: parameters bind after the ambient names.
        let ctx = Context::row(&intrinsics(), "k", "meter", &sample_table());
        assert_eq!(
            ty_of(&ctx, "meter.temperature"),
            Ok(total(ColumnType::Real))
        );
    }
}
