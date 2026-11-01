#!/usr/bin/env bash
# Sends the GPU run to a RunPod pod and starts it (dev tool).
#
#   scripts/pod-bench/start.sh root@1.2.3.4 12345     # from the pod's "SSH over exposed TCP" line
#   scripts/pod-bench/start.sh root@1.2.3.4 12345 pictures   # pictures only
#   scripts/pod-bench/start.sh root@1.2.3.4 12345 minimax    # MiniMax H3 clips only
#
# Needs target/release/xenify-wp-corpus (cargo build --release -p xenify-wp-corpus)
# and target/pod-bench/test-set.jsonl. Copy results back with:
#   ssh -p PORT -i ssh/id_ed25519 HOST 'cd /workspace/xb/results && tar cf - .' | tar xf - -C target/pod-NAME
set -e
host=$1 port=$2 only=$3
cd "$(dirname "$0")/../.."
key=ssh/id_ed25519
opts=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new -i "$key")
ssh "${opts[@]}" -p "$port" "$host" 'mkdir -p /workspace/xb'
scp -q "${opts[@]}" -P "$port" scripts/pod-bench/{run.sh,job.py,ollama_text.py,comfy_setup.sh,comfy_pictures.py,comfy_minimax.py} \
    target/release/xenify-wp-corpus target/pod-bench/test-set.jsonl "$host:/workspace/xb/"
ssh "${opts[@]}" -p "$port" "$host" "cd /workspace/xb && (setsid nohup bash run.sh $only > run.log 2>&1 < /dev/null &) && sleep 5 && cat results/job.log"
