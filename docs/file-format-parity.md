# File-format parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first version: every Photoshop 2026 format from its bundle against our readers and writers; Affinity notes moved here from `roadmap.md`) · **Target:** Adobe Photoshop 2026

Every format Photoshop 2026 (27.11.0) reads or writes, with PhotoCraft's support, fidelity and
tests. Photoshop's list comes from its `Info.plist` `CFBundleDocumentTypes`, the PiPL read/write
flags of `Contents/PlugIns/Required/File Formats`, and format names in the main binary (Open/Save
As behaviour from Adobe's documentation where the bundle doesn't say). Ours comes from
`crates/codecs/src/format.rs` (capability table), `crates/io/src/lib.rs` (import/export
dispatch), `crates/psd`, `crates/raw`, `crates/heif`, `crates/affinity`. More detail on our side:
[raster formats](../book/src/formats/raster-formats.md), [PSD/PSB](../book/src/formats/psd-psb.md),
[OpenRaster](ora.md), [Paint.NET](pdn.md).

**Summary.** By count, 13 of Photoshop's 31 formats are fully supported in the directions Photoshop supports them, 4 partially (48%,
measured). Weighted by use (PSD/PSB 40%, JPEG 15%, PNG 15%, TIFF 10%, camera raw 8%, WebP/AVIF/
HEIF/JXL 6%, PDF 3%, everything else 3%), presence is ~80% and ready-for-real-work ~60%: PSD
is broad but not yet reliable against Photoshop (gap G1), and Photoshop PDF, AVIF read and JPEG
XL are missing (gap G10). Remaining: 300–500 h including PSD fidelity.

## Photoshop's formats

