"""Adds sentence-breaker passages to a spotter data folder made by spotter_data.py.

    python3 breaker_data.py DATA_DIR broken1.jsonl [broken2.jsonl ...]

Each broken passage is a mistake row (span = the damaged words, code mapped
from the breaker kind to the spotter's codes) and its original is a correct
row. Kinds the rules handle (repeated words) are skipped. Dev-only tool (MIT,
part of Xenify).
"""
import json, random, sys

# Breaker kind (ERRANT-style code, see src/breaker.rs) → spotter code.
CODE = {
    "M:DET": "article_missing",
    "U:DET": "article_wrong",
    "R:DET": "article_wrong",
    "M:PREP": "preposition",
    "U:PREP": "preposition",
    "R:PREP": "preposition",
    "M:PRON": "missing_word",
    "R:PRON": "pronoun_case",
    "M:VERB": "missing_word",
    "U:VERB": "verb_form",
    "M:PART": "missing_word",
    "U:PART": "verb_pattern",
    "R:VERB:FORM": "verb_form",
    "R:VERB:TENSE": "tense_shift",
    "R:VERB:INFL": "verb_form",
    "R:NOUN:NUM": "plural_form",
    "R:NOUN:INFL": "plural_form",
    "R:ADJ:FORM": "comparative",
    "WO": "word_order",
    "U:PUNCT": "hyphen",
    "M:PUNCT": "hyphen",
    "GARBLED": "garbled",
}


def code_for(kind, wrong):
    head = kind.split(" ")[0]
    if head == "R:VERB:SVA":
        return "subject_verb_agreement" if "what" in kind else "pronoun_verb_agreement"
    if head == "R:ORTH":
        w = wrong.lower().replace("’", "'")
        return "its_its" if "its" in w or "it's" in w else "homophone"
    return CODE.get(head)  # None: left to the rules (repeated words, adverbs)


def main():
    data, inputs = sys.argv[1], sys.argv[2:]
    labels = json.load(open(f"{data}/labels.json"))
    # Codes only the breaker makes (not in the dev/test sets) are added at the end.
    for c in sorted(set(CODE.values()) - set(labels)):
        labels.append(c)
    json.dump(labels, open(f"{data}/labels.json", "w"), indent=1)
    held = {json.loads(l)["text"] for name in ("dev", "test") for l in open(f"{data}/{name}.jsonl", encoding="utf-8")}
    rows, clean, counts = [], set(), {}
    for path in inputs:
        for line in open(path, encoding="utf-8"):
            it = json.loads(line)
            text, wrong = it["text"], it["mistakes"][0]["wrong"]
            code = code_for(it["error_type"], wrong)
            if it.get("corrected") and it["corrected"] not in held:
                clean.add(it["corrected"])
            if code is None or text in held:
                continue
            assert code in labels, code
            i = text.find(wrong)
            # Not found, or found twice (the span would be ambiguous).
            if i < 0 or text.find(wrong, i + 1) >= 0:
                continue
            rows.append({"text": text, "spans": [[i, i + len(wrong), code]]})
            counts[code] = counts.get(code, 0) + 1
    rows += [{"text": t, "spans": []} for t in sorted(clean)]
    random.seed(20261011)
    train = [json.loads(l) for l in open(f"{data}/train.jsonl", encoding="utf-8")] + rows
    random.shuffle(train)
    with open(f"{data}/train.jsonl", "w", encoding="utf-8") as f:
        for r in train:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"added {len(rows) - len(clean)} broken + {len(clean)} correct rows; train now {len(train)}")
    print(json.dumps(counts, indent=0))


if __name__ == "__main__":
    main()
