//! The typed ingestion decoder (`docs/toolkit/05-ingestion.md`, ADR 0034).
//!
//! A **record** is a name-keyed map of field name to scalar value.  Decoding
//! checks it against a resolved [`Schema`] and produces a [`Row`] in the
//! table's column order.  The decoder is deliberately independent of any
//! wire format: it knows how a record maps onto a schema, not how the record
//! arrived.  JSON Lines is the encoding `mensura ingest` reads from a local
//! file (see [`decode_jsonl`]), and the transports of M7 become further
//! callers rather than rewrites
//! (`docs/decisions/0006-transport-agnostic-surface.md`).

use std::collections::BTreeMap;
use std::fmt;

use mensura_types::{ColumnType, Schema};

use crate::value::{Row, Value};

/// One scalar as it arrives, before it is checked against a column type.
///
/// This is the format-neutral vocabulary every adapter targets: a JSON
/// number, an MQTT payload field, and a GraphQL argument all reduce to one
/// of these, so the type rules below are written once.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Text(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    /// An explicit null: the field was present and empty.  Accepted only
    /// where the column is optional, exactly as an absent field is.
    Null,
}

impl Scalar {
    fn what(&self) -> &'static str {
        match self {
            Scalar::Text(_) => "a string",
            Scalar::Int(_) => "an integer",
            Scalar::Real(_) => "a number",
            Scalar::Bool(_) => "a boolean",
            Scalar::Null => "null",
        }
    }
}

/// A name-keyed record: what one row looks like before it is typed.
pub type Record = BTreeMap<String, Scalar>;

/// Why a record could not be decoded.  Carries the record's position in its
/// batch (1-based) so a report names the offending line rather than the
/// failing statement.
#[derive(Clone, Debug, PartialEq)]
pub struct IngestError {
    /// 1-based position of the record within the batch.
    pub record: usize,
    pub message: String,
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "record {}: {}", self.record, self.message)
    }
}

impl std::error::Error for IngestError {}

/// Decode one record against `schema`, producing a row in column order.
///
/// `at` is the record's 1-based position, used only for diagnostics.
pub fn decode_record(schema: &Schema, record: &Record, at: usize) -> Result<Row, IngestError> {
    let err = |message: String| IngestError {
        record: at,
        message,
    };

    // The column set is closed, so an unrecognized field is an error rather
    // than something to drop: a typo'd field name is exactly the class of
    // mistake silence would hide (ADR 0034 decision 3).
    for name in record.keys() {
        if !schema.columns.iter().any(|c| &c.name == name) {
            let known: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
            return Err(err(format!(
                "unknown field `{name}` for `{}`; its columns are {}",
                schema.store,
                known.join(", ")
            )));
        }
    }

    let mut row = Vec::with_capacity(schema.columns.len());
    for col in &schema.columns {
        let value = match record.get(&col.name) {
            None | Some(Scalar::Null) => {
                if col.optional {
                    Value::Missing
                } else {
                    let why = if record.contains_key(&col.name) {
                        "is null"
                    } else {
                        "is missing"
                    };
                    return Err(err(format!(
                        "field `{}` {why}, but the column is not optional",
                        col.name
                    )));
                }
            }
            Some(scalar) => decode_scalar(scalar, &col.ty).map_err(|expected| {
                err(format!(
                    "field `{}`: expected {expected}, found {}",
                    col.name,
                    scalar.what()
                ))
            })?,
        };
        row.push(value);
    }
    Ok(row)
}

/// Decode a whole batch, stopping at the first bad record.  The batch is
/// applied as one transaction (ADR 0034 decision 4), so a partial decode is
/// never useful.
pub fn decode_records(schema: &Schema, records: &[Record]) -> Result<Vec<Row>, IngestError> {
    records
        .iter()
        .enumerate()
        .map(|(i, r)| decode_record(schema, r, i + 1))
        .collect()
}