| Format | Photoshop | PhotoCraft read | PhotoCraft write | Fidelity and tests |
|---|---|---|---|---|
| PSD | R/W | yes | yes | 1/8/16/32-bit; Bitmap, Gray, Indexed, RGB, CMYK, Multichannel, Lab, Duotone; layers, groups, masks, vector masks, adjustment and fill layers, effects, text, shapes, smart objects (SoLd/lnk2), smart filters (14 mapped, rest verbatim), slices, comps, Advanced Blending. Oracle floors: Photoshop-authored 133/256, io 146/170, psd-tools 236/309; round trip ≥ 169/170, 308/309, 258/256. Known failures: #2374, #1281, #2469, #2465 |
| PSB | R/W | yes | yes | Auto-switches above 2 GB (REL-212-2); 4–5 GB fixtures generated (FILE-216-2); > 4 GB fails on Windows (#375) |
| Cloud PSD (.psdc) | R/W | no | no | Out of scope (Adobe cloud) |
| TIFF | R/W | yes | yes | 8/16/32f, Gray/RGB/CMYK, BigTIFF, every IFD, banded parallel decode; layered TIFF both byte orders (FILE-215-7: extra alpha channels not written) |
| JPEG | R/W | yes | yes | 8-bit Gray/RGB/CMYK, ICC/EXIF/XMP; no JPEG Options dialog (quality 0–12, baseline/progressive) (FILE-215-19) |
| PNG | R/W | yes | yes | 8/16-bit, ICC/text; APNG first frame only; alpha channels on open (#2225) |
| GIF | R/W | yes | yes | First frame only |
| BMP | R/W | yes | yes | 8-bit |
| WebP | R/W | yes | yes | Lossless and our own lossy VP8 encoder; ICC/EXIF/XMP |
| AVIF | R/W | no (#463) | optional feature, off by default | |
| JPEG XL | R/W | inside DNG only | no | |
| HEIF/HEIC | R/W(HEIF) | yes (pure-Rust heic-rs, `heif` feature, on in releases) | no | 8/16-bit |
| OpenEXR | R/W | yes | yes | f16/f32; deep EXR and Cryptomatte readers (`corpus/exr`) |
| Radiance HDR | R/W | yes | yes | f32 |
| Portable Bit Map (PBM/PGM/PPM/PNM/PFM/PAM) | R/W | yes | yes | 8/16/32f |
| Targa | R/W | yes | yes | 8-bit |
| Photoshop PDF | R/W | no (#2541) | raster only (print, artboards to PDF) | No vector/text PDF, no multi-page save, no PDF presentation (FILE-215-10) |
| Photoshop EPS, DCS 1.0/2.0, EPS TIFF/PICT preview | R/W | no | no | |
| Generic EPS / AI | R (rasterize) | no | no | |
| SVG | R (rasterize) | yes, as editable shape layers or a vector smart object | no | Better than Photoshop for editing; gradients approximated (FILE-215-9) |
| JPEG 2000 | R/W | no (#1047) | no | |
| Cineon / DPX | R (DPX), R/W (CIN) | no (#1049) | no | |
| DICOM | R/W | no | no | |
| PCX | R/W | no | no | |
| Pixar | R/W | no | no | |
| Amiga IFF | R/W | no | no | |
| Wireless Bitmap | R/W | no | no | |
| MPO / JPS (stereo) | R/W | no | no | |
| PICT | R | no | no | Legacy |
| Scitex CT | R/W | no | no | Legacy |
| Photoshop Raw (.raw) | R/W | no | no | |
| Camera raw (via Camera Raw 18.7) | R | partial | – | See below |

## Camera raw

Photoshop opens raws through Camera Raw 18.7 (the bundle registers cr2, crw, dcr, dng, erf, mos,
fff, 3fr, mef, mfw, iiq, nrw, rwl, srw, mrw, nef, orf, pef, raf, srf, x3f; Camera Raw itself
supports many more, including CR3).

| Camera format | PhotoCraft | Evidence |
|---|---|---|
| DNG (incl. LJPEG and JPEG XL tiles) | yes; lossy DNG no | FILE-215-11 |
| Canon CR2 | yes | |
| Canon CR3 | detected, unsupported | `crates/raw/src/lib.rs:219` |
| Nikon NEF/NRW (lossless, lossy) | yes; "lossy after split" no | #50 |
| Sony ARW/SR2/SRF (incl. compressed) | yes | |
| Panasonic RW2, Pentax PEF | yes | |
| Olympus ORF | uncompressed only | |
| Fujifilm RAF (Bayer and X-Trans) | uncompressed only; no Fuji colour calibration | |
| Others (CRW, 3FR, IIQ, X3F, SRW, MRW, ERF…) | embedded preview only | |
| .xmp sidecars, highlight modes, raw preferences | missing | FILE-215-11 |
| Camera Raw as a PSD smart filter | partial | 2026-10-07: WB, Light/Presence, curves, HSL, grading, detail, grain, vignette mapped (FILE-215-12) |

## Formats Photoshop doesn't have (ours only)

| Format | Read | Write | Notes |
|---|---|---|---|
| `.pcraft` (native) | yes | yes | Incremental, autosave, crash recovery, atomic save |
| Affinity `.af`, `.afphoto`, `.afdesign`, `.afpub` | yes | no | See below |
| Paint.NET `.pdn` | yes | no | [pdn.md](pdn.md) |
| OpenRaster `.ora` | yes | yes | [ora.md](ora.md) |
| ICO, QOI | yes | yes | |
| Krita `.kra`, GIMP `.xcf`, Procreate | no | no | Not in Photoshop either |

### Affinity import notes (moved from `roadmap.md`)

2026-10-08: Affinity `.af`, `.afdesign`, `.afphoto` and `.afpub` documents (container versions
8–12) open **natively** ([format notes](../crates/affinity/README.md)): pages and artboards, layers
and groups, curves and geometric shapes with fills, gradients and strokes, text as type layers,
placed images as smart objects, pixel layers and masks, as an 8-bit RGB document without a source
save path. Measured against the thumbnail Affinity embeds in each file, 21 pinned public documents
(`cargo xtask corpus --affinity`) differ by 0–4.6 of 255 on average. Layer effects, adjustments,
live filters, brush strokes, special shapes, master pages and CMYK/Lab/16-bit document colour are
approximated or left out, each with a warning; damaged or unknown files fall back to the embedded
preview. Affinity writing is not implemented: no Affinity installation was available to check
written files, so `.af` export stays unsupported.

2026-10-10: current `.af` artboard properties (`phrp`/`aprp`) are recognized alongside the
legacy flag, including converted curve boards. Synthetic two-board regressions check separate
bounds, overflow clipping, `.pcraft`/PSD board and pixel round trips, and moving one board with its
children through undo/redo. Nested boards remain masked groups; rotated/curved board outlines
still import at their bounding rectangle.

2026-10-09: `corpus/affinity/` now has 39 pinned documents and 21 PNGs (20 rendered references and
one bitmap-fill texture): the prior 21 public documents plus 18 CC0 samples for #1606. The new
samples compare against their exported PNGs;
mean differences range from 0.44/255 (conical gradients) to 86.05/255 (RGB/32 reduced to 8-bit).
Clouds, hearts, cogs, callouts, arrows, double stars, tears, crescents, diamonds and circular segments
now import as parametric shapes; the special-shapes sample measures 5.68/255 against its PNG. The
remaining gaps in #1606 stay tracked with per-sample corpus ceilings.

## Presets and data files

| Format | Photoshop | PhotoCraft | Notes |
|---|---|---|---|
| Brushes `.abr` | R/W | R/W | `psd/src/abr.rs` (v6/v12) |
| Gradients `.grd` | R/W | R/W | |
| Patterns `.pat` | R/W | R/W | |
| Swatches `.aco`, `.ase` | R/W | R/W | FILE-215-14 |
| LUTs `.cube` / `.3dl` / `.look` | R/W (Export Color Lookup) | R/W / R / R | `cms/src/lutfile.rs` |
| Keyboard shortcuts `.kys` | R/W | R | `engine/src/kys.rs` |
| ICC profiles | R | R | Own CMS |
| Actions `.atn` | R/W | no | AUTO-219-2 |
| Styles `.asl` | R/W | no | #1693 |
| Contours `.shc`, custom shapes `.csh`, tool presets `.tpl` | R/W | no | FILE-215-15 |
| Curves `.acv`, Levels `.alv`, other adjustment presets | R/W | not audited | |
| C2PA Content Credentials | R/W | no | #1050 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First version: Photoshop 2026 27.11.0's formats from its bundle, against our codecs; Affinity notes moved from `roadmap.md` |
