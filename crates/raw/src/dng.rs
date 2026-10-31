//! Digital Negative (Adobe DNG Specification 1.7): raw IFD selection, CFA
//! description, linearization, black/white levels, active area, default
//! crop, colour calibration and white balance.

use crate::color::{self, Calibration, ColorInfo, IDENTITY};
use crate::error::{RawError, Result};
use crate::sensor::{BlackLevels, Cfa, JpegLayout, Rect, Sensor, read_plane};
use crate::tiff::{Ifd, Tiff, tag};
use crate::{Limits, RawFormat, opcodes};

const PHOTOMETRIC_CFA: u32 = 32803;
const PHOTOMETRIC_LINEAR_RAW: u32 = 34892;

/// The main raw image: NewSubFileType 0 with CFA or LinearRaw data, largest first.
fn raw_ifd(t: &Tiff, ifds: &[Ifd]) -> Option<Ifd> {
    ifds.iter()
        .filter(|i| t.tag_uint(i, tag::NEW_SUBFILE_TYPE).unwrap_or(0) == 0)
        .filter(|i| matches!(t.tag_uint(i, tag::PHOTOMETRIC), Some(PHOTOMETRIC_CFA | PHOTOMETRIC_LINEAR_RAW)))
        .max_by_key(|i| u64::from(t.tag_uint(i, tag::IMAGE_WIDTH).unwrap_or(0)) * u64::from(t.tag_uint(i, tag::IMAGE_LENGTH).unwrap_or(0)))
        .cloned()
}

fn matrix(t: &Tiff, ifd: &Ifd, tg: u16) -> Option<color::Mat3> {
    let v = t.tag_floats(ifd, tg);
    if v.len() != 9 {
        return None;
    }
    color::from_slice(&v)
}

fn triple(t: &Tiff, ifd: &Ifd, tg: u16) -> Option<[f64; 3]> {
    let v = t.tag_floats(ifd, tg);
    (v.len() == 3 && v.iter().all(|x| x.is_finite() && *x > 0.0)).then(|| [v[0], v[1], v[2]])
}

/// Colour calibration from IFD0.
pub(crate) fn color_info(t: &Tiff, ifd0: &Ifd) -> ColorInfo {
    let mut cal = Vec::new();
    for (cm, cc, fm, ill, default_t) in [
        (tag::COLOR_MATRIX_1, tag::CAMERA_CALIBRATION_1, tag::FORWARD_MATRIX_1, tag::CALIBRATION_ILLUMINANT_1, 2856.0),
        (tag::COLOR_MATRIX_2, tag::CAMERA_CALIBRATION_2, tag::FORWARD_MATRIX_2, tag::CALIBRATION_ILLUMINANT_2, 6504.0),
    ] {
        let Some(color_matrix) = matrix(t, ifd0, cm) else { continue };
        if color::invert(&color_matrix).is_none() {
            continue;
        }
        let temperature = t.tag_uint(ifd0, ill).and_then(color::illuminant_temperature).unwrap_or(default_t);
        cal.push(Calibration {
            temperature,
            color_matrix,
            forward_matrix: matrix(t, ifd0, fm).filter(|m| color::invert(m).is_some()),
            camera_calibration: matrix(t, ifd0, cc).filter(|m| color::invert(m).is_some()).unwrap_or(IDENTITY),
        });
    }
    cal.sort_by(|a, b| a.temperature.total_cmp(&b.temperature));
    if cal.len() == 2 && (cal[0].forward_matrix.is_some() != cal[1].forward_matrix.is_some()) {
        for c in &mut cal {
            c.forward_matrix = None;
        }
    }
    let white_xy = {
        let v = t.tag_floats(ifd0, tag::AS_SHOT_WHITE_XY);
        (v.len() == 2 && v.iter().all(|x| *x > 0.0 && *x < 1.0)).then(|| [v[0], v[1]])
    };
    ColorInfo {
        calibrations: cal,
        analog_balance: triple(t, ifd0, tag::ANALOG_BALANCE),
        as_shot_neutral: triple(t, ifd0, tag::AS_SHOT_NEUTRAL),
        as_shot_white_xy: white_xy,
    }
}

