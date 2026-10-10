//! JP2 and raw J2K/J2C. The adapter keeps native integer precision, while independent
//! container/header preflight bounds the work before the backend allocates sample planes.

mod preflight;

use crate::fidelity::Plan;
use crate::{ChannelLayout as L, CodecError, DecodeWarning, EncodeOptions, Format, Image, Limits, SampleType as S};
use oxideav_jpeg2000::{self as j2k, Container, PixelFormat as P, jp2};

const F: Format = Format::Jpeg2000;
const SIGNATURE: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n";

fn malformed(what: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, what)
}

fn backend_error(e: j2k::Error) -> CodecError {
    match e {
        j2k::Error::LimitExceeded(s) => CodecError::LimitExceeded(s),
        j2k::Error::Unsupported(s) => CodecError::unsupported(F, s),
        j2k::Error::NotImplemented => CodecError::unsupported(F, "JPEG 2000 coding tool is not supported"),
        other => malformed(other),
    }
}

/// A native dependency panic must become an error. Wasm aborts rather than unwinds, so this
/// does not replace the checked preflight or the malformed-input tests on the backend.
fn guarded<T>(f: impl FnOnce() -> Result<T, j2k::Error> + std::panic::UnwindSafe) -> Result<T, CodecError> {
    std::panic::catch_unwind(f).map_err(|_| malformed("JPEG 2000 backend rejected damaged data"))?.map_err(backend_error)
}

struct ContainerInfo<'a> {
    stream: &'a [u8],
    dpi: Option<(f32, f32)>,
    dropped: bool,
    header: Option<HeaderInfo>,
    metadata_bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColourModel {
    Gray,
    Rgb,
}

type ChannelDefinitions = ([(u16, u16, u16); 4], usize);

struct HeaderInfo {
    width: u32,
    height: u32,
    components: u16,
    precision: u8,
    bpcc: Option<([u8; 4], usize)>,
    colour: Option<ColourModel>,
    channels: Option<ChannelDefinitions>,
}

impl HeaderInfo {
    fn validate(&self, info: preflight::Info) -> Result<(), CodecError> {
        if self.width != info.width || self.height != info.height || usize::from(self.components) != info.components {
            return Err(malformed("JP2 image header disagrees with its codestream"));
        }
        let descriptor = info.precision - 1;
        match (self.precision, self.bpcc) {
            (255, Some((depths, count))) if count == info.components && depths.get(..count).is_some_and(|v| v.iter().all(|d| *d == descriptor)) => {}
            (value, None) if value == descriptor => {}
            _ => return Err(malformed("JP2 component precision disagrees with its codestream")),
        }
        let base = if info.components <= 2 { 1 } else { 3 };
        let model = if base == 1 { ColourModel::Gray } else { ColourModel::Rgb };
        if self.colour.is_some_and(|colour| colour != model) {
            return Err(CodecError::unsupported(F, "JP2 colour model does not match its components"));
        }
        if let Some((definitions, count)) = self.channels {
            if count != info.components {
                return Err(CodecError::unsupported(F, "JP2 requires one channel definition per component"));
            }
            let mut seen = 0_u16;
            let mut colours = 0_u16;
            let mut alpha = 0;
            for &(channel, ty, association) in definitions.get(..count).ok_or_else(|| malformed("JP2 channel definitions"))? {
                if usize::from(channel) >= count || seen & (1 << channel) != 0 {
                    return Err(malformed("duplicate or out-of-range JP2 channel"));
                }
                seen |= 1 << channel;
                match ty {
                    0 if association >= 1 && usize::from(association) <= base && colours & (1 << association) == 0 => colours |= 1 << association,
                    1 if association == 0 => alpha += 1,
                    _ => return Err(CodecError::unsupported(F, "JP2 channel definitions are not Gray/RGB with straight alpha")),
                }
            }
            let expected_colours = ((1_u16 << (base + 1)) - 1) & !1;
            if colours != expected_colours || alpha != info.components - base {
                return Err(CodecError::unsupported(F, "JP2 colour/alpha channel count is not supported"));
            }
        }
        Ok(())
    }
}

