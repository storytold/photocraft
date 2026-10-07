//! JPEG: decode via `zune-jpeg` (gray, YCbCr, CMYK, YCCK), encode via
//! `jpeg-encoder` (gray, RGB, CMYK). Metadata (APP0 JFIF density, APP1
//! EXIF/XMP, APP2 multi-segment ICC, APP14 Adobe) is parsed by our own marker
//! scanner so behaviour does not depend on decoder internals.

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits};
use crate::orientation::{upright_exif, upright_xmp};

const F: Format = Format::Jpeg;
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const ICC_HEADER: &[u8] = b"ICC_PROFILE\0";

#[derive(Default, Debug)]
pub(crate) struct JpegMeta {
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<String>,
    pub dpi: Option<(f32, f32)>,
    pub adobe_transform: Option<u8>,
    /// (width, height, components) from the first SOFn.
    pub frame: Option<(u32, u32, u8)>,
}

/// Walk the marker segments up to SOS and collect metadata.
pub(crate) fn scan_metadata(b: &[u8]) -> JpegMeta {
    let mut m = JpegMeta::default();
    let mut icc_parts: Vec<(u8, u8, &[u8])> = Vec::new();
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            break;
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 || i + 2 + len > b.len() {
            break;
        }
        let seg = &b[i + 4..i + 2 + len];
        match marker {
            0xE0 if seg.len() >= 12 && seg.starts_with(b"JFIF\0") => {
                let unit = seg[7];
                let x = u16::from_be_bytes([seg[8], seg[9]]) as f32;
                let y = u16::from_be_bytes([seg[10], seg[11]]) as f32;
                if x > 0.0 && y > 0.0 {
                    m.dpi = match unit {
                        1 => Some((x, y)),
                        2 => Some((x * 2.54, y * 2.54)),
                        _ => None,
                    };
                }
            }
            0xE1 if seg.starts_with(EXIF_HEADER) && m.exif.is_none() => {
                m.exif = Some(seg[EXIF_HEADER.len()..].to_vec());
            }
            0xE1 if seg.starts_with(XMP_HEADER) && m.xmp.is_none() => {
                m.xmp = std::str::from_utf8(&seg[XMP_HEADER.len()..]).ok().map(|s| s.trim_end_matches('\0').to_owned());
            }
            0xE2 if seg.len() >= 14 && seg.starts_with(ICC_HEADER) => {
                icc_parts.push((seg[12], seg[13], &seg[14..]));
            }
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) && seg.len() >= 6 && m.frame.is_none() => {
                let h = u16::from_be_bytes([seg[1], seg[2]]) as u32;
                let w = u16::from_be_bytes([seg[3], seg[4]]) as u32;
                m.frame = Some((w, h, seg[5]));
            }
            0xEE if seg.len() >= 12 && seg.starts_with(b"Adobe") => {
                m.adobe_transform = Some(seg[11]);
            }
            _ => {}
        }
        i += 2 + len;
    }
    if !icc_parts.is_empty() {
        icc_parts.sort_by_key(|p| p.0);
        let total = icc_parts[0].1 as usize;
        let seqs_ok = icc_parts.len() == total && icc_parts.iter().enumerate().all(|(k, p)| p.0 as usize == k + 1 && p.1 as usize == total);
        if seqs_ok || icc_parts.len() == 1 {
            m.icc = Some(icc_parts.iter().flat_map(|p| p.2.iter().copied()).collect());
        }
    }
    m
}

/// How a JPEG's data ends, seen from its marker structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataEnd {
    /// An end-of-image marker follows the scans (or the structure is too odd to tell).
    Complete,
    /// The file ends inside the image data, before the end-of-image marker.
    Truncated,
    /// The file ends before the first scan holds any data: nothing can be decoded.
    Empty,
}

