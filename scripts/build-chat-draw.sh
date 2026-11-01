#!/usr/bin/env bash
# Puts Xenify Draw's browser version into Xenify Chat (crates/xenify-chat-app/ui/draw),
# where it runs chats' whiteboards (Draw's embed.js). Without Draw's own spelling
# model: the board doesn't need it, and it would make Chat 87 MB bigger.
set -euo pipefail
cd "$(dirname "$0")/.."
out=crates/xenify-chat-app/ui/draw
SKIP_CHECKER=${SKIP_CHECKER:-1} OUT=$out scripts/build-draw-web.sh "$@"
rm -rf "$out/checker"
echo "built $out"