fn metadata_budget(bytes: &mut u64, length: usize, limits: &Limits) -> Result<(), CodecError> {
    // The backend copies ICC/compatibility entries and allocates box bookkeeping.
    // Include that overhead before allowing it to parse any container metadata.
    let extra = u64::try_from(length).ok().and_then(|n| n.checked_mul(8)).and_then(|n| n.checked_add(256));
    *bytes = extra.and_then(|n| bytes.checked_add(n)).ok_or_else(|| CodecError::LimitExceeded("JP2 metadata allocation overflows".into()))?;
    if *bytes > limits.max_alloc || *bytes > isize::MAX as u64 {
        return Err(CodecError::LimitExceeded("JP2 header and metadata exceed max_alloc".into()));
    }
    Ok(())
}

fn colour_model(data: &[u8]) -> Result<Option<ColourModel>, CodecError> {
    match data.first().copied().ok_or_else(|| malformed("JP2 colour box"))? {
        1 if data.len() == 7 => match data.get(3..7) {
            Some([0, 0, 0, 16]) => Ok(Some(ColourModel::Rgb)),
            Some([0, 0, 0, 17]) => Ok(Some(ColourModel::Gray)),
            _ => Err(CodecError::unsupported(F, "JPEG 2000 colour space is not Gray or RGB")),
        },
        1 => Err(malformed("JP2 enumerated colour box must contain seven bytes")),
        2 | 3 if data.len() >= 4 => match data.get(19..23) {
            Some(b"RGB ") => Ok(Some(ColourModel::Rgb)),
            Some(b"GRAY") => Ok(Some(ColourModel::Gray)),
            Some(b"XYZ " | b"Lab " | b"Luv " | b"YCbr" | b"Yxy " | b"HSV " | b"HLS " | b"CMYK" | b"CMY ") => {
                Err(CodecError::unsupported(F, "JPEG 2000 ICC colour model is not Gray or RGB"))
            }
            Some(sig) if sig.get(1..) == Some(b"CLR") && sig.first().is_some_and(|v| matches!(v, b'2'..=b'9' | b'A'..=b'F')) => {
                Err(CodecError::unsupported(F, "JPEG 2000 multichannel ICC profiles are not supported"))
            }
            // Profiles are opaque codec metadata. The CMS validates them on document import.
            _ => Ok(None),
        },
        2 | 3 => Err(malformed("empty JP2 ICC profile")),
        _ => Err(CodecError::unsupported(F, "JPEG 2000 parameterized colour space is not supported")),
    }
}

/// One checked box view. Extended sizes and final length-zero boxes are supported.
fn next_box<'a>(rest: &mut &'a [u8]) -> Result<([u8; 4], &'a [u8]), CodecError> {
    let header = rest.get(..8).ok_or_else(|| malformed("truncated JP2 box header"))?;
    let size = u32::from_be_bytes(header.get(..4).and_then(|b| b.try_into().ok()).ok_or_else(|| malformed("JP2 box size"))?);
    let kind = header.get(4..8).and_then(|b| b.try_into().ok()).ok_or_else(|| malformed("JP2 box type"))?;
    let (len, start) = match size {
        0 => (rest.len(), 8),
        1 => {
            let n = u64::from_be_bytes(rest.get(8..16).and_then(|b| b.try_into().ok()).ok_or_else(|| malformed("truncated JP2 extended size"))?);
            (usize::try_from(n).map_err(|_| CodecError::LimitExceeded("JP2 box length does not fit in memory".into()))?, 16)
        }
        n => (n as usize, 8),
    };
    if len < start {
        return Err(malformed("JP2 box is smaller than its header"));
    }
    let body = rest.get(start..len).ok_or_else(|| malformed("truncated JP2 box"))?;
    *rest = rest.get(len..).ok_or_else(|| malformed("JP2 box length"))?;
    Ok((kind, body))
}

