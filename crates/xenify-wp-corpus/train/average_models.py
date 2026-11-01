"""Averages two fine-tuned spotter models (same base) weight by weight.
B's extra output rows (labels B has and A doesn't) are kept from B.
    python3 average_models.py A_DIR B_DIR OUT_DIR WEIGHT_OF_A

Pure Python (no numpy); 16-bit or 32-bit safetensors. Dev-only tool (MIT, part of Xenify)."""
import json, struct, sys, shutil, os

a_dir, b_dir, out, wa = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])

def read(d):
    raw = open(f"{d}/model.f16.safetensors", "rb").read()
    n = struct.unpack("<Q", raw[:8])[0]
    return json.loads(raw[8:8 + n]), raw[8 + n:], raw[:8 + n]

ha, da, _ = read(a_dir)
hb, db, head_b = read(b_dir)
fmt = {"F16": "e", "F32": "f"}
parts = []
for name, t in sorted(((k, v) for k, v in hb.items() if k != "__metadata__"), key=lambda kv: kv[1]["data_offsets"][0]):
    s, e = t["data_offsets"]
    f = fmt[t["dtype"]]
    vb = list(struct.unpack(f"<{(e - s) // struct.calcsize(f)}{f}", db[s:e]))
    ta = ha.get(name)
    if ta is not None and ta["dtype"] == t["dtype"]:
        sa, ea = ta["data_offsets"]
        va = struct.unpack(f"<{(ea - sa) // struct.calcsize(f)}{f}", da[sa:ea])
        # Same shape, or B has extra leading rows-of-output at the end (classifier).
        for i in range(min(len(va), len(vb))):
            vb[i] = wa * va[i] + (1 - wa) * vb[i]
        if ta["shape"] != t["shape"]:
            print(f"{name}: {ta['shape']} → {t['shape']}, extra rows kept from B")
    elif ta is None:
        print(f"{name}: only in B")
    parts.append(struct.pack(f"<{len(vb)}{f}", *vb))
os.makedirs(out, exist_ok=True)
with open(f"{out}/model.f16.safetensors", "wb") as o:
    o.write(head_b)
    for p in parts:
        o.write(p)
for f in ("config.json", "special_tokens_map.json", "tokenizer_config.json", "tokenizer.json"):
    shutil.copy(f"{b_dir}/{f}", f"{out}/{f}")
print("wrote", out)
