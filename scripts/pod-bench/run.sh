#!/usr/bin/env bash
# The whole GPU run on a pod (dev tool): installs LocalAI, Ollama and ComfyUI
# with the models, then text on LocalAI and Ollama, pictures and video through
# ComfyUI's API. Started by start.sh; progress in results/job.log, which ends
# with "ALL DONE". Everything worth keeping is under results/.
# `run.sh pictures` installs ComfyUI with the picture models only and makes
# just the pictures; `run.sh minimax` does the same for MiniMax H3's clips.
cd "$(dirname "$0")"
mkdir -p results/media
log() { echo "$(date +%T) $*" >> results/job.log; }
log "card: $(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader), RAM $(free -g | awk '/Mem/{print $2}') GB"
python3 -c "import ctypes,sys; c=ctypes.CDLL('libcuda.so.1'); sys.exit(c.cuInit(0))" || { log "card can't be used (cuInit fails): get another pod"; log "ALL DONE"; exit 1; }

if [ "$1" = minimax ]; then
    ONLY=minimax bash comfy_setup.sh > results/comfy-setup.log 2>&1
    log "setup done: $(du -sh ComfyUI/models | cut -f1) of ComfyUI models"
    until curl -sf -o /dev/null 127.0.0.1:8188/queue; do sleep 2; done
    python3 comfy_minimax.py minimax-h3-comfyui >> results/run.out 2>&1
    log "ALL DONE"; exit 0
fi
if [ "$1" = pictures ]; then
    ONLY=pictures bash comfy_setup.sh > results/comfy-setup.log 2>&1
    log "setup done: $(du -sh ComfyUI/models | cut -f1) of ComfyUI models"
    until curl -sf -o /dev/null 127.0.0.1:8188/queue; do sleep 2; done
    python3 comfy_pictures.py >> results/run.out 2>&1
    log "ALL DONE"; exit 0
fi

( curl -fsSL -o local-ai https://github.com/mudler/LocalAI/releases/download/v4.12.1/local-ai-v4.12.1-linux-amd64 && chmod +x local-ai ) &
( command -v ollama >/dev/null || curl -fsSL https://ollama.com/install.sh | sh > results/ollama-install.log 2>&1 ) &
bash comfy_setup.sh > results/comfy-setup.log 2>&1 &
wait
log "setup done: $(du -sh ComfyUI/models | cut -f1) of ComfyUI models"

pkill -x ollama
python3 job.py text >> results/run.out 2>&1
pkill -f "[l]ocal-ai run"; sleep 3
python3 ollama_text.py >> results/run.out 2>&1
pkill -x ollama; sleep 3
python3 comfy_pictures.py >> results/run.out 2>&1
python3 comfy_minimax.py >> results/run.out 2>&1
log "ALL DONE"
