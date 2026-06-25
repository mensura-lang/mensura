# The must-accept / must-reject corpus

This directory is the language's classification gate (`ROADMAP.md` M0).  It is
driven by `tests/corpus.rs`, which runs the same frontend as `mensura check`
(lex, then parse, then resolve) over every case:

- `accept/` holds programs that must lex, parse, **and** resolve cleanly.
- `reject/` holds programs that must fail at one of those stages.

The runner only checks the accept/reject classification, not the specific
diagnostic; tests that pin a particular message live next to the code that
emits it (in `mensura-syntax` and `mensura-types`).

## Adding a case

Drop a `.mensura` file into `accept/` or `reject/`.  Start it with a one-line
comment saying why it belongs there, and for a rejection name the stage that
rejects it (parse or resolve) and on what grounds.  The suite grows one
language feature at a time, in step with the milestones.
