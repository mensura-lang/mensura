//! Schema fingerprints for the schema guard (ADR 0045 decision 9).
//!
//! A fingerprint is a SHA-256 digest over a canonical text rendering of
//! what the storage layer depends on: the table's name, kind, cardinality,
//! its columns with type, role, and optionality, its `domain` references,
//! and its `lateness` contracts.  Views are excluded, because they are
//! derived and rebuilt by the next materialization.  The rendering is
//! hand-written rather than a serialization of the Rust types, so a field
//! added to the model for the checker's benefit does not silently change
//! every deployment's fingerprint.

use std::collections::BTreeMap;

use mensura_types::{Cardinality, ColumnRole, ColumnType, ResolvedProgram, Schema};
use sha2::{Digest, Sha256};

/// The canonical rendering of one store or registry.
pub fn canonical(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(&format!("table {}\n", schema.store));
    out.push_str(&format!("kind {}\n", schema.kind.keyword()));
    out.push_str(&format!(
        "cardinality {}\n",
        match schema.cardinality {
            Cardinality::Singletons => "singletons",
            Cardinality::Bag => "bag",
        }
    ));
    out.push_str(&format!("unit {}\n", schema.unit));
    for c in &schema.columns {
        out.push_str(&format!(
            "column {} {} {} {}\n",
            c.name,
            render_type(&c.ty),
            match c.role {
                ColumnRole::Key => "key",
                ColumnRole::Attr => "attr",
            },
            if c.optional { "optional" } else { "total" }
        ));
    }
    for fk in &schema.foreign_keys {
        let pairs: Vec<String> = fk
            .columns
            .iter()
            .map(|(child, parent)| format!("{child}={parent}"))
            .collect();
        out.push_str(&format!(
            "domain {} {} -> {} ({})\n",
            fk.field,
            fk.unit,
            fk.store,
            pairs.join(",")
        ));
    }
    for l in &schema.lateness {
        out.push_str(&format!(
            "lateness {} {} grain={}\n",
            l.column,
            l.bound,
            l.grain.join(",")
        ));
    }
    out
}

/// The stable text of a column type: every domain the storage mapping
/// distinguishes, with a quantity's dimension and an enum's variants.
fn render_type(ty: &ColumnType) -> String {
    match ty {
        ColumnType::String => "string".to_string(),
        ColumnType::Int => "int".to_string(),
        ColumnType::Real => "real".to_string(),
        ColumnType::Quantity(d) => format!("{d}[real]"),
        ColumnType::Bool => "bool".to_string(),
        ColumnType::Date => "date".to_string(),
        ColumnType::Instant => "instant".to_string(),
        ColumnType::Enum { name, variants } => format!("enum {name} {{{}}}", variants.join(",")),
    }
}

/// The fingerprint of one store or registry.
pub fn table_fingerprint(schema: &Schema) -> String {
    hex(&Sha256::digest(canonical(schema).as_bytes()))
}

/// Every stored table of the program with its fingerprint, in the
/// program's declaration order.
pub fn program_tables(program: &ResolvedProgram) -> Vec<(String, String)> {
    program
        .schemas
        .iter()
        .map(|s| (s.store.clone(), table_fingerprint(s)))
        .collect()
}

