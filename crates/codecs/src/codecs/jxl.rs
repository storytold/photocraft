//! JPEG XL: read behind the `jxl` feature, lossless write in every build.
//!
//! Decoding lives in the optional `photocraft-jxl` crate (jxl-oxide, a pure-Rust decoder of the
//! whole specification), enabled by this crate's `jxl` feature. Without it, `.jxl` files are
//! still detected and opening one is a [`CodecError::Unsupported`] error, never a panic, and the
//! format is write-only: see [`crate::ASYMMETRIC_EXCEPTIONS`].
//!
//! JPEG XL records orientation in the codestream header, not in EXIF, so the decoder applies it
//! and the EXIF Orientation is rewritten to 1: a file's EXIF tag only mirrors what the header
//! already says, and applying it as well would turn the picture twice. With
//! `keep_orientation` the upright pixels are turned back to how they were coded.
//!
//! Writing is `zune-jpegxl`, a pure-Rust lossless (Modular) encoder. It currently writes a bare
//! JPEG XL codestream from 8- or 16-bit gray, RGB and RGBA samples and does not write ICC/EXIF/XMP
//! boxes. Gray+alpha sources are expanded losslessly to RGBA by the export plan, and the flat
//! export pipeline converts tagged RGB documents to sRGB first.

use std::panic::{AssertUnwindSafe, catch_unwind};

use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions as ZuneOptions};
use zune_jpegxl::JxlSimpleEncoder;

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
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
fn layout(l: photocraft_jxl::Layout) -> ChannelLayout {
    match l {
        photocraft_jxl::Layout::Gray => ChannelLayout::Gray,
        photocraft_jxl::Layout::GrayA => ChannelLayout::GrayA,
        photocraft_jxl::Layout::Rgb => ChannelLayout::Rgb,
        photocraft_jxl::Layout::Rgba => ChannelLayout::Rgba,
    }
}

