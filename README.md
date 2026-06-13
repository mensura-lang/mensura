# Mensura

A statically typed language for data handling.

*Measure twice, run once.*

## What it is

Mensura is a programming language in which the type system encodes properties
of the *data*, not just the shape of values.  A Mensura table type records how
its rows were sampled, how they depend on one another, where they came from,
and what their columns mean (their schema, units, and semantic types).

Because those properties live in the type, the compiler can reject programs
that are syntactically valid but semantically wrong.  Mistakes that other
tools leave to runtime, convention, or discipline (mixing training and test
data, using the wrong cross-validation strategy on time-ordered data, drawing
a biased sample, comparing quantities in incompatible units) become compile
errors instead.

The language itself is small.  The novelty is in the typing rules attached to
each operation, not in the surface syntax.

## Goals

- Turn data-handling correctness into a compile-time property.
- Prevent whole classes of bugs before a program runs: data leakage, the
  wrong cross-validation strategy on temporal data, biased sampling, and unit
  or semantic mismatches.
- Stay a small, focused language whose power comes from its type system rather
  than from a large surface area.

## Learn more

See `docs/language/00-overview.md` for what the language is and `ROADMAP.md`
for the phased plan.

## License

Licensed under either of MIT (`LICENSE-MIT`) or Apache License 2.0
(`LICENSE-APACHE`), at your option.
