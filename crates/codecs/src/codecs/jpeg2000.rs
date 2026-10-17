//! JPEG 2000 (ISO/IEC 15444-1): the JP2 file format and the raw J2K/J2C codestreams, plus the
//! Part 2 JPX/JPF containers, which carry a Part 1 codestream. The Part 6 compound file (JPM)
//! and the Part 3 motion stream (MJ2) are ISO base-media containers: every page or frame holds
//! a Part 1 codestream, so this module decodes the first one and reports the rest with a
//! [`DecodeWarning::MorePages`]/`MoreFrames` warning, like the other multi-image codecs.
//!
//! Built on the pure-Rust `j2k` crate (no C, no build script), so it compiles for wasm with
//! the rest of L0.
//!
//! Reading: the codestream's SIZ marker gives dimensions, component count, precision and
//! signedness; the JP2 `colr` box gives the colour model — an enumerated method-1 box, or an
//! embedded ICC profile, which we keep — and `cdef` marks the opacity channel. Samples come
//! back as one plane per component, which are interleaved into the layout the components
//! describe: a third-party CMYK or CMYK+alpha stream opens as such, because its colour space
//! says so, while one that only says "ICC" is read as gray or RGB by its component count.
//!
//! Writing: one codestream, plus the signature and header boxes for the file wrappers.
//! [`EncodeOptions::jp2_quality`] of 100 — the default, and what every caller that has not
//! asked for a smaller file gets — selects the reversible 5/3 integer wavelet, which is exact:
//! the reason to pick JPEG 2000. Anything lower uses the irreversible 9/7 wavelet with a
//! bits-per-pixel rate target, which is ordinary Part 1 (not HTJ2K), so the result stays a plain
//! `.jp2` that any Part 1 reader opens. The irreversible path carries Part 1's colour transform,
//! which is defined for three colour components, so it is used only for gray and RGB; `caps`
//! keeps CMYK and Lab out of the family, and they are converted to RGB with the usual warning.

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits};

use j2k::{
    J2kChannelAssociation, J2kChannelDefinition, J2kChannelType, J2kColorSpec, J2kDecoder, J2kEncodeValidation, J2kFileBoxMetadata, J2kFileColorSpec,
    J2kFileWrapOptions, J2kLosslessEncodeOptions, J2kLosslessSamples, J2kLossyEncodeOptions, J2kLossySamples, J2kRateTarget, encode_j2k_lossless,
    encode_j2k_lossy, wrap_j2k_codestream,
};
use j2k_core::Colorspace;

/// The format reported in errors: every JPEG 2000 wrapper shares one decoder and encoder, so
/// a message that names the family reads better than one naming the extension that was lost.
const F: Format = Format::Jp2;

/// JPEG 2000 Part 1 stores integers of 1..=38 bits; more than 16 does not fit our sample
/// types, and no Part 1 writer produces it.
const MAX_SUPPORTED_BITS: u8 = 16;

/// The JP2 signature box (ISO/IEC 15444-1 Annex I): 12 bytes, type `jP  `, payload
/// `0D 0A 87 0A`. Every JPEG 2000 file, JPM and MJ2 included, opens with it.
const JP2_SIGNATURE_BOX: &[u8] = &[0x00, 0x00, 0x00, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// Decode a JP2/J2K/JPX/JPF file: a single Part 1 codestream, with or without its JP2 wrapper.
pub fn decode(bytes: &[u8], format: Format, limits: &Limits) -> Result<Image, CodecError> {
    // `inspect_support` accepts both a raw codestream and a JP2-family wrapper, so the same
    // path decodes either; the compressed byte count is the whole file for the bomb guard.
    decode_codestream(bytes, format, limits)
}

/// Decode the first page (JPM) or frame (MJ2) of a multi-image JPEG 2000 ISO base-media
/// container, warning that more pages or frames exist.
pub fn decode_multi(bytes: &[u8], format: Format, limits: &Limits) -> Result<Image, CodecError> {
    // Every JPEG 2000 container opens with the Part 1 signature box; the `ftyp` box that follows
    // names it. Refuse anything that does not look like the container we were asked for.
    if !bytes.starts_with(JP2_SIGNATURE_BOX) {
        return Err(CodecError::malformed(format, "missing JPEG 2000 signature box"));
    }
    let brand = ftyp_brand(bytes).ok_or_else(|| CodecError::malformed(format, "missing ftyp box"))?;
    match format {
        Format::Jpm => {
            if brand != *b"jpm " {
                return Err(CodecError::malformed(format, format!("JPM file names brand {:?} instead of jpm ", brand)));
            }
            let pages = boxes_of_type(bytes, *b"page");
            if pages.is_empty() {
                return Err(CodecError::malformed(format, "JPM compound file holds no page"));
            }
            let page_count = pages.len() as u32;
            let codestream = boxes_of_type(pages[0], *b"jp2c")
                .into_iter()
                .next()
                .or_else(|| boxes_of_type(bytes, *b"jp2c").into_iter().next())
                .ok_or_else(|| CodecError::malformed(format, "JPM compound file holds no jp2c codestream"))?;
            let mut img = decode_codestream(codestream, format, limits)?;
            if page_count > 1 {
                img.warnings.push(DecodeWarning::MorePages { total: Some(page_count) });
            }
            Ok(img)
        }
        Format::Mj2 => {
            if brand != *b"mjp2" && brand != *b"mj2s" {
                return Err(CodecError::malformed(format, format!("Motion JPEG 2000 file names brand {:?} instead of mjp2/mj2s", brand)));
            }
            let Some(mdat) = top_level_box(bytes, *b"mdat") else {
                return Err(CodecError::malformed(format, "Motion JPEG 2000 file holds no mdat"));
            };
            let first = first_mj2_sample(bytes)?;
            let sample_offset = first.offset;
            let sample_size = first.size;
            let total_samples = first.total_samples;
            let candidates = sample_candidates(mdat.box_start, mdat.payload_start, sample_offset);
            let mut codestream: Option<&[u8]> = None;
            for start in candidates {
                let end = start.checked_add(sample_size as usize).ok_or_else(|| CodecError::malformed(format, "MJ2 sample offset and size overflow"))?;
                let Some(sample) = bytes.get(start..end).filter(|sample| !sample.is_empty()) else { continue };
                let payload = if sample.starts_with(&[0xFF, 0x4F]) {
                    Some(sample)
                } else {
                    match next_box(sample, 0) {
                        Some((kind, payload_start, payload_end, next)) if kind == *b"jp2c" && next == sample.len() => sample.get(payload_start..payload_end),
                        _ => None,
                    }
                };
                let Some(payload) = payload.filter(|payload| payload.starts_with(&[0xFF, 0x4F])) else { continue };
                codestream = Some(payload);
                break;
            }
            let codestream = codestream.ok_or_else(|| CodecError::malformed(format, "MJ2 first sample is not a JPEG 2000 codestream"))?;
            let mut img = decode_codestream(codestream, format, limits)?;
            if total_samples > 1 {
                img.warnings.push(DecodeWarning::MoreFrames { total: Some(total_samples) });
            }
            Ok(img)
        }
        _ => Err(CodecError::unsupported(format, "this build only decodes JPEG 2000 single-image containers here")),
    }
}

/// One top-level ISO base-media box located by walk, with its absolute offsets in `bytes`.
struct BoxRef<'a> {
    box_start: usize,
    payload_start: usize,
    payload: &'a [u8],
}

