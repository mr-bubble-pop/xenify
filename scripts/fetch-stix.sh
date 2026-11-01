#!/usr/bin/env bash
# Downloads STIX Two Math (SIL OFL 1.1) from a pinned release tag, checks it
# by SHA-256, and records it in assets/SOURCES.toml. Xenify Write uses it as
# the fallback font for symbols (arrows, maths, Greek) and for equations.
#
#   scripts/fetch-stix.sh
#
# Output:
#   assets/fonts/STIXTwoMath-Regular.ttf
#   assets/fonts/OFL-STIX.txt
set -euo pipefail
cd "$(dirname "$0")/.."

TAG=v2.13b171
BASE="https://raw.githubusercontent.com/stipub/stixfonts/$TAG"
FONT_SHA256=${FONT_SHA256:-49b22767985171bce579307b21d98f4d35012fa39e316acc4ffcab8644dfb69b}
OFL_SHA256=${OFL_SHA256:-0c8825913b60d858aacdb33c4ca6660a7d64b0d6464702efbb19313f5765861a}

fetch() { # url out want_sha
    curl -fsSL "$1" -o "$2.part"
    got=$(sha256sum "$2.part" | cut -d' ' -f1)
    if [ -n "$3" ] && [ "$got" != "$3" ]; then
        rm -f "$2.part"
        echo "$1: SHA-256 $got, expected $3" >&2
        exit 1
    fi
    mv "$2.part" "$2"
    echo "$got"
}

font_sha=$(fetch "$BASE/fonts/static_ttf/STIXTwoMath-Regular.ttf" assets/fonts/STIXTwoMath-Regular.ttf "$FONT_SHA256")
ofl_sha=$(fetch "$BASE/OFL.txt" assets/fonts/OFL-STIX.txt "$OFL_SHA256")

python3 - "$TAG" "$font_sha" "$ofl_sha" <<'PY'
import datetime, re, sys
tag, font_sha, ofl_sha = sys.argv[1:]
today = datetime.date.today().isoformat()
entry = f'''[[asset]]
path = "assets/fonts/STIXTwoMath-Regular.ttf"
name = "STIX Two Math font"
source_url = "https://github.com/stipub/stixfonts/tree/{tag}"
version = "{tag}"
license = "OFL-1.1"
license_file = "assets/fonts/OFL-STIX.txt"
attribution = "STIX Two Math, © 2001-2021 The STIX Fonts Project Authors"
fetched = "{today}"
sha256 = "{font_sha}"
notes = "Unmodified. Fetched by scripts/fetch-stix.sh. Xenify Write: fallback for symbols (arrows, maths, Greek) and equations."

[[asset]]
path = "assets/fonts/OFL-STIX.txt"
name = "STIX Two licence"
source_url = "https://github.com/stipub/stixfonts/tree/{tag}"
version = "{tag}"
license = "OFL-1.1"
attribution = "Licence text for STIX Two"
fetched = "{today}"
sha256 = "{ofl_sha}"
'''
path = "assets/SOURCES.toml"
s = open(path, encoding="utf-8").read()
# Replace an earlier STIX entry, or add after the Inter licence.
s = re.sub(r'\[\[asset\]\]\npath = "assets/fonts/STIXTwoMath-Regular.ttf".*?(?=\n\[\[asset\]\]\nname)', '', s, flags=re.S)
s = re.sub(r'\[\[asset\]\]\npath = "assets/fonts/OFL-STIX.txt".*?(?=\n\[\[asset\]\])', '', s, flags=re.S)
anchor = 'sha256 = "5b9321a4298cfeb6b34354164a1c3afc3db114569984c502b9b35d988fd58c57"\n'
s = s.replace(anchor, anchor + "\n" + entry, 1)
open(path, "w", encoding="utf-8").write(s)
print("STIX Two Math", tag, font_sha)
PY
