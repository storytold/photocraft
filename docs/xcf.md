# GIMP XCF documents

PhotoCraft opens GIMP's native `.xcf` files and gzip, bzip2, and XZ wrapped XCF files with their layers, read only. File ›
Open, recent files, drops, the CLI (`convert`, `info`, `run`, `batch`) and automation (`doc.open`, MCP)
use the same importer in `photocraft-io` (`crates/io/src/xcf.rs`). The shared
[`craft-xcf` reader](https://github.com/storytold/craft-libs/tree/main/crates/xcf) recognizes the
`gimp xcf ` signature, reads the wrapper and records, and delivers source samples one tile at a time.
PhotoCraft maps those records into editable native data and applies its own color, blending,
and document policies. A signature match also opens XCF files with a different extension. Save suggests
`.pcraft`, which keeps everything PhotoCraft holds; exporting as `.xcf` fails with a message.

## What is kept

| | Open |
|---|---|
| Pixel layers (any size and position, also off-canvas), opaque or with alpha | yes |
| Groups, nested; a group in Pass through mode | yes |
| Names, visibility, opacity, offsets | yes |
| Layer masks | yes, applied or disabled as in GIMP |
| Blend modes | the 2.10 set and the legacy set, mapped to Photoshop's (table below) |
| Locks (alpha, content, position), colour tags | yes |
| Precision | 8-bit and 16-bit integers retain their depth. 32-bit integers and 16-bit, 32-bit, and 64-bit floats open as 32-bit floats. |
| Linear and perceptual encodings | PhotoCraft preserves usable assigned colorants while adapting the native ICC transfer curve for the stored samples |
| Colour profile (`icc-profile` parasite), resolution, guides, comment | yes |
| Raw EXIF | Complete TIFF packets within the metadata allowance |
| Grayscale, RGB | as such; indexed images open as RGB |
| Extra channels, the saved selection | yes (alpha channels, the selection) |
| Saved paths | Supported Bézier strokes open as editable named paths |
| Text layers | Supported text configurations open as editable text with their saved appearance. Native typography applies after editing. Other text keeps its saved pixels. |
| Vector and link layers | Their saved pixels |
| Saved view | Supported layer, mask, channel, path, and sampler hints accompany the first native admission |
| Compression | none, RLE, zlib |
| XCF versions | 0 to 26 (GIMP 1.x to 3.2); newer files open with a note, unknown parts skipped |

The conversion report describes how each GIMP layer filter was handled. Editor Open, raster Place
Embedded and rooted automation Open can turn supported Exposure and Brightness/Contrast stages
into editable Smart Filters when the session authorizes the native commands. RGB float layers
can also retain a Levels stage with normalized ascending endpoints and full-strength settings.
Qualified RGB sources can retain Invert, Posterize, Vibrance, Saturation, Channel Mixer and Sepia
stages. The report records that these stages use PhotoCraft's native rendering rules.
A single masked Exposure in an XCF 22 file can retain independent filter opacity and coverage on an
opaque 8-bit or 16-bit RGB raster layer when the stored mask covers the image frame and uses supported settings.
Supported graph chains can contain sequential Invert, Exposure, Brightness/Contrast and Levels
stages. One unmasked, full-strength source-over graph can place the filtered owner over its
original as two editable layers inside a Smart Object. Unsupported stages, graph branches,
coverage and unresolved blend settings keep the saved layer pixels. The report can retain
the original records for inspection within its storage limits. Floating selection attachment
and item-set behavior remain outside the native mapping.

Ordinary background Open on the desktop prompts for a sample byte order when a multi-byte XCF 4–11
file leaves it unspecified. Open As, raster Place Embedded and Smart Object workflows accept an
explicit order. Historical linked Smart Objects bind the order to a verified source revision.
CLI `convert`, `info`, `run`, and
`batch` accept `--historical-xcf-sample-order` with `little-endian` or `big-endian`. In Batch, a
recorded Open As of the same source inherits this order when the step has no explicit choice.
Each nested Batch has its own scope. PhotoCraft records the order used in the
native document's conversion report. Text color blobs have a separate source
encoding. Text that needs a float color with an unknown byte order keeps the saved raster, and the
report can retain the original configuration for inspection. Portable numeric colors remain editable.
Rooted automation can use `holdHistoricalXcf` on `doc.open` and answer the resulting request with
`doc.resolveHistoricalXcfOpen`. The MCP tools expose the equivalent behavior. The answer applies
to the exact bytes held by that backend.

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
64 MiB and strings at most 1 MiB. Wrappers inflate to at most 1 GiB and use bounded decoder
working memory. The reader validates tile framing and compressed stream completion. PhotoCraft
checks the logical drawable size separately from simultaneously live native tiles and scratch
data. The reader computes the payload length of `PROP_COLORMAP` and `PROP_USER_UNIT` by file
version. Truncated files are covered by tests at every cut.

## Verification

Unit tests (`crates/io/src/xcf/tests.rs`) build XCF files from the specification with a small
writer and check every compression, precisions, pointer widths, groups by item path, masks,
channels, the colour map, parasites, limits and malformed input.

The `gimp/` directory of [photocraft-corpus](https://github.com/storytold/photocraft-corpus) stores
authored GIMP files from several releases, merged PNG oracles, and wrapped samples. Run
`cargo xtask corpus --gimp` to fetch the 112 checksum-pinned assets into `corpus/gimp/`.
`crates/io/tests/it/xcf_source_evidence.rs` verifies native text and path handling and effect inventory
against the archived source. `crates/io/tests/it/xcf.rs` checks procedural layer samples and compares
the PhotoCraft composite with GIMP's. Run both through `cargo xtask test-corpus -p io`.

| File | Composite vs GIMP |
|---|---|
| `layers-8bit-perceptual`, `mask-perceptual`, `text-perceptual` | within 2.5/255 everywhere |
| `gray16`, `indexed`, `channel` | within 2/255 |
| `groups-perceptual` | within 2.5/255 except where Clip to backdrop applies (15% of pixels) |
| `legacy-modes-perceptual` | Addition, Subtract, Divide, Dodge, Burn, Hard light, Exclusion, Linear light within 2.5/255 on opaque pixels (translucent ones differ by the clamping order above); the approximated modes reported |
| the default (linear-light) files | the known gap, measured: up to 38/255 (Multiply), 59/255 (mask edge), 73/255 (text edge) |
