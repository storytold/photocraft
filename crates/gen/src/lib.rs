//! Replaceable image editing providers. No API-key provider or automatic fallback.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
pub mod codex;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("AI edit cancelled")]
    Cancelled,
    #[error("AI edit timed out")]
    Timeout,
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    RuntimeUnavailable(String),
    #[error("image service I/O: {0}")]
    Io(#[from] std::io::Error),
}

pub struct EditRequest<'a> {
    /// Flattened, colour-managed PNG of the current canvas.
    pub image: &'a Path,
    pub prompt: &'a str,
    pub workspace: &'a Path,
}

pub struct EditResult {
    pub saved_path: PathBuf,
}

/// Providers own transport/authentication; the engine owns import, colour conversion and undo.
pub trait ImageProvider: Send + Sync {
    fn edit(&self, request: &EditRequest<'_>, cancelled: &dyn Fn() -> bool) -> Result<EditResult, Error>;
}
