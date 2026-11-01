"""Writes model.f16.safetensors (half-size download) next to a trained model.safetensors.

    python3 tof16.py MODEL_DIR
"""
import sys
import torch
from safetensors.torch import load_file, save_file

d = sys.argv[1]
w = load_file(f"{d}/model.safetensors")
save_file({k: v.half() if v.is_floating_point() else v for k, v in w.items()}, f"{d}/model.f16.safetensors", metadata={"format": "pt"})
print("wrote", f"{d}/model.f16.safetensors")
