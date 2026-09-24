//! Record spread elaboration (`docs/decisions/0043-record-spread.md`).
//!
//! A record body's items are labeled fields and spreads `...e`.  This module
//! is the one place that decides which fields the body ends up with and in
//! which order, so the checker and the evaluator cannot disagree on column
//! order: each supplies the ordered top-level fields of every spread operand
//! (typed or evaluated), and [`elaborate`] applies the override and ordering
//! rules of ADR 0043 decisions 3 and 4.

use mensura_syntax::{Expr, ExprKind, RecordField, RecordItem, Span};

/// One field of an elaborated record body, in output order.
#[derive(Debug)]
pub enum Elaborated<'a, T> {
    /// An explicit field, at its own position or, when it overrides a
    /// spread field, at that spread field's position.
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
/// order, its operand's top-level fields in the operand's own order.
///
/// An explicit field overrides a spread field of the same name and takes
/// its position; one that overrides nothing stays where it is written; two
/// explicit fields, or two spreads, sharing a name are a [`Clash`], even
/// when an explicit field overrides the shared name.
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

    let mut out = Vec::new();
    let mut spreads = spreads.into_iter();
    for item in items {
        match item {
            RecordItem::Field(f) => {
                if !spread_names.contains(&f.name.name) {
                    out.push(Elaborated::Field(f));
                }
            }
            RecordItem::Spread { .. } => {
                let fields = spreads.next().unwrap_or_default();
                for (name, value) in fields {
                    match explicit.iter().find(|f| f.name.name == name) {
                        Some(f) => out.push(Elaborated::Field(f)),
                        None => out.push(Elaborated::Spread { name, value }),
                    }
                }
            }
        }
    }
    Ok(out)
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
    fn a_spread_expands_in_place() {
        let it = items("(.c = 1, ...r, .d = 2)");
        let out = elaborate(&it, vec![fields(&["a", "b"])]).unwrap();
        assert_eq!(names(&out), ["=c", "...a", "...b", "=d"]);
    }

    #[test]
    fn an_override_takes_the_spread_position_wherever_it_is_written() {
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
