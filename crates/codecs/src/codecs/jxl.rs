//! Pure-Rust lossless JPEG XL (Modular) writer.
//!
//! Writes a bare JPEG XL codestream unless EXIF/XMP embedding is requested.
//! The encoder accepts 8- or 16-bit gray, RGB and RGBA samples, but not ICC embedding.
//! Gray+alpha sources are expanded losslessly to RGBA by the export plan.
//! The flat export pipeline converts tagged RGB documents to sRGB first.

use std::panic::{AssertUnwindSafe, catch_unwind};

use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions as ZuneOptions};
use zune_jpegxl::JxlSimpleEncoder;

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};

const F: Format = Format::Jxl;

/// A JPEG XL codestream exported here is labelled sRGB. Rewrite only known,
/// well-formed XMP colour-space declarations when they refer to the source
/// profile. Unrelated metadata (camera, dates, GPS, copyright, etc.) stays
/// byte-for-byte unchanged. Unrecognized XML forms are left untouched rather
/// than risking damage to a valid XMP packet.
fn srgb_xmp_metadata(xmp: &str) -> std::borrow::Cow<'_, str> {
    let mut out = xmp.to_owned();
    // Exact attribute/element values only; do not replace free-form user text.
    for (old, new) in [
        ("exif:ColorSpace=\"65535\"", "exif:ColorSpace=\"1\""),
        ("exif:ColorSpace=\"2\"", "exif:ColorSpace=\"1\""),
        ("exif:ColorSpace='65535'", "exif:ColorSpace='1'"),
        ("exif:ColorSpace='2'", "exif:ColorSpace='1'"),
        ("<exif:ColorSpace>65535</exif:ColorSpace>", "<exif:ColorSpace>1</exif:ColorSpace>"),
        ("<exif:ColorSpace>2</exif:ColorSpace>", "<exif:ColorSpace>1</exif:ColorSpace>"),
        ("exif:InteroperabilityIndex=\"R03\"", "exif:InteroperabilityIndex=\"R98\""),
        ("exif:InteroperabilityIndex='R03'", "exif:InteroperabilityIndex='R98'"),
        ("<exif:InteroperabilityIndex>R03</exif:InteroperabilityIndex>",
         "<exif:InteroperabilityIndex>R98</exif:InteroperabilityIndex>"),
        ("photoshop:ICCProfile=\"Adobe RGB (1998)\"", "photoshop:ICCProfile=\"sRGB\""),
        ("photoshop:ICCProfile='Adobe RGB (1998)'", "photoshop:ICCProfile='sRGB'"),
        ("<photoshop:ICCProfile>Adobe RGB (1998)</photoshop:ICCProfile>",
         "<photoshop:ICCProfile>sRGB</photoshop:ICCProfile>"),
    ] {
        if out.contains(old) {
            out = out.replace(old, new);
        }
    }
    if out == xmp { std::borrow::Cow::Borrowed(xmp) } else { std::borrow::Cow::Owned(out) }
}


// Preserve the bare codestream when embedding metadata is not requested.
// JPEG XL stores EXIF and XMP in ISO 18181-2 container boxes.
fn append_jxl_box(out: &mut Vec<u8>, name: &[u8; 4], payload: &[u8]) -> Result<(), CodecError> {
    let short_len = payload
        .len()
        .checked_add(8)
        .ok_or_else(|| CodecError::encode(F, "JPEG XL box size overflow"))?;
    if let Ok(len) = u32::try_from(short_len) {
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(name);
    } else {
        let long_len = u64::try_from(payload.len())
            .ok()
            .and_then(|n| n.checked_add(16))
            .ok_or_else(|| CodecError::encode(F, "JPEG XL box size overflow"))?;
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&long_len.to_be_bytes());
    }
    out.extend_from_slice(payload);
    Ok(())
}

