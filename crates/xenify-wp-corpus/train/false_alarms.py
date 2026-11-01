"""Sorts false alarms (flags on correct text) into groups, so names, old English
and spelling-dialect mixes are reported apart from real checker mistakes.

    python3 false_alarms.py EVAL_DIR NAME [NAME ...]

Reads EVAL_DIR/NAME-SET.false-alarms.jsonl (written by `xenify-wp-corpus score`)
and EVAL_DIR/NAME-SET.json for the passage counts. Dev-only tool (MIT).
"""
import json, os, re, sys
from collections import Counter, defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spotter_data import UK  # US → UK/Australian spellings

US = {v: k for k, v in UK.items()}
# UK spellings the list above doesn't cover.
for w in "centred storey storeys labourer labourers acknowledgement draught draughts whisky grey kerb tyre".split():
    US[w] = w
UK_ENDINGS = re.compile(r"(our|ours|oured|ouring|ourite|ourites|ourhood|ourable|tre|tres|lled|lling|ller|llers|ise|ised|ises|ising|isation|isations|ence|gramme|ogue|ogues)$")
US_ENDINGS = re.compile(r"(or|ors|ored|oring|orite|orhood|orable|ter|ters|led|ling|ize|ized|izes|izing|ization|izations|ense|log)$")
# Old spellings of modern words ("some one", "none the less"). DisjointPrefixes is not here:
# "non-existent" and "co-educational" are correct modern English.
OLD_RULES = {"Someone", "Everyone", "Nonetheless", "Worthwhile", "AWhile"}
DIALECT = re.compile(r"(^|[\s'’])(tha|th|o|an|ye|yer|ter|ain|em|n|ol|d|s)['’]|['’](un|em|tis|twas)\b", re.I)

SETS = ["wiki-test", "uk-books", "us-books", "ai-test"]
GROUPS = ["name or title", "UK/US spelling", "old or dialect English", "word not in dictionary", "grammar rule", "spotter (model)"]
REAL = {"grammar rule", "spotter (model)"}


def group(fa, us_mode):
    rule, word, text = fa["rule"], fa["flagged"].strip(), fa["text"]
    # The scorer gives byte offsets; turn them into character offsets.
    raw = text.encode("utf-8")
    fa = dict(fa, start=len(raw[: fa["start"]].decode("utf-8", "ignore")), end=len(raw[: fa["end"]].decode("utf-8", "ignore")))
    around = text[max(0, fa["start"] - 3) : fa["end"] + 3]
    if rule.startswith("Spotter:"):
        return "spotter (model)"
    # Dialect "an'" (and), "o'" (of): an apostrophe right after the flagged word.
    dialect_cut = text[fa["end"] : fa["end"] + 1] in ("'", "\u2019") and len(word) <= 3
    if rule in OLD_RULES or DIALECT.search(around) or dialect_cut:
        return "old or dialect English"
    if rule in ("SpellCheck", "OrthographicConsistency", "SplitWords"):
        if any(c.isupper() for c in word):
            return "name or title"
        w = word.lower()
        if (us_mode and (w in US or UK_ENDINGS.search(w))) or (not us_mode and (w in UK or US_ENDINGS.search(w))):
            return "UK/US spelling"
        if "'" in w or "’" in w:
            return "old or dialect English"
        return "word not in dictionary"
    return "grammar rule"


def main():
    d, names = sys.argv[1], sys.argv[2:]
    for name in names:
        print(f"\n## {name}\n")
        print("| Set | Passages | Any flag | " + " | ".join(GROUPS) + " | **Real false alarms** |")
        print("|---|---:|---:|" + "---:|" * (len(GROUPS) + 1))
        examples = defaultdict(Counter)
        for s in SETS:
            path = f"{d}/{name}-{s}.false-alarms.jsonl"
            if not os.path.exists(path):
                continue
            # Correct standard passages, as the scoreboard counted them.
            n = json.load(open(f"{d}/{name}-{s}.json"))["clean_standard"]
            us_mode = s != "uk-books"
            by_passage = defaultdict(set)
            for line in open(path, encoding="utf-8"):
                fa = json.loads(line)
                g = group(fa, us_mode)
                by_passage[fa["text"]].add(g)
                examples[g][(fa["rule"], fa["flagged"].strip(), fa["text"])] += 1
            counts = Counter(g for gs in by_passage.values() for g in gs)
            real = sum(1 for gs in by_passage.values() if gs & REAL)
            cells = [f"{100 * counts[g] / n:.1f}%" for g in GROUPS]
            print(f"| {s} | {n} | {100 * len(by_passage) / n:.1f}% | " + " | ".join(cells) + f" | **{100 * real / n:.1f}%** |")
        if "--examples" in os.environ.get("FA_OPTS", ""):
            for g in GROUPS:
                print(f"\n### {g}\n")
                for (rule, word, text), c in examples[g].most_common(12):
                    print(f"- {rule} `{word}`: {text[:150]}")


if __name__ == "__main__":
    main()
