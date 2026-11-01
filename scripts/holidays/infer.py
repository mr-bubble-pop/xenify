#!/usr/bin/env python3
"""Work out holiday RULES for crates/xenify-holidays from python-holidays (MIT).

Dev-only: nothing here ships. python-holidays is used as the reference: for
each country and region we look at its output for many years and find the
simplest rule that gives every date (a fixed day, the nth weekday of a month,
a weekday on or after a day, days from Easter), plus the years it moved or
didn't happen. Holidays that follow no rule (lunar calendars, one-offs) keep
their list of dates. Weekend days that move to a working day are written as a
`substitute` policy, not as dates.

Usage: python3 -I scripts/holidays/infer.py <path to python-holidays site dir> [CC ...]
Writes crates/xenify-holidays/data/<cc>.toml. Check with scripts/holidays/check.py.
"""
import re
import sys
from collections import Counter, defaultdict
from datetime import date, timedelta
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import holidays  # noqa: E402
from dateutil.easter import EASTER_ORTHODOX, EASTER_WESTERN, easter  # noqa: E402

YEARS = range(2000, 2076)
OUT = Path(__file__).resolve().parents[2] / "crates" / "xenify-holidays" / "data"
DEFAULT = ["GB", "IE", "US", "CA", "AU", "NZ", "DE", "FR", "NL", "BE", "ES", "PT", "IT", "AT", "CH",
           "SE", "NO", "DK", "FI", "PL", "CZ", "ZA", "IN", "JP", "SG", "BR", "MX"]
WD = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
OBS = re.compile(r"\s*\((observed|substitute day)[^)]*\)$|^Day off \(substitute from [^)]*\)$", re.I)


def country_class(cc):
    return type(holidays.country_holidays(cc))


def country_name(cc):
    from holidays.registry import COUNTRIES
    name = next((v[0] for v in COUNTRIES.values() if cc in v), cc)
    return re.sub(r"(?<=[a-z])(?=[A-Z])", " ", name)


def dates_for(cc, sub):
    kw = {"subdiv": sub} if sub else {}
    cls = country_class(cc)
    lang = "en_US" if "en_US" in (cls.supported_languages or ()) else None
    if lang:
        kw["language"] = lang
    out = {}
    for y in YEARS:
        out[y] = sorted(holidays.country_holidays(cc, years=y, **kw).items())
    return out


def fmt(d):
    return f"{d.month:02}-{d.day:02}"


def nth_weekday(y, m, wd, n):
    if n > 0:
        d = date(y, m, 1)
        d += timedelta((wd - d.weekday()) % 7)
        d += timedelta(7 * (n - 1))
        return d if d.month == m else None
    nxt = date(y + (m == 12), m % 12 + 1, 1)
    d = nxt - timedelta(1)
    return d - timedelta((d.weekday() - wd) % 7)


def candidates(by_year):
    """Rules (as TOML key/values) and the date each gives in a year."""
    sample = next(iter(by_year.values()))
    rules = [({"date": fmt(sample)}, lambda y, m=sample.month, d=sample.day: safe(y, m, d))]
    for wd in range(7):
        for m in {d.month for d in by_year.values()}:
            for n in (1, 2, 3, 4, 5, -1):
                rules.append(({"weekday": WD[wd], "month": m, "nth": n}, lambda y, m=m, wd=wd, n=n: nth_weekday(y, m, wd, n)))
    for d0 in {fmt(d) for d in by_year.values()}:
        m, dd = map(int, d0.split("-"))
        wd = sample.weekday()
        rules.append(({"weekday": WD[wd], "on_or_after": f"{m:02}-{dd:02}"},
                      lambda y, m=m, dd=dd, wd=wd: on_or_after(y, m, dd, wd)))
        rules.append(({"weekday": WD[wd], "on_or_before": f"{m:02}-{dd:02}"},
                      lambda y, m=m, dd=dd, wd=wd: on_or_before(y, m, dd, wd)))
    for key, method in (("easter", EASTER_WESTERN), ("orthodox_easter", EASTER_ORTHODOX)):
        y0 = next(iter(by_year))
        off = (by_year[y0] - easter(y0, method)).days
        rules.append(({key: off}, lambda y, off=off, method=method: easter(y, method) + timedelta(off)))
    return rules


def safe(y, m, d):
    try:
        return date(y, m, d)
    except ValueError:
        return None


def on_or_after(y, m, d, wd):
    x = safe(y, m, d)
    return x and x + timedelta((wd - x.weekday()) % 7)


def on_or_before(y, m, d, wd):
    x = safe(y, m, d)
    return x and x - timedelta((x.weekday() - wd) % 7)


def best_rule(by_year):
    best = None
    for rule, f in candidates(by_year):
        moved = {y: d for y, d in by_year.items() if f(y) != d}
        if best is None or len(moved) < len(best[1]):
            best = (rule, moved)
        if not moved:
            break
    rule, moved = best
    if len(moved) * 3 > len(by_year):  # no rule: keep the dates
        return {"dates": by_year}, {}
    return rule, moved