fn resolution(body: &[u8]) -> Result<(f32, f32), CodecError> {
    if body.len() != 10 {
        return Err(malformed("JP2 capture resolution must contain ten bytes"));
    }
    let word = |at| -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes(body.get(at..at + 2).and_then(|v| v.try_into().ok()).ok_or_else(|| malformed("JP2 resolution"))?))
    };
    let (vn, vd, hn, hd) = (word(0)?, word(2)?, word(4)?, word(6)?);
    let (ve, he) = (
        body.get(8).copied().ok_or_else(|| malformed("JP2 resolution exponent"))? as i8,
        body.get(9).copied().ok_or_else(|| malformed("JP2 resolution exponent"))? as i8,
    );
    if vd == 0 || hd == 0 {
        return Err(malformed("JP2 resolution denominator is zero"));
    }
    let x = (f64::from(hn) / f64::from(hd) * 10_f64.powi(i32::from(he)) * 0.0254) as f32;
    let y = (f64::from(vn) / f64::from(vd) * 10_f64.powi(i32::from(ve)) * 0.0254) as f32;
    if !x.is_finite() || !y.is_finite() || x <= 0.0 || y <= 0.0 {
        return Err(malformed("JP2 resolution is not finite and positive"));
    }
    Ok((x, y))
}

/// Walk boxes without allocating. In particular, refuse a palette before the backend parses
/// its table and account for all header/metadata bytes before it copies ICC/channel tables.
fn container<'a>(bytes: &'a [u8], limits: &Limits) -> Result<ContainerInfo<'a>, CodecError> {
    if !bytes.starts_with(SIGNATURE) {
        return Ok(ContainerInfo { stream: bytes, dpi: None, dropped: false, header: None, metadata_bytes: 0 });
    }
    let mut rest = bytes.get(12..).ok_or_else(|| malformed("JP2 signature"))?;
    let (mut stream, mut compatible, mut header, mut dpi, mut dropped) = (None, false, None, None, false);
    let mut ftyp_seen = false;
    let mut metadata_bytes = 0_u64;
    let mut boxes = 0_u64;
    while !rest.is_empty() {
        boxes = boxes.saturating_add(1);
        if boxes > 100_000 {
            return Err(CodecError::LimitExceeded("too many JP2 boxes".into()));
        }
        let (kind, body) = next_box(&mut rest)?;
        if boxes == 1 && &kind != b"ftyp" {
            return Err(malformed("JP2 file-type box must follow its signature"));
        }
        match &kind {
            b"ftyp" => {
                if ftyp_seen {
                    return Err(malformed("duplicate JP2 file-type box"));
                }
                ftyp_seen = true;
                metadata_budget(&mut metadata_bytes, body.len(), limits)?;
                let brands = body.get(8..).ok_or_else(|| malformed("JP2 file-type box"))?;
                if brands.len() % 4 != 0 {
                    return Err(malformed("JP2 compatibility list"));
                }
                compatible = brands.as_chunks::<4>().0.iter().any(|b| b == b"jp2 ");
            }
            b"jp2c" => {
                if header.is_none() {
                    return Err(malformed("JP2 image header must precede its codestream"));
                }
                if stream.replace(body).is_some() {
                    return Err(CodecError::unsupported(F, "multiple JPEG 2000 codestreams are not supported"));
                }
            }
            b"jp2h" => {
                if header.is_some() {
                    return Err(malformed("duplicate JP2 image header"));
                }
                metadata_budget(&mut metadata_bytes, body.len(), limits)?;
                let mut children = body;
                let mut image_header = None;
                let (mut bpcc, mut colour, mut channels, mut resolution_seen) = (None, None, None, false);
                let mut colour_seen = false;
                let mut first = true;
                while !children.is_empty() {
                    let (ty, data) = next_box(&mut children)?;
                    metadata_budget(&mut metadata_bytes, 0, limits)?;
                    if first && &ty != b"ihdr" {
                        return Err(malformed("JP2 image header must be its first header box"));
                    }
                    first = false;
                    match &ty {
                        b"ihdr" => {
                            if image_header.is_some() || data.len() != 14 {
                                return Err(malformed("duplicate or malformed JP2 image header"));
                            }
                            let number = |at| -> Result<u32, CodecError> {
                                Ok(u32::from_be_bytes(data.get(at..at + 4).and_then(|v| v.try_into().ok()).ok_or_else(|| malformed("JP2 image dimensions"))?))
                            };
                            let count = u16::from_be_bytes(data.get(8..10).and_then(|v| v.try_into().ok()).ok_or_else(|| malformed("JP2 image components"))?);
                            image_header = Some((number(4)?, number(0)?, count, data.get(10).copied().ok_or_else(|| malformed("JP2 image precision"))?));
                        }
                        b"bpcc" => {
                            if bpcc.is_some() || data.is_empty() || data.len() > 4 {
                                return Err(malformed("duplicate or invalid JP2 precision table"));
                            }
                            let mut depths = [0; 4];
                            depths.get_mut(..data.len()).ok_or_else(|| malformed("JP2 component precisions"))?.copy_from_slice(data);
                            bpcc = Some((depths, data.len()));
                        }
                        b"pclr" | b"cmap" => return Err(CodecError::unsupported(F, "JPEG 2000 palettes and channel mappings are not supported")),
                        b"colr" => {
                            if colour_seen {
                                return Err(CodecError::unsupported(F, "multiple JP2 colour specifications are not supported"));
                            }
                            colour_seen = true;
                            colour = colour_model(data)?;
                        }
                        b"cdef" => {
                            let count = u16::from_be_bytes(data.get(..2).and_then(|v| v.try_into().ok()).ok_or_else(|| malformed("JP2 channel definitions"))?);
                            if channels.is_some() || count > 4 || data.len() != 2 + usize::from(count) * 6 {
                                return Err(malformed("JP2 channel definition count"));
                            }
                            let mut definitions = [(0, 0, 0); 4];
                            for (slot, entry) in definitions.iter_mut().zip(data.get(2..).ok_or_else(|| malformed("JP2 channels"))?.as_chunks::<6>().0) {
                                let word = |at| -> Result<u16, CodecError> {
                                    Ok(u16::from_be_bytes(
                                        entry.get(at..at + 2).and_then(|v| v.try_into().ok()).ok_or_else(|| malformed("JP2 channel definition"))?,
                                    ))
                                };
                                *slot = (word(0)?, word(2)?, word(4)?);
                            }
                            channels = Some((definitions, usize::from(count)));
                        }
                        b"res " => {
                            if resolution_seen {
                                return Err(malformed("duplicate JP2 resolution box"));
                            }
                            resolution_seen = true;
                            let mut children = data;
                            while !children.is_empty() {
                                let (ty, data) = next_box(&mut children)?;
                                metadata_budget(&mut metadata_bytes, 0, limits)?;
                                if &ty == b"resc" {
                                    if dpi.is_some() {
                                        return Err(malformed("duplicate JP2 capture resolution"));
                                    }
                                    dpi = Some(resolution(data)?);
                                }
                            }
                        }
                        b"xml " | b"uuid" => dropped = true,
                        _ => {}
                    }
                }
                if !colour_seen {
                    return Err(malformed("missing JP2 colour specification"));
                }
                let (width, height, components, precision) = image_header.ok_or_else(|| malformed("missing JP2 image header"))?;
                header = Some(HeaderInfo { width, height, components, precision, bpcc, colour, channels });
            }
            b"xml " | b"uuid" | b"jp2i" => dropped = true,
            _ => {}
        }
    }
    if !compatible {
        return Err(CodecError::unsupported(F, "JPX/JPH requires JP2 compatibility"));
    }
    if header.is_none() {
        return Err(malformed("missing JP2 image header"));
    }
    Ok(ContainerInfo { stream: stream.ok_or_else(|| malformed("missing JP2 codestream"))?, dpi, dropped, header, metadata_bytes })
}

