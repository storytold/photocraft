//! WebAssembly stub: the plugin sandbox has no ONNX Runtime and no GPU, so
//! every entry point answers with [`MattingError::UnsupportedOnWasm`] — the
//! type surface stays identical so the engine compiles for both targets and
//! the WebAssembly build simply refuses the command with a clear message.

use crate::error::{MattingError, Result};

/// Mirror of the native `ModelVariant`; never branched here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelVariant {
    #[default]
    General,
    Hr,
}

/// Mirror of the native `GpuInfo`; never constructed on wasm.
#[derive(Debug, Clone)]
pub struct GpuInfo {
    name: String,
}

impl GpuInfo {
    /// One-line adapter description, as the engine reports it.
    pub fn describe(&self) -> String {
        self.name.clone()
    }
}

/// Same shape as the native options; ignored here.
#[derive(Debug, Clone, Default)]
pub struct CutoutOptions {
    pub variant: ModelVariant,
    pub device: DevicePref,
}

/// Same shape as the native result; never constructed on wasm.
#[derive(Debug, Clone)]
pub struct CutoutResult {
    pub alpha: Vec<u8>,
    pub provider: String,
    pub gpu: Option<GpuInfo>,
    pub ms: u128,
}

/// Device preference; the wasm build only ever runs the stub.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DevicePref {
    #[default]
    Auto,
    Cpu,
    Gpu(u32),
}

/// Always `Err(UnsupportedOnWasm)`.
pub fn cutout(_rgba: &[u8], _width: u32, _height: u32, _opts: &CutoutOptions) -> Result<CutoutResult> {
    Err(MattingError::UnsupportedOnWasm)
}

/// Always reports the wasm limitation.
pub fn ort_status() -> String {
    "WebAssembly build: no ONNX Runtime".to_owned()
}

/// No-op on wasm.
pub fn release() {}
