#!/usr/bin/env bash
# Builds Xenify Write's spelling and grammar checker for the browser into
# target/checker-web, the folder every web app uses for it (Write and Draw):
#   kept.js                                the loader the apps' workers import
#   xenify_write_check_web.js, _bg.wasm    the checker (rules, Look up)
#   spotter/                               the mistake-spotter model, in 16 MB parts
#   files.json                             every file with its hash and size,
#                                          for the apps' offline copies (sw.js)
# On xenify.net it's served once at /shared/checker/ and kept in the browser
# once for both apps; a standalone app build gets its own copy in checker/.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/checker-web

scripts/stage-spotter.sh
# (wasm SIMD was tried for the spotter's maths: no faster, so not used.)
cargo build -p xenify-write-check-web --target wasm32-unknown-unknown --profile web-checker

rm -rf "$out"
mkdir -p "$out/spotter"
wasm-bindgen --target web --no-typescript --out-dir "$out" target/wasm32-unknown-unknown/web-checker/xenify_write_check_web.wasm
wasm_opt=$(command -v wasm-opt || ls "$HOME"/tools/binaryen-*/bin/wasm-opt 2>/dev/null | head -1 || true)
if [ -n "$wasm_opt" ]; then
    # For speed (the spotter's maths), not size.
    "$wasm_opt" -O3 --strip-debug --strip-producers --enable-bulk-memory --enable-simd \
        --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
        --enable-reference-types --enable-multivalue \
        -o "$out/xenify_write_check_web_bg.wasm" "$out/xenify_write_check_web_bg.wasm"
else
    echo "warning: wasm-opt (binaryen) not found; the download is bigger than it need be" >&2
fi

cp target/spotter/config.json target/spotter/tokenizer.json target/spotter/README.md target/spotter/LICENSE-DeBERTa.txt "$out/spotter/"
# model.part00, model.part01, ... (in Python: macOS's split has no -d).
python3 - target/spotter/model.f16.safetensors "$out/spotter/model.part" <<'PY'
import sys
src, prefix = sys.argv[1:]
with open(src, "rb") as f:
    i = 0
    while chunk := f.read(16 << 20):
        open(f"{prefix}{i:02d}", "wb").write(chunk)
        i += 1
PY
cp crates/xenify-write-check-web/web/kept.js "$out/"

# kept.js gets the listing of the other files; files.json lists them all.
python3 - "$out" <<'PY'
import hashlib, json, os, sys
root = sys.argv[1]
def entry(p):
    data = open(os.path.join(root, p), "rb").read()
    return {"h": hashlib.sha256(data).hexdigest()[:16], "n": len(data)}
files = {}
for d, _, names in os.walk(root):
    for n in names:
        p = os.path.relpath(os.path.join(d, n), root).replace(os.sep, "/")
        if p not in ("kept.js", "files.json"):
            files[p] = entry(p)
kept = os.path.join(root, "kept.js")
s = open(kept).read()
assert "/*FILES*/ {}" in s
open(kept, "w").write(s.replace("/*FILES*/ {}", json.dumps(dict(sorted(files.items())), separators=(",", ":")), 1))
files["kept.js"] = entry("kept.js")
json.dump(dict(sorted(files.items())), open(os.path.join(root, "files.json"), "w"), separators=(",", ":"))
PY
du -sh "$out" "$out/xenify_write_check_web_bg.wasm"
