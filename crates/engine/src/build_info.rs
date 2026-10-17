//! Version and build provenance shared by every frontend (About dialog, `--version`).
//!
//! The version is the workspace version (`[workspace.package] version` in the root
//! `Cargo.toml`, managed by `cargo xtask version`). The commit and build date are optional and
//! come from environment variables at compile time, set by CI and the packaging scripts:
//!
//! - `PHOTOCRAFT_BUILD_SHA`: the git commit (any length; shown shortened to 9 characters)
//! - `PHOTOCRAFT_BUILD_DATE`: the build date, conventionally `YYYY-MM-DD`
//!
//! Without them (a plain `cargo build`) the build reports itself as a dev build. Nothing here
//! shells out to git, so builds work from a source tarball and for wasm. The desktop and CLI
//! binaries' build scripts do read the commit of a git checkout and hand it over with
//! [`set_checkout`] at startup, so a dev build from git says which commit it is
//! (`0.1.0 (dev build, 3f2a9c1d0)`); keeping it out of this crate means a commit doesn't rebuild
//! the engine.

/// The workspace version, such as `0.1.0` or `0.2.0-rc.1`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The full git commit this build was made from, when the build environment recorded it.
pub fn git_sha() -> Option<&'static str> {
    option_env!("PHOTOCRAFT_BUILD_SHA").map(str::trim).filter(|s| !s.is_empty())
}

/// The build date (`YYYY-MM-DD`), when the build environment recorded it.
pub fn build_date() -> Option<&'static str> {
    option_env!("PHOTOCRAFT_BUILD_DATE").map(str::trim).filter(|s| !s.is_empty())
}

/// The commit shortened for display.
pub fn short_sha() -> Option<&'static str> {
    git_sha().map(shorten)
}

fn shorten(sha: &str) -> &str {
    sha.get(..9).unwrap_or(sha)
}

static CHECKOUT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Records the git commit of the checkout a dev build was compiled from (the binaries' build
/// scripts read it; `None` or empty from a source tarball). Only the first call counts.
pub fn set_checkout(sha: Option<&str>) {
    if let Some(sha) = sha.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = CHECKOUT.set(sha.to_string());
    }
}

/// `0.1.0 (3f2a9c1d0, 2026-10-01)`; a dev build `0.1.0 (dev build, 3f2a9c1d0)` from a git
/// checkout, or `0.1.0 (dev build)` without provenance.
pub fn long_version() -> String {
    match short_sha() {
        Some(s) => format_long(VERSION, Some(s), build_date()),
        None => format_dev(VERSION, CHECKOUT.get().map(|s| shorten(s)), build_date()),
    }
}

fn format_long(version: &str, sha: Option<&str>, date: Option<&str>) -> String {
    match (sha, date) {
        (Some(s), Some(d)) => format!("{version} ({s}, {d})"),
        (Some(s), None) => format!("{version} ({s})"),
        (None, Some(d)) => format!("{version} ({d})"),
        (None, None) => format!("{version} (dev build)"),
    }
}

/// A build without `PHOTOCRAFT_BUILD_SHA`: the checkout's commit, when known, marked as a dev build.
fn format_dev(version: &str, checkout: Option<&str>, date: Option<&str>) -> String {
    match checkout {
        Some(c) => format!("{version} (dev build, {c})"),
        None => format_long(version, None, date),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_version_formats() {
        assert_eq!(format_long("1.2.3", Some("abc"), Some("2026-10-01")), "1.2.3 (abc, 2026-10-01)");
        assert_eq!(format_long("1.2.3", Some("abc"), None), "1.2.3 (abc)");
        assert_eq!(format_long("1.2.3", None, Some("2026-10-01")), "1.2.3 (2026-10-01)");
        assert_eq!(format_long("1.2.3-rc.1", None, None), "1.2.3-rc.1 (dev build)");
        // #5247: a dev build from a git checkout names its commit.
        assert_eq!(format_dev("0.8.2", Some(shorten("6f8505421abcdef")), None), "0.8.2 (dev build, 6f8505421)");
        assert_eq!(format_dev("0.8.2", None, None), "0.8.2 (dev build)");
        assert_eq!(format_dev("0.8.2", None, Some("2026-10-01")), "0.8.2 (2026-10-01)");
        assert_eq!(shorten("abc"), "abc");
    }

    #[test]
    fn version_is_the_workspace_version() {
        assert!(!VERSION.is_empty());
        assert!(long_version().starts_with(VERSION));
        if let Some(s) = short_sha() {
            assert!(s.len() <= 9);
        }
    }
}
