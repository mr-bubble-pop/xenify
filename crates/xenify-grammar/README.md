# xenify-grammar

Offline English grammar, style and tone checking for Rust, built on
[Harper](https://writewithharper.com). Nothing leaves the machine.

On top of Harper's spelling and grammar rules it adds:

- **More grammar rules**: pronoun case (*her and me went*, *between you and I*),
  its/it's both ways, noun subject–verb agreement looking past *of …* phrases,
  *I seen*, *there is many*, durations (*on 10 minutes*, *since five years*),
  joining words (*Although …, but …*), word order (*tell me where is the office*),
  commas around the person addressed (*Let's eat, Grandma*), and more.
- **Embarrassing real-word typos** that spell-checkers miss: *pubic safety*,
  *project manger*, *defiantly interested*, *sing the contract*…
- **Sound-alike words judged by context** across 278 confusable sets
  (*spirits, bear and wine* → *beer*; *whether* / *weather*; *affect* / *effect*).
- **Writing profiles** (Standard, Academic paper, Workplace email, Technical
  documentation, Social post, Casual chat and more): each decides whether slang,
  texting, spoken spellings, dialect grammar, contractions, emoji and so on are
  allowed, gently noted or flagged. Informal and dialect English is never called
  "wrong". Profiles are TOML files you can edit and share.
- **Clarity checks for work writing**: noun stacks, long lead-ins before the main
  point, sentences doing too much, a request buried at the end of an email,
  acronyms used before they're spelled out.
- **"Why?" explanations**: a plain-English account with wrong → right examples for
  every rule ([`explain`]).

```rust
use xenify_grammar::{Checker, Profiles, Settings};

let settings = Settings { profile: Profiles::default().get("workplace-email"), ..Settings::default() };
let mut checker = Checker::new(settings, &[]);
let text = "Me and him went to the store, however it was closed.";
for issue in checker.check(text) {
    println!("{:?} [{}] {} → {:?}", &text[issue.range.clone()], issue.rule, issue.message, issue.suggestions);
    if let Some(why) = xenify_grammar::explain(&issue.rule) {
        println!("  why: {}", why.text);
    }
}
```

`Checker::check_paragraphs` checks a whole document (and adds the
whole-document checks); issue ranges are byte offsets into the text.

## How well it works

Measured on AI-generated test passages with one mistake each, across 30 kinds
of mistake, written by a different model than the one the rules were tuned on,
with wrong answer keys removed by a second checker and a review (the corpus
tools are in the Xenify repository, `crates/xenify-wp-corpus`):

- Harper alone finds 39% of the mistakes; with this crate's rules, **53%**, and
  for 45% the exact fix is among the suggestions.
- About 3% of correct passages get a false alarm, nearly all from the spell
  checker (rare names and technical words); this crate's own rules account for
  about 0.2%.
- Everyday slips (pronoun case, its/it's, real-word typos, sign-offs, eggcorns,
  verb forms, much/many, less/fewer) are caught 70–100% of the time; missing
  articles, comma splices, fragments and tense shifts are mostly out of reach
  for rules. The companion crate `xenify-grammar-spotter` (a small offline
  model) covers those.

## Where the data came from

Everything in this crate is original work under the MIT licence:

- Rules (`rules/*.weir`, `src/grammar.rs`) were written from general grammar
  knowledge and published lists of common mistakes; no rule text was copied from
  other checkers.
- Word lists (`rules/lists/`) and the confusable sets
  (`rules/homophones/sets.txt`) were written for this crate.
- The sound-alike context counts (`rules/homophones/model.tsv`) were learned from
  about 64,000 AI-generated example sentences (DeepSeek V4 Flash, checked by
  Qwen3-235B); only the counts are included.

Harper itself is Apache-2.0.

## Licence

MIT.