/// The fingerprint of a whole program: a digest over its tables'
/// fingerprints in name order, so declaration order does not matter.
pub fn program_fingerprint(tables: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = tables.iter().collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    for (name, fp) in sorted {
        hasher.update(name.as_bytes());
        hasher.update(b" ");
        hasher.update(fp.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// Every view's definition digest, by name.  The schema guard leaves views
/// out because they are derived, but a materialized view is only as
/// current as the definition that produced it: when the digest recorded
/// beside a view's rows differs from the program's, the rows are replaced
/// whatever their position (`SqliteBackend::materialize_view`), so a
/// redeployed program never serves an old definition's rows.
///
/// A view's rows are determined by its resolved plan and by its sources,
/// so the digest covers the plan and chains through each source's digest:
/// a store's table fingerprint, or an upstream view's definition digest.
/// Editing a view therefore invalidates every view that reads it.  The
/// plan is rendered with `Debug`, which includes source spans, so moving
/// a view's text also invalidates it; that is conservative, since a
/// spurious invalidation costs one replacement and never a stale read.
pub fn view_definitions(program: &ResolvedProgram) -> BTreeMap<String, String> {
    let mut digests: BTreeMap<String, String> = BTreeMap::new();
    // Views come in dependency order (ADR 0042), so every upstream view's
    // digest is known when a view needs it.
    for plan in &program.views {
        let mut hasher = Sha256::new();
        hasher.update(format!("{plan:?}").as_bytes());
        for source in &plan.sources {
            let upstream = digests.get(source).cloned().or_else(|| {
                program
                    .schemas
                    .iter()
                    .find(|s| &s.store == source)
                    .map(table_fingerprint)
            });
            hasher.update(b"\nsource ");
            hasher.update(source.as_bytes());
            hasher.update(b" ");
            hasher.update(upstream.unwrap_or_default().as_bytes());
        }
        digests.insert(plan.name.clone(), hex(&hasher.finalize()));
    }
    digests
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(src: &str) -> ResolvedProgram {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let parsed = mensura_syntax::parse(&tokens).expect("should parse");
        mensura_types::resolve(&parsed).expect("should resolve")
    }

    const FLEET: &str = r#"
        import si
        unit Machine { machine_id: string }
        enum Status { "ok" "bad" }
        store machines {
          unit { Machine }
          attr { status: Status  last_service: date? }
        }
        unit Reading { machine_id: string  taken_at: instant }
        registry readings {
          unit { Reading }
          attr { temperature: temperature[real] }
          lateness { taken_at: 10.0 * si.minute }
        }
        view hot { readings |> flat_map |_, r| if r.temperature > 350.0 * kelvin then r else () }
    "#;

    #[test]
    fn the_rendering_names_what_storage_depends_on() {
        let p = program(FLEET);
        let readings = p.schemas.iter().find(|s| s.store == "readings").unwrap();
        let text = canonical(readings);
        assert!(text.contains("kind registry\n"), "{text}");
        assert!(
            text.contains("column taken_at instant key total\n"),
            "{text}"
        );
        assert!(
            text.contains("column temperature temperature[real] attr total\n"),
            "{text}"
        );
        assert!(
            text.contains("lateness taken_at 600000 grain=machine_id\n"),
            "{text}"
        );
        let machines = p.schemas.iter().find(|s| s.store == "machines").unwrap();
        let text = canonical(machines);
        assert!(
            text.contains("column status enum Status {ok,bad} attr total\n"),
            "{text}"
        );
        assert!(
            text.contains("column last_service date attr optional\n"),
            "{text}"
        );
    }

    #[test]
    fn a_view_change_leaves_the_fingerprint_alone_and_a_column_change_does_not() {
        let base = program_fingerprint(&program_tables(&program(FLEET)));
        let view_edited = FLEET.replace("350.0", "340.0");
        assert_eq!(
            base,
            program_fingerprint(&program_tables(&program(&view_edited)))
        );
        let column_edited = FLEET.replace("last_service: date?", "last_service: date");
        assert_ne!(
            base,
            program_fingerprint(&program_tables(&program(&column_edited)))
        );
        let bound_edited = FLEET.replace("10.0 * si.minute", "15.0 * si.minute");
        assert_ne!(
            base,
            program_fingerprint(&program_tables(&program(&bound_edited)))
        );
    }

    #[test]
    fn a_view_definition_change_reaches_the_views_that_read_it() {
        let src = format!("{FLEET}\nview hot_again {{ hot }}\n");
        let base = view_definitions(&program(&src));
        assert_eq!(base, view_definitions(&program(&src)), "deterministic");
        let edited = view_definitions(&program(&src.replace("350.0", "340.0")));
        assert_ne!(base["hot"], edited["hot"]);
        assert_ne!(base["hot_again"], edited["hot_again"], "chained");
    }

    #[test]
    fn declaration_order_does_not_matter() {
        let a = vec![
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
        ];
        let b = vec![
            ("y".to_string(), "2".to_string()),
            ("x".to_string(), "1".to_string()),
        ];
        assert_eq!(program_fingerprint(&a), program_fingerprint(&b));
    }
}
