//! # photocraft-matting
//!
//! BiRefNet neural background removal for PhotoCraft, running on
//! [ONNX Runtime](https://onnxruntime.ai) inside the application process.
//!
//! ## Why this is not a WebAssembly plug-in
//!
//! PhotoCraft's plug-in ABI v1 (`docs/plugins.md`) runs modules in the `wasmi`
//! sandbox: **no imports at all** (no file system, no network, no clock), no
//! SIMD, a 50 M-instruction fuel budget per band and a 60 s wall-clock budget.
//! A 200 M-parameter segmentation network cannot load its weights (there is no
//! file access), cannot reach a GPU (the sandbox has no device model), and
//! would burn the instruction budget on a single convolution. So the model is
//! integrated the way PhotoCraft integrates every other capability: a native
//! crate plus an engine command (`layer.removeBackgroundAI` in
//! `crates/engine/src/matting_cmds.rs`). The WebAssembly build keeps compiling —
//! this crate degrades to [`MattingError::UnsupportedOnWasm`] there.
//!
//! ## GPU selection (picking the adapter that fits the job)
//!
//! On native Windows the session is created with the **DirectML execution
//! provider**, Microsoft's D3D12-based inference backend: one API covers
//! NVIDIA (CUDA-class), AMD and Intel GPUs, which is the same path Microsoft's
//! own ML tooling takes. The DXGI adapter list is enumerated first
//! ([`device::enumerate_gpus`]); the adapter with the most dedicated video
//! memory wins and its index becomes the DirectML device id, so on a machine
//! with several GPUs the discrete one is used. If DirectML cannot be
//! initialised (missing DLL, no D3D12 device, software adapter only) the
//! session is rebuilt with the CPU provider, and the caller is told exactly
//! which provider won — never a silent fallback. Every call reports the GPU
//! name, vendor, the execution provider and real timings.
//!
//! ## Runtime files
//!
//! With `ort`'s `load-dynamic` feature nothing is linked at build time; at
//! runtime the loader looks for `onnxruntime.dll` (and, for DirectML,
//! `DirectML.dll` + `onnxruntime_providers_shared.dll`) in the executable's
//! directory first. `bat/deploy_matting.bat` copies the DLL set next to the
//! built binaries and the model directory `PhotoCraftData/models/matting/`, so the
//! feature works out of the box after one double click.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod error;

#[cfg(not(target_arch = "wasm32"))]
mod device;
#[cfg(not(target_arch = "wasm32"))]
mod postprocess;
#[cfg(not(target_arch = "wasm32"))]
mod preprocess;
#[cfg(not(target_arch = "wasm32"))]
mod session;
#[cfg(target_arch = "wasm32")]
mod wasm;

pub use error::{MattingError, Result};

#[cfg(not(target_arch = "wasm32"))]
pub use device::{GpuInfo, Vendor, enumerate_gpus, pick_gpu};
#[cfg(not(target_arch = "wasm32"))]
pub use session::{CutoutOptions, CutoutResult, DevicePref, ModelVariant, cutout, model_candidates, ort_status, release};

#[cfg(target_arch = "wasm32")]
pub use wasm::{CutoutOptions, CutoutResult, DevicePref, GpuInfo, ModelVariant, cutout, ort_status, release};

/// Default inference resolution of the BiRefNet-general export (the model is
/// resolution-agnostic; 1024 is the fast default, 2048 the sharp one).
pub const DEFAULT_RESOLUTION: u32 = 1024;
