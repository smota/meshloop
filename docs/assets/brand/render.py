"""Rasterize Meshloop brand PNGs from the canonical geometry.

Requires Pillow in a local environment. Not part of the Rust workspace.
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

GRAPHITE = (0x1B, 0x24, 0x29, 255)
TEAL = (0x4F, 0xBF, 0xB8, 255)
COPPER = (0xE0, 0xA0, 0x5A, 255)
MESH = (0x8A, 0x9A, 0xA3, 255)
SNOW = (0xF4, 0xF1, 0xEA, 255)
HERE = Path(__file__).resolve().parent


def draw_mark(size: int) -> Image.Image:
    scale = 8
    canvas = size * scale
    img = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = canvas / 128.0

    def xy(x: float, y: float) -> tuple[float, float]:
        return (x * s, y * s)

    d.ellipse([xy(4, 4), xy(124, 124)], fill=GRAPHITE)
    chords = [(64, 24, 104, 64), (104, 64, 64, 104), (64, 104, 24, 64), (24, 64, 64, 24)]
    width = max(1, round(2.5 * s))
    for x1, y1, x2, y2 in chords:
        d.line([xy(x1, y1), xy(x2, y2)], fill=MESH, width=width)
    ring_w = max(1, round(6 * s))
    d.ellipse([xy(24, 24), xy(104, 104)], outline=TEAL, width=ring_w)
    for cx, cy in ((64, 24), (104, 64), (24, 64)):
        r = 5.5
        d.ellipse([xy(cx - r, cy - r), xy(cx + r, cy + r)], fill=TEAL)
        hole = 2.2
        d.ellipse(
            [xy(cx - hole, cy - hole), xy(cx + hole, cy + hole)],
            fill=GRAPHITE,
        )
    d.rectangle([xy(59.5, 99.5), xy(68.5, 108.5)], fill=COPPER)
    d.rectangle([xy(61.75, 101.75), xy(66.25, 106.25)], fill=GRAPHITE)
    return img.resize((size, size), Image.Resampling.LANCZOS)


def font(path: str, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(path, size)


def draw_social() -> Image.Image:
    w, h = 1280, 640
    img = Image.new("RGBA", (w, h), GRAPHITE)
    d = ImageDraw.Draw(img)
    # Quiet square mesh in the right third.
    fade = MESH[:-1] + (28,)
    overlay = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    od = ImageDraw.Draw(overlay)
    origin_x, origin_y, step = 820, -40, 56
    for i in range(0, 16):
        x = origin_x + i * step
        od.line([(x, 0), (x, h)], fill=fade, width=2)
    for i in range(0, 14):
        y = origin_y + i * step
        od.line([(origin_x, y), (w, y)], fill=fade, width=2)
    img = Image.alpha_composite(img, overlay)
    mark = draw_mark(288)
    img.paste(mark, (96, (h - 288) // 2), mark)
    bold = None
    regular = None
    for candidate in (
        r"C:\Windows\Fonts\seguisb.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ):
        p = Path(candidate)
        if p.exists() and bold is None:
            bold = font(str(p), 92)
        if p.exists() and "segoeui.ttf" in candidate.lower():
            regular = font(str(p), 36)
    if bold is None:
        bold = ImageFont.load_default()
    if regular is None:
        regular = bold
    text_x = 96 + 288 + 56
    d = ImageDraw.Draw(img)
    d.text((text_x, 214), "Meshloop", font=bold, fill=SNOW)
    d.text(
        (text_x, 330),
        "Local orchestration for CLI agents.",
        font=regular,
        fill=MESH,
    )
    return img.convert("RGB")


def main() -> None:
    HERE.mkdir(parents=True, exist_ok=True)
    for size in (32, 128, 512):
        draw_mark(size).save(HERE / f"logo-{size}.png")
    social = draw_social()
    social.save(HERE / "social.png", optimize=True)
    print("wrote", HERE)


if __name__ == "__main__":
    main()
