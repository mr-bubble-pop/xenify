"""Fine-tunes a small encoder (DeBERTa-v3) to mark the words of a passage
that hold a mistake, with the kind of mistake (an error code), and scores it.
Dev-only tool (MIT, part of Xenify). Usage:
  python spotter.py --data DIR --base MODEL_DIR --out OUT_DIR [--epochs 3]
Smaller, faster model: --layers 6 keeps every other layer of --base (best
started from a trained spotter), and --teacher DIR makes it also copy that
trained spotter's scores (distillation).
"""
import argparse, json, math, random, time
import torch
from torch.utils.data import DataLoader
from transformers import AutoTokenizer, AutoModelForTokenClassification, get_linear_schedule_with_warmup

p = argparse.ArgumentParser()
p.add_argument("--data", required=True)
p.add_argument("--base", required=True)
p.add_argument("--out", required=True)
p.add_argument("--epochs", type=float, default=3)
p.add_argument("--lr", type=float, default=5e-5)
p.add_argument("--batch", type=int, default=64)
p.add_argument("--maxlen", type=int, default=160)
p.add_argument("--error-weight", type=float, default=3.0)
p.add_argument("--layers", type=int, default=0, help="keep this many encoder layers, evenly spaced")
p.add_argument("--teacher", help="trained spotter to copy scores from")
p.add_argument("--temperature", type=float, default=2.0)
p.add_argument("--bf16", action="store_true", help="bfloat16 autocast (faster on recent GPUs)")
p.add_argument("--fp16", action="store_true", help="float16 autocast with loss scaling (for GPUs without bfloat16, e.g. V100)")
args = p.parse_args()

dev_name = "cuda" if torch.cuda.is_available() else "cpu"
labels = json.load(open(f"{args.data}/labels.json"))
code = {c: i for i, c in enumerate(labels)}
tok = AutoTokenizer.from_pretrained(args.base)

def load(name):
    return [json.loads(l) for l in open(f"{args.data}/{name}.jsonl", encoding="utf-8")]

def encode(rows):
    enc = tok([r["text"] for r in rows], truncation=True, max_length=args.maxlen, return_offsets_mapping=True, padding=True, return_tensors="pt")
    lab = torch.full(enc["input_ids"].shape, -100, dtype=torch.long)
    for b, r in enumerate(rows):
        for t, (s, e) in enumerate(enc["offset_mapping"][b].tolist()):
            if s == e:
                continue  # special or padding
            l = 0
            for (ss, se, c) in r["spans"]:
                if s < se and ss < e:
                    l = code[c]
            lab[b, t] = l
    return enc, lab

def pretokenize(rows):
    """Tokenizes every row once: (input_ids, labels) lists, unpadded."""
    enc = tok([r["text"] for r in rows], truncation=True, max_length=args.maxlen, return_offsets_mapping=True)
    out = []
    for b, r in enumerate(rows):
        lab = []
        for (s, e) in enc["offset_mapping"][b]:
            if s == e:
                lab.append(-100)
                continue
            l = 0
            for (ss, se, c) in r["spans"]:
                if s < se and ss < e:
                    l = code[c]
            lab.append(l)
        out.append((enc["input_ids"][b], lab))
    return out

def collate(items):
    n = max(len(i) for i, _ in items)
    pad = tok.pad_token_id
    ids = torch.tensor([i + [pad] * (n - len(i)) for i, _ in items])
    att = torch.tensor([[1] * len(i) + [0] * (n - len(i)) for i, _ in items])
    lab = torch.tensor([l + [-100] * (n - len(l)) for _, l in items])
    return ids, att, lab

model = AutoModelForTokenClassification.from_pretrained(args.base, num_labels=len(labels), id2label=dict(enumerate(labels)), label2id=code).to(dev_name)
if args.layers:
    layers = model.deberta.encoder.layer
    keep = [round(i * (len(layers) - 1) / (args.layers - 1)) for i in range(args.layers)] if args.layers > 1 else [len(layers) - 1]
    model.deberta.encoder.layer = torch.nn.ModuleList([layers[i] for i in keep])
    model.config.num_hidden_layers = args.layers
    print("kept layers", keep, flush=True)
teacher = None
if args.teacher:
    teacher = AutoModelForTokenClassification.from_pretrained(args.teacher).to(dev_name).eval()
    for q in teacher.parameters():
        q.requires_grad_(False)
