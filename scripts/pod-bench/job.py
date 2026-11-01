#!/usr/bin/env python3
"""Runs on a rented GPU (dev tool): the AI models the way Xenify uses them.

LocalAI (the version Xenify Hub installs) on this machine; each model is
installed from LocalAI's own catalogue, tested, then deleted to keep the disk
free. Text: Write's AI check (xenify-wp-corpus bench --doc --parts, one
document at a time, as one person would). Pictures and video: fun prompts at
several sizes, each timed; the first run of a model also includes loading it.

    python3 job.py            # in the folder with xenify-wp-corpus, local-ai, test-set.jsonl
Results: results/ (text tables and replies, media/, media.json, log).
"""
import base64, json, os, re, subprocess, sys, time, urllib.error, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "results")
MEDIA = os.path.join(OUT, "media")
API = "http://127.0.0.1:8080"

TEXT = ["gemma-4-26b-a4b-it-qat", "gemma-4-12b-it-q4", "qwen3.8-27b", "qwen3.5-9b"]
# FLUX.1 schnell is gated on Hugging Face (LocalAI gets 401 without a token);
# FLUX.2 klein 4B is Apache 2.0 and its catalogue files are public.
IMAGE = ["z-image-turbo-diffusers", "flux.2-klein-4b"]
# LocalAI's cuda12-vllm-omni engine ships the processor-only PyTorch and a
# vLLM too old for its vLLM-Omni: Wan 2.2 needs the CUDA 13 build
# (CAPABILITY=nvidia-cuda-13). Wan 2.1 1.3B runs on stable-diffusion.cpp.
VIDEO = ["minimax-h3-fl2va-pruned-q4", "vllm-omni-wan2.2-t2v", "wan-2.1-t2v-1.3b-ggml"]

# The same prompts on every card, all realistic (an animation next to a
# realistic picture doesn't compare). One prompt per size.
PICTURES = [
    ("1024x1024", "baby", "A realistic photo of a chubby baby with a big curly moustache eating a slice of cherry pie with both hands, sitting on a patterned rug in a living room"),
    ("1536x1024", "gorilla", "A realistic photo of a baby holding a toy hammer chasing a gorilla down a busy city street, people watching from the pavement"),
    ("1024x1536", "dog", "A realistic photo of a golden retriever wearing a chef's hat, standing at a kitchen stove flipping pancakes in a pan"),
]
CLIPS = [
    ((832, 480), "baby", "Realistic video of a chubby baby with a big curly moustache sitting on a patterned rug eating cherry pie with both hands, smiling, crumbs falling"),
    ((832, 480), "gorilla", "Realistic video of a baby holding a toy hammer chasing a gorilla down a busy city street, the gorilla running away, people watching from the pavement"),
]


def log(*a):
    line = time.strftime("%H:%M:%S ") + " ".join(str(x) for x in a)
    print(line, flush=True)
    with open(os.path.join(OUT, "job.log"), "a") as f:
        f.write(line + "\n")


def call(method, path, body=None, timeout=3600):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            text = r.read().decode()
            return r.status, json.loads(text) if text.strip() else None
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:800]
    except Exception as e:
        return 0, str(e)[:800]


LOCALAI = None


def stop_localai():
    global LOCALAI
    if LOCALAI:
        LOCALAI.terminate()
        try:
            LOCALAI.wait(60)
        except subprocess.TimeoutExpired:
            LOCALAI.kill()
        LOCALAI = None
    subprocess.run(["pkill", "-f", "local-ai"], check=False)
    time.sleep(3)


