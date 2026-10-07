# Raster formats

`photocraft-codecs` provides format detection, capability declarations, decode limits, metadata handling, and encoding for flat images. The default build declares read/write support for PNG, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO, Netpbm/PFM, QOI, OpenEXR, and Radiance HDR. AVIF encoding is optional; AVIF decoding is not implemented in the current crate.

Capability details such as sample depth, channel layout, alpha, ICC, EXIF, XMP, DPI, and animation behavior are defined by `crates/codecs/src/format.rs`. Do not infer fidelity from the filename extension alone.

## Exporting documents

Every route that writes a document to a flat format (Save As, the CLI, MCP `doc_export`, Layers to Files, Layer › Export As) flattens it the same way:

- Formats without transparency (JPEG, Radiance HDR) get the image composited over white, as Export As and Quick Export do; in CMYK, white is no ink.
- Formats that can't embed an ICC profile (GIF, BMP, TGA, QOI, ICO, Netpbm/PFM) get RGB documents in another profile (linear light, Display P3, …) converted to sRGB through the colour engine (perceptual intent), since an untagged file is read as sRGB. OpenEXR and Radiance HDR get linear sRGB instead.

## Implemented decode limits

`DecodeOptions::default()` applies `Limits` before decoded pixel allocation:

| Limit | Default |
|---|---:|
| Maximum width | 262,144 pixels |
| Maximum height | 262,144 pixels |
| Maximum pixel count | 268,435,456 pixels |
| Maximum decoded allocation | 2 GiB |

Format adapters pass compatible allocation limits into PNG, TIFF, WebP, and `image`-based decoders, while other implementations perform explicit checked dimension/buffer validation. Tests cover bomb-like headers, truncation, bit flips, header overwrites, and random data.

`Limits::none()` exists for callers with a deliberate reason to relax policy. It remains bounded by representable memory sizes, but it removes the default resource policy and should not be used for untrusted input.

Animated containers and multi-page TIFFs are represented as a single decoded frame or page by the current codec API; opening one adds an import warning such as "only the first of 3 frames was imported". A JPEG cut off inside its image data opens with a warning that its data ends early (an error when it holds no image data at all). Refer to [`crates/codecs/README.md`](https://github.com/storytold/photocraft/blob/main/crates/codecs/README.md) and source capability declarations for current behavior.
