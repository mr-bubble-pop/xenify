#!/usr/bin/env bash
# Opens two Xenify Chat windows, as two different people, to try Chat by
# yourself: each gets its own settings under ~/.cache/xenify-try/.
#
#   scripts/try-chat.sh [server] [--fake-media]
#
# server defaults to http://127.0.0.1:7810. --fake-media gives each window a
# made-up camera, microphone and screen (for calls without real devices).
# "Open in Draw" uses this checkout's Draw. Close the windows to finish.
set -euo pipefail
cd "$(dirname "$0")/.."
server=http://127.0.0.1:7810
fake=
for a in "$@"; do
    case "$a" in
        --fake-media) fake=1 ;;
        *) server=$a ;;
    esac
done
cargo build -q -p xenify-chat-app -p xenify-draw-app
[ -d crates/xenify-chat-app/ui/draw ] || scripts/build-chat-draw.sh
base="$HOME/.cache/xenify-try"
mkdir -p "$base/bin"
ln -sf "$PWD/target/debug/xenify-draw-app" "$base/bin/xenify-draw"
for who in first second; do
    mkdir -p "$base/$who"
    PATH="$base/bin:$PATH" XDG_CONFIG_HOME="$base/$who/config" XDG_DATA_HOME="$base/$who/data" \
        XENIFY_SERVER="$server" ${fake:+XENIFY_FAKE_MEDIA=1} \
        nohup target/debug/xenify-chat >"$base/$who/log" 2>&1 &
    sleep 2
done
echo "Two Chat windows are opening, connected to $server."
