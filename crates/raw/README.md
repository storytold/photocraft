# photocraft-raw

A clean-room, pure-Rust camera raw decoder and developer. The crate is standalone (no workspace
dependencies; its decoding dependencies are `flate2` for Deflate DNG and `jxl-oxide`, a pure-Rust
JPEG XL decoder, for DNG 1.7), has no `unsafe`, does no I/O (`&[u8]` in), builds for `wasm32-unknown-unknown`
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

## Sources (clean-room)

Implemented only from public specifications, papers and observation of files:

* TIFF 6.0, TIFF/EP (ISO 12234-2) and the Adobe DNG Specification 1.7.
* ITU-T T.81 (ISO 10918-1) Annex H: lossless JPEG, process 14 ("LJ92").
* The published description of Canon's CR2 container (header, raw IFD, slice tag 0xC640).
* Publicly documented maker-note / private tags (ExifTool's tag tables): Canon ModelID (0x0010),
  SensorInfo (0x00E0) and ColorData (0x4001), Nikon WB_RBLevels (0x000C) and BlackLevel (0x003D), Sony
  BlackLevel (0x7310), WB_RGGBLevels (0x7313), SonyRawFileType (0x7000) and SonyToneCurve
  (0x7010), the PanasonicRaw IFD0 tags, Olympus ImageProcessing (0x2040) and CameraSettings
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
* Panasonic RW2 RawFormat 5, uncompressed Olympus ORF and uncompressed Fujifilm RAF: established
  by observation of sample files (bit packing, page layout, sample justification; for RAF the
  three sample storages and the reversed order of the X-Trans / Bayer layout records, each
  verified by developing with every candidate phase against the camera's preview).
* Demosaicing: Malvar, He & Cutler (ICASSP 2004); Hirakawa & Parks, "Adaptive
  homogeneity-directed demosaicing" (IEEE TIP 2005); for X-Trans and other non-Bayer CFAs,
  gradient-weighted green interpolation (Lu & Tan, "Color filter array demosaicking: new method
  and performance measures", IEEE TIP 2003) with edge-aware colour differences.
* McCamy's CCT approximation (1992); the Bradford chromatic adaptation transform.

No code from dcraw, LibRaw, rawspeed, rawler, rawloader or darktable was read or used, and no
camera colour tables were copied.

## Support matrix

| Format | Status |
|---|---|
| DNG | Uncompressed (8–16 bit, packed or not), Deflate (compression 8, predictors 1 and 2), lossless JPEG and JPEG XL (DNG 1.7, e.g. Samsung Expert RAW; decoded by `jxl-oxide`); strips and tiles; CFA (Bayer) and LinearRaw; LinearizationTable, BlackLevel (+ repeat, DeltaH/V), WhiteLevel, ActiveArea, DefaultCrop, ColorMatrix1/2, CameraCalibration, ForwardMatrix, AnalogBalance, AsShotNeutral / AsShotWhiteXY, BaselineExposure, Orientation, OpcodeList2 GainMap (lens shading) |
| DNG (lossy JPEG, floating point; opcodes other than GainMap; ProfileGainTableMap) | Unsupported / not applied (reported) |
| CR2 | Lossless JPEG with slices, borders and as-shot white balance from the maker note, black measured on the masked border. CR2 has no CFA tag and the row phase varies by model, so it is measured from the data (the green diagonal), with a Canon model-ID table as the fallback (see `src/cr2.rs`) |
| CR2 sRAW / mRAW | Unsupported |
| NEF / NRW, ARW, PEF and other TIFF/EP raws | Uncompressed and lossless-JPEG (incl. Sony lossless ARW) CFA data |
| Nikon compressed NEF (compression 34713): lossless and lossy type 1 / 2, 12 and 14 bit | Decoded: fixed Huffman tables + maker-note `0x0096` seeds and curve (see `src/nefc.rs`). BlackLevel `0x003d` is read in 14-bit units. "Lossy after split" files are unsupported (preview fallback) |
| Sony compressed ARW ("cRAW", SonyRawFileType 2) | Decoded: 11-bit min/max + 7-bit delta blocks, SonyToneCurve to 14 bits, the curve's 512 black level when no BlackLevel tag is written (the first-generation bodies, e.g. the ILCE-7) |
| Panasonic / Leica RW2, RawFormat 5 (12- and 14-bit packed) | Decoded, with PanasonicRaw black / white / WB / sensor borders |
| Olympus ORF, uncompressed 16-bit (E-1, E-400…) | Decoded, with ImageProcessing black / WB / ValidBits / crop |
| Nikon "lossy after split" NEF, Sony "Compressed RAW 2", Pentax compressed PEF, RW2 RawFormat 4 and older, Olympus compressed ORF | Unsupported: no public description of these codes was found apart from GPL decoder source, which this crate may not use (clean-room). `photocraft-io` opens the embedded JPEG preview instead |
| Fujifilm RAF, uncompressed (X-Trans I–V, GFX and Bayer bodies since about 2010) | Decoded: 16-bit containers (either byte order), 12-bit LSB-first packing, 14-bit packing in 32-bit words; X-Trans 6×6 or Bayer 2×2 layout, black, as-shot WB, crop, RawExposureBias, orientation from the preview's EXIF |
| Fujifilm compressed RAF, early FinePix / SuperCCD RAF (no CFA TIFF) | Unsupported (no public description of the compression); preview fallback |
| CR3 | Recognised, unsupported (preview fallback where a preview is found) |
| X-Trans and other periodic non-Bayer CFAs (up to 16×16) | Demosaiced by one pattern-agnostic edge-directed method, whatever `Demosaic` is chosen |

## Development pipeline

1. Linearization table, per-position black level, scale to the white level.
2. White balance (as shot; or grey-world when the file has none; or explicit multipliers),
   normalized so the smallest multiplier is 1, then clip to 1 so blown highlights stay white.
3. Demosaic: `Bilinear`, `Mhc` (Malvar–He–Cutler) or `Ahd` (default) for Bayer; non-Bayer CFAs
   (X-Trans) always use the edge-directed generic method (`demosaic_cfa`).
4. Camera → XYZ (D50) per the DNG specification (ColorMatrix interpolated by the white's
   correlated colour temperature, or ForwardMatrix), → linear ProPhoto; exposure
   (BaselineExposure + user EV); gamma 1.8; 16 bits.
5. Orientation.

Files without colour calibration (CR2, NEF, ARW, RW2, ORF, RAF…) use a documented neutral fallback: the
white-balanced camera channels are treated as linear sRGB primaries (colours are plausible but
less saturated than a calibrated profile; converting to DNG gives calibrated colour). No tone
curve is applied: the result is a scene-referred rendering, flatter than a camera JPEG.

`CameraProfile::from_dcp` and `develop_sensor_profile` apply an external, model-matched
profile's calibration matrices, HSV/Look tables and tone curve. The native `photocraft-io`
importer includes 55 explicitly licensed independent calibrations and accepts optional user DCPs.
Adobe directories are not scanned; ProfileEmbedPolicy is enforced for non-DNG inputs. No JPEG
fitting is used. See [camera profiles](../../docs/camera-profiles.md) for selection
and limits. `camera_settings` reads Nikon Picture Control metadata without turning Auto/n/a
codes into invented slider values. `rawinfo --profile /path/to/profile.dcp` tests an explicit profile.

## Tools

`cargo run --release -p photocraft-raw --example rawinfo -- [--dump] [--demosaic ahd] [--png DIR] FILE...`
prints what was decoded, times decode and develop, and can write sRGB PNG previews and the
embedded JPEG previews.
