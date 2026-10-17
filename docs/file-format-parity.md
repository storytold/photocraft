# File-format parity

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-15 · **Change:** minor (PSD smart filters: Smart Sharpen and Dust & Scratches mapped both ways, #6300; earlier: PDF write one flattened page via Save As / Save a Copy / CLI, #3401; CMYK Invert, Posterize and Threshold on the inks; smart filter masks on PSD open and save; Photoshop-oracle floor 188/258; lossy DNG, compression 34892, decodes with its levels and LinearizationTable applied to the stored 8-bit samples, FILE-215-11; CMYK Hue/Saturation and Color Balance keep the black ink; CMYK Posterize on the stored values, psd-tools oracle floor 242/309; CMYK adjustment layers composite on the document's inks; Photoshop-oracle floor 165/258; CMYK Brightness/Contrast on the inks, CMYK Threshold's luma; modern Brightness/Contrast's brightness curve; Photoshop-oracle floor 151/258; Lab documents composite on their stored L, a, b; Photoshop-oracle floor 152/258; Levels on whole levels at every step with Photoshop's shadow toe; Photoshop-oracle floor 146/258, psd-tools 238/309; 32-bit adjustments unclipped; Photoshop-oracle floor 149/258; mask feathers blur with their exact variance; Threshold splits on Photoshop's luminosity; a pass-through clipping base with an adjustment renders isolated; psd-tools oracle floor 240/309; 16-bit Posterize as Photoshop bins it; psd-tools oracle floor 241/309; Gradient Map luminosity; Photoshop-oracle floor 147/258; PSD: `Mt16`/`Mt32` merged transparency read; palette PNGs open in Indexed Color mode, #4804; earlier: CMYK Invert, Posterize and Threshold on the inks; 16-bit Lab Invert; Photoshop-oracle floor 145/258; `.atn` import maps 112 events; DDS textures open, #1327; DPX read and write, #1049; #2465 fixed: CS2 placed layers with only a `PlLd` block open their contents; Photoshop Actions `.atn` read and write; `.shc` contour files read and write; earlier the same day: indexed BMP palette mode; Save keeps a JPEG's quality and a WebP's encoding) · **Target:** Adobe Photoshop 2026

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
| PSD | R/W | yes | yes | 1/8/16/32-bit; Bitmap, Gray, Indexed, RGB, CMYK, Multichannel, Lab, Duotone; layers, groups, masks, vector masks, adjustment and fill layers, effects, text, shapes, smart objects (SoLd/lnk2; CS2 PlLd/lnkD read with their warp, kept as is or re-saved as SoLd once edited), smart filters (16 mapped, incl. Smart Sharpen and Dust & Scratches, plus Filter Gallery with Chrome, rest verbatim), slices, comps, Advanced Blending. Oracle floors: Photoshop-authored 188/258, io 148/170, psd-tools 249/309; round trip ≥ 169/170, 308/309, 258/256. Known failures: #1281, #2469 (#2374, text layers, closed 2026-10-10; #2465, CS2 `PlLd`-only smart objects, fixed 2026-10-12) |
| PSB | R/W | yes | yes | Auto-switches above 2 GB (REL-212-2); 4–5 GB fixtures generated (FILE-216-2); > 4 GB fails on Windows (#375) |
| Cloud PSD (.psdc) | R/W | no | no | Out of scope (Adobe cloud) |
| TIFF | R/W | yes | yes | 8/16/32f, Gray/RGB/CMYK, BigTIFF, every IFD, banded parallel decode; layered TIFF both byte orders (FILE-215-7: extra alpha channels not written) |
| JPEG | R/W | yes | yes | 8-bit Gray/RGB/CMYK, ICC/EXIF/XMP; Save keeps the opened file's quality (estimated from its quantisation tables, #2399); no JPEG Options dialog (quality 0–12, baseline/progressive) (FILE-215-19) |
| PNG | R/W | yes | yes | 8/16-bit, ICC/text; APNG first frame only; alpha channels on open (#2225); palette PNGs open in Indexed Color mode with their transparent entry (#4804) |
| GIF | R/W | yes | yes | File › Open keeps the first frame; New Video Layer from File and Video Frames to Layers take every frame, with the timeline rate from the delays (#572). Per-frame delays and keeping the frames in `.pcraft` are not done |
| BMP | R/W | yes | yes | 8-bit; palette BMPs open in Indexed Color mode |
| WebP | R/W | yes | yes | Lossless and our own lossy VP8 encoder; ICC/EXIF/XMP; Save keeps an opened file lossy (at its quality) or lossless (#2399) |
| AVIF | R/W | no (#463) | optional feature, off by default | |
| JPEG XL | R/W | inside DNG only | no | |
| HEIF/HEIC | R/W(HEIF) | yes (pure-Rust heic-rs, `heif` feature, on in releases) | no | 8/16-bit |
| OpenEXR | R/W | yes | yes | f16/f32; deep EXR and Cryptomatte readers (`corpus/exr`) |
| Radiance HDR | R/W | yes | yes | f32 |
| Portable Bit Map (PBM/PGM/PPM/PNM/PFM/PAM) | R/W | yes | yes | 8/16/32f |
| FITS | R/W (FITS Liberator plug-in) | yes | yes | 8/16/32f gray or RGB, header cards kept; no fpack, no debayer |
| Targa | R/W | yes | yes | 8-bit |
| Photoshop PDF | R/W | no (#2541) | yes, one flattened page (Save As / Save a Copy / CLI, #3401; also print and artboards-to-PDF) | No vector/text PDF, no multi-page save, no PDF presentation, layers are not preserved (FILE-215-10); read still missing (#2541) |
| Photoshop EPS, DCS 1.0/2.0, EPS TIFF/PICT preview | R/W | no | no | |
| Generic EPS / AI | R (rasterize) | no | no | |
| SVG | R (rasterize) | yes, as editable shape layers or a vector smart object | no | Better than Photoshop for editing; gradients approximated (FILE-215-9) |
| JPEG 2000 | R/W | no (#1047) | no | |
| Cineon / DPX | R (DPX), R/W (CIN) | DPX: luma, RGB, RGBA at 8/10/12/16 bits, both byte orders (#1049); Cineon: 10-bit gray/RGB, read-only | DPX: 8/10/12/16 bits (`dpxBits`, `--dpx-bits`); Cineon no | Writes DPX, which Photoshop only reads; header fields (creator, project, copyright, transfer, frame rate, time code…) round-trip; Cineon read-only, no RLE |
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
| DNG (incl. LJPEG, lossy JPEG and JPEG XL tiles) | yes | 2026-10-13: lossy DNG (DNG 1.4 compression 34892: 8-bit baseline JPEG tiles or strips, CFA and LinearRaw, partial edge tiles) decodes to the stored samples, which go through LinearizationTable / BlackLevel / WhiteLevel (default 255) like every other depth; `crates/raw/tests/dng.rs` `lossy_jpeg_*` (FILE-215-11, lossy DNG part; no lossy file in the raw corpus yet) |
| Canon CR2 | yes | |
| Canon CR3 | detected, unsupported | `crates/raw/src/lib.rs:219` |
| Nikon NEF/NRW (lossless, lossy) | yes; "lossy after split" no | #50; D5100 and D90 image areas measured (14-bit NEFs); [validation](nikon-nef-development.md) |
| Sony ARW/SR2/SRF (incl. compressed) | yes | |
| Panasonic RW2, Pentax PEF | yes | |
| Olympus ORF | uncompressed only | |
| Fujifilm RAF (Bayer and X-Trans) | uncompressed only; no Fuji colour calibration | |
| Others (CRW, 3FR, IIQ, X3F, SRW, MRW, ERF…) | embedded preview only | |
| .xmp sidecars, highlight modes, raw preferences | missing | FILE-215-11 |
| Camera Raw as a PSD smart filter | partial | 2026-10-07: WB, Light/Presence, curves, HSL, grading, detail, grain, vignette mapped (FILE-215-12) |

2026-10-11: on #2135's embedded-JPEG rendering work, open-time Camera Raw tuning
preserves the fitted rendering while changing Exposure, Temperature and Tint. Default
tuned import matches normal import exactly; output remains full-resolution 16-bit ProPhoto.
This improves appearance when a usable preview is present, without adding a measured
D5100 colour profile. Counts and the G14 estimate are unchanged.

## Formats Photoshop doesn't have (ours only)

| Format | Read | Write | Notes |
|---|---|---|---|
| `.pcraft` (native) | yes | yes | Incremental, autosave, crash recovery, atomic save |
| Affinity `.af`, `.afphoto`, `.afdesign`, `.afpub` | yes | no | See below |
| Paint.NET `.pdn` | yes | no | [pdn.md](pdn.md) |
| OpenRaster `.ora` | yes | yes | [ora.md](ora.md) |
| ICO, QOI | yes | yes | |
| DirectDraw Surface `.dds` | yes | no | Top mip level of a 2D texture: uncompressed RGB, RGBA or luminance, BC1–BC3 (DXT1/3/5), legacy or DX10 header. Cubemaps, arrays, volumes and BC4–BC7 are refused (#1327). Photoshop needs a plug-in |
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
| Keyboard shortcuts `.kys` | R/W | R/W | `engine/src/kys.rs` (`write`: Edit › Keyboard Shortcuts › Export…, rows carry our id; Photoshop reading our files is unverified) |
| ICC profiles | R | R | Own CMS |
| Actions `.atn` | R/W | R/W | `psd/src/atn.rs` (version 16 sets, exact round trip), `engine/src/atn_cmds.rs` (112 Photoshop events ↔ commands; other steps kept verbatim). Panel menu, CLI `batch --actions set.atn`, `actions.load`/`actions.save`. Photoshop 5-era version 12 files unsupported (AUTO-219-2) |
| Styles `.asl` | R/W | no | #1693 |
| Contours `.shc` | R/W | R/W | `psd/src/shc.rs`; Contour Editor Load…/Save… (FILE-215-15). No user contour library; not yet checked against a Photoshop-written file |
| Custom shapes `.csh`, tool presets `.tpl` | R/W | no | |
| Curves `.acv`, Levels `.alv`, other adjustment presets | R/W | not audited | |
| C2PA Content Credentials | R/W | no | #1050 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | minor | PSD smart filters: Smart Sharpen (filterID 1131, Gaussian removal; Lens/Motion Blur and legacy entries stay verbatim) and Dust & Scratches (`DstS`) open as editable filters and save in Photoshop's measured shape (#6300) |
| 2026-10-14 | minor | PSD smart filter masks: Photoshop stores a smart object's filtered pixels without its filter mask and mixes the unfiltered ones (`FEid`) back in; import now mixes them and export writes them that way, so sf-filter-mask-hard, -gradient and sf-stack-blend-and-mask pass; Photoshop-oracle floor 185 → 188 of 258 |
| 2026-10-13 | minor | Lossy DNG (compression 34892) decodes: 8-bit baseline JPEG tiles and strips, CFA and LinearRaw, partial edge tiles; the stored values take the normal levels and LinearizationTable (FILE-215-11, lossy DNG part) |
| 2026-10-14 | minor | Corpus floors raised to the measured results of the CMYK, Lab, 32-bit, Levels, Threshold, Posterize, feather and pass-through fixes landed together: Photoshop-authored 165 → 185 of 258, io 146 → 148 of 170, psd-tools 242 → 249 of 309 |
| 2026-10-13 | minor | CMYK Hue/Saturation (not Colorize) and Color Balance work on C, M and Y as inverted RGB with black kept, on the inks for display, export and destructive edits; cmyk8/16 hue-saturation 100% → 0.4–0.7% bad pixels, color-balance 74% → 25% |
| 2026-10-13 | minor | CMYK Posterize works on the stored values (1 − ink), as Photoshop does: psd-tools posterize_cmyk passes (13.8 % → 0 of pixels off on the ink path); psd-tools floor → 242 of 309 |
| 2026-10-13 | minor | CMYK documents whose layers are plain raster layers plus Invert, Posterize, Threshold, Brightness/Contrast, Levels or Curves layers composite on their own inks for display, export and flattening: the cmyk8/16 oracles for those seven adjustments pass; Photoshop-oracle floor 151 → 165 of 258 |
| 2026-10-13 | minor | CMYK Threshold compares the Rec. 601 luma of the inks' plain complement, as Photoshop does (every pixel of cmyk8/16 threshold-128.psd); oracle bad pixels 1.6% → 0.2% |
| 2026-10-13 | minor | CMYK Brightness/Contrast (modern and legacy) curves each ink's stored value (1 − ink), exact on destructive edits; cmyk8/16 oracle bad pixels 71% → 48–57% (the RGB composite's separation still differs) |
| 2026-10-13 | minor | Modern Brightness/Contrast: the brightness curve refit to Photoshop, so the rgb and gray 8/16/32 brightness-contrast oracles pass; Photoshop-oracle floor 145 → 151 of 258 |
| 2026-10-13 | minor | Lab documents of plain layers with Invert, Posterize, Threshold, Brightness/Contrast or Lab Levels/Curves adjustment layers composite on the stored L, a, b instead of clipped display RGB, and flattening keeps them: lab16 invert, posterize-4, levels, curves and legacy Brightness/Contrast and lab8 invert and posterize-4 pass; Photoshop-oracle floor 145 → 152 of 258 |
| 2026-10-13 | minor | Levels round the gamma and output steps to whole levels too (8-bit), and the gamma above 1 starts along a toe of slope 2^gamma that joins the power curve: psd-tools levels_rgb and rgb8 stack-levels-curves-hue-posterize pass, rgb8/gray8 levels.psd exact; Photoshop-oracle floor 145 → 146 of 258, psd-tools 236 → 238 of 309 |
| 2026-10-13 | minor | 32-bit documents: legacy Brightness/Contrast, Channel Mixer, Photo Filter and the Black & White tint keep values above 1 and below 0 as Photoshop does (rgb32/gray32 brightness-contrast-legacy, rgb32 channel-mixer and black-white-tint pass); Photoshop-oracle floor 145 → 149 of 258 |
| 2026-10-13 | minor | Mask feathers blur with exactly the feather's variance (three extended boxes; whole-pixel boxes rounded 6 px to 6.48): psd-tools mask-parameters-no-real-channel passes, layer_mask_data 27 % → 4 % off; psd-tools floor → 240 of 309 (main 239) |
| 2026-10-13 | minor | Threshold splits on the luminosity 0.30 R + 0.59 G + 0.11 B of whole samples, as Photoshop does (8-bit: rounded to a level, halves down; 16-bit: against the level in 16 bits), not the Rec. 601 grey: psd-tools threshold_rgb and threshold_16bits_rgb pass; psd-tools floor → 241 of 309 (main 239) |
| 2026-10-13 | minor | A pass-through group that is the base of a clipping group and holds an adjustment layer renders isolated, as Photoshop does (its blend modes still reach the backdrop): psd-tools passthrough_clipping_mask_adjustment passes (was 50 % of pixels off); psd-tools floor → 240 of 309 (main 239) |
| 2026-10-13 | minor | 16-bit Posterize bins the 16-bit sample and lands on 8-bit levels through a 16-bit step, as Photoshop does (RGB, Grayscale and the CMYK/Lab channel path): psd-tools posterize_16bits_rgb and posterize_16bits_grayscale pass; psd-tools floor → 241 of 309 (main 239) |
| 2026-10-13 | minor | Gradient Map adjustment layers are driven by the W3C luminosity (0.3, 0.59, 0.11), not the Rec. 601 grey: rgb8 and rgb16 gradient-map.psd pass; Photoshop-oracle floor 145 → 147 of 258 |
| 2026-10-13 | minor | PSD: a "Saving Merged Transparency" global block (`Mtrn`, `Mt16`, `Mt32`; Photoshop's 32-bit documents keep a positive layer count) marks the merged image's first extra channel as its transparency: opening such a file no longer adds a "Transparency" alpha channel, and the oracle compares psd-tools transparentbg.psd's alpha (83.7 % → 0.9 % of pixels off; its shape edges remain) |
| 2026-10-13 | minor | Palette PNGs open in Indexed Color mode (#4804) |
| 2026-10-12 | minor | `.atn`: 112 events mapped (Select and Mask, Filter Gallery Spatter, Ripple, Crystallize, Pattern Fill layers, mask targeting, Step Backward, points, Bitmap) |
| 2026-10-12 | minor | DDS textures open, read-only (#1327) |
| 2026-10-12 | minor | #2465 fixed: Photoshop CS2 placed layers (`PlLd` only, file in `lnkD`) open as editable smart objects and save their edits |
| 2026-10-12 | minor | CMYK Invert, Posterize and Threshold work on the inks and 16-bit Lab Invert mirrors a and b around neutral (oracle bad pixels: CMYK 71–77% → 9–10%, 37% → 1.6%; lab16 invert 52% → 1.2%); Photoshop-oracle floor 133 → 145 of 258 |
| 2026-10-12 | minor | DPX read and write (#1049); Cineon still missing |
| 2026-10-12 | minor | Photoshop Actions `.atn`: read and write, 90 events mapped, the rest kept verbatim (AUTO-219-2) |
| 2026-10-12 | minor | Presets table: Contours `.shc` read and write through the Contour Editor (FILE-215-15); `.csh` and `.tpl` stay missing |
| 2026-10-12 | minor | Palette BMPs open in Indexed Color mode (#3802) |
| 2026-10-12 | minor | Save writes a JPEG at its own estimated quality and a WebP lossy or lossless as opened (#2399) |
| 2026-10-11 | minor | PSD smart filters: Filter Gallery (`GEfc`) maps to `filter.filterGallery` when every effect is modelled (Chrome so far), editable from the Layers panel |
| 2026-10-11 | minor | D5100 image area and fitted-look preservation in Camera Raw tuning |
| 2026-10-11 | minor | PDF write now covers Save As / Save a Copy / CLI (`io::pdf::export`, #3401): one flattened RGB/Gray/CMYK page at the document's print size with the embedded profile. Read (#2541), multi-page save, PDF presentation, vector/text output still open
| 2026-10-10 | major | First version: Photoshop 2026 27.11.0's formats from its bundle, against our codecs; Affinity notes moved from `roadmap.md` |
