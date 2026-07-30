//! End-to-end materialization over compound stores (ADR 0032): create the
//! stores (dotted columns, composite primary keys, unenforced FOREIGN KEY
//! clauses), seed them, and materialize views that read nested key groups
//! and forward rows containing a unit-reference group.  Seeding happens at
//! the SQL level through the backend; that is a test scaffold until M4's
//! typed ingestion exists.

use mensura_runtime::{SqliteBackend, StorageBackend, Value, materialize_views};
use mensura_types::ResolvedProgram;

const COLLEGE: &str = r#"
    unit Person { id: string }
    unit Department { code: string }
    unit Course {
      department: Department
      name: string
      year: int
    }
    unit Enrollment {
      student: Person
      course: Course
    }
    unit Program { code: string }

    store students {
      unit { Person }
      attr { admission: date }
    }

    store departments {
      unit { Department }
    }

    store courses {
      unit { Course }
      domain { department: departments }
      attr { credits: int }
    }

    store student_grades {
      unit { Enrollment }
      domain {
        student: students
        course:  courses
      }
      attr { grade: real }
    }

    store programs {
      unit { Program }
      domain { coordinator: students }
      attr {
        name:        string
        coordinator: Person
      }
    }

    // Nested key access (ADR 0032): the flattened column
    // `course.department.code` reads as `k.course.department.code`.
    view cs_grades {
      student_grades
        |> flat_map |k, r| if k.course.department.code == "cs" then r else ()
    }

    // Whole-row forwarding over a unit-reference *attribute* group: `r`
    // contains the `coordinator` group, which flattens back to its dotted
    // column in the output; the filter reads through the group.
    view coordinated_by_p1 {
      programs
        |> flat_map |_, r| if r.coordinator.id == "p1" then r else ()
    }
"#;

fn college_program() -> ResolvedProgram {
    let tokens = mensura_syntax::tokenize(COLLEGE).expect("should lex");
    let program = mensura_syntax::parse(&tokens).expect("should parse");
    mensura_types::resolve(&program).expect("should resolve")
}

fn seeded_db(program: &ResolvedProgram) -> SqliteBackend {
    let mut db = SqliteBackend::open_in_memory().unwrap();
    for schema in &program.schemas {
        db.ensure_store(schema).unwrap();
    }
    // Positional seeding follows the flattened column order: the dotted key
    // columns first, in the unit's declaration order, then the attributes.
    db.execute_sql(
        r#"INSERT INTO "students" VALUES ('p1', '2024-08-01'), ('p2', '2025-08-01');
           INSERT INTO "departments" VALUES ('cs'), ('math');
           INSERT INTO "courses" VALUES
             ('cs',   'algorithms', 2026, 6),
             ('math', 'calculus',   2026, 9);
           INSERT INTO "student_grades" VALUES
             ('p1', 'cs',   'algorithms', 2026, 9.5),
             ('p1', 'math', 'calculus',   2026, 7.0),
             ('p2', 'cs',   'algorithms', 2026, 8.0);
           INSERT INTO "programs" VALUES
             ('bsc-cs',   'computer science', 'p1'),
             ('bsc-math', 'mathematics',      'p2');"#,
    )
    .unwrap();
    db
}

#[test]
fn nested_key_access_filters_the_compound_store() {
    let program = college_program();
    let mut db = seeded_db(&program);

    let materialized = materialize_views(&mut db, &program).unwrap();
    assert_eq!(
        materialized,
        vec![
            ("cs_grades".to_string(), 2),
            ("coordinated_by_p1".to_string(), 1),
        ]
    );

    let view = program
        .views
        .iter()
        .find(|v| v.name == "cs_grades")
        .expect("the program declares cs_grades");
    let rows = db.scan(&view.shape()).unwrap();
    // The composite key survives whole: both `cs` enrollments, no `math`.
    assert_eq!(
        rows,
        vec![
            vec![
                Value::String("p1".into()),
                Value::String("cs".into()),
                Value::String("algorithms".into()),
                Value::Int(2026),
                Value::Real(9.5),
            ],
            vec![
                Value::String("p2".into()),
                Value::String("cs".into()),
                Value::String("algorithms".into()),
                Value::Int(2026),
                Value::Real(8.0),
            ],
        ]
    );
}

#[test]
fn forwarded_group_flattens_back_to_dotted_columns() {
    let program = college_program();
    let mut db = seeded_db(&program);
    materialize_views(&mut db, &program).unwrap();

    let view = program
        .views
        .iter()
        .find(|v| v.name == "coordinated_by_p1")
        .expect("the program declares coordinated_by_p1");
    // The whole-row body yields the flattened attributes sorted:
    // `coordinator.id` before `name` (`.` orders below identifier
    // characters).
    let cols: Vec<&str> = view.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cols, vec!["code", "coordinator.id", "name"]);
    let rows = db.scan(&view.shape()).unwrap();
    assert_eq!(
        rows,
        vec![vec![
            Value::String("bsc-cs".into()),
            Value::String("p1".into()),
            Value::String("computer science".into()),
        ]]
    );
}
