#!/usr/bin/env bash
# Downloads the Computer Modern Unicode serif fonts (CMU Serif: roman, italic,
# bold, bold italic; SIL OFL 1.1) from CTAN, checks each by SHA-256, and
# records them in assets/SOURCES.toml. Xenify Write uses them for documents
# in the classic LaTeX look (the Scientific paper template). Unmodified, so
# the Reserved Font Name "Computer Modern Unicode fonts" is kept as is.
#
#   scripts/fetch-cmu.sh
#
# Output:
#   assets/fonts/cmunrm.otf cmunti.otf cmunbx.otf cmunbi.otf
#   assets/fonts/OFL-CMU.txt
set -euo pipefail
cd "$(dirname "$0")/.."

BASE="https://mirrors.ctan.org/fonts/cm-unicode"
declare -A SHA=(
    [cmunrm.otf]=5089693f81ed5b83435be7167bf2f93c2608ce92fb3a7e13f6be238a733c38f2
    [cmunti.otf]=fc38ce9ea06e49bda118a8be9f053791d043a547c248ae30c9e4fad956de3267
    [cmunbx.otf]=e956d496776533b0ca1b6e1acaeac896b81b81b908667cbf49840048ed16bd0c
    [cmunbi.otf]=690cc3c1511a98e79f5bde8a58868405160da1b7d99f3482324c6a915b496b02
)
OFL_SHA256=73273dffdefe2e5f1e138084d4a4b65b1c50df2ab0179f78484f31beefe30d84

fetch() { # url out want_sha
    curl -fsSL "$1" -o "$2.part"
    got=$(sha256sum "$2.part" | cut -d' ' -f1)
    if [ "$got" != "$3" ]; then
        rm -f "$2.part"
        echo "$1: SHA-256 $got, expected $3" >&2
        exit 1
    fi
    mv "$2.part" "$2"
}

for f in cmunrm.otf cmunti.otf cmunbx.otf cmunbi.otf; do
    fetch "$BASE/fonts/otf/$f" "assets/fonts/$f" "${SHA[$f]}"
done
fetch "$BASE/doc/OFL.txt" assets/fonts/OFL-CMU.txt "$OFL_SHA256"

python3 - "${SHA[cmunrm.otf]}" "${SHA[cmunti.otf]}" "${SHA[cmunbx.otf]}" "${SHA[cmunbi.otf]}" "$OFL_SHA256" <<'PY'
import datetime, re, sys
rm, ti, bx, bi, ofl = sys.argv[1:]
today = datetime.date.today().isoformat()
def font(file, style, sha):
    return f'''[[asset]]
path = "assets/fonts/{file}"
name = "CMU Serif {style} font (Computer Modern Unicode)"
source_url = "https://ctan.org/pkg/cm-unicode"
version = "0.7.0 (CTAN)"
license = "OFL-1.1"
license_file = "assets/fonts/OFL-CMU.txt"
attribution = "Computer Modern Unicode fonts, © 2003-2009 Andrey V. Panov; after Donald E. Knuth's Computer Modern and the authors listed in the licence"
fetched = "{today}"
sha256 = "{sha}"
notes = "Unmodified. Fetched by scripts/fetch-cmu.sh. Xenify Write: the classic LaTeX look (Scientific paper template)."
'''
entry = "".join(font(*a) + "\n" for a in [("cmunrm.otf", "Roman", rm), ("cmunti.otf", "Italic", ti), ("cmunbx.otf", "Bold", bx), ("cmunbi.otf", "Bold Italic", bi)])
entry += f'''[[asset]]
path = "assets/fonts/OFL-CMU.txt"
name = "Computer Modern Unicode licence"
source_url = "https://ctan.org/pkg/cm-unicode"
version = "0.7.0 (CTAN)"
license = "OFL-1.1"
attribution = "Licence text for the Computer Modern Unicode fonts"
fetched = "{today}"
sha256 = "{ofl}"
'''
path = "assets/SOURCES.toml"
s = open(path, encoding="utf-8").read()
# Replace earlier CMU entries, then add after the STIX licence.
s = re.sub(r'\[\[asset\]\]\npath = "assets/fonts/(cmun\w+\.otf|OFL-CMU\.txt)".*?\n\n(?=\[\[asset\]\])', '', s, flags=re.S)
anchor = 'sha256 = "0c8825913b60d858aacdb33c4ca6660a7d64b0d6464702efbb19313f5765861a"\n'
assert anchor in s, "STIX licence entry not found"
s = s.replace(anchor, anchor + "\n" + entry, 1)
open(path, "w", encoding="utf-8").write(s)
print("CMU Serif fetched", today)
PY
