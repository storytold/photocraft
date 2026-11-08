//! A raw frame binned down for previews: each `factor × factor` block of the crop becomes one
//! LinearRaw pixel holding the block's mean of every colour, normalized and black-subtracted as
//! [`crate::develop_sensor`] reads them. Developing it runs the same pipeline as the full frame
//! (white balance, highlight clip, colour, exposure, Shadows / Highlights / Clarity / Texture /
//! Dehaze, black point, tone curve) on a frame `factor²` times smaller, with no demosaic: what
//! the open-time Camera Raw dialog needs to show a raw-stage setting as Open will develop it.

use crate::develop::{Scale, gain_row};
use crate::error::{RawError, Result};
use crate::par;
use crate::sensor::{BlackLevels, Rect, Sensor};

/// `s` binned by `factor` (see the module docs): `ceil(width / factor) × ceil(height / factor)`
/// pixels of the crop, with the same colour, white balance, orientation, baseline exposure, tone
/// curve and black point. The blocks of the last row and column are moved back inside the crop
/// rather than cut short, so every block holds whole CFA repeats and every colour.
///
/// A factor smaller than the CFA repeat (or 1) gives an unchanged copy of `s`: such a frame is
/// already preview-sized.
pub fn binned(s: &Sensor, factor: usize) -> Result<Sensor> {
    let c = s.crop;
    let samples = s.width.checked_mul(s.height).and_then(|n| n.checked_mul(s.samples));
    if c.is_empty()
        || c.x.checked_add(c.width).is_none_or(|x| x > s.width)
        || c.y.checked_add(c.height).is_none_or(|y| y > s.height)
        || samples.is_none_or(|n| s.data.len() < n)
    {
        return Err(RawError::malformed("crop lies outside the sensor data"));
    }
    if s.samples != 1 && s.samples != 3 {
        return Err(RawError::unsupported(format!("{} samples per pixel", s.samples)));
    }
    let repeat = s.cfa.as_ref().map_or(1, |cfa| cfa.width.max(cfa.height));
    if s.cfa.as_ref().is_some_and(|cfa| cfa.width == 0 || cfa.height == 0 || cfa.width > 16 || cfa.height > 16 || s.samples != 1) {
        return Err(RawError::malformed("invalid sensor CFA pattern"));
    }
    let k = factor;
    if k < 2 || k < repeat || c.width < k || c.height < k {
        return Ok(s.clone());
    }
    let (bw, bh) = (c.width.div_ceil(k), c.height.div_ceil(k));
    let mono = s.is_monochrome();
    let out_samples = if mono { 1 } else { 3 };
    let sc = Scale::new(s);
    // The start of block `b` of `n` (moved back inside for the last one).
    let start = |b: usize, n: usize| (b * k).min(n - k);
    let rows = par::map(bh, |by| {
        let y0 = c.y + start(by, c.height);
        let mut sum = vec![[0.0f64; 3]; bw];
        let mut count = vec![[0u32; 3]; bw];
        for y in y0..y0 + k {
            // Lens-shading gains of this row, per sample (CFA data: one).
            let gains: Vec<Option<Vec<f32>>> = (0..s.samples).map(|n| gain_row(s, y, c.x, c.width, n)).collect();
            for (bx, (sum, count)) in sum.iter_mut().zip(count.iter_mut()).enumerate() {
                let x0 = start(bx, c.width);
                for i in x0..x0 + k {
                    let x = c.x + i;
                    for n in 0..s.samples {
                        // Clipped to 0…1 as the develop clips each sample (its white-balance
                        // multipliers are at least 1, so clipping first changes nothing).
                        let mut v = sc.value(x, y, n).clamp(0.0, 1.0);
                        if let Some(g) = gains.get(n).and_then(|g| g.as_ref()).and_then(|g| g.get(i)) {
                            v = (v * g).clamp(0.0, 1.0);
                        }
                        let ch = match &s.cfa {
                            Some(cfa) => usize::from(cfa.color(x, y)).min(2),
                            None => n,
                        };
                        sum[ch] += f64::from(v);
                        count[ch] += 1;
                    }
                }
            }
        }
        sum.iter()
            .zip(&count)
            .flat_map(|(sum, count)| {
                let mean = |ch: usize| if count[ch] > 0 { sum[ch] / f64::from(count[ch]) } else { 0.0 };
                let px = [mean(0), mean(1), mean(2)];
                px.into_iter().take(out_samples).map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16).collect::<Vec<_>>()
            })
            .collect::<Vec<u16>>()
    });
    let whole = Rect::new(0, 0, bw, bh);
    Ok(Sensor {
        format: s.format,
        make: s.make.clone(),
        model: s.model.clone(),
        width: bw,
        height: bh,
        samples: out_samples,
        data: rows.concat(),
        cfa: None,
        linearization: None,
        black: BlackLevels::uniform(0.0),
        white: [65535.0; 3],
        active: whole,
        crop: whole,
        color: s.color.clone(),
        camera_wb: s.camera_wb,
        orientation: s.orientation,
        baseline_exposure: s.baseline_exposure,
        gain_maps: Vec::new(),
        tone_curve: s.tone_curve.clone(),
        black_point: s.black_point,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cfa, ColorInfo, DevelopOptions, RawFormat, develop_sensor};

    /// An RGGB sensor of a smooth scene (black 64, white 4000) whose top-right corner saturates.
    fn sensor(w: usize, h: usize) -> Sensor {
        let data = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                let tint = [1.0, 0.8, 0.6][usize::from(Cfa::bayer("RGGB").unwrap().color(i % w, i / w))];
                (64.0 + 3936.0 * (0.05 + 1.2 * x * (1.0 - y)) * tint).min(4000.0) as u16
            })
            .collect();
        Sensor {
            format: RawFormat::Dng,
            make: None,
            model: None,
            width: w,
            height: h,
            samples: 1,
            data,
            cfa: Cfa::bayer("RGGB"),
            linearization: None,
            black: BlackLevels::uniform(64.0),
            white: [4000.0; 3],
            active: Rect::new(0, 0, w, h),
            crop: Rect::new(0, 0, w, h),
            color: ColorInfo::default(),
            camera_wb: Some([2.0, 1.0, 1.5]),
            orientation: 1,
            baseline_exposure: 0.0,
            gain_maps: Vec::new(),
            tone_curve: vec![[0.18, 0.3], [1.0, 1.0]],
            black_point: 0.0,
            warnings: Vec::new(),
        }
    }

    /// The binned develop matches the full develop box-averaged in linear light, also where the
    /// default develop clips and a strong negative exposure brings the highlights back.
    #[test]
    fn a_binned_frame_develops_like_the_full_one_box_averaged() {
        let (w, h, k) = (96, 64, 4);
        let s = sensor(w, h);
        let b = binned(&s, k).unwrap();
        assert_eq!((b.width, b.height, b.samples, b.cfa.is_none()), (w / k, h / k, 3, true));
        for exposure in [0.0, -3.0, 1.5] {
            let opts = DevelopOptions { exposure, ..Default::default() };
            let full = develop_sensor(&s, &opts).unwrap();
            let small = develop_sensor(&b, &opts).unwrap();
            assert_eq!((small.width as usize, small.height as usize), (w / k, h / k));
            let lin = |v: u16| (f64::from(v) / 65535.0).powf(1.8);
            let mut worst = 0.0f64;
            for by in 1..h / k - 1 {
                for bx in 1..w / k - 1 {
                    for ch in 0..3 {
                        let mut acc = 0.0;
                        for y in by * k..(by + 1) * k {
                            for x in bx * k..(bx + 1) * k {
                                acc += lin(full.rgb[(y * w + x) * 3 + ch]);
                            }
                        }
                        let a = acc / (k * k) as f64;
                        let b = lin(small.rgb[(by * (w / k) + bx) * 3 + ch]);
                        worst = worst.max((a.powf(1.0 / 1.8) - b.powf(1.0 / 1.8)).abs());
                    }
                }
            }
            assert!(worst < 0.03, "exposure {exposure}: worst gamma-domain difference {worst}");
        }
    }

    #[test]
    fn small_factors_and_bad_frames_are_handled() {
        let s = sensor(16, 12);
        assert_eq!(binned(&s, 1).unwrap().data, s.data, "factor 1: unchanged");
        // The last row and column of blocks stay whole.
        let b = binned(&s, 5).unwrap();
        assert_eq!((b.width, b.height), (4, 3));
        let mut bad = s.clone();
        bad.data.truncate(10);
        assert!(binned(&bad, 4).is_err());
        let mut bad = s;
        bad.crop = Rect::new(10, 0, 100, 12);
        assert!(binned(&bad, 4).is_err());
    }
}
