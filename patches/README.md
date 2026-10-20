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
