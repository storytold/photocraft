//! FlashRaw supplies the pixel plane; PhotoCraft retains its metadata and colour policy.
//! The facade supplies frames. Its explicitly pinned, public low-level JPEG codec
//! also preserves stored DNG samples before irreversible container linearization.
//! No internal format parser is used; low-level codec upgrades require regressions.

use flashraw::{CorruptionPolicy, DecodeOptions, FlashRaw, OutputFormat, RawPixelData, SampleFormat};

use crate::sensor::Plane;
use crate::tiff::{Tiff, tag};
use crate::{BlackLevels, Cfa, ColorInfo, Limits, RawError, RawFormat, Rect, Sensor};

/// Content recognition only: no sensor decode and no file-name heuristic.
pub(crate) fn identify(bytes: &[u8]) -> Option<RawFormat> {
    let raw = FlashRaw::open_with(bytes, flash_limits(&Limits::default())).ok()?;
    // The library groups encrypted SRF with ARW. Our ARW recognizer handles
    // CFA and reduced-size TIFF layouts; the remaining legacy layout uses frame metadata.
    if raw.format() == flashraw::RawFormat::Arw && crate::identify_builtin(bytes).is_none() {
        return Some(RawFormat::Srf);
    }
    map_format(raw.format())
}

fn map_format(format: flashraw::RawFormat) -> Option<RawFormat> {
    use flashraw::RawFormat as F;
    Some(match format {
        F::Dng => RawFormat::Dng,
        F::Cr2 => RawFormat::Cr2,
        F::Cr3 => RawFormat::Cr3,
        F::Crw => RawFormat::Crw,
        F::Nef | F::Nrw => RawFormat::Nef,
        F::Arw | F::Sr2 => RawFormat::Arw,
        F::Srf => RawFormat::Srf,
        F::Raf => RawFormat::Raf,
        F::Rw2 => RawFormat::Rw2,
        F::Orf => RawFormat::Orf,
        F::Pef => RawFormat::Pef,
        F::Iiq => RawFormat::Iiq,
        F::Mrw => RawFormat::Mrw,
        F::Srw => RawFormat::Srw,
        F::X3f => RawFormat::X3f,
        F::Kdc => RawFormat::Kdc,
        F::ThreeFr => RawFormat::ThreeFr,
        F::Fff => RawFormat::Fff,
        F::Dcr | F::Erf => RawFormat::TiffEp,
        _ => return None,
    })
}

fn flash_limits(limits: &Limits) -> flashraw::Limits {
    flashraw::Limits::default()
        .with_max_dimension(limits.max_width.max(limits.max_height))
        .with_max_samples(limits.max_pixels.saturating_mul(3).min(limits.max_alloc / 2))
}

/// Metadata for newly supported representations comes from the validated raw frame.
/// Existing per-camera profiles and DNG calibration continue through our metadata readers.
fn frame_metadata(bytes: &[u8], format: RawFormat, frame: &flashraw::RawFrame) -> bool {
    match format {
        RawFormat::Crw
        | RawFormat::Mrw
        | RawFormat::Srw
        | RawFormat::X3f
        | RawFormat::Iiq
        | RawFormat::Srf
        | RawFormat::Kdc
        | RawFormat::ThreeFr
        | RawFormat::Fff
        | RawFormat::Cr3 => true,
        RawFormat::Rw2 => Tiff::new(bytes).and_then(|t| t.ifd_at(t.first_ifd, 0).and_then(|i| t.tag_uint(&i, 0x2d))).is_some_and(|v| v >= 6),
        RawFormat::Orf => Tiff::new(bytes)
            .and_then(|t| {
                t.ifd_at(t.first_ifd, 0).map(|i| {
                    t.tag_uints(&i, tag::STRIP_BYTE_COUNTS).iter().map(|v| u64::from(*v)).sum::<u64>() < u64::from(frame.width) * u64::from(frame.height) * 2
                        || t.tag_uint(&i, tag::COMPRESSION).unwrap_or(1) != 1
                })
            })
            .unwrap_or(false),
        RawFormat::Pef | RawFormat::TiffEp => {
            let Some(t) = Tiff::new(bytes) else { return true };
            let ifds = t.all_ifds();
            let Some(raw) = crate::tiffep::raw_ifd(&t, &ifds) else { return true };
            crate::tiffep::cfa(&t, &raw, &ifds).is_none()
                || t.tag_uint(&raw, tag::IMAGE_WIDTH) != Some(frame.width)
                || t.tag_uint(&raw, tag::IMAGE_LENGTH) != Some(frame.height)
        }
        _ => false,
    }
}

