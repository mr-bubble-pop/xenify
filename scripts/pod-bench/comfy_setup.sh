#!/usr/bin/env bash
# Installs ComfyUI and the files its MiniMax H3, Wan 2.2 14B, FLUX.2 klein 4B and
# Z-Image Turbo templates load (about 110 GB), then starts it on 127.0.0.1:8188. Dev tool for the pod.
# ONLY=pictures skips the video models; ONLY=minimax fetches MiniMax H3 only.
set -e
cd "$(dirname "$0")"
[ -d ComfyUI ] || git clone -q --depth 1 https://github.com/comfyanonymous/ComfyUI
cd ComfyUI
[ -d venv ] || uv venv -q -p 3.12 venv
# cu128 builds leave out Volta (V100, compute 7.0) and need a newer driver; cu126 runs on both.
cu=cu128; [ "$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)" -lt 75 ] && cu=cu126
uv pip install -q --python venv/bin/python --index-url https://download.pytorch.org/whl/$cu torch torchvision torchaudio
uv pip install -q --python venv/bin/python -r requirements.txt
hf=https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/main
get() { [ -s "models/$2/$(basename "$1")" ] || curl -sfL -o "models/$2/$(basename "$1")" "$hf/$1"; }
mkdir -p models/diffusion_models models/text_encoders models/vae models/loras
if [ "$ONLY" != pictures ]; then
get diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors diffusion_models &
get text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors text_encoders &
get vae/minimax_h3_video_vae_int8_convrot.safetensors vae &
get vae/minimax_h3_audio_vae_fp32.safetensors vae &
# The template also loads the turbo LoRA (switched off by default).
get loras/minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors loras &
fi
if [ -z "$ONLY" ]; then
wan=https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files
getw() { [ -s "models/$2/$(basename "$1")" ] || curl -sfL -o "models/$2/$(basename "$1")" "$wan/$1"; }
getw diffusion_models/wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors diffusion_models &
getw diffusion_models/wan2.2_t2v_low_noise_14B_fp8_scaled.safetensors diffusion_models &
getw text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors text_encoders &
getw vae/wan_2.1_vae.safetensors vae &
getw loras/wan2.2_t2v_lightx2v_4steps_lora_v1.1_high_noise.safetensors loras &
getw loras/wan2.2_t2v_lightx2v_4steps_lora_v1.1_low_noise.safetensors loras &
fi
if [ "$ONLY" != minimax ]; then
geturl() { [ -s "models/$2/$(basename "$1")" ] || curl -sfL -o "models/$2/$(basename "$1")" "https://huggingface.co/$1"; }
geturl Comfy-Org/flux2-klein/resolve/main/split_files/diffusion_models/flux-2-klein-4b.safetensors diffusion_models &
geturl Comfy-Org/flux2-klein/resolve/main/split_files/text_encoders/qwen_3_4b.safetensors text_encoders &
geturl Comfy-Org/flux2-dev/resolve/main/split_files/vae/flux2-vae.safetensors vae &
geturl Comfy-Org/z_image_turbo/resolve/main/split_files/diffusion_models/z_image_turbo_bf16.safetensors diffusion_models &
geturl Comfy-Org/z_image_turbo/resolve/main/split_files/vae/ae.safetensors vae &
fi
wait
for t in video_minimax_h3_t2v video_wan2_2_14B_t2v image_z_image_turbo; do
    curl -sfL -o ../$t.json https://raw.githubusercontent.com/Comfy-Org/workflow_templates/main/benchmarks/$t.json
done
setsid nohup venv/bin/python main.py --listen 127.0.0.1 --port 8188 > ../results/comfyui.log 2>&1 < /dev/null &
