"""Writes small valid seed files for the io fuzz targets whose formats are ZIP archives.

libFuzzer rarely builds a ZIP with matching CRCs and the XML a parser wants from nothing, so
`import_kra` and `import_ora` start from these. Standard library only. Usage, from crates/io/fuzz:

    python3 seeds.py <target>    # writes into corpus/<target>/; any other target is a no-op
"""

import io
import os
import struct
import sys
import zipfile
import zlib


def png(width, height, rgba):
    """An 8-bit RGBA PNG; `rgba(x, y)` gives each pixel."""

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b"".join(b"\0" + b"".join(bytes(rgba(x, y)) for x in range(width)) for y in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def archive(mimetype, files):
    """A ZIP whose first entry is the stored `mimetype`, as both formats require."""
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as z:
        z.writestr(zipfile.ZipInfo("mimetype"), mimetype, compress_type=zipfile.ZIP_STORED)
        for name, data in files:
            z.writestr(name, data, compress_type=zipfile.ZIP_DEFLATED)
    return out.getvalue()


def ora():
    layer = png(4, 4, lambda x, y: (60 * x, 60 * y, 200, 255 if x != y else 128))
    stack = (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<image version="0.0.5" w="6" h="5" xres="300" yres="300"><stack>'
        '<layer name="top" src="data/top.png" x="1" y="0" opacity="0.5" composite-op="svg:multiply"/>'
        '<stack name="group" isolation="isolate" visibility="visible">'
        '<layer name="inner" src="data/inner.png" x="0" y="1" visibility="hidden" composite-op="svg:screen"/>'
        "</stack></stack></image>"
    )
    files = [("stack.xml", stack), ("data/top.png", layer), ("data/inner.png", layer)]
    files.append(("mergedimage.png", png(6, 5, lambda x, y: (x * 40, y * 50, 90, 255))))
    return archive("image/openraster", files)


def kra_tiles(pixel_size, tiles, width=4):
    """A Krita VERSION 2 tile file; every tile stored raw (compression byte 0)."""
    out = f"VERSION 2\nTILEWIDTH {width}\nTILEHEIGHT {width}\nPIXELSIZE {pixel_size}\nDATA {len(tiles)}\n".encode()
    for x, y, pixels in tiles:
        body = b"\0" + pixels
        out += f"{x},{y},LZF,{len(body)}\n".encode() + body
    return out


def kra():
    bgra = bytes(v for p in range(16) for v in (p * 15, 255 - p * 15, 128, 255))
    mask = bytes(p * 16 for p in range(16))
    layers = (
        '<layer nodetype="grouplayer" name="group" filename="layer1" compositeop="multiply" opacity="200" visible="1" x="0" y="0">'
        '<layers><layer nodetype="paintlayer" name="paint" colorspacename="RGBA" filename="layer2" compositeop="normal" opacity="255" visible="1" x="1" y="1">'
        '<masks><mask nodetype="transparencymask" name="mask" filename="mask3" x="0" y="0"/></masks>'
        "</layer></layers></layer>"
    )
    maindoc = (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<DOC xmlns="http://www.calligra.org/DTD/krita" syntaxVersion="2.0">'
        '<IMAGE name="img" width="8" height="6" colorspacename="RGBA" x-res="300" y-res="300">'
        f"<layers>{layers}</layers></IMAGE></DOC>"
    )
    files = [
        ("maindoc.xml", maindoc),
        ("img/layers/layer2", kra_tiles(4, [(0, 0, bgra), (4, 0, bgra)])),
        ("img/layers/layer2.defaultpixel", b"\0\0\0\0"),
        ("img/layers/mask3.pixelselection", kra_tiles(1, [(0, 0, mask)])),
        ("img/layers/mask3.pixelselection.defaultpixel", b"\0"),
        ("mergedimage.png", png(8, 6, lambda x, y: (x * 30, y * 40, 70, 255))),
    ]
    return archive("application/x-krita", files)


SEEDS = {"import_ora": [("seed.ora", ora)], "import_kra": [("seed.kra", kra)]}


def main():
    target = sys.argv[1] if len(sys.argv) > 1 else ""
    for name, make in SEEDS.get(target, []):
        directory = os.path.join("corpus", target)
        os.makedirs(directory, exist_ok=True)
        with open(os.path.join(directory, name), "wb") as f:
            f.write(make())


if __name__ == "__main__":
    main()
