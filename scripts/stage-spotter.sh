#!/usr/bin/env bash
# Puts the mistake spotter (model B7) and its credits in target/spotter, for
# the packages (scripts/package-write.sh) and the web build to ship:
#   config.json  tokenizer.json  model.f16.safetensors  README.md LICENSE-DeBERTa.txt
# The model isn't kept in the repo (67 MB). It's taken from, in order:
# $SPOTTER_DIR, the installed copy (~/.local/share/xenify/spotter), the
# backup checkpoints/models/B7.tgz, or $SPOTTER_URL (a B7.tgz to download,
# e.g. a release asset). A downloaded B7.tgz must match B7_SHA256 below (or
# $SPOTTER_SHA256, for another model); checkpoints/SHA256SUMS has the same
# value. See docs/grammar-checker.md.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/spotter
# B7.tgz as made on 2026-10-12 (checkpoints/SHA256SUMS).
B7_SHA256=93e60b346e81b418d07c5302724da8f0b79cbef7ac8b5095928131a756c8e8ae
want=(config.json tokenizer.json model.f16.safetensors)
has_model() { for f in "${want[@]}"; do [ -f "$1/$f" ] || return 1; done; }
src=""
for d in "${SPOTTER_DIR:-}" "${XDG_DATA_HOME:-$HOME/.local/share}/xenify/spotter"; do
    if [ -n "$d" ] && has_model "$d"; then src=$d; break; fi
done
if [ -z "$src" ] && [ ! -f checkpoints/models/B7.tgz ] && [ -n "${SPOTTER_URL:-}" ]; then
    want_sha=${SPOTTER_SHA256:-$B7_SHA256}
    if [ -z "$want_sha" ]; then
        echo "Set SPOTTER_SHA256 to the model's checksum before downloading it." >&2
        exit 1
    fi
    mkdir -p checkpoints/models
    # Kept only once it matches.
    curl -fsSL "$SPOTTER_URL" -o checkpoints/models/B7.tgz.part
    if ! echo "$want_sha  checkpoints/models/B7.tgz.part" | { command -v sha256sum >/dev/null && sha256sum -c - || shasum -a 256 -c -; } >/dev/null; then
        rm -f checkpoints/models/B7.tgz.part
        echo "The downloaded model doesn't match its checksum ($want_sha): not used." >&2
        exit 1
    fi
    mv checkpoints/models/B7.tgz.part checkpoints/models/B7.tgz
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
cp assets/licenses/deberta-LICENSE.txt "$out/LICENSE-DeBERTa.txt"
echo "Mistake spotter ready in $out (from $src)"