/// Walks the marker segments (skipped by their length) and the entropy-coded scan data
/// (skipped up to the next marker that isn't a stuffed `FF 00`, a restart or a fill byte),
/// so an embedded thumbnail's end marker or data after the end of the image don't count.
fn data_end(b: &[u8]) -> DataEnd {
    // Where the bytes run out: inside the image data once a scan has held some.
    let cut = |scan_data: bool| if scan_data { DataEnd::Truncated } else { DataEnd::Empty };
    let mut i = 2; // past SOI
    let mut scan_data = false;
    loop {
        let Some(&byte) = b.get(i) else { return cut(scan_data) };
        if byte != 0xFF {
            // Not where a marker should be: too odd to judge.
            return DataEnd::Complete;
        }
        let Some(&code) = b.get(i + 1) else { return cut(scan_data) };
        match code {
            0xD9 => return DataEnd::Complete,
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            _ => {
                let Some(len) = b.get(i + 2..i + 4).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes) else {
                    return cut(scan_data);
                };
                i = i.saturating_add(2 + usize::from(len));
                if code == 0xDA {
                    let start = i;
                    loop {
                        let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&v| v == 0xFF)) else {
                            return cut(scan_data || b.len() > start);
                        };
                        i += p;
                        match b.get(i + 1) {
                            Some(0x00 | 0xD0..=0xD7) => i += 2,
                            Some(0xFF) => i += 1,
                            _ => break,
                        }
                    }
                    scan_data |= i > start;
                }
            }
        }
    }
}

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let end = data_end(bytes);
    if end == DataEnd::Empty {
        return Err(err("the file ends before any image data"));
    }
    let meta = scan_metadata(bytes);
    if let Some((w, h, nc)) = meta.frame {
        limits.check_bytes(w, h, u64::from(nc.max(1)))?;
    }
    let options = DecoderOptions::default().set_strict_mode(false).set_max_width(65535).set_max_height(65535);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    dec.decode_headers().map_err(err)?;
    let (w, h) = dec.dimensions().ok_or_else(|| err("no dimensions"))?;
    let (w, h) = (w as u32, h as u32);
    let input = dec.input_colorspace().ok_or_else(|| err("no colorspace"))?;
    let (out_cs, layout) = match input {
        ColorSpace::Luma | ColorSpace::LumaA => (ColorSpace::Luma, ChannelLayout::Gray),
        ColorSpace::CMYK => (ColorSpace::CMYK, ChannelLayout::Cmyk),
        ColorSpace::YCCK => (ColorSpace::YCCK, ChannelLayout::Cmyk),
        _ => (ColorSpace::RGB, ChannelLayout::Rgb),
    };
    limits.check(w, h, layout, SampleType::U8)?;
    dec.set_options(dec.options().jpeg_set_out_colorspace(out_cs));
    let mut px = dec.decode().map_err(err)?;
    let expected = w as usize * h as usize * layout.channels();
    if px.len() < expected {
        return Err(err("short pixel buffer"));
    }
    px.truncate(expected);
    match input {
        // Adobe-style CMYK is stored inverted (255 = no ink).
        ColorSpace::CMYK => px.iter_mut().for_each(|v| *v = 255 - *v),
        ColorSpace::YCCK => {
            // Adobe APP14 transform 2 (what Photoshop writes for CMYK JPEGs): the YCbCr
            // carries the *inverted* inks and K passes through — libjpeg's `ycck_cmyk_convert`
            // (YCbCr→RGB, then CMY = 255−RGB) and zune-jpeg decode it the same way.
            for p in px.as_chunks_mut::<4>().0 {
                let (y, cb, cr) = (p[0] as f32, p[1] as f32 - 128.0, p[2] as f32 - 128.0);
                let r = y + 1.402 * cr;
                let g = y - 0.344_136 * cb - 0.714_136 * cr;
                let b = y + 1.772 * cb;
                p[0] = 255 - r.round().clamp(0.0, 255.0) as u8;
                p[1] = 255 - g.round().clamp(0.0, 255.0) as u8;
                p[2] = 255 - b.round().clamp(0.0, 255.0) as u8;
            }
        }
        _ => {}
    }
    let mut img = Image::from_raw(w, h, layout, SampleType::U8, px)?;
    img.icc = meta.icc;
    img.meta = Metadata { exif: meta.exif, xmp: meta.xmp, dpi: meta.dpi, ..Default::default() };
    if end == DataEnd::Truncated {
        img.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(img)
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    let (w, h) = img.dimensions();
    let (w16, h16) = match (u16::try_from(w), u16::try_from(h)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return Err(CodecError::encode(F, "JPEG dimensions are limited to 65535"));
        }
    };
    // Adobe CMYK JPEGs are YCCK (APP14 transform 2): the YCbCr carries the *inverted* inks and
    // K is straight — what Photoshop writes and libjpeg/zune decode. `jpeg-encoder`'s
    // `CmykAsYcck` takes YCbCr over the values it is given and stores K inverted, so hand it
    // the inverted buffer: the file then holds YCbCr(R,G,B) and K.
    let (data, ct): (std::borrow::Cow<[u8]>, _) = match img.layout() {
        ChannelLayout::Gray => (std::borrow::Cow::Borrowed(img.data()), jpeg_encoder::ColorType::Luma),
        ChannelLayout::Rgb => (std::borrow::Cow::Borrowed(img.data()), jpeg_encoder::ColorType::Rgb),
        ChannelLayout::Cmyk => {
            let mut v = img.data().to_vec();
            for p in v.as_chunks_mut::<4>().0 {
                for s in p {
                    *s = 255 - *s;
                }
            }
            (std::borrow::Cow::Owned(v), jpeg_encoder::ColorType::CmykAsYcck)
        }
        l => return Err(CodecError::encode(F, format!("unsupported layout {l:?}"))),
    };
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, opts.jpeg_quality.clamp(1, 100));
    // CMYK is always written 4:4:4: subsampled 4-component JPEGs are not
    // decoded consistently across implementations.
    let sampling = if opts.jpeg_chroma_subsampling && img.layout() == ChannelLayout::Rgb {
        jpeg_encoder::SamplingFactor::R_4_2_0
    } else {
        jpeg_encoder::SamplingFactor::R_4_4_4
    };
    enc.set_sampling_factor(sampling);
    let e = |e: jpeg_encoder::EncodingError| CodecError::encode(F, e);
    if opts.embed_metadata {
        if let Some((x, y)) = img.meta.dpi
            && x >= 1.0
            && y >= 1.0
        {
            enc.set_density(jpeg_encoder::Density::Inch { x: x.round().min(65535.0) as u16, y: y.round().min(65535.0) as u16 });
        }
        // Metadata too large for its one segment is left out (and reported by the fidelity
        // warnings) rather than failing the whole export.
        if let Some(seg) = img.meta.exif.as_deref().and_then(exif_segment) {
            enc.add_app_segment(1, &seg).map_err(e)?;
        }
        if let Some(seg) = img.meta.xmp.as_deref().and_then(xmp_segment) {
            enc.add_app_segment(1, &seg).map_err(e)?;
        }
    }
    if opts.embed_icc
        && let Some(icc) = &img.icc
    {
        enc.add_icc_profile(icc).map_err(e)?;
    }
    enc.encode(&data, w16, h16, ct).map_err(e)?;
    Ok(out)
}

