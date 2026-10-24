# GIMP XCF documents

PhotoCraft imports GIMP's `.xcf` files (and `.xcf.gz`, `.xcf.bz2`, `.xcf.xz`) into native editable
PhotoCraft document data. The source is read-only; native edits and Save As remain available. File ›
Open, recent files, drops, the CLI (`convert`, `info`, `batch`) and automation (`doc.open`, MCP)
use the same reader in `photocraft-io` (`crates/io/src/xcf.rs`). Plain and wrapped files are
recognised by the inflated `gimp xcf ` signature, including under another extension. Concatenated
members form one bounded logical stream; checksums and compressed EOF are verified, XZ stream
padding is accepted, and unrelated trailing compressed data is rejected. XZ decoding has a
separate 64 MiB dictionary/decoder budget checked before allocation. Save opens Save As and
defaults to PhotoCraft (`.pcraft`) for these conversion-bearing imports, keeping everything
PhotoCraft holds. Exporting as `.xcf` fails with a message.

## What is kept

| | Open |
|---|---|
| Pixel layers, including off-canvas positions, opaque or with alpha | yes, within the import limits below |
| Groups, nested; a group in Pass through mode | yes |
| Names, visibility, opacity, offsets | yes |
| Layer masks | yes, applied or disabled as in GIMP |
| Blend modes | the 2.10 set and the legacy set, mapped to Photoshop's (table below) |
| Locks (alpha, content, position), colour tags | yes |
| Precision | 8 and 16-bit integer as such; normalized 32-bit integer and 16/32/64-bit float as native 32-bit float |
| Linear-light precisions | interpreted using the saved sample encoding and a compatible native ICC profile; unsupported essential profile data produces an error |
| Colour profile (`icc-profile` parasite), resolution, guides, comment | yes |
| Grayscale, RGB | as such; indexed images open as RGB |
| Extra channels, the saved selection | yes (alpha channels, the selection) |
| Text | supported saved text/styles become native editable text, rendered with native fonts and typography; modified or unsupported text preserves its stored pixels with a notice |
| Saved paths and vector layers | supported curves/paint become native paths and shapes; unsupported or independently modified paint preserves stored pixels and inert source configuration |
| Link layers | usable stored pixels/placement are preserved; source locators remain inert and cannot create live filesystem links |
| Stored effects | supported operation/version/argument combinations become ordinary native filters, styles, fills or embedded native structures; incomplete mappings remain unapplied with per-item outcomes |
| Stored graph effects | bounded known chains, references and auxiliary images become ordinary native contents, masks, fills, transforms and filters; required missing images are reported |
| Floating selections and legacy linked movement | separate positioned native layers and supported native link groups; attachment/anchoring differences are disclosed |
| Saved view markers and sample points | supported initial native targets are applied once; samplers use the native capacity of 10 and remain session-only |
| Compression | none, RLE, zlib |
| XCF versions | 0 to 26 (GIMP 1.x to 3.2); newer versions are attempted using known framing with a notice; unfamiliar essential data may fail |

U32 and F64 imports report reduced precision. Finite negative and above-one color values are
preserved at native float precision. Nonfinite samples, values beyond the finite native F32
range, invalid coverage and malformed or missing palette entries produce an import error.
Nonzero double samples that round to zero produce a conversion notice for the affected item.

Known graph syntax is converted once during import. Auxiliary masks and displacement maps are
captured from the current native inputs into static native resources. They can be edited with
ordinary PhotoCraft controls; they do not keep a live GIMP graph dependency. Native compositing,
channel, profile, precision and sampling rules apply, and the conversion report records the
approximations. No GIMP or GEGL renderer is installed in the native document.

Full stock-XCF qualification is still in progress. A supported operation name does not imply support
for every saved argument, resource or owner. Unknown operation versions, unavailable resources
and incomplete mappings remain inert with an explicit conversion outcome. Supported named and
hexadecimal text-markup colors become ordinary editable native text colors, with native profile
conversion and source alpha preserved. Unknown color names retain an explicit stored-pixel fallback. Source item sets
and unsupported markers remain historical metadata rather than new editing behavior.

The compatibility boundary is stock GIMP 3.2.6 and documented XCF 0–26 records. Synthetic wire
tests and locally saved historical samples cover specific records; they do not qualify every
historical GIMP release. High-depth versions 7–11 require an explicit sample byte-order choice:
those development files stored the writer machine's byte order without recording it in the file.
Open asks for little-endian or big-endian before creating a document; Cancel keeps the current
document. The selected order applies only to sample bytes, including layer alpha, masks and
channels. Structural records and modern XCF sample order remain unchanged. The choice is recorded
as import provenance; native editing and `.pcraft` reopening require no XCF byte-order setting.

