//! The candle (CPU) implementation of the depth model.

use std::sync::Arc;

use candle_core::{DType, Device, Module, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::depth_anything_v2::{DepthAnythingV2, DepthAnythingV2Config};
use candle_transformers::models::dinov2;

use crate::{DepthError, DepthMap, MODEL_SIZE, postprocess, preprocess};

fn be(e: candle_core::Error) -> DepthError {
    DepthError::Backend(e.to_string())
}

/// Depth Anything V2 Small on the CPU. Weights are f32 in memory (the files are fp16).
pub struct Model {
    net: DepthAnythingV2,
    device: Device,
}

impl Model {
    /// Build the model from the DINOv2 backbone and the depth head safetensors files (the files
    /// of [`crate::WEIGHTS`], in that order). Nothing is memory-mapped.
    pub fn load(dinov2: &[u8], head: &[u8]) -> Result<Model, DepthError> {
        if dinov2.is_empty() || head.is_empty() {
            return Err(DepthError::Invalid("empty weight file".into()));
        }
        let device = Device::Cpu;
        let vb_dino = VarBuilder::from_buffered_safetensors(dinov2.to_vec(), DType::F32, &device).map_err(be)?;
        let vb_head = VarBuilder::from_buffered_safetensors(head.to_vec(), DType::F32, &device).map_err(be)?;
        Self::from_var_builders(vb_dino, vb_head, device)
    }

    /// Build from prepared variable builders (tests use all-zero weights).
    pub fn from_var_builders(vb_dino: VarBuilder, vb_head: VarBuilder, device: Device) -> Result<Model, DepthError> {
        let backbone = dinov2::vit_small(vb_dino).map_err(be)?;
        let net = DepthAnythingV2::new(Arc::new(backbone), DepthAnythingV2Config::vit_small(), vb_head).map_err(be)?;
        Ok(Model { net, device })
    }

    /// Estimate relative depth for interleaved RGB (0..1, `w * h * 3`). The result is at model
    /// resolution (longer side 518, both sides multiples of 14), 0..1, larger = nearer.
    ///
    /// `interrupt` receives progress (0..1) and returns true to cancel; the forward pass itself
    /// cannot be interrupted, so it is checked before and after it.
    pub fn estimate(&self, rgb: &[f32], w: u32, h: u32, interrupt: &dyn Fn(f32) -> bool) -> Result<DepthMap, DepthError> {
        if interrupt(0.0) {
            return Err(DepthError::Cancelled);
        }
        let prep = preprocess(rgb, w, h)?;
        if interrupt(0.1) {
            return Err(DepthError::Cancelled);
        }
        let (pw, ph, side) = (prep.width as usize, prep.height as usize, MODEL_SIZE as usize);
        // The network input is square (518 x 518): pad by repeating the last row and column, then
        // crop the output back.
        let mut sq = vec![0.0f32; 3 * side * side];
        for c in 0..3 {
            for y in 0..side {
                let sy = y.min(ph - 1);
                for x in 0..side {
                    let sx = x.min(pw - 1);
                    if let (Some(o), Some(v)) = (sq.get_mut(c * side * side + y * side + x), prep.data.get(c * pw * ph + sy * pw + sx)) {
                        *o = *v;
                    }
                }
            }
        }
        let input = Tensor::from_vec(sq, (1, 3, side, side), &self.device).map_err(be)?;
        let out = self.net.forward(&input).map_err(be)?;
        if interrupt(0.95) {
            return Err(DepthError::Cancelled);
        }
        let flat = out.flatten_all().map_err(be)?.to_vec1::<f32>().map_err(be)?;
        if flat.len() != side * side {
            return Err(DepthError::Backend(format!("unexpected output size {}", flat.len())));
        }
        let mut cropped = Vec::with_capacity(pw * ph);
        for y in 0..ph {
            if let Some(row) = flat.get(y * side..y * side + pw) {
                cropped.extend_from_slice(row);
            }
        }
        postprocess(&cropped, prep.width, prep.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_weights() {
        assert!(matches!(Model::load(&[], &[1]), Err(DepthError::Invalid(_))));
        assert!(Model::load(b"not safetensors", b"nope").is_err());
    }

    #[test]
    #[ignore = "builds and runs the full network on zero weights (slow)"]
    fn pipeline_with_zero_weights() {
        let dev = Device::Cpu;
        let vb = VarBuilder::zeros(DType::F32, &dev);
        let model = Model::from_var_builders(vb.clone(), vb, dev).unwrap();
        let (w, h) = (64u32, 48u32);
        let rgb = vec![0.5f32; (w * h * 3) as usize];
        let map = model.estimate(&rgb, w, h, &|_| false).unwrap();
        assert_eq!((map.width, map.height), (518, 392));
        assert_eq!(map.data.len(), 518 * 392);
        assert!(map.data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(matches!(model.estimate(&rgb, w, h, &|_| true), Err(DepthError::Cancelled)));
    }

    #[test]
    #[ignore = "needs the real weights in PHOTOCRAFT_DEPTH_WEIGHTS_DIR"]
    fn real_weights_if_present() {
        let Some(dir) = std::env::var_os("PHOTOCRAFT_DEPTH_WEIGHTS_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        let (Ok(a), Ok(b)) = (std::fs::read(dir.join(crate::WEIGHTS[0].name)), std::fs::read(dir.join(crate::WEIGHTS[1].name))) else {
            return;
        };
        let model = Model::load(&a, &b).unwrap();
        // A left-to-right gradient: the map must be finite and span 0..1.
        let (w, h) = (128u32, 96u32);
        let mut rgb = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                rgb.extend([x as f32 / w as f32; 3]);
            }
        }
        let map = model.estimate(&rgb, w, h, &|_| false).unwrap();
        assert!(map.data.iter().all(|v| v.is_finite()));
        let (lo, hi) = map.data.iter().fold((1.0f32, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 0.5);
    }
}
