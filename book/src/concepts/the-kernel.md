# The kernel operations

> These operations are a design preview: they are specified but not yet
> implemented, so the snippets here are not checked by the compiler.  The
> [What's next](../whats-next.md) page tracks the frontier.  The spelling is
> preliminary; the ideas, and the theorems behind them, are settled.  The full
> specification is in `docs/language/07-pipelines.md`.

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
  them: `(train, test) |> bind`.

```mensura,ignore
readings
|> map |r| (.celsius = r.kelvin - 273.15)
|> extend_key machine
```

## The primitives

Each is a pure function from a table to a table.

| operation | what it does |
| --- | --- |
| `map` | per-row transform: rewrite each row independently |
| `group_map` | per-key transform over a whole group (an aggregate, or a window) |
| `extend_key` | move a non-index column *into* the key (refine the index) |
| `shrink_key` | move a column *out* of the key (coarsen the index) |
| `left_join` / `inner_join` | join the table against a fixed lookup table |
| `split` | partition a table by a predicate over the key, into two halves |
| `bind` | merge two tables of the same schema into one |
| `unpivot` | reshape wide to long: turn value columns into rows |
| `pivot` | reshape long to wide: gather rows into one wide row per key |

A few relationships are worth seeing now, because the next chapter turns on
them:

- `extend_key` and `shrink_key` are inverses in direction: one makes the key
  finer, the other coarser.
- `split` and `bind` are partner operations: `split` cuts a table into two
  halves that share no key, and `bind` is what puts two tables back together.
- `unpivot` and `pivot` are inverses: long form and wide form of the same data.

That is the whole kernel.  Everything else (the named forms `filter`, `mutate`,
`select`, `aggregate`, window functions, and the streaming operations) is sugar
or specialization over these, and arrives later.  With the operations named, the
next chapter can show what their *types* track and why that catches mistakes
other tools cannot.
