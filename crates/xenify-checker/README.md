# xenify-checker

A small background process that loads the
[mistake-spotter model](../xenify-grammar-spotter) once and shares it between
Xenify apps (Xenify Write, the picture editor) and all their windows, instead
of each window holding its own copy (~170 MB).

- **Starts itself.** The first app that needs the model starts
  `xenify-checker` (found next to the app, or on the `PATH`).
- **Exits itself.** Each app keeps one connection open while it uses the
  model. When no app has been connected for 60 seconds, the checker exits and
  the memory is freed.
- **Private.** It listens on a Unix socket in `$XDG_RUNTIME_DIR/xenify/` (or a
  private folder in the temp folder), which only the user can open.
- **Falls back.** If the checker can't run (no Unix sockets, program not
  installed), the app loads the model itself as before.

The protocol is one JSON line per request and answer (`Request`, `Response`).

```rust
let client = xenify_checker::Client::connect()?;          // starts it if needed
client.load(&model_dir)?;                                 // kinds of mistake known
let spots = client.spot(&model_dir, "I have pet dog.", 0.5, 3)?;
```

`xenify-checker [--grace SECONDS]` runs it by hand.

## Licence

MIT.
