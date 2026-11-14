//! `photocraft-depth`: monocular depth estimation with Depth Anything V2 Small, run on the CPU
//! through candle (native builds only; on wasm `Model::load` returns [`DepthError::Unsupported`]).
//!
//! The pipeline is [`preprocess`] (resize so the longer side is 518 px and both sides are
//! multiples of 14, ImageNet normalisation, NCHW), the network, and [`postprocess`] (crop, min-max
//! normalise to 0..1, larger = nearer). The weights are not shipped: [`WEIGHTS`] pins the two files
//! the app downloads on request, by URL, size and sha256.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Model;

#[cfg(target_arch = "wasm32")]
pub use stub::Model;

/// Side of the (square) network input in pixels.
pub const MODEL_SIZE: u32 = 518;
/// Patch size of the DINOv2 backbone: both preprocessed sides are multiples of it.
pub const PATCH: u32 = 14;
/// Largest accepted input, in pixels (w * h), so lengths and allocations stay bounded.
pub const MAX_PIXELS: u64 = 1 << 28;

/// ImageNet channel means and standard deviations.
pub const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// One downloadable weight file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeightFile {
    pub name: &'static str,
    pub url: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// Lower-case hex sha256 of the file.
    pub sha256: &'static str,
}

/// The DINOv2 ViT-S/14 backbone and the Depth Anything V2 Small head (fp16 safetensors), at a
/// pinned revision of `pcuenq/candle-depth-anything-v2-small`. Both are Apache-2.0
/// (`LICENSE-depth-anything-v2.txt`).
pub const WEIGHTS: [WeightFile; 2] = [
    WeightFile {
        name: "dinov2_vits14.fp16.safetensors",
        url: "https://huggingface.co/pcuenq/candle-depth-anything-v2-small/resolve/11c7cdb8f744281f9264f36ffb541f00e36fe9b8/dinov2_vits14.fp16.safetensors",
        size: 45_667_104,
        sha256: "ebfaef20f34130315ced79ef159f27acfba48ba271d6a2261483199b02a02983",
    },
    WeightFile {
        name: "depth_anything_v2_vits.fp16.safetensors",
        url: "https://huggingface.co/pcuenq/candle-depth-anything-v2-small/resolve/11c7cdb8f744281f9264f36ffb541f00e36fe9b8/depth_anything_v2_vits.fp16.safetensors",
        size: 49_595_170,
        sha256: "73e34d35b475bc075ac0b061671e5780800ed734c7eba2f702755f756b60b171",
    },
];

/// Why depth estimation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepthError {
    /// Bad input (size, length, weights).
    Invalid(String),
    /// Not available in this build (wasm).
    Unsupported,
    /// The interrupt callback asked to stop.
    Cancelled,
    /// The inference backend failed.
    Backend(String),
}

impl fmt::Display for DepthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DepthError::Invalid(m) => write!(f, "invalid depth input: {m}"),
            DepthError::Unsupported => write!(f, "depth estimation is not supported in this build"),
            DepthError::Cancelled => write!(f, "depth estimation cancelled"),
            DepthError::Backend(m) => write!(f, "depth model error: {m}"),
        }
    }
}

impl std::error::Error for DepthError {}

/// A relative depth map at model resolution: `data` is `width * height` values in 0..1, min-max
/// normalised, larger = nearer.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthMap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

/// An input resized and normalised for the network.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub width: u32,
    pub height: u32,
    /// Planar (NCHW with N = 1) channels: `3 * width * height` normalised values.
    pub data: Vec<f32>,
}

/// The preprocessed size for a `w` x `h` input: the longer side is [`MODEL_SIZE`], the shorter is
/// scaled to match and rounded to a multiple of [`PATCH`] (at least one patch).
pub fn target_size(w: u32, h: u32) -> Result<(u32, u32), DepthError> {
    check_dims(w, h)?;
    let snap = |short: u32, long: u32| -> u32 {
        let v = f64::from(short) * f64::from(MODEL_SIZE) / f64::from(long);
        let p = f64::from(PATCH);
        let n = ((v / p).round() as u32).max(1);
        (n * PATCH).min(MODEL_SIZE)
    };
    Ok(if w >= h { (MODEL_SIZE, snap(h, w)) } else { (snap(w, h), MODEL_SIZE) })
}

