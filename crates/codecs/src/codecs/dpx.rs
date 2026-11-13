//! DPX (SMPTE 268M Digital Picture Exchange) and Kodak Cineon (`.cin`): the format code is
//! craft-libs' `craft-dpx`. This maps its frames and header fields onto this crate's image:
//!
//! * The first image element, luma, RGB or RGBA (ABGR comes back as RGBA). Y'CbCr elements are
//!   not converted here and stay unsupported.
//! * 8- and 16-bit code values become U8 and U16 samples unchanged; 10- and 12-bit ones become U16
//!   as `round(c × 65535 / (2^b − 1))`, so they come back exactly when written at their depth.
//! * No colour transform: the transfer characteristic and colorimetric specification are reported
//!   ([`DecodeWarning::DpxEncoding`]), so a log frame is labelled log, not shown as display-referred.
//! * The image orientation is applied on import (the caller turns the image upright).
//! * Header fields a pipeline relies on travel as text metadata under the [`KEYS`] names, and a
//!   DPX export writes them back.
//!
//! Writing: one element, big-endian, unencoded, luma, RGB or RGBA; 8 and 16 bits with packing 0,
//! 10 and 12 bits with packing 1 (method A). The bit depth follows the image (8 → 8, 16 → 16)
//! unless [`EncodeOptions::dpx_bits`] picks one. Cineon is written at 10 bits.

use craft_dpx::{Frame, Info, Kind, Layout};

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, SampleType};
use crate::options::{DpxBits, EncodeOptions, Limits};

const F: Format = Format::Dpx;

/// Header fields carried as text metadata.
pub(crate) const KEY_CREATOR: &str = "DPX:Creator";
pub(crate) const KEY_PROJECT: &str = "DPX:Project";
pub(crate) const KEY_COPYRIGHT: &str = "DPX:Copyright";
pub(crate) const KEY_CREATION_TIME: &str = "DPX:CreationTime";
pub(crate) const KEY_TRANSFER: &str = "DPX:Transfer";
pub(crate) const KEY_COLORIMETRIC: &str = "DPX:Colorimetric";
pub(crate) const KEY_FRAME_POSITION: &str = "DPX:FramePosition";
pub(crate) const KEY_FRAME_RATE: &str = "DPX:FrameRate";
pub(crate) const KEY_TIME_CODE: &str = "DPX:TimeCode";
pub(crate) const KEYS: [&str; 9] =
    [KEY_CREATOR, KEY_PROJECT, KEY_COPYRIGHT, KEY_CREATION_TIME, KEY_TRANSFER, KEY_COLORIMETRIC, KEY_FRAME_POSITION, KEY_FRAME_RATE, KEY_TIME_CODE];

fn err(format: Format, e: craft_dpx::Error) -> CodecError {
    match e {
        craft_dpx::Error::NotDpx => CodecError::malformed(format, "missing the DPX or Cineon magic number"),
        craft_dpx::Error::Malformed(m) => CodecError::malformed(format, m),
        craft_dpx::Error::Unsupported(m) => CodecError::unsupported(format, m),
        craft_dpx::Error::LimitExceeded(m) => CodecError::LimitExceeded(m.into()),
        craft_dpx::Error::Encode(m) => CodecError::encode(format, m),
    }
}

fn dpx_limits(l: &Limits) -> craft_dpx::Limits {
    let mut out = craft_dpx::Limits::default();
    out.max_pixels = l.max_pixels;
    out
}

/// The header, for a file of `kind`; anything else is malformed.
fn info(bytes: &[u8], kind: Kind) -> Result<Info, CodecError> {
    let format = if kind == Kind::Dpx { F } else { Format::Cineon };
    if craft_dpx::sniff(bytes) != Some(kind) {
        return Err(CodecError::malformed(
            format,
            if kind == Kind::Dpx { "missing SDPX/XPDS magic number" } else { "missing the Cineon magic number (802A5FD7)" },
        ));
    }
    // The size is checked by the caller, against its own limits.
    let mut unlimited = craft_dpx::Limits::default();
    unlimited.max_pixels = u64::MAX;
    craft_dpx::probe(bytes, &unlimited).map_err(|e| err(format, e))
}

/// The decoder opens only element zero; protect the other declared elements from flat saves.
pub(crate) fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
    Ok(info(bytes, Kind::Dpx)?.elements > 1)
}

