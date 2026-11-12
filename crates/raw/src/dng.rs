//! Digital Negative (Adobe DNG Specification 1.7): the file format is craft-libs' `craft-raw`
//! (raw IFD selection, decompression, linearization, levels, active area, default crop, CFA,
//! colour tags and opcode lists). This maps its `RawImage` onto [`Sensor`], keeps the cases this
//! developer does not handle as unsupported, and applies the MapPolynomial opcodes lossy DNGs
//! carry (#8125).

use crate::color::{self, Calibration, ColorInfo, IDENTITY};
use crate::error::{RawError, Result};
use crate::sensor::{BlackLevels, Cfa, Rect, Sensor};
use crate::{Limits, RawFormat, opcodes};

/// The shared crate's error as this crate's.
pub(crate) fn raw_err(e: craft_raw::RawError) -> RawError {
    match e {
        craft_raw::RawError::NotRaw => RawError::NotRaw,
        craft_raw::RawError::Unsupported(m) => RawError::Unsupported(m),
        craft_raw::RawError::Corrupt(m) => RawError::Malformed(m),
        craft_raw::RawError::Limit(m) => RawError::LimitExceeded(m.to_string()),
        craft_raw::RawError::Tiff(e) => RawError::Malformed(e.to_string()),
    }
}

/// This crate's limits as a sample count for the shared decoder.
pub(crate) fn raw_limits(limits: &Limits) -> craft_raw::Limits {
    let mut out = craft_raw::Limits::default();
    // Samples are 2 bytes (or 4 for floats, checked again below).
    out.max_samples = usize::try_from(limits.max_pixels.saturating_mul(4).min(limits.max_alloc / 2)).unwrap_or(usize::MAX);
    out
}