fn check_dims(w: u32, h: u32) -> Result<(), DepthError> {
    if w == 0 || h == 0 {
        return Err(DepthError::Invalid("empty image".into()));
    }
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(DepthError::Invalid(format!("image too large ({w}x{h})")));
    }
    Ok(())
}

/// Weights of a separable triangle resampler: for each output index the first source index and
/// the normalised weights. The kernel is widened by the scale when shrinking, so a downscale
/// averages the covered source pixels (area-correct) and an upscale is plain bilinear.
fn axis_weights(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = src as f64 / dst as f64;
    let support = scale.max(1.0);
    (0..dst)
        .map(|o| {
            let centre = (o as f64 + 0.5) * scale;
            let lo = ((centre - support).floor().max(0.0)) as usize;
            let hi = (((centre + support).ceil()) as usize).min(src);
            let mut w: Vec<f32> = (lo..hi).map(|i| (1.0 - ((i as f64 + 0.5 - centre) / support).abs()).max(0.0) as f32).collect();
            let sum: f32 = w.iter().sum();
            if sum > 0.0 {
                w.iter_mut().for_each(|x| *x /= sum);
            } else {
                // Cannot happen for a non-empty range; fall back to the nearest pixel.
                let n = (centre as usize).min(src.saturating_sub(1));
                return (n, vec![1.0]);
            }
            (lo, w)
        })
        .collect()
}

/// Resize interleaved RGB (0..1, `w * h * 3`) to [`target_size`] and normalise with the ImageNet
/// mean and standard deviation into planar channels.
pub fn preprocess(rgb: &[f32], w: u32, h: u32) -> Result<Prepared, DepthError> {
    let (nw, nh) = target_size(w, h)?;
    let (wu, hu, nwu, nhu) = (w as usize, h as usize, nw as usize, nh as usize);
    if rgb.len() != wu * hu * 3 {
        return Err(DepthError::Invalid(format!("expected {} values, got {}", wu * hu * 3, rgb.len())));
    }
    if rgb.iter().any(|v| !v.is_finite()) {
        return Err(DepthError::Invalid("non-finite pixel value".into()));
    }
    let wx = axis_weights(wu, nwu);
    let wy = axis_weights(hu, nhu);
    // Horizontal pass: h rows x nw columns x 3.
    let mut mid = vec![0.0f32; hu * nwu * 3];
    for y in 0..hu {
        let row = rgb.get(y * wu * 3..(y + 1) * wu * 3).unwrap_or(&[]);
        for (x, (start, ws)) in wx.iter().enumerate() {
            let mut acc = [0.0f32; 3];
            for (k, wt) in ws.iter().enumerate() {
                let i = (start + k) * 3;
                if let Some(px) = row.get(i..i + 3) {
                    for c in 0..3 {
                        acc[c] += px[c] * wt;
                    }
                }
            }
            if let Some(o) = mid.get_mut((y * nwu + x) * 3..(y * nwu + x) * 3 + 3) {
                o.copy_from_slice(&acc);
            }
        }
    }
    // Vertical pass, written planar and normalised.
    let plane = nwu * nhu;
    let mut out = vec![0.0f32; 3 * plane];
    for (y, (start, ws)) in wy.iter().enumerate() {
        for x in 0..nwu {
            let mut acc = [0.0f32; 3];
            for (k, wt) in ws.iter().enumerate() {
                if let Some(px) = mid.get(((start + k) * nwu + x) * 3..((start + k) * nwu + x) * 3 + 3) {
                    for c in 0..3 {
                        acc[c] += px[c] * wt;
                    }
                }
            }
            for c in 0..3 {
                if let Some(o) = out.get_mut(c * plane + y * nwu + x) {
                    *o = (acc[c] - MEAN[c]) / STD[c];
                }
            }
        }
    }
    Ok(Prepared { width: nw, height: nh, data: out })
}

/// Turn the network's raw output (`width * height` values, larger = nearer) into a [`DepthMap`]
/// min-max normalised to 0..1. A constant map becomes all zeros.
pub fn postprocess(raw: &[f32], width: u32, height: u32) -> Result<DepthMap, DepthError> {
    check_dims(width, height)?;
    if raw.len() != width as usize * height as usize {
        return Err(DepthError::Invalid("depth output has the wrong size".into()));
    }
    if raw.iter().any(|v| !v.is_finite()) {
        return Err(DepthError::Backend("depth output is not finite".into()));
    }
    let (lo, hi) = raw.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
    let range = hi - lo;
    let data = if range > f32::EPSILON { raw.iter().map(|v| ((v - lo) / range).clamp(0.0, 1.0)).collect() } else { vec![0.0; raw.len()] };
    Ok(DepthMap { width, height, data })
}

