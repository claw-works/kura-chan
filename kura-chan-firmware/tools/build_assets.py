#!/usr/bin/env python3
# Resample master character PNGs (any size) down to the device screen size with
# high-quality LANCZOS (alpha preserved), so the device draws 1:1 (crisp).
# Usage: build_assets.py <src_dir> <dst_dir> [screenW screenH]
import os, sys
from PIL import Image
src, dst = sys.argv[1], sys.argv[2]
SW = int(sys.argv[3]) if len(sys.argv) > 3 else 320
SH = int(sys.argv[4]) if len(sys.argv) > 4 else 240
os.makedirs(dst, exist_ok=True)
pngs = sorted(f for f in os.listdir(src) if f.lower().endswith(".png"))
# uniform scale from the first image (all layers share the master canvas)
im0 = Image.open(os.path.join(src, pngs[0]))
W, H = im0.size
scale = min(SW / W, SH / H)
tw, th = max(1, round(W * scale)), max(1, round(H * scale))
print(f"master {W}x{H} -> device {tw}x{th} (scale {scale:.3f})")
for f in pngs:
    im = Image.open(os.path.join(src, f)).convert("RGBA")
    if im.size != (W, H):
        im = im.resize((W, H), Image.LANCZOS)  # normalize odd layers to master
    out = im.resize((tw, th), Image.LANCZOS)
    out.save(os.path.join(dst, f), optimize=True)
print(f"wrote {len(pngs)} files to {dst}")