def start_localai():
    global LOCALAI
    if call("GET", "/readyz", timeout=5)[0] == 200:
        return
    # Write sends a document's parts at once: LocalAI must answer requests
    # in parallel (one-model-at-a-time mode deadlocks on them: it tries to
    # unload the busy model to make room). Each test deletes its model after.
    # LocalAI picks an engine build from the CUDA it detects (here "12", from
    # the image's CUDA_VERSION although the driver does 13); vllm-cpp, which
    # MiniMax H3 uses, has only a CUDA 13 build, so it fell back to the
    # processor build and refused to load. Forcing "nvidia" picks the card's.
    env = {**os.environ, "LOCALAI_PARALLEL_REQUESTS": "true", "LLAMACPP_PARALLEL": "4",
           "LOCALAI_FORCE_META_BACKEND_CAPABILITY": os.environ.get("CAPABILITY", "nvidia")}
    LOCALAI = subprocess.Popen([os.path.join(HERE, "local-ai"), "run", "--models-path", os.path.join(HERE, "models"),
                      "--address", "127.0.0.1:8080"], env=env,
                     stdout=open(os.path.join(OUT, "localai.log"), "a"), stderr=subprocess.STDOUT)
    for _ in range(360):
        if call("GET", "/readyz", timeout=5)[0] == 200:
            log("LocalAI ready")
            return
        time.sleep(5)
    sys.exit("LocalAI didn't start")


def install(name):
    t = time.time()
    status, r = call("POST", "/models/apply", {"id": f"localai@{name}"})
    if status >= 300 or not isinstance(r, dict):
        raise RuntimeError(f"install refused ({status}): {r}")
    while True:
        _, j = call("GET", f"/models/jobs/{r['uuid']}", timeout=30)
        if isinstance(j, dict) and j.get("processed"):
            if j.get("error"):
                raise RuntimeError(f"install failed: {j['error']}")
            return time.time() - t
        if time.time() - t > 4 * 3600:
            raise RuntimeError("install took over 4 hours")
        time.sleep(15)


def roomy(name, tokens=16384):
    """Gives a text model enough context for Write's parts: LocalAI splits a
    model's context between its parallel slots, and catalogue entries are
    small, so a 250-word part plus the answer overflowed."""
    path = os.path.join(HERE, "models", name + ".yaml")
    if not os.path.exists(path):
        return
    lines = [l for l in open(path).read().splitlines() if not l.startswith("context_size:")]
    lines.insert(0, f"context_size: {tokens}")
    open(path, "w").write("\n".join(lines) + "\n")
    # LocalAI keeps the settings it read at install time: start it again.
    stop_localai()
    start_localai()


def unguided(name):
    """LocalAI's diffusers engine reads cfg_scale 0 as unset and uses 7, which
    burns out turbo models (Z-Image Turbo's catalogue entry says 0): 1 means
    no guidance there too."""
    path = os.path.join(HERE, "models", name + ".yaml")
    if not os.path.exists(path):
        return
    text = open(path).read()
    fixed = re.sub(r"(?m)^cfg_scale: 0(\.0)?$", "cfg_scale: 1", text)
    if fixed != text:
        open(path, "w").write(fixed)
        log("  cfg_scale 0 → 1")
        stop_localai()
        start_localai()


def remove(name):
    call("POST", f"/models/delete/{name}", timeout=120)


def save_media(r, stem):
    """The picture or clip in a LocalAI answer, saved; returns the file name."""
    item = (r.get("data") or [{}])[0] if isinstance(r, dict) else {}
    if item.get("b64_json"):
        raw = base64.b64decode(item["b64_json"])
    elif item.get("url"):
        url = item["url"] if item["url"].startswith("http") else API + "/" + item["url"].lstrip("/")
        raw = urllib.request.urlopen(url, timeout=600).read()
    else:
        raise RuntimeError(f"no picture in the answer: {str(r)[:300]}")
    ext = ".png" if raw[:4] == b"\x89PNG" else ".jpg" if raw[:2] == b"\xff\xd8" else ".webm" if raw[:4] == b"\x1aE\xdf\xa3" else ".mp4"
    name = stem + ext
    with open(os.path.join(MEDIA, name), "wb") as f:
        f.write(raw)
    return name


def results():
    path = os.path.join(OUT, "media.json")
    return json.load(open(path)) if os.path.exists(path) else {}


def record(model, entry):
    data = results()
    data.setdefault(model, []).append(entry)
    json.dump(data, open(os.path.join(OUT, "media.json"), "w"), indent=1)


