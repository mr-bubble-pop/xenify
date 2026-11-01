# Files and protocols across versions

Each Xenify release has to open files, settings and server messages from
older versions, and has to behave sensibly with ones from newer versions:
it either opens them or says "made with a newer version, update".
It must never report them as damaged, crash, or quietly throw away what
it doesn't understand. This page sets the rules, and the table records
where each format stands.

## Rules for every change

1. **New fields are optional.** Give each one `#[serde(default)]` (or make it
   an `Option`), so older files still load. Never rename or remove a field
   that has shipped. Add a new field and keep reading the old one.
2. **Unknown fields are ignored.** No `deny_unknown_fields` on anything that
   is saved or sent, including config files. A newer version may have
   written them, and a rollback must still start.
3. **New enum values need a plan.** serde fails the *whole* file or message
   on one unknown variant, so pick one of these:
   - Small value enums (a kind, a role, a level): read unknown names as a
     safe fallback (see `lenient!` in xenify-chat-proto, `Security` in
     mail-core). Use the fallback that does the least harm, such as TLS,
     "member" or "notify all".
   - Lists where an item can be skipped (grammar features): drop the items
     this version doesn't know (`known_features` in xenify-grammar).
   - Document content (layers, blocks, sheet kinds, ops): bump the format
     level, below.
4. **Documents carry two numbers.**
   - `format_version` (Calc: `version`) is the lowest reader level that can
     open the file correctly. The writer works it out from what the file
     uses, so a plain file stays readable by older apps.
   - `saved_with` is the writer's own level.

   A reader refuses a file whose `format_version` is above its own level,
   and checks this on the raw JSON *before* the typed read. If the typed
   read then fails and `saved_with` is above its own level, it says
   "made with a newer version, update" rather than "damaged".
   When you add a document feature that older apps would misread, raise
   the app's level, and write that level only for files that use the
   feature.
5. **A local file that won't read is kept, not overwritten.** Starting
   empty is fine, but first keep the unreadable copy aside (chat-core keeps
   it as `<what>-unreadable-<time>`), or stop saving over it.
6. **SQLite stores keep their level in `PRAGMA user_version`.** Each step is
   additive (new tables, or columns with a DEFAULT) and keyed on that level.
   A store from a newer app keeps its higher number.
7. **Protocols are additive.** New message types and fields need no bump:
   each side drops messages it can't read. Bump the protocol number only
   when older peers would truly misbehave. In that case the side that
   notices says *who* has to update ("update the app" or "update the
   server"), rather than giving a generic error.
8. **Co-editing changes** that the server accepted but this app can't read
   say "someone is working on this with a newer version, update"
   (`decode` in the draw, calc and plan `remote.rs`).

## Where each format stands

| What | Level today | Notes |
|---|---|---|
| Write `.xwrite` (zip: `mimetype` plus JSON) | `format_version` 1 | Checked before the typed read; `saved_with` written |
| Draw `.xdraw` (zip) | 3 (1 when there are no images, 2 when there is no video or audio) | `saved_with` written; a newer file says "update" |
| Calc `.xcalc` | `version` 2 (1 when there are no linked CSV sheets) | Checked before the typed read; `saved_with` written. The Excel extras (`xenify/calc.json`) are read part by part |
| Plan `.xplan` | `version` 1 | Checked before the typed read; `saved_with` written. History ops (`ops::Op`) count as format: a new kind needs a higher level |
| Chat protocol | `xenify_chat_proto::VERSION` 4 | Kind, Role, AppMode, Presence and Notify are lenient |
| Server REST | `/v1/`, `xenify_server_proto::VERSION` 2, `OLDEST` 2 | `/v1/info` has `protocol` and `features`; apps check with `works_with()` (a range, not equality) |
| Hub release list (signed) | `min_hub` (optional) | Products and releases this Hub can't read are left out; `xenify-release sign --min-hub` only for a list older Hubs would misread |
| Hub settings and server policy | – | An unknown channel or mode reads as the default (policy: the person's choice) |
| Trusted servers (JSON) | – | Entries read one by one; unknown ones kept on save; an unreadable file is kept aside |
| MCP (Calc, Plan) | `2025-06-18`, also `2025-03-26` and `2024-11-05` | Answers with the program's version when it speaks it, else its newest |
| Server database (Postgres) | `SCHEMA_LEVEL` 1, kept as setting "schema_level" | An older server on a newer database warns and skips its own schema steps |
| Server config.toml | – | Unknown keys are warned about, not refused |
| Server AI settings | – | Unknown runner reads as OpenAI-style; never saved over when unreadable |
| Backups | snapshot keeps `version` and `schema` | Restore refuses a snapshot from a newer server |
| Encryption at rest (`params` beside the folder) | `kdf` "argon2id-13" | A newer kdf says "update the server" |
| Mail store (SQLite) | `user_version` 1 | |
| Mail accounts (JSON, app and server) | – | Every field has a default; an unknown `security` reads as TLS |
| Chat calendar and contacts (JSON) | – | Unreadable copies are kept aside |
| Grammar profiles (TOML) | – | Unknown features and categories are skipped |
