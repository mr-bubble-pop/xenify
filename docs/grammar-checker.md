# Xenify Write grammar checker: models, findings, rules and licences

A record of how the checker is built, what was tried, what was learned, and
what has to be credited. Written 2026-10-18. The per-crate READMEs go deeper:
[`xenify-grammar`](../crates/xenify-grammar/README.md),
[`xenify-grammar-spotter`](../crates/xenify-grammar-spotter/README.md),
[`xenify-checker`](../crates/xenify-checker/README.md) and the
[proofing corpus dataset card](../crates/xenify-wp-corpus/DATASET_CARD.md).

## How it fits together

Each paragraph goes through these layers, in order:

1. **Harper** (`harper-core`): spelling and about 950 grammar and style rules.
2. **Xenify's own rules** (`crates/xenify-grammar/src/grammar.rs` plus `.weir`
   files in `rules/`): mistakes Harper misses.
3. **The sound-alike checker** (`sound_alike.rs`): 356 sets of words that sound
   the same (their/there, pie/pi…), and a context model that says which word
   fits.
4. **Style, clarity, consistency and tone notes**, switched by the writing
   profile (`style.rs`, `clarity.rs`, `consistency.rs`, `register.rs`).
5. **The mistake spotter**: a small AI model that runs on the computer, never
   online. It marks words that look wrong and the kind of mistake, and the
   checker then tests candidate fixes with it. It's loaded once by the
   background process `xenify-checker` and shared by every window and app.

