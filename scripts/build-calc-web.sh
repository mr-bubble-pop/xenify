#!/usr/bin/env bash
# Builds Xenify Calc's engine as WebAssembly and puts it where the screens
# load it:
#   crates/xenify-calc-app/ui/pkg   the desktop app's copy (Tauri bundles ui/)
#   target/calc-web                 the browser version: the same screens and engine
# Usage: scripts/build-calc-web.sh [--dev]   (--dev: a quick, unoptimised engine)
# Try the browser version with: python3 -m http.server -d target/calc-web
set -euo pipefail
cd "$(dirname "$0")/.."
profile=web
if [ "${1:-}" = "--dev" ]; then profile=dev; fi
dir() { [ "$1" = dev ] && echo debug || echo "$1"; }
app=crates/xenify-calc-app
out=${OUT:-target/calc-web}
# Spelling and grammar: Xenify Write's checker in a worker (as Draw has it).
# A site with more than one app shares it: CHECKER_AT=../shared/checker/.
checker_at=${CHECKER_AT:-checker/}

[ "${SKIP_CHECKER:-}" = 1 ] || [ -f target/checker-web/kept.js ] || scripts/build-checker-web.sh

cargo build -p xenify-calc-web --target wasm32-unknown-unknown --profile $profile

wasm_opt=$(command -v wasm-opt || ls "$HOME"/tools/binaryen-*/bin/wasm-opt 2>/dev/null | head -1 || true)
rm -rf "$app/ui/pkg"
wasm-bindgen --target web --no-typescript --out-dir "$app/ui/pkg" "target/wasm32-unknown-unknown/$(dir $profile)/xenify_calc_web.wasm"
if [ -n "$wasm_opt" ] && [ "$profile" != dev ]; then
    "$wasm_opt" -O3 --strip-debug --strip-producers --enable-bulk-memory --enable-simd \
        --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
        --enable-reference-types --enable-multivalue \
        -o "$app/ui/pkg/xenify_calc_web_bg.wasm" "$app/ui/pkg/xenify_calc_web_bg.wasm"
fi

rm -rf "$out"
mkdir -p "$out"
cp -r "$app/ui/." "$out/"
cp "$app/icons/icon.png" "$out/icon.png"
if [ "$checker_at" = checker/ ]; then
    cp -r target/checker-web "$out/checker"
    rm -f "$out/checker/files.json"
else
    sed -i.orig "s|'./checker/|'./${checker_at}|" "$out/checker-worker.js" && rm "$out/checker-worker.js.orig"
fi
cp LICENSE "$out/LICENSE.txt"
cp THIRD_PARTY_LICENSES.txt "$out/"

# The engine is loaded by its content hash, so browsers don't keep an old one.
engine_hash=$(python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest()[:16])' "$out/pkg/xenify_calc_web_bg.wasm")
python3 - "$out/index.html" "$engine_hash" <<'PY'
import sys
path, h = sys.argv[1:]
s = open(path).read()
old = '<script src="errors.js"></script>'
assert old in s, "index.html changed: update build-calc-web.sh"
s = s.replace(old, f'<link rel="icon" href="icon.png">\n<script>window.XENIFY_WASM_URL = \'pkg/xenify_calc_web_bg.wasm?v={h}\';</script>\n' + old, 1)
open(path, "w").write(s)
PY

cat > "$out/_headers" <<'HEADERS'
/*
  Cache-Control: no-cache
  X-Content-Type-Options: nosniff
/pkg/*.wasm
  Cache-Control: public, max-age=31536000, immutable
HEADERS
du -sh "$out" "$out"/pkg/*.wasm