fn layout(format: P) -> Result<(L, S), CodecError> {
    Ok(match format {
        P::Gray8 => (L::Gray, S::U8),
        P::Gray10Le | P::Gray12Le | P::Gray16Le => (L::Gray, S::U16),
        P::Ya8 => (L::GrayA, S::U8),
        P::Ya16Le => (L::GrayA, S::U16),
        P::Rgb24 => (L::Rgb, S::U8),
        P::Rgb48Le => (L::Rgb, S::U16),
        P::Rgba => (L::Rgba, S::U8),
        P::Rgba64Le => (L::Rgba, S::U16),
        _ => return Err(CodecError::unsupported(F, "JPEG 2000 layout is not Gray/RGB with optional alpha")),
    })
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let container = container(bytes, limits)?;
    let max_alloc = limits
        .max_alloc
        .min(isize::MAX as u64)
        .checked_sub(container.metadata_bytes)
        .ok_or_else(|| CodecError::LimitExceeded("JP2 metadata exhausts the decode budget".into()))?;
    let info = preflight::check(container.stream, &Limits { max_alloc, ..*limits })?;
    if let Some(header) = &container.header {
        header.validate(info)?;
    }
    let options = j2k::DecodeOptions::new()
        .with_max_width(limits.max_width)
        .with_max_height(limits.max_height)
        .with_max_pixels(limits.max_pixels)
        .with_max_bytes(limits.max_alloc)
        .with_strict(true);
    let mut decoded = guarded(|| j2k::decode_with(bytes, &options))?;
    let (layout, sample) = layout(decoded.format)?;
    if decoded.width != info.width
        || decoded.height != info.height
        || decoded.bit_depth != info.precision
        || layout.channels() != info.components
        || decoded.planes.len() != 1
    {
        return Err(malformed("decoded JPEG 2000 image disagrees with its header"));
    }
    limits.check(decoded.width, decoded.height, layout, sample)?;
    let row_bytes = (decoded.width as usize)
        .checked_mul(layout.channels())
        .and_then(|v| v.checked_mul(sample.bytes()))
        .ok_or_else(|| CodecError::LimitExceeded("JPEG 2000 row length overflows".into()))?;
    let plane = decoded.planes.pop().ok_or_else(|| malformed("missing JPEG 2000 sample plane"))?;
    let expected = row_bytes.checked_mul(decoded.height as usize).ok_or_else(|| CodecError::LimitExceeded("JPEG 2000 image length overflows".into()))?;
    if plane.stride != row_bytes || plane.data.len() != expected {
        return Err(malformed("JPEG 2000 sample buffer has an unexpected stride or length"));
    }
    let mut data = plane.data;
    let max = (1_u32 << info.precision) - 1;
    if sample == S::U8 {
        for v in &mut data {
            if u32::from(*v) > max {
                return Err(malformed("JPEG 2000 sample exceeds its precision"));
            }
            *v = ((u32::from(*v) * 255 + max / 2) / max) as u8;
        }
    } else {
        for word in data.as_chunks_mut::<2>().0 {
            let value = u16::from_le_bytes(*word);
            if u32::from(value) > max {
                return Err(malformed("JPEG 2000 sample exceeds its precision"));
            }
            let scaled = ((u32::from(value) * 65535 + max / 2) / max) as u16;
            word.copy_from_slice(&scaled.to_ne_bytes());
        }
    }
    let mut image = Image::from_raw(decoded.width, decoded.height, layout, sample, data)?;
    image.icc = decoded.metadata.icc;
    image.meta.dpi = container.dpi;
    if container.dropped {
        image.warnings.push(DecodeWarning::MetadataDropped { format: F });
    }
    Ok(image)
}

