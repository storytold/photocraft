//! Conservative SDR slider estimates. Analysis never changes white balance or image pixels.
use super::{CameraRaw, luma, white_balance_gains};
use crate::photo_util::{linear_to_srgb, srgb_to_linear};
use serde::Serialize;

/// Bounded analysis input, shared by the editor and automation.
pub const MAX_SAMPLES: usize = 16_384;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTone {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
}

impl AutoTone {
    pub fn apply(self, params: &mut CameraRaw) {
        params.exposure = self.exposure;
        params.contrast = self.contrast;
        params.highlights = self.highlights;
        params.shadows = self.shadows;
        params.whites = self.whites;
        params.blacks = self.blacks;
        params.vibrance = self.vibrance;
        params.saturation = self.saturation;
    }
}

fn quantile(values: &mut [(f32, f32)], fraction: f32) -> f32 {
    values.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let target = values.iter().map(|v| v.1).sum::<f32>() * fraction;
    let mut sum = 0.0;
    for &(value, weight) in values.iter() {
        sum += weight;
        if sum >= target {
            return value;
        }
    }
    values.last().map_or(0.0, |v| v.0)
}

/// Straight, document-encoded RGBA samples; alpha includes selection/filter-mask coverage.
/// Statistics use the filter's working encoding and current WB gains. Existing automatic
/// sliders are deliberately excluded: pressing Auto twice returns the same settings.
pub fn estimate(samples: &[[f32; 4]], current: &CameraRaw) -> Result<AutoTone, String> {
    if samples.is_empty() || samples.len() > MAX_SAMPLES {
        return Err(format!("Camera Raw Auto needs 1–{MAX_SAMPLES} samples"));
    }
    if !current.temperature.is_finite() || !current.tint.is_finite() || current.temperature.abs() > 100.0 || current.tint.abs() > 100.0 {
        return Err("Camera Raw white balance is out of range".into());
    }
    let gains = white_balance_gains(current.temperature, current.tint);
    let norm = luma(gains);
    let mut luminance = Vec::with_capacity(samples.len());
    let mut peaks = Vec::with_capacity(samples.len());
    let (mut chroma, mut chroma_weight) = (0.0, 0.0);
    for &[r, g, b, a] in samples {
        if ![r, g, b, a].iter().all(|v| v.is_finite()) || a <= 0.0 {
            continue;
        }
        let weight = a.min(1.0);
        let mut rgb = [r, g, b];
        for (v, gain) in rgb.iter_mut().zip(gains) {
            *v = linear_to_srgb(srgb_to_linear(v.clamp(0.0, 16.0)) * gain / norm);
        }
        let y = luma(rgb);
        let hi = rgb.into_iter().fold(0.0, f32::max);
        let lo = rgb.into_iter().fold(f32::MAX, f32::min);
        luminance.push((y, weight));
        peaks.push((hi, weight));
        // Near-black and clipped pixels give unreliable saturation estimates.
        if y > 0.03 && hi < 0.98 {
            chroma += (hi - lo) / hi.max(1e-6) * weight;
            chroma_weight += weight;
        }
    }
    if luminance.is_empty() {
        return Err("Camera Raw Auto found no visible finite pixels".into());
    }
    let low = quantile(&mut luminance, 0.01);
    let median = quantile(&mut luminance, 0.5);
    let high = quantile(&mut luminance, 0.99);
    let mut result = AutoTone { exposure: 0.0, contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0, vibrance: 0.0, saturation: 0.0 };
    // Featureless patches and entirely clipped images do not identify a useful exposure.
    if high - low < 0.015 || median <= 0.005 {
        return Ok(result);
    }
    let desired = (srgb_to_linear(0.46) / srgb_to_linear(median).max(1e-6)).log2().clamp(-2.0, 2.0);
    let peak = quantile(&mut peaks, 0.99);
    let headroom = (srgb_to_linear(0.98) / srgb_to_linear(peak).max(1e-6)).log2();
    // Bright-tail protection preserves low-key scenes instead of lifting every median to grey.
    result.exposure = if desired > 0.0 { desired.min(headroom.max(0.0)) } else { desired };
    let expose = |v: f32| linear_to_srgb(srgb_to_linear(v) * 2.0f32.powf(result.exposure));
    let (p01, p10, p90, p99) = (expose(low), expose(quantile(&mut luminance, 0.1)), expose(quantile(&mut luminance, 0.9)), expose(high));
    result.shadows = ((0.18 - p10) * 160.0).clamp(0.0, 35.0);
    result.highlights = ((0.85 - p90) * 170.0).clamp(-50.0, 0.0);
    result.whites = ((0.96 - p99) * 100.0).clamp(-20.0, 15.0);
    result.blacks = ((0.025 - p01) * 150.0).clamp(-15.0, 8.0);
    result.contrast = ((0.65 - (p90 - p10)) * 40.0).clamp(-10.0, 20.0);
    let saturation = chroma / chroma_weight.max(1e-6);
    if chroma_weight > 0.0 && saturation > 0.025 {
        result.vibrance = ((0.35 - saturation) * 80.0).clamp(-12.0, 20.0);
        result.saturation = ((0.3 - saturation) * 20.0).clamp(-12.0, 4.0);
    }
    Ok(result)
}
