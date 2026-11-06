#!/usr/bin/env python3
"""Assemble the captured connection frames and generate Tauri's icon/installer assets.

Run after tools/capture-docs.mjs. Requires Pillow. The capture uses the browser mock,
so this script never touches the system proxy or creates a VPN connection.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
import shutil

ROOT = Path(__file__).resolve().parent.parent
IMG = ROOT / "docs" / "img"
FRAMES = ROOT / "docs" / ".frames"
ICONS = ROOT / "src-tauri" / "icons"
INSTALLER = ROOT / "src-tauri" / "installer"
GREEN = (61, 220, 132)
DARK = (8, 13, 10)


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


def make_icons():
    source = Image.open(IMG / "logo.png").convert("RGBA")
    if source.width < 512:
        raise RuntimeError("The logo must be at least 512 px")
    sizes = {
        "icon.png": 512,
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "StoreLogo.png": 50,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
    }
    for name, size in sizes.items():
        source.resize((size, size), resample()).save(ICONS / name, optimize=True)
    source.save(ICONS / "icon.ico", format="ICO", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    source.save(ICONS / "icon.icns", format="ICNS", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512)])
    print("generated Tauri PNG, ICO and ICNS icons")


def font(size, bold=False):
    candidates = [
        Path("C:/Windows/Fonts/segoeuib.ttf" if bold else "C:/Windows/Fonts/segoeui.ttf"),
        Path("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf" if bold else "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ]
    for candidate in candidates:
        if candidate.exists():
            return ImageFont.truetype(str(candidate), size)
    return ImageFont.load_default()


def make_installer():
    logo = Image.open(IMG / "logo.png").convert("RGBA")
    header_path = INSTALLER / "header.bmp"
    sidebar_path = INSTALLER / "sidebar.bmp"

    old = Image.open(header_path)
    header = Image.new("RGB", old.size, DARK)
    draw = ImageDraw.Draw(header)
    mark = logo.resize((old.height - 10, old.height - 10), resample())
    header.paste(mark, (5, 5), mark)
    draw.text((old.height + 5, old.height // 2), "QUADRA", font=font(max(14, old.height // 3), True), fill=(230, 241, 234), anchor="lm")
    draw.rectangle((0, old.height - 3, old.width, old.height), fill=GREEN)
    header.save(header_path, format="BMP")

    old = Image.open(sidebar_path)
    sidebar = Image.new("RGB", old.size, DARK)
    draw = ImageDraw.Draw(sidebar)
    for y in range(0, old.height, 18):
        for x in range(0, old.width, 18):
            draw.ellipse((x, y, x + 1, y + 1), fill=(28, 52, 37))
    mark_size = min(old.width - 34, 126)
    mark = logo.resize((mark_size, mark_size), resample())
    sidebar.paste(mark, ((old.width - mark_size) // 2, 35), mark)
    draw.text((old.width // 2, 184), "QUADRA", font=font(25, True), fill=(230, 241, 234), anchor="mm")
    draw.text((old.width // 2, 213), "FAST · OPEN · YOURS", font=font(9), fill=(97, 127, 108), anchor="mm")
    draw.rectangle((0, old.height - 4, old.width, old.height), fill=GREEN)
    sidebar.save(sidebar_path, format="BMP")
    print("generated branded NSIS installer images")


if __name__ == "__main__":
    make_gif()
    make_icons()
    make_installer()
    shutil.rmtree(FRAMES, ignore_errors=True)
