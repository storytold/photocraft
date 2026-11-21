# GIMP XCF documents

PhotoCraft opens GIMP's native `.xcf` files, including gzip, bzip2 and XZ wrappers, with their layers, read only. File ›
Open, recent files, drops, the CLI (`convert`, `info`, `batch`) and automation (`doc.open`, MCP)
use the same enhanced import in `photocraft-io` (`crates/io/src/xcf.rs`). This adapter retains the precision, metadata and editable-content decoder while the other format adapters use shared craft-libs readers. A file is recognised by its
`gimp xcf ` signature, so an XCF with another extension still opens as one. Save suggests
`.pcraft`, which keeps everything PhotoCraft holds; exporting as `.xcf` fails with a message.

## Historical sample byte order

Unmarked XCF versions 7–11 with multi-byte samples require the byte order used when the file was
saved. Select **Little-endian** or **Big-endian** when desktop Open, Replace Contents, or Relink
to File asks. Edit Contents also asks when an older bundle embeds such a source without a saved
choice. The chosen order is stored with the Smart Object. Relink keeps the previous choice when
the source bytes match.

For CLI Convert, Info, Run, Batch, and Droplet, pass `--historical-xcf-sample-order little-endian` or
`--historical-xcf-sample-order big-endian`. Use `historical_xcf_sample_order` for MCP `doc_open`
and for headless MCP `doc_place` and `doc_replace_smart_object`.
For desktop control `app.open`, use `historicalXcfSampleOrder`. Both parameters accept
`"little-endian"` or `"big-endian"`.
Use **Historical XCF Sample Order** in desktop Batch, Image Processor, Create Droplet, Load
Files into Stack, Statistics, and Contact Sheet II. Choose **Unspecified** to leave files that
require a choice as import errors. Create Droplet saves the choice for later runs.
When Batch runs with `overrideOpen`, an order in the recorded Open step takes precedence.
Open steps without an order use the Batch choice.

## Decoder ownership

PhotoCraft keeps the enhanced XCF decoder in its own IO crate because it is the only current
consumer of the XCF reader. We will revisit moving it to `craft-xcf` only if another app needs
XCF import.

## What is kept

| | Open |
|---|---|
| Pixel layers (any size and position, also off-canvas), opaque or with alpha | yes |
| Groups, nested; a group in Pass through mode | yes |
| Names, visibility, opacity, offsets | yes |
| Layer masks | yes, applied or disabled as in GIMP |
| Blend modes | the 2.10 set and the legacy set, mapped to Photoshop's (table below) |
| Locks (alpha, content, position), colour tags | yes |
| Precision | 8-bit and 16-bit integer remain integer. U32 and 16-bit, 32-bit, or 64-bit float import as F32 |
| Linear-light precisions | a usable assigned linear RGB profile can preserve its colorants with a linear transfer curve. The fallback is linear sRGB or linear gray |
| Colour profile (`icc-profile` parasite), resolution, guides, comment | usable profiles and the other fields are retained. Unusable profile assignments fall back with an import diagnostic |
| Grayscale, RGB | as such; indexed images open as RGB |
| Extra channels, the saved selection | yes (alpha channels, the selection) |
| Text and vector layers | supported records become editable native content; other records keep usable saved pixels and a conversion report |
| Linked content | inert references and saved pixels; external resources are not fetched |
| Compression | none, RLE, zlib; gzip, bzip2 and XZ transport wrappers |
| XCF versions | 0 to 26 (GIMP 1.x to 3.2); newer files open with a note, unknown parts skipped |

Text layers in documents with unequal horizontal and vertical DPI keep their saved pixels.
Vector layers with an enabled pattern also keep their saved pixels, with the source path
available separately for editing. Active paint that cannot be decoded, including writer-endian
float colours without byte-order evidence, also preserves the saved appearance.

Saved paths, guides and supported editing-view selections are imported. Floating pixels remain separate raster layers. Unsupported layer effects and optional records are disclosed in the conversion report. Retention limits omit optional data while preserving usable pixels; structural and pixel allocation limits still reject unsafe inputs.

## Blend modes

