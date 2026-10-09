//! AVIF still pictures through safe Rust APIs. Dependencies are pinned, assembly is disabled.

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
use crate::{ChannelLayout as L, SampleType as S};
use crate::{CodecError, EncodeOptions, Format, Image, Limits};
const F: Format = Format::Avif;

#[cfg(not(all(feature = "avif", not(target_arch = "wasm32"))))]
pub(crate) fn decode(_: &[u8], _: &Limits) -> Result<Image, CodecError> {
    Err(CodecError::unsupported(F, "AVIF support isn't included in this build of PhotoCraft"))
}
#[cfg(not(all(feature = "avif", not(target_arch = "wasm32"))))]
pub(crate) fn encode(_: &Image, _: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::unsupported(F, "AVIF support isn't included in this build of PhotoCraft"))
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn guard<T>(f: impl FnOnce() -> Result<T, CodecError>) -> Result<T, CodecError> {
    // The upstream safe API still contains assertions; malformed data must not escape them.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(CodecError::malformed(F, "the AVIF codec rejected malformed or unsupported data")))
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn picture(data: &[u8], limits: &Limits) -> Result<rav1d::Picture, CodecError> {
    let md = avif_parse::AV1Metadata::parse_av1_bitstream(data).map_err(|e| CodecError::malformed(F, e))?;
    // AVIF 1.2 §2.1 permits a general AV1 sequence header for a single image item.
    // still_picture/reduced_still_picture_header are optional overhead optimizations;
    // the container and actual decoded picture count determine still-image support.
    // Include decoder workspace in the budget, not just the final RGB buffer. A conservative
    // 64 bytes/pixel covers reference planes, padded borders and output for this single frame.
    limits.check_bytes(md.max_frame_width.get(), md.max_frame_height.get(), 64)?;
    let mut settings = rav1d::Settings::new();
    settings.set_n_threads(1); // Bounds per-image worker memory and avoids nested worker pools.
    settings.set_max_frame_delay(1);
    settings.set_frame_size_limit(limits.max_pixels.min(limits.max_alloc / 64).clamp(1, u64::from(u32::MAX)) as u32);
    settings.set_strict_std_compliance(true);
    let mut decoder = rav1d::Decoder::with_settings(&settings).map_err(|e| CodecError::malformed(F, e))?;
    let sent = decoder.send_data(data.to_vec().into_boxed_slice(), None, None, None);
    if let Err(e) = sent
        && e != rav1d::Rav1dError::TryAgain
    {
        return Err(CodecError::malformed(F, e));
    }
    let frame = decoder.get_picture().map_err(|e| CodecError::malformed(F, e))?;
    // A still payload must be fully consumed and contain exactly one picture.
    let pending = decoder.send_pending_data();
    if let Err(e) = pending
        && e != rav1d::Rav1dError::TryAgain
    {
        return Err(CodecError::malformed(F, e));
    }
    match decoder.get_picture() {
        Ok(_) => Err(CodecError::unsupported(F, "multiple AV1 pictures in a still image")),
        Err(rav1d::Rav1dError::TryAgain) if pending.is_ok() => Ok(frame),
        Err(e) => Err(CodecError::malformed(F, e)),
    }
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn sample(pic: &rav1d::Picture, component: rav1d::PlanarImageComponent, x: u32, y: u32) -> Result<f64, CodecError> {
    let depth = pic.bits_per_component().ok_or_else(|| CodecError::malformed(F, "invalid AV1 bit depth"))?.0;
    let bps = if depth == 8 { 1usize } else { 2 };
    // rav1d's stride is in bytes, including padding, even for high-depth planes.
    let offset = (y as usize)
        .checked_mul(pic.stride(component) as usize)
        .and_then(|n| n.checked_add(x as usize * bps))
        .ok_or_else(|| CodecError::malformed(F, "AV1 plane offset overflow"))?;
    let bytes = pic.plane(component).get(offset..offset.saturating_add(bps)).ok_or_else(|| CodecError::malformed(F, "truncated AV1 plane"))?;
    let value = if bps == 1 {
        u16::from(*bytes.first().ok_or_else(|| CodecError::malformed(F, "missing AV1 sample"))?)
    } else {
        u16::from_ne_bytes(bytes.try_into().map_err(|_| CodecError::malformed(F, "missing AV1 sample"))?)
    };
    Ok(f64::from(value))
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    guard(|| decode_inner(bytes, limits))
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn decode_inner(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    use rav1d::{PixelLayout as P, PlanarImageComponent as C};
    if bytes.len() as u64 > limits.max_alloc {
        return Err(CodecError::LimitExceeded("AVIF file exceeds allocation budget".into()));
    }
    let props = super::avif_container::properties(bytes)?;
    if let Some((w, h)) = props.dimensions {
        limits.check_bytes(w, h, 64)?;
    }
    let mut reader = bytes;
    let avif = avif_parse::read_avif(&mut reader).map_err(|e| CodecError::malformed(F, e))?;
    let color = picture(&avif.primary_item, limits)?;
    let alpha = avif.alpha_item.as_deref().map(|a| picture(a, limits)).transpose()?;
    let (w, h) = (color.width(), color.height());
    if props.dimensions.is_some_and(|size| size != (w, h)) {
        return Err(CodecError::malformed(F, "AVIF container and AV1 picture dimensions disagree"));
    }
    if alpha.as_ref().is_some_and(|a| a.width() != w || a.height() != h || a.pixel_layout() != P::I400) {
        return Err(CodecError::malformed(F, "alpha must be a monochrome picture matching the colour dimensions"));
    }
    let depth = color.bits_per_component().ok_or_else(|| CodecError::malformed(F, "invalid AV1 bit depth"))?.0;
    let max = f64::from((1u16 << depth) - 1);
    let layout = if alpha.is_some() { L::Rgba } else { L::Rgb };
    let output_sample = if depth == 8 && alpha.as_ref().is_none_or(|a| a.bits_per_component().is_some_and(|d| d.0 == 8)) { S::U8 } else { S::U16 };
    limits.check(w, h, layout, output_sample)?;
    let (primaries, transfer, matrix, full) = props.cicp.unwrap_or((
        color.color_primaries() as u16,
        color.transfer_characteristic() as u16,
        color.matrix_coefficients() as u16,
        color.color_range() == rav1d::pixel::YUVRange::Full,
    ));
    // An ICC profile defines the RGB meaning. Otherwise only colour spaces the CMS can tag
    // exactly are accepted. PQ/HLG are refused instead of silently appearing as sRGB.
    if props.icc.is_none() && !matches!((primaries, transfer), (1 | 2, 13 | 2) | (1, 8) | (12, 13) | (9, 1 | 14 | 15)) {
        return Err(CodecError::unsupported(
            F,
            format!("CICP primaries {primaries}, transfer {transfer} need a supported ICC profile (PQ/HLG are not supported)"),
        ));
    }
    let coeffs = match matrix {
        0 => None,
        1 => Some((0.2126, 0.0722)),
        2 | 5 | 6 => Some((0.299, 0.114)),
        9 => Some((0.2627, 0.0593)),
        _ => return Err(CodecError::unsupported(F, format!("AVIF matrix coefficients {matrix} are not supported"))),
    };
    if matrix == 0 && (color.pixel_layout() != P::I444 || !full) {
        return Err(CodecError::malformed(F, "identity colour matrix requires full-range 4:4:4"));
    }
    let bytes_per_pixel = layout.channels() * output_sample.bytes();
    let count = (w as usize)
        .checked_mul(h as usize)
        .and_then(|v| v.checked_mul(bytes_per_pixel))
        .ok_or_else(|| CodecError::LimitExceeded("AVIF output size overflow".into()))?;
    let mut data = Vec::new();
    data.try_reserve_exact(count).map_err(|_| CodecError::LimitExceeded("not enough memory for AVIF pixels".into()))?;
    let shift = f64::from(1u16 << (depth - 8));
    for y in 0..h {
        for x in 0..w {
            let yy = sample(&color, C::Y, x, y)?;
            let luma = if full { yy / max } else { (yy - 16.0 * shift) / (219.0 * shift) };
            let mut rgb = if color.pixel_layout() == P::I400 {
                [luma; 3]
            } else {
                let (sx, sy) = match color.pixel_layout() {
                    P::I420 => (1, 1),
                    P::I422 => (1, 0),
                    _ => (0, 0),
                };
                let u = sample(&color, C::U, x >> sx, y >> sy)?;
                let v = sample(&color, C::V, x >> sx, y >> sy)?;
                if let Some((kr, kb)) = coeffs {
                    let range = if full { max } else { 224.0 * shift };
                    let cb = (u - 128.0 * shift) / range;
                    let cr = (v - 128.0 * shift) / range;
                    let r = luma + 2.0 * (1.0 - kr) * cr;
                    let b = luma + 2.0 * (1.0 - kb) * cb;
                    [r, (luma - kr * r - kb * b) / (1.0 - kr - kb), b]
                } else {
                    [v / max, yy / max, u / max]
                }
            };
            let a = match &alpha {
                Some(a) => {
                    let d = a.bits_per_component().ok_or_else(|| CodecError::malformed(F, "invalid alpha depth"))?.0;
                    sample(a, C::Y, x, y)? / f64::from((1u16 << d) - 1)
                }
                None => 1.0,
            };
            if avif.premultiplied_alpha {
                for c in &mut rgb {
                    *c = if a > 0.0 { *c / a } else { 0.0 };
                }
            }
            for value in rgb.into_iter().chain(alpha.as_ref().map(|_| a)) {
                if output_sample == S::U8 {
                    data.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
                } else {
                    data.extend_from_slice(&((value.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes());
                }
            }
        }
    }
    let mut img = Image::from_raw(w, h, layout, output_sample, data)?;
    img.icc = props.icc;
    if img.icc.is_none() {
        img.meta.cicp = Some((primaries, transfer));
    }
    Ok(img)
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
pub(crate) fn encode(src: &Image, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    if !(1..=100).contains(&opts.avif_quality)
        || !(1..=100).contains(&opts.avif_alpha_quality)
        || !(1..=10).contains(&opts.avif_speed)
        || !matches!(opts.avif_depth, 0 | 8 | 10)
    {
        return Err(CodecError::InvalidImage("AVIF quality/alpha quality must be 1–100, speed 1–10 and depth 0, 8 or 10".into()));
    }
    if src.layout().is_cmyk() {
        return Err(CodecError::unsupported(F, "convert CMYK to RGB through a colour profile before AVIF encoding"));
    }
    if src.layout().is_gray() && src.icc.is_some() {
        return Err(CodecError::unsupported(F, "convert profiled grayscale to RGB through its colour profile before AVIF encoding"));
    }
    if src.icc.is_none() && src.meta.cicp.is_some_and(|c| !matches!(c, (1 | 2, 13 | 2))) {
        return Err(CodecError::unsupported(F, "embed an RGB ICC profile or convert non-sRGB CICP pixels through the document I/O layer before AVIF encoding"));
    }
    Limits { max_width: 65536, max_height: 65536, max_pixels: 120_000_000, max_alloc: 8 << 30 }.check_bytes(src.width(), src.height(), 64)?;
    let depth = match opts.avif_depth {
        0 if src.sample_type() == S::U8 => 8,
        0 => 10,
        d => d,
    };
    guard(|| {
        let encoder = ravif::Encoder::new()
            .with_quality(f32::from(opts.avif_quality))
            .with_alpha_quality(f32::from(opts.avif_alpha_quality))
            .with_speed(opts.avif_speed)
            .with_num_threads(Some(1));
        // Identity GBR avoids an extra YCbCr colour transform and preserves arbitrary RGB ICC
        // meanings. No chroma subsampling. Quantize only to the explicitly selected AV1 depth.
        let layout = if src.layout().has_alpha() { L::Rgba } else { L::Rgb };
        let img = src.convert(layout, S::F32);
        let vals = img.to_normalized();
        let ch = layout.channels();
        let quantize = |v: f32| -> u16 { (v.clamp(0.0, 1.0) * f32::from((1u16 << depth) - 1)).round() as u16 };
        let planes = vals.chunks_exact(ch).map(|p| {
            let [r, g, b, ..] = p else { return [0; 3] };
            [quantize(*g), quantize(*b), quantize(*r)]
        });
        let alpha = layout.has_alpha().then(|| vals.chunks_exact(ch).map(|p| quantize(p.last().copied().unwrap_or(1.0))));
        let encoded = if depth == 8 {
            encoder.encode_raw_planes_8_bit(
                src.width() as usize,
                src.height() as usize,
                planes.map(|p| p.map(|v| v as u8)),
                alpha.map(|a| a.map(|v| v as u8)),
                ravif::PixelRange::Full,
                ravif::MatrixCoefficients::Identity,
            )
        } else {
            encoder.encode_raw_planes_10_bit(
                src.width() as usize,
                src.height() as usize,
                planes,
                alpha,
                ravif::PixelRange::Full,
                ravif::MatrixCoefficients::Identity,
            )
        }
        .map_err(|e| CodecError::encode(F, e))?;
        match src.icc.as_deref().filter(|_| opts.embed_icc) {
            Some(icc) => super::avif_container::mux(&encoded.avif_file, icc, src.width(), src.height(), depth),
            None => Ok(encoded.avif_file),
        }
    })
}
