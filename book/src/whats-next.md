# What's next

This book grows with the language.  Today it covers modelling data (units,
scalar types, stores, and shapes) and transforming it (views, `flat_map`, and
the reshape pair), plus checking a program and creating and materializing its
tables in a database.  The features still being built are documented here as
they land.

On the way, roughly in order:

- **Lineage and leak-free validation.**  The property that motivates the whole
  language: the type system proving that a training set and a test set share no
  entities, so a split cannot leak.
- **Physical units.**  Dimensioned attribute types (temperature,
  vibration) with unit mismatches as compile errors.
- **Ingestion and serving.**  Typed ingestion feeding observations into
  stores and registries, and running a program as a service.
- **Run and deploy configurations.**  Targeting backends other than the
  bundled SQLite without changing a program's source.

The phased plan, with the driving application (a streaming
predictive-maintenance service over a fleet of devices), is in `ROADMAP.md`.
The language design documents under `docs/language/` and the toolkit documents
under `docs/toolkit/` specify each piece before it is built.
