# AVIF in PhotoCraft

AVIF still-image import and export are experimental and **opt-in on native builds**.
Build the desktop app or CLI with the non-default `avif` feature:

```text
cargo build -j12 -p photocraft --features avif
cargo build -j12 -p photocraft-cli --features avif
```

`photocraft-codecs` and `photocraft-io` also expose this feature for library consumers.
Default builds recognise AVIF files but report that support is not included; AVIF is
omitted from the export format selector and native save filters. The browser build
has no AVIF decoding or encoding, even if a consumer enables the feature on wasm.
AVIF dependencies are native-target dependencies and are not compiled into wasm.

When enabled, Open, Open As, drag-and-drop, Place Embedded, Save As, Export As,
CLI conversions/batch, engine Save a Copy, control and headless MCP use the same
codec/I/O implementation. Save As uses codec defaults; Export As exposes these controls.

| Option | Values | Default |
|---|---|---|
| Colour quality | 1–100 | 90 |
| Speed | 1–10, higher is faster with larger files | 8 |
| Bit depth | Automatic, 8, 10 | Automatic: 8 for U8 documents, 10 otherwise |
| Alpha quality | 1–100, independent of colour quality | 100 |
| Transparency | Keep alpha or flatten over white | Keep |

Exports are **lossy**, including quality 100. There is no lossless export claim or checkbox:
the current Rust encoder does not implement true lossless AV1 encoding
([upstream tracking issue](https://github.com/xiph/rav1e/issues/151)).
The encoder stores full-range identity GBR with no chroma subsampling (4:4:4). U16 and float
inputs are quantized to 10 bits unless 8 bits is explicitly selected. Float values outside
0–1 are clipped, and warnings report this. Alpha is straight and independently encoded;
quality 100 minimizes alpha quantization error but high-depth alpha is still quantized.

RGB ICC profiles are embedded byte-for-byte and restored on import. The I/O layer validates
ICC colour models, converts grayscale and CMYK through the CMS to sRGB, and uses the existing
colour-managed compositor for Lab documents. Disabling ICC embedding in the codec export
options converts document colours to sRGB before omitting the profile. AVIF without ICC uses
CICP metadata: sRGB, linear sRGB, Display P3 with sRGB transfer, and Rec.2020 with BT.709-family
transfer are tagged using CMS profiles. Unspecified primaries/transfer use the documented
sRGB fallback. PQ/HLG and other unsupported colour encodings require a supported ICC profile
or produce an actionable error; they are never silently displayed as sRGB.

The standalone raw codec requires an RGB ICC profile when encoding non-sRGB CICP
pixels; it rejects an unprofiled direct re-encode instead of relabelling the values.
Document I/O supplies the appropriate profile, so normal app and agent saves preserve
their colour meaning.

Import supports 8/10/12-bit AV1 still pictures, including general AV1 sequence headers
without the optional `still_picture` flag ([AVIF 1.2 §2.1](https://aomediacodec.github.io/av1-avif/v1.2.0.html#av1-image-item)),
alpha, monochrome (expanded to RGB),
full/limited range, and common BT.601/BT.709/BT.2020 nonconstant-luminance matrices. High-depth
samples are scaled to full-range U16 storage. Subsampled chroma currently uses nearest-neighbour
upsampling; import quality differs from viewers with filtered chroma reconstruction.
Sequences, grids/derived images, essential unknown properties, rotation/mirror/clean-aperture
transforms and unsupported matrices are rejected. EXIF, XMP, DPI and text are not preserved by
this initial AVIF path; exports warn about losses. Gain maps and non-alpha auxiliary items are
not imported. No HDR gain-map reconstruction is claimed.

Dimensions and allocation budgets are checked before pixel decode. The decoder reserves a
conservative 64 bytes per declared pixel for working memory; a tight allocation budget can
therefore reject an image whose final RGB buffer alone would fit. Export is capped at 120 MP
and 65536 pixels per side. Third-party codec calls are guarded against escaped Rust panics on
native builds. AVIF is unavailable on browser wasm.

## Agent and CLI options

```text
photocraft-cli convert input.png output.avif --quality 90 --avif-speed 8 --avif-depth 10 --avif-alpha-quality 100
```

`run` and `batch` accept the same CLI flags. Engine `file.saveACopy` and file/batch exports use
`avifQuality`, `avifSpeed`, `avifDepth` (0, 8, 10), `avifAlphaQuality`; JPEG's legacy `quality`
0–12 scale remains separate. Headless RPC `doc.save` and MCP `doc_save` accept those four
parameters; their generic `quality` is 1–100 and also sets AVIF colour quality. Live bridge
MCP saves use app defaults and reject headless-only option requests explicitly. For custom
live settings, use the Export As dialog and `ui.dialog.set`.

## Dependencies and verification

Reviewed on 2026-10-09 against upstream documentation and crate sources:

* [`ravif` 0.13.0](https://docs.rs/ravif/0.13.0/ravif/struct.Encoder.html), BSD-3-Clause,
  MSRV 1.85, backed by Rust `rav1e`. Assembly and threading features are disabled.
* Official [`memorysafety/rav1d`](https://github.com/memorysafety/rav1d), BSD-2-Clause,
  MSRV 1.79, pinned to upstream commit
  [`d3d1cd67059f47803919be8276650e5870c9fd02`](https://github.com/memorysafety/rav1d/commit/d3d1cd67059f47803919be8276650e5870c9fd02).
  The published 1.1.0 crate predates its safe Rust API, so this version uses an exact
  Git revision. The former `sqzer-rav1d` fork is removed. No local unsafe code is added;
  the official decoder's internals contain unsafe Rust. Assembly is disabled, although
  upstream still declares `cc` and `nasm-rs` build dependencies; its assembly-disabled
  build script does not invoke a C compiler or assembler.
* [`avif-parse` 2.1.0](https://github.com/kornelski/avif-parse), MPL-2.0, MSRV 1.90,
  fallible container/AV1 header parsing. It remains an optional, unmodified dependency.
  **Maintainer acceptance of this licence remains required before merging.** Opt-in
  gating does not resolve that licensing decision.

No system codec libraries, NASM installation or external image command are required.
The official decoder does not include the fork's wasm libc shim; this first version
therefore supports native builds only. Default release and browser builds stay unchanged
unless their native packaging explicitly enables `avif`.

Synthetic tests cover quality/depth/profile/alpha, independent 12-bit libaom output and
libdav1d decode pixels, sniffing, truncation, hostile dimensions and validation. Fixture
provenance and reproducible generation live in
[`crates/codecs/tests/fixtures/avif/README.md`](../crates/codecs/tests/fixtures/avif/README.md).
The test's maximum source round-trip channel error at quality 100 is bounded by 0.012.

The original handoff reported Windows x64 correctness, I/O, UI, automation, corpus
and wasm checks using the previous decoder. Its offscreen Export As screenshots
remain useful UI evidence, but those runs do not validate the replacement decoder.
Current Linux validation with the official decoder is recorded in the PR description.
Tests include every truncation of two independent libaom fixtures, corrupt AV1 payloads,
hostile dimensions and allocation limits, plus disabled-feature import/export checks.
Native CI explicitly enables `heif,avif`; a separate Linux pass tests default builds.
Browser CI checks the workspace without AVIF and checks the feature-enabled codec stub.

The following performance figure is historical handoff data using the former decoder;
it has not been remeasured with the official dependency.

A 24 MP synthetic RGB8 gradient at quality 90/speed 8 in the release scalar,
single-thread path took 18445 ms to encode and 1244 ms to decode (15486 bytes).
This run overlapped compiler activity and establishes a diagnostic timing, not
an enforced budget or representative photographic compression ratio. There is
no before measurement because this build adds AVIF decoding. Reproduce with:

```text
cargo test -j12 --release -p photocraft-codecs --features avif --test avif avif_24mp_release_timing -- --ignored --nocapture
```
