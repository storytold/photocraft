//! Whether PhotoCraft runs elevated above the desktop on Windows ("Run as administrator").
//!
//! Windows' User Interface Privilege Isolation (UIPI) drops the drag-and-drop and window messages
//! that a process at a lower integrity level (File Explorer, a browser) sends to one at a higher
//! level. An elevated PhotoCraft therefore ignores every file dropped from Explorer, with no
//! error on either side. Knowing it lets the app say so (`notices::elevated_file_drop_guidance`)
//! instead of looking broken.

/// The process token is the full half of a split UAC token: PhotoCraft was elevated while the
/// desktop, and Explorer with it, runs limited. `false` with UAC off, where every process of an
/// administrator gets the full token and drops work (`TokenElevationTypeDefault`; asking
/// `TokenIsElevated` instead says yes there), off Windows, and when Windows can't tell.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use winsafe::{HACCESSTOKEN, TokenInfo, co};
    // The current-process pseudo-handle needs no opening or closing.
    match HACCESSTOKEN::GetCurrentProcessToken().GetTokenInformation(co::TOKEN_INFORMATION_CLASS::ElevationType) {
        Ok(TokenInfo::ElevationType(kind)) => *kind == co::TOKEN_ELEVATION_TYPE::Full,
        Ok(_) => false,
        Err(e) => {
            log::warn!("could not read the process elevation: {e}");
            false
        }
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(test)]
mod tests {
    /// Reading the token never fails the launch, elevated or not (the answer depends on how the
    /// test runner was started, so only its absence of side effects is checked).
    #[test]
    fn reading_the_elevation_is_harmless() {
        let first = super::is_elevated();
        assert_eq!(super::is_elevated(), first);
        #[cfg(not(windows))]
        assert!(!first);
    }
}
