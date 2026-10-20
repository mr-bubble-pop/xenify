#!/usr/bin/env bash
# Regenerates THIRD_PARTY_LICENSES.txt. Ship that file (and LICENSE) with
# every release build. Run after changing dependencies.
# Needs: cargo install cargo-about --locked --features cli
set -euo pipefail
cd "$(dirname "$0")/.."

out=THIRD_PARTY_LICENSES.txt
cargo about generate --workspace --fail about.hbs > "$out"

# Assets (fonts, emoji, …): every assets/SOURCES.toml entry with a licence file.
python3 - "$out" <<'PY'
import sys, tomllib
out = sys.argv[1]
with open("assets/SOURCES.toml", "rb") as f:
    entries = tomllib.load(f).get("asset", [])
with open(out, "a") as o:
    for e in entries:
        if "license_file" not in e:
            continue
        o.write("-" * 80 + "\n")
        o.write(f"{e['name']} ({e.get('path', '')}, {e['license']})\n")
        if "source_url" in e:
            o.write(f"Source: {e['source_url']} {e.get('version', '')}\n")
        if "attribution" in e:
            o.write(f"Attribution: {e['attribution']}\n")
        o.write("\n")
        with open(e["license_file"]) as lf:
            o.write(lf.read())
        o.write("\n")
PY

echo "wrote $out"
