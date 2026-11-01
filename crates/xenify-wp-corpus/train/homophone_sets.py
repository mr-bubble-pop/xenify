"""Sound-alike sets from the CMU Pronouncing Dictionary (BSD-2-Clause), for
the sound-alike checker: words with the same pronunciation, kept when they're
common in our public-domain books and have a WordNet meaning, minus UK/US
spelling pairs and words the hand-written sets already have.

    python3 homophone_sets.py cmudict.dict oewn.txt.z sets.txt OUT.txt BOOKS.jsonl...

Writes new lines in sets.txt's format (`word = meaning | word = meaning`).
Meanings are WordNet's (Open English WordNet, CC BY 4.0). Dev-only (MIT).
"""
import collections, json, os, re, sys, zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spotter_data import UK  # US → UK spellings

cmu_path, oewn_path, sets_path, out_path, *books = sys.argv[1:]

# How common each word is, in the books.
freq = collections.Counter()
for path in books:
    for line in open(path, encoding="utf-8"):
        try:
            t = json.loads(line).get("text", "")
        except ValueError:
            continue
        freq.update(w.lower() for w in re.findall(r"[A-Za-z]+", t))

# Same pronunciation → one group.
groups = collections.defaultdict(set)
for line in open(cmu_path, encoding="utf-8"):
    parts = line.split("#")[0].split()
    if not parts:
        continue
    # Each word's main pronunciation only (the "(2)" ones are other accents).
    w = parts[0]
    if re.fullmatch(r"[a-z]+", w):
        groups[" ".join(parts[1:])].add(w)

# WordNet meanings: per word, its most used sense's definition.
senses = collections.defaultdict(list)  # word → [(count, pos, gloss)]
for line in zlib.decompress(open(oewn_path, "rb").read()).decode().split("\n")[2:]:
    f = line.split("\t")
    if len(f) < 3:
        continue
    pos, words, gloss = f[0], f[1].split("|"), f[2]
    count = int(f[4].split(",")[0]) if len(f) > 4 and f[4].split(",")[0].isdigit() else 0
    for w in words:
        if re.fullmatch(r"[a-z]+", w):
            senses[w].append((count, pos, gloss))

POS = {"n": "noun", "v": "verb", "a": "adjective", "s": "adjective", "r": "adverb"}


def base(w):
    """The WordNet word `w` is a form of (itself, or minus -s/-es/-ed/-ing…)."""
    if w in senses:
        return w
    for cut, add in (("ies", "y"), ("es", ""), ("s", ""), ("ied", "y"), ("ed", ""), ("ed", "e"), ("ing", ""), ("ing", "e")):
        if w.endswith(cut) and w[: -len(cut)] + add in senses:
            return w[: -len(cut)] + add
    return None


def meaning(w):
    b = base(w)
    if b is None:
        return None
    count, pos, gloss = max(senses[b], key=lambda s: s[0])
    gloss = gloss.split(";")[0].strip()
    kind = POS.get(pos, "")
    if b == w:
        return f"{gloss} ({kind})" if kind else gloss
    return f"a form of \"{b}\": {gloss}"


def variant(a, b):
    """UK/US spellings of one word (colour/color, centre/center, travelled/traveled)."""
    if UK.get(a) == b or UK.get(b) == a:
        return True
    norm = lambda w: re.sub(r"our", "or", re.sub(r"tre$", "ter", re.sub(r"ll", "l", re.sub(r"is(e|ed|es|ing)$", r"iz\1", w))))
    return norm(a) == norm(b)


# Words the hand-written sets already cover (left to them).
have = set()
for line in open(sets_path, encoding="utf-8"):
    if line.startswith("#"):
        continue
    for part in line.split("|"):
        w = part.split("=")[0].strip().lower()
        if w:
            have.add(w)

# Very common short words: too risky to second-guess everywhere ("for", "by", "in").
FUNCTION = set(
    "a an and are as at be been by do for from had has have he her his i if in is it its me my no not of on or our "
    "so some than that the their them then there these they this to too two was we were what when where which who "
    "will with would you your oh owe ow".split()
)

out = []
for words in groups.values():
    keep = sorted(w for w in words if freq[w] >= 5 and w not in have and w not in FUNCTION and len(w) > 1)
    # Drop the second spelling of a UK/US pair.
    kept = []
    for w in sorted(keep, key=lambda w: -freq[w]):
        if not any(variant(w, k) for k in kept):
            kept.append(w)
    pairs = [(w, meaning(w)) for w in kept]
    pairs = [(w, m) for w, m in pairs if m]
    if len(pairs) < 2:
        continue
    # Inflections of the same word ("bowed", "bode"?): need different base words.
    if len({base(w) for w, _ in pairs}) < 2:
        continue
    out.append((sum(freq[w] for w, _ in pairs), " | ".join(f"{w} = {m.replace('|', '/')}" for w, m in pairs)))

out.sort(key=lambda x: -x[0])
with open(out_path, "w", encoding="utf-8") as f:
    for _, line in out:
        f.write(line + "\n")
print(f"{len(out)} new sets → {out_path}")
