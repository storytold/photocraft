# photocraft-raw

A pure-Rust camera raw decoder and developer. Packed-ORF provenance remains under review
(see [the source audit below](#packed-orf-provenance-review-2026-11-06)). The crate is standalone (no workspace
dependencies; its decoding dependencies are `flate2` for Deflate DNG and `jxl-oxide`, a pure-Rust
JPEG XL decoder, for DNG 1.7, native builds only), has no `unsafe`, does no I/O (`&[u8]` in), builds for `wasm32-unknown-unknown`
(sequential there, rayon-parallel on native) and never panics on hostile input: every offset is
bounds-checked and sizes are checked against `Limits` before allocating.

```rust
use photocraft_raw::{develop, DevelopOptions, Demosaic};

let dev = develop(&bytes, &DevelopOptions { demosaic: Demosaic::Ahd, ..Default::default() })?;
// dev.rgb: interleaved 16-bit RGB in ProPhoto RGB (ROMM primaries, D50, gamma 1.8)
// dev.warnings: anything approximated or not applied
```

`photocraft-io` uses it so opening a raw file yields a normal 16-bit RGB document tagged with the
built-in ProPhoto-compatible profile.

## Sources and provenance

The project requires implementation from public specifications and observation, without copying
incompatible decoder code. The source notes below describe the recorded inputs; they do not
resolve the packed-ORF provenance questions documented below:

* TIFF 6.0, TIFF/EP (ISO 12234-2) and the Adobe DNG Specification 1.7.
* ITU-T T.81 (ISO 10918-1) Annex H: lossless JPEG, process 14 ("LJ92").
* The published description of Canon's CR2 container (header, raw IFD, slice tag 0xC640).
* Publicly documented maker-note / private tags (ExifTool's tag tables): Canon ModelID (0x0010),
  SensorInfo (0x00E0) and ColorData (0x4001), Nikon WB_RBLevels (0x000C), ColorSpace (0x001E) and BlackLevel (0x003D), Sony
  BlackLevel (0x7310), WB_RGGBLevels (0x7313), SonyRawFileType (0x7000) and SonyToneCurve
  (0x7010), SR2Private Offset / Length / Key (0x7200 / 0x7201 / 0x7221), the PanasonicRaw IFD0 tags, Olympus ImageProcessing (0x2040) and CameraSettings
  (0x2020) preview tags, the Fujifilm RAF header and metadata records (RawImageCropTopLeft /
  RawImageCroppedSize, FujiLayout, XTransLayout, WB_GRGBLevels, RawExposureBias) and FujiIFD
  (RawImageFullWidth / Height, BitsPerSample, StripOffsets / ByteCounts, BlackLevel,
  WB_GRBLevels).
* Nikon compressed NEF: Laurent Clévy's NEF structure notes and Bill Claff's "NEF Compression"
  (prose: the `0x0096` table layout, a lossy curve followed by a fixed-table Huffman stage),
  ExifTool's Nikon tag-name documentation and ITU-T T.81 Annex F / H. The Huffman tables are not
  stored in the files; they were recovered by black-box analysis of CC0 raw.pixls.us samples (the
  same scenes shot compressed and uncompressed) for LightCraft (storytold/lightcraft#86) and
  verified there on ~45 files from ~35 bodies.
* Sony cRAW (ARW 2): H. Dietz, "Sony ARW2 Compression: Artifacts And Credible Repair"
  (Electronic Imaging 2016) and the RawDigger / diglloyd write-ups of the 11 + 7-bit scheme;
  the exact bit layout and tone-curve scale were established by observation of sample files.
  Encrypted SR2SubIFD metadata with key bytes `11 22 33 44` was measured independently against
  ExifTool's metadata dumps; see [Sony metadata evidence](../../docs/sony-arw-metadata.md).
* Panasonic RW2 RawFormat 5, uncompressed 16-bit Olympus ORF and uncompressed Fujifilm RAF: established
  by observation of sample files (bit packing, page layout, sample justification; for RAF the
  three sample storages and the reversed order of the X-Trans / Bayer layout records, each
  verified by developing with every candidate phase against the camera's preview).
* Panasonic RW2 RawFormat 3/4 (vendor-compressed): the fixed-rate 16 bytes per 14 pixels, the
  predictor groups with per-parity predictors, the 2/4/8-bit field widths, the page-permuted
  downward bit order and the black level sitting 15 counts above the recorded tags were all
  established by observation of sample files.
* Padded uncompressed Olympus ORF: attributed to the prose in
  [LightCraft #651](https://github.com/storytold/lightcraft/pull/651), contributed by radialmonster
  with earlier padded-layout work by voshart in
  [LightCraft #360](https://github.com/storytold/lightcraft/pull/360). The layout is ten LSB-first
  12-bit samples in 15 bytes followed by one padding byte, with whole blocks per row. The original
  contribution records checks of CC0 E-300 / E-500 / E-330 and E-M5 II / PEN-F high-resolution files.
  These are attributed historical claims, not an independently verified clean-room certification.
* Packed uncompressed Olympus ORF: attributed to the prose in
  [LightCraft #600](https://github.com/storytold/lightcraft/pull/600), contributed by radialmonster.
  Whole rows of MSB-first 12-bit samples form two fields separated by one gap; the strip table
  omits the final row beyond the declared image. The file's EXIF CFA and maker-note red/blue
  balance entries are used. The original contribution separately records checking the XZ-2 /
  XZ-10 / SH-2 single-strip layout, with MSB-first 12-bit samples in little-endian 32-bit words.
  The historical measurement records and source-exposure questions remain unresolved below.
* Demosaicing: Malvar, He & Cutler (ICASSP 2004); Hirakawa & Parks, "Adaptive
  homogeneity-directed demosaicing" (IEEE TIP 2005); for X-Trans and other non-Bayer CFAs,
  gradient-weighted green interpolation (Lu & Tan, "Color filter array demosaicking: new method
  and performance measures", IEEE TIP 2003) with edge-aware colour differences.
* McCamy's CCT approximation (1992); the Bradford chromatic adaptation transform.
* Per-file colour from the camera's embedded JPEG (`fit_look`, `src/look.rs`): the method follows
  the prose description in LightCraft's `docs/camera-preview-colour.md` (storytold/lightcraft#165,
  Apache-2.0, same organisation); no LightCraft code was used. Pool-adjacent-violators isotonic
  regression (Ayer et al., 1955), Tukey's biweight (Beaton & Tukey, 1974), ridge regression
  (Hoerl & Kennard, 1970) and the CIE 1976 L\*a\*b\* colour difference.

The implementation policy excludes copying incompatible decoder code or camera colour tables.
The packed-ORF review below qualifies the earlier blanket statement about source exposure.

### Packed ORF provenance review (2026-11-06)

This review separates what the public records say from what can be independently established.
It does not certify clean-room origin or determine whether a contribution is legally derived.

* **LightCraft #600:** its author reports black-box analysis of six CC0 files. The
  [published commit](https://github.com/radialmonster/lightcraft/commit/37d36215056838bc705900984a5f503961a1b14d)
  changes two Rust files and does not include the referenced investigation log. The report of
  independent measurement is an author claim; the underlying chronology cannot be checked from
  that patch alone.
* **LightCraft #651:** its author discloses that an analyst model recalled the broad packing
  pattern before the reported comparison of 384 alternatives. The
  [maintainer discussion](https://github.com/storytold/lightcraft/pull/651#issuecomment-6094472505)
  and [follow-up commit](https://github.com/radialmonster/lightcraft/commit/82566817e6b16bd82bcdd3a60551b516394f5faf)
  explicitly remove references to missing analysis files. The named
  `data/testing/retry-olympus-e300/log.md` and `tools/retry-olympus-e300/` paths are absent from
  both the [initial public tree](https://github.com/radialmonster/lightcraft/tree/44b126274d3a4e0e8237de3eec67a8ad1064a5f4)
  and the final public tree at `82566817e6b16bd82bcdd3a60551b516394f5faf`.
  The model's recollection and subsequent correctness checks establish neither copying nor
  absence of prior decoder knowledge.
* **Earlier padded-layout work:** the pinned
  [format record](https://github.com/voshart/Rust-Olympus-RAW-decoder/blob/9d8c927b8f181c6eea4358db30b3a71641922aaf/research/orf12-format.md)
  claims that packing hypotheses came from byte measurements before comparison with reference
  pixels. The accompanying
  [provenance record](https://github.com/voshart/Rust-Olympus-RAW-decoder/blob/9d8c927b8f181c6eea4358db30b3a71641922aaf/research/orf12-provenance.md)
  identifies later comparisons with a rawpy 0.27.1 / LibRaw 0.22.1 binary. It also discloses later
  exposure to RawSpeed/LibRaw-attributed prose during compressed-decoder research. That disclosure
  remains relevant to the broader project, but is not evidence that the earlier padded-layout
  hypothesis was source-derived. This chronology is the author's recorded account.
* **This PhotoCraft contribution:** its
  [original README](https://github.com/harrisonoconnorhover/photocraft/blob/8a8cb43035641e3ed86466b168d2c77943caedb7/crates/raw/README.md)
  records independent implementation, direct CC0 measurements, and no LightCraft implementation
  being read or copied. The original local measurement and implementation records could not be
  recovered for this review, so those historical statements cannot be newly certified. The
  present audit inspected public source records and diffs; that audit-time exposure must not be
  confused with evidence about the earlier implementation process.

The scope still excludes the compressed `olympus_load_raw` predictor. Its absence is not proof
of independent origin: related packed-storage operations also occur in the generic
`packed_load_raw` routines in
[dcraw (unofficial mirror, pinned)](https://github.com/ncruces/dcraw/blob/8fe4d3825595816ea7d6880d9253f9edd143cacb/dcraw.c#L1921-L1957)
and [LibRaw 0.21.4 (pinned)](https://github.com/LibRaw/LibRaw/blob/9646d776c7c61976080a8f2be67928df0750493e/src/decoders/generic.cpp#L40-L85).
These include word-oriented input, field reordering and per-block padding checks; the routines
are not asserted to be identical. This audit inspected these external readers read-only; no
implementation was copied from those readers during this review. Similar bit operations, numerical agreement,
maintainer invitations and prior merges do not independently establish or disprove derivation.
The rebase and current tests check correctness, not historical provenance; the decoding algorithm
has not been replaced as part of this review.

The unresolved evidence is the original #600/#651 investigation logs and tools, their chronology
and source-exposure details, and PhotoCraft's original measurement/implementation records. Until
those can be evaluated, or maintainers decide that the accurately disclosed source chain meets
their contribution policy, the packed-ORF provenance review remains open.

## Support matrix

| Format | Status |
|---|---|
| DNG | Uncompressed (8–16 bit, packed or not), Deflate (compression 8, predictors 1 and 2), lossless JPEG, lossy JPEG (DNG 1.4 compression 34892: 8-bit baseline JPEG tiles or strips, gray for CFA, YCbCr-coded RGB for LinearRaw; decoded by `zune-jpeg`, the stored 8-bit values then go through the same LinearizationTable / BlackLevel / WhiteLevel as every other depth) and JPEG XL (DNG 1.7, e.g. Samsung Expert RAW; decoded by `jxl-oxide`; not in wasm32 builds, where it is `Unsupported`); strips and tiles; CFA (Bayer) and LinearRaw (three-sample colour, or one-sample monochrome such as Leica Monochrom and film-scanner DNGs, developed as neutral grey with no white balance or colour matrix); LinearizationTable, BlackLevel (+ repeat, DeltaH/V), WhiteLevel, ActiveArea, DefaultCrop, ColorMatrix1/2, CameraCalibration, ForwardMatrix, AnalogBalance, AsShotNeutral / AsShotWhiteXY, BaselineExposure, Orientation, floating-point JPEG XL raw data (DNG 1.7 lossy LinearRaw, as Adobe DNG Converter writes it; scaled by the highest WhiteLevel), OpcodeList2 GainMap (lens shading) and MapPolynomial |
| DNG (floating point other than JPEG XL; opcodes other than OpcodeList2 GainMap and MapPolynomial; ProfileGainTableMap) | Unsupported / not applied (reported) |
| CR2 | Lossless JPEG with slices, borders and as-shot white balance from the maker note, black measured on the masked border. CR2 has no CFA tag and the row phase varies by model, so it is measured from the data (the green diagonal), with a Canon model-ID table as the fallback (see `src/cr2.rs`) |
| CR2 sRAW / mRAW | Unsupported |
| NEF / NRW, ARW, PEF and other TIFF/EP raws | Uncompressed and lossless-JPEG (incl. Sony lossless ARW) CFA data |
| Nikon compressed NEF (compression 34713): lossless and lossy type 1 / 2, 12 and 14 bit | Decoded: fixed Huffman tables + maker-note `0x0096` seeds and curve (see `src/nefc.rs`). BlackLevel `0x003d` is read in 14-bit units. "Lossy after split" files are unsupported (preview fallback) |
| Sony compressed ARW ("cRAW", SonyRawFileType 2) | Decoded: 11-bit min/max + 7-bit delta blocks, SonyToneCurve to 14 bits; applied WB and black from plain tags or encrypted SR2SubIFD with the measured common key. If metadata is unreadable, black falls back to 512 and WB to grey-world. Measured ILCE-7 / ILCE-6000 image area; camera colour calibration remains incomplete |
| Panasonic / Leica RW2, RawFormat 5 (12- and 14-bit packed) | Decoded, with PanasonicRaw black / white / WB / sensor borders |
| Panasonic / Leica RW2, RawFormat 3 and 4 (vendor-compressed, 12-bit) | Decoded; the black level is read 15 counts above the recorded tag; PanasonicRaw white / WB / sensor borders |
| Olympus ORF, uncompressed 16-bit (E-1, E-400…) | Decoded, with ImageProcessing black / WB / ValidBits / crop |
| Olympus ORF, padded uncompressed 12-bit (E-300, E-500, E-330, E-M5 II and PEN-F high-resolution) | Decoded, including files declaring 16-bit samples; exact single-strip 12.8-bit-per-pixel storage only. Reuses the file's CFA / ImageProcessing metadata and existing uncalibrated colour fallback |
| Olympus ORF, packed uncompressed 12-bit (XZ-2, XZ-10, SH-2; two-field C-5060WZ, C-7070WZ, SP-510UZ, SP-550UZ, SP-565UZ, SP-570UZ) | Decoded from measured storage layouts and the file's CFA. Two-field storage requires the exact strip/gap layout; no crop is guessed when its origin is absent. Camera colour rendering remains uncalibrated |
| Nikon "lossy after split" NEF, Sony "Compressed RAW 2", Pentax compressed PEF, RW2 RawFormat 6 and newer, Olympus compressed ORF | Unsupported: no public description of these codes was found apart from GPL decoder source, which this crate may not use (clean-room). `photocraft-io` opens the embedded JPEG preview instead |
| Fujifilm RAF, uncompressed (X-Trans I–V, GFX and Bayer bodies since about 2010) | Decoded: 16-bit containers (either byte order), 12-bit LSB-first packing, 14-bit packing in 32-bit words; X-Trans 6×6 or Bayer 2×2 layout, black, as-shot WB, crop, RawExposureBias, orientation from the preview's EXIF |
| Fujifilm compressed RAF, early FinePix / SuperCCD RAF (no CFA TIFF) | Unsupported (no public description of the compression); preview fallback |
| CR3 | Native CRX unsupported; bounded JPEG-track/PRVW preview fallback with CMT1 orientation. I/O labels the result as an 8-bit camera preview |
| X-Trans and other periodic non-Bayer CFAs (up to 16×16) | Demosaiced by one pattern-agnostic edge-directed method, whatever `Demosaic` is chosen |

Nikon maker-note compression modes 6/10 select checked little-endian packed 12/14-bit
rows even when TIFF Compression is 34713. The modern lossy `0x44 0x40` curve uses
its smaller encoded domain while preserving sensor ADUs. Valid Nikon CropArea
coordinates take precedence over model fallbacks. HE/HE* and lossy-after-split
remain unsupported. Four pinned CC0 Z6 files cover both bit depths and both paths.

## Development pipeline

1. Linearization table, per-position black level, scale to the white level.
2. White balance (as shot; or grey-world when the file has none; or explicit multipliers),
   normalized so the smallest multiplier is 1, then clip to 1 so blown highlights stay white.
3. Demosaic: `Bilinear`, `Mhc` (Malvar–He–Cutler) or `Ahd` (default) for Bayer; non-Bayer CFAs
   (X-Trans) always use the edge-directed generic method (`demosaic_cfa`).
4. Camera → XYZ (D50) per the DNG specification (ColorMatrix interpolated by the white's
   correlated colour temperature, or ForwardMatrix), or a camera look fitted to the embedded JPEG,
   → linear ProPhoto; exposure (BaselineExposure + user EV); the look's tone curve; gamma 1.8;
   16 bits.
5. Orientation.

Development validates public sensor/crop/CFA metadata before allocation. The working-memory
limit checks peak live development buffers instead of summing disjoint phases. AHD limits
simultaneous scratch bands only when required; normal load balancing and pixels are preserved.
The default 2 GiB limit is unchanged and excludes caller-owned sensor/document storage.

Files without colour calibration (CR2, NEF, ARW, RW2, ORF, RAF…) have no camera matrix. Every one of
them carries a JPEG the camera rendered from the same sensor data, so `fit_look` estimates the
camera's rendering per file: both the sensor data (white-balanced as `develop` does) and the
decoded JPEG are reduced to a common grid of at most 256 cells on the long edge, and a 3×3
camera → ProPhoto matrix plus one monotone tone curve (applied per channel after exposure) are fitted
so that the developed raw matches the JPEG, by alternating isotonic regression for the curve and
weighted, ridge-regularised least squares for the matrix (residuals scaled to approximate L\*,
Tukey biweights against misaligned edges and local tone mapping; clipped and near-black cells left
out). One cell in three is held out: the look is used only when it shows the same picture in colour,
the matrix is plausible and the held-out mean CIE76 ΔE drops to at most 85 % of the fallback's.
Pass it as `DevelopOptions::camera_look`; `develop` then notes "colour fitted to the camera's
embedded JPEG (mean ΔE …)". The crate cannot decode JPEG itself: `photocraft-io` decodes the
smallest preview at least 512 pixels long, fits and develops in one go. A preview without an ICC
profile is read in the colour space the raw states (`embedded_preview_color_space`): the EXIF
ColorSpace (a Sony body set to Adobe RGB writes Uncalibrated with Interoperability "R03"), else a
Nikon maker note's ColorSpace (0x001E; Nikon writes no EXIF ColorSpace), sRGB when the file says
nothing.
Camera Raw's tuned import fits at the default exposure and as-shot white balance first,
then applies the user's Exposure, Temperature and Tint to that same rendering.
The D5100's measured image area is 4928 × 3264 at (8, 8) in a 4992 × 3280
sensor plane; the D90's is 4288 × 2848 at (10, 10) in a 4352 × 2868 plane; each
removes the border without changing levels, WB or calibration.
Measured image areas preserve an explicit file crop.

For a slightly tighter camera JPEG, fitting can align the reference proxy using a bounded
90–100% centred search on training cells. It requires a strong correlation improvement and
retains the held-out colour gates; the developed crop and camera calibration are unchanged.

Without a usable fit (no preview, an unrelated, monochrome or differently shaped one, or a fit that
does not clearly help) the documented neutral fallback stays: the white-balanced camera channels are
treated as linear sRGB primaries, with no tone curve (plausible but flat, greyish colours; converting
to DNG gives calibrated colour), and `develop` says so.

Measured on 8 Nikon D3200 NEFs (12-bit lossy compressed, indoor scenes) against each file's
embedded JPEG (mean CIE76 ΔE, 300-pixel grid, full develop output in sRGB): fallback 11.6–26.0,
fitted 1.0–7.5; all 8 fits were accepted. The highest remaining errors are shots where the camera
brightened shadows locally (Active D-Lighting), which one global curve cannot follow. The fit
(preview decode included) takes about 50–70 ms on a 24 MP file. It is an estimate of the camera's
look, not a measured calibration: picture styles and hue-dependent rendering are approximated.

Cameras with a profile measured against Photoshop's Camera Raw (`src/cameras.rs`: the Nikon D4) carry
a calibration, so no look is fitted for them. On 6 D4 NEFs against Photoshop 25.4's default Camera
Raw rendering (quarter size, medians over the frames) the measured profile is 0.048 stop from its
luminance with a mean CIE76 ΔE of 1.8; the look fitted to the embedded JPEG would be 0.30 stop and
ΔE 6.4, as it follows the camera's picture style instead. The open-time re-develop
(`photocraft-io`'s `import_raw_tuned`) and the tone curve its previews use (`tone_curve`) take the
same look as the default open, so changing a slider does not change the colour rendering. The D4's
saturation level was measured on 14-bit files and follows the sample depth (a 12-bit frame is the
14-bit one shifted right by two, so its level is a quarter of it): the three public 12-bit D4 files
develop within 0.02 stop of their 14-bit twin.

## Tools

`cargo run --release -p photocraft-raw --example rawinfo -- [--dump] [--demosaic ahd] [--png DIR] FILE...`
prints what was decoded, times decode and develop, and can write sRGB PNG previews and the
embedded JPEG previews.

`cargo run --release -p photocraft-io --example rawlook -- [--png DIR] FILE...` times the
camera-JPEG colour fit, reports ΔE against the embedded JPEG with the neutral fallback and with the
fit, and can write 1200-pixel sRGB PNGs of both and of the JPEG; `--synthetic` times it on a
generated 24 MP file.
