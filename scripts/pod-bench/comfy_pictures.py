#!/usr/bin/env python3
"""Pictures through ComfyUI's API (dev tool), the way the server's ComfyUI
connection will make them. Z-Image Turbo uses ComfyUI's own benchmark
template (benchmarks/image_z_image_turbo.json); FLUX.2 klein 4B is the
distilled text-to-image subgraph of templates/image_flux2_klein_text_to_image.json
written out as an API graph (4 steps, CFG 1).

    python3 comfy_pictures.py   # in /workspace/xb, ComfyUI on 127.0.0.1:8188
Results: results/media/<model>-<prompt>-<size>.png, entries in results/media.json.
"""
import json, os, shutil, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from comfy_minimax import HERE, OUT, call, log, record  # noqa: E402
from job import PICTURES  # noqa: E402


def klein(prompt, w, h, stem, seed=0):
    return {
        "70": {"class_type": "UNETLoader", "inputs": {"unet_name": "flux-2-klein-4b.safetensors", "weight_dtype": "default"}},
        "71": {"class_type": "CLIPLoader", "inputs": {"clip_name": "qwen_3_4b.safetensors", "type": "flux2", "device": "default"}},
        "72": {"class_type": "VAELoader", "inputs": {"vae_name": "flux2-vae.safetensors"}},
        "74": {"class_type": "CLIPTextEncode", "inputs": {"text": prompt, "clip": ["71", 0]}},
        "76": {"class_type": "ConditioningZeroOut", "inputs": {"conditioning": ["74", 0]}},
        "63": {"class_type": "CFGGuider", "inputs": {"cfg": 1, "model": ["70", 0], "positive": ["74", 0], "negative": ["76", 0]}},
        "61": {"class_type": "KSamplerSelect", "inputs": {"sampler_name": "euler"}},
        "62": {"class_type": "Flux2Scheduler", "inputs": {"steps": 4, "width": w, "height": h}},
        "66": {"class_type": "EmptyFlux2LatentImage", "inputs": {"width": w, "height": h, "batch_size": 1}},
        "73": {"class_type": "RandomNoise", "inputs": {"noise_seed": seed}},
        "64": {"class_type": "SamplerCustomAdvanced", "inputs": {"noise": ["73", 0], "guider": ["63", 0], "sampler": ["61", 0],
                                                                   "sigmas": ["62", 0], "latent_image": ["66", 0]}},
        "65": {"class_type": "VAEDecode", "inputs": {"samples": ["64", 0], "vae": ["72", 0]}},
        "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": stem, "images": ["65", 0]}},
    }


def zimage(prompt, w, h, stem, seed=0):
    g = json.load(open(os.path.join(HERE, "image_z_image_turbo.json")))
    g["57:27"]["inputs"]["text"] = prompt
    g["57:13"]["inputs"].update({"width": w, "height": h})
    g["57:3"]["inputs"]["seed"] = seed
    g["9"]["inputs"]["filename_prefix"] = stem
    return g


MODELS = [("flux.2-klein-4b", klein), ("z-image-turbo", zimage)]


def make(graph):
    t = time.time()
    pid = call("/prompt", {"prompt": graph})["prompt_id"]
    while pid not in (h := call(f"/history/{pid}")):
        time.sleep(1)
    status = h[pid].get("status", {})
    if status.get("status_str") != "success":
        raise RuntimeError(json.dumps(status.get("messages", []))[-600:])
    item = h[pid]["outputs"]["9"]["images"][0]
    return time.time() - t, os.path.join(HERE, "ComfyUI", "output", item.get("subfolder", ""), item["filename"])


def main():
    for name, build in MODELS:
        log(f"== {name} (pictures)")
        # A warm-up picture first, so loading the model isn't in any timed
        # picture (it's recorded on its own as the loading time).
        size, stem, prompt = PICTURES[0]
        w, h = map(int, size.split("x"))
        try:
            secs, _ = make(build(prompt, w, h, f"{name}-warmup", seed=1))
            record(name, {"kind": "load", "size": size, "seconds": round(secs, 1)})
            log(f"  warm-up with loading: {secs:.1f} s")
        except Exception as e:
            log(f"  warm-up failed: {e}")
        for size, stem, prompt in PICTURES:
            w, h = map(int, size.split("x"))
            try:
                secs, src = make(build(prompt, w, h, f"{name}-{stem}-{size}"))
            except Exception as e:
                log(f"  {size}: failed: {e}")
                record(name, {"kind": "failed", "why": str(e)[:500]})
                break
            file = f"{name}-{stem}-{size}.png"
            shutil.copy(src, os.path.join(OUT, "media", file))
            record(name, {"kind": "picture", "size": size, "prompt": prompt, "seconds": round(secs, 1), "file": file})
            log(f"  {size}: {file} in {secs:.1f} s")
        try:
            call("/free", {"unload_models": True, "free_memory": True})
        except json.JSONDecodeError:
            pass


if __name__ == "__main__":
    main()
