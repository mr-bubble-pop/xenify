#!/usr/bin/env python3
"""Times the suggested AI models on rented GPUs (dev tool, stdlib only).

For each size of graphics card asked for, rents whatever RunPod has free in
that size, runs LocalAI there (its own GPU image, the version Xenify Hub
installs), installs each model through LocalAI's own catalogue and times it:
the first answer (which includes loading) and warm answers. Text models also
run the whole-document grammar test (xenify-wp-corpus bench --doc). The pod
is always deleted at the end, even when something fails.

    RUNPOD_API_KEY in .env (read here, never printed)
    scripts/gpu-bench.py 12 24 80          # card sizes in GB
    scripts/gpu-bench.py --list            # what's free now, by size

Results: target/gpu-bench/<size>gb-<card>.json and summary.md.
"""
import json, os, re, secrets, subprocess, sys, time, urllib.error, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "target", "gpu-bench")
IMAGE = "localai/localai:v4.12.1-gpu-nvidia-cuda-12"
SIZES = [12, 16, 24, 32, 48, 80]

# (catalogue name, kind, smallest card in GB worth trying). Too-big models are
# still tried one size down: a failure there is worth knowing too.
MODELS = [
    ("qwen3.5-9b", "text", 12),
    ("gpt-oss-20b", "text", 16),
    ("qwen_qwen3.5-35b-a3b", "text", 24),
    ("z-image-turbo-diffusers", "image", 12),
    ("flux.1-schnell", "image", 16),
    ("minimax-h3-fl2va-pruned-q4", "video", 12),
    ("vllm-omni-wan2.2-t2v", "video", 24),
]
PROMPTS = {
    "image": "A red fox reading a newspaper on a park bench, morning light",
    "video": "A paper boat drifting down a rainy street, close up",
}


def env_key(name):
    for line in open(os.path.join(ROOT, ".env")):
        m = re.match(r"\s*%s\s*=\s*(.*)" % name, line)
        if m:
            return m.group(1).strip().strip('"')
    sys.exit(f"{name} isn't in .env")


def call(method, url, body=None, headers=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json", **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            text = r.read().decode()
            return r.status, (json.loads(text) if text.strip() else None)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:500]


class RunPod:
    def __init__(self, key):
        self.h = {"Authorization": f"Bearer {key}"}

    def gql(self, query):
        status, r = call("POST", "https://api.runpod.io/graphql", {"query": query}, self.h)
        if status != 200 or not isinstance(r, dict) or r.get("errors"):
            raise RuntimeError(f"RunPod said {status}: {r}")
        return r["data"]

    def cards(self):
        """{size: [(price, id, name)]} for cards with stock now, cheapest first."""
        d = self.gql("{ gpuTypes { id displayName memoryInGb lowestPrice(input: {gpuCount: 1}) { stockStatus uninterruptablePrice } } }")
        out = {}
        for g in d["gpuTypes"]:
            lp = g.get("lowestPrice") or {}
            if not lp.get("stockStatus") or not lp.get("uninterruptablePrice"):
                continue
            size = next((s for s in SIZES if g["memoryInGb"] <= s), None)
            if size:
                out.setdefault(size, []).append((lp["uninterruptablePrice"], g["id"], g["displayName"], g["memoryInGb"]))
        for v in out.values():
            v.sort()
        return out

    def start(self, ids, api_key):
        body = {
            "name": "xenify-gpu-bench", "imageName": IMAGE, "gpuTypeIds": ids, "gpuCount": 1,
            "gpuTypePriority": "availability", "cloudType": "ALL", "containerDiskInGb": 250,
            "ports": ["8080/http"],
            "env": {"LOCALAI_API_KEY": api_key, "LOCALAI_SINGLE_ACTIVE_BACKEND": "true"},
        }
        status, r = call("POST", "https://rest.runpod.io/v1/pods", body, self.h)
        if status >= 300:
            raise RuntimeError(f"couldn't rent a card ({status}): {r}")
        return r

    def stop(self, pod_id):
        return call("DELETE", f"https://rest.runpod.io/v1/pods/{pod_id}", None, self.h)[0]


