#!/usr/bin/env python3
"""Assemble the captured connection frames into docs/img/connect.gif.

Run after tools/capture-docs.mjs. Requires Pillow. The capture uses the browser mock,
so this script never touches the system proxy or creates a VPN connection.

The app icon and the installer art are not made here: they come from tools/gen_icon.py
(the 3x3 block) and tools/brand.py (the cube render), see docs/DEVELOPMENT.md.
"""
from pathlib import Path
from PIL import Image
import shutil

ROOT = Path(__file__).resolve().parent.parent
IMG = ROOT / "docs" / "img"
FRAMES = ROOT / "docs" / ".frames"


def resample():
    return Image.Resampling.LANCZOS


def make_gif():
    paths = sorted(FRAMES.glob("frame-*.png"))
    if not paths:
        raise RuntimeError("No captured frames found; run node tools/capture-docs.mjs first")
    frames = []
    for path in paths:
        frame = Image.open(path).convert("RGB").resize((960, 600), resample())
        frames.append(frame.quantize(colors=112, method=Image.Quantize.MEDIANCUT))
    frames[0].save(
        IMG / "connect.gif",
        save_all=True,
        append_images=frames[1:],
        duration=140,
        loop=0,
        optimize=True,
        disposal=2,
    )
    print(f"assembled docs/img/connect.gif ({len(frames)} frames)")


if __name__ == "__main__":
    make_gif()
    shutil.rmtree(FRAMES, ignore_errors=True)
