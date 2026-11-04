//! The PSD/PSB thumbnail image resource (1036), written when Preferences › File Handling › Image
//! Previews saves them (Photoshop's default, Always Save).
//!
//! Layout (Adobe PSD spec, "Thumbnail resource format"): a 28-byte big-endian header (format 1 =
//! kJpegRGB, width, height, widthbytes = (width × 24 + 31) / 32 × 4, total size = widthbytes ×
//! height, compressed size, bits per pixel 24, planes 1) followed by the JFIF stream. Photoshop
//! keeps the long side at 160 px or less.

use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType as CSample};
use photocraft_color::ColorMode;
use photocraft_doc::Document;
use photocraft_psd::ReadAt;
use photocraft_psd::resources::{ImageResource, ids};
use photocraft_raster::Rgba8Image;

use crate::IoError;

/// Longest side of the thumbnail, as Photoshop writes it.
pub const MAX_SIDE: u32 = 160;

/// Size of the header in front of the JFIF data.
pub const HEADER_LEN: usize = 28;

/// The 1036 resource for `doc`: its composite reduced to at most [`MAX_SIDE`], converted to
/// sRGB through the document's profile, matted over white and JPEG-compressed.
pub(crate) fn resource(doc: &Document) -> Result<ImageResource, IoError> {
    let mut buf = photocraft_compose::thumbnail_buffer(doc, MAX_SIDE);
    let (w, h) = (buf.rect.width(), buf.rect.height());
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || buf.px.len() != (w as usize) * (h as usize) {
        return Err(IoError::Unsupported("the image preview could not be rendered".into()));
    }
    let src = composite_profile(doc);
    let dst = Builtin::Srgb.profile();
    if !src.same_colors(dst) {
        let t = Transform::new(&src, dst, Intent::RelativeColorimetric, true).map_err(|e| IoError::Unsupported(e.to_string()))?;
        t.apply(buf.px.as_flattened_mut(), 4);
    }
    let mut rgb = Vec::with_capacity(buf.px.len() * 3);
    for p in &buf.px {
        let a = finite(p[3]);
        for &c in &p[..3] {
            // Over white, as Photoshop shows a transparent document's preview.
            let v = finite(c) * a + (1.0 - a);
            rgb.push((v * 255.0).round() as u8);
        }
    }
    let img = Image::from_raw(w, h, ChannelLayout::Rgb, CSample::U8, rgb)?;
    let opts = EncodeOptions { jpeg_quality: 80, embed_icc: false, embed_metadata: false, ..EncodeOptions::default() };
    let jfif = photocraft_codecs::encode(&img, Format::Jpeg, &opts)?;
    Ok(ImageResource::new(ids::THUMBNAIL, thumbnail_bytes(w, h, &jfif)?))
}

/// Longest side [`read`] accepts: the format keeps the long side at 160 and some writers go a little over, so a
/// larger one is not a thumbnail and decoding it could cost more than the file is worth.
pub const MAX_READ_SIDE: u32 = 1024;

/// The embedded thumbnail (resource 1036) of the PSD or PSB in `src`, as RGBA, or `None` when it
/// has none. Only the file's header and image resource section are read, however large the file
/// is, and every length in them is checked first. A damaged file, an unsupported thumbnail format
/// or a JPEG that does not match its header is an error, never a panic.
pub fn read<R: ReadAt + ?Sized>(src: &R) -> Result<Option<Rgba8Image>, IoError> {
    let resources = photocraft_psd::resources::read_resources(src)?;
    resources.iter().find(|r| r.id == ids::THUMBNAIL).map(|r| decode_resource(&r.data)).transpose()
}

/// [`read`] for the file at `path`, without reading it whole.
#[cfg(any(unix, windows))]
pub fn read_path(path: &std::path::Path) -> Result<Option<Rgba8Image>, IoError> {
    let src = photocraft_psd::FileSource::open(path).map_err(|e| IoError::Read(format!("{}: {e}", path.display())))?;
    read(&src)
}

