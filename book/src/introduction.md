# Introduction

Mensura is a statically typed language for **data handling**.  Its type system
encodes properties of the *data*, not just the shape of values: how rows were
sampled, how they depend on one another, where they came from, and what their
columns mean.  Because those properties live in the type, the compiler rejects
programs that are syntactically valid but semantically wrong, the mistakes
other tools leave to runtime, convention, or discipline:

- mixing training and test data (leakage),
- using the wrong cross-validation strategy on time-ordered data,
- drawing a biased sample,
- comparing quantities in incompatible units.

The motto is *measure twice, run once*.

## Who this book is for

This book teaches Mensura to people who want to *use* it.  It assumes you have
written data pipelines before (in pandas, the tidyverse, Polars, or SQL) and
are comfortable reading types.  It does not assume you have built a compiler or
read the language's design documents.  If you want the formal specification,
the grammar, and the design decisions, those live under `docs/` in the
repository; this book cross-links to them but stands on its own.

## What you can do today

Mensura is early.  This book documents only what the toolchain actually
compiles, and every example in it is checked by the real compiler when the
book is built: if an example here stops compiling, the build fails.  So
nothing in these pages is aspirational.

Today that means **modelling data**: declaring the *units* that identify your
entities and the *stores* that hold their attributes, and creating those stores
in a database.  The chapters that compute over stored data (expressions,
pipelines, the leak-free validation that motivates the whole language) are
being built and will arrive here as they land.  The [What's next](whats-next.md)
page tracks the frontier.

## How to read it

Work through *Getting started* in order: install the toolchain, write and run a
first program, then learn what `mensura check` and `mensura run` do.  *Modelling
data* then takes the three building blocks (units, stores, shapes) one at a
time.
