//! Reject image decoders without enforceable budgets before Hayro sees a page.
//! This deliberately supports a conservative subset: unfiltered, Flate and JPEG
//! streams. JPX/JBIG2/CCITT/LZW and inline images fail explicitly, not as missing
//! artwork. PDF dictionaries alone cannot bound codec-internal allocations.
use std::io::Read;

use super::error;
use crate::IoError;
use hayro::hayro_syntax::{
    Pdf,
    object::{Array, Dict, Name, Object, Stream},
};
use photocraft_raster::Interrupt;

const STREAM_BYTES: usize = 64 * 1024 * 1024;
const TOTAL_BYTES: usize = 256 * 1024 * 1024;
const IMAGE_PIXELS: u64 = 16_000_000;
const TOTAL_IMAGE_PIXELS: u64 = 64_000_000;
const OBJECTS: usize = 100_000;

fn dimensions(w: u32, h: u32) -> Result<u64, IoError> {
    let pixels = u64::from(w) * u64::from(h);
    if w == 0 || h == 0 || w > 16_384 || h > 16_384 || pixels > IMAGE_PIXELS {
        return Err(error("embedded PDF image exceeds the 16,384-pixel side or 16-megapixel decode limit"));
    }
    Ok(pixels)
}

fn cancelled(ctl: &Interrupt) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}

/// Check the actual JPEG frame, not only the PDF's Width/Height declaration.
fn jpeg_dimensions(bytes: &[u8]) -> Result<u64, IoError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(error("invalid embedded JPEG"));
    }
    let mut pos = 2usize;
    while let Some(&byte) = bytes.get(pos) {
        if byte != 0xff {
            return Err(error("invalid embedded JPEG marker"));
        }
        while bytes.get(pos) == Some(&0xff) {
            pos += 1;
        }
        let marker = *bytes.get(pos).ok_or_else(|| error("truncated embedded JPEG"))?;
        pos += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 1 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let len = bytes.get(pos..pos + 2).ok_or_else(|| error("truncated embedded JPEG segment"))?;
        let [a, b] = *len else {
            return Err(error("invalid JPEG segment length"));
        };
        let len = usize::from(u16::from_be_bytes([a, b]));
        if len < 2 {
            return Err(error("invalid embedded JPEG segment"));
        }
        let segment = bytes.get(pos..pos + len).ok_or_else(|| error("truncated embedded JPEG segment"))?;
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            let frame = segment.get(2..8).ok_or_else(|| error("truncated embedded JPEG frame"))?;
            let [depth, h0, h1, w0, w1, components] = *frame else {
                return Err(error("invalid JPEG frame"));
            };
            if depth != 8 || ![1, 3, 4].contains(&components) {
                return Err(error("unsupported embedded JPEG color depth"));
            }
            return dimensions(u32::from(u16::from_be_bytes([w0, w1])), u32::from(u16::from_be_bytes([h0, h1])));
        }
        pos += len;
    }
    Err(error("embedded JPEG has no supported frame"))
}

fn inflate(bytes: &[u8], ctl: &Interrupt) -> Result<Vec<u8>, IoError> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        cancelled(ctl)?;
        let n = decoder.read(&mut chunk).map_err(|_| error("invalid PDF Flate stream"))?;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > STREAM_BYTES {
            return Err(error("PDF decoded stream exceeds 64 MB"));
        }
        out.extend_from_slice(chunk.get(..n).ok_or_else(|| error("invalid decoded stream length"))?);
    }
    Ok(out)
}

fn predictor(dict: &Dict<'_>) -> Result<(), IoError> {
    let params = dict.get::<Dict<'_>>(b"DecodeParms").or_else(|| dict.get::<Dict<'_>>(b"DP"));
    // Filter arrays are rejected below; don't let an array of parameters evade checks.
    if dict.get::<Array<'_>>(b"DecodeParms").is_some() || dict.get::<Array<'_>>(b"DP").is_some() {
        return Err(error("PDF stream filter parameter arrays are not supported by bounded import"));
    }
    if let Some(p) = params {
        let columns = p.get::<u32>(b"Columns").unwrap_or(1);
        let colors = p.get::<u32>(b"Colors").unwrap_or(1);
        let bits = p.get::<u32>(b"BitsPerComponent").unwrap_or(8);
        if columns == 0 || columns > 16_384 || !(1..=4).contains(&colors) || ![1, 2, 4, 8, 16].contains(&bits) {
            return Err(error("PDF predictor exceeds decode limits"));
        }
    }
    Ok(())
}

