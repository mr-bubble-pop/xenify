#!/usr/bin/env bash
# Builds Xenify Write for the browser into target/write-web: the desktop app's
# screens (crates/xenify-write-app/ui) with the document engine as
# WebAssembly. Upload the folder to any static web host (see
# docs/write-packaging.md for the server settings), or try it with
#   scripts/serve-write-web.py
#
# What loads when, so the first page shows quickly:
#   1. index.html, the screens, the document engine   pkg/xenify_write_web_bg.wasm
#   2. maths, CMU Serif and emoji fonts               fonts.bin
#   3. spelling, grammar and Look up (a Web Worker)    checker/ (scripts/build-checker-web.sh)
#   4. the mistake spotter model, in parts            checker/spotter/
# Everything is kept in the browser by content hash (offline.js, sw.js): it
# works offline after the first visit, and an update downloads only the
# files that changed. Each file also gets .br and .gz copies for servers
# that send pre-compressed files.
#
# For a site with more than one app (scripts/build-site.sh), the checker is
# shared: CHECKER_AT=../shared/checker/ points the worker there instead of
# copying it in, OUT=… builds somewhere else, and SKIP_CHECKER=1 uses the
# target/checker-web already built.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${OUT:-target/write-web}
checker_at=${CHECKER_AT:-checker/}
web=crates/xenify-write-web/web
profile=web
wasm_dir=target/wasm32-unknown-unknown/$profile

[ "${SKIP_CHECKER:-}" = 1 ] || scripts/build-checker-web.sh
cargo build -p xenify-write-web --target wasm32-unknown-unknown --profile $profile

rm -rf "$out"
mkdir -p "$out/pkg"
cp -r crates/xenify-write-app/ui/. "$out/"
cp $web/tauri-shim.js $web/offline.js $web/checker-worker.js $web/sw.js $web/manifest.webmanifest "$out/"
if [ "$checker_at" = checker/ ]; then
    cp -r target/checker-web "$out/checker"
    rm "$out/checker/files.json"
else
    sed -i "s|'./checker/|'./${checker_at}|" "$out/checker-worker.js"
fi
cp crates/xenify-write-app/icons/icon.png "$out/icon.png"
if command -v convert >/dev/null; then convert "$out/icon.png" -resize 192x192 "$out/icon-192.png"; else cp "$out/icon.png" "$out/icon-192.png"; fi

wasm_opt=$(command -v wasm-opt || ls "$HOME"/tools/binaryen-*/bin/wasm-opt 2>/dev/null | head -1 || true)
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" "$wasm_dir/xenify_write_web.wasm"
if [ -n "$wasm_opt" ]; then
    # The engine for size (the checker is built for speed).
    "$wasm_opt" -Os --strip-debug --strip-producers --enable-bulk-memory --enable-simd \
        --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
        --enable-reference-types --enable-multivalue \
        -o "$out/pkg/xenify_write_web_bg.wasm" "$out/pkg/xenify_write_web_bg.wasm"
else
    echo "warning: wasm-opt (binaryen) not found; the downloads are bigger than they need be" >&2
fi

# The fonts that come after the first page: a little-endian u32 length
# before each file (read by late_fonts in crates/xenify-write-web).
python3 - "$out/fonts.bin" assets/fonts/STIXTwoMath-Regular.ttf assets/fonts/cmunrm.otf assets/fonts/cmunti.otf \
    assets/fonts/cmunbx.otf assets/fonts/cmunbi.otf assets/emoji/fluent-emoji.json.z <<'PY'
import struct, sys
with open(sys.argv[1], "wb") as out:
    for path in sys.argv[2:]:
        data = open(path, "rb").read()
        out.write(struct.pack("<I", len(data)) + data)
PY

cp LICENSE "$out/LICENSE.txt"
if command -v cargo-about >/dev/null; then scripts/licenses.sh; fi
cp THIRD_PARTY_LICENSES.txt "$out/"

# index.html: the offline copy's manifest, early downloads, a blank sheet
# until the engine has drawn the first page, and the shim before the screens.
core_hash() { sha256sum "$1" | cut -c1-16; }
core_key="pkg/xenify_write_web_bg.wasm?v=$(core_hash "$out/pkg/xenify_write_web_bg.wasm")"
python3 - "$out/index.html" "$core_key" <<'PY'
import sys
path, core = sys.argv[1:]
s = open(path).read()
head = f'''<link rel="manifest" href="manifest.webmanifest">
<meta name="theme-color" content="#EFECE8">
<link rel="icon" href="icon-192.png">
<link rel="preload" href="{core}" as="fetch" type="application/wasm" crossorigin>
<link rel="modulepreload" href="tauri-shim.js">
<link rel="modulepreload" href="app.js">
<style>#web-loading {{ position: absolute; top: 40px; left: 50%; width: min(794px, calc(100% - 32px)); height: 1123px;
  transform: translateX(-50%); background: var(--sheet, #fff); box-shadow: var(--shadow); border-radius: 2px; }}</style>
'''
s = s.replace('<link rel="stylesheet" href="style.css">', '<link rel="stylesheet" href="style.css">\n' + head, 1)
s = s.replace('<script type="module" src="app.js"></script>',
              '<script type="module" src="tauri-shim.js"></script>\n<script type="module" src="app.js"></script>', 1)
