//! Errors for the matting crate. Nothing here panics: every failure is a
//! value the engine command can turn into a user-visible message.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MattingError {
    /// The ONNX Runtime shared library could not be found. With `load-dynamic`
    /// the loader searches the executable's directory (then `ORT_DYLIB_PATH`).
    #[error("ONNX Runtime library (onnxruntime.dll) not found next to the application; run the deploy script (bat/deploy_matting.bat)")]
    OrtLibraryMissing,

    /// No model file at any of the searched locations.
    #[error("matting model not found; searched: {}", searched.join(", "))]
    ModelNotFound { searched: Vec<String> },

    /// The session could not be created (corrupt model, provider failure).
    #[error("failed to create the ONNX session: {0}")]
    Session(String),

    /// Inference itself failed.
    #[error("inference failed: {0}")]
    Inference(String),

    /// The pixel buffer does not match `width * height * channels`.
    #[error("bad pixel buffer: expected {expected} samples for {width}x{height}x{channels}, got {got}")]
    BadBuffer { expected: usize, got: usize, width: u32, height: u32, channels: u32 },

    /// Neither DirectML nor the CPU provider could serve the model.
    #[error("no execution provider available (DirectML and CPU both failed: {detail})")]
    NoProvider { detail: String },

    /// WebAssembly build: the sandbox has no ONNX Runtime and no GPU.
    #[error("Neural matting needs the native build (the WebAssembly build has no ONNX Runtime)")]
    UnsupportedOnWasm,
}

pub type Result<T> = std::result::Result<T, MattingError>;