Filters run after all layers. They drop known false alarms, flags that cut into
a word, suggestions with made-up or offensive words, and spelling guesses far
from what was typed (see [Rules](#rules-added-and-changed)).

## The models

All models are DeBERTa-v3 (MIT), with the vocabulary cut from 128,000 to 31,700
word pieces (`prune_vocab.py`). They're stored as 16-bit weights.

| Model | Size | Trained on | Result | Status |
|---|---|---|---|---|
| B5, B6 | xsmall, 67 MB | AI passages + book breaker data | superseded | archive only |
| **B7** | xsmall, 67 MB | data7: B6 data + WikEd edits ×2 | best on the older test sets | **installed** |
| B8 | xsmall | + hyphen and 354-set sound-alike breaker data | ~1 point fewer catches | rejected |
| B9 | xsmall | + four "doesn't make sense" kinds | fewer false alarms, 1–3 points fewer catches | rejected |
| B79 | xsmall | 50/50 weight average of B7 and B9 | less sure of itself; trades catches for false alarms | rejected, deleted |
| B10 | **small**, 135 MB | data9 | same as B9, twice the size | rejected |
| B11 | xsmall, 67 MB | data11: data9 kinds, every sentence broken twice | level with B7 overall; best on everyday text | **alternative**, not installed |

- **Installed model:** `~/.local/share/xenify/spotter` (`xenify_wp_check::default_spotter_dir()`).
  It's B7, cut-off 97% (`DEFAULT_THRESHOLD`). Sentences that may not make
  sense use their own cut-off of 99% (`GARBLED_THRESHOLD`). That folder isn't
  kept when the dev container is rebuilt: reinstall from `data/ml/v4/B7` or
  `checkpoints/models/B7.tgz`.
- **Backups:** `checkpoints/models/B7.tgz` and `B11.tgz`, plus `checkpoints/eval-sets-2026-10-18.tgz` (all test sets, including the everyday one), with `checkpoints/SHA256SUMS`. `checkpoints/` and `target/` are not in git: models are too big for it, and the training data includes CC BY-SA Wikipedia text. Keep a copy of `checkpoints/` outside the repo; ship the chosen model as a separate download (e.g. a GitHub release) with its README and licence notes. Older models (B5, B6, B8–B10, F7, F8) were deleted; the table above records what they were.
- **Training recipe for B11:** `data/ml/run11.sh` with `data/ml/data11.tgz`, `base.tgz` (the trimmed base model), `spotter.py` and `tof16.py`. Other data can be rebuilt from the scripts (see below).
  `run11.sh`, with the data archives `data/ml/data*.tgz`.
- **Training cost:** about 20–40 minutes on one rented GPU (RTX 3090/A5000/4000
  Ada, bf16). Use `--fp16` on GPUs without bf16, like the V100.
- **Speed on the laptop:** a 3,844-word document takes 10.5 s with B7 and
  10.9 s with B10 (4 threads, including loading). Peak memory is 412 MB for
  B7 and 605 MB for B10.

### Latest scores

Everyday test (122 short everyday sentences with one slip each, plus 148
correct ones; Australian English):

| Setup | Caught | Right fix | False alarms |
|---|---:|---:|---:|
| Rules only | 60% | 55% | 0 |
| Rules + B7 (installed) | 93% | 75% | 1 |
| Rules + B11 | 90% | 75% | 1 |
| Rules + B11, "doesn't make sense" on | **94%** | **76%** | 1 |

On the older test sets (AI passages, Wikipedia edits, broken book text), with
the doubled-word rule: B7 catches 92.5% / 62.8% / 80–93%, B11 about 0.5 points
less on each, and real false alarms are level (books about 2%, AI 2.2%,
Wikipedia 8.2–8.6%). B11 catches sound-alikes in everyday sentences that B7
misses ("enjoying pi", "looked grate", "the bare growled").

## What we found

- **Test what users type.** The AI-passage and Wikipedia test sets said "92%
  caught" while the user's own sentences ("Mr Monkey sit at table enjoying
  pie") got no useful flags. The everyday test
  (`crates/xenify-wp-corpus/train/everyday_set.py`) fixed that. Add every
  sentence a user reports to it.
- **Keep the breaker's per-kind counts when adding kinds.** Adding six mistake
  kinds with one damaged copy per sentence cut the common old kinds by about
  41% (missing a/the: 8,700 → 5,100 examples), and catches fell 1–3 points.
  B8–B10 all had this. Breaking every sentence twice (`--per-source 2`, data11)
  fixed it.
- **A bigger model wasn't the answer.** B10 (small, twice the size) scored the
  same as B9.
- **Training randomness is about ±1 point** on any one mistake kind (no fixed
  seed). Don't read single-point differences as real unless they hold on
  every set.
- **Averaging two models makes them less sure.** At the same cut-off it trades
  catches for fewer false alarms, so compare at several cut-offs.
- **The rules cover the most valuable cases.** Doubled words ("dogs dogs"),
  "we're/were", a name followed by a base verb, and "off/of" were all missed by
  the models and are now caught by rules with no false alarms on about 84,000
  correct passages.
- **Silly suggestions look worse than misses.** A review of 906 suggestions by
  an AI reviewer found slurs offered for "nigiri", made-up words ("mous",
  "tos"), and fragments ("Ge[o]rge" → "O"). Bad suggestions went from 214 to
  191. Most of the rest are old-book dialect and names.
- **Book text adds little to the sound-alike model.** Rare words barely occur
  in it (13 uses of "flour").
- **Harper's `LookingForwardTo` rule prints a debug line** ("gerund_form: …") to
  standard output. It breaks `xenify-write check --json` output, not the app.
  Upstream bug in harper-core 2.11, not fixed.

## Rules added and changed

Xenify's own rules (`grammar.rs`, each with an explanation in `explain.rs` and
a test):

| Rule | Catches | Example |
|---|---|---|
| `DoubledWord` | a longer word typed twice | "my three dogs dogs bark" |
| `WereForWeAre` | "we're" after there/they/we/you… | "There we're three cakes" → were |
| `TitleNameAgreement` | a title and name, then a base verb | "Mr Monkey sit" → sits |
| `OffForOf` | "off" typed for "of" | "new ways off working" |
| `LyHyphen`, `HyphenAfterNoun` | unneeded hyphens | "highly-skilled", "is well-known" |
| `EncaseInCase.weir` | encase/in case | "just encase" → in case |

Document checks (`consistency.rs`, profile switches "Consistency" and "Word
choice"):
- -ise/-ize mixes (from a fixed word list), email/e-mail, Internet/internet
  (fixed list), and 3/three;
- clichés, "very good" → "excellent", and three sentences in a row starting the
  same way.

Checks dropped after the false-alarm test: the Oxford comma, repeated words,
and general capitals.

Off by default (`OFF_BY_DEFAULT`, still in the rules list so users can turn them
on): `MissingPreposition`, and Harper's `CompoundNouns` ("man go" → "mango"),
`AvoidCurses` ("asses" → "butts"), `OxfordComma` and `PhrasalVerbAsCompoundNoun`
("Checkout redesign", "the setup" flagged). Each was wrong more often than right.

"Sentences that may not make sense" (the spotter's `garbled` kind) is a profile
setting, off by default because it offers no fix. When it's off, the spotter
leaves that kind out and its next-best kind can still show.

Suggestion filters (`lib.rs` and `xenify-wp-check`):
- `rules/lists/never-suggest.txt`: slurs and crude words never offered by any rule.
- Spelling: only guesses within 1 edit (words of 4 letters or fewer) or 2 edits,
  and only the closest ones ("occured" → "occurred", not "obscured").
- Names after "Mr.", "Dr." or an initial ("Jonathan H. McAllister") aren't spelling mistakes.
- `inside_word`: any flag that starts or ends inside a word is dropped.
- `sensible_suggestion`: every word a spotter suggestion adds must be in the dictionary.
- An abbreviation expanded mid-sentence starts with a small letter ("ASAP" → "as soon as possible").
- "This sentence has no verb" is skipped when the sentence has a spelling flag.
- "had had" and "that that" are allowed; "Or…/And…" at a sentence start isn't someone addressed.

Sound-alikes (`rules/homophones/`):
- **`sets.txt`:** 356 sets. The 77 after the CMU note came from the CMU
  Pronouncing Dictionary, curated by an AI reviewer and by hand. A `!` before a
  word (`!pi`, `!beer`) means it's never suggested for another word.
- **`model.tsv`:** context counts from about 95,000 checked AI sentences. Held
  out, it catches 45% of swaps with 0.22% false alarms.
- **`freq.tsv`:** how common each word is. A suggestion much rarer than the
  typed word needs extra evidence (half the log of the frequency ratio).
- **"to":** left alone at the end of a phrase ("glad to.") and in "as to".

## How to test and retrain

All tools are dev-only, MIT, in `crates/xenify-wp-corpus/train/` unless noted.

| Task | Command |
|---|---|
| Score a model on all seven sets | `eval_spotter.sh NAME MODEL_DIR 12` (`THRESHOLD`, `SETS` env) |
| Compare runs | `compare.py data/proofing-corpus/eval A B` and `false_alarms.py …` |
| Everyday test | `everyday_set.py data/proofing-corpus/eval/sets/everyday.jsonl`, then `xenify-wp-corpus score … --dialect au --using both --spotter-model DIR` |
| Turn on "doesn't make sense" while scoring | `XENIFY_SENTENCE_SENSE=1` |
| Review suggestions | collect with `xenify-write check --json` (paragraphs numbered from 1), then `judge_suggestions.py TRIPLES OUT` |
| See the model's raw view | `cargo run --release -p xenify-grammar-spotter --example spot -- MODEL 0.3 < lines.txt` |
| See the sound-alike model's view | `cargo run --release -p xenify-grammar --example sounds < lines.txt` |
| Sound-alike model | `xenify-wp-corpus sounds generate / verify / train / eval` |
| Breaker data | `xenify-wp-corpus break INPUT -o OUT --seed N --per-source 2`, then `spotter_data.py`, `breaker_data.py`, `wiked_data.py` |
| Average two models | `average_models.py A B OUT 0.5` (pure Python) |

Before deleting or adding a rule, run the false-alarm test: score the rules
on the four correct-text sets plus `gutenberg-train/books.jsonl` (about 84,000
passages) and read every flag from the changed rule. Aim for zero.

On the laptop, write a keep-awake time before a long run:
`echo $(( $(date +%s) + 3*3600 )) > target/KEEP-AWAKE`. Then
`scripts/keep-awake.sh`, running on the host, blocks sleep until that time.

## Licences and credits

Everything shipped is MIT or compatible. The release must carry these credits
(most are already in `assets/SOURCES.toml` and `THIRD_PARTY_LICENSES.txt`;
regenerate the latter with `scripts/licenses.sh`, which runs `cargo about --fail`).

| What | Licence | What we owe |
|---|---|---|
| harper-core (rules, spelling) | Apache-2.0 | listed in THIRD_PARTY_LICENSES.txt |
| DeBERTa-v3 xsmall/small (base models) | MIT | credit Microsoft in the model's README |
| WikEd Error Corpus (Wikipedia edits up to 2014) | Wikipedia text CC BY-SA 4.0 | the models were partly trained on it: say "derived from Wikipedia edits" when releasing a model. The raw edits are never shipped. |
| Project Gutenberg books | public domain | used for training and tests only. Ship no texts, and don't use the Gutenberg name for anything shipped. |
| CMU Pronouncing Dictionary | BSD-2-Clause | `rules/homophones/CMUDICT-LICENSE.txt` ships with sets.txt (77 sets derived from it) |
| Open English WordNet 2025 | CC BY 4.0 | shipped as the dictionary panel's data with attribution; used only to draft candidate sound-alike meanings, which were rewritten |
| AI-generated sentences (DeepSeek V4 Flash: MIT weights; Qwen3-235B, gpt-oss-120b: Apache-2.0; via OpenRouter, zero data retention) | ours (MIT) | no third-party text; record as generated |
| candle, tokenizers, fancy-regex, dirs, jiff | MIT/Apache-2.0/Unlicense | listed in THIRD_PARTY_LICENSES.txt |
| option-ext (pulled in by `dirs`) | MPL-2.0 | replaced by our MIT stand-in `patches/option-ext` (like ammonia and colored) |
| never-suggest.txt, sets.txt (own sets), everyday test, all rules | MIT | written for Xenify |

Rules we keep: no GPL/LGPL/MPL code or data in the release, and no "Grammarly",
"Word" or "Google Docs" in product names. Never send a user's own text to an
online AI; test sentences are fine.

## Missing words (2026-10-19)

- **The HTML Write (`xenify-write-app`) had no spotter**: its checker
  (`xenify-write-host/src/proof.rs`) ran the rules only, which miss most
  missing words. It now runs the spotter after the rules, like the egui Write,
  with the same `proofing.spotter` settings (Settings › Spelling and grammar ›
  "Also find harder mistakes"). In debug builds the model is very slow, so test it with `--release`.
- `missing_word` suggestions now also try two-word gaps ("to the", "in the",
  "at the"…), and the gap between the spot's words is tried first (only the
  first 40 candidates are tested): "I went store" → "to the store".
- New rule `MissingSmallWord`: "sat my chair" → "sat on my chair", "pass me
  salt" → "pass me the salt" (the model misses both). No false alarms on the books,
  Wikipedia or everyday sets.
- Suggestions that would leave a new grammar mistake where they go are dropped
  (`drop_fixes_that_break`): "Their is" offers "There", not "They're".

- New rule `RunOnSentence`: two sentences with nothing joining them ("I went
  home I ate dinner", "Mum cooked dinner we watched a film"), flagged even
  though the joining word can't be known; suggestions are a full stop, ", and"
  or a semicolon. It only fires on a new I/he/she/we/they with its own verb,
  after a clause with its own verb and subject, and not after a joining or
  saying word ("I think he left"), a noun with a/the ("the book I read"), or
  in a sentence that opens with when/as/if/a phrase. False alarms: 1 in about
  6,600 correct book passages ("Closer and closer they came"); 0 on Wikipedia,
  AI and everyday text. If more is needed, fine-tune **B7** with a run-on kind
  in the breaker before thinking about B11.

## Known gaps and next steps

- **Still missed:**
  - sentences that don't make sense, when the "doesn't make sense" setting is
    off (or below its 99% cut-off): "Challenge the grammar do things";
  - some missing words the model doesn't spot and no rule covers;
  - some sound-alikes: "He through the ball", "fresh flower to make bread".
- **B7 or B11:** decided 2026-10-18: **B7 stays.** It worked well in the user's own testing once the rules above were added. B11 (better on everyday sound-alikes) is kept in `checkpoints/models/B11.tgz` in case that changes.
  a fresh, unseen everyday test, written by a different AI model, before
  switching.
- **Cut-offs:** test lower cut-offs (97% overall, 99% for "doesn't make sense")
  on the everyday test and the 84,000 correct passages.
- **Users' own settings:** Write saves the cut-off in each user's preferences.
  An old saved 99% makes the model much quieter than the 97% default.
