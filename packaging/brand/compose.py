#!/usr/bin/env python3
"""Compose the macOS-port lockups and rasterise the app icon.

Keeps the original Lightforge artwork and adds a text 'for macOS' / 'Mac'
indicator. No Apple glyphs. Text is outlined to SVG paths so the sources
do not depend on fonts at render time.

Run from anywhere:

    python3 packaging/brand/compose.py
"""
from __future__ import annotations

import base64
import math
import subprocess
import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[2]
BRAND = Path(__file__).resolve().parent
WORKMARK = BRAND / "wordmark-mark.png"
BANNER_BASE = BRAND / "banner-base.png"
ICON_SVG = ROOT / "packaging/icons/hicolor/scalable/apps/pro.tuxcore.Lightforge.svg"
HICOLOR = ROOT / "packaging/icons/hicolor"

FONT_SEMIBOLD = Path("/usr/share/fonts/truetype/macos/Inter-SemiBold.ttf")
FONT_BOLD = Path("/usr/share/fonts/truetype/macos/Inter-Bold.ttf")

# Wordmark letters in the PNG are #BF9A00; the badge uses the Play-button gold
# so it reads as a label, not as more of the same type.
LETTER_GOLD = "#BF9A00"
BADGE_GOLD = "#E9AB28"
BADGE_TEXT = "#1D2021"

ICON_SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)


def _font_paths(font_path: Path, text: str, size_px: float, tracking_em: float):
    font = TTFont(font_path)
    upem = font["head"].unitsPerEm
    scale = size_px / upem
    cmap = font.getBestCmap()
    gs = font.getGlyphSet()
    hmtx = font["hmtx"]
    os2 = font["OS/2"]
    asc = os2.sTypoAscender * scale
    desc = os2.sTypoDescender * scale
    x = 0.0
    paths: list[str] = []
    for i, ch in enumerate(text):
        name = cmap.get(ord(ch))
        if name is None:
            x += size_px * 0.3
            continue
        pen = SVGPathPen(gs)
        tpen = TransformPen(pen, (scale, 0, 0, -scale, x, asc))
        gs[name].draw(tpen)
        d = pen.getCommands()
        if d:
            paths.append(d)
        adv, _ = hmtx[name]
        x += adv * scale
        if i != len(text) - 1:
            x += tracking_em * size_px
    return paths, x, asc - desc


def _path_el(paths: list[str], fill: str) -> str:
    return "\n    ".join(f'<path fill="{fill}" d="{d}"/>' for d in paths)


def _data_uri(path: Path) -> str:
    return "data:image/png;base64," + base64.b64encode(path.read_bytes()).decode("ascii")


def _rasterize(svg: Path, png: Path, width: int, height: int) -> None:
    png.parent.mkdir(parents=True, exist_ok=True)
    subprocess.check_call(
        ["rsvg-convert", "-w", str(width), "-h", str(height), "-o", str(png), str(svg)]
    )


def write_wordmark() -> tuple[int, int]:
    """Original mark with a 'for macOS' pill to its right — readable at 28px."""
    from PIL import Image

    mark = Image.open(WORKMARK)
    wm_w, wm_h = mark.size
    paths, tw, th = _font_paths(FONT_BOLD, "for macOS", 22.0, 0.08)
    pad_x, pad_y = 12.0, 6.5
    pill_w = tw + pad_x * 2
    pill_h = th + pad_y * 2
    gap = 18.0
    canvas_w = wm_w + int(math.ceil(gap + pill_w + 2))
    canvas_h = wm_h
    pill_x = wm_w + gap
    pill_y = (canvas_h - pill_h) / 2
    rx = pill_h / 2
    tx = pill_x + pad_x
    ty = pill_y + pad_y
    svg = f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     width="{canvas_w}" height="{canvas_h}" viewBox="0 0 {canvas_w} {canvas_h}">
  <title>Lightforge for macOS</title>
  <image x="0" y="0" width="{wm_w}" height="{wm_h}" href="{_data_uri(WORKMARK)}"/>
  <g id="macos-badge" aria-label="for macOS">
    <rect x="{pill_x:.3f}" y="{pill_y:.3f}" width="{pill_w:.3f}" height="{pill_h:.3f}"
          rx="{rx:.3f}" fill="{BADGE_GOLD}"/>
    <g transform="translate({tx:.3f} {ty:.3f})">
    {_path_el(paths, BADGE_TEXT)}
    </g>
  </g>
</svg>
'''
    dest_svg = ROOT / "crates/launcher-gui/assets/wordmark.svg"
    dest_png = ROOT / "crates/launcher-gui/assets/wordmark.png"
    dest_svg.write_text(svg)
    _rasterize(dest_svg, dest_png, canvas_w, canvas_h)
    print(f"wordmark {canvas_w}x{canvas_h} -> {dest_png.relative_to(ROOT)}")
    return canvas_w, canvas_h


def write_banner() -> None:
    """Original banner with 'for macOS' centred under the existing mark."""
    paths, tw, _th = _font_paths(FONT_SEMIBOLD, "for macOS", 28.0, 0.22)
    tx = (1280 - tw) / 2
    # Original ink ends at y=174; sit the caption in the space below.
    ty = 186.0
    svg = f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     width="1280" height="300" viewBox="0 0 1280 300">
  <title>Lightforge for macOS</title>
  <image x="0" y="0" width="1280" height="300" href="{_data_uri(BANNER_BASE)}"/>
  <g id="macos-caption" aria-label="for macOS" transform="translate({tx:.3f} {ty:.3f})">
    {_path_el(paths, LETTER_GOLD)}
  </g>
</svg>
'''
    dest_svg = ROOT / "docs/banner.svg"
    dest_png = ROOT / "docs/banner.png"
    dest_svg.write_text(svg)
    _rasterize(dest_svg, dest_png, 1280, 300)
    print(f"banner 1280x300 -> {dest_png.relative_to(ROOT)}")


