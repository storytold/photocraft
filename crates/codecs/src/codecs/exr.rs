//! OpenEXR via the pure-Rust `exr` crate: F16/F32, Y/YA/RGB/RGBA channels,
//! lossless compression. Multi-part files (a Maya/Arnold render writes one part
//! per AOV) open the part that looks most like a colour image, with a warning
//! naming how many parts exist; [`info`] lists every part and [`decode_part`]
//! decodes a chosen one. Pixel data is taken from the data window.

use std::io::Cursor;

use exr::meta::MetaData;
use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Layer, LayerAttributes, LineOrder, ReadChannels, ReadLayers, WritableImage, read,
};
use half::f16;

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, ExrChannelInfo, ExrPartInfo, Image, SampleType, base_channel_name};
use crate::options::{EncodeOptions, ExrCompression, Limits};

const F: Format = Format::OpenExr;

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

/// An `exr`-crate error, with its `NotSupported` kind kept distinct from malformed data
/// (the same split the TIFF codec makes), so unsupported features say so.
fn map_exr(e: exr::error::Error) -> CodecError {
    match e {
        exr::error::Error::NotSupported(message) => CodecError::unsupported(F, format!("not supported: {message}")),
        e => err(e),
    }
}

/// Read the headers without the `exr` crate's validation (it rejects deep parts outright),
/// after checking every part's declared size against the limits, before any pixel allocation.
pub(super) fn read_meta(bytes: &[u8], limits: &Limits) -> Result<MetaData, CodecError> {
    let meta = MetaData::read_from_buffered(Cursor::new(bytes), false).map_err(err)?;
    // Flat decoding may hold a source part and its display-window output at once. Admit
    // both allocations before pixel decoding, and cap the declared display canvas too.
    let mut total: u64 = 0;
    for header in meta.headers.iter() {
        let size = header.layer_size;
        let (w, h) = (u32::try_from(size.0).unwrap_or(u32::MAX), u32::try_from(size.1).unwrap_or(u32::MAX));
        let bpp = header.channels.list.len().max(1) as u64 * 4;
        limits.check_bytes(w, h, bpp)?;
        let display = header.shared_attributes.display_window;
        let (dw, dh) = (u32::try_from(display.size.0).unwrap_or(u32::MAX), u32::try_from(display.size.1).unwrap_or(u32::MAX));
        // The output is at most RGBA F32, regardless of the source's channel count.
        limits.check_bytes(dw, dh, 16)?;
        let source_bytes = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(bpp);
        // Image::from_f32/from_f16 owns a copy while the staging Vec is still alive.
        // Only one part is decoded per request, so cap the worst part's peak, not the
        // sum of the output stages of every part in a multi-part file.
        let peak_bytes = source_bytes.saturating_add(u64::from(dw).saturating_mul(u64::from(dh)).saturating_mul(32));
        total = total.max(peak_bytes);
    }
    if total > limits.max_alloc {
        return Err(CodecError::LimitExceeded(format!("{total} bytes across {} parts exceed max_alloc {}", meta.headers.len(), limits.max_alloc)));
    }
    Ok(meta)
}

/// Header facts of one part, in file order.
fn part_info(index: usize, header: &exr::meta::header::Header) -> ExrPartInfo {
    let (w, h) = (u32::try_from(header.layer_size.0).unwrap_or(u32::MAX), u32::try_from(header.layer_size.1).unwrap_or(u32::MAX));
    let sample = |t: exr::meta::attribute::SampleType| match t {
        exr::meta::attribute::SampleType::F16 => SampleType::F16,
        // U32 channels (Arnold ID buffers) are listed as F32: exact up to 2^24.
        exr::meta::attribute::SampleType::F32 | exr::meta::attribute::SampleType::U32 => SampleType::F32,
    };
    ExrPartInfo {
        index,
        name: header.own_attributes.layer_name.as_ref().map(|n| n.to_string()),
        view: header.own_attributes.view_name.as_ref().map(|v| v.to_string()),
        width: w,
        height: h,
        deep: header.deep,
        tiled: matches!(header.blocks, exr::meta::BlockDescription::Tiles(_)),
        channels: header.channels.list.iter().map(|c| ExrChannelInfo { name: c.name.to_string(), sample: sample(c.sample_type) }).collect(),
    }
}