fn rect(r: craft_raw::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

/// Colour calibration from the colour tags: invertible matrices only, sorted by temperature,
/// forward matrices used only when both calibrations have one.
pub(crate) fn color_info(c: &craft_raw::ColorData) -> ColorInfo {
    let mut cal = Vec::new();
    for (i, default_t) in [(0, 2856.0), (1, 6504.0)] {
        let Some(color_matrix) = c.color_matrix[i] else { continue };
        if color::invert(&color_matrix).is_none() {
            continue;
        }
        let temperature = color::illuminant_temperature(u32::from(c.illuminant[i])).unwrap_or(default_t);
        cal.push(Calibration {
            temperature,
            color_matrix,
            forward_matrix: c.forward_matrix[i].filter(|m| color::invert(m).is_some()),
            camera_calibration: c.camera_calibration[i].filter(|m| color::invert(m).is_some()).unwrap_or(IDENTITY),
        });
    }
    cal.sort_by(|a, b| a.temperature.total_cmp(&b.temperature));
    if cal.len() == 2 && (cal[0].forward_matrix.is_some() != cal[1].forward_matrix.is_some()) {
        for c in &mut cal {
            c.forward_matrix = None;
        }
    }
    let positive = |v: [f64; 3]| v.iter().all(|x| x.is_finite() && *x > 0.0);
    ColorInfo {
        calibrations: cal,
        analog_balance: c.analog_balance.filter(|v| positive(*v)),
        as_shot_neutral: c.as_shot_neutral.filter(|v| positive(*v)),
        as_shot_white_xy: c.as_shot_white_xy.filter(|v| v.iter().all(|x| *x > 0.0 && *x < 1.0)),
    }
}

/// Decodes a DNG file's raw image and metadata.
pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Sensor> {
    let raw = craft_raw::decode_with(bytes, &raw_limits(limits)).map_err(raw_err)?;
    let samples = raw.cpp;
    let is_cfa = raw.cfa.is_some();
    // LinearRaw: three colour samples, or one for a monochrome camera or scanner (DNG 1.7,
    // ColorPlanes 1: Leica Monochrom, VueScan film scans).
    if !is_cfa && samples != 1 && samples != 3 {
        return Err(RawError::unsupported(format!("LinearRaw data with {samples} samples per pixel")));
    }
    limits.check(raw.width as u64, raw.height as u64, 2 * samples as u64)?;
    let mut warnings = Vec::new();
    let active = rect(raw.active_area);
    if active.is_empty() {
        return Err(RawError::malformed("empty active area"));
    }
    let cfa = match &raw.cfa {
        Some(c) => {
            let c = Cfa { width: c.width, height: c.height, colors: c.pattern.clone(), origin_x: 0, origin_y: 0 };
            if !c.is_bayer() {
                return Err(RawError::unsupported(format!("{}x{} non-Bayer CFA (e.g. X-Trans)", c.height, c.width)));
            }
            Some(c)
        }
        None => None,
    };
    let crop = Rect::new(active.x + raw.crop.x, active.y + raw.crop.y, raw.crop.width, raw.crop.height).intersect(&active);
    let crop = if crop.is_empty() { active } else { crop };
    if (raw.pixel_aspect - 1.0).abs() > 1e-6 {
        warnings.push("non-square pixels (DefaultScale) are not resampled".to_string());
    }
    // Opcode lists: OpcodeList2 gain maps (lens shading) and polynomials (the tone mapping lossy
    // DNGs carry per plane, #8125) are applied; everything else is reported.
    let mut gain_maps = Vec::new();
    let mut polynomials = Vec::new();
    for (list, what) in [(&raw.opcodes.list1, 1), (&raw.opcodes.list2, 2), (&raw.opcodes.list3, 3)] {
        let ops = opcodes::from_list(list);
        if what == 2 {
            gain_maps = ops.gain_maps;
            polynomials = ops.polynomials;
        } else {
            if !ops.gain_maps.is_empty() {
                warnings.push(format!("DNG OpcodeList{what}: GainMap is not applied"));
            }
            if !ops.polynomials.is_empty() {
                warnings.push(format!("DNG OpcodeList{what}: MapPolynomial is not applied"));
            }
        }
        if !ops.skipped.is_empty() {
            warnings.push(format!("DNG OpcodeList{what}: {} not applied", ops.skipped.join(", ")));
        }
    }
    // DNG 1.6 ProfileGainTableMap: the local tone mapping that phones (Samsung Expert RAW) rely on
    // for their default look. Without it the rendering is darker in the shadows.
    if raw.color.profile.gain_table_map.is_some() {
        warnings.push("DNG ProfileGainTableMap (local tone mapping) is not applied".to_string());
    }

    // Floating-point samples are scaled so the highest white level becomes 65535 (no plane is
    // clipped below its own white), and their levels with them.
    let (mut data, unit) = match raw.data {
        craft_raw::RawData::U16(v) => (v, 1.0),
        craft_raw::RawData::F32(v) => {
            let white = raw.white.iter().copied().filter(|v| v.is_finite() && *v > 0.0).fold(0.0f32, f32::max);
            let unit = 65535.0 / if white > 0.0 { white } else { 1.0 };
            (v.into_iter().map(|s| (s * unit).round().clamp(0.0, 65535.0) as u16).collect(), unit)
        }
    };
    let white = [0, 1, 2].map(|s| raw.white.get(s).or_else(|| raw.white.first()).map(|v| v * unit).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(65535.0));
    let b = &raw.black;
    let mut black = if b.values.len() > 1 {
        BlackLevels { rows: b.repeat_rows, cols: b.repeat_cols, values: b.values.iter().map(|v| v * unit).collect(), delta_h: Vec::new(), delta_v: Vec::new() }
    } else {
        BlackLevels::uniform(b.values.first().copied().unwrap_or(0.0) * unit)
    };
    if b.delta_h.len() == active.width {
        black.delta_h = b.delta_h.iter().map(|v| v * unit).collect();
    }
    if b.delta_v.len() == active.height {
        black.delta_v = b.delta_v.iter().map(|v| v * unit).collect();
    }
    // MapPolynomial maps linear reference values: the shared decoder has already applied any
    // LinearizationTable, so the samples are mapped in place.
    if !polynomials.is_empty() {
        opcodes::apply_polynomials(&mut data, raw.width, samples, active, &polynomials, &black, white);
    }

    let color = color_info(&raw.color);
    // A monochrome image has no colour to calibrate: it develops as neutral grey.
    if color.calibrations.is_empty() && (is_cfa || samples != 1) {
        warnings.push("the DNG has no colour matrix; colours are approximate".to_string());
    }
    let baseline_exposure = raw.color.baseline_exposure;
    let model = raw.camera.unique_model.clone().or_else(|| raw.camera.model.clone());
    Ok(Sensor {
        format: RawFormat::Dng,
        make: raw.camera.make.clone(),
        model,
        width: raw.width,
        height: raw.height,
        samples,
        data,
        cfa,
        linearization: None,
        black,
        white,
        active,
        crop,
        color,
        camera_wb: None,
        orientation: raw.orientation.to_exif(),
        baseline_exposure: if baseline_exposure.is_finite() { baseline_exposure.clamp(-10.0, 10.0) } else { 0.0 },
        gain_maps,
        tone_curve: Vec::new(),
        black_point: 0.0,
        warnings,
    })
}
