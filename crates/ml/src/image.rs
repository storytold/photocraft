use photocraft_algo::segment::RgbImage;
use photocraft_raster::Interrupt;

use crate::{Error, Result, check};

const MAX_PIXELS: usize = 2048 * 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaskKind {
    /// A matting model's continuous opacity, interpolated without thresholding.
    Alpha,
    /// SAM logits: interpolate at destination resolution, then threshold at zero.
    Logits,
}

#[derive(Clone, Debug)]
pub struct AlphaMask {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
    pub kind: MaskKind,
}

impl AlphaMask {
    pub fn validate(&self) -> Result<()> {
        let n = self.width.checked_mul(self.height).filter(|n| *n > 0 && *n <= MAX_PIXELS);
        if n != Some(self.values.len()) || self.values.iter().any(|v| !v.is_finite()) {
            return Err(Error::Inference("invalid mask dimensions or non-finite values".into()));
        }
        Ok(())
    }

    /// Bilinear, half-pixel centres, with border clamping. Coordinates are normalized to the
    /// source image, so non-square documents and prompts use the same mapping.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        bilinear(self.width, self.height, x, y, |i| self.values.get(i).copied().unwrap_or(0.0))
    }

    pub fn coverage(&self, x: f32, y: f32) -> u8 {
        let v = self.sample(x, y);
        match self.kind {
            MaskKind::Alpha => (v.clamp(0.0, 1.0) * 255.0).round() as u8,
            MaskKind::Logits => {
                if v > 0.0 {
                    255
                } else {
                    0
                }
            }
        }
    }
}

fn bilinear(w: usize, h: usize, x: f32, y: f32, at: impl Fn(usize) -> f32) -> f32 {
    if w.checked_mul(h).is_none_or(|n| n == 0 || n > MAX_PIXELS) || !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let x = (x * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (y * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let a = at(y0 * w + x0) * (1.0 - fx) + at(y0 * w + x1) * fx;
    let b = at(y1 * w + x0) * (1.0 - fx) + at(y1 * w + x1) * fx;
    a * (1.0 - fy) + b * fy
}

/// Model RGB is sRGB in 0..=1, resized to its training square and ImageNet-normalized NCHW.
/// The engine is responsible for converting the document profile before reaching this seam.
pub fn normalize(image: &RgbImage, side: usize, ctl: &Interrupt<'_>) -> Result<Vec<f32>> {
    let n = image.w.checked_mul(image.h).filter(|n| *n > 0 && *n <= MAX_PIXELS);
    let plane = side.checked_mul(side).filter(|n| *n > 0 && *n <= MAX_PIXELS);
    if n != Some(image.px.len()) || plane.is_none() || image.px.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::Input("invalid image dimensions or values".into()));
    }
    let plane = plane.ok_or_else(|| Error::Input("invalid input size".into()))?;
    let mut out = vec![0.0; plane * 3];
    let mean = [0.485, 0.456, 0.406];
    let std = [0.229, 0.224, 0.225];
    for (c, channel) in out.chunks_exact_mut(plane).enumerate() {
        for (y, row) in channel.chunks_exact_mut(side).enumerate() {
            check(ctl)?;
            for (x, dst) in row.iter_mut().enumerate() {
                let v = bilinear(image.w, image.h, (x as f32 + 0.5) / side as f32, (y as f32 + 0.5) / side as f32, |i| {
                    image.px.get(i).and_then(|p| p.get(c)).copied().unwrap_or(0.0)
                });
                *dst = (v.clamp(0.0, 1.0) - mean.get(c).copied().unwrap_or(0.0)) / std.get(c).copied().unwrap_or(1.0);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_rgb_uses_channels_first_and_keeps_aspect_mapping() {
        let im = RgbImage { w: 2, h: 1, px: vec![[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]] };
        let v = normalize(&im, 2, &Interrupt::NONE).unwrap();
        assert_eq!(v.len(), 12);
        assert!((v[0] - (1.0 - 0.485) / 0.229).abs() < 1e-6);
        assert!((v[5] - (1.0 - 0.456) / 0.224).abs() < 1e-6);
        assert_eq!(v[0], v[2]);
        assert_eq!(v[8], v[11]);
    }
    #[test]
    fn matting_preserves_fractional_coverage_and_sam_thresholds_after_resize() {
        let mut m = AlphaMask { width: 2, height: 1, values: vec![0.0, 1.0], kind: MaskKind::Alpha };
        assert_eq!(m.coverage(0.5, 0.5), 128);
        assert_eq!(m.coverage(0.0, 0.5), 0);
        assert_eq!(m.coverage(1.0, 0.5), 255);
        m.kind = MaskKind::Logits;
        m.values = vec![-2.0, 2.0];
        assert_eq!(m.coverage(0.75, 0.5), 255);
        assert_eq!(m.coverage(0.25, 0.5), 0);
    }
    #[test]
    fn malformed_tensors_and_cancellation_fail_gracefully() {
        let im = RgbImage { w: usize::MAX, h: 2, px: vec![] };
        assert!(normalize(&im, 2048, &Interrupt::NONE).is_err());
        let m = AlphaMask { width: 1, height: 1, values: vec![f32::NAN], kind: MaskKind::Alpha };
        assert!(m.validate().is_err());
        let im = RgbImage { w: 1, h: 1, px: vec![[0.0; 3]] };
        assert!(matches!(normalize(&im, 2, &Interrupt::cancel_only(&|| true)), Err(Error::Cancelled)));
    }
}
