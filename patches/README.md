# Dependency stand-ins

Small MIT crates that replace dependencies we don't use but that would
otherwise ship under a copyleft licence. They're wired in with `[patch.crates-io]`
in the workspace `Cargo.toml`.

| Crate | Replaces | Why it's safe |
|---|---|---|
| `ammonia` 4.2.0 | HTML sanitiser (MIT/Apache, but pulls in MPL-2.0 `cssparser` and `dtoa-short`) | Harper calls `ammonia::clean` only to turn lint messages and rule descriptions into HTML. Xenify Write uses the plain-text messages. The stand-in escapes all markup, so it stays safe if anything ever calls it. |
| `colored` 3.1.1 | Terminal colours (MPL-2.0) | `burn-tensor` uses `.bold()`/`.red()`/`.green()`/`.yellow()` only in a debug report it prints. The stand-in returns the text uncoloured. |
| `option-ext` 0.2.0 | `Option` helper methods (MPL-2.0) | `dirs-sys` (through `dirs`, used for the data and settings folders) calls only `OptionExt::contains` when reading the XDG user-dirs file. The stand-in has the same three methods, written for Xenify.|

If an upgrade needs more of either API, the build fails and the stand-in is extended here.

## Build-time only: Tauri's MPL-2.0 crates (no stand-in needed)

The Xenify desktop app (`crates/xenify-chat-app`) uses Tauri. Tauri's build
script and code-generating macros (`tauri-build`, `tauri-codegen`,
`tauri-macros`, through `tauri-utils` → `dom_query`) use the MPL-2.0 crates
`cssparser`, `selectors` and `dtoa-short` to adjust the HTML pages while the
app is compiled. They run on the build machine only; none of their code is
in the app that is shipped, so there is nothing MPL to distribute. Decided
with the project owner on 2026-10-18. Check with
`cargo tree -p xenify-chat-app -e normal -i cssparser` that every path still
goes through a build-dependency or a proc-macro.

cargo-about counts proc-macro dependencies as shipped, so `about.toml` allows
these four crates by name (only them, only MPL-2.0). Check it still holds
with `cargo tree -p xenify-chat-app -e normal,no-proc-macro -i cssparser`,
which must find nothing.

## IronCalc with tables that grow (`ironcalc_base` 0.9.1)

Not a stand-in: a copy of IronCalc's engine (MIT OR Apache-2.0, unchanged
licence files and third-party notices kept) with these changes for Xenify
Calc, each marked `Xenify` in the source:

- Structured references (`Sales[Revenue]`, `Sales[[#Totals],[Revenue]]`) are
  kept in formulas as a `TableRefKind` node: stored, shown and saved to xlsx
  as written, with the range they stand for underneath. Upstream turns them
  into fixed ranges when the formula is typed, so a totals row stops at the
  rows that existed then.
- The parser is given the current tables whenever formulas are parsed again
  (`Model::refresh_tables` after changing `workbook.tables`).
- Inserting or deleting rows and columns grows, shrinks or moves the tables
  on that sheet (`src/xenify_tables.rs`); renaming or deleting a sheet updates
  or removes its tables.
- The lexer reads structured references in R1C1 mode too (how formulas
  are stored), except after names that start an R1C1 reference (R, C3, R1C2).
- `Model::rename_table` and `Model::rename_table_column` rename a table or one
  of its columns in every formula.
- `Model::displace_formula` gives a formula as it reads once rows or columns
  move (Calc's editing together adjusts formulas typed in another window).
- `build.rs` no longer asks git for the version.
- Numbers are exact decimals (`src/num.rs`, `Num`: `rust_decimal`, 28
  significant digits, MIT) instead of `f64`, in cells, formula results,
  arrays and formula literals, so 16 to 28 digit ids and money keep every
  digit and 0.1 + 0.2 is 0.3. Typed, pasted and read numbers are exact, and
  so are + − × ÷, comparisons (`=`, MATCH, VLOOKUP, COUNTIF, SUMIF…), SUM,
  AVERAGE, MIN/MAX, LARGE/SMALL, the ROUND family, TRUNC, INT, ABS, MOD,
  FLOOR/CEILING, SUBTOTAL and the *IFS. Everything else (SQRT, LN, finance,
  statistics…) works in `f64` as upstream does: its answers stay floating
  point, and anything worked out from one does too. A cell shows an exact
  number's digits in full under "General" (`format_cell_number`); TEXT keeps
  Excel's General. Function code reads a number with `.f()` and hands one
  back with `Num::from(f64)`. A `Num` is 16 bytes, so cells don't grow.
  Recalculating is about 1.4 times slower than with `f64`.

IronCalc's own tests pass (`cargo test` in this folder); `test_tables.rs` was
changed to expect the kept names, and a few tests to expect exact answers
(2.3e-13 for "23e-12%"). When upgrading, copy the new release here and carry
these changes over (search for `Xenify`).

## IronCalc's xlsx reading and writing (`ironcalc` 0.9.1)

A copy of IronCalc's file crate (MIT OR Apache-2.0, licence files kept) for
the exact numbers above, changes marked `Xenify`:

- `<v>` numbers are read and written with `Num::from_xlsx_str` and
  `Num::to_xlsx_string`: Excel writes its floating point numbers with 16 or
  17 significant digits (-2.2999999999999998), so those stay floating point
  (and compare as in Excel); every other number is exact. Exact fractions
  that would have 16 or 17 digits get trailing zeros when written, so they
  read back exact.
- `compare.rs` (tests) compares numbers as `f64`.

Its tests need its dev-dependencies, so they don't build in the workspace:
run them from a small workspace with this folder and `patches/ironcalc_base`
as members and `[patch.crates-io] ironcalc_base = { path = ... }`. 7 of
IronCalc's xlsx test files fail with or without these changes.
