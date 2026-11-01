"""Runs a trained spotter over a corpus file and writes its findings, for
`xenify-wp-corpus score --spotter`. Each output line: {"text": ..., "found":
[[start, end, p_error, code], ...]} for every passage text (with mistakes,
corrected, and clean). Dev-only tool (MIT, part of Xenify)."""
import json, sys, torch
from safetensors.torch import load_file
from transformers import AutoTokenizer, AutoModelForTokenClassification, AutoConfig

model_dir, corpus, out = sys.argv[1], sys.argv[2], sys.argv[3]
cfg = AutoConfig.from_pretrained(model_dir)
m = AutoModelForTokenClassification.from_config(cfg)
w = model_dir + "/model.f16.safetensors"
m.load_state_dict({k: v.float() for k, v in load_file(w).items()}, strict=False)
m.eval()
torch.set_num_threads(8)
tok = AutoTokenizer.from_pretrained(model_dir)
texts = []
for line in open(corpus, encoding="utf-8"):
    it = json.loads(line)
    texts.append(it["text"])
    if it.get("corrected"):
        texts.append(it["corrected"])
texts = sorted(set(texts))
with open(out, "w", encoding="utf-8") as f, torch.no_grad():
    for i in range(0, len(texts), 32):
        chunk = texts[i:i + 32]
        enc = tok(chunk, truncation=True, max_length=256, padding=True, return_offsets_mapping=True, return_tensors="pt")
        off = enc.pop("offset_mapping")
        prob = m(**enc).logits.softmax(-1)
        for b, t in enumerate(chunk):
            found = []
            for k, (s, e) in enumerate(off[b].tolist()):
                if s == e:
                    continue
                pe = 1.0 - prob[b, k, 0].item()
                if pe >= 0.3:
                    found.append([s, e, round(pe, 4), cfg.id2label[int(prob[b, k, 1:].argmax()) + 1]])
            f.write(json.dumps({"text": t, "found": found}, ensure_ascii=False) + "\n")
        if i % 640 == 0:
            print(f"{i}/{len(texts)}", flush=True)
print("done", len(texts))
