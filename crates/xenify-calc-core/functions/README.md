# The function dictionary

One JSON file per group. Each holds a list of entries, one per function.
`all.tsv` lists every function the engine knows and the file its entry
goes in. Calc shows these while a formula is typed (names, arguments)
and in Help › Functions (everything, with the example drawn as a little
sheet). `src/functions.rs` loads them; its tests check that every function
has an entry and that every example gives the result written down:

    cargo test -p xenify-calc-core functions
    XENIFY_FUNCTIONS=text cargo test -p xenify-calc-core every_example_works

(`XENIFY_FUNCTIONS` takes file names without `.json`, comma-separated:
`maths`, `statistics-2`…)

## An entry

```json
{
  "name": "SUMIF",
  "args": [
    { "name": "range", "about": "the cells to test" },
    { "name": "criteria", "about": "the test, like \"Apple\" or \">10\"" },
    { "name": "sum_range", "about": "the cells to add up (the tested cells if left out)", "optional": true }
  ],
  "about": "Adds up the numbers in rows that pass a test.",
  "example": {
    "cells": [["Fruit", "Sold"], ["Apple", "3"], ["Pear", "5"], ["Apple", "4"]],
    "formula": "=SUMIF(A2:A4,\"Apple\",B2:B4)",
    "result": "7",
    "why": "Only the Apple rows count: 3 + 4."
  },
  "notes": "SUMIFS takes more than one test."
}
```

- `name`: exactly as in `all.tsv`.
- `args`: in order, with the usual spreadsheet names (`number1`, `lookup_value`,
  `start_date`…). `about` says what to give, in a few lowercase words.
  Optional arguments have `"optional": true`. When the last one can be
  given again and again (`number2`, `value2`…) add `"more": true` to the
  entry.
- `about`: one sentence, plain words, what it gives. No jargon where a
  plain word will do ("adds up", "the middle value", "how many").
- `example.cells`: example data from A1, as rows of strings typed into
  cells (numbers as "12.5", dates as "2026-01-31", text as is). Small and
  realistic (fruit, months, prices, people), with a header row when it helps.
  It may be empty for functions that need no data (`=PI()`).
- `example.formula`: uses the function, over those cells. The formula goes
  in the cell two columns right of the data, on row 1.
- `example.result`: exactly what the cell shows, as text (Calc's default
  number format, about ten significant figures: `350`, `0.5`, `3.141592654`, `TRUE`, `#N/A`). Use
  `null` only for results that change every time (NOW, TODAY, RAND…).
  Functions that give several cells (FILTER, SORT, SEQUENCE…) show their
  first one here; say what spills in `why`.
- `example.why`: one sentence on how the result comes about.
- `notes` (optional): when it gives an error, which function to use
  instead, or how it differs from a similar one.

Older names (`STDEVP`, `CONCATENATE`…) say which newer function to use in
`notes`.

Write British English, like the rest of Xenify.
