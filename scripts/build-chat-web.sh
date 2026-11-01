#!/usr/bin/env bash
# Builds Xenify Chat for the browser into target/chat-web: the desktop app's
# screens (crates/xenify-chat-app/ui) with the shared core
# (crates/xenify-chat-core) as WebAssembly and a page-side shim
# (crates/xenify-chat-web/web/tauri-shim.js) that keeps the WebSocket, does
# HTTP and sends files. Whiteboards use Xenify Draw's browser version and
# spelling uses the site's checker, both found beside it on the site:
#   DRAW_AT=../draw/  CHECKER_AT=../shared/checker/  (the defaults)
# A Xenify server serving the site (XENIFY_APPS) serves Chat at /chat/ too,
# signed in to that server; on xenify.net it asks which server.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${OUT:-target/chat-web}
draw_at=${DRAW_AT:-../draw/}
checker_at=${CHECKER_AT:-../shared/checker/}
profile=${PROFILE:-web}
wasm_dir=target/wasm32-unknown-unknown/$([ "$profile" = dev ] && echo debug || echo "$profile")

cargo build -p xenify-chat-web --target wasm32-unknown-unknown --profile "$profile"

rm -rf "$out"
mkdir -p "$out/pkg"
rsync -a --exclude=draw/ crates/xenify-chat-app/ui/ "$out/"
cp crates/xenify-chat-web/web/tauri-shim.js crates/xenify-chat-web/web/sw.js "$out/"
cp crates/xenify-chat-app/icons/128x128.png "$out/icon.png" 2>/dev/null || cp crates/xenify-chat-app/icons/icon.png "$out/icon.png"

wasm_opt=$(command -v wasm-opt || ls "$HOME"/tools/binaryen-*/bin/wasm-opt 2>/dev/null | head -1 || true)
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" "$wasm_dir/xenify_chat_web.wasm"
if [ -n "$wasm_opt" ]; then
    "$wasm_opt" -Os --strip-debug --strip-producers --enable-bulk-memory --enable-simd \
        --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
        --enable-reference-types --enable-multivalue \
        -o "$out/pkg/xenify_chat_web_bg.wasm" "$out/pkg/xenify_chat_web_bg.wasm"
else
    echo "warning: wasm-opt (binaryen) not found; the download is bigger than it need be" >&2
fi

# index.html: where Draw and the checker are, and the shim before the screens.
python3 - "$out/index.html" "$draw_at" "$checker_at" <<'PY'
import sys
path, draw, checker = sys.argv[1:]
s = open(path).read()
head = f'''<link rel="icon" href="icon.png">
<meta name="theme-color" content="#EFECE8">
<link rel="modulepreload" href="tauri-shim.js">
<script>window.XENIFY_DRAW_AT = {draw!r}; window.XENIFY_CHECKER_AT = {checker!r};</script>
'''
s = s.replace('<link rel="stylesheet" href="style.css">', '<link rel="stylesheet" href="style.css">\n' + head, 1)
s = s.replace('<script type="module" src="app.js"></script>',
              '<script type="module" src="tauri-shim.js"></script>\n<script type="module" src="app.js"></script>', 1)
assert s.count('tauri-shim.js') == 2, "index.html changed: update build-chat-web.sh"
open(path, "w").write(s)
PY

# Always the newest screens and core (they must match the server's).
cat > "$out/_headers" <<'H'
/*
  Cache-Control: no-cache
H
echo "built $out"
