//! Record spread elaboration (`docs/decisions/0043-record-spread.md`).
//!
//! A record body's items are labeled fields and spreads `...e`.  This module
//! is the one place that decides which fields the body ends up with, so the
//! checker and the evaluator cannot disagree on a body's columns: each
//! supplies the top-level fields of every spread operand (typed or
//! evaluated), and [`elaborate`] applies the override rule of ADR 0043
//! decision 3.  Column order carries no meaning (decision 4), so the fields
//! come out in canonical order, by name, whatever order they were written
//! in.

use mensura_syntax::{Expr, ExprKind, RecordField, RecordItem, Span};

/// One field of an elaborated record body.
#[derive(Debug)]
pub enum Elaborated<'a, T> {
    /// An explicit field, whether or not it overrides a spread field.
    Field(&'a RecordField),
    /// A top-level field contributed by a spread, carrying whatever the
    /// caller attached to it (a type, a value, a column list).
    Spread { name: String, value: T },
}

/// A name collision the override rule does not resolve.
#[derive(Clone, Debug, PartialEq)]
pub enum Clash {
    /// Two explicit fields share `name`; `span` is the later one's name.
    Field { name: String, span: Span },
    /// Two spreads both contribute `name`; `span` is the later spread.
    Spread { name: String, span: Span },
}

impl Clash {
    /// The diagnostic wording, shared by the checker and the evaluator.
    pub fn message(&self) -> String {
        match self {
            Clash::Field { name, .. } => format!("field `{name}` is set twice in this record"),
            Clash::Spread { name, .. } => format!(
                "field `{name}` comes from two spreads; the spread records must \
                 have disjoint fields"
            ),
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Clash::Field { span, .. } | Clash::Spread { span, .. } => *span,
        }
    }
}

/// Elaborate a record body.  `spreads` holds, for each spread item in item
/// order, its operand's top-level fields.
///
/// An explicit field overrides a spread field of the same name; two
/// explicit fields, or two spreads, sharing a name are a [`Clash`], even
/// when an explicit field overrides the shared name.  The fields come out
/// sorted by name.  Flattening a unit-reference group afterwards keeps the
/// dotted columns sorted too, because `.` orders below every identifier
/// character, so the flat columns are in canonical order (ADR 0043
/// decision 4).
pub fn elaborate<'a, T>(
    items: &'a [RecordItem],
    spreads: Vec<Vec<(String, T)>>,
) -> Result<Vec<Elaborated<'a, T>>, Vec<Clash>> {
    let mut clashes = Vec::new();
    let mut explicit: Vec<&RecordField> = Vec::new();
    for item in items {
        if let RecordItem::Field(f) = item {
            if explicit.iter().any(|g| g.name.name == f.name.name) {
                clashes.push(Clash::Field {
                    name: f.name.name.clone(),
                    span: f.name.span,
                });
            } else {
                explicit.push(f);
            }
        }
    }
    let spread_spans = items.iter().filter_map(|item| match item {
        RecordItem::Spread { span, .. } => Some(*span),
        RecordItem::Field(_) => None,
    });
    let mut spread_names: Vec<String> = Vec::new();
    for (fields, span) in spreads.iter().zip(spread_spans) {
        let mut own = Vec::new();
        for (name, _) in fields {
            if spread_names.contains(name) {
                clashes.push(Clash::Spread {
                    name: name.clone(),
                    span,
                });
            }
            own.push(name.clone());
        }
        spread_names.extend(own);
    }
    if !clashes.is_empty() {
        return Err(clashes);
    }

    let mut out: Vec<Elaborated<'a, T>> = explicit.iter().copied().map(Elaborated::Field).collect();
    for (name, value) in spreads.into_iter().flatten() {
        if !explicit.iter().any(|f| f.name.name == name) {
            out.push(Elaborated::Spread { name, value });
        }
    }
    out.sort_by(|a, b| a.name().cmp(b.name()));
    Ok(out)
}

impl<T> Elaborated<'_, T> {
    /// The field's name, explicit or spread.
    pub fn name(&self) -> &str {
        match self {
            Elaborated::Field(f) => &f.name.name,
            Elaborated::Spread { name, .. } => name,
        }
    }
}

/// The operand of a spread as a path: a bare name and the member steps
/// after it (`r` is `("r", [])`, `r.course` is `("r", ["course"])`), or
/// `None` when the operand is not a path.  ADR 0043 decision 1 admits paths
/// only, which is what lets the evaluator know a spread's columns without a
/// row to evaluate it on.
pub fn spread_path(expr: &Expr) -> Option<(&str, Vec<&str>)> {
    match &expr.kind {
        ExprKind::Name(n) => Some((n, Vec::new())),
        ExprKind::Member(base, field) => {
            let (root, mut steps) = spread_path(base)?;
            steps.push(&field.name);
            Some((root, steps))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mensura_syntax::{parse_expr, tokenize};

    fn items(src: &str) -> Vec<RecordItem> {
        match parse_expr(&tokenize(src).unwrap()).expect("parses").kind {
            ExprKind::Record(items) => items,
            other => panic!("not a record: {other:?}"),
        }
    }

    fn names<T>(out: &[Elaborated<'_, T>]) -> Vec<String> {
        out.iter()
            .map(|e| match e {
                Elaborated::Field(f) => format!("={}", f.name.name),
                Elaborated::Spread { name, .. } => format!("...{name}"),
            })
            .collect()
    }

    fn fields(ns: &[&str]) -> Vec<(String, ())> {
        ns.iter().map(|n| (n.to_string(), ())).collect()
    }

    #[test]
    fn the_fields_come_out_by_name() {
        let it = items("(.d = 1, ...r, .a = 2)");
        let out = elaborate(&it, vec![fields(&["c", "b"])]).unwrap();
        assert_eq!(names(&out), ["=a", "...b", "...c", "=d"]);
    }

    #[test]
    fn an_override_is_the_same_record_wherever_it_is_written() {
        for src in ["(.b = 1, ...r)", "(...r, .b = 1)"] {
            let it = items(src);
            let out = elaborate(&it, vec![fields(&["a", "b", "c"])]).unwrap();
            assert_eq!(names(&out), ["...a", "=b", "...c"], "{src}");
        }
    }

    #[test]
    fn two_explicit_fields_clash() {
        let it = items("(.a = 1, .a = 2)");
        let errs = elaborate::<()>(&it, vec![]).unwrap_err();
        assert!(matches!(&errs[..], [Clash::Field { name, .. }] if name == "a"));
    }

    #[test]
    fn two_spreads_clash_even_when_overridden() {
        let it = items("(...r, ...s, .a = 1)");
        let errs = elaborate(&it, vec![fields(&["a"]), fields(&["a", "b"])]).unwrap_err();
        assert!(matches!(&errs[..], [Clash::Spread { name, .. }] if name == "a"));
    }

    #[test]
    fn a_spread_operand_is_a_path() {
        let e = parse_expr(&tokenize("r.course.dept").unwrap()).unwrap();
        assert_eq!(spread_path(&e), Some(("r", vec!["course", "dept"])));
        let app = parse_expr(&tokenize("f r").unwrap()).unwrap();
        assert_eq!(spread_path(&app), None);
    }
}
