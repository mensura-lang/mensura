# Decision index

One line per ADR.  The ADR is authoritative; this is a map.  The status
column also names what amended or superseded a decision, so a reader
knows which ADR to open instead.  ADR 0025 is the translation reference
for names used before it (`index` to `key`, `map` to `flat_map`,
`group_map` to `map_bags`, `extend_key`/`shrink_key` to
`promote`/`demote`).  Adding an ADR adds a row; amending one updates
both rows.

| ADR | Decision | Status |
|---|---|---|
| 0001 | A `unit` is an identity criterion, nothing else | Accepted |
| 0002 | A `store` tabulates one unit; many stores per unit | Accepted; blocks per 0019 |
| 0003 | A `shape` is a structural contract claimed with `:` | Accepted; blocks per 0019 |
| 0004 | Table type `Table<Qs, C>`; qualifiers propagate facts | Accepted; boundary per 0013 |
| 0005 | Mensura verifies federated SPIFFE-style identities | Proposed; subset in 0045 |
| 0006 | Programs name surfaces, never wires | Proposed; HTTP in 0045, deploy config in 0046 |
| 0007 | One expression sublanguage at every site | Accepted |
| 0008 | The algebra is mechanized in Lean 4 under `formal/` | Accepted |
| 0009 | Pipelines are curried expressions over a small kernel | Accepted; names per 0025 |
| 0010 | Attributes total by default; `T?` admits absence | Accepted |
| 0011 | Comments on a lexer side channel | Proposed |
| 0012 | A `view` hosts a pipeline; its type is computed | Accepted |
| 0013 | `C` is structure, `Qs` is every propagated fact | Accepted |
| 0014 | `int`/`real` split; domain properties gate operations | Accepted |
| 0015 | `flat_map` returns a row multiset; key-first lambdas | Accepted; names per 0025 |
| 0016 | `unpivot`/`pivot` surface syntax | Accepted; amended by 0020 |
| 0017 | `completeness_check` and `assume { complete }` | Accepted; consumer per 0023 |
| 0018 | `x \|> g` means `g x`; juxtaposition is application | Accepted |
| 0019 | `attr` blocks; `const`/`var` dropped | Accepted |
| 0020 | `unpivot`/`pivot` as a true inverse pair | Accepted |
| 0021 | CI-gated, blueprint-driven Lean proofs | Accepted |
| 0022 | Stores declare `singletons` or `bag` (`attr*`) | Accepted |
| 0023 | The reducing `map_bags` consumes completeness | Accepted; amended by 0035 |
| 0024 | `promote`/`demote` inverse; key gradings | Accepted |
| 0025 | Nomenclature sweep; the translation reference | Accepted |
| 0026 | Dimensions as `D[real]` over the seven SI bases | Accepted |
| 0027 | Top-level `let`, `import`, bundled modules | Accepted |
| 0028 | The bundled `si` standard library | Accepted |
| 0029 | Aggregates as a fold and scan family; two proof stages | Accepted, design-only; revised by 0031 |
| 0030 | Lambdas as const values; explicit currying | Accepted |
| 0031 | `fold`/`scan` primitives; `bag` and `series` modules | Accepted |
| 0032 | Compound keys flatten to dotted columns | Accepted |
| 0033 | `registry`: append-only, complete by mechanism | Accepted; amended by 0035 |
| 0034 | Typed ingestion API and `mensura ingest` | Accepted |
| 0035 | A coarsening `demote` clears completeness | Accepted |
| 0036 | `instant`, civil versus absolute, torsor arithmetic | Accepted |
| 0037 | `window`, `lateness`, `closed` | Accepted; amended by 0041, 0044 |
| 0038 | `dense` completes the window grid | Accepted |
| 0039 | Operators lift over `T?`; `??` discharges late | Accepted |
| 0040 | Partial operator precedence | Accepted |
| 0041 | Per-grain watermark and the closure floor | Accepted |
| 0042 | Views read views, in dependency order | Accepted |
| 0043 | Record spread `...r` | Accepted |
| 0044 | `last p` names the point reduction | Accepted |
| 0045 | `mensura serve`: roles, write log, HTTP projection | Accepted |
| 0046 | `mensura deploy`: the deploy configuration and the compose bundle | Accepted |