fn error(e: flashraw::Error) -> RawError {
    use flashraw::Error as E;
    let message = format!("FlashRaw: {e}");
    match e {
        E::UnsupportedFormat(_) | E::UnsupportedCompression { .. } | E::CameraUnsupported { .. } | E::NotAvailable(_) => RawError::unsupported(message),
        E::LimitExceeded(_) | E::OutOfMemory(_) => RawError::LimitExceeded(message),
        _ => RawError::malformed(message),
    }
}

pub(crate) fn decode(bytes: &[u8], format: RawFormat, limits: &Limits) -> Result<Sensor, RawError> {
    if let Some(t) = Tiff::new(bytes) {
        if format == RawFormat::Dng && crate::dng::flashraw_stored(&t) {
            return crate::dng::decode_flashraw_stored(&t, limits);
        }
        let ifd0 = t.ifd_at(t.first_ifd, 0);
        if matches!(format, RawFormat::Orf | RawFormat::Rw2) {
            let (wtag, htag) = if format == RawFormat::Rw2 { (2, 3) } else { (256, 257) };
            let width = ifd0.as_ref().and_then(|i| t.tag_uint(i, wtag)).unwrap_or(0);
            let height = ifd0.as_ref().and_then(|i| t.tag_uint(i, htag)).unwrap_or(0);
            if width < 64 || height < 64 {
                return Err(RawError::unsupported("small ORF/RW2 sensor"));
            }
        }
        match format {
            RawFormat::Dng => crate::dng::flashraw_preflight(&t, limits)?,
            RawFormat::Nef | RawFormat::Arw | RawFormat::Pef => crate::tiffep::flashraw_preflight(&t, format)?,
            _ => {}
        }
    }
    if format == RawFormat::Raf {
        crate::raf::flashraw_preflight(bytes)?;
    }
    if format == RawFormat::Cr2 {
        crate::cr2::flashraw_preflight(bytes)?;
    }
    if format == RawFormat::Cr3 && bytes.len() < 64 {
        return Err(RawError::unsupported("CR3 without sensor data"));
    }
    // FlashRaw has one dimension cap and counts samples rather than pixels. Enforce
    // PhotoCraft's asymmetric bounds and byte cap again before decoding any pixels.
    let sample_cap = limits.max_pixels.saturating_mul(3).min(limits.max_alloc / 4);
    let fl = flash_limits(limits);
    let raw = FlashRaw::open_with(bytes, fl).map_err(error)?;
    let info = raw.frame_info();
    if info.sample_format != SampleFormat::U16 || info.white_balance_applied || !matches!(info.cpp, 1 | 3) {
        return Err(RawError::unsupported("FlashRaw sensor representation needs the built-in decoder"));
    }
    limits.check(u64::from(info.width), u64::from(info.height), u64::from(info.cpp) * 2)?;
    let samples = u64::from(info.width).saturating_mul(u64::from(info.height)).saturating_mul(u64::from(info.cpp));
    if samples > sample_cap {
        return Err(RawError::unsupported("FlashRaw scratch memory needs the built-in decoder"));
    }

    let mut opts = DecodeOptions {
        output: OutputFormat::RawU16,
        crop: false,
        apply_opcodes: false,
        corruption_policy: CorruptionPolicy::Strict,
        preview_fallback: false,
        limits: fl.with_max_samples(sample_cap),
        ..Default::default()
    };
    // Transitive FlashRaw crates can still contain rayon. The explicit sequential
    // context prevents their global pool from being initialized in a browser.
    if cfg!(target_arch = "wasm32") {
        opts.threads = Some(1);
    }
    let mut frame = raw.decode(opts).map_err(error)?;
    if frame.is_incomplete() || frame.origin != (0, 0) || frame.white_balance_applied {
        return Err(RawError::malformed("FlashRaw returned incomplete or transformed sensor data"));
    }
    let RawPixelData::U16(data) = std::mem::replace(&mut frame.data, RawPixelData::U16(Vec::new())) else {
        return Err(RawError::unsupported("FlashRaw did not return integer sensor samples"));
    };
    let plane = Plane {
        width: frame.width as usize,
        height: frame.height as usize,
        samples: frame.cpp as usize,
        bits: u32::from(frame.bits_per_sample),
        data,
        float_scale: None,
        linearized: format == RawFormat::Dng,
    };
    plane.validate(limits)?;
    if frame_metadata(bytes, format, &frame) {
        let format =
            if format == RawFormat::TiffEp { map_format(frame.metadata.format.unwrap_or(flashraw::RawFormat::Other)).unwrap_or(format) } else { format };
        return frame_sensor(plane, &frame, format, limits);
    }
    if format == RawFormat::Raf {
        return crate::raf::decode_with_plane(bytes, limits, Some(plane));
    }
    let t = Tiff::new(bytes).ok_or(RawError::NotRaw)?;
    match format {
        RawFormat::Dng => crate::dng::decode_with_plane(&t, limits, Some(plane)).map(crate::lift_clipped),
        RawFormat::Cr2 => crate::cr2::decode_with_plane(&t, limits, Some(plane)),
        RawFormat::Rw2 => crate::rw2::decode_with_plane(&t, limits, Some(plane)),
        RawFormat::Orf => crate::orf::decode_with_plane(&t, limits, Some(plane)),
        f => crate::tiffep::decode_with_plane(&t, f, limits, Some(plane)),
    }
}

