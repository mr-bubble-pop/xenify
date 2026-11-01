"""Curates candidate sound-alike sets (from homophone_sets.py) with an AI
checker: keeps only words people really mix up in modern writing, drops
spelling variants, accent-only pairs, names and oddities, and writes short
plain meanings. Through OpenRouter with a zero-data-retention model; the
key is read from .env and never printed.

    python3 curate_homophones.py CANDIDATES.txt OUT.txt [MODEL]

Dev-only tool (MIT, part of Xenify).
"""
import json, os, pathlib, sys, urllib.request

cand_path, out_path = sys.argv[1], sys.argv[2]
model = sys.argv[3] if len(sys.argv) > 3 else "qwen/qwen3-235b-a22b-2507"
# "strict": a second pass over already-curated sets (checks pronunciation and rarity again).
strict = len(sys.argv) > 4 and sys.argv[4] == "strict"


def api_key():
    for d in [pathlib.Path.cwd(), *pathlib.Path.cwd().parents]:
        env = d / ".env"
        if env.exists():
            for line in env.read_text().splitlines():
                if line.startswith("OPENROUTER_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"')
    return os.environ.get("OPENROUTER_API_KEY", "")


KEY = api_key()
PROMPT = """You are curating sets of English sound-alike words for a spelling and grammar checker. Writers sometimes type one word of a set when they mean another (e.g. "bare with me" for "bear with me").

For each numbered set below, decide which words to keep. Keep a word only if ALL hold:
- it's a word people use in everyday modern writing (not a name, letter, musical syllable, abbreviation, archaic or very rare word);
- it sounds the same as at least one other kept word in standard British and American speech (not only in some accents);
- it isn't just another spelling of the same word with the same meaning (drop whisky/whiskey, axe/ax, colour/color pairs);
- a real writer could plausibly type it by mistake for another kept word.
Keep the set only if at least two words remain. For each kept word, write a short plain-English meaning (under 12 words) of its usual sense(s), the way a learner's dictionary would, e.g. "a male deer" or "to put up with; to carry".

Answer with only a JSON array, one object per numbered set: {"n": number, "keep": true/false, "words": [{"word": "...", "meaning": "..."}]}.

Sets:
"""

STRICT = """You are checking sets of English sound-alike words for a spelling checker. For each numbered set, check each word strictly:
- Is it pronounced EXACTLY the same as another word of the set in ordinary British and American speech? (weary/wiry and harry/hairy are NOT; heart/hart and died/dyed are.)
- Is it a word in common modern use? Drop rare, archaic, literary or technical words (lade, wight, mien, hansom, rood) and drop interjections.
- Is the meaning given right and in plain English? Fix it if not (frank = honest, open; franc = a former French or Swiss coin).
Keep the set only if at least two words remain.

Answer with only a JSON array, one object per numbered set: {"n": number, "keep": true/false, "words": [{"word": "...", "meaning": "..."}]}.

Sets:
"""
lines = [l.strip() for l in open(cand_path, encoding="utf-8") if l.strip()]
out = []
spent = 0.0
for start in range(0, len(lines), 25):
    batch = lines[start : start + 25]
    if strict:
        listed = "\n".join(f"{i + 1}. {l}" for i, l in enumerate(batch))
    else:
        listed = "\n".join(f"{i + 1}. " + " | ".join(p.split("=")[0].strip() for p in l.split("|")) for i, l in enumerate(batch))
    body = {
        "model": model,
        "messages": [{"role": "user", "content": (STRICT if strict else PROMPT) + listed}],
        "temperature": 0,
        "usage": {"include": True},
        "provider": {"data_collection": "deny", "zdr": True},
    }
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"},
    )
    resp = json.load(urllib.request.urlopen(req, timeout=300))
    spent += resp.get("usage", {}).get("cost", 0) or 0
    text = (resp["choices"][0]["message"].get("content") or "").strip()
    if "[" not in text:
        print(f"no answer for sets {start + 1}-{start + len(batch)}; skipped", file=sys.stderr)
        continue
    text = text[text.find("[") : text.rfind("]") + 1]
    for item in json.loads(text):
        # Words the answer gives in full (anything malformed is left out).
        words = [w for w in item.get("words", []) if isinstance(w, dict) and w.get("word") and w.get("meaning")]
        if item.get("keep") and len(words) >= 2:
            out.append(" | ".join(f"{w['word'].strip().lower()} = {w['meaning'].strip().replace('|', '/')}" for w in words))
    print(f"{start + len(batch)}/{len(lines)} sets read, {len(out)} kept, ${spent:.4f}", file=sys.stderr)

with open(out_path, "w", encoding="utf-8") as f:
    f.write("\n".join(out) + "\n")
print(f"{len(out)} sets kept → {out_path} (${spent:.4f})")