/// Check one scalar against a column type.  On mismatch, returns what the
/// column wanted, for the caller to phrase.
fn decode_scalar(scalar: &Scalar, ty: &ColumnType) -> Result<Value, &'static str> {
    match ty {
        ColumnType::String => match scalar {
            Scalar::Text(s) => Ok(Value::String(s.clone())),
            _ => Err("a string"),
        },
        ColumnType::Int => match scalar {
            Scalar::Int(n) => Ok(Value::Int(*n)),
            _ => Err("an integer"),
        },
        // `int` does not widen to `real`: the domains stay apart at the
        // boundary exactly as they do in expressions (ADR 0014).  A JSON
        // `300` for a `real` column is a mistake worth naming.
        ColumnType::Real | ColumnType::Quantity(_) => match scalar {
            Scalar::Real(x) => Ok(Value::Real(*x)),
            _ => Err("a real number (with a decimal point)"),
        },
        ColumnType::Bool => match scalar {
            Scalar::Bool(b) => Ok(Value::Bool(*b)),
            _ => Err("a boolean"),
        },
        ColumnType::Date => match scalar {
            Scalar::Text(s) => Ok(Value::Date(s.clone())),
            _ => Err("a date string"),
        },
        // Pass-through for now; ADR 0036 decision 7's validation and UTC
        // normalization land in the next slice of this PR.
        ColumnType::Instant => match scalar {
            Scalar::Text(s) => Ok(Value::Instant(s.clone())),
            _ => Err("an RFC 3339 instant string"),
        },
        ColumnType::Enum { variants, .. } => match scalar {
            Scalar::Text(s) if variants.iter().any(|v| v == s) => Ok(Value::Enum(s.clone())),
            Scalar::Text(_) => Err("one of the enum's declared variants"),
            _ => Err("a string naming an enum variant"),
        },
    }
}

/// Read a JSON Lines batch into records: one JSON object per line, blank
/// lines skipped (ADR 0034 decision 4).
///
/// Line-oriented so a large batch streams and a malformed record is
/// localized; the schema is already the contract, so no schema negotiation
/// is needed.  This is an adapter over [`decode_record`], not part of it.
pub fn read_jsonl(src: &str) -> Result<Vec<Record>, IngestError> {
    let mut out = Vec::new();
    // Count every line so a diagnostic points at the file, not at the index
    // among non-blank lines.
    for (i, line) in src.lines().enumerate() {
        let at = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let err = |message: String| IngestError {
            record: at,
            message,
        };
        let parsed: serde_json::Value =
            serde_json::from_str(line).map_err(|e| err(format!("invalid JSON ({e})")))?;
        let serde_json::Value::Object(fields) = parsed else {
            return Err(err("expected a JSON object".to_string()));
        };
        let mut record = Record::new();
        for (name, v) in fields {
            record.insert(
                name.clone(),
                json_scalar(&v).ok_or_else(|| {
                    err(format!(
                        "field `{name}`: expected a scalar, found an array or object"
                    ))
                })?,
            );
        }
        out.push(record);
    }
    Ok(out)
}

/// Decode a JSON Lines batch against `schema`, in one step.
pub fn decode_jsonl(schema: &Schema, src: &str) -> Result<Vec<Row>, IngestError> {
    let records = read_jsonl(src)?;
    decode_records(schema, &records)
}

