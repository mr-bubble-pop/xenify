"""Builds training and evaluation files for the error spotter from the corpus.

Each line: {"text": ..., "spans": [[start, end, code], ...]} with byte...
character offsets into text and an error code (the corpus's mistake kind).
Correct passages have no spans. Dev-only tool (MIT, part of Xenify).
"""
import json, random, sys

# US → UK/Australian spellings (our own list). Some training passages are
# respelled so the model treats both as normal; spelling is the spell
# checker's job, not the model's.
UK = {w.split(":")[0]: w.split(":")[1] for w in """
color:colour colors:colours colored:coloured favorite:favourite favorites:favourites honor:honour honored:honoured
behavior:behaviour behaviors:behaviours neighbor:neighbour neighbors:neighbours neighborhood:neighbourhood labor:labour
flavor:flavour flavors:flavours humor:humour rumor:rumour harbor:harbour vapor:vapour endeavor:endeavour savor:savour
center:centre centers:centres theater:theatre theaters:theatres meter:metre meters:metres liter:litre fiber:fibre
organize:organise organized:organised organizing:organising organization:organisation organizations:organisations
realize:realise realized:realised realizing:realising recognize:recognise recognized:recognised apologize:apologise
apologized:apologised prioritize:prioritise prioritized:prioritised analyze:analyse analyzed:analysed analyzing:analysing
optimize:optimise optimized:optimised summarize:summarise summarized:summarised emphasize:emphasise minimize:minimise
maximize:maximise customize:customise customized:customised finalize:finalise finalized:finalised authorize:authorise
utilize:utilise criticize:criticise memorize:memorise standardize:standardise categorize:categorise characterize:characterise
traveled:travelled traveling:travelling traveler:traveller canceled:cancelled canceling:cancelling labeled:labelled
labeling:labelling modeled:modelled modeling:modelling fueled:fuelled leveled:levelled counseling:counselling
defense:defence offense:offence license:licence pretense:pretence catalog:catalogue dialog:dialogue analog:analogue
gray:grey mom:mum moms:mums program:programme check:cheque tire:tyre tires:tyres jewelry:jewellery pajamas:pyjamas
aluminum:aluminium plow:plough mold:mould skeptical:sceptical maneuver:manoeuvre fulfill:fulfil enrollment:enrolment
aging:ageing judgment:judgement""".split()}
UK.pop("check", None)  # "check" is also a verb; only the bank meaning is "cheque"
UK.pop("program", None)  # computer program stays "program" in UK English

def respell(text):
    out, word = [], ""
    for ch in text + " ":
        if ch.isalpha():
            word += ch
            continue
        if word:
            low = word.lower()
            rep = UK.get(low)
            if rep:
                word = rep.capitalize() if word[0].isupper() else rep
            out.append(word)
            word = ""
        out.append(ch)
    return "".join(out)[:-1]

# Mistake kinds left to the rules (spell checker etc.), not taught to the model.
SKIP = {"spelling", "capitalization", "repeated_word"}

def mistake_rows(path, with_clean_registers=True):
    rows = []
    for line in open(path, encoding="utf-8"):
        it = json.loads(line)
        if it.get("error_type") in SKIP:
            if it.get("corrected"):
                rows.append({"text": it["corrected"], "spans": []})
            continue
        if it.get("error_type"):
            text, wrong = it["text"], it["mistakes"][0]["wrong"]
            i = text.find(wrong)
            if i < 0 or not it.get("corrected"):
                continue
            rows.append({"text": text, "spans": [[i, i + len(wrong), it["error_type"]]]})
            rows.append({"text": it["corrected"], "spans": []})
        elif with_clean_registers:
            rows.append({"text": it["text"], "spans": []})
    return rows

def main():
    corpus = sys.argv[1]  # data/proofing-corpus
    out = sys.argv[2]
    random.seed(20261009)
    # Double-checked files (train-clean = both checkers; dev/holdout-clean = reviewed).
    train = mistake_rows(f"{corpus}/train-clean.jsonl")
    sounds = [json.loads(l) for l in open(f"{corpus}/sounds.jsonl", encoding="utf-8")]
    sounds = [s["text"] for s in sounds if s.get("verified") is not False]
    random.shuffle(sounds)
    # Respell about a quarter of passages the UK/Australian way, keeping spans in step.
    respelled = []
    for r in train:
        if random.random() < 0.25:
            t = respell(r["text"])
            if len(t) == len(r["text"]) or not r["spans"]:
                respelled.append({"text": t, "spans": r["spans"]} if len(t) == len(r["text"]) else {"text": t, "spans": []})
            else:
                # Lengths changed: re-find each wrong span in the respelled text.
                spans = []
                for s0, e0, c in r["spans"]:
                    w = respell(r["text"][s0:e0])
                    i = t.find(w)
                    if i >= 0:
                        spans.append([i, i + len(w), c])
                if len(spans) == len(r["spans"]):
                    respelled.append({"text": t, "spans": spans})
    train += respelled
    # Longer passages with several mistakes: join 2–3 passages (some correct).
    pool = list(train)
    joined = []
    for _ in range(len(pool) // 3):
        parts = random.sample(pool, random.choice([2, 3]))
        text, spans = "", []
        for p in parts:
            if text:
                text += " "
            spans += [[s + len(text), e + len(text), c] for s, e, c in p["spans"]]
            text += p["text"]
        joined.append({"text": text, "spans": spans})
    train += joined
    n_mistakes = sum(1 for r in train if r["spans"])
    train += [{"text": t, "spans": []} for t in sounds[: n_mistakes]]
    random.shuffle(train)
    dev = mistake_rows(f"{corpus}/dev-clean.jsonl", with_clean_registers=False)
    test = mistake_rows(f"{corpus}/holdout-clean.jsonl", with_clean_registers=False)
    # Nothing from dev/test in training.
    held = {r["text"] for r in dev + test}
    train = [r for r in train if r["text"] not in held]
    codes = sorted({s[2] for r in train + dev + test for s in r["spans"]})
    for name, rows in [("train", train), ("dev", dev), ("test", test)]:
        with open(f"{out}/{name}.jsonl", "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
        print(name, len(rows), "rows,", sum(1 for r in rows if r["spans"]), "with a mistake")
    json.dump(["OK"] + codes, open(f"{out}/labels.json", "w"), indent=1)
    print(len(codes), "codes")

if __name__ == "__main__":
    main()
