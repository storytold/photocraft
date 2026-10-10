//! Generative image providers (Generative Fill and friends, issue #41).
//!
//! [`ImageProvider`] is the one interface the engine talks to; each backend (Gemini today, others
//! later) is a separate implementation and none is privileged. Providers never open a socket
//! themselves: every request goes through an [`HttpTransport`] the app injects, so this crate (and
//! the engine above it) stays network-free, builds for wasm, and tests run against a fake.
//!
//! Images cross this interface as encoded bytes (PNG in, PNG or JPEG out) in sRGB. Masks are PNGs
//! of the same size as their image: white marks the pixels to change, black the pixels to keep.
//!
//! Credentials: an [`ApiKey`] never prints (its `Debug` is redacted) and no error message carries
//! a key or a response body; a service's own error text is passed through only after any copy of
//! the key is scrubbed from it.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod gemini;

/// Largest response body a provider accepts (a 4K PNG is well under this).
pub const MAX_RESPONSE: usize = 48 * 1024 * 1024;
/// Largest prompt, in bytes.
pub const MAX_PROMPT: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenError {
    #[error("{0} can't do that")]
    Unsupported(&'static str),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("the API key was rejected; check it in Generative AI Settings")]
    Auth,
    #[error("the image service's quota or rate limit was reached; try again later")]
    Quota,
    #[error("the image service declined this request: {0}")]
    Refused(String),
    #[error("the image service failed (HTTP {status}): {message}")]
    Http { status: u16, message: String },
    #[error("could not reach the image service: {0}")]
    Network(String),
    #[error("the image service sent a response PhotoCraft can't read: {0}")]
    BadResponse(&'static str),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, GenError>;

/// A secret in process memory. It never prints; read it only to build a request.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// `None` for a blank key. Surrounding whitespace (a pasted newline) is dropped.
    pub fn new(key: &str) -> Option<ApiKey> {
        let k = key.trim();
        (!k.is_empty()).then(|| ApiKey(k.to_owned()))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    /// `text` with every copy of the key replaced, for error messages that echo a request.
    pub fn scrub(&self, text: &str) -> String {
        text.replace(&self.0, "[key]")
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(…)")
    }
}

/// One HTTPS POST. Header values may hold a credential, so a transport must not log them.
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

/// Any HTTP status comes back as a response (the provider reads the service's error body);
/// only a failure to talk to the server at all is an `Err`.
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The network seam. The desktop app implements it over HTTPS; tests use a fake; the web build
/// has none, so generative commands are simply unavailable there.
pub trait HttpTransport: Send + Sync {
    /// Blocking: called from an engine background job, never the UI thread.
    fn post(&self, request: HttpRequest) -> Result<HttpResponse>;
}

/// What a provider can do; the UI and the engine's `enabled` checks read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub generate: bool,
    pub edit: bool,
    pub variations: bool,
}

/// Who a request goes to, for the privacy notice and error messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// Stable id stored in preferences (`gemini`).
    pub id: &'static str,
    /// Display name (`Google Gemini`).
    pub name: &'static str,
    /// Model the requests use.
    pub model: String,
    /// Host the image, mask and prompt are sent to.
    pub host: &'static str,
}

/// Output size class. Providers map it to their nearest supported size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageSize {
    /// About 1024 px on the long side.
    #[default]
    Small,
    /// About 2048 px.
    Medium,
    /// About 4096 px.
    Large,
}

impl ImageSize {
    /// The class that covers `long_side` pixels without upscaling much.
    pub fn for_long_side(long_side: u32) -> ImageSize {
        match long_side {
            0..=1200 => ImageSize::Small,
            1201..=2400 => ImageSize::Medium,
            _ => ImageSize::Large,
        }
    }
}

/// Text to image.
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub prompt: String,
    /// Wanted output width and height (only their ratio matters).
    pub aspect: (u32, u32),
    pub size: ImageSize,
}

/// Change the masked part of `image` (Generative Fill). An empty prompt asks the provider to
/// remove what is under the mask and continue the surroundings.
#[derive(Debug, Clone)]
pub struct EditRequest {
    pub image: Vec<u8>,
    pub mask: Vec<u8>,
    pub prompt: String,
    pub size: ImageSize,
}

/// Another take on a whole image, optionally steered by a prompt.
#[derive(Debug, Clone)]
pub struct VariationRequest {
    pub image: Vec<u8>,
    pub prompt: String,
    pub size: ImageSize,
}