/// The box at the top level of `bytes` whose four-character type is `want`, if any.
fn top_level_box<'a>(bytes: &'a [u8], want: [u8; 4]) -> Option<BoxRef<'a>> {
    let mut offset = 0;
    while let Some((kind, start, end, next)) = next_box(bytes, offset) {
        if kind == want {
            return Some(BoxRef { box_start: offset, payload_start: start, payload: &bytes[start..end] });
        }
        offset = next;
    }
    None
}

/// Every top-level box of `bytes` whose four-character type is `want`, in file order.
fn boxes_of_type(bytes: &[u8], want: [u8; 4]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some((kind, start, end, next)) = next_box(bytes, offset) {
        if kind == want {
            out.push(&bytes[start..end]);
        }
        offset = next;
    }
    out
}

/// The four-character `ftyp` major brand of the container, or `None` when there is no usable
/// `ftyp` box right after the signature box.
fn ftyp_brand(bytes: &[u8]) -> Option<[u8; 4]> {
    let ftyp = top_level_box(bytes, *b"ftyp")?;
    let payload = ftyp.payload;
    if payload.len() < 8 {
        return None;
    }
    Some([payload[0], payload[1], payload[2], payload[3]])
}

/// Walk one ISO base-media box starting at `offset` in `bytes`.
///
/// Returns `(box type, payload start, payload end, next box offset)`. A `size` of 1 means a
/// 64-bit large size follows the type; a `size` of 0 means the box runs to the end of `bytes`.
/// Malformed headers never make the walk go backwards, so it always terminates.
fn next_box(bytes: &[u8], offset: usize) -> Option<([u8; 4], usize, usize, usize)> {
    if offset + 8 > bytes.len() {
        return None;
    }
    let size = u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as u64;
    let kind = [bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7]];
    let (header_len, total) = if size == 1 {
        if offset + 16 > bytes.len() {
            return None;
        }
        let large = u64::from_be_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
            bytes[offset + 12],
            bytes[offset + 13],
            bytes[offset + 14],
            bytes[offset + 15],
        ]);
        (16, large)
    } else if size == 0 {
        // Runs to the end of the file.
        (8, bytes.len() as u64 - offset as u64)
    } else {
        (8, size)
    };
    let payload_start = offset.checked_add(header_len)?;
    let payload_end = match usize::try_from(total) {
        Ok(t) => offset.saturating_add(t),
        Err(_) => bytes.len(),
    }
    .min(bytes.len())
    .max(payload_start);
    // Guarantee forward progress even when a box declares a size smaller than its header.
    let next = payload_end.max(offset + header_len);
    Some((kind, payload_start, payload_end, next))
}

/// The candidate absolute file offsets of the first MJ2 sample, preferring the ISO base-media
/// interpretation (absolute in the file) before trying relative offsets used by some writers.
fn sample_candidates(mdat_box_start: usize, mdat_payload_start: usize, stco_offset: u64) -> [usize; 3] {
    let offset = stco_offset as usize;
    [offset, mdat_payload_start.saturating_add(offset), mdat_box_start.saturating_add(offset)]
}

