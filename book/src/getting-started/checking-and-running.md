# Checking and running

The toolchain has two subcommands for working with a program: `check` proves it
is well typed, and `run` does that and then creates its stores.

## `mensura check`

`check` runs the whole front end (lex, parse, name resolution) and reports any
errors, but touches no database.  It is the fast feedback loop and what you
wire into CI.

```console
$ mensura check machines.mensura
```

A well-typed program prints nothing and exits zero.  When something is wrong,
`check` reports every problem it found rather than stopping at the first.
Suppose the store names a unit that was never declared:

```mensura,ignore
store machines {
  unit { Machne }   // typo: there is no unit by this name
  const { commissioned: date }
}
```

`check` rejects it, pointing at the unresolved name.  Because resolution
collects all diagnostics in one pass, fixing the first error does not hide the
others behind it.

## `mensura run`

`run` first checks the program, then creates each store in a database.  A store
becomes a table whose primary key is its unit's index fields.

```console
$ mensura run machines.mensura --db machines.db
```

The `--db` flag chooses the SQLite file to create the stores in.  Omit it and
the database is held in memory, which is handy for a quick check that a program
not only types but also maps cleanly to storage:

```console
$ mensura run machines.mensura
```

Running never reaches the database if checking fails, so `run` is always safe
to use: a program that would not pass `check` creates nothing.

With a program written, checked, and run, the rest of the book looks more
closely at the pieces you have been using.  We start with [units and
indices](../modelling/units.md).