fn copy_bytes(bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len()).map_err(|e| CodecError::LimitExceeded(format!("JPEG 2000 sample allocation: {e}")))?;
    out.extend_from_slice(bytes);
    Ok(out)
}

fn grid_axis(dpi: f32) -> Result<(u16, u16, i8), CodecError> {
    if !dpi.is_finite() || dpi <= 0.0 {
        return Err(CodecError::encode(F, "resolution must be finite and positive"));
    }
    let mut value = f64::from(dpi) / 0.0254;
    let mut exponent = 0_i8;
    while value > 65535.0 {
        value /= 10.0;
        exponent = exponent.checked_add(1).ok_or_else(|| CodecError::encode(F, "resolution exponent exceeds JP2"))?;
    }
    while value < 1.0 {
        value *= 10.0;
        exponent = exponent.checked_sub(1).ok_or_else(|| CodecError::encode(F, "resolution exponent exceeds JP2"))?;
    }
    let denominator = (65535.0 / value).floor().clamp(1.0, 65535.0) as u16;
    let numerator = (value * f64::from(denominator)).round().clamp(1.0, 65535.0) as u16;
    Ok((numerator, denominator, exponent))
}

pub(crate) fn encode(image: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    if opts.jpeg2000_quality.is_some_and(|q| !(1..=100).contains(&q)) {
        return Err(CodecError::encode(F, "JPEG 2000 quality must be 1..=100"));
    }
    let converted = image.converted(plan.layout, plan.sample);
    let format = match (plan.layout, plan.sample) {
        (L::Gray, S::U8) => P::Gray8,
        (L::GrayA, S::U8) => P::Ya8,
        (L::Rgb, S::U8) => P::Rgb24,
        (L::Rgba, S::U8) => P::Rgba,
        (L::Gray, S::U16) => P::Gray16Le,
        (L::GrayA, S::U16) => P::Ya16Le,
        (L::Rgb, S::U16) => P::Rgb48Le,
        (L::Rgba, S::U16) => P::Rgba64Le,
        _ => return Err(CodecError::unsupported(F, "JPEG 2000 requires integer Gray/RGB samples")),
    };
    let mut data = copy_bytes(converted.data())?;
    if plan.sample == S::U16 {
        for word in data.as_chunks_mut::<2>().0 {
            let v = u16::from_ne_bytes(*word);
            word.copy_from_slice(&v.to_le_bytes());
        }
    }
    let packed = j2k::Jpeg2000Image::packed(image.width(), image.height(), format, data).map_err(|e| CodecError::encode(F, e))?;
    let mut options = j2k::EncodeOptions::new().with_container(Container::J2k);
    if let Some(q) = opts.jpeg2000_quality {
        // One encoding pass with rate allocation; a PSNR search would repeatedly decode
        // full-size candidates. This is our byte-budget policy, not libjpeg's scale.
        let squared = usize::from(q).pow(2);
        let len = converted.data().len();
        // Split before multiplying so the intermediate cannot overflow for q <= 100.
        let target = (len / 10_000) * squared + (len % 10_000) * squared / 10_000;
        let target = target.checked_add(512).ok_or_else(|| CodecError::encode(F, "JPEG 2000 byte budget overflow"))?;
        options = options.with_lossy(6).with_target_bytes(target);
    }
    let stream = std::panic::catch_unwind(|| j2k::encode(&packed, &options))
        .map_err(|_| CodecError::encode(F, "JPEG 2000 backend rejected the image"))?
        .map_err(|e| CodecError::encode(F, e))?;
    if opts.jpeg2000_codestream {
        return Ok(stream);
    }
    let mut jp2_options = jp2::Jp2WriteOptions::for_components(plan.layout.channels());
    if opts.embed_icc
        && let Some(icc) = converted.icc.as_deref()
    {
        jp2_options.colour = vec![jp2::Colr {
            method: jp2::ColrMethod::RestrictedIccProfile,
            precedence: 0,
            approximation: 0,
            enumerated: None,
            icc_profile: Some(copy_bytes(icc)?),
            parameterized: None,
        }];
    }
    if opts.embed_metadata
        && let Some((x, y)) = image.meta.dpi
    {
        let (hn, hd, he) = grid_axis(x)?;
        let (vn, vd, ve) = grid_axis(y)?;
        jp2_options.resolution = Some(jp2::Resolution {
            capture: Some(jp2::GridResolution {
                vertical_numerator: vn,
                vertical_denominator: vd,
                horizontal_numerator: hn,
                horizontal_denominator: hd,
                vertical_exponent: ve,
                horizontal_exponent: he,
            }),
            display: None,
        });
    }
    guarded(|| jp2::write_jp2(&stream, &jp2_options))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror the production acceptance gate, then deliberately bypass guarded().
    /// An escaped backend panic must fail this regression rather than become a
    /// Malformed error; the native unwind fallback is unavailable on wasm.
    fn accepted_backend_result(bytes: &[u8], limits: &Limits, context: &str) -> Option<bool> {
        let view = container(bytes, limits).ok()?;
        let max_alloc = limits.max_alloc.min(isize::MAX as u64).checked_sub(view.metadata_bytes)?;
        let info = preflight::check(view.stream, &Limits { max_alloc, ..*limits }).ok()?;
        if let Some(header) = &view.header {
            header.validate(info).ok()?;
        }
        let options = j2k::DecodeOptions::new()
            .with_max_width(limits.max_width)
            .with_max_height(limits.max_height)
            .with_max_pixels(limits.max_pixels)
            .with_max_bytes(limits.max_alloc)
            .with_strict(true);
        let result = std::panic::catch_unwind(|| j2k::decode_with(bytes, &options));
        assert!(result.is_ok(), "accepted JPEG 2000 input panicked in the backend: {context}");
        Some(result.unwrap().is_ok())
    }

    fn small_image(layout: L, sample: S) -> Image {
        let mut values: Vec<u16> = (0..15 * layout.channels()).map(|i| (i as u16).wrapping_mul(11_743).wrapping_add(53)).collect();
        values[..4].copy_from_slice(&[0, u16::MAX, 1, 32_769]);
        let mut image = if sample == S::U16 {
            Image::from_u16(5, 3, layout, &values).unwrap()
        } else {
            Image::from_u8(5, 3, layout, values.into_iter().map(|v| v as u8).collect()).unwrap()
        };
        // Exercise ICC and nested resolution boxes in the container probes.
        let mut icc = vec![0; 64];
        icc[16..20].copy_from_slice(if layout == L::Gray { b"GRAY" } else { b"RGB " });
        image.icc = Some(icc);
        image.meta.dpi = Some((72.0, 144.0));
        image
    }

    #[test]
    fn accepted_mutations_and_entropy_truncations_do_not_panic_in_the_backend() {
        // Eight tiny public-encoder fixtures, with exactly 256 deterministic
        // probes each. The small dimensions and working budget bound runtime
        // even when mutated header fields request much larger packet plans.
        let limits = Limits { max_width: 32, max_height: 32, max_pixels: 1_024, max_alloc: 1 << 20 };
        for layout in [L::Gray, L::Rgba] {
            for sample in [S::U8, S::U16] {
                let image = small_image(layout, sample);
                for raw in [false, true] {
                    let fixture = crate::encode(&image, F, &EncodeOptions { jpeg2000_codestream: raw, ..Default::default() }).unwrap();
                    let name = format!("{layout:?}/{sample:?}/raw={raw}");
                    assert_eq!(accepted_backend_result(&fixture, &limits, &name), Some(true), "valid fixture: {name}");
                    let view = container(&fixture, &limits).unwrap();
                    // The public encoder puts its normal-size jp2c box last.
                    // These offsets are derived only from our valid fixture.
                    let stream_start = fixture.len() - view.stream.len();
                    assert_eq!(&fixture[stream_start..], view.stream);
                    if !raw {
                        assert_eq!(&fixture[stream_start - 4..stream_start], b"jp2c");
                    }
                    let sot = stream_start + view.stream.windows(2).position(|v| v == [0xff, 0x90]).unwrap();
                    let body = stream_start + view.stream.windows(2).position(|v| v == [0xff, 0x93]).unwrap() + 2;
                    let end = fixture.len() - 2;
                    assert_eq!(&fixture[end..], &[0xff, 0xd9]);
                    assert!(body < end);
                    let (mut accepted, mut rejected_by_backend) = (0, 0);
                    for probe in 0..192 {
                        let mut changed = fixture.clone();
                        let position = (probe * 73 + probe / 8 * 11) % changed.len();
                        changed[position] ^= [1, 0x80, 0xff, 0x55][probe % 4];
                        let context = format!("{name}, mutation {probe} at byte {position}");
                        if let Some(success) = accepted_backend_result(&changed, &limits, &context) {
                            accepted += 1;
                            rejected_by_backend += usize::from(!success);
                        }
                    }
                    for probe in 0..64 {
                        // Keep SIZ/COD, box framing, Psot and EOC valid while
                        // cutting entropy data. Ordinary prefix truncations
                        // fail preflight and never exercise the backend.
                        let cut = body + (end - body) * probe / 64;
                        let mut changed = fixture[..cut].to_vec();
                        changed.extend_from_slice(&[0xff, 0xd9]);
                        let psot = u32::try_from(cut - sot).unwrap();
                        changed[sot + 6..sot + 10].copy_from_slice(&psot.to_be_bytes());
                        if !raw {
                            let box_start = stream_start - 8;
                            let size = u32::try_from(changed.len() - box_start).unwrap();
                            changed[box_start..box_start + 4].copy_from_slice(&size.to_be_bytes());
                        }
                        let context = format!("{name}, entropy truncation {probe} at byte {cut}");
                        let success = accepted_backend_result(&changed, &limits, &context).expect("repaired entropy truncation passes the header gate");
                        accepted += 1;
                        rejected_by_backend += usize::from(!success);
                    }
                    assert!(accepted > 64, "mutation probes must also reach the backend: {name}");
                    assert!(rejected_by_backend > 0, "exercise accepted damaged input, not just successful decodes: {name}");
                }
            }
        }
    }
}
