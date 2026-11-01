#!/usr/bin/env bash
# Downloads Open English WordNet (CC BY 4.0, derived from Princeton WordNet)
# from a pinned release checked by SHA-256, packs the parts Xenify Write's
# dictionary and thesaurus use into one compressed file, and records it in
# assets/SOURCES.toml. Run it again to move to another edition (change
# EDITION and SHA256).
#
#   scripts/fetch-oewn.sh
#
# Output:
#   assets/lexicon/oewn.txt.z   zlib-compressed text, read by
#                               crates/xenify-wp-check/src/lexicon.rs:
#     line 1     "OEWN <edition>"
#     "S <n>"    then n synsets: pos \t words (|) \t definition \t examples (|)
#                \t broader (synset numbers, ,) \t similar (,)
#     "E <n>"    then n entries: word \t pos \t pronunciation \t senses (,)
#                \t forms (|); a sense is a synset number, optionally
#                "=antonym/antonym"
#   assets/lexicon/LICENSE      OEWN's licence and the Princeton WordNet licence
set -euo pipefail
cd "$(dirname "$0")/.."

EDITION=2025
SHA256=7d749f6e2c39e6970e4997839dcf6e42fd281f3c2fae0171d2192bae8cfa4b51
export EDITION SHA256

python3 - <<'PY'
import datetime, hashlib, io, json, os, re, urllib.request, zipfile, zlib

edition, want_sha = os.environ["EDITION"], os.environ["SHA256"]
base = f"https://github.com/globalwordnet/english-wordnet/releases/download/{edition}-edition"
url = f"{base}/english-wordnet-{edition}-json.zip"
data = urllib.request.urlopen(url, timeout=300).read()
got = hashlib.sha256(data).hexdigest()
if got != want_sha:
    raise SystemExit(f"{url}: SHA-256 {got}, expected {want_sha}")
raw = f"https://raw.githubusercontent.com/globalwordnet/english-wordnet/{edition}-edition"
licence = b"".join(urllib.request.urlopen(f"{raw}/{name}", timeout=60).read() + b"\n\n"
                   for name in ("LICENSE.md", "WNDB_License.txt"))

synsets, entries = {}, {}
with zipfile.ZipFile(io.BytesIO(data)) as z:
    for name in sorted(z.namelist()):
        base_name = os.path.basename(name)
        if not base_name.endswith(".json") or base_name == "frames.json":
            continue
        part = json.loads(z.read(name))
        (entries if base_name.startswith("entries-") else synsets).update(part)

ids = sorted(synsets)
index = {s: i for i, s in enumerate(ids)}
clean = lambda s: re.sub(r"[\t\n|]+", " ", s).strip()
out = [f"OEWN {edition}", f"S {len(ids)}"]
for s in ids:
    y = synsets[s]
    pos = y["partOfSpeech"]
    words = "|".join(clean(m) for m in y.get("members", []))
    definition = clean("; ".join(y.get("definition", [])))
    examples = "|".join(clean(e if isinstance(e, str) else e.get("text", "")) for e in y.get("example", [])[:2])
    broader = ",".join(str(index[h]) for h in y.get("hypernym", []) + y.get("instance_hypernym", []) if h in index)
    similar = ",".join(str(index[h]) for h in y.get("similar", []) if h in index)
    out.append("\t".join([pos, words, definition, examples, broader, similar]))

rows = []
for word, by_pos in entries.items():
    for pos, e in by_pos.items():
        ipa = next((p["value"] for p in e.get("pronunciation", []) if p.get("variety") == "US"),
                   next((p["value"] for p in e.get("pronunciation", [])), ""))
        senses = []
        for sense in e.get("sense", []):
            if sense["synset"] not in index:
                continue
            item = str(index[sense["synset"]])
            ants = sorted({a.split("%")[0].replace("_", " ") for a in sense.get("antonym", [])})
            if ants:
                item += "=" + "/".join(clean(a) for a in ants)
            senses.append(item)
        if senses:
            rows.append("\t".join([clean(word), pos[0], clean(ipa), ",".join(senses), "|".join(clean(f) for f in e.get("form", []))]))
rows.sort()
out.append(f"E {len(rows)}")
out.extend(rows)
text = ("\n".join(out) + "\n").encode()

os.makedirs("assets/lexicon", exist_ok=True)
open("assets/lexicon/oewn.txt.z", "wb").write(zlib.compress(text, 9))
open("assets/lexicon/LICENSE", "wb").write(licence)

today = datetime.date.today().isoformat()
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
attribution = ("Open English WordNet (https://en-word.net), © the Open English WordNet team, CC BY 4.0; "
               "derived from Princeton WordNet 3.0, © 2006 Princeton University, WordNet License")
block = f'''# BEGIN fetch-oewn.sh (written by scripts/fetch-oewn.sh; don't edit by hand)
[[asset]]
path = "assets/lexicon/oewn.txt.z"
name = "Open English WordNet {edition}: dictionary and thesaurus ({len(ids)} meanings, {len(rows)} words)"
source_url = "https://en-word.net ({url})"
version = "{edition} edition"
license = "CC-BY-4.0"
license_file = "assets/lexicon/LICENSE"
attribution = "{attribution}"
fetched = "{today}"
sha256 = "{sha('assets/lexicon/oewn.txt.z')}"
notes = "Repacked by scripts/fetch-oewn.sh: definitions, up to two examples, words, broader and similar meanings, US pronunciations, antonyms and irregular forms, as zlib-compressed text. Other relations and the remaining examples are left out; the wording is unchanged."

[[asset]]
path = "assets/lexicon/LICENSE"
name = "Open English WordNet licence"
source_url = "https://github.com/globalwordnet/english-wordnet"
version = "{edition} edition"
license = "CC-BY-4.0"
attribution = "Licence text for Open English WordNet (CC BY 4.0) and Princeton WordNet (WordNet License)"
fetched = "{today}"
sha256 = "{sha('assets/lexicon/LICENSE')}"
# END fetch-oewn.sh
'''
src = open("assets/SOURCES.toml").read()
src = re.sub(r"\n?# BEGIN fetch-oewn\.sh.*?# END fetch-oewn\.sh\n", "", src, flags=re.S)
open("assets/SOURCES.toml", "w").write(src.rstrip("\n") + "\n\n" + block)
print(f"wrote {len(ids)} synsets, {len(rows)} entries, {len(text)} bytes -> {os.path.getsize('assets/lexicon/oewn.txt.z')} compressed")
PY