s = s.replace('<div id="stack" class="stack">', '<div id="web-loading" aria-hidden="true"></div>\n        <div id="stack" class="stack">', 1)
assert s.count('tauri-shim.js') == 2 and 'web-loading' in s, "index.html changed: update build-write-web.sh"
open(path, "w").write(s)
PY

# Every file with its content hash and size, for offline.js and sw.js; sw.js
# also gets the checker's files (kept in a cache of their own, shared with
# the other apps on the site) and where they are.
python3 - "$out" "$checker_at" <<'PY'
import hashlib, json, os, sys
root, checker_at = sys.argv[1:]
files = {}
for d, _, names in os.walk(root):
    for n in names:
        p = os.path.relpath(os.path.join(d, n), root).replace(os.sep, "/")
        if p == "sw.js" or p.endswith((".br", ".gz")) or p == "_headers" or p.startswith("checker/"):
            continue
        data = open(os.path.join(root, p), "rb").read()
        files[p] = {"h": hashlib.sha256(data).hexdigest()[:16], "n": len(data)}
# offline.js is listed with its hash before it's filled in; it changes
# whenever any other file does, so the browser still sees a new version.
listing = json.dumps(dict(sorted(files.items())), separators=(",", ":"))
for name in ("offline.js", "sw.js"):
    path = os.path.join(root, name)
    s = open(path).read()
    assert "/*FILES*/ {}" in s, name
    open(path, "w").write(s.replace("/*FILES*/ {}", listing, 1))
sw = os.path.join(root, "sw.js")
s = open(sw).read()
shared = open("target/checker-web/files.json").read()
assert "/*SHARED*/ {}" in s and "/*SHARED_AT*/ 'checker/'" in s
s = s.replace("/*SHARED*/ {}", shared, 1).replace("/*SHARED_AT*/ 'checker/'", json.dumps(checker_at), 1)
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
/fonts.bin
  Cache-Control: public, max-age=31536000, immutable
/checker/*.wasm
  Cache-Control: public, max-age=31536000, immutable
/checker/spotter/model.part*
  Cache-Control: public, max-age=31536000, immutable
EOF

# Pre-compressed copies (kept only where they save something).
find "$out" -type f ! -name '*.br' ! -name '*.gz' ! -name '_headers' ! -name '*.png' -print0 |
    xargs -0 -P "$(nproc)" -I{} sh -c '
        f="$1"
        case "$f" in *model.part*) q=5 ;; *) q=11 ;; esac
        brotli -f -q $q -o "$f.br" "$f" && gzip -9 -k -f "$f"
        for c in "$f.br" "$f.gz"; do
            [ $(stat -c%s "$c") -lt $(( $(stat -c%s "$f") * 95 / 100 )) ] || rm -f "$c"
        done' _ {}

size() { du -b "$@" | awk '{s+=$1} END {printf "%.1f MB", s/1e6}'; }
first=("$out/index.html" "$out"/*.js "$out/style.css" "$out"/fonts/*.ttf "$out/pkg/xenify_write_web.js" "$out/pkg/xenify_write_web_bg.wasm")
brs() { for f in "$@"; do if [ -d "$f" ]; then cat "$f"/*; elif [ -f "$f.br" ]; then cat "$f.br"; else cat "$f"; fi; done | wc -c | awk '{printf "%.1f MB", $1/1e6}'; }
echo "Built $out"
echo "  first page:   $(brs "${first[@]}") to download (brotli)"
echo "  fonts:        $(brs "$out/fonts.bin")"
if [ -d "$out/checker" ]; then
    echo "  checker:      $(brs "$out/checker/xenify_write_check_web_bg.wasm" "$out/checker/xenify_write_check_web.js" "$out/checker/kept.js")"
    echo "  spotter:      $(brs "$out"/checker/spotter/model.part[0-9][0-9] "$out/checker/spotter/config.json" "$out/checker/spotter/tokenizer.json")"
else
    echo "  checker:      shared, at $checker_at"
fi
