"""Builds a test set of real human grammar mistakes from the WikEd Error Corpus
(Wikipedia edits up to 2014, so before AI writing tools).

    python3 wiked_extract.py wiked.tok.err wiked.tok.cor harper-dictionary.dict OUT.jsonl [per_kind] [EXCLUDE.jsonl ...]

Keeps edits to one full sentence that change one or two words, where both
versions are dictionary words and the change is grammatical (articles,
prepositions, agreement, verb forms, pronouns, noun number, word order,
missing/extra small words). Spelling, punctuation, capitals, numbers, names
and content changes are dropped. Writes corpus items (text = before,
corrected = after). Edits in the EXCLUDE files (by corpus line or by sentence)
are left out, so a training set never shares sentences with the test set.

Wikipedia text is CC BY-SA. The edits themselves are never shipped; a model
trained on them is released saying it was derived from Wikipedia edits
(the user's decision, 2026-10-12). Dev-only tool (MIT).
"""
import html, json, random, re, sys
from difflib import SequenceMatcher

err_path, cor_path, dict_path, out_path = sys.argv[1:5]
per_kind = int(sys.argv[5]) if len(sys.argv) > 5 else 400
exclude_lines, exclude_texts = set(), set()
for path in sys.argv[6:]:
    for line in open(path, encoding="utf-8"):
        it = json.loads(line)
        exclude_lines.add(int(it["id"].rsplit(":", 1)[1]))
        exclude_texts.update((it["text"], it["corrected"]))

words = set()
for line in open(dict_path, encoding="utf-8"):
    w = line.split("/")[0].strip()
    if w and not w[0].isdigit():
        words.add(w.lower())

ARTICLES = {"a", "an", "the"}
PREPS = {"in", "on", "at", "to", "from", "with", "of", "for", "by", "into", "onto", "about", "over", "under",
         "through", "during", "since", "until", "between", "among", "towards", "toward", "upon", "within", "without"}
BE_HAVE_DO = {"is", "are", "was", "were", "be", "been", "being", "am", "has", "have", "had", "do", "does", "did"}
PRONOUNS = {"i", "me", "he", "him", "she", "her", "we", "us", "they", "them", "who", "whom", "it", "its",
            "his", "their", "theirs", "our", "my", "your", "which", "that", "this", "these", "those"}
SMALL = ARTICLES | PREPS | BE_HAVE_DO | {"to", "and", "or", "not", "it", "there", "that", "as", "than", "will", "would", "can", "could"}


def detok(s):
    s = html.unescape(s.replace("&apos;", "'").replace("&quot;", '"'))
    s = re.sub(r" ([.,;:!?%)\]}])", r"\1", s)
    s = re.sub(r"([(\[{$]) ", r"\1", s)
    s = re.sub(r" (n't|'s|'re|'ve|'ll|'d|'m|'t)\b", r"\1", s)  # Moses splits "doesn't" as "doesn 't"
    s = re.sub(r'" (.*?) "', r'"\1"', s)
    return s


def kind(a, b):
    """The kind of grammatical change from word list a to b (lower-case), or None."""
    la, lb = [w.lower() for w in a], [w.lower() for w in b]
    if sorted(la) == sorted(lb) and la != lb and len(la) >= 2:
        return "word_order"
    if not la and len(lb) == 1 or not lb and len(la) == 1:
        w = (lb or la)[0]
        miss = not la
        if w in ARTICLES:
            return "article_missing" if miss else "article_extra"
        if w in PREPS:
            return "preposition_missing" if miss else "preposition_extra"
        if w in SMALL:
            return "missing_word" if miss else "extra_word"
        return None
    if len(la) == 1 and len(lb) == 1:
        x, y = la[0], lb[0]
        if x in ARTICLES and y in ARTICLES:
            return "article_wrong"
        if x in PREPS and y in PREPS:
            return "preposition_wrong"
        if x in BE_HAVE_DO and y in BE_HAVE_DO:
            return "verb_agreement"
        if x in PRONOUNS and y in PRONOUNS:
            return "pronoun"
        if x.rstrip("s") == y.rstrip("s") and x != y and len(x) > 3:
            return "number_or_agreement"  # report/reports, need/needs
        if SequenceMatcher(None, x, y).ratio() > 0.6 and x[:3] == y[:3] and len(x) > 2:
            # went/gone, taken/took, writing/written: forms of one word (both real words).
            return "verb_form" if x not in ("then", "than") else "confused_words"
        if {x, y} in ({"then", "than"}, {"its", "it's"}, {"your", "you're"}, {"their", "there"}, {"affect", "effect"}, {"lose", "loose"}):
            return "confused_words"
    return None


def main():
    random.seed(20141011)
    pools, seen = {}, {}
    with open(err_path, encoding="utf-8") as fe, open(cor_path, encoding="utf-8") as fc:
        for n, (e, c) in enumerate(zip(fe, fc)):
            e, c = e.strip(), c.strip()
            if e == c:
                continue
            ew, cw = e.split(" "), c.split(" ")
            if not (8 <= len(cw) <= 40) or not cw[0][:1].isupper() or cw[-1] not in (".", "!", "?"):
                continue
            ops = [op for op in SequenceMatcher(None, ew, cw, autojunk=False).get_opcodes() if op[0] != "equal"]
            if len(ops) != 1:
                continue
            tag, i1, i2, j1, j2 = ops[0]
            a, b = ew[i1:i2], cw[j1:j2]
            if len(a) > 3 or len(b) > 3:
                continue
            changed = a + b
            # Only real words, no names (capitals mid-sentence), numbers or punctuation.
            if any(not w.isalpha() and w.lower() not in ("it's", "you're") for w in changed):
                continue
            if any(w.lower() not in words and w.lower() not in ("it's", "you're") for w in changed):
                continue
            if any(w[0].isupper() for w, pos in zip(a, range(i1, i2)) if pos > 0) or any(w[0].isupper() for w, pos in zip(b, range(j1, j2)) if pos > 0):
                continue
            k = kind(a, b)
            if k is None:
                continue
            # The changed words with one neighbour each side, as in the corpus format.
            lo, hi = max(i1 - 1, 0), min(i2 + 1, len(ew))
            clo, chi = max(j1 - 1, 0), min(j2 + 1, len(cw))
            text, corrected = detok(e), detok(c)
            if n in exclude_lines or text in exclude_texts or corrected in exclude_texts:
                continue
            wrong, fix = detok(" ".join(ew[lo:hi])), detok(" ".join(cw[clo:chi]))
            if wrong not in text:
                continue
            pool = pools.setdefault(k, [])
            seen[k] = seen.get(k, 0) + 1
            # Reservoir sample per kind, so every eligible edit has the same chance.
            item = (text, corrected, wrong, fix, n)
            if len(pool) < per_kind:
                pool.append(item)
            else:
                r = random.randrange(seen[k])
                if r < per_kind:
                    pool[r] = item
    with open(out_path, "w", encoding="utf-8") as f:
        for k, pool in sorted(pools.items()):
            for text, corrected, wrong, fix, n in pool:
                f.write(json.dumps({
                    "id": f"wiked:{k}:{n}", "job": f"wiked:{k}", "setting": "wikipedia", "register": "standard",
                    "error_type": k, "text": text, "corrected": corrected,
                    "mistakes": [{"wrong": wrong, "fix": fix}], "generator": "wiked-v1.0",
                }, ensure_ascii=False) + "\n")
            print(f"{k:22} {len(pool)} of {seen[k]}")


if __name__ == "__main__":
    main()
