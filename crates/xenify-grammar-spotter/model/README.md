# Xenify mistake spotter (B7)

The small offline model Xenify Write uses to find mistakes the rules can't,
like missing words ("pass me salt"). It runs on your own computer, or in your
own browser; no text is sent anywhere.

- **What it is:** DeBERTa-v3-xsmall, fine-tuned for token classification
  (each word piece: OK, or the kind of mistake). 67 MB, 16-bit weights.
- **Base model:** DeBERTa-v3 by Microsoft, MIT licence
  (https://github.com/microsoft/DeBERTa; licence text in
  `LICENSE-DeBERTa.txt`, © Microsoft Corporation). Its vocabulary was cut from
  128,000 to 31,700 word pieces.
- **Training data:** the Xenify proofing corpus (synthetic passages, CC0-1.0)
  and passages derived from Wikipedia edits (the WikEd Error Corpus;
  Wikipedia text is CC BY-SA 4.0). The model holds no copy of that text.
- **Licence:** the fine-tuned weights are released under the MIT licence,
  the same as Xenify.

Files: `config.json`, `tokenizer.json`, `model.f16.safetensors`.
See docs/grammar-checker.md for how it was trained and tested.
