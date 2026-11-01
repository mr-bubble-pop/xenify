"""Shrinks a DeBERTa-v3 base model's vocabulary to the word pieces English
text actually uses (the embeddings are ~70% of xsmall's weights), before
fine-tuning. Every single-character piece is kept, so any text still
tokenizes; rare words just split into smaller pieces.
Dev-only tool (MIT, part of Xenify). Usage:
  python prune_vocab.py BASE_DIR OUT_DIR CORPUS_FILES... [--dict harper.dict] [--max 32000]
"""
import json, sys, collections, copy
from transformers import AutoTokenizer, AutoModel, PreTrainedTokenizerFast

base, out, files = sys.argv[1], sys.argv[2], sys.argv[3:]
dict_file, most = None, 32000
if "--max" in files:
    i = files.index("--max")
    most = int(files[i + 1])
    files = files[:i] + files[i + 2:]
if "--dict" in files:
    i = files.index("--dict")
    dict_file = files[i + 1]
    files = files[:i] + files[i + 2:]

tok = AutoTokenizer.from_pretrained(base)
texts = []
for f in files:
    for line in open(f, encoding="utf-8"):
        r = json.loads(line)
        for k in ("text", "corrected"):
            if r.get(k):
                texts.append(r[k])
if dict_file:
    for line in open(dict_file, encoding="utf-8"):
        w = line.split("/")[0].strip()
        if w and not w[0].isdigit() and not w.startswith("#"):
            texts += [w, w.capitalize(), "the " + w]
print(len(texts), "texts")

count = collections.Counter()
for i in range(0, len(texts), 5000):
    for ids in tok(texts[i:i + 5000], add_special_tokens=False)["input_ids"]:
        count.update(ids)

spec = json.loads(tok.backend_tokenizer.to_str())
vocab = spec["model"]["vocab"]  # [[piece, score], ...]; id = position
def latin(ch):
    # Letters, digits, punctuation and symbols English text uses (Latin-1,
    # Latin Extended, general punctuation, currency, arrows, common emoji).
    o = ord(ch)
    return o < 0x250 or 0x2000 <= o < 0x2BFF or 0x1F300 <= o < 0x1FAFF
keep = set(range(4))
keep |= {i for i, (p, _) in enumerate(vocab) if len(p.lstrip("▁")) == 1 and latin(p.lstrip("▁"))}
keep |= {i for i, _ in count.most_common(most - len(keep))}
keep = sorted(i for i in keep if i < len(vocab))
added = sorted(a["id"] for a in spec["added_tokens"] if a["id"] >= len(vocab))
old_ids = keep + added  # new id = position here
new_id = {o: n for n, o in enumerate(old_ids)}
print(f"keeping {len(keep)} of {len(vocab)} pieces")

spec["model"]["vocab"] = [vocab[i] for i in keep]
spec["model"]["unk_id"] = new_id[spec["model"]["unk_id"]]
for a in spec["added_tokens"]:
    a["id"] = new_id[a["id"]]
for st in spec["post_processor"]["special_tokens"].values():
    st["ids"] = [new_id[i] for i in st["ids"]]

model = AutoModel.from_pretrained(base)
emb = model.get_input_embeddings()
import torch
new = torch.nn.Embedding(len(old_ids), emb.weight.shape[1], padding_idx=0)
new.weight.data = emb.weight.data[torch.tensor(old_ids)].clone()
model.set_input_embeddings(new)
model.config.vocab_size = len(old_ids)
model.save_pretrained(out, safe_serialization=True)

import os, tempfile
path = os.path.join(out, "tokenizer.json")
json.dump(spec, open(path, "w", encoding="utf-8"), ensure_ascii=False)
ft = PreTrainedTokenizerFast(tokenizer_file=path, bos_token="[CLS]", eos_token="[SEP]", unk_token="[UNK]",
                             sep_token="[SEP]", pad_token="[PAD]", cls_token="[CLS]", mask_token="[MASK]")
ft.save_pretrained(out)
sample = "Recieved the colour-coded mitochondria report from Ngozi at 3pm, arvo."
print(tok.tokenize(sample)); print(ft.tokenize(sample))
print("params", sum(p.numel() for p in model.parameters()) / 1e6, "M")
