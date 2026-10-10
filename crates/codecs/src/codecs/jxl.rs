//! JPEG XL, read-only: see [`crate::ASYMMETRIC_EXCEPTIONS`].
//!
//! Decoding lives in the optional `photocraft-jxl` crate (jxl-oxide, a pure-Rust decoder of the
//! whole specification), enabled by this crate's `jxl` feature. Without it, `.jxl` files are
//! still detected and opening one is a [`CodecError::Unsupported`] error, never a panic.
//!
//! JPEG XL records orientation in the codestream header, not in EXIF, so the decoder applies it
//! and the EXIF Orientation is rewritten to 1: a file's EXIF tag only mirrors what the header
//! already says, and applying it as well would turn the picture twice. With
//! `keep_orientation` the upright pixels are turned back to how they were coded.

use crate::Format;
use crate::error::CodecError;
use crate::image::Image;
use crate::options::Limits;

const F: Format = Format::Jxl;

/// The reason given when this build has no JPEG XL decoder.
#[cfg(not(feature = "jxl"))]
pub(crate) const NOT_IN_BUILD: &str = "JPEG XL support isn't included in this build of PhotoCraft";

#[cfg(not(feature = "jxl"))]
pub(crate) fn decode(_bytes: &[u8], _limits: &Limits, _keep_orientation: bool) -> Result<Image, CodecError> {
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(feature = "jxl")]
fn err(e: photocraft_jxl::Error) -> CodecError {
    match e {
        photocraft_jxl::Error::Unsupported(what) => CodecError::unsupported(F, what),
        photocraft_jxl::Error::Limit(what) => CodecError::LimitExceeded(what),
        photocraft_jxl::Error::Malformed(what) => CodecError::malformed(F, what),
    }
}

#[cfg(feature = "jxl")]
fn layout(l: photocraft_jxl::Layout) -> crate::image::ChannelLayout {
    use crate::image::ChannelLayout as L;
    match l {
        photocraft_jxl::Layout::Gray => L::Gray,
        photocraft_jxl::Layout::GrayA => L::GrayA,
        photocraft_jxl::Layout::Rgb => L::Rgb,
        photocraft_jxl::Layout::Rgba => L::Rgba,
    }
}

/// Our sample type for the decoder's: 16-bit floats come back as F16, everything else as the
/// decoder hands it out.
#[cfg(feature = "jxl")]
fn sample(s: photocraft_jxl::Sample, float: bool, bits: u32) -> crate::image::SampleType {
    use crate::image::SampleType as S;
    match s {
        photocraft_jxl::Sample::U8 => S::U8,
        photocraft_jxl::Sample::U16 => S::U16,
        photocraft_jxl::Sample::F32 if float && bits <= 16 => S::F16,
        photocraft_jxl::Sample::F32 => S::F32,
    }
}

/// The orientation that undoes `o` (EXIF numbering): 6 and 8 are each other's inverse, the
/// rest are their own.
#[cfg(feature = "jxl")]
fn inverse(o: u8) -> u16 {
    match o {
        6 => 8,
        8 => 6,
        o => u16::from(o),
    }
}

#[cfg(feature = "jxl")]
pub(crate) fn decode(bytes: &[u8], limits: &Limits, keep_orientation: bool) -> Result<Image, CodecError> {
    use crate::image::{DecodeWarning, Metadata, SampleType};

    // The header alone: the declared size is checked before any pixel is decoded. Orientations
    // 5–8 swap width and height, so both the coded and the upright size must fit, and float and
    // premultiplied-alpha images are rendered through a 32-bit float working buffer, so that
    // size is what the budget must hold.
    let decoder = photocraft_jxl::Decoder::open(bytes, limits.alloc_usize()).map_err(err)?;
    let info = decoder.info();
    let s = sample(info.sample, info.float, info.bits_per_sample);
    let working = if info.float || info.premultiplied { SampleType::F32 } else { s };
    limits.check(info.width, info.height, layout(info.layout), working)?;
    limits.check(info.coded_width, info.coded_height, layout(info.layout), working)?;

    let d = decoder.decode().map_err(err)?;
    // Trust the decoded output's shape, not the header's.
    let l = layout(d.layout);
    let s = sample(d.sample, d.float, d.bits_per_sample);
    let mut img = if s == SampleType::F16 {
        let floats: Vec<f32> = d.data.chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect();
        Image::from_normalized(d.width, d.height, l, s, &floats)?
    } else {
        Image::from_raw(d.width, d.height, l, s, d.data)?
    };
    img.icc = d.icc;
    img.meta = Metadata {
        exif: d.exif.map(|e| if keep_orientation { e } else { crate::orientation::upright_exif(&e).into_owned() }),
        xmp: d.xmp.map(|x| if keep_orientation { x } else { crate::orientation::upright_xmp(&x).into_owned() }),
        ..Default::default()
    };
    // An animation opens at its first frame. A complete file says how many it held; a file cut
    // off after its first frame (or inside a later one) only that there were more.
    if d.frames > 1 {
        img.warnings.push(DecodeWarning::MoreFrames { total: Some(d.frames) });
    } else if d.animated && !d.complete {
        img.warnings.push(DecodeWarning::MoreFrames { total: None });
    }
    if keep_orientation && d.orientation != 1 {
        // Turning the pixels back as coded; the metadata stays as the file has it (`oriented`
        // would rewrite its Orientation to 1, which is right only for upright pixels).
        let (icc, meta, warnings) = (img.icc.take(), std::mem::take(&mut img.meta), std::mem::take(&mut img.warnings));
        img = img.oriented(inverse(d.orientation))?;
        img.icc = icc;
        img.meta = meta;
        img.warnings = warnings;
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARE: &[u8] = &[0xFF, 0x0A, 0x30, 0x54];
    const CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl \0\0\0\0jxl ";

    #[cfg(not(feature = "jxl"))]
    #[test]
    fn without_the_feature_jxl_is_a_clear_unsupported_error() {
        for header in [BARE, CONTAINER] {
            assert_eq!(crate::detect(header), Some(F), "still recognised");
            let r = crate::decode(header);
            assert!(matches!(&r, Err(CodecError::Unsupported { format: Format::Jxl, reason }) if reason.contains("isn't included in this build")), "{r:?}");
        }
        assert!(!crate::caps(F).read);
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn with_the_feature_jxl_is_readable_and_a_bare_header_is_malformed() {
        assert!(crate::caps(F).read);
        for header in [BARE, CONTAINER] {
            let r = crate::decode(header);
            assert!(matches!(&r, Err(CodecError::Malformed { format: Format::Jxl, .. })), "{r:?}");
        }
        assert!(matches!(crate::decode_as(F, b""), Err(CodecError::Malformed { .. })));
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn inverse_orientation_undoes_each_one() {
        for o in 1..=8u8 {
            let img = crate::image::Image::from_u8(3, 2, crate::image::ChannelLayout::Gray, vec![1, 2, 3, 4, 5, 6]).unwrap();
            let back = img.clone().oriented(u16::from(o)).unwrap().oriented(inverse(o)).unwrap();
            assert_eq!(back.data(), img.data(), "orientation {o}");
        }
    }
}
