#!/usr/bin/env bash
# Copies the Xenify server client (crates/xenify-server/client: server.js,
# serverui.js, server.css)
# into the apps that use it. A test in xenify-server checks the copies match.
set -euo pipefail
cd "$(dirname "$0")/.."
for app in crates/xenify-write-app/ui crates/xenify-draw-app/ui crates/xenify-calc-app/ui crates/xenify-plan-app/ui; do
  cp crates/xenify-server/client/server.js crates/xenify-server/client/serverui.js crates/xenify-server/client/server.css "$app/"
done
echo "copied the server client into Write, Draw, Calc and Plan"
