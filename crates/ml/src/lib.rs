//! Generative provider interface for PhotoCraft (layer 4, slot `ml`).
//!
//! The engine hands a provider an RGBA image plus a mask (white = fill this area) and gets an
//! RGBA image of the same size back. Nothing here knows about documents, layers or the UI, and
//! nothing here talks to the network directly: every provider goes through a [`Transport`], so
//! tests use [`FakeTransport`] and the wasm build compiles without an HTTP client.
//!
//! First provider: [`IoPaintProvider`], a local [IOPaint](https://github.com/Sanster/IOPaint)
//! helper running the LaMa model. The provider is off unless the app injects one.

#![forbid(unsafe_code)]

mod iopaint;
mod mask;
mod png_codec;
mod transport;

pub use iopaint::{DEFAULT_ENDPOINT, IoPaintProvider};
pub use mask::{Rect, expand_coverage, expand_mask};
pub use png_codec::{decode_png_rgba, encode_png_rgba};
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub use transport::UreqTransport;
pub use transport::{FakeTransport, HttpResponse, Transport};

/// 8-bit RGBA pixels, row-major, no padding (`data.len() == width * height * 4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// A solid-colour image; `None` if the size is zero or would not fit in memory.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Option<Self> {
        let n = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if n == 0 {
            return None;
        }
        let mut data = Vec::with_capacity(n);
        for _ in 0..n / 4 {
            data.extend_from_slice(&rgba);
        }
        Some(Self { width, height, data })
    }

    /// `true` when the buffer length matches the dimensions.
    pub fn is_consistent(&self) -> bool {
        (self.width as usize).checked_mul(self.height as usize).and_then(|n| n.checked_mul(4)).is_some_and(|n| n == self.data.len() && n > 0)
    }
}

/// 8-bit single-channel coverage, row-major (`data.len() == width * height`).
/// 255 = generate here, 0 = keep the original pixel. Providers treat values above 127 as "fill".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Mask {
    /// `true` when the buffer length matches the dimensions.
    pub fn is_consistent(&self) -> bool {
        (self.width as usize).checked_mul(self.height as usize).is_some_and(|n| n == self.data.len() && n > 0)
    }
}

/// Options for one generation. Everything is optional so a provider that ignores a field
/// (LaMa has no prompt) still gets a complete request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenerateOptions {
    /// Free-text guidance. Ignored by providers that cannot use it (reported by
    /// [`GenerativeProvider::accepts_prompt`]).
    pub prompt: Option<String>,
    /// Seed for providers with randomness; `None` = provider's choice.
    pub seed: Option<u64>,
}

/// What a provider says about itself, for the UI and for the privacy notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// Short name shown in Preferences, e.g. "IOPaint (LaMa)".
    pub name: String,
    /// Where pixels go, in words a user understands, e.g. "a helper program on this computer at
    /// http://127.0.0.1:8080". Shown before the first request (upstream rule 5).
    pub destination: String,
}

/// Something that fills a masked area of an image. `Send + Sync` because the engine shares one
/// provider (behind an `Arc`) with the worker thread that runs a generation job.
pub trait GenerativeProvider: Send + Sync {
    /// Name and data destination, for Preferences and the privacy notice.
    fn info(&self) -> ProviderInfo;

    /// Whether [`GenerateOptions::prompt`] has any effect. The UI hides the prompt field when not.
    fn accepts_prompt(&self) -> bool;

    /// Cheap reachability check (no pixels sent). `Ok` means a later `generate` has a chance.
    fn ping(&self) -> Result<(), Error>;

    /// Fill the masked area. `image` and `mask` must have the same size. `cancelled` is polled
    /// between steps; a `true` turns into [`Error::Cancelled`]. The result has the same size as
    /// `image`; pixels where the mask is 0 are expected to be unchanged, but callers should not
    /// rely on it and re-apply the mask themselves.
    fn generate(&self, image: &RgbaImage, mask: &Mask, options: &GenerateOptions, cancelled: &dyn Fn() -> bool) -> Result<RgbaImage, Error>;
}

/// Everything that can go wrong between the engine and a provider.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("image and mask sizes differ ({image_w}x{image_h} vs {mask_w}x{mask_h})")]
    SizeMismatch { image_w: u32, image_h: u32, mask_w: u32, mask_h: u32 },
    #[error("invalid image or mask buffer")]
    InvalidBuffer,
    #[error("cancelled")]
    Cancelled,
    #[error("cannot reach the generative helper at {url}: {detail}")]
    Unreachable { url: String, detail: String },
    #[error("the generative helper answered HTTP {status}: {detail}")]
    HttpStatus { status: u16, detail: String },
    #[error("the generative helper sent an unreadable reply: {0}")]
    BadReply(String),
    #[error("the generative helper returned {got_w}x{got_h} pixels, expected {want_w}x{want_h}")]
    WrongSize { want_w: u32, want_h: u32, got_w: u32, got_h: u32 },
    #[error("PNG error: {0}")]
    Png(String),
}
