//! Scanner import for the desktop app.
//!
//! macOS talks to Image Capture (USB and network scanners) from [`macos`]. Every other platform
//! reports that scanning is unavailable. The public functions never panic when no scanner is
//! attached: an empty list is a successful answer.
//!
//! Image Capture delivers device callbacks on the main thread, so [`list_devices`] and [`acquire`]
//! must be called on that thread. They pump the main run loop until the scan finishes or the
//! timeout elapses. Callers should paint a "scanning" state first; the window does not redraw
//! while a call is in progress.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::time::Duration;

mod decode;

#[cfg(target_os = "macos")]
mod macos;

/// One scanner Image Capture can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scanner {
    /// Persistent device id (`UUIDString`).
    pub id: String,
    /// The name Image Capture reports.
    pub name: String,
}

/// Colour mode of the scan. The image is always returned as 8-bit RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Color,
    Gray,
    BlackAndWhite,
}

/// What to scan. `resolution` is snapped to 75, 150, 300 or 600, then to a value the scanner accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanRequest {
    pub device_id: String,
    pub resolution: u32,
    pub mode: Mode,
    /// How long the whole acquire may take, including finding the device.
    pub timeout: Duration,
}

/// Decoded scan, top to bottom, 8-bit RGBA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Dots per inch the scanner was asked to use (the file's own metadata is not trusted).
    pub dpi: u32,
    /// Pages the feeder delivered. Only the first page is in [`Self::rgba`].
    pub pages: u32,
}

/// Why a scan could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Image Capture callbacks run on the main thread.
    NotMainThread,
    /// This build has no scanner driver.
    Unsupported(String),
    /// The device, the file, or Image Capture failed.
    Platform(String),
    /// The wait elapsed before a device or a page arrived.
    Timeout,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotMainThread => write!(f, "scanning must run on the main thread"),
            Error::Unsupported(why) => write!(f, "{why}"),
            Error::Platform(why) => write!(f, "{why}"),
            Error::Timeout => write!(f, "the scanner took too long"),
        }
    }
}

impl std::error::Error for Error {}

/// `true` on macOS, where Image Capture is linked.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}

/// Snap `dpi` to the nearest of 75, 150, 300 and 600.
pub fn normalize_resolution(dpi: u32) -> u32 {
    const CHOICES: [u32; 4] = [75, 150, 300, 600];
    CHOICES.iter().copied().min_by_key(|c| c.abs_diff(dpi)).unwrap_or(300)
}

/// Scanners currently attached. An empty vector means none were found before `timeout`.
/// `timeout` of zero returns an empty list without opening Image Capture.
pub fn list_devices(timeout: Duration) -> Result<Vec<Scanner>, Error> {
    if !available() {
        return Err(Error::Unsupported("Scanning needs macOS Image Capture.".into()));
    }
    if timeout.is_zero() {
        return Ok(Vec::new());
    }
    #[cfg(target_os = "macos")]
    {
        macos::list_devices(timeout)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::Unsupported("Scanning needs macOS Image Capture.".into()))
    }
}

/// Scan one page from `request.device_id` into memory.
pub fn acquire(request: ScanRequest) -> Result<ScanImage, Error> {
    if request.device_id.trim().is_empty() {
        return Err(Error::Platform("choose a scanner".into()));
    }
    if !available() {
        return Err(Error::Unsupported("Scanning needs macOS Image Capture.".into()));
    }
    let request = ScanRequest { resolution: normalize_resolution(request.resolution), ..request };
    #[cfg(target_os = "macos")]
    {
        macos::acquire(request)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err(Error::Unsupported("Scanning needs macOS Image Capture.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_snaps_to_the_cs4_choices() {
        assert_eq!(normalize_resolution(0), 75);
        assert_eq!(normalize_resolution(100), 75);
        assert_eq!(normalize_resolution(120), 150);
        assert_eq!(normalize_resolution(300), 300);
        assert_eq!(normalize_resolution(450), 300);
        assert_eq!(normalize_resolution(10_000), 600);
    }

    #[test]
    fn an_empty_device_is_rejected_before_any_driver_call() {
        let err = acquire(ScanRequest { device_id: "  ".into(), resolution: 300, mode: Mode::Color, timeout: Duration::from_secs(1) }).unwrap_err();
        assert_eq!(err, Error::Platform("choose a scanner".into()));
    }

    #[test]
    fn platforms_without_image_capture_refuse() {
        if available() {
            return;
        }
        assert!(matches!(list_devices(Duration::from_secs(1)), Err(Error::Unsupported(_))));
        let err = acquire(ScanRequest { device_id: "x".into(), resolution: 300, mode: Mode::Gray, timeout: Duration::from_secs(1) }).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }

    #[test]
    fn a_zero_wait_does_not_open_the_browser() {
        if !available() {
            return;
        }
        assert_eq!(list_devices(Duration::ZERO).unwrap(), Vec::new());
    }

    #[test]
    fn a_background_thread_cannot_scan() {
        if !available() {
            return;
        }
        let err = list_devices(Duration::from_millis(50)).unwrap_err();
        assert_eq!(err, Error::NotMainThread);
    }
}