| GIMP | PhotoCraft |
|---|---|
| Normal, Dissolve, Multiply, Screen, Overlay, Difference, Darken only, Lighten only, Divide, Hard light, Vivid light, Pin light, Linear light, Hard mix, Exclusion, Linear burn, Subtract | the same |
| Addition | Linear Dodge (Add) |
| Dodge, Burn | Color Dodge, Color Burn |
| Pass through (groups) | Pass Through |
| Soft light (both the 2.10 and the legacy one) | Soft Light (GIMP's formula differs a little; noted) |
| HSV Hue / Saturation / Value, HSL Color, LCH Hue / Chroma / Color / Lightness, Luminance | Hue, Saturation, Luminosity, Color (Photoshop's HSL-based modes; noted) |
| Luma darken only, Luma lighten only | Darker Color, Lighter Color (noted) |
| Behind, Grain extract, Grain merge, Color erase, Erase, Merge, Split | Normal, with a note |

## How the composite differs from GIMP

Three of GIMP's defaults have no Photoshop counterpart, so a file opened in PhotoCraft can render
a little differently from GIMP; the first two are noted on open when they apply:

- **Linear light.** Since 2.10 GIMP blends most modes and composites every layer (alpha blending
  too) in linear light by default. PhotoCraft, like Photoshop, composes in the document's own
  encoding. Semi-transparent edges, masks and most blend modes therefore render slightly
  differently: in the test files, a Multiply layer at 60% differs by up to 38/255 in one channel,
  anti-aliased text edges by up to 73/255 at the edge pixels. A layer whose blend and composite
  spaces GIMP set to *RGB (perceptual)* matches PhotoCraft within 2.5/255 (the `-perceptual` test
  files).
- **Clip to backdrop.** GIMP composites every mode but Normal with *Clip to backdrop*: the layer
  shows nothing where the layers beneath it are transparent. PhotoCraft shows it there. The
  difference only appears over transparent areas of the stack.

- **Clamping after alpha.** For modes whose result can leave 0..1 (Addition, Subtract, Divide,
  Dodge, Burn, Linear light), GIMP applies the pixel's alpha and the layer's opacity first and
  clamps at the end; Photoshop clamps the blend result first. Fully opaque pixels at 100% match
  exactly; translucent ones differ where the raw result overshoots.

Modes without an equivalent open as Normal, and the HSV/LCH/luma modes use Photoshop's HSL-based
formulas, so those layers render differently; the note names each layer.

## Limits and safety

Written from the public XCF specification (https://developer.gimp.org/core/standards/xcf/) and
files GIMP 3.2 writes; no GIMP code. Every read is bounds-checked and returns an error on a
malformed file, never a panic: canvases at most 300,000 px a side and 1 GiB of decoded pixels,
at most 8,192 layers and 1,024 channels, nesting at most 64 deep, property payloads at most
64 MiB, strings 1 MiB, gzipped files inflating to at most 1 GiB. Tile data is decoded into the
exact size the drawable declares; a run that crosses a tile or a zlib stream that ends early is an
error. The property lengths the specification warns about (`PROP_COLORMAP`, `PROP_USER_UNIT`) are
computed from the payload instead of trusted. Truncated files are covered by tests at every cut.

## Verification

Unit tests (`crates/io/src/xcf/tests.rs`) build XCF files from the specification with a small
writer and check every compression, precisions, pointer widths, groups by item path, masks,
channels, the colour map, parasites, limits and malformed input.

Real files come from GIMP 3.2.6 itself: `gimp/` of [photocraft-corpus](https://github.com/storytold/photocraft-corpus)
(generator `tools/gimp-xcf/xcf_fixtures.py`, run inside GIMP), fetched into `corpus/gimp/` by
`cargo xtask corpus --gimp` (28 files, 251 KB): procedural layers the tests regenerate sample for
sample, and for each file GIMP's own merged rendering as a PNG. `crates/io/tests/it/xcf.rs` (feature
`corpus`, run by `cargo xtask test-corpus -p io`) checks the layer data against the formulas and our
composite against GIMP's:

| File | Composite vs GIMP |
|---|---|
| `layers-8bit-perceptual`, `mask-perceptual`, `text-perceptual` | within 2.5/255 everywhere |
| `gray16`, `indexed`, `channel` | within 2/255 |
| `groups-perceptual` | within 2.5/255 except where Clip to backdrop applies (15% of pixels) |
| `legacy-modes-perceptual` | Addition, Subtract, Divide, Dodge, Burn, Hard light, Exclusion, Linear light within 2.5/255 on opaque pixels (translucent ones differ by the clamping order above); the approximated modes reported |
| the default (linear-light) files | the known gap, measured: up to 38/255 (Multiply), 59/255 (mask edge), 73/255 (text edge) |