fn rect(r: flashraw::Rect) -> Rect {
    Rect::new(r.x as usize, r.y as usize, r.width as usize, r.height as usize)
}

// FlashRaw anchors repeating levels/deltas to sensor (0,0); Sensor::black
// uses active-area coordinates. Re-anchor the pattern and slice full-buffer
// deltas, broadcasting a uniform scalar correctly for linear RGB as well.
fn frame_black(black: &flashraw::BlackLevel, active: Rect, samples: usize, width: usize, height: usize, limits: &Limits) -> Result<BlackLevels, RawError> {
    let (rows, cols, channels) = (usize::from(black.rows.max(1)), usize::from(black.cols.max(1)), usize::from(black.samples.max(1)));
    let count = rows.checked_mul(cols).and_then(|n| n.checked_mul(samples)).ok_or_else(|| RawError::malformed("black-level pattern overflows"))?;
    if (count as u64).saturating_mul(4) > limits.max_alloc {
        return Err(RawError::LimitExceeded("sensor black-level pattern".into()));
    }
    let mut values = Vec::with_capacity(count);
    for y in 0..rows {
        for x in 0..cols {
            for channel in 0..samples {
                let r = (active.y % rows + y) % rows;
                let c = (active.x % cols + x) % cols;
                let index = (r * cols + c) * channels + channel % channels;
                values.push(black.values.get(index).copied().unwrap_or(0.0));
            }
        }
    }
    let slice = |data: &[f32], start: usize, len: usize, full: usize| -> Result<Vec<f32>, RawError> {
        if data.is_empty() {
            return Ok(Vec::new());
        }
        if data.len() != full {
            return Err(RawError::malformed("sensor black-level delta dimensions differ"));
        }
        if (len as u64).saturating_mul(4) > limits.max_alloc {
            return Err(RawError::LimitExceeded("sensor black-level deltas".into()));
        }
        let end = start.checked_add(len).ok_or_else(|| RawError::malformed("black-level delta range overflow"))?;
        data.get(start..end).map(<[f32]>::to_vec).ok_or_else(|| RawError::malformed("black-level deltas outside sensor"))
    };
    Ok(BlackLevels {
        rows,
        cols,
        values,
        delta_h: slice(&black.col_deltas, active.x, active.width, width)?,
        delta_v: slice(&black.row_deltas, active.y, active.height, height)?,
    })
}

