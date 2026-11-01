#!/usr/bin/env bash
# Runs Calc's screen tests (crates/xenify-calc-app/tests/ui/*.js) in headless
# Chromium against the browser build. Each test sets window.XENIFY_TEST,
# which the app runs once it has started, and writes PASSED or FAILED with
# what it checked into #test-result.
# Usage: scripts/test-calc-ui.sh [test-name ...]   (default: every test)
#   SHOTS=dir   also saves a screenshot of each test's end state there
# Needs the browser build: scripts/build-calc-web.sh --dev (built if missing).
set -euo pipefail
cd "$(dirname "$0")/.."
web=${OUT:-target/calc-web}
tests=crates/xenify-calc-app/tests/ui
[ -f "$web/index.html" ] || scripts/build-calc-web.sh --dev
chrome=$(command -v chromium || command -v chromium-browser || command -v google-chrome)
names=("$@")
[ ${#names[@]} -gt 0 ] || names=($(cd "$tests" && ls *.js | sed 's/\.js$//'))

port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
python3 -m http.server -b 127.0.0.1 -d "$web" "$port" >/dev/null 2>&1 &
server=$!
profiles=$(mktemp -d)
trap 'kill $server 2>/dev/null; rm -rf "$profiles" "$web"/test-*.html "$web"/test-*.js' EXIT
sleep 0.5

failed=0
for name in "${names[@]}"; do
    cp "$tests/$name.js" "$web/test-$name.js"
    sed "s|<script src=\"errors.js\"></script>|<script src=\"test-$name.js\"></script>\n&|" "$web/index.html" > "$web/test-$name.html"
    url="http://127.0.0.1:$port/test-$name.html"
    # A new profile each time: no recovery copies left from an earlier run.
    flags=(--headless --no-sandbox --disable-gpu --user-data-dir="$(mktemp -d -p "$profiles")" --window-size=1400,900 --virtual-time-budget=30000)
    result=$("$chrome" "${flags[@]}" --dump-dom "$url" 2>/dev/null | python3 -c '
import html, re, sys
m = re.search(r"<pre id=\"test-result\">(.*?)</pre>", sys.stdin.read(), re.S)
print(html.unescape(m.group(1)) if m else "FAILED\n(no result: the test did not finish)")')
    echo "== $name"
    echo "$result"
    [ "$(head -1 <<<"$result")" = PASSED ] || failed=1
    if [ -n "${SHOTS:-}" ]; then
        mkdir -p "$SHOTS"
        flags[3]=--user-data-dir="$(mktemp -d -p "$profiles")"
        "$chrome" "${flags[@]}" --screenshot="$SHOTS/$name.png" "$url" >/dev/null 2>&1 || true
    fi
done
exit $failed