#[cfg(target_arch = "wasm32")]
mod stub {
    use super::{DepthError, DepthMap};

    /// The depth model (not available on wasm).
    pub struct Model {
        _private: (),
    }

    impl Model {
        pub fn load(_dinov2: &[u8], _head: &[u8]) -> Result<Model, DepthError> {
            Err(DepthError::Unsupported)
        }

        pub fn estimate(&self, _rgb: &[f32], _w: u32, _h: u32, _interrupt: &dyn Fn(f32) -> bool) -> Result<DepthMap, DepthError> {
            Err(DepthError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_sizes() {
        assert_eq!(target_size(4000, 3000), Ok((518, 392)));
        assert_eq!(target_size(3000, 4000), Ok((392, 518)));
        assert_eq!(target_size(100, 100), Ok((518, 518)));
        assert_eq!(target_size(10_000, 10), Ok((518, 14)));
        assert_eq!(target_size(1, 1), Ok((518, 518)));
        for (w, h) in [(7, 5000), (4000, 3001), (640, 480), (1, 9999)] {
            let (a, b) = target_size(w, h).unwrap();
            assert_eq!(a % PATCH, 0);
            assert_eq!(b % PATCH, 0);
            assert_eq!(a.max(b), MODEL_SIZE);
            assert!(a >= PATCH && b >= PATCH);
        }
    }

    #[test]
    fn bad_input_is_rejected() {
        assert!(matches!(preprocess(&[], 0, 4,), Err(DepthError::Invalid(_))));
        assert!(matches!(preprocess(&[0.0; 5], 2, 1), Err(DepthError::Invalid(_))));
        assert!(matches!(preprocess(&[f32::NAN; 6], 2, 1), Err(DepthError::Invalid(_))));
        assert!(matches!(target_size(u32::MAX, u32::MAX), Err(DepthError::Invalid(_))));
        assert!(postprocess(&[0.0; 3], 2, 2).is_err());
        assert!(postprocess(&[f32::NAN; 4], 2, 2).is_err());
    }

    #[test]
    fn normalisation_values() {
        // A flat mid-grey image normalises to (0.5 - mean) / std in every plane.
        let p = preprocess(&vec![0.5; 8 * 6 * 3], 8, 6).unwrap();
        assert_eq!((p.width, p.height), (518, 392));
        let plane = (p.width * p.height) as usize;
        assert_eq!(p.data.len(), 3 * plane);
        for c in 0..3 {
            let want = (0.5 - MEAN[c]) / STD[c];
            assert!(p.data[c * plane..(c + 1) * plane].iter().all(|v| (v - want).abs() < 1e-4));
        }
    }

    #[test]
    fn downscale_averages() {
        // A black/white checkerboard shrinks to mid-grey (area average), not aliased.
        let (w, h) = (1036u32, 1036u32);
        let mut img = vec![0.0f32; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                if (x + y) % 2 == 0 {
                    let i = ((y * w + x) * 3) as usize;
                    img[i..i + 3].fill(1.0);
                }
            }
        }
        let p = preprocess(&img, w, h).unwrap();
        let want = (0.5 - MEAN[0]) / STD[0];
        let mid = p.data[(p.height as usize / 2) * p.width as usize + p.width as usize / 2];
        assert!((mid - want).abs() < 0.05, "{mid} vs {want}");
    }

    #[test]
    fn postprocess_normalises() {
        let m = postprocess(&[2.0, 4.0, 6.0, 10.0], 2, 2).unwrap();
        assert_eq!(m.data, vec![0.0, 0.25, 0.5, 1.0]);
        let flat = postprocess(&[3.0; 4], 2, 2).unwrap();
        assert!(flat.data.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn weights_are_pinned() {
        for w in WEIGHTS {
            assert_eq!(w.sha256.len(), 64);
            assert!(w.url.starts_with("https://huggingface.co/pcuenq/candle-depth-anything-v2-small/resolve/"));
            assert!(w.url.ends_with(w.name));
            assert!(w.size > 1_000_000);
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn wasm_is_unsupported() {
        assert!(matches!(Model::load(&[], &[]), Err(DepthError::Unsupported)));
    }
}