def substitute_policy(pairs, all_days):
    """pairs: (holiday date, observed date); all_days: every date it fell on."""
    if pairs and all(o < d for d, o in pairs):
        return "before"
    if any(o < d for d, o in pairs):
        return "nearest"
    moved = {d for d, _ in pairs}
    sats = [d for d in all_days if d.weekday() == 5]
    if sats and not any(d in moved for d in sats):
        return "sunday"
    return "next-free"


def holidays_of(cc, sub):
    """name -> {year: date}, and name -> [(date, observed)]."""
    data = dates_for(cc, sub)
    by_name = defaultdict(dict)
    observed = defaultdict(list)
    obs_raw = defaultdict(list)
    for y, items in data.items():
        for d, names in items:
            for name in names.split("; "):
                if OBS.search(name):
                    obs_raw[y].append((d, OBS.sub("", name).strip()))
                elif y in by_name[name]:
                    by_name[name + f" ({d.month}/{d.day})"][y] = d
                else:
                    by_name[name][y] = d
    for y, lst in obs_raw.items():
        for o, base in lst:
            # The day it moved from: the same name within a week, maybe in the next or last year.
            near = [d for yy in (y - 1, y, y + 1) for d in [by_name.get(base, {}).get(yy)] if d and abs((o - d).days) <= 7]
            if near:
                observed[base].append((min(near, key=lambda d: abs((o - d).days)), o))
            else:
                by_name.setdefault(base + " (observed)", {})[y] = o
    return by_name, observed


def entries(cc, sub):
    by_name, observed = holidays_of(cc, sub)
    out = {}
    for name, by_year in by_name.items():
        years = sorted(by_year)
        rule, moved = best_rule(by_year)
        e = dict(rule)
        if "dates" not in e:
            first, last = years[0], years[-1]
            if first > YEARS[0]:
                e["from"] = first
            if last < YEARS[-1]:
                e["until"] = last
            missing = [y for y in range(first, last + 1) if y not in by_year]
            if missing:
                e["except"] = missing
            if moved:
                e["moved"] = {y: d for y, d in moved.items()}
        if observed.get(name):
            pol = e["substitute"] = substitute_policy(observed[name], by_year.values())
            # Moving off weekends can start in a later year (New Zealand 2014).
            moved_years = {d.year for d, _ in observed[name]}
            first = min(moved_years)
            weekend = 6 if pol == "sunday" else 5
            if any(d.weekday() >= weekend and y < first for y, d in by_year.items()):
                e["substitute_from"] = first
        out[name] = e
    return out


def toml_value(v):
    if isinstance(v, str):
        return '"' + v.replace('\\', '\\\\').replace('"', '\\"') + '"'
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f'"{k}" = {toml_value(fmt(d) if isinstance(d, date) else d)}' for k, d in v.items()) + " }"
    raise TypeError(v)


def write(cc):
    cls = country_class(cc)
    subs = list(cls.subdivisions)
    aliases = {v: k for k, v in (cls.subdivisions_aliases or {}).items()}
    per = {None: entries(cc, None)}
    for s in subs:
        per[s] = entries(cc, s)
    # Merge identical holidays across regions.
    merged = defaultdict(list)  # (name, rule) -> regions
    for s, ents in per.items():
        for name, e in ents.items():
            key = (name, repr(sorted((k, repr(v)) for k, v in e.items())))
            merged[key].append((s, e))
    lines = [
        "# Generated by scripts/holidays/infer.py from python-holidays (MIT,",
        "# https://github.com/vacanza/holidays): rules worked out from its dates.",
        f'country = "{cc}"',
        f'name = {toml_value(country_name(cc))}',
        "",
    ]
    if subs:
        lines.append("[regions]")
        for s in subs:
            lines.append(f'{toml_value(s)} = {toml_value(aliases.get(s, s))}')
        lines.append("")
    everywhere = set([None] + subs)
    for (name, _), lst in sorted(merged.items(), key=lambda kv: first_date(kv[1][0][1])):
        regions = [s for s, _ in lst]
        e = lst[0][1]
        lines.append("[[holiday]]")
        lines.append(f"name = {toml_value(name)}")
        for k in ("date", "weekday", "month", "nth", "on_or_after", "on_or_before", "easter", "orthodox_easter",
                  "from", "until", "except", "substitute", "substitute_from"):
            if k in e:
                lines.append(f"{k} = {toml_value(e[k])}")
        if set(regions) != everywhere:
            # "" is the country with no region chosen.
            lines.append(f"regions = {toml_value(['' if s is None else s for s in regions])}")
        for k in ("moved", "dates"):
            if k in e:
                lines.append(f"{k} = {toml_value({str(y): d for y, d in e[k].items()})}")
        lines.append("")
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{cc.lower()}.toml").write_text("\n".join(lines))
    print(cc, len(merged), "holidays,", len(subs), "regions")


def first_date(e):
    if "date" in e:
        return e["date"]
    if "month" in e:
        return f"{e['month']:02}-00"
    for k in ("on_or_after", "on_or_before"):
        if k in e:
            return e[k]
    if "easter" in e or "orthodox_easter" in e:
        return "04-00"
    if "dates" in e:
        return fmt(next(iter(e["dates"].values())))
    return "99"


if __name__ == "__main__":
    for cc in sys.argv[2:] or DEFAULT:
        write(cc)
