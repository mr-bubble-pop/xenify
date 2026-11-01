"""Adds verified WikEd edits (real Wikipedia mistakes, up to 2014) to a spotter
data folder made by spotter_data.py.

    python3 wiked_data.py DATA_DIR wiked-train.jsonl [repeat]

Each edit gives a mistake row (span = the wrong words, code mapped from the
edit kind to the spotter's codes) and its corrected sentence as a correct
row. `repeat` (default 1) adds the mistake rows that many times, to weigh
real mistakes against the much larger made-up set. Items whose text is in
the dev or test sets are skipped.

Wikipedia text is CC BY-SA: the data folder stays local, and a model trained
on it is released saying it was derived from Wikipedia edits. Dev-only tool
(MIT, part of Xenify).
"""
import json, random, sys

BE_HAVE_DO = {"is", "are", "was", "were", "be", "been", "being", "am", "has", "have", "had", "do", "does", "did"}
CASE = [{"i", "me"}, {"he", "him"}, {"she", "her"}, {"we", "us"}, {"they", "them"}, {"who", "whom"}]
SIMPLE = {
    "article_missing": "article_missing",
    "article_extra": "article_wrong",
    "article_wrong": "article_wrong",
    "preposition_missing": "preposition",
    "preposition_extra": "preposition",
    "preposition_wrong": "preposition",
    "missing_word": "missing_word",
    "verb_agreement": "subject_verb_agreement",
    "verb_form": "verb_form",
    "word_order": "word_order",
    "number_or_agreement": "plural_form",
}


def changed(wrong, fix):
    """The words that differ between the wrong and fixed spans (lower-case)."""
    a, b = wrong.lower().split(), fix.lower().split()
    return [w for w in a if w not in b] + [w for w in b if w not in a]


def code_for(kind, wrong, fix):
    if kind in SIMPLE:
        return SIMPLE[kind]
    words = changed(wrong, fix)
    if kind == "extra_word":
        # "had left" → "left": a verb form; other extra small words are left out.
        return "verb_form" if any(w in BE_HAVE_DO for w in words) else None
    if kind == "pronoun":
        return "pronoun_case" if set(words) in CASE else "pronoun_antecedent"
    if kind == "confused_words":
        return "its_its" if any(w in ("its", "it's") for w in words) else "confused_words"
    return None


def main():
    data, path = sys.argv[1], sys.argv[2]
    repeat = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    labels = json.load(open(f"{data}/labels.json"))
    held = {json.loads(l)["text"] for name in ("dev", "test") for l in open(f"{data}/{name}.jsonl", encoding="utf-8")}
    rows, clean, counts = [], set(), {}
    for line in open(path, encoding="utf-8"):
        it = json.loads(line)
        m = it["mistakes"][0]
        code = code_for(it["error_type"], m["wrong"], m["fix"])
        if code is None or it["text"] in held:
            continue
        assert code in labels, code
        i = it["text"].find(m["wrong"])
        if i < 0:
            continue
        rows += [{"text": it["text"], "spans": [[i, i + len(m["wrong"]), code]]}] * repeat
        counts[code] = counts.get(code, 0) + 1
        if it["corrected"] not in held:
            clean.add(it["corrected"])
    rows += [{"text": t, "spans": []} for t in sorted(clean)]
    random.seed(20261012)
    train = [json.loads(l) for l in open(f"{data}/train.jsonl", encoding="utf-8")] + rows
    random.shuffle(train)
    with open(f"{data}/train.jsonl", "w", encoding="utf-8") as f:
        for r in train:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"added {sum(counts.values())} mistakes (x{repeat}) + {len(clean)} correct rows; train now {len(train)}")
    print(json.dumps(counts, indent=0))


if __name__ == "__main__":
    main()