fn image_colorspace(space: Object<'_>, depth: u8) -> Result<(), IoError> {
    if depth > 2 {
        return Err(error("PDF image color space nesting exceeds decode limits"));
    }
    let supported = match space {
        Object::Name(n) => matches!(n.as_ref(), b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK"),
        Object::Array(a) => {
            let mut items = a.iter::<Object<'_>>();
            match items.next().and_then(Object::into_name).as_ref().map(|n| n.as_ref()) {
                Some(b"ICCBased") => items.next().and_then(Object::into_stream).is_some_and(|s| [1, 3, 4].contains(&s.dict().get::<u32>(b"N").unwrap_or(0))),
                Some(b"Indexed") => {
                    if let Some(base) = items.next() {
                        image_colorspace(base, depth + 1)?;
                    } else {
                        return Err(error("missing PDF indexed color space"));
                    }
                    true
                }
                _ => false,
            }
        }
        _ => false,
    };
    if !supported {
        return Err(error("PDF image color space is not supported by bounded import"));
    }
    Ok(())
}

fn has_inline_image(data: &[u8]) -> bool {
    let delimiter = |b: u8| b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b);
    data.split(|b| delimiter(*b)).any(|token| {
        let mut bytes = token.iter().copied();
        // Hayro normalizes name escapes while reading operators too. Cover those
        // spellings even though conforming PDF content uses the literal BI token.
        let mut next = || {
            let b = bytes.next()?;
            if b != b'#' {
                return Some(b);
            }
            let a = char::from(bytes.next()?).to_digit(16)? as u8;
            let b = char::from(bytes.next()?).to_digit(16)? as u8;
            Some((a << 4) | b)
        };
        next() == Some(b'B') && next() == Some(b'I') && next().is_none()
    })
}

fn stream(stream: &Stream<'_>, ctl: &Interrupt, images: &mut u64) -> Result<usize, IoError> {
    cancelled(ctl)?;
    let dict = stream.dict();
    let is_image = dict.get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image");
    if is_image {
        if let Some(space) = dict.get::<Object<'_>>(b"ColorSpace") {
            image_colorspace(space, 0)?;
        }
        *images = images.saturating_add(dimensions(dict.get::<u32>(b"Width").unwrap_or(0), dict.get::<u32>(b"Height").unwrap_or(0))?);
        if *images > TOTAL_IMAGE_PIXELS {
            return Err(error("PDF embedded images exceed 64 megapixels in total"));
        }
        if ![1, 2, 4, 8, 16].contains(&dict.get::<u32>(b"BitsPerComponent").unwrap_or(1)) {
            return Err(error("unsupported PDF image bit depth"));
        }
    }
    // Inspect names via the parser so escaped names and indirect filters cannot bypass this.
    if dict.get::<Array<'_>>(b"Filter").is_some() || dict.get::<Array<'_>>(b"F").is_some() {
        return Err(error("PDF filter chains are not supported by bounded import; resave with Flate or JPEG compression"));
    }
    let name = dict.get::<Name<'_>>(b"Filter").or_else(|| dict.get::<Name<'_>>(b"F"));
    let filter = name.as_ref().map(|n| n.as_ref());
    if filter.is_none() && (dict.contains_key(b"Filter") || dict.contains_key(b"F")) {
        return Err(error("invalid PDF stream filter"));
    }
    if !matches!(filter, None | Some(b"FlateDecode" | b"Fl" | b"DCTDecode" | b"DCT")) {
        return Err(error(
            "PDF image/stream codec has no bounded decoder (including JPEG2000, JBIG2, CCITT and LZW). Resave the PDF using JPEG or Flate compression.",
        ));
    }
    predictor(dict)?;
    if !is_image && dict.get::<Dict<'_>>(b"DecodeParms").or_else(|| dict.get::<Dict<'_>>(b"DP")).is_some_and(|p| p.get::<u32>(b"Predictor").unwrap_or(1) != 1) {
        return Err(error("PDF predictors are only supported on image streams"));
    }
    let raw = stream.raw_data();
    if raw.len() > STREAM_BYTES {
        return Err(error("PDF stream exceeds 64 MB"));
    }
    let decoded = match filter {
        Some(b"FlateDecode" | b"Fl") => std::borrow::Cow::Owned(inflate(&raw, ctl)?),
        Some(b"DCTDecode" | b"DCT") => {
            if !is_image {
                return Err(error("JPEG filter on a non-image PDF stream"));
            }
            // Both dimensions are bounded independently; count the larger codec frame too.
            *images = images.saturating_add(jpeg_dimensions(&raw)?);
            if *images > TOTAL_IMAGE_PIXELS {
                return Err(error("PDF embedded images exceed the decode budget"));
            }
            return Ok(raw.len());
        }
        _ => raw,
    };
    // Inline images can occur in page, form, pattern, appearance and Type3 streams.
    // Reject the operator before interpretation, including when nested in resources.
    // This conservative lexical check can also reject a literal string containing BI.
    if !is_image && has_inline_image(&decoded) {
        return Err(error("Inline PDF images are not supported by bounded import; resave them as image XObjects"));
    }
    Ok(decoded.len())
}

pub(super) fn preflight(pdf: &Pdf, ctl: &Interrupt) -> Result<(), IoError> {
    if pdf.len() > OBJECTS {
        return Err(error("PDF exceeds 100,000 objects"));
    }
    let (mut total, mut images) = (0usize, 0u64);
    for object in pdf.objects() {
        cancelled(ctl)?;
        if let Some(s) = object.into_stream() {
            total = total.saturating_add(stream(&s, ctl, &mut images)?);
            if total > TOTAL_BYTES {
                return Err(error("PDF decoded streams exceed 256 MB in total"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn inline_image_check_covers_operator_delimiters() {
        for data in [b"BI /W 1".as_slice(), b"q BI/W 1", b"q\nBI\n/W 1", b"BI", b"(x)BI/W 1", b"q B#49/W 1", b"q #42I/W 1"] {
            assert!(super::has_inline_image(data));
        }
        assert!(!super::has_inline_image(b"BT (BINDER) Tj ET"));
    }
}