/// Decode a raw Part 1 codestream (the body shared by every JPEG 2000 wrapper).
fn decode_codestream(codestream: &[u8], format: Format, limits: &Limits) -> Result<Image, CodecError> {
    let header = J2kDecoder::inspect_support(codestream).map_err(|e| CodecError::malformed(format, format!("JPEG 2000 header: {e}")))?;
    let info = &header.info;
    let (width, height) = info.dimensions;
    let layout = layout_for(info.colorspace, info.components)?;
    let sample = sample_for(info.bit_depth)?;

    limits.check(width, height, layout, sample)?;
    Limits::check_plausible(u64::from(width) * u64::from(height) * (layout.channels() * sample.bytes()) as u64, codestream.len(), 1 << 16)?;

    let mut decoder = J2kDecoder::new(codestream).map_err(|e| CodecError::malformed(format, format!("JPEG 2000: {e}")))?;
    let planes = decoder.decode_components().map_err(|e| CodecError::malformed(format, format!("JPEG 2000 decode: {e}")))?;
    let channels = layout.channels();
    if planes.planes().len() < channels {
        return Err(CodecError::malformed(format, format!("JPEG 2000 decode produced {} of {channels} components", planes.planes().len())));
    }

    let data = interleave(planes.planes(), width, height, layout, sample)?;
    Ok(Image::from_raw(width, height, layout, sample, data)?.with_meta(Metadata::default()).with_icc(icc_profile(&header)))
}

/// Encode as a JP2 file. A `.jpx`/`.jpf` save writes this same Part 1 container, which every
/// JPX reader opens; it is not a Part 2 sequence.
pub fn encode(image: &Image, format: Format, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let codestream = codestream(image, format, plan, opts)?;
    wrap(image, plan, &codestream)
}

/// Encode as a raw J2K/J2C codestream: the same bitstream without the JP2 boxes.
pub fn encode_j2k(image: &Image, format: Format, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    codestream(image, format, plan, opts)
}

/// The colour model a component count and `colr` box describe. The extra component beyond
/// a colour model is the opacity channel, which is where `cdef` points at it.
fn layout_for(colorspace: Colorspace, components: u16) -> Result<ChannelLayout, CodecError> {
    let layout = match (colorspace, components) {
        // An ICC-coloured stream does not name its model, so the component count decides.
        (Colorspace::IccTagged, 1) | (Colorspace::Grayscale, 1) | (Colorspace::SGray, 1) => ChannelLayout::Gray,
        (Colorspace::IccTagged, 2) | (Colorspace::Grayscale, 2) | (Colorspace::SGray, 2) => ChannelLayout::GrayA,
        (Colorspace::Cmyk, 4) | (Colorspace::Ycck, 4) => ChannelLayout::Cmyk,
        (Colorspace::Cmyk, 5) | (Colorspace::Ycck, 5) => ChannelLayout::CmykA,
        (_, 1) => ChannelLayout::Gray,
        (_, 2) => ChannelLayout::GrayA,
        (_, 3) => ChannelLayout::Rgb,
        (_, 4) => ChannelLayout::Rgba,
        (_, 5) => ChannelLayout::CmykA,
        (colorspace, n) => {
            return Err(CodecError::unsupported(F, format!("{n}-component JPEG 2000 images ({colorspace:?}) cannot be mapped to a channel layout")));
        }
    };
    Ok(layout)
}

/// 8-bit components decode as U8, wider ones as U16 (a 12-bit stream widens without loss).
fn sample_for(bit_depth: u8) -> Result<SampleType, CodecError> {
    match bit_depth {
        0 => Err(CodecError::malformed(F, "JPEG 2000 SIZ marker declares zero-bit components")),
        1..=8 => Ok(SampleType::U8),
        9..=MAX_SUPPORTED_BITS => Ok(SampleType::U16),
        bits => Err(CodecError::unsupported(F, format!("JPEG 2000 images of {bits} bits per sample are not supported (16 at most)"))),
    }
}

/// The ICC profile of the first method-2 `colr` box, which the decoder reads back as the
/// document's profile.
fn icc_profile(header: &j2k::J2kSupportInfo) -> Option<Vec<u8>> {
    header.file_metadata.as_ref()?.color_specs.iter().find_map(|spec| match spec {
        J2kColorSpec::IccProfile { profile } => Some(profile.clone()),
        _ => None,
    })
}

/// One decoded component, with the arithmetic that turns its raw samples into 0..1.
struct Plane<'a> {
    samples: &'a [f32],
    /// What is added before dividing: zero for an unsigned component, the signed maximum for
    /// a signed one, whose range is lifted to 0..1 rather than clamped into it.
    bias: f32,
    /// `2^bits - 1`, or twice the signed magnitude; never zero.
    divisor: f32,
}

impl<'a> Plane<'a> {
    fn new(plane: &'a j2k::J2kComponentPlane<'a>) -> Self {
        let (bias, divisor) = sample_range(plane.bit_depth(), plane.signed());
        Self { samples: plane.samples(), bias, divisor }
    }