/// The picture in the data of a 1036 resource: the 28-byte header of [`thumbnail_bytes`], then a
/// JFIF stream (format 1) or raw RGB rows padded to `widthbytes` (format 0).
fn decode_resource(data: &[u8]) -> Result<Rgba8Image, IoError> {
    let bad = |why: &str| IoError::Unsupported(format!("image preview: {why}"));
    let u32_at = |at: usize| data.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes).ok_or_else(|| bad("truncated header"));
    let (format, w, h, width_bytes, compressed) = (u32_at(0)?, u32_at(4)?, u32_at(8)?, u32_at(12)?, u32_at(20)?);
    if w == 0 || h == 0 || w > MAX_READ_SIDE || h > MAX_READ_SIDE {
        return Err(bad("unsupported size"));
    }
    let body = data.get(HEADER_LEN..).ok_or_else(|| bad("truncated header"))?;
    match format {
        1 => {
            // The stream's own length when the header's is missing or wrong.
            let jfif = usize::try_from(compressed).ok().filter(|&n| n > 0).and_then(|n| body.get(..n)).unwrap_or(body);
            let limits = photocraft_codecs::Limits {
                max_width: MAX_READ_SIDE,
                max_height: MAX_READ_SIDE,
                max_pixels: u64::from(MAX_READ_SIDE) * u64::from(MAX_READ_SIDE),
                max_alloc: 64 << 20,
            };
            let img = photocraft_codecs::decode_with(jfif, &photocraft_codecs::DecodeOptions { limits, ..Default::default() })?;
            // The picture's own size: some writers round the header's.
            let (width, height) = img.dimensions();
            Ok(Rgba8Image { width, height, pixels: img.to_rgba8() })
        }
        0 => {
            let row = usize::try_from(width_bytes).map_err(|_| bad("unsupported size"))?;
            let (wide, high) = (w as usize, h as usize);
            let needed = row.checked_mul(high).ok_or_else(|| bad("unsupported size"))?;
            if row < wide * 3 || body.len() < needed {
                return Err(bad("truncated pixels"));
            }
            let mut pixels = Vec::with_capacity(wide * high * 4);
            for y in 0..high {
                let line = body.get(y * row..y * row + wide * 3).ok_or_else(|| bad("truncated pixels"))?;
                for px in line.as_chunks::<3>().0 {
                    pixels.extend_from_slice(px);
                    pixels.push(255);
                }
            }
            Ok(Rgba8Image { width: w, height: h, pixels })
        }
        _ => Err(bad("unsupported format")),
    }
}

/// A sample clamped to 0–1 (NaN as 0): 32-bit composites can hold values past white.
fn finite(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// Header plus JFIF data of a `w`×`h` thumbnail resource.
fn thumbnail_bytes(w: u32, h: u32, jfif: &[u8]) -> Result<Vec<u8>, IoError> {
    let too_big = || IoError::Unsupported("image preview too large".into());
    let width_bytes = w.checked_mul(24).map(|v| v.div_ceil(32) * 4).ok_or_else(too_big)?;
    let total = width_bytes.checked_mul(h).ok_or_else(too_big)?;
    let compressed = u32::try_from(jfif.len()).map_err(|_| too_big())?;
    let mut out = Vec::with_capacity(HEADER_LEN + jfif.len());
    for v in [1, w, h, width_bytes, total, compressed] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&24u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(jfif);
    Ok(out)
}

/// The profile of the compositor's RGB output for `doc`: an RGB document's own (embedded when it
/// parses, else sRGB), a gray document's gray profile seen as RGB, and sRGB for CMYK and Lab
/// (which the compositor renders in sRGB).
pub(crate) fn composite_profile(doc: &Document) -> Profile {
    let space = match doc.mode {
        ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => ColorSpace::Gray,
        ColorMode::Cmyk | ColorMode::Lab => return Builtin::Srgb.profile().clone(),
        _ => ColorSpace::Rgb,
    };
    let own = doc.icc_profile.as_ref().and_then(|b| Profile::parse(b).ok()).filter(|p| p.color_space == space);
    match space {
        ColorSpace::Gray => {
            own.and_then(|p| p.gray_as_rgb()).or_else(|| Builtin::SGray.profile().gray_as_rgb()).unwrap_or_else(|| Builtin::Srgb.profile().clone())
        }
        _ => own.unwrap_or_else(|| Builtin::Srgb.profile().clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_follows_the_spec() {
        let b = thumbnail_bytes(160, 90, &[0xFF, 0xD8, 0xFF, 0xD9]).unwrap();
        let u = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
        assert_eq!((u(0), u(4), u(8), u(12), u(16), u(20)), (1, 160, 90, 480, 480 * 90, 4));
        assert_eq!(&b[24..28], &[0, 24, 0, 1]);
        assert_eq!(&b[28..], &[0xFF, 0xD8, 0xFF, 0xD9]);
        // Width bytes are padded to 4: 3 px × 3 bytes = 9 → 12.
        let odd = thumbnail_bytes(3, 1, &[]).unwrap();
        assert_eq!(u32::from_be_bytes(odd[12..16].try_into().unwrap()), 12);
    }
}