/// An encoded image a provider returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    /// `image/png` or `image/jpeg`.
    pub mime: String,
}

/// A generative image backend. Methods block (they run in a background job) and default to
/// [`GenError::Unsupported`], so a backend implements only what its [`Capabilities`] report.
pub trait ImageProvider: Send + Sync {
    fn info(&self) -> ProviderInfo;
    fn capabilities(&self) -> Capabilities;
    /// The width:height an edit of a `w`×`h` image comes back at. Providers with a fixed set of
    /// output shapes return the nearest one, so the caller can grow its context area to match
    /// instead of stretching the result.
    fn edit_aspect(&self, w: u32, h: u32) -> (u32, u32) {
        (w, h)
    }
    fn generate(&self, _request: &GenerateRequest) -> Result<GeneratedImage> {
        Err(GenError::Unsupported(self.info().name))
    }
    fn edit(&self, _request: &EditRequest) -> Result<GeneratedImage> {
        Err(GenError::Unsupported(self.info().name))
    }
    fn variations(&self, _request: &VariationRequest) -> Result<GeneratedImage> {
        Err(GenError::Unsupported(self.info().name))
    }
}

/// Checks shared by every provider, run before anything is sent.
pub fn check_prompt(prompt: &str, required: bool) -> Result<()> {
    if prompt.len() > MAX_PROMPT {
        return Err(GenError::Invalid(format!("the prompt is longer than {MAX_PROMPT} bytes")));
    }
    if required && prompt.trim().is_empty() {
        return Err(GenError::Invalid("the prompt is empty".into()));
    }
    Ok(())
}

/// The entry of `ratios` (width, height) closest to `w`×`h`, compared in log space so 2:1 and
/// 1:2 are equally far from 1:1.
pub fn nearest_ratio(w: u32, h: u32, ratios: &[(u32, u32)]) -> Option<(u32, u32)> {
    let target = (f64::from(w.max(1)) / f64::from(h.max(1))).ln();
    ratios.iter().copied().filter(|&(a, b)| a > 0 && b > 0).min_by(|&(a, b), &(c, d)| {
        let da = ((f64::from(a) / f64::from(b)).ln() - target).abs();
        let dc = ((f64::from(c) / f64::from(d)).ln() - target).abs();
        da.total_cmp(&dc)
    })
}

/// Width and height of a PNG from its header, without decoding it.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || !bytes.starts_with(SIG) || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_keys_never_print_and_are_scrubbed() {
        let k = ApiKey::new("  sk-secret-123\n").unwrap();
        assert_eq!(k.expose(), "sk-secret-123");
        assert!(!format!("{k:?}").contains("secret"));
        assert_eq!(k.scrub("bad key sk-secret-123!"), "bad key [key]!");
        assert!(ApiKey::new(" \n").is_none());
    }

    #[test]
    fn nearest_ratio_is_symmetric_in_log_space() {
        let r = [(1, 1), (16, 9), (9, 16), (4, 3), (3, 4)];
        assert_eq!(nearest_ratio(1000, 1000, &r), Some((1, 1)));
        assert_eq!(nearest_ratio(1920, 1080, &r), Some((16, 9)));
        assert_eq!(nearest_ratio(1080, 1920, &r), Some((9, 16)));
        assert_eq!(nearest_ratio(400, 310, &r), Some((4, 3)));
        assert_eq!(nearest_ratio(0, 0, &r), Some((1, 1)));
        assert_eq!(nearest_ratio(5, 5, &[]), None);
    }

    #[test]
    fn prompt_and_size_checks() {
        assert!(check_prompt("", false).is_ok());
        assert!(check_prompt(" ", true).is_err());
        assert!(check_prompt(&"x".repeat(MAX_PROMPT + 1), false).is_err());
        assert_eq!(ImageSize::for_long_side(900), ImageSize::Small);
        assert_eq!(ImageSize::for_long_side(2000), ImageSize::Medium);
        assert_eq!(ImageSize::for_long_side(5000), ImageSize::Large);
    }

    #[test]
    fn png_size_reads_the_header_and_rejects_junk() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((640, 480)));
        assert_eq!(png_size(b"GIF89a"), None);
        assert_eq!(png_size(&png[..20]), None);
    }
}
