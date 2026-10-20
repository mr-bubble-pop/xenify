# Xenify Write (HTML)

The word processor with HTML screens: the same documents, layout, spelling
and grammar as the egui Xenify Write, with the look agreed in
`docs/design/write-html/`.

## Run it

```sh
cargo run --release -p xenify-write-app -- [files…]
```

Use `--release`: the mistake spotter (an offline model) is very slow in a
debug build. The window draws its own title area (`ui/window.js`); its
corners are rounded when the desktop has a compositor.

## How it fits together

| Crate | What it does |
|---|---|
| `xenify-wp-engine` | One open document: editing, layout, keys, typing, the mouse, page scenes (display lists) and glyph outlines. No toolkit; builds for WebAssembly. |
| `xenify-write-host` | The open documents and every call the screens make (`Docs::call(name, args)`), settings, the spelling checker. Desktop-only parts (files, printing, crash recovery) sit behind `cfg`. |
| `xenify-write-app` | The desktop app (Tauri), installed as `xenify-write`. `ui/` holds the screens (plain HTML, CSS and JavaScript); `src/` the commands, file dialogs, the AI assistant and the command line (`cli.rs`: convert, check, profiles, merge). |
| `xenify-write-web` | The browser build's document engine: the host as WebAssembly behind the same calls, without the checker; `web/` holds the shim that stands in for Tauri, the offline copy (`offline.js`, `sw.js`) and the checker's worker. |
| `xenify-write-check-web` | The browser build's spelling and grammar checker, Look up and mistake spotter, run in a Web Worker. |

The screens draw each page on a `<canvas>` from the engine's display lists
(glyph outlines are fetched once and cached as `Path2D`), so a page looks the
same on screen, in the PDF and in print. A hidden text box carries the
caret's paragraph for input methods and screen readers.

## Tests

```sh
cargo test -p xenify-wp-engine -p xenify-write-host
```

## In the browser

```sh
scripts/build-write-web.sh     # → target/write-web (needs wasm-bindgen-cli 0.2.128, binaryen, brotli)
scripts/serve-write-web.py     # http://localhost:8080/
```

Not in the browser yet: AI help, printing (export a PDF instead). See
docs/write-packaging.md for what loads when, offline use and hosting.

## Packages

```sh
scripts/package-write.sh [linux|windows|macos]   # → dist-write/
```

Linux: .deb, .rpm, .AppImage, .tar.gz. Windows: a setup .exe (cross-built
here with MinGW). macOS: a .dmg, built on a Mac. Each has the mistake-spotter
model inside. See docs/write-packaging.md.
