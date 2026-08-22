# The kernel operations

> The spelling is preliminary, but these operations are implemented: the
> snippet below is compiled by the book's check gate.  The ideas, and the
> theorems behind them, are settled.  The full specification is in
> `docs/language/07-pipelines.md`.

A pipeline transforms one table into another, but it is not a separate kind of
thing in Mensura.  A pipeline is an ordinary expression of table type, built
from one small set of **table-valued operations**.  There is no special
pipeline grammar: stages are composed left to right, and each stage is one of a
handful of primitives.  This page names them, one line each, so the chapters
that follow can use them without stopping to explain.

## Composing operations

Three pieces of glue thread operations together:

- **`|>`**, the pipe: `data |> op` applies `op` to `data`, so a pipeline reads
  top to bottom as a sequence of stages.
- **`let`**, to name an intermediate table and reuse it (forking a pipeline is
  binding a table once and using it twice).
- **tuples**, to bring several tables together for an operation that merges
  them: `(train, test) |> union`.

```mensura
unit Reading { ts: int }
store readings { unit { Reading } attr { kelvin:real machine:string } }

view celsius {
  readings
  |> promote machine
  |> flat_map |k, r| (.celsius = r.kelvin - 273.15)
}
```

## The primitives

Each is a pure function from a table to a table.

| operation | what it does |
| --- | --- |
| `flat_map` | per-row transform returning a row multiset: rewrite, drop, or expand each row (so filtering is `flat_map` with `if c then r else ()`) |
| `map_bags` | per-key transform over the whole bag (an aggregate, or a window) |
| `promote` | move a non-key column *into* the key (refine the key) |
| `demote` | move a column *out* of the key (coarsen the key) |
| `lookup` / `lookup_total` | join the table against a fixed lookup table |
| `split` | partition a table by a predicate over the key, into two halves |
| `union` | merge two tables of the same schema into one |
| `unpivot` | reshape wide to long: turn value columns into rows |
| `pivot` | reshape long to wide: gather rows into one wide row per key |

A few relationships are worth seeing now, because the next chapter turns on
them:

- `promote` and `demote` are true inverses: one makes the key finer,
  the other coarser, and promoting a column then demoting it (in either
  order) is the identity.
- `split` and `union` are partner operations: `split` cuts a table into two
  halves that share no key, and `union` is what puts two tables back together.
- `unpivot` and `pivot` are inverses: long form and wide form of the same data.

That is the whole kernel.  Everything else is sugar or specialization over
these: the named forms `filter`, `mutate`, `select`, and `aggregate` are still
to come, while the window functions and the streaming operations have landed.
`window` is the clearest case of a specialization rather than a new idea: it is
a replicating `flat_map` followed by a `promote`, which is why the facts it
carries are the facts those two carry, and it has a chapter of its own in
[Windows and time](../windows/over-time.md).  With the operations named, the
next chapter can show what their *types* track and why that catches mistakes
other tools cannot.