def _icon_badge_fragment() -> str:
    """Compact 'Mac' pill, bottom-right of the circular icon."""
    paths, tw, th = _font_paths(FONT_BOLD, "Mac", 16.0, 0.04)
    view = 188.99953
    pad_x, pad_y = 7.0, 3.5
    pill_w = tw + pad_x * 2
    pill_h = th + pad_y * 2
    pill_x = view - 18 - pill_w
    pill_y = view - 20 - pill_h
    rx = pill_h / 2
    tx = pill_x + pad_x
    ty = pill_y + pad_y
    return f'''
  <g id="mac-badge" aria-label="Mac">
    <rect x="{pill_x:.4f}" y="{pill_y:.4f}" width="{pill_w:.4f}" height="{pill_h:.4f}"
          rx="{rx:.4f}" fill="{BADGE_GOLD}"/>
    <g transform="translate({tx:.4f} {ty:.4f})">
    {_path_el(paths, BADGE_TEXT)}
    </g>
  </g>
'''


def ensure_icon_badge() -> None:
    svg = ICON_SVG.read_text()
    if 'id="mac-badge"' in svg:
        print("icon svg already has mac-badge")
        return
    stripped = svg.rstrip()
    if not stripped.endswith("</svg>"):
        sys.exit(f"unexpected SVG tail in {ICON_SVG}")
    ICON_SVG.write_text(stripped[: -len("</svg>")] + _icon_badge_fragment() + "</svg>\n")
    print(f"icon badge -> {ICON_SVG.relative_to(ROOT)}")


def rasterize_icons() -> None:
    for size in ICON_SIZES:
        dest = HICOLOR / f"{size}x{size}/apps/pro.tuxcore.Lightforge.png"
        _rasterize(ICON_SVG, dest, size, size)
        print(f"icon {size}x{size}")


def rasterize_steam() -> None:
    """Library art Steam shows for a shortcut: field + icon + wordmark."""
    steam = ROOT / "crates/launcher-gui/assets/steam"
    wordmark = ROOT / "crates/launcher-gui/assets/wordmark.png"
    tmp = BRAND / ".tmp-steam"
    tmp.mkdir(exist_ok=True)
    bg, glow = "#0B0D10", "#0A2333"

    def field(w: int, h: int, out: Path) -> None:
        subprocess.check_call(
            [
                "convert",
                "-size",
                f"{w}x{h}",
                f"radial-gradient:{glow}-{bg}",
                "-attenuate",
                "0.25",
                "+noise",
                "Uniform",
                str(out),
            ]
        )

    def icon(size: int, out: Path) -> None:
        _rasterize(ICON_SVG, out, size, size)

    field(600, 900, tmp / "tall.png")
    icon(300, tmp / "i300.png")
    subprocess.check_call(
        [
            "convert",
            str(tmp / "tall.png"),
            str(tmp / "i300.png"),
            "-gravity",
            "center",
            "-geometry",
            "+0-110",
            "-composite",
            "(",
            str(wordmark),
            "-resize",
            "480x",
            ")",
            "-gravity",
            "center",
            "-geometry",
            "+0+150",
            "-composite",
            "-strip",
            "-quality",
            "88",
            str(steam / "capsule.jpg"),
        ]
    )
    field(920, 430, tmp / "wide.png")
    subprocess.check_call(
        [
            "convert",
            str(tmp / "wide.png"),
            "(",
            str(wordmark),
            "-resize",
            "640x",
            ")",
            "-gravity",
            "center",
            "-composite",
            "-strip",
            "-quality",
            "88",
            str(steam / "wide.jpg"),
        ]
    )
    field(1920, 620, tmp / "hero.png")
    icon(900, tmp / "i900.png")
    subprocess.check_call(
        [
            "convert",
            str(tmp / "hero.png"),
            "(",
            str(tmp / "i900.png"),
            "-channel",
            "A",
            "-evaluate",
            "multiply",
            "0.18",
            "+channel",
            ")",
            "-gravity",
            "east",
            "-geometry",
            "-120+0",
            "-composite",
            "-strip",
            "-quality",
            "88",
            str(steam / "hero.jpg"),
        ]
    )
    subprocess.check_call(["convert", str(wordmark), "-strip", str(steam / "logo.png")])
    icon(256, steam / "icon.png")
    for p in tmp.iterdir():
        p.unlink()
    tmp.rmdir()
    print(f"steam art -> {steam.relative_to(ROOT)}")


def main() -> None:
    for needed in (WORKMARK, BANNER_BASE, ICON_SVG, FONT_SEMIBOLD, FONT_BOLD):
        if not needed.is_file():
            sys.exit(f"missing {needed}")
    write_wordmark()
    write_banner()
    ensure_icon_badge()
    rasterize_icons()
    rasterize_steam()


if __name__ == "__main__":
    main()