    /// One sample as a 0..1 position. A non-finite value (a decoder bug or a damaged file)
    /// reads as black rather than propagating.
    fn at(&self, index: usize) -> f32 {
        let raw = self.samples.get(index).copied().unwrap_or(0.0);
        if raw.is_finite() { ((raw + self.bias) / self.divisor).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// Interleave the first `layout.channels()` component planes into our native-endian samples.
/// The planes are per component, so this handles every layout the family can describe without
/// asking the codec for an interleaved format that has no gray+alpha or four-colour variant.
fn interleave(planes: &[j2k::J2kComponentPlane<'_>], width: u32, height: u32, layout: ChannelLayout, sample: SampleType) -> Result<Vec<u8>, CodecError> {
    let channels = layout.channels();
    let step = sample.bytes();
    let count = (width as usize)
        .checked_mul(height as usize)
        .filter(|pixels| *pixels > 0)
        .ok_or_else(|| CodecError::InvalidImage("JPEG 2000 image size overflows usize".into()))?;
    let size =
        count.checked_mul(channels).and_then(|n| n.checked_mul(step)).ok_or_else(|| CodecError::InvalidImage("JPEG 2000 image size overflows usize".into()))?;
    let mut bytes = crate::options::alloc_zeroed(size)?;

    // A component's precision does not change inside the loop, so read it once up front.
    let mut sources: Vec<Plane<'_>> = Vec::with_capacity(channels);
    for channel in 0..channels {
        let plane = planes.get(channel).ok_or_else(|| CodecError::malformed(F, "JPEG 2000 decode produced too few components"))?;
        sources.push(Plane::new(plane));
    }

    match sample {
        SampleType::U8 => {
            let full = f32::from(u8::MAX);
            for pixel in 0..count {
                let base = pixel * channels;
                for (channel, plane) in sources.iter().enumerate() {
                    if let Some(slot) = bytes.get_mut(base + channel) {
                        *slot = (plane.at(pixel) * full).round() as u8;
                    }
                }
            }
        }
        SampleType::U16 => {
            let full = f32::from(u16::MAX);
            for pixel in 0..count {
                let base = pixel * channels * step;
                for (channel, plane) in sources.iter().enumerate() {
                    let at = base + channel * step;
                    let value = (plane.at(pixel) * full).round() as u16;
                    if let Some(slot) = bytes.get_mut(at..at + 2) {
                        slot.copy_from_slice(&value.to_ne_bytes());
                    }
                }
            }
        }
        // `sample_for` never returns a float type; the branch keeps the match total.
        SampleType::F16 | SampleType::F32 => return Err(CodecError::unsupported(F, "JPEG 2000 float sample types are not produced by this decoder")),
    }
    Ok(bytes)
}

/// The offset and divisor that scale one component's raw samples into 0..1. A 1-, 4- or
/// 12-bit component is scaled by its own maximum, not by 255 or 65535, so an 8-bit file from a
/// 12-bit source still lands on the full range.
fn sample_range(bit_depth: u8, signed: bool) -> (f32, f32) {
    let bits = u32::from(bit_depth).clamp(1, 24);
    if signed {
        let max = ((1u64 << (bits - 1)) - 1) as f32;
        if max <= 0.0 { (0.0, 1.0) } else { (max, 2.0 * max) }
    } else {
        let max = ((1u64 << bits) - 1) as f32;
        if max <= 0.0 { (0.0, 1.0) } else { (0.0, max) }
    }
}

/// Encode `image` per `plan` into a raw codestream.
fn codestream(image: &Image, format: Format, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = image.converted(plan.layout, plan.sample);
    let (width, height) = img.dimensions();
    let components = u16::try_from(plan.layout.channels()).map_err(|_| CodecError::encode(format, "too many components for JPEG 2000"))?;
    let bit_depth = bits_for(plan.sample, format)?;
    // No quality asked for means the reversible path; see `EncodeOptions::jp2_quality`.
    let quality = opts.jp2_quality.unwrap_or(100).clamp(1, 100);

    // Quality 100 is the reversible path; so is any layout whose colour transform the
    // irreversible path would get wrong, which today means anything but the enumerated models.
    if quality >= 100 || !lossy_transform_is_safe(plan.layout) {
        return lossless(img.as_ref(), format, width, height, components, bit_depth);
    }
    lossy(img.as_ref(), format, width, height, components, bit_depth, quality)
}

/// The irreversible 9/7 path applies Part 1's colour transform, which is defined for exactly
/// three colour components. That is sound for gray, RGB and RGB+alpha (the transform touches
/// the first three components and the fourth is coded independently) but would corrupt inks, so
/// `caps` keeps CMYK out of the family entirely and this is the belt to that braces.
fn lossy_transform_is_safe(layout: ChannelLayout) -> bool {
    layout.is_gray() || layout.is_rgb()
}

fn lossless(image: &Image, format: Format, width: u32, height: u32, components: u16, bit_depth: u8) -> Result<Vec<u8>, CodecError> {
    let samples = J2kLosslessSamples::new(image.data(), width, height, components, bit_depth, false)
        .map_err(|e| CodecError::encode(format, format!("JPEG 2000 samples: {e}")))?;
    let options = J2kLosslessEncodeOptions::default().with_validation(J2kEncodeValidation::CpuRoundTrip);
    encode_j2k_lossless(samples, &options).map(|encoded| encoded.codestream).map_err(|e| CodecError::encode(format, format!("JPEG 2000 lossless encode: {e}")))
}

fn lossy(image: &Image, format: Format, width: u32, height: u32, components: u16, bit_depth: u8, quality: u8) -> Result<Vec<u8>, CodecError> {
    let samples = J2kLossySamples::new(image.data(), width, height, components, bit_depth, false)
        .map_err(|e| CodecError::encode(format, format!("JPEG 2000 samples: {e}")))?;
    // JPEG 2000 has no quality scale of its own, so quality picks a rate the way JPEG's
    // quantisation tables pick one: a curve, near-lossless at the top and a thumbnail at the
    // bottom, as a fraction of the uncompressed size so the scale means the same at any depth.
    let raw_bits_per_pixel = f64::from(components) * f64::from(bit_depth);
    let q = f64::from(quality.clamp(1, 99)) / 100.0;
    let bits_per_pixel = raw_bits_per_pixel * (0.02 + 0.98 * q * q);
    let options =
        J2kLossyEncodeOptions::default().with_rate_target(Some(J2kRateTarget::BitsPerPixel(bits_per_pixel))).with_validation(J2kEncodeValidation::CpuRoundTrip);
    encode_j2k_lossy(samples, &options).map(|encoded| encoded.codestream).map_err(|e| CodecError::encode(format, format!("JPEG 2000 lossy encode: {e}")))
}

/// The significant bits per component of a planned image: 8 or 16. Float plans never reach
/// here (`caps` keeps them at U16), but a direct caller could ask.
fn bits_for(sample: SampleType, format: Format) -> Result<u8, CodecError> {
    match sample {
        SampleType::U8 => Ok(8),
        SampleType::U16 | SampleType::F16 => Ok(16),
        SampleType::F32 => Err(CodecError::unsupported(format, "32-bit float images are not stored by JPEG 2000; they are written at 16 bits")),
    }
}

/// Wrap a codestream in the JP2 boxes: signature, `ftyp`, then a header box holding the
/// colour specification, the alpha channel's `cdef` entry and the ICC profile when asked for.
fn wrap(image: &Image, plan: Plan, codestream: &[u8]) -> Result<Vec<u8>, CodecError> {
    let layout = plan.layout;
    let color: J2kFileColorSpec<'_> = match image.icc.as_deref().filter(|icc| !icc.is_empty()) {
        // A method-2 `colr` box: the ICC profile describes the samples, so no enumerated
        // colour space is written beside it.
        Some(icc) => J2kFileColorSpec::IccProfile(icc),
        // Greyscale plus opacity is two components, which no enumerated colour space covers,
        // so name the colour model (sGray) and let `cdef` mark the second channel.
        _ if layout == ChannelLayout::GrayA => J2kFileColorSpec::Enumerated(Colorspace::SGray),
        _ => J2kFileColorSpec::Infer,
    };
    let defs = channel_definitions(layout);

    let options =
        J2kFileWrapOptions::jp2().with_color(color).with_metadata(J2kFileBoxMetadata { palette: None, component_mappings: &[], channel_definitions: &defs });
    wrap_j2k_codestream(codestream, options).map_err(|e| CodecError::encode(F, format!("JPEG 2000 wrapping: {e}")))
}

/// The `cdef` box entries for a layout with an alpha channel: one per component, as readers
/// require, each colour channel bound to its own one-based index and the last one declared
/// opacity for the whole image. A layout without alpha needs none — the channels are colour.
fn channel_definitions(layout: ChannelLayout) -> Vec<J2kChannelDefinition> {
    if !layout.has_alpha() {
        return Vec::new();
    }
    let color = layout.color_channels();
    let mut defs: Vec<J2kChannelDefinition> = Vec::with_capacity(color + 1);
    for index in 0..color {
        let Some(channel) = u16::try_from(index).ok() else { continue };
        defs.push(J2kChannelDefinition {
            channel_index: channel,
            channel_type: J2kChannelType::Color,
            // The association is the one-based colour channel; the JPEG 2000 limit of 16384
            // components keeps this inside a u16.
            association: J2kChannelAssociation::Color { index: channel.saturating_add(1) },
        });
    }
    if let Ok(alpha) = u16::try_from(color) {
        defs.push(J2kChannelDefinition { channel_index: alpha, channel_type: J2kChannelType::Opacity, association: J2kChannelAssociation::WholeImage });
    }
    defs
}

// ---------------------------------------------------------------------------
// MJ2 sample-table parsing (Part 3 motion containers)
// ---------------------------------------------------------------------------

/// The first sample of an MJ2 presentation: its offset and size within `mdat`, and the total
/// number of samples (frames) when the sample table says so.
struct Mj2FirstSample {
    offset: u64,
    size: u32,
    total_samples: u32,
}

/// Locate the first MJ2 sample and count the frames from `moov`'s sample table.
fn first_mj2_sample(bytes: &[u8]) -> Result<Mj2FirstSample, CodecError> {
    let Some(moov) = top_level_box(bytes, *b"moov") else {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 file holds no moov"));
    };
    // `moov`, `trak`, `mdia`, `minf` and `stbl` are containers, not full boxes.
    let trak = top_level_box(moov.payload, *b"trak").ok_or_else(|| CodecError::malformed(Format::Mj2, "Motion JPEG 2000 moov holds no track"))?;
    let mdia = top_level_box(trak.payload, *b"mdia").ok_or_else(|| CodecError::malformed(Format::Mj2, "Motion JPEG 2000 track holds no media"))?;
    let minf = top_level_box(mdia.payload, *b"minf").ok_or_else(|| CodecError::malformed(Format::Mj2, "Motion JPEG 2000 media holds no information"))?;
    let stbl = top_level_box(minf.payload, *b"stbl").ok_or_else(|| CodecError::malformed(Format::Mj2, "Motion JPEG 2000 moov holds no track sample table"))?;
    // `stbl` itself is not a full box: its payload is the box sequence directly.
    let stbl_kids = &stbl.payload;

    // Sanity: a sample description must declare at least one entry (the `jp2m`/`jpeg` sample).
    let stsd = boxes_of_type(stbl_kids, *b"stsd").into_iter().next();
    let _ = stsd
        .filter(|s| s.len() >= 8 && u32::from_be_bytes([s[4], s[5], s[6], s[7]]) > 0)
        .ok_or_else(|| CodecError::malformed(Format::Mj2, "Motion JPEG 2000 sample table holds no sample description"))?;

    let stco = boxes_of_type(stbl_kids, *b"stco").into_iter().next();
    let co64 = boxes_of_type(stbl_kids, *b"co64").into_iter().next();
    let offsets = match (stco, co64) {
        (Some(s), None) => parse_u32_table(s)?,
        (None, Some(s)) => parse_u64_table(s)?,
        (Some(_), Some(_)) => return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 sample table holds both stco and co64")),
        (None, None) => return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 sample table holds no chunk offsets")),
    };
    if offsets.is_empty() {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 sample table holds no chunks"));
    }
    let first_chunk_offset = offsets[0];

    let stsz = boxes_of_type(stbl_kids, *b"stsz")
        .into_iter()
        .next()
        .ok_or_else(|| CodecError::malformed(Format::Mj2, "Motion JPEG 2000 sample table holds no sample sizes"))?;
    let (uniform_size, per_sample) = parse_stsz(stsz)?;
    let first_size = per_sample.as_ref().and_then(|sizes| sizes.first().copied()).unwrap_or(uniform_size);
    if first_size == 0 {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 first sample has zero size"));
    }

    let total_samples = if let Some(sizes) = &per_sample {
        sizes.len() as u32
    } else if let Some(stts) = boxes_of_type(stbl_kids, *b"stts").into_iter().next() {
        parse_stts_total(stts)?
    } else {
        1
    };

    Ok(Mj2FirstSample { offset: first_chunk_offset, size: first_size, total_samples })
}

/// Parse a full box's `sample_size`/`entry_count` table: `(uniform_size, per_sample_sizes)`.
fn parse_stsz(stsz: &[u8]) -> Result<(u32, Option<Vec<u32>>), CodecError> {
    if stsz.len() < 12 {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 stsz is too short"));
    }
    // Full box: version + flags (4 bytes), then sample_size (u32) and entry_count (u32).
    let sample_size = u32::from_be_bytes([stsz[4], stsz[5], stsz[6], stsz[7]]);
    let entry_count = u32::from_be_bytes([stsz[8], stsz[9], stsz[10], stsz[11]]) as usize;
    if entry_count == 0 {
        return Ok((sample_size, None));
    }
    if sample_size == 0 {
        // Per-sample sizes follow.
        let needed = 12 + entry_count * 4;
        if stsz.len() < needed {
            return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 stsz is truncated"));
        }
        let mut sizes = Vec::with_capacity(entry_count);
        for i in 0..entry_count {
            let base = 12 + i * 4;
            sizes.push(u32::from_be_bytes([stsz[base], stsz[base + 1], stsz[base + 2], stsz[base + 3]]));
        }
        Ok((0, Some(sizes)))
    } else {
        Ok((sample_size, None))
    }
}

/// Parse a `stco` (u32 chunk offsets) or `co64` (u64 chunk offsets) full-box payload into a
/// vector of offsets and whether they came from co64.
fn parse_u32_table(box_payload: &[u8]) -> Result<Vec<u64>, CodecError> {
    if box_payload.len() < 8 {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 chunk-offset table is too short"));
    }
    let entry_count = u32::from_be_bytes([box_payload[4], box_payload[5], box_payload[6], box_payload[7]]) as usize;
    let needed = 8 + entry_count * 4;
    if box_payload.len() < needed {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 chunk-offset table is truncated"));
    }
    let mut offsets = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let base = 8 + i * 4;
        offsets.push(u64::from(u32::from_be_bytes([box_payload[base], box_payload[base + 1], box_payload[base + 2], box_payload[base + 3]])));
    }
    Ok(offsets)
}

fn parse_u64_table(box_payload: &[u8]) -> Result<Vec<u64>, CodecError> {
    if box_payload.len() < 8 {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 co64 table is too short"));
    }
    let entry_count = u32::from_be_bytes([box_payload[4], box_payload[5], box_payload[6], box_payload[7]]) as usize;
    let needed = 8 + entry_count * 8;
    if box_payload.len() < needed {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 co64 table is truncated"));
    }
    let mut offsets = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let base = 8 + i * 8;
        offsets.push(u64::from_be_bytes([
            box_payload[base],
            box_payload[base + 1],
            box_payload[base + 2],
            box_payload[base + 3],
            box_payload[base + 4],
            box_payload[base + 5],
            box_payload[base + 6],
            box_payload[base + 7],
        ]));
    }
    Ok(offsets)
}

/// Total samples from a `stts` (time-to-sample) box: the sum of every entry's sample count.
fn parse_stts_total(stts: &[u8]) -> Result<u32, CodecError> {
    if stts.len() < 8 {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 stts is too short"));
    }
    let entry_count = u32::from_be_bytes([stts[4], stts[5], stts[6], stts[7]]) as usize;
    let needed = 8 + entry_count * 8;
    if stts.len() < needed {
        return Err(CodecError::malformed(Format::Mj2, "Motion JPEG 2000 stts is truncated"));
    }
    let mut total = 0u32;
    for i in 0..entry_count {
        let base = 8 + i * 8;
        let sample_count = u32::from_be_bytes([stts[base], stts[base + 1], stts[base + 2], stts[base + 3]]);
        total = total.saturating_add(sample_count);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic JP2 signature box: 12 bytes, type `jP  `, payload `0D 0A 87 0A`.
    const SIGNATURE: &[u8] = &[0x00, 0x00, 0x00, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

    /// A JPEG 2000 file header: the signature box, then the `ftyp` box that names the
    /// container. Not a decodable image, only enough to identify one.
    fn header(brand: &[u8; 4]) -> Vec<u8> {
        let mut ftyp = 0x14u32.to_be_bytes().to_vec();
        ftyp.extend_from_slice(b"ftyp");
        ftyp.extend_from_slice(brand);
        ftyp.extend_from_slice(&0u32.to_be_bytes());
        ftyp.extend_from_slice(brand);
        SIGNATURE.iter().chain(&ftyp).copied().collect()
    }

    /// A minimal ISO base-media box: `size` + `type` + `payload`, size counting the header.
    fn box_bytes(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8usize + payload.len()) as u32;
        let mut b = size.to_be_bytes().to_vec();
        b.extend_from_slice(&kind);
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn every_container_is_detected_from_its_own_header() {
        for brand in [b"jp2 ", b"jpx ", b"jpf "] {
            assert_eq!(crate::detect(&header(brand)), Some(Format::Jp2), "{brand:?}");
        }
        // Part 6 and Part 3 are recognised as themselves so they can be decoded (first page /
        // frame) rather than refused.
        assert_eq!(crate::detect(&header(b"jpm ")), Some(Format::Jpm));
        assert_eq!(crate::detect(&header(b"mjp2")), Some(Format::Mj2));
        // A signature box with no `ftyp` behind it is still a Part 1 file.
        assert_eq!(crate::detect(SIGNATURE), Some(Format::Jp2));
    }

    #[test]
    fn raw_codestream_is_detected() {
        assert_eq!(crate::detect(&[0xFF, 0x4F, 0xFF, 0x51, 0, 0]), Some(Format::J2k));
    }

    #[test]
    fn next_box_walks_every_top_level_box() {
        let a = box_bytes(*b"abcd", &[1, 2, 3]);
        let b = box_bytes(*b"wxyz", &[]);
        let file = a.iter().chain(&b).copied().collect::<Vec<_>>();
        let mut got = Vec::new();
        let mut offset = 0;
        while let Some((kind, _start, _end, next)) = next_box(&file, offset) {
            got.push(kind);
            offset = next;
        }
        assert_eq!(got, vec![*b"abcd", *b"wxyz"]);
    }

    #[test]
    fn zero_size_box_runs_to_end_and_terminates() {
        // A size-0 box runs to the end of the input; the next walk starts at the end.
        let mut file = 0u32.to_be_bytes().to_vec();
        file.extend_from_slice(b"zero");
        file.extend_from_slice(&[9]);
        file.extend_from_slice(&[0xAA, 0xBB]); // trailing bytes belong to the size-0 box
        let mut kinds = Vec::new();
        let mut offset = 0;
        while let Some((kind, _start, _end, next)) = next_box(&file, offset) {
            kinds.push(kind);
            offset = next;
        }
        assert_eq!(kinds, vec![*b"zero"]);
    }

    #[test]
    fn large_size_box_is_parsed() {
        // size == 1 with a 64-bit total size equal to the real box size.
        let payload = &[7u8; 5];
        let total = 16 + payload.len();
        let mut b = 1u32.to_be_bytes().to_vec();
        b.extend_from_slice(b"big ");
        b.extend_from_slice(&(total as u64).to_be_bytes());
        b.extend_from_slice(payload);
        let (kind, start, end, _next) = next_box(&b, 0).expect("large box");
        assert_eq!(kind, *b"big ");
        assert_eq!(&b[start..end], payload);
    }

    #[test]
    fn malformed_small_size_does_not_loop_forever() {
        // size (2) is smaller than the 8-byte header but the buffer holds the header: the walk
        // must still advance past it and terminate instead of looping.
        let b: &[u8] = &[0, 0, 0, 2, b'x', b'x', 0, 0, 0xAA];
        let mut offset = 0;
        let mut steps = 0;
        while let Some((_kind, _start, _end, next)) = next_box(b, offset) {
            steps += 1;
            offset = next;
        }
        assert_eq!(steps, 1);
        assert_eq!(offset, 8);
        assert!(next_box(b, 8).is_none());
    }

    #[test]
    fn ftyp_brand_reads_the_major_brand() {
        assert_eq!(ftyp_brand(&header(b"jpm ")), Some(*b"jpm "));
        assert_eq!(ftyp_brand(&header(b"mjp2")), Some(*b"mjp2"));
        // A signature box with no ftyp has no brand.
        assert_eq!(ftyp_brand(SIGNATURE), None);
    }

    #[test]
    fn boxes_of_type_collects_in_file_order() {
        let p = box_bytes(*b"page", &[1]);
        let q = box_bytes(*b"page", &[2]);
        let r = box_bytes(*b"jp2c", &[3]);
        let file = SIGNATURE.iter().chain(&p).chain(&q).chain(&r).copied().collect::<Vec<_>>();
        let pages = boxes_of_type(&file, *b"page");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0], &[1]);
        assert_eq!(pages[1], &[2]);
    }

    #[test]
    fn alpha_layouts_get_one_cdef_entry_per_component() {
        let indices = |defs: Vec<J2kChannelDefinition>| defs.iter().map(|d| (d.channel_index, d.channel_type)).collect::<Vec<_>>();
        assert!(channel_definitions(ChannelLayout::Rgb).is_empty(), "no alpha, no cdef");
        assert!(channel_definitions(ChannelLayout::Gray).is_empty());
        assert_eq!(
            indices(channel_definitions(ChannelLayout::Rgba)),
            vec![(0, J2kChannelType::Color), (1, J2kChannelType::Color), (2, J2kChannelType::Color), (3, J2kChannelType::Opacity)]
        );
        assert_eq!(indices(channel_definitions(ChannelLayout::GrayA)), vec![(0, J2kChannelType::Color), (1, J2kChannelType::Opacity)]);
        // A colour channel's association is its one-based index.
        let defs = channel_definitions(ChannelLayout::Rgba);
        assert!(matches!(defs[2].association, J2kChannelAssociation::Color { index: 3 }));
        assert!(matches!(defs[3].association, J2kChannelAssociation::WholeImage));
    }

    #[test]
    fn component_counts_map_to_layouts() {
        assert_eq!(layout_for(Colorspace::SRgb, 3).unwrap(), ChannelLayout::Rgb);
        assert_eq!(layout_for(Colorspace::SRgb, 4).unwrap(), ChannelLayout::Rgba);
        assert_eq!(layout_for(Colorspace::SGray, 1).unwrap(), ChannelLayout::Gray);
        assert_eq!(layout_for(Colorspace::SGray, 2).unwrap(), ChannelLayout::GrayA);
        assert_eq!(layout_for(Colorspace::Cmyk, 4).unwrap(), ChannelLayout::Cmyk);
        assert_eq!(layout_for(Colorspace::Cmyk, 5).unwrap(), ChannelLayout::CmykA);
        assert_eq!(layout_for(Colorspace::IccTagged, 1).unwrap(), ChannelLayout::Gray);
        assert_eq!(layout_for(Colorspace::IccTagged, 3).unwrap(), ChannelLayout::Rgb);
    }

    #[test]
    fn component_counts_outside_the_map_are_unsupported_not_guessed() {
        assert!(layout_for(Colorspace::SRgb, 0).is_err());
        assert!(layout_for(Colorspace::SRgb, 6).is_err());
    }

    #[test]
    fn precisions_pick_the_narrower_sample_type() {
        assert_eq!(sample_for(1).unwrap(), SampleType::U8);
        assert_eq!(sample_for(8).unwrap(), SampleType::U8);
        assert_eq!(sample_for(9).unwrap(), SampleType::U16);
        assert_eq!(sample_for(16).unwrap(), SampleType::U16);
        assert!(sample_for(0).is_err());
        assert!(sample_for(17).is_err());
    }

    #[test]
    fn planes_scale_by_their_own_precision() {
        let scaled = |raw: f32, bits: u8, signed: bool| {
            let (bias, divisor) = sample_range(bits, signed);
            if raw.is_finite() { ((raw + bias) / divisor).clamp(0.0, 1.0) } else { 0.0 }
        };
        assert_eq!(scaled(255.0, 8, false), 1.0);
        assert_eq!(scaled(15.0, 4, false), 1.0);
        assert_eq!(scaled(7.0, 3, false), 1.0);
        assert_eq!(scaled(32767.0, 15, false), 1.0);
        assert_eq!(scaled(65535.0, 16, false), 1.0);
        // Signed: the range lifts into 0..1, so the extremes are the ends.
        assert_eq!(scaled(0.0, 16, true), 0.5);
        assert_eq!(scaled(32767.0, 16, true), 1.0);
        // Hostile samples never escape 0..1, and a non-finite one reads as black.
        assert_eq!(scaled(-5.0, 8, false), 0.0);
        assert_eq!(scaled(f32::NAN, 8, false), 0.0);
        assert_eq!(scaled(f32::INFINITY, 8, false), 0.0);
        assert_eq!(scaled(f32::NEG_INFINITY, 8, false), 0.0);
        assert_eq!(scaled(1.0e9, 8, false), 1.0);
        // A zero- or absurd-precision component divides by one instead of by zero.
        assert_eq!(sample_range(0, false), (0.0, 1.0));
        assert_eq!(sample_range(1, true), (0.0, 1.0));
    }
}
