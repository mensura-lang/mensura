# What's next

This book grows with the language.  Today it covers modelling data: units,
scalar types, stores, and shapes, plus checking a program and creating its
stores in a database.  The features that compute over stored data are being
built and will be documented here as they land.

On the way, roughly in order:

- **Expressions and pipelines.**  Transforming stored data with a typed
  algebra, where each operation carries rules for how it changes a table's
  content and qualifiers.  The type checker for this already exists: value
  expressions, the Tier A pipeline operations (`map`, `group_map`, `extend_key`,
  the joins, `split`/`bind`), and `view` declarations all type-check today
  (`mensura check`); a dedicated chapter and the runtime that materializes a
  view follow.
- **Lineage and leak-free validation.**  The property that motivates the whole
  language: the type system proving that a training set and a test set share no
  entities, so a split cannot leak.
- **Physical units and precision.**  Dimensioned attribute types (temperature,
  vibration) with unit mismatches as compile errors.
- **Ingestion and serving.**  Declaring devices that feed observations into
  stores, and running a program as a service.
- **Run and deploy configurations.**  Targeting backends other than the
  bundled SQLite without changing a program's source.

The phased plan, with the driving application (a streaming
predictive-maintenance service over a fleet of devices), is in `ROADMAP.md`.
The language design documents under `docs/language/` and the toolkit documents
under `docs/toolkit/` specify each piece before it is built.