/// Lists every part of the file (header facts only, no pixel data): what a multi-part
/// chooser offers, and what [`decode_part`] takes an index into.
pub(crate) fn info(bytes: &[u8], limits: &Limits) -> Result<Vec<ExrPartInfo>, CodecError> {
    let meta = read_meta(bytes, limits)?;
    Ok(meta.headers.iter().enumerate().map(|(i, h)| part_info(i, h)).collect())
}

/// The part plain [`decode`] opens: the highest [`ExrPartInfo::color_rank`], earliest wins.
fn pick_part(meta: &MetaData) -> usize {
    let mut best = 0;
    let mut best_rank = meta.headers.first().map_or(0, |h| part_info(0, h).color_rank());
    for (i, header) in meta.headers.iter().enumerate().skip(1) {
        let rank = part_info(i, header).color_rank();
        if rank > best_rank {
            best = i;
            best_rank = rank;
        }
    }
    best
}

/// Selects one part for the `exr` crate's first-valid-layer reader: `create_channels_reader`
/// is asked about the headers in file order and the first `Ok` wins, so every header before
/// `index` (and, harmlessly, those after it) is declined. Only the chosen part's chunks are
/// then decompressed and only its planes allocated; the other parts are skipped unread.
struct OnlyPart<C> {
    inner: C,
    index: usize,
    seen: std::cell::Cell<usize>,
}

impl<'s, C: ReadChannels<'s>> ReadChannels<'s> for OnlyPart<C> {
    type Reader = C::Reader;

    fn create_channels_reader(&'s self, header: &exr::meta::header::Header) -> exr::error::Result<Self::Reader> {
        let position = self.seen.get();
        self.seen.set(position.saturating_add(1));
        if position == self.index { self.inner.create_channels_reader(header) } else { Err(exr::error::Error::Invalid("part not requested".into())) }
    }
}

/// Decode one part's flat layer via the `exr` crate (it handles every flat compression).
/// Other parts are not decompressed, so an AOV file costs one part per request, not the file.
pub(super) fn read_layer(bytes: &[u8], index: usize) -> Result<Layer<AnyChannels<FlatSamples>>, CodecError> {
    let only = OnlyPart { inner: read().no_deep_data().largest_resolution_level().all_channels(), index, seen: std::cell::Cell::new(0) };
    only.first_valid_layer().all_attributes().from_buffered(Cursor::new(bytes)).map(|image| image.layer_data).map_err(map_exr)
}

