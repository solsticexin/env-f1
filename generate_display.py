#!/usr/bin/env python3
"""
Generate a RGB565 frame buffer with Chinese labels for the ST7735 display.

The script creates a 160x128 image that contains Chinese labels for the
environment metrics (temperature, humidity, light intensity and soil moisture),
and exports it both as an optional preview PNG as well as a raw RGB565 file
that can be embedded directly into the firmware with `include_bytes!`.

Example:
    python generate_display.py \\
        --font /path/to/NotoSansSC-Regular.otf \\
        --temp 25.4 --humidity 61.2 --light 435.0 --soil 48.0
"""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Iterable, Tuple

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError as exc:  # pragma: no cover - runtime guard
    raise SystemExit(
        "Pillow is required. Install it with `pip install pillow`."
    ) from exc

DEFAULT_OUTPUT = Path("assets/images/status_background.rgb565")
DEFAULT_PREVIEW = Path("assets/images/status_preview.png")
IMAGE_SIZE = (120, 96)
BACKGROUND_COLOR = (0, 0, 0)
LABEL_COLOR = (220, 220, 220)
VALUE_COLOR = (0, 212, 255)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render Chinese labels to an RGB565 image for the ST7735 display."
    )
    parser.add_argument(
        "--font",
        required=True,
        type=Path,
        help="Path to a TrueType/OpenType font that contains the required Chinese glyphs.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help=f"RGB565 output file path (default: {DEFAULT_OUTPUT})",
    )
    parser.add_argument(
        "--preview",
        type=Path,
        default=DEFAULT_PREVIEW,
        help=f"Optional PNG preview path (default: {DEFAULT_PREVIEW})",
    )
    parser.add_argument("--labels-only", action="store_true", help="Only draw Chinese labels and skip numeric values.")
    parser.add_argument("--temp", type=float, default=0.0, help="Temperature reading in ℃.")
    parser.add_argument("--humidity", type=float, default=0.0, help="Relative humidity in %.")
    parser.add_argument("--light", type=float, default=0.0, help="Light intensity in lux.")
    parser.add_argument("--soil", type=float, default=0.0, help="Soil moisture in %.")  # noqa: B950
    return parser.parse_args()


def compose_lines(args: argparse.Namespace) -> Iterable[Tuple[str, str]]:
    """Return label/value pairs in Chinese with formatted values."""
    if args.labels_only:
        return [
            ("温度", ""),
            ("湿度", ""),
            ("光照强度", ""),
            ("土壤湿度", ""),
        ]
    return [
        ("温度", f"{args.temp:.1f} ℃"),
        ("湿度", f"{args.humidity:.1f} %"),
        ("光照强度", f"{args.light:.1f} lx"),
        ("土壤湿度", f"{args.soil:.1f} %"),
    ]


def render_image(font_path: Path, lines: Iterable[Tuple[str, str]]) -> Image.Image:
    """Render the Chinese labels and values to an RGB image."""
    image = Image.new("RGB", IMAGE_SIZE, BACKGROUND_COLOR)
    draw = ImageDraw.Draw(image)

    # Larger labels for the expanded 120x96 layout while leaving room on the right for numbers.
    font_labels = ImageFont.truetype(str(font_path), size=18)
    font_values = ImageFont.truetype(str(font_path), size=18)

    vertical_spacing = 20
    start_y = 14

    for idx, (label_text, value_text) in enumerate(lines):
        y = start_y + idx * vertical_spacing
        draw.text((10, y), label_text, font=font_labels, fill=LABEL_COLOR)
        if value_text:
            draw.text((76, y), value_text, font=font_values, fill=VALUE_COLOR)

    return image


def to_rgb565_le(image: Image.Image) -> bytes:
    """Convert a Pillow image to little-endian RGB565 raw bytes."""
    if image.mode != "RGB":
        image = image.convert("RGB")

    buffer = bytearray()
    for red, green, blue in image.getdata():
        rgb565 = ((red & 0xF8) << 8) | ((green & 0xFC) << 3) | (blue >> 3)
        buffer.append(rgb565 & 0xFF)  # low byte first (little endian)
        buffer.append((rgb565 >> 8) & 0xFF)
    return bytes(buffer)


def main() -> None:
    args = parse_args()
    lines = list(compose_lines(args))

    image = render_image(args.font, lines)

    if args.preview:
        args.preview.parent.mkdir(parents=True, exist_ok=True)
        image.save(args.preview)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(to_rgb565_le(image))

    print(f"RGB565 frame written to {args.output}")
    if args.preview:
        print(f"Preview PNG written to {args.preview}")


if __name__ == "__main__":
    main()
