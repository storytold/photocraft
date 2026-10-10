#!/usr/bin/env python3
"""Writes the XCF fixtures under crates/io/tests/fixtures/xcf with GIMP itself (an independent
writer), each next to GIMP's own merged rendering as a PNG, so `crates/io/tests/xcf.rs` checks
both the layer data it reads and the composite it renders against GIMP:

    layers-8bit.xcf     three RGB layers: offsets (one off-canvas), opacity, Multiply and Screen
                        modes, a hidden layer, lock alpha, a colour tag, resolution and guides
    groups.xcf          a group with two children and a nested group, one group pass-through,
                        group opacity
    mask.xcf            a layer mask painted with a gradient, one applied and one disabled
    gray16.xcf          a 16-bit grayscale image with two layers
    float-linear.xcf    a 32-bit float linear-light RGB image (precision 600)
    indexed.xcf         a 16-colour indexed image with transparency
    text.xcf            a text layer (opens as pixels)
    channel.xcf         an extra channel and a saved selection
    legacy-modes.xcf    Addition, Subtract, Divide, Dodge, Burn, Grain Merge, HSV Hue/Value and
                        LCH Color modes

Each file with blend modes or transparency also comes as `<name>-perceptual.xcf`, with every
layer set to blend and composite in gamma-encoded RGB (GIMP's default is linear light for most
modes), which is the space PhotoCraft composes in; the tests compare our composite tightly to
GIMP's PNG for those and measure the known gap on the defaults.

Run with:  GIMPHOTO_COMFYUI=off flatpak run --command=gimp io.github.diegochagas.GIMPhoto -i -n \
               --batch-interpreter=python-fu-eval -b "exec(open('scripts/xcf_fixtures.py').read())" -b 'quit()'
(or the official GIMP 3 Flatpak with `org.gimp.GIMP`). The patterns are mirrored by `pattern()` in
crates/io/tests/xcf.rs; change both.
"""

import os
import struct

import gi

gi.require_version("Gimp", "3.0")
gi.require_version("Gegl", "0.4")
from gi.repository import Gegl, Gimp, Gio  # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)) if "__file__" in globals() else os.getcwd(), "..", "crates", "io", "tests", "fixtures", "xcf")
OUT = os.path.normpath(OUT) if os.path.isdir(os.path.dirname(OUT)) else os.path.join(os.getcwd(), "crates", "io", "tests", "fixtures", "xcf")
os.makedirs(OUT, exist_ok=True)
W, H = 40, 30


def pattern(x, y, c, maxval):
    """Integer sample, every channel different (mirrored in xcf.rs)."""
    return (x * 31 + y * 17 + c * 101 + 7) * 13 % (maxval + 1)