/// Our sample type for the decoder's: 16-bit floats come back as F16, everything else as the
/// decoder hands it out.
#[cfg(feature = "jxl")]
fn sample(s: photocraft_jxl::Sample, float: bool, bits: u32) -> SampleType {
    match s {
        photocraft_jxl::Sample::U8 => SampleType::U8,
        photocraft_jxl::Sample::U16 => SampleType::U16,
        photocraft_jxl::Sample::F32 if float && bits <= 16 => SampleType::F16,
        photocraft_jxl::Sample::F32 => SampleType::F32,
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
    use crate::image::{DecodeWarning, Metadata};

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
        let floats: Vec<f32> = d.data.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
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
    if d.complete && d.frames > 1 {
        img.warnings.push(DecodeWarning::MoreFrames { total: Some(d.frames) });
    } else if d.frames > 1 || (d.animated && !d.complete) {
        // The loaded count is only a lower bound while the file is cut off.
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

// zune-jpegxl 0.5.2 emits ExtraChannelInfo(all_default=true) for every alpha
// channel. In JPEG XL that flag means *8-bit* alpha, even when the modular
// payload was encoded from 16-bit samples. Replace only that metadata bundle,
// leaving the entire encoded frame and its TOC untouched. The replacement is
// exactly 16 bits longer, so subsequent byte-aligned data stays aligned.
//
// This deliberately verifies the pinned encoder's header layout rather than
// trying to parse or rewrite arbitrary JPEG XL codestreams. A different header
// must fail closed, not produce a file that silently corrupts transparency.
fn correct_16_bit_alpha_header(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CodecError> {
    fn read(bytes: &[u8], pos: &mut usize, count: usize) -> Option<u32> {
        let mut value = 0u32;
        for bit in 0..count {
            let offset = *pos;
            let byte = *bytes.get(offset / 8)?;
            value |= u32::from((byte >> (offset % 8)) & 1) << bit;
            *pos += 1;
        }
        Some(value)
    }

    fn read_dimension(bytes: &[u8], pos: &mut usize) -> Option<u32> {
        let bits = match read(bytes, pos, 2)? {
            0 => 9,
            1 => 13,
            2 => 18,
            _ => 30,
        };
        read(bytes, pos, bits)?.checked_add(1)
    }

    fn write_bits(out: &mut Vec<u8>, pos: &mut usize, count: usize, value: u32) {
        for bit in 0..count {
            if (*pos).is_multiple_of(8) {
                out.push(0);
            }
            out[*pos / 8] |= (((value >> bit) & 1) as u8) << (*pos % 8);
            *pos += 1;
        }
    }

    let mut pos = 0;
    let header_matches = read(bytes, &mut pos, 16) == Some(0x0aff)
        && read(bytes, &mut pos, 1) == Some(0)
        && read_dimension(bytes, &mut pos) == Some(height)
        && read(bytes, &mut pos, 3) == Some(0)
        && read_dimension(bytes, &mut pos) == Some(width)
        && read(bytes, &mut pos, 1) == Some(0) // metadata not all-default
        && read(bytes, &mut pos, 1) == Some(0) // no extra fields
        && read(bytes, &mut pos, 1) == Some(0) // integer samples
        && read(bytes, &mut pos, 2) == Some(3) // custom bit depth
        && read(bytes, &mut pos, 6) == Some(15) // 16 bits
        && read(bytes, &mut pos, 1) == Some(0) // uses >14 bits
        && read(bytes, &mut pos, 2) == Some(1); // one extra channel
    let alpha_flag_pos = pos;
    let header_matches = header_matches
        && read(bytes, &mut pos, 1) == Some(1) // incorrect default 8-bit alpha
        && read(bytes, &mut pos, 1) == Some(0) // not XYB
        && read(bytes, &mut pos, 1) == Some(1) // default sRGB encoding (RGBA)
        && read(bytes, &mut pos, 2) == Some(0) // no extensions
        && read(bytes, &mut pos, 1) == Some(1); // default transform

    let frame_offset = pos.div_ceil(8);
    let header_matches = header_matches && frame_offset <= bytes.len() && (pos..frame_offset * 8).all(|bit| bytes[bit / 8] & (1 << (bit % 8)) == 0);
    if !header_matches {
        return Err(CodecError::encode(F, "unexpected zune-jpegxl header for 16-bit alpha"));
    }

    let mut fixed = Vec::with_capacity(bytes.len() + 2);
    let mut cursor = 0;
    for bit in 0..alpha_flag_pos {
        write_bits(&mut fixed, &mut cursor, 1, u32::from((bytes[bit / 8] >> (bit % 8)) & 1));
    }

    // JPEG XL ExtraChannelInfo, non-default, straight 16-bit alpha:
    // type=kAlpha (Enum=0), bit_depth=uint16, dim_shift=0, name_len=0,
    // alpha_associated=false. Adds exactly 16 bits versus all_default=true.
    write_bits(&mut fixed, &mut cursor, 1, 0);
    write_bits(&mut fixed, &mut cursor, 2, 0);
    write_bits(&mut fixed, &mut cursor, 1, 0);
    write_bits(&mut fixed, &mut cursor, 2, 3);
    write_bits(&mut fixed, &mut cursor, 6, 15);
    write_bits(&mut fixed, &mut cursor, 2, 0);
    write_bits(&mut fixed, &mut cursor, 2, 0);
    write_bits(&mut fixed, &mut cursor, 1, 0);

    for bit in alpha_flag_pos + 1..frame_offset * 8 {
        write_bits(&mut fixed, &mut cursor, 1, u32::from((bytes[bit / 8] >> (bit % 8)) & 1));
    }
    if cursor != (frame_offset + 2) * 8 {
        return Err(CodecError::encode(F, "JPEG XL alpha header alignment mismatch"));
    }
    fixed.extend_from_slice(&bytes[frame_offset..]);
    Ok(fixed)
}

pub(crate) fn encode(image: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let image = image.converted(plan.layout, plan.sample);
    let space = match image.layout() {
        ChannelLayout::Gray => ColorSpace::Luma,
        ChannelLayout::GrayA => ColorSpace::LumaA,
        ChannelLayout::Rgb => ColorSpace::RGB,
        ChannelLayout::Rgba => ColorSpace::RGBA,
        _ => return Err(CodecError::unsupported(F, "JPEG XL export supports grayscale and RGB, with optional alpha")),
    };
    let depth = match image.sample_type() {
        SampleType::U8 => BitDepth::Eight,
        SampleType::U16 => BitDepth::Sixteen,
        _ => return Err(CodecError::unsupported(F, "JPEG XL lossless export supports 8 or 16-bit integer pixels")),
    };
    let (width, height) = image.dimensions();
    let samples = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(image.layout().channels()))
        .and_then(|n| n.checked_mul(image.sample_type().bytes()))
        .ok_or_else(|| CodecError::encode(F, "image dimensions overflow the buffer size"))?;
    if image.data().len() != samples {
        return Err(CodecError::InvalidImage("incorrect JPEG XL pixel buffer length".into()));
    }
    // A panic in an external image encoder must not terminate the host application.
    catch_unwind(AssertUnwindSafe(|| {
        let options = ZuneOptions::new(width as usize, height as usize, space, depth);
        let mut output = Vec::new();
        JxlSimpleEncoder::new(image.data(), options).encode(&mut output).map_err(|e| CodecError::encode(F, e.to_string()))?;
        if depth == BitDepth::Sixteen && image.layout() == ChannelLayout::Rgba { correct_16_bit_alpha_header(&output, width, height) } else { Ok(output) }
    }))
    .unwrap_or_else(|_| Err(CodecError::encode(F, "the JPEG XL encoder failed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARE: &[u8] = &[0xFF, 0x0A, 0x30, 0x54];
    const CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl \0\0\0\0jxl ";

    #[test]
    fn alpha_header_correction_rejects_unexpected_or_truncated_headers() {
        for bytes in [&[][..], &[0xff, 0x0a][..], &[0; 32][..]] {
            assert!(correct_16_bit_alpha_header(bytes, 2, 2).is_err());
        }
    }

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
