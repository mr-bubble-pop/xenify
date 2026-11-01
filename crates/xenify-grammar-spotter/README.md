# xenify-grammar-spotter

A small offline model that marks the words in English text that look wrong and
says what kind of mistake it is (`article_missing`, `comma_splice`,
`subject_verb_agreement`… 27 kinds). It runs on the CPU with
[candle](https://github.com/huggingface/candle): no GPU, no internet, no
Python.

It pairs with [`xenify-grammar`](../xenify-grammar), the rule-based checker:
the rules give exact fixes for the mistakes they know, and the spotter catches
what rules can't see (missing words, tense changes, run-on sentences, word
order). Suggestions for spotted mistakes are found by trying candidate words in
the flagged spot and keeping the ones the model is happy with
([`Spotter::suggest`]).

```rust
let spotter = xenify_grammar_spotter::Spotter::load(Path::new("spotter-model"))?;
let text = "I have pet dog at home.";
for f in spotter.spot(text, 0.9)? {
    println!("{:?} {} {:.2}", f.range, f.code, f.confidence); // 7..14 article_missing 1.00
    for fix in spotter.suggest(text, &f, 3)? { println!("  → {}", fix.text); } // a pet dog
}
```

## The model

A fine-tuned [DeBERTa-v3-xsmall](https://huggingface.co/microsoft/deberta-v3-xsmall)
(MIT), made smaller for everyday computers:

- the word-piece list cut from 128,000 to 31,700 pieces that English text
  uses (every single letter is kept, so any text still works);
- an optional 6-layer version trained to copy the full 12-layer model (the
  current one predates the Wikipedia training data)
  (distillation).

| | Download | Memory | Speed (one paragraph, 4 CPU threads) |
|---|---:|---:|---:|
| 12-layer (default) | 67 MB | ~170 MB | ~45 ms |
| 6-layer | 47 MB | ~135 MB | ~25 ms |

The 12-layer model catches clearly more real mistakes for about 40 MB more
memory. In Xenify the model is loaded once by a background process,
[`xenify-checker`](../xenify-checker), and shared by every window and app.

## How well it works

Together with the rules of `xenify-grammar` (12-layer model, shown only when
97% sure; "caught" = the mistake was marked, "fixed" = a suggestion gives
exactly the right correction; "real false alarms" = correct passages wrongly
marked by a grammar rule or the model, leaving out names and words not in the
dictionary):

| Test set | Rules caught / fixed | Rules + model caught / fixed | Real false alarms, rules + model |
|---|---|---|---|
| Real Wikipedia mistakes (edits up to 2014, 1,689, none used in training) | 31% / 20% | 62% / 36% | 8.2%* |
| AI-written test passages (another model than training) | 53% / 45% | 88% / 65% | 2.0% |
| Public-domain books, UK and US (correct text only) | | | 1.6–1.8% |

\* The correct Wikipedia sentences are the editors' corrected versions, so
each holds a spot that editors changed, often for style; ordinary text gets
far fewer.

At 99% (a setting in Xenify Write) it catches a little less (58% / 86%) with
fewer false alarms (Wikipedia 5.3%, books 1.3–1.5%, AI 1.5%).

## Training it yourself

Everything is in the Xenify repository, `crates/xenify-wp-corpus/train/`. The
training data:

- AI-written passages (CC0);
- correct sentences from public-domain books, damaged in known ways by
  `xenify-wp-corpus break` (`breaker_data.py`);
- 39,000 real mistakes from Wikipedia edits up to 2014, from the
  [WikEd Error Corpus](https://github.com/snukky/wikiedits) (Grundkiewicz and
  Junczys-Dowmunt, 2014), kept only when an AI checker (Qwen3-235B) agreed the
  edit fixed a real grammar mistake (`wiked_extract.py`, `wiked_data.py`).
  Wikipedia text is CC BY-SA, so those edits are not included here.

```sh
python spotter_data.py CORPUS_DIR data          # passages → training rows
python prune_vocab.py microsoft/deberta-v3-xsmall base-pruned CORPUS_FILES... --dict harper.dict
python spotter.py --data data --base base-pruned --out big --bf16  # --fp16 on GPUs without bf16 --lr 1e-4 --batch 128 --maxlen 256
python spotter.py --data data --base big --teacher big --layers 6 --out small --bf16 --lr 1e-4 --batch 128 --maxlen 256
```

About 6 minutes per model on one RTX 5080.

## Licence

MIT. The base model (DeBERTa-v3) is MIT. The model was trained partly on
Wikipedia edits (Wikipedia text: CC BY-SA 4.0, via the WikEd Error Corpus);
see Training above.
