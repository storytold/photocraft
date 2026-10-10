# photocraft-io

Import and export between `photocraft_doc::Document` and PSD/PSB, the native `.pcraft` bundle,
camera raw inputs and flat raster formats. The byte-oriented API performs no file-system I/O:
`import(name, bytes)` returns an `ImportResult`, and `export(document, name_or_extension, options)`
returns encoded bytes and fidelity warnings. The app and automation services handle writing files.

Flat exports render the document, keeping a single raster layer's native samples where possible.
They report flattening, depth reduction, clipped HDR values and unsupported metadata instead of
silently promising document fidelity. Layered PSD/PSB and `.pcraft` saves keep document data;
TIFF can keep Photoshop layer data when `ExportOptions::tiff_layers` is enabled.

## Optional format features

- `avif`: static AVIF export through the pure-Rust encoder in `photocraft-codecs`.
- `heif`: HEIC/HEIF import through `photocraft-codecs`.
- `corpus`: pinned real-file tests; missing corpus files fail with fetching instructions.

Library features are opt-in. The desktop, CLI and web apps enable AVIF by default; builds using
`--no-default-features` omit its encoder unless `--features avif` is added again.

## AVIF export

`export(&document, "output.avif", &options)` writes one 8-bit RGB/RGBA image. Quality is
`options.encode.jpeg_quality` (1–100), shared with JPEG; both colour and alpha compression are
lossy, including at quality 100. The export result warns when 16/32-bit document samples are
reduced to 8 bits or HDR values are clipped. ICC, EXIF, XMP and physical resolution are not
preserved; `XmpEmbed::None` prevents an unwanted XMP packet from reaching the encoder.

RGB documents with a non-sRGB profile are converted to sRGB before their profile is dropped.
CMYK raster data is converted to sRGB through the document's ICC profile, or the built-in coated
CMYK profile when none is usable. Grayscale tone curves are converted to sGray before expansion
to neutral RGB, keeping alpha; untagged or unusably tagged gray uses the sGray fallback. This
document path uses `photocraft-cms`; direct `photocraft-codecs::encode` performs only layout/depth
conversion.

AVIF import and animated AVIF export are unsupported. Without the `avif` feature, export returns
an error identifying that AVIF cannot be written in this build. The encoder's maximum canvas is
65535 pixels on each axis; larger dimensions return an error before allocating a pixel buffer.
This backend limit does not guarantee that such a large canvas fits in memory.

## Tests

```sh
cargo test -p photocraft-io --test avif_export
cargo test -p photocraft-io --features avif --test avif_export
```

The enabled-feature tests cover document export, transparency, explicit colour-managed RGB/CMYK
references, grayscale gamma profiles, quality, depth/HDR and metadata warnings. Set
`PHOTOCRAFT_AVIF_ORACLE_DIR` to an artifact directory while running them to save AVIF/PNG pairs
for pixel checks with an independent decoder. Files use the `io-rgb`, `io-rgba`, `io-cmyk-srgb`,
`io-p3-srgb`, `io-gray-gamma22` and `io-gray-linear` prefixes. Compare visible colours or
composites; RGB under fully transparent pixels is not guaranteed.