fn json_scalar(v: &serde_json::Value) -> Option<Scalar> {
    Some(match v {
        serde_json::Value::Null => Scalar::Null,
        serde_json::Value::Bool(b) => Scalar::Bool(*b),
        // A JSON number is `int` only when it is written without a fraction
        // or exponent, which keeps ADR 0014's split visible in the payload.
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) if !n.to_string().contains(['.', 'e', 'E']) => Scalar::Int(i),
            _ => Scalar::Real(n.as_f64()?),
        },
        serde_json::Value::String(s) => Scalar::Text(s.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(src: &str, table: &str) -> Schema {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let program = mensura_syntax::parse(&tokens).expect("should parse");
        mensura_types::resolve(&program)
            .expect("should resolve")
            .schemas
            .into_iter()
            .find(|s| s.store == table)
            .unwrap_or_else(|| panic!("no table named {table}"))
    }

    const EVERY_TYPE: &str = r#"
        unit K { id: string }
        enum Status { "ok" "bad" }
        registry rows {
          unit { K }
          attr {
            count:  int
            ratio:  real
            flag:   bool
            day:    date
            status: Status
            note:   string?
          }
        }
    "#;

    fn full_record() -> Record {
        Record::from([
            ("id".into(), Scalar::Text("k-1".into())),
            ("count".into(), Scalar::Int(3)),
            ("ratio".into(), Scalar::Real(0.5)),
            ("flag".into(), Scalar::Bool(true)),
            ("day".into(), Scalar::Text("2026-07-31".into())),
            ("status".into(), Scalar::Text("ok".into())),
            ("note".into(), Scalar::Text("hi".into())),
        ])
    }

    #[test]
    fn decodes_every_column_type_in_column_order() {
        let s = schema(EVERY_TYPE, "rows");
        let row = decode_record(&s, &full_record(), 1).expect("decodes");
        assert_eq!(
            row,
            vec![
                Value::String("k-1".into()),
                Value::Int(3),
                Value::Real(0.5),
                Value::Bool(true),
                Value::Date("2026-07-31".into()),
                Value::Enum("ok".into()),
                Value::String("hi".into()),
            ]
        );
    }

    #[test]
    fn an_absent_optional_is_missing_and_an_explicit_null_is_too() {
        let s = schema(EVERY_TYPE, "rows");
        let mut absent = full_record();
        absent.remove("note");
        assert_eq!(
            decode_record(&s, &absent, 1).unwrap().last(),
            Some(&Value::Missing)
        );

        let mut null = full_record();
        null.insert("note".into(), Scalar::Null);
        assert_eq!(
            decode_record(&s, &null, 1).unwrap().last(),
            Some(&Value::Missing)
        );
    }

    #[test]
    fn a_missing_required_field_is_an_error() {
        let s = schema(EVERY_TYPE, "rows");
        let mut r = full_record();
        r.remove("count");
        let e = decode_record(&s, &r, 7).expect_err("required");
        assert_eq!(e.record, 7);
        assert!(e.message.contains("`count`"), "{e}");
        assert!(e.message.contains("not optional"), "{e}");
    }

    #[test]
    fn an_unknown_field_is_an_error() {
        // The column set is closed: a typo must not be silently dropped.
        let s = schema(EVERY_TYPE, "rows");
        let mut r = full_record();
        r.insert("kelvon".into(), Scalar::Real(300.0));
        let e = decode_record(&s, &r, 1).expect_err("unknown");
        assert!(e.message.contains("unknown field `kelvon`"), "{e}");
    }

    #[test]
    fn a_bad_enum_variant_is_an_error() {
        let s = schema(EVERY_TYPE, "rows");
        let mut r = full_record();
        r.insert("status".into(), Scalar::Text("sideways".into()));
        let e = decode_record(&s, &r, 1).expect_err("bad variant");
        assert!(e.message.contains("declared variants"), "{e}");
    }

    #[test]
    fn an_int_does_not_widen_to_a_real() {
        // ADR 0014 keeps the domains apart, at the boundary as in
        // expressions, so `300` for a `real` column is named as a mistake.
        let s = schema(EVERY_TYPE, "rows");
        let mut r = full_record();
        r.insert("ratio".into(), Scalar::Int(1));
        let e = decode_record(&s, &r, 1).expect_err("no widening");
        assert!(e.message.contains("`ratio`"), "{e}");
        assert!(e.message.contains("real number"), "{e}");
    }

    #[test]
    fn a_dotted_compound_column_is_addressed_by_its_path() {
        // ADR 0032: a flattened unit-reference field is one column named by
        // its access path, and the payload uses that same name.
        let src = r#"
            unit Machine { serial: string }
            unit Event { machine: Machine  at: date }
            store machines { unit { Machine } attr { commissioned: date } }
            registry events {
              unit { Event }
              domain { machine: machines }
              attr { note: string }
            }
        "#;
        let s = schema(src, "events");
        let r = Record::from([
            ("machine.serial".into(), Scalar::Text("m-01".into())),
            ("at".into(), Scalar::Text("2026-07-31".into())),
            ("note".into(), Scalar::Text("swap".into())),
        ]);
        assert_eq!(
            decode_record(&s, &r, 1).expect("decodes"),
            vec![
                Value::String("m-01".into()),
                Value::Date("2026-07-31".into()),
                Value::String("swap".into()),
            ]
        );
    }

    #[test]
    fn jsonl_reads_a_batch_and_skips_blank_lines() {
        let s = schema(EVERY_TYPE, "rows");
        let src = "\
{\"id\":\"k-1\",\"count\":1,\"ratio\":0.5,\"flag\":true,\"day\":\"2026-07-31\",\"status\":\"ok\",\"note\":null}

{\"id\":\"k-2\",\"count\":2,\"ratio\":1.5,\"flag\":false,\"day\":\"2026-08-01\",\"status\":\"bad\",\"note\":\"x\"}
";
        let rows = decode_jsonl(&s, src).expect("decodes");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], Value::String("k-1".into()));
        assert_eq!(rows[0][6], Value::Missing);
        assert_eq!(rows[1][5], Value::Enum("bad".into()));
    }

    #[test]
    fn jsonl_reports_the_source_line_of_a_bad_record() {
        let s = schema(EVERY_TYPE, "rows");
        let src = "\
{\"id\":\"k-1\",\"count\":1,\"ratio\":0.5,\"flag\":true,\"day\":\"2026-07-31\",\"status\":\"ok\"}
not json
";
        let e = decode_jsonl(&s, src).expect_err("bad line");
        assert_eq!(e.record, 2, "{e}");
        assert!(e.message.contains("invalid JSON"), "{e}");
    }

    #[test]
    fn jsonl_rejects_a_nested_value() {
        let s = schema(EVERY_TYPE, "rows");
        let e = decode_jsonl(&s, "{\"id\": {\"nested\": 1}}\n").expect_err("nested");
        assert!(e.message.contains("expected a scalar"), "{e}");
    }
}
