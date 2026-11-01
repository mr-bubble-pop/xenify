#!/usr/bin/env python3
"""Runs the video models the way most people do (dev tool): ComfyUI's own
default text-to-video templates (Comfy-Org/workflow_templates, benchmarks/),
with our prompts instead of their samples. LocalAI couldn't: its MiniMax
engine (vllm-cpp) didn't finish a clip in 98 minutes on an RTX A6000, and
both builds of its Wan 2.2 engine ship the processor-only PyTorch.

    python3 comfy_minimax.py   # in /workspace/xb, after comfy_setup.sh
Results: results/media/<model>-comfyui-*.mp4, entries in results/media.json.
"""
import json, os, shutil, sys, time, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "results")
API = "http://127.0.0.1:8188"

CLIPS = [
    ("baby", "Realistic video of a chubby baby with a big curly moustache sitting on a patterned rug eating cherry pie with both hands, smiling, crumbs falling"),
    ("gorilla", "Realistic video of a baby holding a toy hammer chasing a gorilla down a busy city street, the gorilla running away, people watching from the pavement"),
]

# Each template's defaults (frames, fps, steps), where its prompt and output
# go, and the size: each template's default (MiniMax 864x480, Wan 640x640),
# the same on every card so the cards compare.
MODELS = [
    {"name": "minimax-h3-comfyui", "template": "video_minimax_h3_t2v.json", "prompt": ("140:131", "prompt"),
     "size_node": "140:131", "save": "92", "size": "864x480", "frames": 124, "fps": 24, "steps": 20,
     "warmup": {"140:131": {"length": 22}, "140:137": {"value": 4}}},
    {"name": "wan2.2-14b-comfyui", "template": "video_wan2_2_14B_t2v.json", "prompt": ("128:89", "text"),
     "size_node": "128:74", "save": "80", "size": "640x640", "frames": 81, "fps": 16, "steps": 20,
     "warmup": {"128:74": {"length": 17}, "128:114": {"value": 4}, "128:115": {"value": 2}}},
]


def log(*a):
    line = time.strftime("%H:%M:%S ") + " ".join(str(x) for x in a)
    print(line, flush=True)
    with open(os.path.join(OUT, "job.log"), "a") as f:
        f.write(line + "\n")


def call(path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def record(model, entry):
    path = os.path.join(OUT, "media.json")
    data = json.load(open(path)) if os.path.exists(path) else {}
    data.setdefault(model, []).append(entry)
    json.dump(data, open(path, "w"), indent=1)


def run(m, graph, stem, prompt, first, warmup=False):
    g = json.loads(json.dumps(graph))
    node, field = m["prompt"]
    g[node]["inputs"][field] = prompt
    w, h = map(int, m["size"].split("x"))
    g[m["size_node"]]["inputs"].update({"width": w, "height": h})
    if warmup:
        # A few frames and steps: loads the model without a full clip.
        for n, inputs in m["warmup"].items():
            g[n]["inputs"].update(inputs)
    g[m["save"]]["inputs"]["filename_prefix"] = f"video/{m['name']}-{stem}"
    t = time.time()
    pid = call("/prompt", {"prompt": g})["prompt_id"]
    while True:
        h = call(f"/history/{pid}")
        if pid in h:
            break
        if time.time() - t > 3 * 3600:
            raise RuntimeError("over 3 hours")
        time.sleep(10)
    secs = time.time() - t
    status = h[pid].get("status", {})
    if status.get("status_str") != "success":
        raise RuntimeError(json.dumps(status.get("messages", []))[-600:])
    if warmup:
        record(m["name"], {"kind": "load", "seconds": round(secs, 1)})
        log(f"  warm-up with loading: {secs:.0f} s")
        return
    out = h[pid]["outputs"][m["save"]]
    item = (out.get("images") or out.get("videos") or out.get("gifs") or [None])[0]
    src = os.path.join(HERE, "ComfyUI", "output", item.get("subfolder", ""), item["filename"])
    name = f"{m['name']}-{stem}-{m['size']}.mp4"
    shutil.copy(src, os.path.join(OUT, "media", name))
    record(m["name"], {"kind": "video", "size": m["size"], "frames": m["frames"], "fps": m["fps"], "steps": m["steps"],
                       "prompt": prompt, "seconds": round(secs, 1), "first": first, "file": name,
                       "via": "ComfyUI default template"})
    log(f"  {m['size']}: {name} in {secs:.0f} s")


def main():
    for _ in range(120):
        try:
            call("/system_stats")
            break
        except Exception:
            time.sleep(5)
    else:
        sys.exit("ComfyUI didn't start")
    only = set(sys.argv[1:])
    for m in MODELS:
        if only & {x["name"] for x in MODELS} and m["name"] not in only:
            continue
        graph = json.load(open(os.path.join(HERE, m["template"])))
        log(f"== {m['name']} (ComfyUI default template)")
        clips = [c for c in CLIPS if c[0] in only] or CLIPS
        try:
            run(m, graph, "warmup", clips[0][1], False, warmup=True)
        except Exception as e:
            log(f"  warm-up failed: {e}")
        for i, (stem, prompt) in enumerate(clips):
            try:
                run(m, graph, stem, prompt, i == 0)
            except Exception as e:
                log(f"  {stem}: failed: {e}")
                record(m["name"], {"kind": "failed", "why": str(e)[:500]})
                break
        # Free the card for the next model (the answer is empty).
        try:
            call("/free", {"unload_models": True, "free_memory": True})
        except json.JSONDecodeError:
            pass


if __name__ == "__main__":
    main()