/// CFA pattern of a DNG raw IFD, anchored at the active area origin.
fn cfa(t: &Tiff, ifd: &Ifd, active: &Rect) -> Result<Cfa> {
    let dim = t.tag_uints(ifd, tag::CFA_REPEAT_PATTERN_DIM);
    let (rows, cols) = match dim.as_slice() {
        [r, c] => (*r as usize, *c as usize),
        _ => (2, 2),
    };
    let pattern = t.tag_uints(ifd, tag::CFA_PATTERN);
    if rows == 0 || cols == 0 || rows > 16 || cols > 16 || pattern.len() != rows * cols {
        return Err(RawError::malformed("bad CFA pattern"));
    }
    let planes = match t.tag_uints(ifd, tag::CFA_PLANE_COLOR) {
        v if v.is_empty() => vec![0, 1, 2],
        v => v,
    };
    if planes != [0, 1, 2] {
        return Err(RawError::unsupported(format!("CFA plane colours {planes:?} (only red, green, blue)")));
    }
    if t.tag_uint(ifd, tag::CFA_LAYOUT).unwrap_or(1) != 1 {
        return Err(RawError::unsupported("non-rectangular CFA layout"));
    }
    let colors = pattern
        .into_iter()
        .map(|code| u8::try_from(code).ok().filter(|c| *c <= 2))
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| RawError::unsupported("CFA colours other than red, green and blue"))?;
    let c = Cfa { width: cols, height: rows, colors, origin_x: active.x, origin_y: active.y };
    if !c.is_bayer() {
        return Err(RawError::unsupported(format!("{rows}x{cols} non-Bayer CFA (e.g. X-Trans)")));
    }
    Ok(c)
}

fn rect_from(t: &Tiff, ifd: &Ifd, width: usize, height: usize) -> Rect {
    let full = Rect::new(0, 0, width, height);
    match t.tag_uints(ifd, tag::ACTIVE_AREA).as_slice() {
        [top, left, bottom, right] if bottom > top && right > left => {
            Rect::new(*left as usize, *top as usize, (*right - *left) as usize, (*bottom - *top) as usize).intersect(&full)
        }
        _ => full,
    }
}

fn crop_value(value: f64, name: &str) -> Result<usize> {
    if !value.is_finite() || value < 0.0 {
        return Err(RawError::malformed(format!("invalid DNG {name}")));
    }
    let rounded = value.round();
    let usize_limit = 2.0_f64.powi(usize::BITS as i32);
    if rounded >= usize_limit {
        return Err(RawError::malformed(format!("DNG {name} is not representable")));
    }
    Ok(rounded as usize)
}

