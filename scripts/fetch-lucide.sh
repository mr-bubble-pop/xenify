#!/usr/bin/env bash
# Downloads the Lucide icons Xenify Write uses (ISC; the few derived from
# Feather are MIT), from a pinned npm release checked by SHA-256, into
# assets/icons/, and records them in assets/SOURCES.toml. Run it again to
# move to another release (change VERSION and SHA256) or after adding a name
# to ICONS below (and to crates/xenify-wp-ui/src/icons.rs).
#
#   scripts/fetch-lucide.sh
#
# Output:
#   assets/icons/lucide.json   {name: [[element, {attribute: value}]…]} for the icons below
#   assets/icons/LICENSE       Lucide's licence (ISC, with Feather's MIT notice)
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=1.55.0
SHA256=809e2e759aeecd05ef3abc231395a0ccb6a58169823cbd26a425960b4d282797
export VERSION SHA256

python3 - <<'PY'
import datetime, hashlib, io, json, os, re, tarfile, urllib.request

version, want_sha = os.environ["VERSION"], os.environ["SHA256"]
url = f"https://registry.npmjs.org/lucide-static/-/lucide-static-{version}.tgz"
data = urllib.request.urlopen(url, timeout=120).read()
got = hashlib.sha256(data).hexdigest()
if got != want_sha:
    raise SystemExit(f"{url}: SHA-256 {got}, expected {want_sha}")

ICONS = """
undo-2 redo-2 bold italic underline strikethrough superscript subscript baseline highlighter
text-align-start text-align-center text-align-end text-align-justify list list-ordered list-checks
list-indent-increase list-indent-decrease image table link message-square-plus message-square
message-square-reply circle-check file-plus folder-open save save-all printer file-down file-cog
clock log-out scissors copy clipboard-paste clipboard-type search replace sun moon monitor ruler
columns-2 columns-3 panel-top panel-bottom panel-right heading pilcrow quote minus remove-formatting
zoom-in zoom-out file-text list-tree footprints library book-open bookmark spell-check sparkles mail
settings circle-question-mark info x check plus trash table-cells-merge table-cells-split
between-horizontal-start between-vertical-start paint-bucket square-dashed brush text-wrap crop
pen-line file-diff eye user layout-template separator-horizontal type a-large-small chevrons-up-down
arrow-up-narrow-wide arrow-down-wide-narrow hash square-check rows-2 text-cursor-input chevron-down
panel-bottom-close panel-right-close arrow-down-a-z arrow-down-z-a table-properties pipette
arrow-up house face-slightly-smiling omega chevron-right
""".split()

with tarfile.open(fileobj=io.BytesIO(data)) as t:
    nodes = json.load(t.extractfile("package/icon-nodes.json"))
    licence = t.extractfile("package/LICENSE").read()
missing = [n for n in ICONS if n not in nodes]
if missing:
    raise SystemExit(f"not in Lucide {version}: {missing}")
os.makedirs("assets/icons", exist_ok=True)
out = {n: nodes[n] for n in sorted(set(ICONS))}
open("assets/icons/lucide.json", "w").write(json.dumps(out, separators=(",", ":"), sort_keys=True) + "\n")
open("assets/icons/LICENSE", "wb").write(licence)

today = datetime.date.today().isoformat()
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
block = f'''# BEGIN fetch-lucide.sh (written by scripts/fetch-lucide.sh; don't edit by hand)
[[asset]]
path = "assets/icons/lucide.json"
name = "Lucide icons, {len(out)} icons (Xenify Write menus and toolbar)"
source_url = "https://lucide.dev ({url})"
version = "lucide-static {version}"
license = "ISC"
license_file = "assets/icons/LICENSE"
attribution = "Lucide icons © Lucide Icons and Contributors (ISC); icons derived from Feather © Cole Bemis (MIT)"
fetched = "{today}"
sha256 = "{sha('assets/icons/lucide.json')}"
notes = "The shapes (paths, lines, circles, rectangles) of the icons Write uses, copied unmodified from the package's icon-nodes.json into one file. Drawn in the app in the theme's text colour."

[[asset]]
path = "assets/icons/LICENSE"
name = "Lucide icons licence"
source_url = "https://lucide.dev"
version = "lucide-static {version}"
license = "ISC"
attribution = "Licence text for Lucide (ISC) and Feather (MIT)"
fetched = "{today}"
sha256 = "{sha('assets/icons/LICENSE')}"
# END fetch-lucide.sh
'''
src = open("assets/SOURCES.toml").read()
src = re.sub(r"\n?# BEGIN fetch-lucide\.sh.*?# END fetch-lucide\.sh\n", "", src, flags=re.S)
open("assets/SOURCES.toml", "w").write(src.rstrip("\n") + "\n\n" + block)
print(f"wrote {len(out)} icons")
PY