/// The most data one APP segment holds (its 16-bit length counts its own two bytes).
const MAX_SEGMENT: usize = 65533;

/// XMP properties (local names, any namespace prefix) that only describe the layered document:
/// every document ever placed in it, the text of every type layer. A flat image doesn't need them,
/// and they alone can outgrow a segment.
const LAYERED_ONLY_XMP: [&str; 2] = ["DocumentAncestors", "TextLayers"];

/// The EXIF APP1 segment, or `None` when it doesn't fit in one segment.
pub(crate) fn exif_segment(exif: &[u8]) -> Option<Vec<u8>> {
    // The pixels are written as they are shown: never let a viewer rotate them again.
    let seg = [EXIF_HEADER, upright_exif(exif).as_ref()].concat();
    (seg.len() <= MAX_SEGMENT).then_some(seg)
}

/// The XMP APP1 segment. When the packet doesn't fit in one segment, the layered-document
/// properties are left out; `None` when it still doesn't fit.
pub(crate) fn xmp_segment(xmp: &str) -> Option<Vec<u8>> {
    let xmp = upright_xmp(xmp);
    let seg = |x: &str| [XMP_HEADER, x.as_bytes()].concat();
    if XMP_HEADER.len() + xmp.len() <= MAX_SEGMENT {
        return Some(seg(&xmp));
    }
    let trimmed = LAYERED_ONLY_XMP.iter().fold(xmp.into_owned(), |x, name| remove_element(&x, name));
    (XMP_HEADER.len() + trimmed.len() <= MAX_SEGMENT).then(|| seg(&trimmed))
}

/// `xmp` without its `<prefix:name …>…</prefix:name>` and `<prefix:name …/>` elements. Unbalanced
/// markup is kept as is.
fn remove_element(xmp: &str, name: &str) -> String {
    let local = format!(":{name}");
    let mut out = String::with_capacity(xmp.len());
    let mut rest = xmp;
    while let Some(j) = rest.find(&local) {
        let head = rest.get(..j).unwrap_or_default();
        let prefix_len = head.bytes().rev().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')).count();
        let lt = j - prefix_len;
        let after = rest.get(j + local.len()..).unwrap_or_default();
        // An opening tag of exactly this name (not a closing tag, an attribute or a longer name).
        let opening = prefix_len > 0 && lt > 0 && rest.get(lt - 1..lt) == Some("<");
        if !opening || !after.starts_with(['>', '/', ' ', '\t', '\r', '\n']) {
            out.push_str(rest.get(..j + local.len()).unwrap_or_default());
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let end = if after.get(..gt).is_some_and(|s| s.ends_with('/')) {
            gt + 1
        } else {
            let close = format!("</{}{local}>", rest.get(lt..j).unwrap_or_default());
            let Some(c) = after.find(&close) else { break };
            c + close.len()
        };
        out.push_str(rest.get(..lt - 1).unwrap_or_default());
        rest = after.get(end..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}