/// The EXIF orientation (1–8) that turns the stored lines upright; 1 for an unreadable header.
pub(crate) fn orientation(b: &[u8]) -> u16 {
    info(b, Kind::Dpx).map_or(1, |i| i.exif_orientation())
}

/// The Cineon magic number (`802A5FD7` hex).
pub(crate) fn is_cineon(b: &[u8]) -> bool {
    craft_dpx::sniff(b) == Some(Kind::Cineon)
}

/// The EXIF orientation (1–8) for a Cineon header's orientation code.
pub(crate) fn cineon_orientation(b: &[u8]) -> u16 {
    info(b, Kind::Cineon).map_or(1, |i| i.exif_orientation())
}

/// A decoded frame as an image: luma, RGB or RGBA, 8 bits as U8, deeper codes widened to U16.
fn image(format: Format, frame: Frame) -> Result<(Info, Image), CodecError> {
    let i = &frame.info;
    let layout = match i.layout {
        Layout::Luma => ChannelLayout::Gray,
        Layout::Rgb => ChannelLayout::Rgb,
        Layout::Rgba => ChannelLayout::Rgba,
        l => return Err(CodecError::unsupported(format, format!("{l:?} (Y'CbCr) elements are not supported (only luma, RGB and RGBA)"))),
    };
    let img = match i.bits {
        8 => Image::from_u8(i.width, i.height, layout, frame.codes.iter().map(|&c| c as u8).collect())?,
        _ => Image::from_u16(i.width, i.height, layout, &frame.samples_u16())?,
    };
    Ok((frame.info, img))
}

/// Decodes a frame of `kind` within `limits`, checked as this crate checks every decoder.
fn decode_kind(bytes: &[u8], limits: &Limits, kind: Kind) -> Result<(Info, Image), CodecError> {
    let format = if kind == Kind::Dpx { F } else { Format::Cineon };
    let i = info(bytes, kind)?;
    let layout = match i.layout {
        Layout::Luma => ChannelLayout::Gray,
        Layout::Rgb => ChannelLayout::Rgb,
        _ => ChannelLayout::Rgba,
    };
    limits.check(i.width, i.height, layout, if i.bits == 8 { SampleType::U8 } else { SampleType::U16 })?;
    image(format, craft_dpx::decode(bytes, &dpx_limits(limits)).map_err(|e| err(format, e))?)
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let (info, mut img) = decode_kind(bytes, limits, Kind::Dpx)?;
    let i = &info;
    if i.elements > 1 {
        img.warnings.push(DecodeWarning::MoreElements { total: u32::from(i.elements) });
    }
    if i.transfer.is_some() || i.colorimetric.is_some() {
        img.warnings.push(DecodeWarning::DpxEncoding { transfer: i.transfer.unwrap_or(u8::MAX), colorimetric: i.colorimetric.unwrap_or(u8::MAX) });
    }
    img.meta.text = header_text(i);
    Ok(img)
}

/// Decode a Cineon (`.cin`) frame: 10-bit logarithmic film scans, one (gray) or three (RGB)
/// channels. The file's black/white reference points are not applied (a log conversion is a
/// display choice).
pub(crate) fn decode_cineon(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    decode_kind(bytes, limits, Kind::Cineon).map(|(_, img)| img)
}

/// The kept header fields as text metadata, in [`KEYS`] order; undefined ones are left out.
fn header_text(i: &Info) -> Vec<(String, String)> {
    let mut text = Vec::new();
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            text.push((k.to_string(), v));
        }
    };
    put(KEY_CREATOR, i.creator.clone());
    put(KEY_PROJECT, i.project.clone());
    put(KEY_COPYRIGHT, i.copyright.clone());
    put(KEY_CREATION_TIME, i.creation_time.clone());
    put(KEY_TRANSFER, i.transfer.map(|v| v.to_string()));
    put(KEY_COLORIMETRIC, i.colorimetric.map(|v| v.to_string()));
    put(KEY_FRAME_POSITION, i.frame_position.map(|v| v.to_string()));
    put(KEY_FRAME_RATE, i.frame_rate.map(|v| v.to_string()));
    put(KEY_TIME_CODE, i.time_code.map(craft_dpx::format_time_code));
    text
}

/// Bits written for `sample` when the options pick none: 8 for U8, 16 otherwise.
pub(crate) fn bits_for(opt: Option<DpxBits>, sample: SampleType) -> u8 {
    match opt {
        Some(b) => b.bits(),
        None if sample == SampleType::U8 => 8,
        None => 16,
    }
}

