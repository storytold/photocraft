//! # photocraft-ml
//!
//! Local neural-network inference for the editor, on [tract](https://github.com/sonos/tract)
//! (pure Rust ONNX): no network, no native runtime, no `unsafe`.
//!
//! * [`segment_subject`]: salient-object matte (BiRefNet-lite, MIT) with a guided-filter edge
//!   refinement so hair and fur keep partial coverage at full resolution.
//! * [`inpaint`]: large-hole inpainting (LaMa, Apache-2.0) on a context window around the hole,
//!   composited back so only the hole changes.
//!
//! Models are **not** in the repository or the binary. They are looked up by [`model_path`] in
//! `PHOTOCRAFT_MODELS_DIR`, else the per-user data folder (see [`models_dir`]); an absent model is
//! [`MlError::ModelMissing`], and callers fall back to the classical algorithms. Licences and
//! sources are listed in `ATTRIBUTION.md` and `docs/ml.md`.
//!
//! Buffers are interleaved normalised `f32` RGB (`0..=1`) and `w × h` masks, so any depth or
//! colour model the caller can sample works.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod guided;
mod inpaint;
mod inpaint_detail;
#[cfg(not(target_arch = "wasm32"))]
mod install;
mod resize;
mod segment;
mod store;

pub use guided::guided_filter;
pub use inpaint::{INPAINT_SIZE, inpaint, inpaint_with};
#[cfg(not(target_arch = "wasm32"))]
pub use install::{download, remove};
pub use segment::{SEGMENT_SIZE, segment_subject, segment_subject_with};
pub use store::{Model, disable_models, model_path, models_dir, models_disabled};

use std::fmt;

/// Why a model could not run. None of these is a bug in the caller's document: the caller reports
/// it, or falls back to a classical algorithm.
#[derive(Debug, Clone, PartialEq)]
pub enum MlError {
    /// The model file is not installed; `path` is where it was looked for.
    ModelMissing { model: Model, path: String },
    /// Inference is not available here (the browser build).
    Unsupported,
    /// The inputs were unusable (empty, mismatched sizes, too large).
    BadInput(String),
    /// The caller cancelled between stages.
    Cancelled,
    /// The model failed to load or run.
    Backend(String),
    /// Downloading, verifying or removing a model file failed.
    Download(String),
}

impl fmt::Display for MlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelMissing { model, path } => write!(f, "the {} model is not installed (looked for {path}); see docs/ml.md", model.label()),
            Self::Unsupported => f.write_str("neural models are not available in this build"),
            Self::BadInput(m) => write!(f, "bad input: {m}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Backend(m) => write!(f, "model error: {m}"),
            Self::Download(m) => write!(f, "model download failed: {m}"),
        }
    }
}

impl std::error::Error for MlError {}

/// Largest image side the helpers accept (rejects absurd sizes before allocating).
pub const MAX_SIDE: usize = 32_768;

/// An interleaved RGB image, normalised `0..=1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    /// `w × h × 3` samples.
    pub px: Vec<f32>,
}

impl Rgb {
    /// An image from `px`, which must hold exactly `w × h × 3` samples.
    pub fn new(w: usize, h: usize, px: Vec<f32>) -> Result<Self, MlError> {
        Self::check_dims(w, h)?;
        if px.len() != w * h * 3 {
            return Err(MlError::BadInput(format!("{} samples for a {w}×{h} RGB image", px.len())));
        }
        Ok(Self { w, h, px })
    }

    pub(crate) fn check_dims(w: usize, h: usize) -> Result<(), MlError> {
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(MlError::BadInput(format!("a {w}×{h} image")));
        }
        Ok(())
    }
}

/// Cancellation hook: asked between stages; `true` stops with [`MlError::Cancelled`].
pub type Cancel<'a> = &'a dyn Fn() -> bool;

pub(crate) fn check_cancel(c: Cancel<'_>) -> Result<(), MlError> {
    if c() { Err(MlError::Cancelled) } else { Ok(()) }
}

#[cfg(not(target_arch = "wasm32"))]
mod backend;

/// The browser build has no models to run.
#[cfg(target_arch = "wasm32")]
mod backend {
    use crate::{MlError, store::Model};

    pub(crate) fn run_with(_: Model, _: &[(&[usize], Vec<f32>)], _: crate::Cancel<'_>) -> Result<Vec<f32>, MlError> {
        Err(MlError::Unsupported)
    }
}
