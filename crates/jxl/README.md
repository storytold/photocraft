# photocraft-jxl

The optional JPEG XL decoder of PhotoCraft. A thin, panic-guarded wrapper around
[`jxl-oxide`](https://github.com/tirr-c/jxl-oxide), a pure-Rust decoder of the whole JPEG XL
specification (ISO/IEC 18181, MIT OR Apache-2.0): Modular and VarDCT images, lossless and lossy,
bare codestreams and the container, 1 to 32-bit integer and 16/32-bit float samples, grayscale
and RGB with alpha, the orientation field, ICC profiles and enum colour encodings, animation
(first frame), spot colours (rendered in), EXIF and XMP boxes, and losslessly recompressed JPEGs.
Read-only: there is no pure-Rust JPEG XL encoder yet (libjxl is C++).

```rust
let info = photocraft_jxl::probe(&bytes, 8 << 30)?;      // size, layout, depth: check limits first
let img = photocraft_jxl::decode(&bytes, &photocraft_jxl::Options { alloc_limit: 8 << 30 })?;
// img.width, img.height (upright), img.layout, img.sample, img.data, img.icc, img.exif, img.xmp, img.frames
```

Errors are `Error::Unsupported` (CMYK images), `Error::Limit` (the decoder's own allocations
passed `alloc_limit`) or `Error::Malformed`. It never panics: every call into jxl-oxide runs under
`catch_unwind`, and a panic inside it becomes `Error::Malformed`.

## Why it is a separate, optional crate

Like `photocraft-heif`: `photocraft-codecs` uses this crate only behind its non-default `jxl`
feature, so distributors choose at build time whether a decoder for a still-young format goes into
their build; official PhotoCraft builds and CI enable it. Without it, `.jxl` files are still
recognised and opening one is a clear "JPEG XL support isn't included in this build" error.

The crate depends on no other PhotoCraft crate and knows nothing of `photocraft-codecs`' types;
`codecs` → `jxl` is one of the two allowed dependencies between standalone crates (`cargo xtask
layers`). Tests on real files live in `photocraft-codecs` (`tests/jxl.rs`, feature `jxl`, fixtures
in `tests/fixtures/jxl/` written by `scripts/jxl_fixtures.py` with libjxl's `cjxl`).
