# What's next

This book grows with the language.  Today it covers modelling data (units,
scalar types, stores, registries, and shapes), transforming it (views,
`flat_map`, the reshape pair), windowing it over time (the grid, finality, and
completing the grid), and checking a program and creating and materializing its
tables in a database.  The features still being built are documented here as
they land.

Two pieces are implemented but not yet written up here, and their design
documents are the place to read them meanwhile: **physical units**, where an
attribute carries a dimension and a unit mismatch is a compile error
(`docs/language/11-physical-units.md`), and **typed ingestion**, which feeds
observations into stores and registries through `mensura ingest`
(`docs/toolkit/05-ingestion.md`).

On the way, roughly in order:

- **Incremental refresh.**  A closed window's result is final, which is what
  makes it safe to *maintain* a view rather than recompute it: `on_change`
  refresh through the processing layer, and with it an honest way to report the
  window that is still filling instead of withholding it or misreporting it.
- **Lineage and leak-free validation.**  The property that motivates the whole
  language: the type system proving that a training set and a test set share no
  entities, so a split cannot leak.
- **Model signatures and validation strategies.**  `fit`, `predict`, and
  `evaluate` as typed operations, with k-fold, stratified, temporal, and grouped
  strategies each carrying its own disjointness proof.
- **Serving.**  Running a program as a service, with endpoints generated from
  the stores, registries, and views it already declares.
- **Run and deploy configurations.**  Targeting backends other than the
  bundled SQLite without changing a program's source.

The phased plan, with the driving application (a streaming
predictive-maintenance service over a fleet of devices), is in `ROADMAP.md`.
The language design documents under `docs/language/` and the toolkit documents
under `docs/toolkit/` specify each piece before it is built.
