#!/usr/bin/env bash
# Builds the showcase presentation (assets/showcase.xenify) with the
# program's own builder, and records it in assets/SOURCES.toml. Everything in
# it is made by the program: shapes, text, particles, 3D, clips rendered from
# its own compositions, and sounds and music from its synthesizer; plus the
# bundled Fluent Emoji (MIT).
#
#   scripts/build-showcase.sh [product name]
set -euo pipefail
cd "$(dirname "$0")/.."
NAME="${1:-Spark}"
cargo run -q --release -p xenify-cli -- showcase --name "$NAME" assets/showcase.xenify
export NAME
python3 - <<'PY'
import datetime, hashlib, os, re
sha = hashlib.sha256(open("assets/showcase.xenify", "rb").read()).hexdigest()
today = datetime.date.today().isoformat()
block = f'''# BEGIN build-showcase.sh (written by scripts/build-showcase.sh; don't edit by hand)
[[asset]]
path = "assets/showcase.xenify"
name = "The showcase presentation (Help › Showcase)"
source = "generated"
generator = "crates/xenify-showcase/src/lib.rs (build_with), crates/xenify-audio/src/synth.rs"
license = "MIT"
fetched = "{today}"
sha256 = "{sha}"
notes = """Original. Built by the program's own editing operations (scripts/build-showcase.sh, product name \\
"{os.environ['NAME']}"): shapes, text, particles, 3D and diagrams drawn in code; its video clips are rendered from its \\
own compositions; every sound and the music bed come from the built-in synthesizer. The only outside material is the \\
bundled Fluent Emoji (MIT, see its entry). No third-party material otherwise; no GIPHY or downloaded content."""
# END build-showcase.sh
'''
src = open("assets/SOURCES.toml").read()
src = re.sub(r"\n?# BEGIN build-showcase\.sh.*?# END build-showcase\.sh\n", "", src, flags=re.S)
open("assets/SOURCES.toml", "w").write(src.rstrip("\n") + "\n\n" + block)
print(f"showcase: {os.path.getsize('assets/showcase.xenify') // 1024} KB, sha256 {sha[:12]}…")
PY