fn layer_to_image(layer: &Layer<AnyChannels<FlatSamples>>, header: &exr::meta::header::Header) -> Result<Image, CodecError> {
    let (w, h) = (layer.size.0, layer.size.1);
    let channels = &layer.channel_data.list;
    let find = |n: &str| channels.iter().find(|c| base_channel_name(&c.name.to_string()) == n);
    let (color, layout): (Vec<_>, ChannelLayout) = if let (Some(r), Some(g), Some(b)) = (find("R"), find("G"), find("B")) {
        match find("A") {
            Some(a) => (vec![r, g, b, a], ChannelLayout::Rgba),
            None => (vec![r, g, b], ChannelLayout::Rgb),
        }
    } else if let Some(y) = find("Y").or_else(|| (channels.len() == 1).then(|| &channels[0])) {
        match find("A") {
            Some(a) => (vec![y, a], ChannelLayout::GrayA),
            None => (vec![y], ChannelLayout::Gray),
        }
    } else {
        return Err(CodecError::unsupported(F, "no R/G/B or Y channels"));
    };
    let all_f16 = color.iter().all(|c| matches!(c.sample_data, FlatSamples::F16(_)));
    let sample = if all_f16 { SampleType::F16 } else { SampleType::F32 };
    let npx = w.checked_mul(h).ok_or_else(|| err("EXR pixel count overflows usize"))?;
    if color.iter().any(|c| c.sample_data.len() != npx) {
        return Err(err("channel sample count mismatch (subsampled channels are unsupported)"));
    }
    let nc = color.len();
    let display = header.shared_attributes.display_window;
    let display_w = u32::try_from(display.size.0).map_err(|_| CodecError::LimitExceeded("EXR display-window width exceeds u32".into()))?;
    let display_h = u32::try_from(display.size.1).map_err(|_| CodecError::LimitExceeded("EXR display-window height exceeds u32".into()))?;
    // Compute exclusive bounds in i64. IntegerBounds::end() converts the size to i32 and can
    // panic for hostile headers, so do not use its convenience methods here.
    let display_x0 = i64::from(display.position.0);
    let display_y0 = i64::from(display.position.1);
    let data_x0 = i64::from(layer.attributes.layer_position.0);
    let data_y0 = i64::from(layer.attributes.layer_position.1);
    let display_x1 = display_x0.checked_add(i64::from(display_w)).ok_or_else(|| err("EXR display-window x bound overflows"))?;
    let display_y1 = display_y0.checked_add(i64::from(display_h)).ok_or_else(|| err("EXR display-window y bound overflows"))?;
    let data_x1 =
        data_x0.checked_add(i64::try_from(w).map_err(|_| err("EXR data-window width exceeds i64"))?).ok_or_else(|| err("EXR data-window x bound overflows"))?;
    let data_y1 = data_y0
        .checked_add(i64::try_from(h).map_err(|_| err("EXR data-window height exceeds i64"))?)
        .ok_or_else(|| err("EXR data-window y bound overflows"))?;
    let copy_x0 = display_x0.max(data_x0);
    let copy_y0 = display_y0.max(data_y0);
    let copy_x1 = display_x1.min(data_x1);
    let copy_y1 = display_y1.min(data_y1);
    let copy_w = usize::try_from(copy_x1.saturating_sub(copy_x0)).map_err(|_| err("EXR clipped width"))?;
    let copy_h = usize::try_from(copy_y1.saturating_sub(copy_y0)).map_err(|_| err("EXR clipped height"))?;
    let source_x = usize::try_from(copy_x0.saturating_sub(data_x0)).map_err(|_| err("EXR source x offset"))?;
    let source_y = usize::try_from(copy_y0.saturating_sub(data_y0)).map_err(|_| err("EXR source y offset"))?;
    let target_x = usize::try_from(copy_x0.saturating_sub(display_x0)).map_err(|_| err("EXR target x offset"))?;
    let target_y = usize::try_from(copy_y0.saturating_sub(display_y0)).map_err(|_| err("EXR target y offset"))?;
    let output_pixels = (display_w as usize).checked_mul(display_h as usize).ok_or_else(|| err("EXR display pixel count overflows usize"))?;
    let out_len = output_pixels.checked_mul(nc).ok_or_else(|| err("EXR display sample count overflows usize"))?;
    let mut warnings = Vec::new();
    if display_w as usize != w || display_h as usize != h || data_x0 != display_x0 || data_y0 != display_y0 {
        let origin_x = i32::try_from(data_x0 - display_x0).map_err(|_| err("EXR data-window x origin overflows"))?;
        let origin_y = i32::try_from(data_y0 - display_y0).map_err(|_| err("EXR data-window y origin overflows"))?;
        warnings.push(DecodeWarning::DisplayWindowStage { stage: (display_w, display_h), origin: (origin_x, origin_y) });
    }
    let img = if all_f16 {
        let mut out = Vec::new();
        out.try_reserve_exact(out_len).map_err(|_| CodecError::LimitExceeded("EXR display image allocation".into()))?;
        out.resize(out_len, f16::ZERO);
        for (ci, c) in color.iter().enumerate() {
            if let FlatSamples::F16(v) = &c.sample_data {
                for (src, sample) in v.iter().enumerate() {
                    let y = src / w;
                    let x = src % w;
                    if x < source_x || x >= source_x.saturating_add(copy_w) || y < source_y || y >= source_y.saturating_add(copy_h) {
                        continue;
                    }
                    let dst_y = target_y.saturating_add(y - source_y);
                    let dst_x = target_x.saturating_add(x - source_x);
                    let dst = dst_y
                        .checked_mul(display_w as usize)
                        .and_then(|v| v.checked_add(dst_x))
                        .and_then(|v| v.checked_mul(nc))
                        .and_then(|v| v.checked_add(ci))
                        .ok_or_else(|| err("EXR target sample offset"))?;
                    *out.get_mut(dst).ok_or_else(|| err("EXR target sample is out of bounds"))? = *sample;
                }
            }
        }
        Image::from_f16(display_w, display_h, layout, &out)?
    } else {
        let mut out = Vec::new();
        out.try_reserve_exact(out_len).map_err(|_| CodecError::LimitExceeded("EXR display image allocation".into()))?;
        out.resize(out_len, 0f32);
        for (ci, c) in color.iter().enumerate() {
            let mut seen = 0usize;
            for (src, sample) in c.sample_data.values_as_f32().enumerate() {
                seen = seen.saturating_add(1);
                let y = src / w;
                let x = src % w;
                if x < source_x || x >= source_x.saturating_add(copy_w) || y < source_y || y >= source_y.saturating_add(copy_h) {
                    continue;
                }
                let dst_y = target_y.saturating_add(y - source_y);
                let dst_x = target_x.saturating_add(x - source_x);
                let dst = dst_y
                    .checked_mul(display_w as usize)
                    .and_then(|v| v.checked_add(dst_x))
                    .and_then(|v| v.checked_mul(nc))
                    .and_then(|v| v.checked_add(ci))
                    .ok_or_else(|| err("EXR target sample offset"))?;
                *out.get_mut(dst).ok_or_else(|| err("EXR target sample is out of bounds"))? = sample;
            }
            if seen != npx {
                return Err(err("EXR channel sample count mismatch"));
            }
        }
        Image::from_f32(display_w, display_h, layout, &out)?
    };
    debug_assert_eq!(img.sample_type(), sample);
    let mut img = img;
    img.warnings.extend(warnings);
    Ok(img)
}