class LocalAI:
    def __init__(self, base, key):
        self.base, self.h = base, {"Authorization": f"Bearer {key}"}

    def wait(self, limit=1800):
        t = time.time()
        while time.time() - t < limit:
            try:
                if call("GET", f"{self.base}/readyz", timeout=10)[0] == 200:
                    return time.time() - t
            except Exception:
                pass
            time.sleep(10)
        raise RuntimeError("LocalAI didn't start")

    def install(self, name, limit=3600):
        status, r = call("POST", f"{self.base}/models/apply", {"id": f"localai@{name}"}, self.h)
        if status >= 300:
            raise RuntimeError(f"install refused ({status}): {r}")
        t = time.time()
        while time.time() - t < limit:
            _, j = call("GET", f"{self.base}/models/jobs/{r['uuid']}", None, self.h)
            if isinstance(j, dict) and j.get("processed"):
                if j.get("error"):
                    raise RuntimeError(f"install failed: {j['error']}")
                return time.time() - t
            time.sleep(10)
        raise RuntimeError("install took too long")

    def ask(self, kind, model):
        if kind == "text":
            path, body = "/v1/chat/completions", {"model": model, "messages": [{"role": "user", "content": "Say hello in five words."}]}
        elif kind == "image":
            path, body = "/v1/images/generations", {"model": model, "prompt": PROMPTS["image"], "size": "1024x1024"}
        else:
            path, body = "/video", {"model": model, "prompt": PROMPTS["video"]}
        t = time.time()
        status, r = call("POST", self.base + path, body, self.h, timeout=3600)
        if status >= 300:
            raise RuntimeError(f"{status}: {str(r)[:300]}")
        return time.time() - t


def bench_card(rp, size, cards):
    ids = [c[1] for c in cards[:6]]
    api_key = secrets.token_urlsafe(24)
    pod = rp.start(ids, api_key)
    pod_id = pod["id"]
    result = {"size": size, "pod": pod_id, "asked": ids, "models": {}}
    try:
        base = f"https://{pod_id}-8080.proxy.runpod.net"
        ai = LocalAI(base, api_key)
        result["start_seconds"] = round(ai.wait())
        _, info = call("GET", f"https://rest.runpod.io/v1/pods/{pod_id}", None, rp.h)
        if isinstance(info, dict):
            result["card"] = (info.get("machine") or {}).get("gpuDisplayName") or info.get("gpuTypeId") or "?"
            result["price"] = info.get("costPerHr")
        print(f"{size} GB: {result.get('card')} ready after {result['start_seconds']} s", flush=True)
        for name, kind, smallest in MODELS:
            if size < smallest and SIZES.index(size) + 1 < SIZES.index(smallest):
                continue
            m = result["models"][name] = {"kind": kind}
            try:
                m["install_seconds"] = round(ai.install(name))
                m["first_seconds"] = round(ai.ask(kind, name), 1)
                warm = [ai.ask(kind, name) for _ in range(2)]
                m["warm_seconds"] = round(sorted(warm)[0], 1)
                if kind == "text":
                    md = os.path.join(OUT, f"{size}gb-{name}-doc40.md")
                    subprocess.run([os.path.join(ROOT, "target/release/xenify-wp-corpus"), "bench", "--doc", "40", "--docs", "3",
                                    "--models", name, "--threads", "1", "--url", f"{base}/v1", "--key-env", "BENCH_LOCALAI_KEY",
                                    "-o", md], cwd=ROOT, env={**os.environ, "BENCH_LOCALAI_KEY": api_key}, check=False)
                    m["doc40"] = md
                print(f"  {name}: {m}", flush=True)
            except Exception as e:
                m["failed"] = str(e)[:300]
                print(f"  {name}: failed: {m['failed']}", flush=True)
            save(result)
    finally:
        status = rp.stop(pod_id)
        print(f"  pod {pod_id} deleted ({status})", flush=True)
    return result


def save(result):
    os.makedirs(OUT, exist_ok=True)
    name = re.sub(r"[^a-z0-9]+", "-", str(result.get("card", "card")).lower())
    with open(os.path.join(OUT, f"{result['size']}gb-{name}.json"), "w") as f:
        json.dump(result, f, indent=1)


def main():
    rp = RunPod(env_key("RUNPOD_API_KEY"))
    cards = rp.cards()
    if "--list" in sys.argv:
        for s in SIZES:
            print(f"{s} GB:", ", ".join(f"{n} ${p:.2f}/h" for p, _, n, _ in cards.get(s, [])) or "none free")
        return
    for s in [int(a) for a in sys.argv[1:]]:
        if not cards.get(s):
            print(f"{s} GB: no card free now, skipped")
            continue
        try:
            save(bench_card(rp, s, cards[s]))
        except Exception as e:
            print(f"{s} GB: {e}")


if __name__ == "__main__":
    main()
