#!/usr/bin/env python3
"""Compare crates/xenify-holidays with python-holidays (dev-only).

Only working days count (a holiday on a Sunday changes nothing in a plan), so
both sides are reduced to the weekday dates in 2000–2075.

Usage: python3 -I scripts/holidays/check.py <python-holidays site dir> [--show N]
Exit code 1 when anything differs that isn't in known-differences.txt
(rare edge cases the rules don't cover yet; --write refreshes that list).
"""
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import holidays  # noqa: E402

SHOW = int(sys.argv[sys.argv.index("--show") + 1]) if "--show" in sys.argv else 5
ROOT = Path(__file__).resolve().parents[2]
ours = json.loads(subprocess.run(
    ["cargo", "run", "-q", "-p", "xenify-holidays", "--example", "dump"],
    cwd=ROOT, check=True, capture_output=True, text=True).stdout)

KNOWN = Path(__file__).with_name("known-differences.txt")
known = set(KNOWN.read_text().splitlines()) if KNOWN.exists() else set()
seen = set()
bad = 0
for key, mine in sorted(ours.items()):
    cc, sub = key.split("/")
    kw = {"subdiv": sub} if sub else {}
    ref = set()
    for y in range(2000, 2076):
        for d in holidays.country_holidays(cc, years=y, **kw):
            if d.weekday() < 5:
                ref.add(d.isoformat())
    mine = set(mine)
    diff = {f"{key}:-{d}" for d in ref - mine} | {f"{key}:+{d}" for d in mine - ref}
    seen |= diff
    if "--write" not in sys.argv:
        diff -= known
    missing = sorted(x.split(":-")[1] for x in diff if ":-" in x)
    extra = sorted(x.split(":+")[1] for x in diff if ":+" in x)
    if missing or extra:
        bad += 1
        print(f"{key}: {len(missing)} missing {missing[:SHOW]}  {len(extra)} extra {extra[:SHOW]}")
if "--write" in sys.argv:
    KNOWN.write_text("\n".join(sorted(seen)) + "\n")
    print(f"wrote {len(seen)} known differences")
print(f"{len(ours) - bad} of {len(ours)} country/region sets match")
sys.exit(1 if bad else 0)
