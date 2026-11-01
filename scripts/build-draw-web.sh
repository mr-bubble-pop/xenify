#!/usr/bin/env bash
# Builds Xenify Draw's engine as WebAssembly and puts it where the screens
# load it:
#   crates/xenify-draw-app/ui/pkg   the desktop app's copy (Tauri bundles ui/)
#   target/draw-web                 the browser version: the same screens, the
#                                   engine, and Xenify Write's checker in a
#                                   worker (checker/, scripts/build-checker-web.sh)
# Usage: scripts/build-draw-web.sh [--dev]   (--dev: a quick, unoptimised engine)
# Try the browser version with: python3 -m http.server -d target/draw-web
#
# For a site with more than one app (scripts/build-site.sh), the checker is
# shared: CHECKER_AT=../shared/checker/ points the worker there instead of
# copying it in, OUT=… builds somewhere else, and SKIP_CHECKER=1 uses the
# target/checker-web already built.
set -euo pipefail
cd "$(dirname "$0")/.."
profile=web
if [ "${1:-}" = "--dev" ]; then profile=dev; fi
dir() { [ "$1" = dev ] && echo debug || echo "$1"; }
app=crates/xenify-draw-app
out=${OUT:-target/draw-web}
checker_at=${CHECKER_AT:-checker/}

[ "${SKIP_CHECKER:-}" = 1 ] || scripts/build-checker-web.sh
# WebAssembly SIMD (Chrome/Edge 91+, Firefox 89+, Safari 16.4+, WebKitGTK
# 2.40+) makes Vello's drawing 3-5 times faster: Draw's fire and smoke effects
# were unusable in browsers without it. Its own build folder, so the other
# apps' builds (the checker's candle doesn't build with SIMD) aren't redone.
draw_target=target/draw-simd
CARGO_TARGET_DIR=$draw_target RUSTFLAGS='--cfg getrandom_backend="wasm_js" -C target-feature=+simd128' \
    cargo build -p xenify-draw-web --target wasm32-unknown-unknown --profile $profile

wasm_opt=$(command -v wasm-opt || ls "$HOME"/tools/binaryen-*/bin/wasm-opt 2>/dev/null | head -1 || true)
rm -rf "$app/ui/pkg"
wasm-bindgen --target web --no-typescript --out-dir "$app/ui/pkg" "$draw_target/wasm32-unknown-unknown/$(dir $profile)/xenify_draw_web.wasm"
if [ -n "$wasm_opt" ] && [ "$profile" != dev ]; then
    "$wasm_opt" -O3 --strip-debug --strip-producers --enable-bulk-memory --enable-simd \
        --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
        --enable-reference-types --enable-multivalue \
        -o "$app/ui/pkg/xenify_draw_web_bg.wasm" "$app/ui/pkg/xenify_draw_web_bg.wasm"
fi

rm -rf "$out"
mkdir -p "$out"
cp -r "$app/ui/." "$out/"
cp "$app/icons/icon.png" "$out/icon.png"
if [ "$checker_at" = checker/ ]; then
    cp -r target/checker-web "$out/checker"
    rm "$out/checker/files.json"
else
    # (-i with a suffix works with both GNU and macOS sed.)
    sed -i.orig "s|'./checker/|'./${checker_at}|" "$out/checker-worker.js" && rm "$out/checker-worker.js.orig"
fi
cp LICENSE "$out/LICENSE.txt"
cp THIRD_PARTY_LICENSES.txt "$out/"

# The engine is loaded by its content hash, so the offline copy (sw.js) can
# take it from the browser's cache instead of downloading it again.
engine_hash=$(python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest()[:16])' "$out/pkg/xenify_draw_web_bg.wasm")
python3 - "$out/index.html" "$engine_hash" <<'PY'
import sys
path, h = sys.argv[1:]
s = open(path).read()
old = '<script src="errors.js"></script>'
assert old in s, "index.html changed: update build-draw-web.sh"
s = s.replace(old, f'<link rel="icon" href="icon.png">\n<script>window.XENIFY_WASM_URL = \'pkg/xenify_draw_web_bg.wasm?v={h}\';</script>\n' + old, 1)
open(path, "w").write(s)
PY

# Offline: sw.js gets every file with its content hash, and the checker's
# files (kept in a cache shared with the site's other apps) and where they are.
python3 - "$out" "$checker_at" <<'PY'
import hashlib, json, os, sys
out, checker_at = sys.argv[1:]
files = {}
for root, _, names in os.walk(out):
    for n in names:
        rel = os.path.relpath(os.path.join(root, n), out).replace(os.sep, "/")
        if rel in ("sw.js", "_headers") or rel.startswith("checker/"):
            continue
        files[rel] = {"h": hashlib.sha256(open(os.path.join(out, rel), "rb").read()).hexdigest()[:16]}
sw = os.path.join(out, "sw.js")
s = open(sw).read()
for old in ("/*FILES*/ {}", "/*SHARED*/ {}", "/*SHARED_AT*/ 'checker/'"):
    assert old in s, old
s = s.replace("/*FILES*/ {}", json.dumps(dict(sorted(files.items())), separators=(",", ":")), 1)
s = s.replace("/*SHARED*/ {}", open("target/checker-web/files.json").read(), 1)
s = s.replace("/*SHARED_AT*/ 'checker/'", json.dumps(checker_at), 1)
open(sw, "w").write(s)
PY

# Headers for hosts that read a _headers file (Netlify, Cloudflare Pages):
# files fetched by hash never change; everything else is checked each time.
cat > "$out/_headers" <<'EOF'
/*
  Cache-Control: no-cache
  X-Content-Type-Options: nosniff
/pkg/*.wasm
  Cache-Control: public, max-age=31536000, immutable
/checker/*.wasm
  Cache-Control: public, max-age=31536000, immutable
/checker/spotter/model.part*
  Cache-Control: public, max-age=31536000, immutable
EOF
du -sh "$out" "$out"/pkg/*.wasm
