#!/usr/bin/env bash
# Checks the dev container has everything the builds and tests use (run it
# after rebuilding the container). Prints what's missing; exit code 1 if any.
missing=0
need() { if "$@" >/dev/null 2>&1; then printf '  ok       %s\n' "$*"; else printf '  MISSING  %s\n' "$*"; missing=1; fi; }
echo "Commands:"
for c in cargo rustup wasm-bindgen wasm-opt cargo-tauri cargo-about twiggy brotli zstd \
         x86_64-w64-mingw32-gcc makensis wine chromium Xvfb xdotool convert import python3 pkg-config; do
    need command -v $c
done
echo "Versions that must match:"
need sh -c 'wasm-bindgen --version | grep -q " 0.2.128"'
need sh -c 'cargo tauri --version | grep -q "^tauri-cli 2\."'
echo "Rust targets:"
for t in wasm32-unknown-unknown x86_64-pc-windows-gnu; do need sh -c "rustup target list --installed | grep -qx $t"; done
echo "Libraries (Tauri's web view):"
for l in webkit2gtk-4.1 javascriptcoregtk-4.1 libsoup-3.0 gtk+-3.0; do need pkg-config --exists $l; done
echo "Mistake-spotter model (for packages; not in the repo):"
need sh -c '[ -f checkpoints/models/B7.tgz ] || [ -f "${XDG_DATA_HOME:-$HOME/.local/share}/xenify/spotter/config.json" ]'
[ $missing = 0 ] && echo "All there." || echo "Something's missing: see .devcontainer/Dockerfile."
exit $missing
