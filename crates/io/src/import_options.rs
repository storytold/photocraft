//! Explicit authority for imports that may otherwise discover installed camera profiles.

/// Which camera calibration sources a caller authorizes while importing RAW pixels.
/// This is process authority supplied by the caller, never a value read from a document.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CameraProfilePolicy {
    /// Use only the embedded nine-profile bundle and the RAW's own calibration.
    /// No profile environment variables, user directories or external profile cache are read.
    #[default]
    BundledOnly,
    /// A trusted local caller authorizes explicitly configured and installed profiles.
    /// Invalid explicit overrides fail; unusable automatically discovered candidates are skipped.
    Installed,
}