/// Decodes a DNG file's raw image and metadata.
pub(crate) fn decode(t: &Tiff, limits: &Limits) -> Result<Sensor> {
    let ifds = t.all_ifds();
    let ifd0 = t.ifd_at(t.first_ifd, 0).ok_or_else(|| RawError::malformed("DNG has no IFD0"))?;
    let raw = raw_ifd(t, &ifds).ok_or_else(|| RawError::malformed("DNG has no raw image"))?;
    let mut warnings = Vec::new();
    let photometric = t.tag_uint(&raw, tag::PHOTOMETRIC).unwrap_or(0);
    let plane = read_plane(t, &raw, limits, JpegLayout::Flat)?;
    let samples = plane.samples;
    let is_cfa = photometric == PHOTOMETRIC_CFA;
    if is_cfa && samples != 1 {
        return Err(RawError::unsupported(format!("CFA data with {samples} samples per pixel")));
    }
    // LinearRaw: three colour samples, or one for a monochrome camera or scanner (DNG 1.7,
    // ColorPlanes 1: Leica Monochrom, VueScan film scans).
    if !is_cfa && samples != 1 && samples != 3 {
        return Err(RawError::unsupported(format!("LinearRaw data with {samples} samples per pixel")));
    }
    let active = rect_from(t, &raw, plane.width, plane.height);
    if active.is_empty() {
        return Err(RawError::malformed("empty active area"));
    }
    let cfa = if is_cfa { Some(cfa(t, &raw, &active)?) } else { None };

    // Default crop, relative to the active area. Each tag has its own default (DNG 1.7.1.0,
    // DefaultCropOrigin and DefaultCropSize): the origin (0, 0), the size the whole image.
    let mut crop = active;
    if raw.has(tag::DEFAULT_CROP_ORIGIN) || raw.has(tag::DEFAULT_CROP_SIZE) {
        // A present tag must hold two values; an absent one takes its default.
        let pair = |tg: u16, default: [f64; 2]| -> Option<[f64; 2]> {
            if !raw.has(tg) {
                return Some(default);
            }
            match t.tag_floats(&raw, tg).as_slice() {
                [a, b] => Some([*a, *b]),
                _ => None,
            }
        };
        let origin = pair(tag::DEFAULT_CROP_ORIGIN, [0.0, 0.0]);
        let size = pair(tag::DEFAULT_CROP_SIZE, [plane.width as f64, plane.height as f64]);
        if let (Some([ox, oy]), Some([cw, ch])) = (origin, size) {
            let (ox, oy, cw, ch) = (
                crop_value(ox, "DefaultCropOrigin"),
                crop_value(oy, "DefaultCropOrigin"),
                crop_value(cw, "DefaultCropSize"),
                crop_value(ch, "DefaultCropSize"),
            );
            let (ox, oy, cw, ch) = (ox?, oy?, cw?, ch?);
            let active_right = active.x.checked_add(active.width).ok_or_else(|| RawError::malformed("DNG active-area right edge overflow"))?;
            let active_bottom = active.y.checked_add(active.height).ok_or_else(|| RawError::malformed("DNG active-area bottom edge overflow"))?;
            let candidate = active.x.checked_add(ox).zip(active.y.checked_add(oy)).and_then(|(x, y)| {
                let right = x.checked_add(cw)?;
                let bottom = y.checked_add(ch)?;
                (x >= active.x && y >= active.y && right <= active_right && bottom <= active_bottom).then_some(Rect::new(x, y, cw, ch))
            });
            if let Some(r) = candidate.filter(|r| !r.is_empty()) {
                crop = r;
            } else {
                warnings.push("out-of-range or empty DNG default crop; active area used".to_string());
            }
        } else {
            warnings.push("incomplete DNG default crop; active area used".to_string());
        }
    }
    let scale = t.tag_floats(&raw, tag::DEFAULT_SCALE);
    if scale.len() == 2 && (scale[0] - scale[1]).abs() > 1e-6 {
        warnings.push("non-square pixels (DefaultScale) are not resampled".to_string());
    }
    // Opcode lists: OpcodeList2 gain maps (lens shading) and polynomials (the tone mapping lossy
    // DNGs carry per plane, #8125) are applied; everything else is reported.
    let mut gain_maps = Vec::new();
    let mut polynomials = Vec::new();
    for (tg, what) in [(tag::OPCODE_LIST_1, 1), (tag::OPCODE_LIST_2, 2), (tag::OPCODE_LIST_3, 3)] {
        let Some(bytes) = raw.get(tg).and_then(|e| t.raw(e)) else { continue };
        let ops = opcodes::parse(bytes);
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
    let gain_table_map = ifd0.has(tag::PROFILE_GAIN_TABLE_MAP) || raw.has(tag::PROFILE_GAIN_TABLE_MAP);
    if gain_table_map {
        warnings.push("DNG ProfileGainTableMap (local tone mapping) is not applied".to_string());
    }

    let max = ((1u32 << plane.bits) - 1) as f32;
    // A LinearizationTable maps integer samples; floating-point data is linear already.
    let linearization =
        raw.get(tag::LINEARIZATION_TABLE).filter(|_| plane.float_scale.is_none()).map(|e| t.uints(e)).filter(|v| !v.is_empty() && v.len() <= 65536);
    let mut linearization: Option<Vec<u16>> = linearization.map(|v| v.into_iter().map(|x| x.min(65535) as u16).collect());
    let white_default = match &linearization {
        Some(l) => f32::from(l.iter().copied().max().unwrap_or(u16::MAX)),
        None => max,
    };
    // Floating-point levels are in the floats' units: scale them as the samples were.
    let unit = plane.float_scale.unwrap_or(1.0);
    let wl = t.tag_floats(&raw, tag::WHITE_LEVEL);
    let white = [0, 1, 2].map(|s| wl.get(s).or_else(|| wl.first()).map(|v| *v as f32 * unit).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(white_default));

    let (rows, cols) = match t.tag_uints(&raw, tag::BLACK_LEVEL_REPEAT_DIM).as_slice() {
        [r, c] if (1..=16).contains(r) && (1..=16).contains(c) => (*r as usize, *c as usize),
        _ => (1, 1),
    };
    let bl = t.tag_floats(&raw, tag::BLACK_LEVEL);
    let mut black = if bl.len() == rows * cols * samples {
        BlackLevels { rows, cols, values: bl.iter().map(|v| *v as f32 * unit).collect(), delta_h: Vec::new(), delta_v: Vec::new() }
    } else {
        BlackLevels::uniform(bl.first().copied().unwrap_or(0.0) as f32 * unit)
    };
    let dh = t.tag_floats(&raw, tag::BLACK_LEVEL_DELTA_H);
    if dh.len() == active.width {
        black.delta_h = dh.iter().map(|v| *v as f32 * unit).collect();
    }
    let dv = t.tag_floats(&raw, tag::BLACK_LEVEL_DELTA_V);
    if dv.len() == active.height {
        black.delta_v = dv.iter().map(|v| *v as f32 * unit).collect();
    }

    // MapPolynomial maps linear reference values: linearize the samples first (the table is then
    // done with), and map them in place.
    let mut data = plane.data;
    if !polynomials.is_empty() {
        if let Some(l) = linearization.take() {
            for v in &mut data {
                *v = l.get(usize::from(*v)).or_else(|| l.last()).copied().unwrap_or(*v);
            }
        }
        opcodes::apply_polynomials(&mut data, plane.width, samples, active, &polynomials, &black, white);
    }

    let color = color_info(t, &ifd0);
    // A monochrome image has no colour to calibrate: it develops as neutral grey.
    if color.calibrations.is_empty() && (is_cfa || samples != 1) {
        warnings.push("the DNG has no colour matrix; colours are approximate".to_string());
    }
    // The DNG specification applies ProfileGainTableMap first, in linear light, then this curve.
    // Profiles that carry both (Apple ProRAW, Pixel) build the curve for gained data: without the
    // map it crushes the shadows (an iPhone 12 Pro file: mean dE against its own preview 15.7 with
    // no curve, 24.2 with it), so the curve waits until the map is applied.
    let tone_curve = match ifd0.get(tag::PROFILE_TONE_CURVE) {
        Some(_) if gain_table_map => {
            warnings.push("DNG ProfileToneCurve is not applied without its ProfileGainTableMap".to_string());
            Vec::new()
        }
        Some(e) => profile_tone_curve(&t.floats(e)).unwrap_or_else(|| {
            warnings.push("malformed DNG ProfileToneCurve; no tone curve applied".to_string());
            Vec::new()
        }),
        None => Vec::new(),
    };
    let baseline_exposure = t.tag_floats(&ifd0, tag::BASELINE_EXPOSURE).first().copied().unwrap_or(0.0)
        + t.tag_floats(&raw, tag::BASELINE_EXPOSURE_OFFSET).first().copied().unwrap_or(0.0);
    let model = t.tag_ascii(&ifd0, tag::UNIQUE_CAMERA_MODEL).or_else(|| t.tag_ascii(&ifd0, tag::MODEL));
    Ok(Sensor {
        format: RawFormat::Dng,
        make: t.tag_ascii(&ifd0, tag::MAKE),
        model,
        width: plane.width,
        height: plane.height,
        samples,
        data,
        cfa,
        linearization,
        black,
        white,
        active,
        crop,
        color,
        camera_wb: None,
        orientation: crate::tiff::orientation(t.tag_uint(&ifd0, tag::ORIENTATION)),
        baseline_exposure: if baseline_exposure.is_finite() { baseline_exposure.clamp(-10.0, 10.0) } else { 0.0 },
        gain_maps,
        tone_curve,
        black_point: 0.0,
        warnings,
    })
}

/// DNG ProfileToneCurve (DNG 1.2.0.0): `[input, output]` pairs in linear gamma, both in 0–1,
/// starting at (0, 0) and ending at (1, 1); the camera profile's default tone curve (Sigma,
/// DJI and phone DNGs carry one). It is applied like a measured camera profile's curve
/// ([`crate::Sensor::tone_curve`]: per channel, after exposure).
/// The specification interpolates the samples with a cubic spline, so the curve is returned
/// as that spline ([`spline_resampled`]), sampled densely enough that the log-linear
/// interpolation of [`crate::develop::tone`] between samples follows it.
/// `None` when the values are not such a curve: inputs must strictly increase and outputs must
/// not decrease (the Camera Raw previews invert the curve).
fn profile_tone_curve(v: &[f64]) -> Option<Vec<[f32; 2]>> {
    let (pairs, rest) = v.as_chunks::<2>();
    if pairs.len() < 2 || !rest.is_empty() {
        return None;
    }
    let curve: Vec<[f32; 2]> = pairs.iter().map(|[x, y]| [*x as f32, *y as f32]).collect();
    let in_range = curve.iter().flatten().all(|c| c.is_finite() && (0.0..=1.0).contains(c));
    let increasing = curve.windows(2).all(|w| w[1][0] > w[0][0] && w[1][1] >= w[0][1]);
    let (first, last) = (curve.first()?, curve.last()?);
    let ends = first[0] == 0.0 && first[1] == 0.0 && last[0] == 1.0 && last[1] == 1.0;
    let identity = curve.iter().all(|[x, y]| x == y);
    (in_range && increasing && ends).then(|| if identity { Vec::new() } else { spline_resampled(&curve) })
}

/// Samples of the natural cubic spline (zero second derivative at both ends; the DNG
/// specification asks for a cubic spline without naming end conditions) through `knots`, which
/// have strictly increasing inputs: 1025 points spaced evenly in the square root of the input,
/// so the shadows, where tone curves are steep, get most of them, plus the knots themselves at
/// their own outputs. A spline can overshoot between knots, so each sample is kept between its
/// two knots' outputs (and in 0–1, non-decreasing).
fn spline_resampled(knots: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = knots.len();
    let x: Vec<f64> = knots.iter().map(|k| f64::from(k[0])).collect();
    let y: Vec<f64> = knots.iter().map(|k| f64::from(k[1])).collect();
    // Second derivatives m: a tridiagonal system (Thomas algorithm), m[0] = m[n-1] = 0.
    let mut m = vec![0.0f64; n];
    if n > 2 {
        let mut c = vec![0.0f64; n];
        let mut d = vec![0.0f64; n];
        for i in 1..n - 1 {
            let (h0, h1) = (x[i] - x[i - 1], x[i + 1] - x[i]);
            let rhs = 6.0 * ((y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0);
            let diag = 2.0 * (h0 + h1) - h0 * c[i - 1];
            c[i] = h1 / diag;
            d[i] = (rhs - h0 * d[i - 1]) / diag;
        }
        for i in (1..n - 1).rev() {
            m[i] = d[i] - c[i] * m[i + 1];
        }
    }
    let eval = |v: f64| -> f64 {
        let i = x.partition_point(|&k| k <= v).clamp(1, n - 1);
        let (x0, x1, h) = (x[i - 1], x[i], x[i] - x[i - 1]);
        let (a, b) = ((x1 - v) / h, (v - x0) / h);
        let s = a * y[i - 1] + b * y[i] + ((a * a * a - a) * m[i - 1] + (b * b * b - b) * m[i]) * h * h / 6.0;
        // A spline overshoots between steep and flat knots (a shoulder): keep each sample between
        // its two knots' outputs, so no knot is raised and the curve stays non-decreasing.
        s.clamp(y[i - 1].min(y[i]), y[i - 1].max(y[i]))
    };
    const SAMPLES: usize = 1025;
    let mut xs: Vec<f64> = (0..SAMPLES).map(|i| (i as f64 / (SAMPLES - 1) as f64).powi(2)).collect();
    xs.extend(&x);
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let mut out = Vec::with_capacity(xs.len());
    let mut floor = 0.0f32;
    for v in xs {
        let o = match x.binary_search_by(|k| k.total_cmp(&v)) {
            Ok(i) => y[i],
            Err(_) => eval(v),
        };
        floor = floor.max(o.clamp(0.0, 1.0) as f32);
        out.push([v as f32, floor]);
    }
    out
}
