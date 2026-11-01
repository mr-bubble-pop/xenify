"""Compares scoreboards from eval_spotter.sh: caught and false alarms per set and model.

    python3 compare.py EVAL_DIR NAME [NAME ...]
"""
import json, os, sys

d, names = sys.argv[1], sys.argv[2:]
sets = ["ai-test", "wiki-test", "uk-books", "us-books", "ai-broken", "uk-broken", "us-broken"]


def pct(a, b):
    return f"{100 * a / b:.1f}%" if b else "–"


print("| Set | " + " | ".join(f"{n} caught | {n} fixed | {n} false alarms" for n in names) + " |")
print("|---|" + "---:|---:|---:|" * len(names))
for s in sets:
    cells = []
    for n in names:
        p = f"{d}/{n}-{s}.json"
        if not os.path.exists(p):
            cells += ["…", "…", "…"]
            continue
        j = json.load(open(p))
        # "fixed": a suggestion, applied, gives exactly the corrected text.
        cells += [
            pct(j["caught"], j["mistakes"]),
            pct(j["fixed"], j["mistakes"]),
            pct(j["clean_standard_flagged"], j["clean_standard"]),
        ]
    print(f"| {s} | " + " | ".join(cells) + " |")