def text(model):
    roomy(model)
    # Thinking off only: with it on, a part took minutes (Gemma 4 thought for
    # 284 s on one 250-word part on an A6000), shown on the first run.
    for variant, docs, extra in [("parts-nothink", "10", ["--no-thinking"])]:
        out = os.path.join(OUT, f"text-{variant}", model + ".md")
        os.makedirs(os.path.dirname(out), exist_ok=True)
        t = time.time()
        subprocess.run([os.path.join(HERE, "xenify-wp-corpus"), "bench", os.path.join(HERE, "test-set.jsonl"),
                        "--doc", "40", "--docs", docs, "--parts", "250", "--threads", "1", "--budget", "100",
                        "--models", model, "--url", API + "/v1", "-o", out, *extra],
                       stdout=open(out + ".log", "w"), stderr=subprocess.STDOUT)
        log(f"  text {variant}: {time.time() - t:.0f} s for {docs} documents")


def pictures(model):
    unguided(model)
    for i, (size, stem, prompt) in enumerate(PICTURES):
        t = time.time()
        status, r = call("POST", "/v1/images/generations",
                         {"model": model, "prompt": prompt, "size": size, "response_format": "b64_json"})
        secs = time.time() - t
        entry = {"kind": "picture", "size": size, "prompt": prompt, "seconds": round(secs, 1), "first": i == 0}
        try:
            if status >= 300:
                raise RuntimeError(f"{status}: {str(r)[:300]}")
            entry["file"] = save_media(r, f"{model}-{stem}-{size}")
        except Exception as e:
            entry["failed"] = str(e)[:300]
        record(model, entry)
        log(f"  {size}: {entry.get('file') or entry.get('failed')} in {secs:.0f} s")
    # Loading is part of the first picture: time the first size again, warm.
    size, stem, prompt = PICTURES[0]
    t = time.time()
    status, _ = call("POST", "/v1/images/generations", {"model": model, "prompt": prompt, "size": size, "response_format": "b64_json"})
    record(model, {"kind": "picture", "size": size, "warm_again": True, "seconds": round(time.time() - t, 1), "ok": status < 300})


def clips(model):
    for i, ((w, h), stem, prompt) in enumerate(CLIPS):
        t = time.time()
        status, r = call("POST", "/video", {"model": model, "prompt": prompt, "width": w, "height": h,
                                            "num_frames": 81, "fps": 16, "response_format": "b64_json"}, timeout=3 * 3600)
        secs = time.time() - t
        entry = {"kind": "video", "size": f"{w}x{h}", "frames": 81, "fps": 16, "prompt": prompt, "seconds": round(secs, 1), "first": i == 0}
        try:
            if status >= 300:
                raise RuntimeError(f"{status}: {str(r)[:300]}")
            entry["file"] = save_media(r, f"{model}-{stem}-{w}x{h}")
        except Exception as e:
            entry["failed"] = str(e)[:300]
        record(model, entry)
        log(f"  {w}x{h}: {entry.get('file') or entry.get('failed')} in {secs:.0f} s")


def main():
    os.makedirs(MEDIA, exist_ok=True)
    gpu = subprocess.run(["nvidia-smi", "--query-gpu=name,memory.total", "--format=csv,noheader"], capture_output=True, text=True).stdout.strip()
    log("card:", gpu)
    # nvidia-smi can list a card the driver can't use (cuInit 999 on a bad
    # host): LocalAI's engine then quietly runs on the processor instead.
    import ctypes
    try:
        cuda = ctypes.CDLL("libcuda.so.1")
        err, n = cuda.cuInit(0), ctypes.c_int()
        cuda.cuDeviceGetCount(ctypes.byref(n))
    except OSError as e:
        err, n = str(e), ctypes.c_int()
    if err != 0 or n.value == 0:
        log(f"the graphics card can't be used here (cuInit {err}, {n.value} devices): get another machine")
        sys.exit(1)
    start_localai()
    only = set(sys.argv[1:])
    for kind, models, run in [("text", TEXT, text), ("image", IMAGE, pictures), ("video", VIDEO, clips)]:
        for m in models:
            if only and m not in only and kind not in only:
                continue
            log(f"== {m}")
            try:
                secs = install(m)
                log(f"  installed in {secs:.0f} s")
                record(m, {"kind": "install", "seconds": round(secs), "card": gpu})
                run(m)
            except Exception as e:
                log(f"  failed: {e}")
                record(m, {"kind": "failed", "why": str(e)[:500], "card": gpu})
            remove(m)
    log("done")


if __name__ == "__main__":
    main()