def alpha(x, y, w, h, maxval):
    return min(maxval, (x + y) * maxval // max(1, w + h - 4))


def fill(drawable, w, h, channels, bits, sample, fmt=None):
    """Writes procedural samples into a drawable's buffer."""
    data = bytearray()
    for y in range(h):
        for x in range(w):
            for c in range(channels):
                v = sample(x, y, c)
                if bits == 8:
                    data.append(v)
                elif bits == 16:
                    data += struct.pack("<H", v)
                else:
                    data += struct.pack("<f", v)
    buf = drawable.get_buffer()
    if fmt is None:
        fmt = {(1, 8): "Y' u8", (2, 8): "Y'A u8", (3, 8): "R'G'B' u8", (4, 8): "R'G'B'A u8", (1, 16): "Y' u16", (2, 16): "Y'A u16", (4, 16): "R'G'B'A u16", (4, 32): "RGBA float", (3, 32): "RGB float"}[(channels, bits)]
    buf.set(Gegl.Rectangle.new(0, 0, w, h), fmt, bytes(data))
    buf.flush()
    drawable.merge_shadow(True)
    drawable.update(0, 0, w, h)


def all_layers(parent):
    """Every layer of an image or group, depth first."""
    out = []
    for layer in parent.get_layers() if isinstance(parent, Gimp.Image) else parent.get_children():
        out.append(layer)
        if isinstance(layer, Gimp.GroupLayer):
            out += all_layers(layer)
    return out


def save(img, name, perceptual_variant=True):
    """Writes `name.xcf` as GIMP composes it by default (linear light for most modes), and
    `name-perceptual.xcf` with every layer blending and compositing in gamma-encoded RGB, which
    is how Photoshop and PhotoCraft compose; each with GIMP's merged rendering as a PNG."""
    save_one(img, name)
    if perceptual_variant:
        for layer in all_layers(img):
            layer.set_blend_space(Gimp.LayerColorSpace.RGB_PERCEPTUAL)
            layer.set_composite_space(Gimp.LayerColorSpace.RGB_PERCEPTUAL)
        save_one(img, name + "-perceptual")


def save_one(img, name):
    path = os.path.join(OUT, name)
    Gimp.file_save(Gimp.RunMode.NONINTERACTIVE, img, Gio.File.new_for_path(path + ".xcf"), None)
    # GIMP's own rendering of the visible layers, as the oracle for our composite.
    merged = img.duplicate()
    merged.merge_visible_layers(Gimp.MergeType.CLIP_TO_IMAGE)
    Gimp.file_save(Gimp.RunMode.NONINTERACTIVE, merged, Gio.File.new_for_path(path + ".png"), None)
    merged.delete()
    print("wrote", name, os.path.getsize(path + ".xcf"), "bytes")


def rgba_layer(img, name, w, h, seed, mode=Gimp.LayerMode.NORMAL, opacity=100.0):
    layer = Gimp.Layer.new(img, name, w, h, Gimp.ImageType.RGBA_IMAGE, opacity, mode)
    img.insert_layer(layer, None, 0)
    fill(layer, w, h, 4, 8, lambda x, y, c: alpha(x, y, w, h, 255) if c == 3 else pattern(x + seed, y, c, 255))
    return layer


def layers_8bit():
    img = Gimp.Image.new(W, H, Gimp.ImageBaseType.RGB)
    img.set_resolution(300.0, 300.0)
    bg = Gimp.Layer.new(img, "Background", W, H, Gimp.ImageType.RGB_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(bg, None, 0)
    fill(bg, W, H, 3, 8, lambda x, y, c: pattern(x, y, c, 255))
    a = rgba_layer(img, "Multiply 60%", 20, 16, 3, Gimp.LayerMode.MULTIPLY, 60.0)
    a.set_offsets(5, 4)
    a.set_lock_alpha(True)
    a.set_color_tag(Gimp.ColorTag.GREEN)
    b = rgba_layer(img, "Screen off-canvas", 24, 12, 7, Gimp.LayerMode.SCREEN, 100.0)
    b.set_offsets(-8, 20)
    hidden = rgba_layer(img, "Hidden", 10, 10, 11)
    hidden.set_offsets(28, 2)
    hidden.set_visible(False)
    img.add_hguide(10)
    img.add_vguide(25)
    save(img, "layers-8bit")


def groups():
    img = Gimp.Image.new(W, H, Gimp.ImageBaseType.RGB)
    bg = rgba_layer(img, "Background", W, H, 0)
    group = Gimp.GroupLayer.new(img, "Group")
    img.insert_layer(group, None, 0)
    group.set_opacity(70.0)
    c1 = rgba_layer(img, "Child Multiply", 20, 20, 2, Gimp.LayerMode.MULTIPLY)
    img.reorder_item(c1, group, 0)
    c1.set_offsets(3, 3)
    nested = Gimp.GroupLayer.new(img, "Nested pass-through")
    img.insert_layer(nested, group, 0)
    nested.set_mode(Gimp.LayerMode.PASS_THROUGH)
    c2 = rgba_layer(img, "Nested child Screen", 16, 12, 5, Gimp.LayerMode.SCREEN)
    img.reorder_item(c2, nested, 0)
    c2.set_offsets(18, 10)
    top = rgba_layer(img, "Top", 12, 8, 9)
    top.set_offsets(26, 20)
    save(img, "groups")


def mask():
    img = Gimp.Image.new(W, H, Gimp.ImageBaseType.RGB)
    bg = Gimp.Layer.new(img, "Background", W, H, Gimp.ImageType.RGB_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(bg, None, 0)
    fill(bg, W, H, 3, 8, lambda x, y, c: [200, 200, 200][c])
    masked = rgba_layer(img, "Masked", 30, 20, 4)
    masked.set_offsets(5, 5)
    m = masked.create_mask(Gimp.AddMaskType.WHITE)
    masked.add_mask(m)
    fill(m, 30, 20, 1, 8, lambda x, y, c: x * 255 // 29)
    disabled = rgba_layer(img, "Mask disabled", 20, 10, 6)
    disabled.set_offsets(15, 18)
    m2 = disabled.create_mask(Gimp.AddMaskType.BLACK)
    disabled.add_mask(m2)
    disabled.set_apply_mask(False)
    save(img, "mask")


def gray16():
    img = Gimp.Image.new_with_precision(W, H, Gimp.ImageBaseType.GRAY, Gimp.Precision.U16_NON_LINEAR)
    bg = Gimp.Layer.new(img, "Background", W, H, Gimp.ImageType.GRAY_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(bg, None, 0)
    fill(bg, W, H, 1, 16, lambda x, y, c: pattern(x, y, c, 65535))
    top = Gimp.Layer.new(img, "Gray alpha", 20, 15, Gimp.ImageType.GRAYA_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(top, None, 0)
    fill(top, 20, 15, 2, 16, lambda x, y, c: alpha(x, y, 20, 15, 65535) if c == 1 else pattern(x + 9, y, 0, 65535))
    top.set_offsets(10, 8)
    save(img, "gray16", perceptual_variant=False)


def float_linear():
    img = Gimp.Image.new_with_precision(W, H, Gimp.ImageBaseType.RGB, Gimp.Precision.FLOAT_LINEAR)
    bg = Gimp.Layer.new(img, "Background", W, H, Gimp.ImageType.RGB_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(bg, None, 0)
    fill(bg, W, H, 3, 32, lambda x, y, c: (x * 3 + y * 5 + c * 7) / 64.0)
    top = Gimp.Layer.new(img, "Float alpha", 16, 16, Gimp.ImageType.RGBA_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(top, None, 0)
    fill(top, 16, 16, 4, 32, lambda x, y, c: (x + y) / 30.0 if c == 3 else (x * 2 + y * 3 + c * 5) / 64.0)
    top.set_offsets(12, 7)
    save(img, "float-linear", perceptual_variant=False)


def indexed():
    img = Gimp.Image.new(W, H, Gimp.ImageBaseType.RGB)
    bg = rgba_layer(img, "Background", W, H, 0)
    top = rgba_layer(img, "Top", 20, 20, 5)
    top.set_offsets(10, 5)
    img.convert_indexed(Gimp.ConvertDitherType.NONE, Gimp.ConvertPaletteType.GENERATE, 16, False, False, "")
    save(img, "indexed", perceptual_variant=False)


def text():
    img = Gimp.Image.new(W * 4, H * 2, Gimp.ImageBaseType.RGB)
    bg = Gimp.Layer.new(img, "Background", W * 4, H * 2, Gimp.ImageType.RGB_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(bg, None, 0)
    fill(bg, W * 4, H * 2, 3, 8, lambda x, y, c: 255)
    font = Gimp.context_get_font()
    tl = Gimp.TextLayer.new(img, "XCF", font, 24.0, Gimp.Unit.pixel())
    img.insert_layer(tl, None, 0)
    tl.set_offsets(10, 10)
    save(img, "text")


def channel():
    img = Gimp.Image.new(W, H, Gimp.ImageBaseType.RGB)
    bg = rgba_layer(img, "Background", W, H, 0)
    color = Gegl.Color.new("red")
    ch = Gimp.Channel.new(img, "Alpha 1", W, H, 50.0, color)
    img.insert_channel(ch, None, 0)
    fill(ch, W, H, 1, 8, lambda x, y, c: y * 255 // (H - 1))
    img.select_rectangle(Gimp.ChannelOps.REPLACE, 4, 6, 20, 12)
    save(img, "channel", perceptual_variant=False)


def legacy_modes():
    img = Gimp.Image.new(W, H, Gimp.ImageBaseType.RGB)
    bg = Gimp.Layer.new(img, "Background", W, H, Gimp.ImageType.RGB_IMAGE, 100.0, Gimp.LayerMode.NORMAL)
    img.insert_layer(bg, None, 0)
    fill(bg, W, H, 3, 8, lambda x, y, c: pattern(x, y, c, 255))
    modes = [
        ("Addition", Gimp.LayerMode.ADDITION),
        ("Subtract", Gimp.LayerMode.SUBTRACT),
        ("Divide", Gimp.LayerMode.DIVIDE),
        ("Dodge", Gimp.LayerMode.DODGE),
        ("Burn", Gimp.LayerMode.BURN),
        ("Grain merge", Gimp.LayerMode.GRAIN_MERGE),
        ("HSV Hue", Gimp.LayerMode.HSV_HUE),
        ("HSV Value", Gimp.LayerMode.HSV_VALUE),
        ("LCH Color", Gimp.LayerMode.LCH_COLOR),
        ("Hard light", Gimp.LayerMode.HARDLIGHT),
        ("Exclusion", Gimp.LayerMode.EXCLUSION),
        ("Linear light", Gimp.LayerMode.LINEAR_LIGHT),
    ]
    for i, (name, mode) in enumerate(modes):
        layer = rgba_layer(img, name, 8, 8, i * 3, mode)
        layer.set_offsets((i % 5) * 8, (i // 5) * 10)
    save(img, "legacy-modes")


for make in [layers_8bit, groups, mask, gray16, float_linear, indexed, text, channel, legacy_modes]:
    try:
        make()
    except Exception as e:  # noqa: BLE001
        print("FAILED", make.__name__, repr(e))
print("done")
