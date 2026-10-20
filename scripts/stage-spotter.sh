#!/usr/bin/env bash
# Puts the mistake spotter (model B7) and its credits in target/spotter, for
# the packages (scripts/package-write.sh) and the web build to ship:
#   config.json  tokenizer.json  model.f16.safetensors  README.md
# The model isn't kept in the repo (67 MB). It's taken from, in order:
# $SPOTTER_DIR, the installed copy (~/.local/share/xenify/spotter), the
# backup checkpoints/models/B7.tgz, or $SPOTTER_URL (a B7.tgz to download,
# e.g. a release asset; checked against $SPOTTER_SHA256 when set). See
# docs/grammar-checker.md.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/spotter
want=(config.json tokenizer.json model.f16.safetensors)
has_model() { for f in "${want[@]}"; do [ -f "$1/$f" ] || return 1; done; }
src=""
for d in "${SPOTTER_DIR:-}" "${XDG_DATA_HOME:-$HOME/.local/share}/xenify/spotter"; do
    if [ -n "$d" ] && has_model "$d"; then src=$d; break; fi
done
if [ -z "$src" ] && [ ! -f checkpoints/models/B7.tgz ] && [ -n "${SPOTTER_URL:-}" ]; then
    mkdir -p checkpoints/models
    curl -fsSL "$SPOTTER_URL" -o checkpoints/models/B7.tgz
    if [ -n "${SPOTTER_SHA256:-}" ]; then
        echo "$SPOTTER_SHA256  checkpoints/models/B7.tgz" | sha256sum -c - >/dev/null || { echo "B7.tgz doesn't match SPOTTER_SHA256" >&2; exit 1; }
    fi
fi
if [ -z "$src" ] && [ -f checkpoints/models/B7.tgz ]; then
    rm -rf target/spotter-unpack && mkdir -p target/spotter-unpack
    tar -C target/spotter-unpack -xzf checkpoints/models/B7.tgz
    src=target/spotter-unpack/B7
fi
if [ -z "$src" ] || ! has_model "$src"; then
    echo "No mistake spotter model found. Set SPOTTER_DIR to the B7 folder." >&2
    exit 1
fi
mkdir -p "$out"
for f in "${want[@]}"; do
    cmp -s "$src/$f" "$out/$f" 2>/dev/null || cp "$src/$f" "$out/$f"
done
cp crates/xenify-grammar-spotter/model/README.md "$out/README.md"
echo "Mistake spotter ready in $out (from $src)"