fn jxl_with_metadata(codestream: Vec<u8>, exif: Option<&[u8]>, xmp: Option<&str>) -> Result<Vec<u8>, CodecError> {
    let exif = exif.filter(|b| !b.is_empty());
    let xmp = xmp.filter(|s| !s.is_empty());
    if exif.is_none() && xmp.is_none() {
        return Ok(codestream);
    }
    if !codestream.starts_with(&[0xff, 0x0a]) {
        return Err(CodecError::encode(F, "expected a JPEG XL codestream"));
    }

    let mut output = Vec::new();
    output.extend_from_slice(b"\0\0\0\x0cJXL \r\n\x87\n");
    output.extend_from_slice(b"\0\0\0\x14ftypjxl \0\0\0\0jxl ");
    if let Some(exif) = exif {
        // TIFF data follows a four-byte big-endian offset of zero.
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(exif);
        append_jxl_box(&mut output, b"Exif", &payload)?;
    }
    if let Some(xmp) = xmp {
        append_jxl_box(&mut output, b"xml ", xmp.as_bytes())?;
    }
    append_jxl_box(&mut output, b"jxlc", &codestream)?;
    Ok(output)
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

pub(crate) fn encode(image: &Image, plan: Plan, opts: &crate::EncodeOptions) -> Result<Vec<u8>, CodecError> {
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
    let codestream = catch_unwind(AssertUnwindSafe(|| {
        let options = ZuneOptions::new(width as usize, height as usize, space, depth);
        let mut output = Vec::new();
        JxlSimpleEncoder::new(image.data(), options).encode(&mut output).map_err(|e| CodecError::encode(F, e.to_string()))?;
        if depth == BitDepth::Sixteen && image.layout() == ChannelLayout::Rgba { correct_16_bit_alpha_header(&output, width, height) } else { Ok(output) }
    }))
    .unwrap_or_else(|_| Err(CodecError::encode(F, "the JPEG XL encoder failed")))?;
    if opts.embed_metadata {
        // The RGB modular codestream is sRGB. Only its RGB metadata needs
        // colour-space reconciliation; leave grayscale EXIF/XMP untouched.
        let exif = image.meta.exif.as_deref().map(|b| {
            if image.layout().is_rgb() {
                crate::exif_color::srgb_exif_metadata(b)
            } else {
                std::borrow::Cow::Borrowed(b)
            }
        });
        let xmp = image.meta.xmp.as_deref().map(|packet| {
            if image.layout().is_rgb() {
                srgb_xmp_metadata(packet)
            } else {
                std::borrow::Cow::Borrowed(packet)
            }
        });
        jxl_with_metadata(codestream, exif.as_deref(), xmp.as_deref())
    } else {
        Ok(codestream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_header_correction_rejects_unexpected_or_truncated_headers() {
        for bytes in [&[][..], &[0xff, 0x0a][..], &[0; 32][..]] {
            assert!(correct_16_bit_alpha_header(bytes, 2, 2).is_err());
        }
    }

    #[test]
    fn xmp_colour_metadata_is_reconciled_without_changing_unrelated_fields() {
        let source = "<rdf:Description exif:ColorSpace=\"2\" photoshop:ICCProfile=\"Adobe RGB (1998)\" dc:creator=\"Camera A\"><exif:InteroperabilityIndex>R03</exif:InteroperabilityIndex></rdf:Description>";
        let adjusted = srgb_xmp_metadata(source);
        assert!(adjusted.contains("exif:ColorSpace=\"1\""));
        assert!(adjusted.contains("photoshop:ICCProfile=\"sRGB\""));
        assert!(adjusted.contains("<exif:InteroperabilityIndex>R98</exif:InteroperabilityIndex>"));
        assert!(adjusted.contains("dc:creator=\"Camera A\""));
        assert!(!adjusted.contains("Adobe RGB (1998)"));
        assert!(matches!(srgb_xmp_metadata(&adjusted), std::borrow::Cow::Borrowed(_)));
        let unchanged = "free-form text about Adobe RGB (1998) is not a colour tag";
        assert!(matches!(srgb_xmp_metadata(unchanged), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn metadata_container_keeps_exif_xmp_and_codestream_bytes() {
        let stream = vec![0xff, 0x0a, 0x12, 0x34];
        let exif = b"II*\0\x08\0\0\0";
        let xmp = "<xmp>test</xmp>";
        let result = jxl_with_metadata(stream.clone(), Some(exif), Some(xmp)).unwrap();
        assert_eq!(&result[..12], b"\0\0\0\x0cJXL \r\n\x87\n");
        assert_eq!(&result[12..32], b"\0\0\0\x14ftypjxl \0\0\0\0jxl ");
        let mut pos = 32;
        let mut exif_box = vec![0, 0, 0, 0];
        exif_box.extend_from_slice(exif);
        for (name, expected) in [
            (b"Exif".as_slice(), exif_box),
            (b"xml ".as_slice(), xmp.as_bytes().to_vec()),
            (b"jxlc".as_slice(), stream),
        ] {
            let len = u32::from_be_bytes(result[pos..pos + 4].try_into().unwrap()) as usize;
            assert!(len >= 8);
            assert_eq!(&result[pos + 4..pos + 8], name);
            assert_eq!(&result[pos + 8..pos + len], expected.as_slice());
            pos += len;
        }
        assert_eq!(pos, result.len());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn missing_metadata_preserves_the_existing_bare_codestream() {
        let original = vec![0xff, 0x0a, 0x12, 0x34];
        assert_eq!(jxl_with_metadata(original.clone(), None, None).unwrap(), original);
        assert_eq!(jxl_with_metadata(original.clone(), Some(&[]), Some("")).unwrap(), original);
        assert!(jxl_with_metadata(vec![1, 2], Some(b"TIFF"), None).is_err());
    }
}
