#!/usr/bin/env python3
"""Write's AI check on Ollama (dev tool), to compare with LocalAI: the same
models, documents and settings as job.py's text test, thinking off.

    python3 ollama_text.py   # in /workspace/xb, with ollama installed
Results: results/text-ollama/<model>.md (+ replies), pull times in job.log.
"""
import os, subprocess, sys, time, urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from job import HERE, OUT, log  # noqa: E402

API = "http://127.0.0.1:11434"
MODELS = ["gemma4:26b", "gemma4:12b", "qwen3.8:27b", "qwen3.5:9b"]


def main():
    subprocess.run(["pkill", "-x", "ollama"], check=False)
    time.sleep(3)
    # Write sends a document's parts at once, so 4 requests in parallel.
    env = {**os.environ, "OLLAMA_NUM_PARALLEL": "4", "OLLAMA_CONTEXT_LENGTH": "16384",
           "OLLAMA_MODELS": os.path.join(HERE, "ollama")}
    server = subprocess.Popen(["ollama", "serve"], env=env, stdout=open(os.path.join(OUT, "ollama.log"), "a"),
                              stderr=subprocess.STDOUT)
    for _ in range(60):
        try:
            urllib.request.urlopen(API, timeout=5)
            break
        except Exception:
            time.sleep(2)
    out_dir = os.path.join(OUT, "text-ollama")
    os.makedirs(out_dir, exist_ok=True)
    for m in MODELS:
        log(f"== {m} (Ollama)")
        t = time.time()
        if subprocess.run(["ollama", "pull", m], env=env, capture_output=True).returncode != 0:
            log("  pull failed")
            continue
        log(f"  pulled in {time.time() - t:.0f} s")
        out = os.path.join(out_dir, m.replace(":", "-") + ".md")
        t = time.time()
        subprocess.run([os.path.join(HERE, "xenify-wp-corpus"), "bench", os.path.join(HERE, "test-set.jsonl"),
                        "--doc", "40", "--docs", "10", "--parts", "250", "--threads", "1", "--budget", "100",
                        "--models", m, "--url", API + "/v1", "-o", out, "--no-thinking"],
                       stdout=open(out + ".log", "w"), stderr=subprocess.STDOUT)
        log(f"  text ollama: {time.time() - t:.0f} s for 10 documents")
        subprocess.run(["ollama", "stop", m], env=env, capture_output=True)
        subprocess.run(["ollama", "rm", m], env=env, capture_output=True)  # the disk is needed for video
    server.terminate()


if __name__ == "__main__":
    main()