train = load("train")
steps_per_epoch = math.ceil(len(train) / args.batch)
total = int(steps_per_epoch * args.epochs)
opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.01)
AMP = (args.bf16 or args.fp16) and dev_name == "cuda"
AMP_DTYPE = torch.float16 if args.fp16 else torch.bfloat16
# float16 needs loss scaling to keep small gradients; bfloat16 doesn't (scaler off = plain steps).
scaler = torch.amp.GradScaler("cuda", enabled=args.fp16 and dev_name == "cuda")
sched = get_linear_schedule_with_warmup(opt, int(0.06 * total), total)
weights = torch.ones(len(labels), device=dev_name)
weights[1:] = args.error_weight
loss_fn = torch.nn.CrossEntropyLoss(weight=weights, ignore_index=-100)

t_tok = time.time()
train_tok = pretokenize(train)
print(f"{len(train)} training rows tokenized in {time.time()-t_tok:.0f}s; {total} steps on {dev_name}", flush=True)
loader = DataLoader(train_tok, batch_size=args.batch, shuffle=True, collate_fn=collate, num_workers=4, pin_memory=True, persistent_workers=True)
model.train()
step, t0 = 0, time.time()
while step < total:
    for ids, att, lab in loader:
        if step >= total:
            break
        with torch.autocast(dev_name, dtype=AMP_DTYPE, enabled=AMP):
            out = model(input_ids=ids.to(dev_name, non_blocking=True), attention_mask=att.to(dev_name, non_blocking=True))
        lab = lab.to(dev_name, non_blocking=True)
        loss = loss_fn(out.logits.float().view(-1, len(labels)), lab.view(-1))
        if teacher is not None:
            with torch.no_grad(), torch.autocast(dev_name, dtype=AMP_DTYPE, enabled=AMP):
                t_logits = teacher(input_ids=ids.to(dev_name), attention_mask=att.to(dev_name)).logits.float()
            m = lab.view(-1) != -100
            T = args.temperature
            s_log = (out.logits.float().view(-1, len(labels))[m] / T).log_softmax(-1)
            t_p = (t_logits.view(-1, len(labels))[m] / T).softmax(-1)
            kd = torch.nn.functional.kl_div(s_log, t_p, reduction="batchmean") * T * T
            loss = 0.5 * loss + 0.5 * kd
        scaler.scale(loss).backward()
        scaler.unscale_(opt)
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        scaler.step(opt); scaler.update(); sched.step(); opt.zero_grad()
        step += 1
        if step % 200 == 0:
            print(f"step {step}/{total} loss {loss.item():.4f} ({time.time()-t0:.0f}s)", flush=True)

model.eval()

@torch.no_grad()
def predict(rows):
    """Per row: list of (start, end, p_error, best_code)."""
    res = []
    for i in range(0, len(rows), 128):
        chunk = rows[i:i + 128]
        enc, _ = encode(chunk)
        with torch.autocast(dev_name, dtype=AMP_DTYPE, enabled=AMP):
            logits = model(input_ids=enc["input_ids"].to(dev_name), attention_mask=enc["attention_mask"].to(dev_name)).logits.float()
        prob = logits.softmax(-1).cpu()
        for b in range(len(chunk)):
            toks = []
            for t, (s, e) in enumerate(enc["offset_mapping"][b].tolist()):
                if s == e:
                    continue
                pe = 1.0 - prob[b, t, 0].item()
                best = int(prob[b, t, 1:].argmax().item()) + 1
                toks.append((s, e, pe, labels[best]))
            res.append(toks)
    return res

def evaluate(name):
    rows = load(name)
    preds = predict(rows)
    report = {"thresholds": {}}
    for th in [0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]:
        n = hit = right_code = clean = fa = 0
        per = {}
        for r, toks in zip(rows, preds):
            if r["spans"]:
                s0, e0, c0 = r["spans"][0]
                n += 1
                near = [t for t in toks if t[0] <= e0 + 1 and s0 <= t[1] + 1 and t[2] >= th]
                d = per.setdefault(c0, [0, 0])
                d[0] += 1
                if near:
                    hit += 1
                    d[1] += 1
                    if max(near, key=lambda t: t[2])[3] == c0:
                        right_code += 1
            else:
                clean += 1
                if any(t[2] >= th for t in toks):
                    fa += 1
        report["thresholds"][th] = {"found": hit / n, "right_code": right_code / n, "false_alarms": fa / clean, "per_kind": {k: v[1] / v[0] for k, v in per.items()}}
        print(f"{name} t={th}: found {hit}/{n} ({100*hit/n:.1f}%), right code {100*right_code/n:.1f}%, correct passages flagged {fa}/{clean} ({100*fa/clean:.1f}%)", flush=True)
    return report

report = {"dev": evaluate("dev"), "test": evaluate("test"), "args": vars(args)}
model.save_pretrained(args.out, safe_serialization=True)
tok.save_pretrained(args.out)
json.dump(report, open(f"{args.out}/report.json", "w"), indent=1)
print("saved", args.out, flush=True)
# Data-loader workers can hold GPU memory after the run; exit hard.
import os
os._exit(0)