/// The bits `format` stores below its U16 plan: DPX at 10 or 12 bits, Cineon's 10. `None` when
/// the stored depth is the plan's sample type.
pub(crate) fn packed_bits(format: Format, opts: &EncodeOptions) -> Option<u8> {
    match format {
        Format::Dpx => opts.dpx_bits.filter(|b| matches!(b, DpxBits::Bits10 | DpxBits::Bits12)).map(DpxBits::bits),
        Format::Cineon => Some(10),
        _ => None,
    }
}

/// `true` when every sample of a U16 image is already a `bits`-bit code value, so writing it at
/// `bits` and reading it back gives the same samples (a frame opened from such a file).
pub(crate) fn keeps_u16_samples(img: &Image, bits: u8) -> bool {
    if img.sample_type() != SampleType::U16 || !(1..16).contains(&bits) {
        return false;
    }
    img.data().as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes(*c)).all(|v| craft_dpx::widen(craft_dpx::narrow(v, bits), bits) == v)
}

/// The plan's sample type for the chosen bits: U8 for 8 bits, U16 for 10, 12 and 16.
pub(crate) fn plan_sample(opt: Option<DpxBits>, src: SampleType) -> SampleType {
    match opt {
        Some(DpxBits::Bits8) => SampleType::U8,
        Some(_) => SampleType::U16,
        None if src == SampleType::U8 => SampleType::U8,
        None => SampleType::U16,
    }
}

fn text<'a>(img: &'a Image, key: &str) -> Option<&'a str> {
    img.meta.text.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
}

/// `img`'s samples as `bits`-bit code values, with the layout they are written in.
fn codes(format: Format, img: &Image, bits: u8) -> Result<(Layout, Vec<u16>), CodecError> {
    let layout = match img.layout() {
        ChannelLayout::Gray => Layout::Luma,
        ChannelLayout::Rgb => Layout::Rgb,
        ChannelLayout::Rgba if format == F => Layout::Rgba,
        l => return Err(CodecError::encode(format, format!("{format:?} cannot store {l:?}"))),
    };
    let codes: Vec<u16> = match (img.sample_type(), bits) {
        (SampleType::U8, 8) => img.data().iter().map(|&v| u16::from(v)).collect(),
        (SampleType::U16, 16) => img.to_u16_samples().unwrap_or_default(),
        (SampleType::U16, 10 | 12) => img.to_u16_samples().unwrap_or_default().into_iter().map(|v| craft_dpx::narrow(v, bits)).collect(),
        (s, b) => return Err(CodecError::encode(format, format!("cannot write {s:?} samples at {b} bits"))),
    };
    Ok((layout, codes))
}

/// Encode a Cineon 4.5 frame at 10 bits. The plan has already made the image U16 gray or RGB.
pub(crate) fn encode_cineon(src: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    let (layout, codes) = codes(Format::Cineon, &img, 10)?;
    let (width, height) = img.dimensions();
    craft_dpx::encode_cineon(&craft_dpx::Picture { width, height, layout, bits: 10, codes: &codes }).map_err(|e| err(Format::Cineon, e))
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    let bits = bits_for(opts.dpx_bits, img.sample_type());
    let (layout, codes) = codes(F, &img, bits)?;
    let keep = |key| if opts.embed_metadata { text(&img, key) } else { None };
    // No claim about the encoding unless the document came with one.
    let code = |key| keep(key).and_then(|v| v.parse::<u8>().ok());
    let meta = craft_dpx::Metadata {
        creator: Some(keep(KEY_CREATOR).unwrap_or("PhotoCraft")),
        project: keep(KEY_PROJECT),
        copyright: keep(KEY_COPYRIGHT),
        creation_time: keep(KEY_CREATION_TIME),
        transfer: code(KEY_TRANSFER),
        colorimetric: code(KEY_COLORIMETRIC),
        frame_position: keep(KEY_FRAME_POSITION).and_then(|v| v.parse::<u32>().ok()),
        frame_rate: keep(KEY_FRAME_RATE).and_then(|v| v.parse::<f32>().ok()),
        time_code: keep(KEY_TIME_CODE).and_then(craft_dpx::parse_time_code),
    };
    let (width, height) = img.dimensions();
    craft_dpx::encode(&craft_dpx::Picture { width, height, layout, bits, codes: &codes }, &meta).map_err(|e| err(F, e))
}
