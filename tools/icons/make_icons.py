"""Builds Glint's icons from the source artwork in assets/icon-source/.

    python tools/icons/make_icons.py

The source SVG wraps one 1024 x 1024 PNG clipped to a rounded square (rx 224). This script takes
that PNG out losslessly, cuts the rounded corners itself with 8x8 supersampled edges, then writes:

- assets/glint-1024.png  master
- assets/glint-256.png   README, window icon fallback
- assets/glint.ico       16, 20, 24, 32, 40, 48, 64, 72, 80, 96, 128 and 256 px (every size Windows
                         asks for at 100 to 300 % scale), 32-bit with alpha; 256 stored as PNG.

Every size is resampled straight from the 1024 master with Lanczos (never from another small
size), and sizes up to 48 px get a light unsharp mask so the glass edges stay crisp in the tray and
taskbar.
"""
import base64
import io
import re
import struct
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "assets" / "icon-source" / "Glint - Color.svg"
OUT = ROOT / "assets"
SIZES = [16, 20, 24, 32, 40, 48, 64, 72, 80, 96, 128, 256]
RADIUS = 224 / 1024  # the source clip-path


def master() -> Image.Image:
    svg = SRC.read_text(encoding="utf-8")
    m = re.search(r'href="data:image/png;base64,([^"]+)"', svg)
    if not m:
        raise SystemExit("no embedded PNG in " + str(SRC))
    img = Image.open(io.BytesIO(base64.b64decode(m.group(1)))).convert("RGBA")
    if img.size != (1024, 1024):
        img = img.resize((1024, 1024), Image.LANCZOS)
    # Rounded-square alpha, drawn at 8x and reduced: smooth corners.
    big = 1024 * 8
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, big - 1, big - 1), radius=int(RADIUS * big), fill=255)
    mask = mask.resize((1024, 1024), Image.LANCZOS)
    # Multiply with the art's own alpha, so any transparency already in it is kept.
    img.putalpha(ImageChops.multiply(img.getchannel("A"), mask))
    out = img
    return out


def sized(m: Image.Image, n: int) -> Image.Image:
    # Resize in premultiplied space so the transparent corners do not bleed dark fringes.
    pm = m.convert("RGBa").resize((n, n), Image.LANCZOS)
    if n <= 48:
        pm = pm.filter(ImageFilter.UnsharpMask(radius=0.6, percent=55 if n <= 24 else 40, threshold=0))
    return pm.convert("RGBA")


def dib_entry(img: Image.Image) -> bytes:
    """A classic 32-bit BMP icon image: header, bottom-up BGRA rows, empty AND mask."""
    n = img.width
    header = struct.pack("<IiiHHIIiiII", 40, n, n * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    px = img.tobytes("raw", "BGRA")
    rows = [px[y * n * 4:(y + 1) * n * 4] for y in range(n)]
    xor = b"".join(reversed(rows))
    mask_row = ((n + 31) // 32) * 4
    return header + xor + bytes(mask_row * n)


def write_ico(images: list[Image.Image], path: Path) -> None:
    entries = []
    for img in images:
        if img.width >= 256:
            buf = io.BytesIO()
            img.save(buf, "PNG", optimize=True)
            entries.append((img.width, buf.getvalue()))
        else:
            entries.append((img.width, dib_entry(img)))
    offset = 6 + 16 * len(entries)
    out = struct.pack("<HHH", 0, 1, len(entries))
    blobs = b""
    for n, data in entries:
        dim = 0 if n >= 256 else n
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        blobs += data
    path.write_bytes(out + blobs)


def main() -> None:
    m = master()
    m.save(OUT / "glint-1024.png", optimize=True)
    sized(m, 256).save(OUT / "glint-256.png", optimize=True)
    write_ico([sized(m, n) for n in SIZES], OUT / "glint.ico")
    print("wrote", ", ".join(str(OUT / f) for f in ("glint-1024.png", "glint-256.png", "glint.ico")))


if __name__ == "__main__":
    main()