// CR3 had no sensor decoder or camera profile here. Use file geometry/levels/WB,
// and leave colour uncalibrated so I/O can fit the camera JPEG as it does for NEF.
fn frame_sensor(p: Plane, frame: &flashraw::RawFrame, format: RawFormat, limits: &Limits) -> Result<Sensor, RawError> {
    let meta = &frame.metadata;
    let black = &frame.black_level;
    let white = &frame.white_level;
    let cfa = frame
        .cfa
        .as_ref()
        .map(|c| {
            let colors = c
                .colors()
                .into_iter()
                .map(|v| match v {
                    flashraw::CfaColor::Red => Ok(0),
                    flashraw::CfaColor::Green => Ok(1),
                    flashraw::CfaColor::Blue => Ok(2),
                    flashraw::CfaColor::Emerald => Ok(3),
                    _ => Err(RawError::unsupported("non-RGB CFA")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Cfa { width: c.width(), height: c.height(), colors, origin_x: 0, origin_y: 0 })
        })
        .transpose()?;
    if cfa.as_ref().is_some_and(|c| c.width == 0 || c.height == 0 || c.width > 16 || c.height > 16 || c.width.checked_mul(c.height) != Some(c.colors.len())) {
        return Err(RawError::unsupported("sensor CFA period is not supported"));
    }
    if (cfa.is_none() && p.samples != 3) || (cfa.is_some() && p.samples != 1) {
        return Err(RawError::unsupported("FlashRaw sensor layout"));
    }
    if black.values.iter().chain(&black.col_deltas).chain(&black.row_deltas).any(|v| !v.is_finite()) {
        return Err(RawError::malformed("non-finite sensor black levels"));
    }
    let bounds = Rect::new(0, 0, p.width, p.height);
    let active = rect(frame.active_area);
    let crop = rect(frame.crop);
    if active.is_empty() || active.intersect(&bounds) != active || crop.is_empty() || crop.intersect(&active) != crop {
        return Err(RawError::malformed("FlashRaw sensor crop is outside its image"));
    }
    let white = [0, 1, 2].map(|i| white.values.get(i).or_else(|| white.values.first()).copied().unwrap_or(65535) as f32);
    if white.iter().any(|v| !v.is_finite() || *v <= 0.0 || *v > 65535.0) {
        return Err(RawError::malformed("invalid sensor white levels"));
    }
    let camera_wb = frame
        .white_balance
        .as_ref()
        .map(|w| [f64::from(w.coeffs[0]), f64::from(w.coeffs[1]), f64::from(w.coeffs[2])])
        .filter(|v| v.iter().all(|v| v.is_finite() && *v > 0.0));
    Ok(Sensor {
        format,
        make: (!meta.make.is_empty()).then(|| meta.make.clone()),
        model: (!meta.model.is_empty()).then(|| meta.model.clone()),
        width: p.width,
        height: p.height,
        samples: p.samples,
        data: p.data,
        cfa,
        linearization: None,
        black: frame_black(black, active, p.samples, p.width, p.height, limits)?,
        white,
        active,
        crop,
        color: ColorInfo::default(),
        camera_wb,
        orientation: frame.orientation as u16,
        baseline_exposure: 0.0,
        gain_maps: Vec::new(),
        tone_curve: Vec::new(),
        black_point: 0.0,
        warnings: Vec::new(),
    })
}

/// The public codec API is re-exported by the exactly pinned facade. The high-level
/// DNG API irreversibly linearizes samples, including non-injective tables. Decode
/// the JPEG bytes into our bounded buffer instead; PhotoCraft owns all DNG metadata.
pub(crate) fn decode_stored_jpeg(src: &[u8], width: usize, height: usize, samples: usize, limits: &Limits) -> Result<Vec<u16>, RawError> {
    use flashraw::low_level::codecs::ljpeg::{LjpegOptions, decode_into_par};
    limits.check(width as u64, height as u64, (samples as u64).saturating_mul(2))?;
    let frame = crate::ljpeg::frame(src)?;
    limits.check(frame.width as u64, frame.height as u64, (frame.components as u64).saturating_mul(2))?;
    let row = width.checked_mul(samples).ok_or_else(|| RawError::malformed("JPEG row size overflow"))?;
    let count = row.checked_mul(height).ok_or_else(|| RawError::malformed("JPEG sample count overflow"))?;
    if frame.samples().is_none_or(|n| n < count) {
        return Err(RawError::malformed("JPEG codestream is smaller than its segment"));
    }
    // The codec's unstuff/checkpoint scratch must fit the caller's byte policy.
    if (src.len() as u64).saturating_mul(4) > limits.max_alloc {
        return Err(RawError::unsupported("FlashRaw JPEG scratch budget"));
    }
    let tail = src.iter().rposition(|v| *v != 0 && *v != 0xff).ok_or_else(|| RawError::malformed("JPEG has no end marker"))?;
    if src.get(tail) != Some(&0xd9) || tail.checked_sub(1).and_then(|i| src.get(i)) != Some(&0xff) {
        return Err(RawError::malformed("JPEG end marker is missing"));
    }
    let mut data = vec![0u16; count];
    let header = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_into_par(src, LjpegOptions::default(), &mut data, row, row, height, !cfg!(target_arch = "wasm32") && count >= 1_000_000).map_err(error)
    }))
    .unwrap_or_else(|_| Err(RawError::malformed("FlashRaw JPEG codec failed")))?;
    if header.point_transform > 0 {
        if header.point_transform >= 16 {
            return Err(RawError::malformed("invalid JPEG point transform"));
        }
        for value in &mut data {
            let shifted = u32::from(*value)
                .checked_shl(header.point_transform)
                .filter(|v| *v <= u32::from(u16::MAX))
                .ok_or_else(|| RawError::malformed("JPEG point transform overflows its sample"))?;
            *value = shifted as u16;
        }
    }
    Ok(data)
}