/// Place an already selected channel group on the EXR display canvas. This covers the
/// named-channel path, whose channel selection happens after the source layer is decoded.
fn stage_image(mut image: Image, header: &exr::meta::header::Header) -> Result<Image, CodecError> {
    let (w, h) = image.dimensions();
    let display = header.shared_attributes.display_window;
    if display.size.0 == w as usize && display.size.1 == h as usize && display.position == header.own_attributes.layer_position {
        return Ok(image);
    }
    let dw = u32::try_from(display.size.0).map_err(|_| CodecError::LimitExceeded("EXR display-window width exceeds u32".into()))?;
    let dh = u32::try_from(display.size.1).map_err(|_| CodecError::LimitExceeded("EXR display-window height exceeds u32".into()))?;
    let dx = i64::from(display.position.0);
    let dy = i64::from(display.position.1);
    let sx = i64::from(header.own_attributes.layer_position.0);
    let sy = i64::from(header.own_attributes.layer_position.1);
    let ex = dx.checked_add(i64::from(dw)).ok_or_else(|| err("EXR display-window x bound overflows"))?;
    let ey = dy.checked_add(i64::from(dh)).ok_or_else(|| err("EXR display-window y bound overflows"))?;
    let sex = sx.checked_add(i64::from(w)).ok_or_else(|| err("EXR data-window x bound overflows"))?;
    let sey = sy.checked_add(i64::from(h)).ok_or_else(|| err("EXR data-window y bound overflows"))?;
    let ix = dx.max(sx);
    let iy = dy.max(sy);
    let iw = usize::try_from(ex.min(sex).saturating_sub(ix)).map_err(|_| err("EXR clipped width"))?;
    let ih = usize::try_from(ey.min(sey).saturating_sub(iy)).map_err(|_| err("EXR clipped height"))?;
    let src_x = usize::try_from(ix.saturating_sub(sx)).map_err(|_| err("EXR source x offset"))?;
    let src_y = usize::try_from(iy.saturating_sub(sy)).map_err(|_| err("EXR source y offset"))?;
    let dst_x = usize::try_from(ix.saturating_sub(dx)).map_err(|_| err("EXR target x offset"))?;
    let dst_y = usize::try_from(iy.saturating_sub(dy)).map_err(|_| err("EXR target y offset"))?;
    let channels = image.layout().channels();
    let pixel_bytes = channels.checked_mul(image.sample_type().bytes()).ok_or_else(|| err("EXR pixel stride overflows"))?;
    let dst_len = (dw as usize).checked_mul(dh as usize).and_then(|n| n.checked_mul(pixel_bytes)).ok_or_else(|| err("EXR staged buffer size overflows"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(dst_len).map_err(|_| CodecError::LimitExceeded("EXR display image allocation".into()))?;
    data.resize(dst_len, 0);
    let src = image.data();
    let src_stride = (w as usize).checked_mul(pixel_bytes).ok_or_else(|| err("EXR source row size overflows"))?;
    let dst_stride = (dw as usize).checked_mul(pixel_bytes).ok_or_else(|| err("EXR target row size overflows"))?;
    let copy_bytes = iw.checked_mul(pixel_bytes).ok_or_else(|| err("EXR copy row size overflows"))?;
    for row in 0..ih {
        let src_start = (src_y + row)
            .checked_mul(src_stride)
            .and_then(|v| src_x.checked_mul(pixel_bytes).and_then(|x| v.checked_add(x)))
            .ok_or_else(|| err("EXR source row offset"))?;
        let dst_start = (dst_y + row)
            .checked_mul(dst_stride)
            .and_then(|v| dst_x.checked_mul(pixel_bytes).and_then(|x| v.checked_add(x)))
            .ok_or_else(|| err("EXR target row offset"))?;
        let src_end = src_start.checked_add(copy_bytes).ok_or_else(|| err("EXR source row end overflows"))?;
        let dst_end = dst_start.checked_add(copy_bytes).ok_or_else(|| err("EXR target row end overflows"))?;
        let source = src.get(src_start..src_end).ok_or_else(|| err("EXR source row is truncated"))?;
        let target = data.get_mut(dst_start..dst_end).ok_or_else(|| err("EXR target row is out of bounds"))?;
        target.copy_from_slice(source);
    }
    let mut staged = Image::from_raw(dw, dh, image.layout(), image.sample_type(), data)?;
    staged.icc = image.icc.take();
    staged.meta = image.meta;
    staged.warnings = std::mem::take(&mut image.warnings);
    if dw != w || dh != h || sx != dx || sy != dy {
        let ox = i32::try_from(sx - dx).map_err(|_| err("EXR data-window x origin overflows"))?;
        let oy = i32::try_from(sy - dy).map_err(|_| err("EXR data-window y origin overflows"))?;
        staged.warnings.push(DecodeWarning::DisplayWindowStage { stage: (dw, dh), origin: (ox, oy) });
    }
    Ok(staged)
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let meta = read_meta(bytes, limits)?;

    // Deep parts are rejected by the `exr` crate's reader; walk their chunks ourselves and
    // composite the samples into the flat image (a warning says what was lost).
    if meta.requirements.has_deep_data {
        let deep = super::deep_exr::decode_deep(&meta, bytes, limits)?;
        return super::deep_exr::flatten(&deep);
    }

    let pick = pick_part(&meta);
    let layer = read_layer(bytes, pick)?;
    let header = meta.headers.get(pick).ok_or_else(|| err("EXR part header missing"))?;
    let mut img = layer_to_image(&layer, header)?;
    if meta.headers.len() > 1 {
        let total = u32::try_from(meta.headers.len()).ok();
        img.warnings.push(DecodeWarning::MoreParts { total });
    }
    Ok(img)
}

/// The size of the part [`decode`] opens, from the headers alone. Deep files are refused (their
/// flattened size comes from the deep decoder), as is a part [`decode`] cannot show.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<(u32, u32), CodecError> {
    let meta = read_meta(bytes, limits)?;
    if meta.requirements.has_deep_data {
        return Err(CodecError::unsupported(F, "no header-only size probe for deep data"));
    }
    let pick = pick_part(&meta);
    let info = meta.headers.get(pick).map(|h| part_info(pick, h)).ok_or_else(|| err("no parts"))?;
    if info.color_rank() == 0 {
        return Err(CodecError::unsupported(F, "no R/G/B or Y channels"));
    }
    let header = meta.headers.get(pick).ok_or_else(|| err("no parts"))?;
    let display = header.shared_attributes.display_window;
    Ok((
        u32::try_from(display.size.0).map_err(|_| CodecError::LimitExceeded("EXR display-window width exceeds u32".into()))?,
        u32::try_from(display.size.1).map_err(|_| CodecError::LimitExceeded("EXR display-window height exceeds u32".into()))?,
    ))
}

/// Decode one part of a multi-part file by its index into [`info`]'s list. A file with
/// deep parts is not mixed: the deep decoder composites those instead.
pub(crate) fn decode_part(bytes: &[u8], index: usize, limits: &Limits) -> Result<Image, CodecError> {
    let meta = read_meta(bytes, limits)?;
    if meta.requirements.has_deep_data {
        return Err(CodecError::unsupported(F, "this file has deep parts; the deep decoder composites those (mixing flat and deep parts is not supported)"));
    }
    if index >= meta.headers.len() {
        return Err(err(format!("part index {index} is out of range ({} parts)", meta.headers.len())));
    }
    let layer = read_layer(bytes, index)?;
    let header = meta.headers.get(index).ok_or_else(|| err("EXR part header missing"))?;
    layer_to_image(&layer, header)
}

/// Decodes the named channels of one flat part (exact names as the file stores them, e.g.
/// `diffuse.R`) into an image: R/G/B (plus A when named) give Rgba, a lone Y gives Gray. This
/// is how a channel group renders without the part's other channels in the way.
pub(crate) fn decode_channels(bytes: &[u8], index: usize, names: &[&str], limits: &Limits) -> Result<Image, CodecError> {
    let meta = read_meta(bytes, limits)?;
    if meta.requirements.has_deep_data {
        return Err(CodecError::unsupported(F, "this file has deep parts; the deep decoder composites those (mixing flat and deep parts is not supported)"));
    }
    if index >= meta.headers.len() {
        return Err(err(format!("part index {index} is out of range ({} parts)", meta.headers.len())));
    }
    let layer = read_layer(bytes, index)?;
    let (w, h) = (layer.size.0, layer.size.1);
    let channels = &layer.channel_data.list;
    let npx = w * h;
    let mut picked: Vec<&AnyChannel<FlatSamples>> = Vec::with_capacity(names.len());
    for name in names {
        let c = channels.iter().find(|c| c.name.to_string() == *name).ok_or_else(|| err(format!("the part has no channel {name}")))?;
        if c.sample_data.len() != npx {
            return Err(err("channel sample count mismatch (subsampled channels are unsupported)"));
        }
        picked.push(c);
    }
    let base = |c: &AnyChannel<FlatSamples>| -> String { base_channel_name(&c.name.to_string()).to_string() };
    let has = |n: &str| picked.iter().any(|c| base(c) == n);
    let (layout, order): (ChannelLayout, Vec<Option<usize>>) = if has("R") && has("G") && has("B") {
        let at = |n: &str| picked.iter().position(|c| base(c) == n);
        match has("A").then(|| at("A")) {
            Some(a) => (ChannelLayout::Rgba, vec![at("R"), at("G"), at("B"), a]),
            None => (ChannelLayout::Rgb, vec![at("R"), at("G"), at("B"), None]),
        }
    } else if has("Y") {
        let y = picked.iter().position(|c| base(c) == "Y");
        match picked.iter().position(|c| base(c) == "A") {
            Some(a) => (ChannelLayout::GrayA, vec![y, Some(a)]),
            None => (ChannelLayout::Gray, vec![y]),
        }
    } else if picked.len() == 1 {
        (ChannelLayout::Gray, vec![Some(0)])
    } else {
        return Err(CodecError::unsupported(F, "the named channels are no R/G/B or Y set"));
    };
    let all_f16 = picked.iter().all(|c| matches!(c.sample_data, FlatSamples::F16(_)));
    let nc = layout.channels();
    let (w32, h32) = (w as u32, h as u32);
    // The None slots sit past the layout's own channels (the absent A of an RGB pick) and
    // are skipped, so the buffer keeps its pixel stride.
    let img = if all_f16 {
        let mut out = vec![f16::ZERO; npx * nc];
        for (ci, pick) in order.iter().enumerate() {
            let Some(idx) = pick else { continue };
            let Some(FlatSamples::F16(v)) = picked.get(*idx).map(|c| &c.sample_data) else { continue };
            for (i, s) in v.iter().enumerate() {
                out[i * nc + ci] = *s;
            }
        }
        Image::from_f16(w32, h32, layout, &out)
    } else {
        let mut out = vec![0f32; npx * nc];
        for (ci, pick) in order.iter().enumerate() {
            let Some(idx) = pick else { continue };
            let Some(c) = picked.get(*idx) else { continue };
            for (i, s) in c.sample_data.values_as_f32().enumerate() {
                out[i * nc + ci] = s;
            }
        }
        Image::from_f32(w32, h32, layout, &out)
    };
    stage_image(img.map_err(|e| CodecError::encode(F, e))?, meta.headers.get(index).ok_or_else(|| err("EXR part header missing"))?)
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    let (w, h) = (img.width() as usize, img.height() as usize);
    let layout = img.layout();
    let names: &[&str] = match layout {
        ChannelLayout::Gray => &["Y"],
        ChannelLayout::GrayA => &["Y", "A"],
        ChannelLayout::Rgb => &["R", "G", "B"],
        ChannelLayout::Rgba => &["R", "G", "B", "A"],
        l => return Err(CodecError::encode(F, format!("unsupported layout {l:?}"))),
    };
    let nc = names.len();
    let mut list: Vec<AnyChannel<FlatSamples>> = Vec::with_capacity(nc);
    match img.sample_type() {
        SampleType::F16 => {
            let s = img.to_f16_samples().unwrap_or_default();
            for (ci, n) in names.iter().enumerate() {
                let v: Vec<f16> = s.iter().skip(ci).step_by(nc).copied().collect();
                list.push(AnyChannel::new(*n, FlatSamples::F16(v)));
            }
        }
        SampleType::F32 => {
            let s = img.to_f32_samples().unwrap_or_default();
            for (ci, n) in names.iter().enumerate() {
                let v: Vec<f32> = s.iter().skip(ci).step_by(nc).copied().collect();
                list.push(AnyChannel::new(*n, FlatSamples::F32(v)));
            }
        }
        s => return Err(CodecError::encode(F, format!("unsupported sample {s:?}"))),
    }
    let compression = match opts.exr_compression {
        ExrCompression::None => Compression::Uncompressed,
        ExrCompression::Rle => Compression::RLE,
        ExrCompression::Zip1 => Compression::ZIP1,
        ExrCompression::Zip16 => Compression::ZIP16,
        ExrCompression::Piz => Compression::PIZ,
    };
    let encoding = Encoding { compression, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing };
    let channels = AnyChannels::sort(list.into_iter().collect());
    let layer = Layer::new((w, h), LayerAttributes::default(), encoding, channels);
    let image = exr::image::Image::from_layer(layer);
    let mut cursor = Cursor::new(Vec::new());
    image.write().to_buffered(&mut cursor).map_err(|e| CodecError::encode(F, e))?;
    Ok(cursor.into_inner())
}
