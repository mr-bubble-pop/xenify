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
#   3. spelling, grammar and Look up (a Web Worker)    pkg/xenify_write_check_web_bg.wasm
#   4. the mistake spotter model, in parts            spotter/
# Everything is kept in the browser by content hash (offline.js, sw.js): it
# works offline after the first visit, and an update downloads only the
# files that changed. Each file also gets .br and .gz copies for servers
# that send pre-compressed files.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/write-web
web=crates/xenify-write-web/web
profile=web
wasm_dir=target/wasm32-unknown-unknown/$profile

scripts/stage-spotter.sh

# Two builds, not one: together, cargo would build the checker into both.
cargo build -p xenify-write-web --target wasm32-unknown-unknown --profile $profile
# (wasm SIMD was tried for the spotter's maths: no faster, so not used.)
cargo build -p xenify-write-check-web --target wasm32-unknown-unknown --profile web-checker

rm -rf "$out"
mkdir -p "$out/pkg" "$out/spotter"
cp -r crates/xenify-write-app/ui/. "$out/"
cp $web/tauri-shim.js $web/offline.js $web/checker-worker.js $web/sw.js $web/manifest.webmanifest "$out/"
cp crates/xenify-write-app/icons/icon.png "$out/icon.png"
if command -v convert >/dev/null; then convert "$out/icon.png" -resize 192x192 "$out/icon-192.png"; else cp "$out/icon.png" "$out/icon-192.png"; fi

wasm_opt=$(command -v wasm-opt || ls "$HOME"/tools/binaryen-*/bin/wasm-opt 2>/dev/null | head -1 || true)
for crate in xenify_write_web xenify_write_check_web; do
    from=$wasm_dir; [ $crate = xenify_write_check_web ] && from=target/wasm32-unknown-unknown/web-checker
    wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" "$from/$crate.wasm"
    if [ -n "$wasm_opt" ]; then
        # The engine for size; the checker for speed (the spotter's maths).
        level=-Os; [ $crate = xenify_write_check_web ] && level=-O3
        "$wasm_opt" $level --strip-debug --strip-producers --enable-bulk-memory \
            --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
            --enable-reference-types --enable-multivalue \
            -o "$out/pkg/${crate}_bg.wasm" "$out/pkg/${crate}_bg.wasm"
    else
        echo "warning: wasm-opt (binaryen) not found; the downloads are bigger than they need be" >&2
    fi
done

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

# The mistake spotter: its credits, and the weights in 16 MB parts.
cp target/spotter/config.json target/spotter/tokenizer.json target/spotter/README.md "$out/spotter/"
split -b 16M -d -a 2 target/spotter/model.f16.safetensors "$out/spotter/model.part"
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

# Every file with its content hash and size, for offline.js and sw.js.
python3 - "$out" <<'PY'
import hashlib, json, os, sys
root = sys.argv[1]
files = {}
for d, _, names in os.walk(root):
    for n in names:
        p = os.path.relpath(os.path.join(d, n), root).replace(os.sep, "/")
        if p == "sw.js" or p.endswith((".br", ".gz")) or p == "_headers":
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
/spotter/model.part*
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
echo "  checker:      $(brs "$out/pkg/xenify_write_check_web_bg.wasm" "$out/pkg/xenify_write_check_web.js")"
echo "  spotter:      $(brs "$out"/spotter/model.part[0-9][0-9] "$out/spotter/config.json" "$out/spotter/tokenizer.json")"