#[cfg(test)]
mod stored_jpeg_tests {
    use super::*;
    use crate::testgen::lj92_encode;

    #[test]
    fn black_levels_follow_active_origin_and_broadcast_rgb_channels() {
        let mut black = flashraw::BlackLevel::pattern2x2([10.0, 20.0, 30.0, 40.0]);
        black.col_deltas = (0..8).map(|x| x as f32 * 0.25).collect();
        black.row_deltas = (0..6).map(|y| y as f32 * 0.5).collect();
        let active = Rect::new(1, 1, 6, 4);
        for samples in [1, 3] {
            let ours = frame_black(&black, active, samples, 8, 6, &Limits::default()).unwrap();
            for y in 0..active.height {
                for x in 0..active.width {
                    for channel in 0..samples {
                        assert_eq!(ours.at(x, y, channel, samples), black.at(x + 1, y + 1, channel));
                    }
                }
            }
        }
        assert!(matches!(frame_black(&black, active, 3, 8, 6, &Limits { max_alloc: 1, ..Default::default() }), Err(RawError::LimitExceeded(_))));
    }

    #[test]
    fn generic_sensor_metadata_rejects_bad_crop_and_nonfinite_levels() {
        let bytes = crate::testgen::DngSpec::cfa(128, 96, vec![1000; 128 * 96]).build();
        let raw = FlashRaw::open(bytes.as_slice()).unwrap();
        let options = DecodeOptions { crop: false, apply_opcodes: false, corruption_policy: CorruptionPolicy::Strict, ..Default::default() };
        let mut frame = raw.decode(options).unwrap();
        let plane = || Plane { width: 128, height: 96, samples: 1, bits: 16, data: vec![1000; 128 * 96], float_scale: None, linearized: false };
        assert!(frame_sensor(plane(), &frame, RawFormat::Crw, &Limits::default()).is_ok());
        let saved = frame.crop;
        frame.crop.width = u32::MAX;
        assert!(matches!(frame_sensor(plane(), &frame, RawFormat::Crw, &Limits::default()), Err(RawError::Malformed(_))));
        frame.crop = saved;
        frame.black_level.values = vec![f32::NAN];
        assert!(matches!(frame_sensor(plane(), &frame, RawFormat::Crw, &Limits::default()), Err(RawError::Malformed(_))));
    }

    #[test]
    fn stored_codec_keeps_point_transform_precision_and_rejects_truncated_entropy() {
        for components in [1, 3] {
            let (w, h) = (128, 96);
            let encoded: Vec<u16> = (0..w * h * components).map(|i| ((i * 37 + i / 128 * 11) & 4095) as u16).collect();
            let mut src = lj92_encode(&encoded, w, h, components, 12, 1, None);
            let sof = src.windows(2).position(|b| b == [0xff, 0xc3]).unwrap();
            src[sof + 4] = 14;
            let sos = src.windows(2).position(|b| b == [0xff, 0xda]).unwrap();
            let len = usize::from(u16::from_be_bytes([src[sos + 2], src[sos + 3]]));
            src[sos + len + 1] = 2;
            let (_, reference) = crate::ljpeg::decode(&src, w * h * components).unwrap();
            assert_eq!(reference, encoded.iter().map(|v| v << 2).collect::<Vec<_>>());
            let got = decode_stored_jpeg(&src, w, h, components, &Limits::default()).unwrap();
            assert_eq!(got, reference);
            let mut truncated = src[..src.len() / 2].to_vec();
            truncated.extend_from_slice(&[0xff, 0xd9]);
            assert!(decode_stored_jpeg(&truncated, w, h, components, &Limits::default()).is_err());
            assert!(matches!(decode_stored_jpeg(&src, w, h, components, &Limits { max_width: 64, ..Default::default() }), Err(RawError::LimitExceeded(_))));
        }
    }
}
