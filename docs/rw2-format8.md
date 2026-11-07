# Panasonic RW2 RawFormat 8

RW2 is a container with several sensor encodings. RawFormat 8 (IFD0 tag `0x002d`)
previously returned `Unsupported`, so opening one of these files used its embedded
JPEG instead of the sensor data. The native decoder now reads its prefix-coded
strips and tiles, including the multi-row layout used by high-resolution modes.
The existing RAW developer then supplies the normal 16-bit RGB document.

## Format evidence

This is a PhotoCraft implementation based on the format measurements published by
the sibling LightCraft project, whose source is MIT OR Apache-2.0:

- [LightCraft #215](https://github.com/storytold/lightcraft/pull/215): the initial
  format description and 178-file Panasonic/Leica validation, including format-8
  GH6, GH7, G9 II, S5 II/IIX, S9, S1R II and S1 II files.
- [LightCraft #255](https://github.com/storytold/lightcraft/pull/255): cells are
  coded top-left, bottom-left, top-right, bottom-right. Reading them in row order
  exchanges the two green sites and produces coloured edges.
- [LightCraft #1304](https://github.com/storytold/lightcraft/pull/1304): tag `0x0045`
  contains `(x, y)` pairs, and `0x0043` counts tile rows. High-resolution S1R II
  files use two rows of four tiles.
- [Pinned format notes and reference implementation](https://github.com/storytold/lightcraft/blob/12c096669ab76556dcbaf6dbeb8704a3b96f646d/crates/raw/src/vendor/rw2.rs).

These are LightCraft's coverage claims; they are not a claim that this PhotoCraft
implementation has been run on every one of those cameras. No source from dcraw,
LibRaw, RawSpeed, rawler or other third-party RAW decoders was used. LibRaw was
used only as an independent executable oracle for the local sample comparison.

## Layout and bounds

- Samples are 12, 14 or 16 bit. Tag `0x0040` supplies 17 `(length, code)` pairs
  for T.81 signed-difference categories. Prefixes are at most 12 bits; shorter
  prefixes win over unreachable longer entries in real camera tables.
- Bytes are consumed least-significant bit first. Four predictors follow each
  2×2 cell; a row pair starts from the first cell of the preceding row pair.
- Tags `0x0044`–`0x0048` contain count-prefixed arrays of offsets, `(x, y)`
  origins, bit lengths, widths and heights. Wide values are low/high u16 pairs.
- At most 64 tiles are accepted. Each must be in the file and sensor bounds;
  their non-overlapping areas must cover the sensor exactly once. Bitstream
  reads honor the stated bit length even within a final byte.
- Unknown values of the measured fixed fields are `Unsupported`; malformed
  tables, layouts and truncated streams are errors. Sample predictors cannot
  wrap around. Tile buffers are decoded in parallel on native builds and
  sequentially on WebAssembly; the tile buffers and assembled plane are both
  charged against `Limits` before allocation.
- Format 8 uses the recorded black levels without the format-3/4 offset of 15.
  Existing white balance, sensor-border crop, CFA, orientation and development
  behavior are retained. This does not add lens correction or camera calibration.

## Verification

`crates/raw/tests/it/rw2_format8.rs` uses generated files, with no photographs:

- Exact sensor samples at 12/14/16 bit, single strips, multiple strips and 4×2 tiles.
- An independently hand-coded cell that detects exchanged green sites.
- The camera's 12-bit prefix table, including shadowed codes, and partial cells.
- Black levels, white balance, crop and full-resolution 16-bit development.
- Overlaps, holes, invalid dimensions/counts, unknown fields, duplicate codes,
  allocation limits and every truncation of a small fixture.
- 1,000 deterministic mutations under small allocation limits.

The original implementation fails the new successful-decode tests with
`Panasonic RW2 raw format 8 ... is not decoded yet`.

This was reproduced against `f68e227026701cf24e0e182530694240e2d3cfe4`:
an isolated copy of the RAW crate with that revision's `rw2.rs` and the new
`strips_and_tiles_decode_exactly_at_all_supported_depths` test fails with
`Unsupported("Panasonic RW2 raw format 8 (vendor-compressed) is not decoded yet")`.
The isolated crate uses the same registry dependency versions and checksums as
the repository's lockfile.

A locally supplied DC-S9 12-bit file was also checked without distributing it:
all **24,160,256 sensor samples** (6016×4016) match LibRaw 0.22.2 through rawpy
0.28.0 exactly, with zero differing samples. Its (8,8) 6000×4000 crop develops
to 4000×6000 after orientation. On 2026-11-07, a default release build on an
Apple M1 (8 cores, 16 GB) measured a median 85 ms for decode and 207 ms for AHD
development over five runs after two warm-ups (decode 85–87 ms, develop
204–216 ms). These phase timings exclude disk reads and camera-JPEG look
fitting. The baseline decoder does not support this encoding, so this is not
a before/after speedup claim or a cross-camera performance guarantee.

Reproduce the synthetic checks with:

```sh
cargo test -p photocraft-raw --lib --tests --locked
cargo clippy -p photocraft-raw --all-targets --locked -- -D warnings
cargo check -p photocraft-raw --target wasm32-unknown-unknown --locked
cargo run --release -p photocraft-raw --example rawinfo -- /path/to/photo.RW2
```
