"""Coverage chart: merges scoreboard .json files into one table per mistake kind.

    python3 coverage.py OUT.md LABEL=board.json [LABEL=board.json ...]
"""
import json, sys

out, boards = sys.argv[1], [a.split("=", 1) for a in sys.argv[2:]]
data = {label: json.load(open(path)) for label, path in boards}
kinds = sorted({k for d in data.values() for k in d["by_type"]})


def pct(a, b):
    return f"{100 * a / b:.0f}%" if b else "–"


rows = []
for k in kinds:
    cells, n_all, c_all = [], 0, 0
    for label, _ in boards:
        t = data[label]["by_type"].get(k)
        cells.append(f"{pct(t['caught'], t['n'])} ({t['n']})" if t else "–")
        if t:
            n_all, c_all = n_all + t["n"], c_all + t["caught"]
    rows.append((c_all / n_all if n_all else 0, k, cells, pct(c_all, n_all)))
rows.sort()
lines = ["| Kind | " + " | ".join(l for l, _ in boards) + " | All |", "|---|" + "---:|" * (len(boards) + 1)]
lines += [f"| {k} | " + " | ".join(c) + f" | **{a}** |" for _, k, c, a in rows]
tot = [f"{pct(d['caught'], d['mistakes'])} ({d['mistakes']})" for d in data.values()]
fa = [pct(d["clean_standard_flagged"], d["clean_standard"]) for d in data.values()]
lines += ["| **All kinds** | " + " | ".join(tot) + " | |", "| False alarms on the correct versions | " + " | ".join(fa) + " | |"]
open(out, "w").write("# Coverage chart\n\nCaught (passages) per kind of mistake, worst first.\n\n" + "\n".join(lines) + "\n")
print("\n".join(lines))