CLI `convert`, `info`, `run` and `batch` accept
`--historical-xcf-sample-order little-endian` (or `big-endian`). Automation `doc.open`, MCP
`doc_open`, desktop `app.open`, and engine `file.openAs` accept
`historicalXcfSampleOrder: "little-endian"` (or `"big-endian"`). Without a choice, noninteractive
imports refuse ambiguous high-depth files rather than publish potentially wrong pixels. The
public IO API exposes the same choice through `ImportOptions::historical_xcf_sample_order`.
Genuine U16/F32 sources qualify selected little-endian cases across XCF7–11: Windows GIMP
2.9.6 supplies versions 7/8/9/11, and unmodified GIMP 2.9.5 development commit
[`0874703`](https://github.com/GNOME/gimp/commit/0874703e3b7f2d211816feccd745404ed8ec3d71)
supplies version 10. Every source was independently saved, reopened twice and checked against
its original formula and stored bytes. The v8 and v10 files use the writers' internal zlib
compression; the U16 samples are asymmetric. Synthetic checks cover both orders across 7–11.
These isolated runtimes qualify historical development formats; they are not PhotoCraft
dependencies, and stable-source coverage is separate. The publicly
pinned corpus currently contains 14 XCF originals and their 14 rendered references. Expanded
source fixtures are proposed in [corpus PR #9](https://github.com/storytold/photocraft-corpus/pull/9);
upstream acceptance and an immutable pin/manifest update are still required for ordinary CI.

Open notices summarize material differences. The read-only **Import Conversion Report** lists
native equivalents, native approximations and unapplied source features per item; automation
can page the same data using `document.importReport`. The report survives `.pcraft` saves and
does not control rendering, resource access or document writability. Native reopen needs neither
the source XCF nor GIMP. Source paths and existing aliases are protected against accidental
save-back; normal native Save As creates writable work.

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

Imported documents use PhotoCraft's normal compositing and editing rules. Three source defaults
can therefore render differently; the first two are noted on open when they apply:

- **Linear light.** Since 2.10 GIMP blends most modes and composites every layer (alpha blending
  too) in linear light by default. PhotoCraft, like Photoshop, composes in the document's own
  encoding. Semi-transparent edges, masks and most blend modes therefore render slightly
  differently: in the raster test files, a Multiply layer at 60% differs by up to 38/255 in one
  channel. The listed raster fixtures with blend and composite spaces set to *RGB (perceptual)*
  match within 2.5/255 where their native blend mappings agree; the table below lists exceptions.
  Editable text uses native typography and is checked against independent native text instead.
- **Clip to backdrop.** GIMP composites every mode but Normal with *Clip to backdrop*: the layer
  shows nothing where the layers beneath it are transparent. PhotoCraft shows it there. The
  difference only appears over transparent areas of the stack.

- **Clamping after alpha.** For modes whose result can leave 0..1 (Addition, Subtract, Divide,
  Dodge, Burn, Linear light), GIMP applies the pixel's alpha and the layer's opacity first and
  clamps at the end; Photoshop clamps the blend result first. For normalized opaque values under
  matching encoding, clamping order alone has no effect; other source/native differences still
  apply. Translucent values can differ where the raw blend result overshoots.

Modes without an equivalent open as Normal, and the HSV/LCH/luma modes use Photoshop's HSL-based
formulas, so those layers render differently; the note names each layer.

## Limits and safety

Written from the public XCF specification (https://developer.gimp.org/core/standards/xcf/) and
files GIMP 3.2 writes; no GIMP code. Every read is bounds-checked and returns an error on a
malformed file, never a panic: canvases at most 300,000 px a side and 1 GiB of decoded pixels,
at most 8,192 layers and 1,024 channels, nesting at most 64 deep, property payloads at most
64 MiB, strings 1 MiB, compressed inputs inflating to at most 1 GiB. Tile data is decoded into the
exact size the drawable declares; a run that crosses a tile or a zlib stream that ends early is an
error. The property lengths the specification warns about (`PROP_COLORMAP`, `PROP_USER_UNIT`) are
computed from the payload instead of trusted. Truncated files are covered by tests at every cut.
Generated native resources and embedded contents have separate bounded working/source
allowances and cancellation checks. These component limits are not a whole-process memory claim.

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
| `layers-8bit-perceptual`, `mask-perceptual` | within 2.5/255 everywhere |
| `text`, `text-perceptual` | saved typography is checked against independently constructed native text; native shaping and rasterization determine appearance |
| `gray16`, `indexed`, `channel` | within 2/255 |
| `groups-perceptual` | within 2.5/255 except where Clip to backdrop applies (15% of pixels) |
| `legacy-modes-perceptual` | Addition, Subtract, Divide, Dodge, Burn, Hard light, Exclusion, Linear light within 2.5/255 on opaque pixels (translucent ones differ by the clamping order above); the approximated modes reported |
| the default (linear-light) raster files | the known gap, measured: up to 38/255 (Multiply), 59/255 (mask edge); editable text uses the independent native typography check above |

Run the public baseline and native conversion regressions from the repository root:

```sh
cargo xtask corpus --gimp
cargo test -p photocraft-io --features corpus --test it xcf
cargo test -p photocraft-engine --test it xcf_native
cargo test -p photocraft-ui-egui --test xcf_native_workflows
```

These tests use publicly pinned files or independently constructed synthetic inputs. Locally
authored, unpublished corpus additions are not required to run them and do not establish full
source-family acceptance. Native counterpart tests validate PhotoCraft rendering, editing and
`.pcraft` persistence; GIMP-rendered images document source differences rather than define a
second compositing model.

The additional `xcf_source_evidence` suite consumes unpublished, independently saved XCF
fixtures for precision, profiles, text, paths, vectors, floating/linked content and effects.
Its tests are explicitly ignored pending publication and an upstream corpus pin update.
Running that suite with `--ignored` currently requires those additional local corpus files;
it cannot yet be reproduced from the public pin alone. Local passes do not accept those
source families as complete.

The ignored genuine historical-source test also accepts `PHOTOCRAFT_XCF_SOURCE_CORPUS_DIR`
pointing to the authoring checkout's `gimp/` directory. This test-only override allows explicit
local qualification without changing the ordinary corpus cache or upstream fetch defaults.
