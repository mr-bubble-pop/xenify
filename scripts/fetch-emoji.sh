#!/usr/bin/env bash
# Downloads the bundled emoji: Microsoft Fluent Emoji, flat style (MIT),
# from a pinned commit, into assets/emoji/, and records them in
# assets/SOURCES.toml (source, commit, licence, SHA-256). Run it again only
# to move to another commit (change COMMIT below).
#
#   scripts/fetch-emoji.sh
#
# Output:
#   assets/emoji/fluent-emoji.json.z   zlib-packed JSON: [{name, glyph, group, keywords, svg}]
#   assets/emoji/LICENSE               the set's MIT licence
set -euo pipefail
cd "$(dirname "$0")/.."

COMMIT=1ffb34c752ecf5d402f04cfb4b392c77f57c54bc
export COMMIT

python3 - <<'PY'
import concurrent.futures, datetime, hashlib, json, os, re, urllib.parse, urllib.request, zlib

commit = os.environ["COMMIT"]
repo = "microsoft/fluentui-emoji"
raw = f"https://raw.githubusercontent.com/{repo}/{commit}/"

def get(path, tries=4):
    url = raw + urllib.parse.quote(path)
    for i in range(tries):
        try:
            return urllib.request.urlopen(url, timeout=60).read()
        except Exception as e:
            err = e
    raise SystemExit(f"download failed: {url}: {err}")

tree = json.load(urllib.request.urlopen(f"https://api.github.com/repos/{repo}/git/trees/{commit}?recursive=1", timeout=120))
assert not tree.get("truncated"), "the tree listing was cut short"
paths = [e["path"] for e in tree["tree"]]
folders = sorted({p.split("/")[1] for p in paths if p.startswith("assets/") and p.count("/") >= 2})

# Every emoji of these groups…
WHOLE = {"Smileys & Emotion", "Activities", "Animals & Nature", "Food & Drink"}
# …and from the others, those whose name mentions one of these words.
WORDS = re.compile(r"\b(heart|star|sparkl|fire|rocket|check|cross mark|hundred|rainbow|sun|moon|cloud|lightning|high voltage|"
                   r"snowflake|gift|balloon|trophy|medal|crown|bell|musical|light bulb|money|camera|book|pencil|phone|laptop|"
                   r"clock|hourglass|key|lock|hand|thumbs|clapping|waving|ok|victory|flexed|eyes|folded|index pointing|"
                   r"raising|megaphone|loudspeaker|bomb|collision|dizzy|droplet|water wave|globe|house|chart|calendar|"
                   r"magnifying|gear|hammer|wrench|link|paperclip|pushpin|envelope|package|shopping|artist palette|"
                   r"paintbrush|crayon|film|clapper|headphone|microphone|guitar|brain|anatomical|people hugging|speech|"
                   r"thought|zzz|warning|no entry|prohibited|recycling|infinity|question|exclamation|plus|minus|"
                   r"multiply|divide|arrow|play button|pause|stop button|record|red circle|green circle|blue circle|"
                   r"yellow circle|purple circle|orange circle|white circle|black circle)\b", re.I)

def flat_svg(folder):
    pre = f"assets/{folder}/"
    cands = [p for p in paths if p.startswith(pre) and p.endswith(".svg") and "/Flat/" in p]
    default = [p for p in cands if "/Default/" in p]
    return (default or [p for p in cands if p.count("/") == 3] or cands or [None])[0]

def meta(folder):
    return json.loads(get(f"assets/{folder}/metadata.json"))

with concurrent.futures.ThreadPoolExecutor(24) as ex:
    metas = dict(zip(folders, ex.map(meta, folders)))

chosen = [f for f in folders
          if metas[f].get("group") in WHOLE
          or (metas[f].get("group") not in ("Flags",) and WORDS.search(metas[f].get("cldr", f)))]
chosen = [f for f in chosen if flat_svg(f)]

def svg(folder):
    return get(flat_svg(folder)).decode("utf-8")

with concurrent.futures.ThreadPoolExecutor(24) as ex:
    svgs = dict(zip(chosen, ex.map(svg, chosen)))

items = [{
    "name": metas[f].get("cldr", f),
    "glyph": metas[f].get("glyph", ""),
    "group": metas[f].get("group", ""),
    "keywords": metas[f].get("keywords", []),
    "svg": svgs[f],
} for f in chosen]
items.sort(key=lambda i: (i["group"], i["name"]))

os.makedirs("assets/emoji", exist_ok=True)
packed = zlib.compress(json.dumps(items, ensure_ascii=False, separators=(",", ":")).encode("utf-8"), 9)
open("assets/emoji/fluent-emoji.json.z", "wb").write(packed)
open("assets/emoji/LICENSE", "wb").write(get("LICENSE"))

sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
today = datetime.date.today().isoformat()
block = f'''# BEGIN fetch-emoji.sh (written by scripts/fetch-emoji.sh; don't edit by hand)
[[asset]]
path = "assets/emoji/fluent-emoji.json.z"
name = "Fluent Emoji (flat), {len(items)} emoji"
source_url = "https://github.com/{repo}"
version = "commit {commit}"
license = "MIT"
license_file = "assets/emoji/LICENSE"
attribution = "Fluent Emoji © Microsoft Corporation, MIT License"
fetched = "{today}"
sha256 = "{sha('assets/emoji/fluent-emoji.json.z')}"
notes = "The flat SVGs (default skin tone) of the groups Smileys & Emotion, Activities, Animals & Nature, Food & Drink and common symbols/objects, with their names, groups and keywords, packed into one zlib-compressed JSON file. The SVGs are unmodified."

[[asset]]
path = "assets/emoji/LICENSE"
name = "Fluent Emoji licence"
source_url = "https://github.com/{repo}"
version = "commit {commit}"
license = "MIT"
attribution = "Licence text for Fluent Emoji"
fetched = "{today}"
sha256 = "{sha('assets/emoji/LICENSE')}"
# END fetch-emoji.sh
'''
src = open("assets/SOURCES.toml").read()
src = re.sub(r"\n?# BEGIN fetch-emoji\.sh.*?# END fetch-emoji\.sh\n", "", src, flags=re.S)
open("assets/SOURCES.toml", "w").write(src.rstrip("\n") + "\n\n" + block)
print(f"wrote {len(items)} emoji ({len(packed) // 1024} KB packed)")
PY
