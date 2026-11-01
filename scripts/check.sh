#!/usr/bin/env bash
# Runs everything CI runs. Usage: scripts/check.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Core crates must stay browser-compatible (see plan: web build later).
cargo check --target wasm32-unknown-unknown -p xenify-route -p xenify-doc -p xenify-anim -p xenify-ops -p xenify-render -p xenify-platform -p xenify-audio
# Xenify Write's document, layout and file formats too (web version later).
cargo check --target wasm32-unknown-unknown -p xenify-wp-doc -p xenify-wp-layout -p xenify-wp-io -p xenify-math
