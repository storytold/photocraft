"""Render an Adobe-style app tile (a rounded square with two letters, as the Creative Cloud
application tiles look) into every icon file a craft app reads, for the owner's personal local
builds only: the tiles imitate Adobe's and Dassault's marks, so they never go into a published
build (docs/comfy/upstream.md has the brand checklist).

Usage: python -I icon-tile.py <repo-root> <name> <letters> <tile-hex> <letter-hex> [<app-id>]

  photocraft  Sp  #001E36 #31A8FF   (ShotoPhopEX; navy and blue, its own letters)
  vectorcraft Ai  #330000 #FF9A00   (Illustrator)
  pdfcraft    @ribbon  -  -        (Acrobat: a red ribbon "A" on a white tile)
  solidcraft  @cube    -  -        (SolidWorks: a red cube with S and W on its faces)

Writes, under <repo-root>/assets/app-icon/: <name>.svg and <name>-small.svg (text-based masters),
<name>-1024.png (Apple's padded grid), <name>-macos-512.png, <name>.icns, <name>.ico (16-256 px),
hicolor/<s>x<s>/apps/<app-id>.png for 16..512 and hicolor/scalable/apps/<app-id>.svg. app-id
defaults to ai.storyteller.<name>. Geometry as packaging/icon-px.py: a 512-unit tile with rx=112,
full bleed for Windows and Linux (no crop, so the corners stay round, as upstream #4478), padded to 636 units for macOS.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

def find_font(*candidates):
    """The first font file that exists (Windows first, then macOS and Linux)."""
    for c in candidates:
        if os.path.isfile(c):
            return c
    sys.exit(f"no font found among {candidates}")


FONT = find_font(
    r"C:\Windows\Fonts\segoeuib.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/Library/Fonts/Arial Bold.ttf",
    "/usr/share/fonts/opentype/inter/InterDisplay-Bold.otf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
)
# The cube's letters: a black weight, as SolidWorks' mark.
BLACK_FONT = find_font(
    r"C:\Windows\Fonts\ariblk.ttf",
    "/System/Library/Fonts/Supplemental/Arial Black.ttf",
    "/Library/Fonts/Arial Black.ttf",
    FONT,
)
SCALE = 8  # render the 512 tile at 4096 px, then downsample


def rgba(hexcode):
    h = hexcode.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), 255)


def tile(letters, bg, fg, size_units=512, margin_units=0):
    """The tile at SCALE x, centred in a square of `size_units` with `margin_units` around it."""
    px = size_units * SCALE
    img = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = margin_units * SCALE
    tile_px = px - 2 * m
    r = int(112 * tile_px / 512)
    d.rounded_rectangle([m, m, m + tile_px - 1, m + tile_px - 1], radius=r, fill=bg)
    # Two letters at 56 % of the tile, a little less for wide pairs such as "SW".
    font_px = int(tile_px * (0.42 if len(letters) > 1 and letters.isupper() else 0.56))
    font = ImageFont.truetype(FONT, font_px)
    bbox = d.textbbox((0, 0), letters, font=font)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
    x = m + (tile_px - tw) / 2 - bbox[0]
    y = m + (tile_px - th) / 2 - bbox[1] - tile_px * 0.01
    d.text((x, y), letters, font=font, fill=fg)
    return img


def padded(art, size_units, margin_units):
    """`art` (a 512-unit render) centred on a `size_units` canvas with `margin_units` around it."""
    px = size_units * SCALE
    inner = px - 2 * margin_units * SCALE
    img = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    img.alpha_composite(art.resize((inner, inner), Image.LANCZOS), (margin_units * SCALE, margin_units * SCALE))
    return img


def gradient_fill(img, poly, p0, c0, p1, c1):
    """Fill polygon `poly` with a linear gradient from colour c0 at point p0 to c1 at p1."""
    w, h = img.size
    ys, xs = np.mgrid[0:h, 0:w].astype(np.float32)
    dx, dy = p1[0] - p0[0], p1[1] - p0[1]
    t = np.clip(((xs - p0[0]) * dx + (ys - p0[1]) * dy) / max(dx * dx + dy * dy, 1e-6), 0, 1)[..., None]
    rgb = (np.array(c0[:3], np.float32) * (1 - t) + np.array(c1[:3], np.float32) * t).astype(np.uint8)
    layer = Image.fromarray(np.dstack([rgb, np.full((h, w), 255, np.uint8)]), "RGBA")
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).polygon(poly, fill=255)
    img.paste(layer, (0, 0), mask)


def perspective_coeffs(dst, src):
    """Image.transform PERSPECTIVE data mapping output quad `dst` back onto input quad `src`."""
    rows, rhs = [], []
    for (x, y), (u, v) in zip(dst, src):
        rows.append([x, y, 1, 0, 0, 0, -u * x, -u * y])
        rows.append([0, 0, 0, x, y, 1, -v * x, -v * y])
        rhs += [u, v]
    return np.linalg.solve(np.array(rows, np.float64), np.array(rhs, np.float64)).tolist()


def letter_on_face(img, letter, quad, colour, inset=0.14):
    """A bold `letter` drawn square, then warped onto the face `quad` (TL, TR, BR, BL)."""
    t = 1024
    tex = Image.new("RGBA", (t, t), (0, 0, 0, 0))
    d = ImageDraw.Draw(tex)
    font = ImageFont.truetype(BLACK_FONT, int(t * 0.95))
    bb = d.textbbox((0, 0), letter, font=font)
    lw, lh = bb[2] - bb[0], bb[3] - bb[1]
    sx, sy = t * (1 - 2 * inset) / lw, t * (1 - 2 * inset) / lh
    big = Image.new("RGBA", (lw + 4, lh + 4), (0, 0, 0, 0))
    ImageDraw.Draw(big).text((2 - bb[0], 2 - bb[1]), letter, font=font, fill=colour)
    big = big.resize((max(1, int(lw * sx)), max(1, int(lh * sy))), Image.LANCZOS)
    tex.alpha_composite(big, ((t - big.width) // 2, (t - big.height) // 2))
    src = [(0, 0), (t, 0), (t, t), (0, t)]
    warped = tex.transform(img.size, Image.PERSPECTIVE, perspective_coeffs(quad, src), Image.BICUBIC)
    img.alpha_composite(warped)


def cube():
    """A SolidWorks-style mark: a red isometric cube, "S" on its left face, "W" on its right."""
    px = 512 * SCALE
    img = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    # The drawing's 860-unit square onto 86 % of the tile, centred: clear of the rounded corners.
    k = SCALE * 512 / 860 * 0.86
    o = SCALE * 512 * 0.07
    p = lambda x, y: (o + x * k, o + y * k)  # noqa: E731
    top = [p(28, 128), p(430, 32), p(832, 128), p(430, 248)]
    left = [p(28, 128), p(430, 248), p(430, 840), p(48, 692)]
    right = [p(430, 248), p(832, 128), p(806, 656), p(430, 840)]
    gradient_fill(img, top, p(300, 220), rgba("#E60000"), p(780, 110), rgba("#FF7A7A"))
    gradient_fill(img, left, p(230, 180), rgba("#7E0000"), p(240, 780), rgba("#520000"))
    gradient_fill(img, right, p(630, 180), rgba("#A80000"), p(620, 760), rgba("#7A0000"))
    letter_on_face(img, "S", left, rgba("#E6E6E6"))
    letter_on_face(img, "W", right, rgba("#FFFFFF"))
    return img


def bezier(p0, c1, c2, p3, n=120):
    out = []
    for i in range(n + 1):
        t = i / n
        a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t * t, t**3
        out.append((a * p0[0] + b * c1[0] + c * c2[0] + d * p3[0], a * p0[1] + b * c1[1] + c * c2[1] + d * p3[1]))
    return out


# The ribbon "A" as cubic segments in a 1950-unit drawing, each with its width at both ends:
# the tail and bottom loop, the long left leg, the small top loop, the right leg, the right
# loop and the crossbar back to the left leg.
RIBBON = [
    ((600, 1335), (430, 1430), (292, 1530), (300, 1610), 6, 30),
    ((300, 1610), (308, 1690), (395, 1705), (455, 1680), 30, 34),
    ((455, 1680), (640, 1560), (840, 1080), (915, 770), 34, 64),
    ((915, 770), (850, 540), (800, 320), (955, 255), 64, 28),
    ((955, 255), (1065, 285), (1060, 450), (1020, 565), 28, 30),
    ((1020, 565), (995, 650), (978, 700), (978, 722), 30, 40),
    ((978, 722), (1060, 920), (1180, 1060), (1295, 1118), 40, 58),
    ((1295, 1118), (1560, 1080), (1752, 1080), (1745, 1205), 58, 26),
    ((1745, 1205), (1725, 1305), (1560, 1320), (1352, 1168), 26, 36),
    ((1352, 1168), (1100, 1185), (900, 1232), (768, 1292), 36, 8),
]


def ribbon():
    """An Acrobat-style mark: a red ribbon "A" on a white-to-grey tile."""
    px = 512 * SCALE
    img = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    mask = Image.new("L", (px, px), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, px - 1, px - 1], radius=112 * SCALE, fill=255)
    gradient_fill(img, [(0, 0), (px, 0), (px, px), (0, px)], (0, 0), rgba("#FFFFFF"), (0, px), rgba("#E2E2E2"))
    img.putalpha(mask)
    d = ImageDraw.Draw(img)
    k = SCALE * 0.27
    q = lambda x, y: (256 * SCALE + (x - 1022) * k, 256 * SCALE + (y - 975) * k)  # noqa: E731
    red = rgba("#E3202A")
    for p0, c1, c2, p3, w0, w1 in RIBBON:
        pts = bezier(q(*p0), q(*c1), q(*c2), q(*p3))
        for i, (x, y) in enumerate(pts):
            r = (w0 + (w1 - w0) * i / (len(pts) - 1)) * 1.8 * k / 2
            d.ellipse([x - r, y - r, x + r, y + r], fill=red)
    return img


def save(img, path, size):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.resize((size, size), Image.LANCZOS).save(path, "PNG", optimize=True)


def main():
    if len(sys.argv) < 6:
        sys.exit(__doc__)
    root, name, letters, bg_hex, fg_hex = sys.argv[1:6]
    app_id = sys.argv[6] if len(sys.argv) > 6 else f"ai.storyteller.{name}"
    bg, fg = rgba(bg_hex), rgba(fg_hex)
    out = os.path.join(root, "assets", "app-icon")
    # `@cube` and `@ribbon` draw the SolidWorks- and Acrobat-style marks instead of letters.
    full = cube() if letters == "@cube" else ribbon() if letters == "@ribbon" else tile(letters, bg, fg)
    mac = padded(full, 636, 62)

    save(mac, os.path.join(out, f"{name}-1024.png"), 1024)
    save(mac, os.path.join(out, f"{name}-macos-512.png"), 512)
    for s in (16, 24, 32, 48, 64, 128, 256, 512):
        save(full, os.path.join(out, "hicolor", f"{s}x{s}", "apps", f"{app_id}.png"), s)
    full.resize((256, 256), Image.LANCZOS).save(
        os.path.join(out, f"{name}.ico"), format="ICO", sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
    )
    mac.resize((1024, 1024), Image.LANCZOS).save(
        os.path.join(out, f"{name}.icns"), format="ICNS", sizes=[(s, s) for s in (16, 32, 64, 128, 256, 512, 1024)]
    )
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">\n'
        f'  <!-- {name} app icon: a rounded tile with "{letters}"; rendered by\n'
        '       packaging/icon-tile.py (Segoe UI Bold, or the bold sans the system has). -->\n'
        f'  <rect width="512" height="512" rx="112" fill="{bg_hex}"/>\n'
        '  <text x="256" y="356" text-anchor="middle" font-family="Segoe UI, Helvetica Neue, Arial, sans-serif"\n'
        f'        font-weight="700" font-size="287" fill="{fg_hex}">{letters}</text>\n'
        '</svg>\n'
    )
    if letters.startswith("@"):
        # The drawn marks have no text form: the SVG carries the 512 px render.
        import base64
        import io

        buf = io.BytesIO()
        full.resize((512, 512), Image.LANCZOS).save(buf, "PNG", optimize=True)
        data = base64.b64encode(buf.getvalue()).decode("ascii")
        svg = (
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">\n'
            f'  <!-- {name} app icon for local builds, drawn by packaging/icon-tile.py ({letters}). Not for a published build. -->\n'
            f'  <image width="512" height="512" href="data:image/png;base64,{data}"/>\n'
            '</svg>\n'
        )
    for f_name in (f"{name}.svg", f"{name}-small.svg"):
        with open(os.path.join(out, f_name), "w", encoding="utf-8", newline="\n") as f:
            f.write(svg)
    scalable = os.path.join(out, "hicolor", "scalable", "apps")
    os.makedirs(scalable, exist_ok=True)
    with open(os.path.join(scalable, f"{app_id}.svg"), "w", encoding="utf-8", newline="\n") as f:
        f.write(svg)
    print(f"{name}: {letters} tile written to {out}")


if __name__ == "__main__":
    main()
