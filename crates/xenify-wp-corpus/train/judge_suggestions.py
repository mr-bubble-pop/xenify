"""Asks an AI reviewer whether each checker suggestion makes sense, so silly
ones ("dogging", "thes", "pie" → "pi") can be found and fixed. Through
OpenRouter with a zero-data-retention model; the key is read from .env and
never printed. Only test sentences are sent (public books, Wikipedia,
generated text), never a user's writing.

    python3 judge_suggestions.py TRIPLES.json OUT.json [MODEL]

TRIPLES.json: [{sentence, flagged, suggestions, rule, ...}]. Each gets
"verdict" (good / ok / bad) and "why". Dev-only tool (MIT, part of Xenify).
"""
import json, os, pathlib, sys, urllib.request

src, out_path = sys.argv[1], sys.argv[2]
model = sys.argv[3] if len(sys.argv) > 3 else "qwen/qwen3-235b-a22b-2507"


def api_key():
    for d in [pathlib.Path.cwd(), *pathlib.Path.cwd().parents]:
        env = d / ".env"
        if env.exists():
            for line in env.read_text().splitlines():
                if line.startswith("OPENROUTER_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"')
    return os.environ.get("OPENROUTER_API_KEY", "")


KEY = api_key()
PROMPT = """You are checking the suggestions of an English spelling and grammar checker. For each numbered item you get a sentence, the words the checker marked, and its suggested replacement(s) for those words.

Rate the FIRST suggestion:
- "good": the marked words really are a mistake and the suggestion fixes it sensibly.
- "ok": the original was acceptable (or a style choice), but the suggestion is a reasonable alternative a careful editor might offer.
- "bad": the suggestion is nonsense, not a real word, a rare or unrelated word, changes the meaning, or makes the sentence worse.

Answer with only a JSON array, one object per item: {"n": number, "verdict": "good"|"ok"|"bad", "why": "under 12 words"}.

Items:
"""

items = json.load(open(src, encoding="utf-8"))
spent = 0.0
for start in range(0, len(items), 20):
    batch = items[start : start + 20]
    listed = "\n".join(
        f'{i + 1}. Sentence: "{it["sentence"][:400]}"\n   Marked: "{it["flagged"]}"\n   Suggestions: {" | ".join(it["suggestions"])}'
        for i, it in enumerate(batch)
    )
    body = {
        "model": model,
        "messages": [{"role": "user", "content": PROMPT + listed}],
        "temperature": 0,
        "usage": {"include": True},
        "provider": {"data_collection": "deny", "zdr": True},
    }
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"},
    )
    try:
        resp = json.load(urllib.request.urlopen(req, timeout=300))
    except Exception as e:  # one failed batch shouldn't lose the rest
        print(f"batch {start}: {e}", file=sys.stderr)
        continue
    spent += resp.get("usage", {}).get("cost", 0) or 0
    text = (resp["choices"][0]["message"].get("content") or "").strip()
    if "[" not in text:
        continue
    try:
        verdicts = json.loads(text[text.find("[") : text.rfind("]") + 1])
    except ValueError:
        continue
    for v in verdicts:
        n = v.get("n", 0) - 1
        if 0 <= n < len(batch):
            batch[n]["verdict"] = v.get("verdict")
            batch[n]["why"] = v.get("why", "")
    print(f"{start + len(batch)}/{len(items)} judged, ${spent:.4f}", file=sys.stderr)

json.dump(items, open(out_path, "w", encoding="utf-8"), ensure_ascii=False, indent=0)
print(f"judged → {out_path} (${spent:.4f})")
