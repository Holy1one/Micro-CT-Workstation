"""Regenerate maintained Windows icons from the transparent logo (requires Pillow).

Only asset maintenance needs Python; normal app builds validate the checked-in ICO
with Node. Tauri's Windows codegen decodes the FIRST ICO entry for its window icon,
so keep 256px first instead of Pillow/Tauri CLI's usual smallest-first ordering.
"""
from io import BytesIO
from pathlib import Path
import struct

from PIL import Image

ASSETS = Path(__file__).resolve().parents[2] / "assets"
SIZES = (256, 192, 128, 96, 80, 64, 48, 40, 32, 24, 20, 16)


def resize_rgba(image, size):
    # Filter premultiplied color to avoid a matte around transparent edges.
    return image.convert("RGBa").resize(size, Image.Resampling.LANCZOS).convert("RGBA")


def desktop_frame(source, size):
    cropped = source.crop(source.getchannel("A").getbbox())
    scale = round(size * 0.88) / max(cropped.size)
    dimensions = tuple(round(length * scale) for length in cropped.size)
    content = resize_rgba(cropped, dimensions)
    frame = Image.new("RGBA", (size, size))
    frame.paste(content, ((size - content.width) // 2, (size - content.height) // 2))
    return frame


def main():
    source = Image.open(ASSETS / "micro-ct-logo.png").convert("RGBA")
    frames = {size: desktop_frame(source, size) for size in SIZES}
    entries, payloads = [], []
    offset = 6 + 16 * len(SIZES)
    for size in SIZES:
        output = BytesIO()
        frames[size].save(output, format="PNG", optimize=True)
        payload = output.getvalue()
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                                   1, 32, len(payload), offset))
        payloads.append(payload)
        offset += len(payload)
    (ASSETS / "app-icon.ico").write_bytes(
        struct.pack("<HHH", 0, 1, len(SIZES)) + b"".join(entries) + b"".join(payloads))
    for size, name in [(32, "app-32x32.png"), (128, "app-128x128.png"),
                       (256, "app-128x128@2x.png")]:
        frames[size].save(ASSETS / name, optimize=True)
    # UI logos share their original canvas/spacing; desktop icons are tighter.
    resize_rgba(source, (256, 256)).save(ASSETS / "app-logo-light-256.png", optimize=True)
    dark = Image.open(ASSETS / "micro-ct-logo-dark.png").convert("RGBA")
    resize_rgba(dark, (256, 256)).save(ASSETS / "app-logo-dark-256.png", optimize=True)
    print("PASS transparent Windows ICO: 256px first; frames " + ", ".join(map(str, SIZES)))


if __name__ == "__main__":
    main()
