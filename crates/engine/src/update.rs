//! Application update checking from GitHub releases feed.
//!
//! Checks the public GitHub Releases feed (`storytold/photocraft/releases/latest`)
//! for new versions, compares using semver against the current build, ignores drafts
//! and pre-releases, and respects rate limits.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const RELEASES_API_URL: &str = "https://api.github.com/repos/storytold/photocraft/releases/latest";
pub const USER_AGENT: &str = "Photocraft-dev";
/// Minimum interval between automatic periodic checks (24 hours).
pub const MIN_CHECK_INTERVAL_SECS: u64 = 86400;

/// Information about a published release from GitHub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub published_at: String,
    pub prerelease: bool,
    pub draft: bool,
}

/// Parses a GitHub Release JSON object into a [`ReleaseInfo`].
///
/// Rejects drafts and pre-releases.
pub fn parse_release(v: &Value) -> Result<ReleaseInfo, String> {
    let draft = v.get("draft").and_then(Value::as_bool).unwrap_or(false);
    if draft {
        return Err("release is a draft".into());
    }

    let prerelease = v.get("prerelease").and_then(Value::as_bool).unwrap_or(false);
    if prerelease {
        return Err("release is a pre-release".into());
    }

    let tag_name = v.get("tag_name").and_then(Value::as_str).ok_or("missing `tag_name`")?.to_string();
    let version = clean_version_str(&tag_name);

    // Verify candidate version is valid semver and not a pre-release like 1.0.0-rc.1
    let parsed_semver = clean_semver(&version).ok_or_else(|| format!("invalid semver: {tag_name}"))?;
    if !parsed_semver.pre.is_empty() {
        return Err("candidate version is a pre-release".into());
    }

    let name = v.get("name").and_then(Value::as_str).unwrap_or(&tag_name).to_string();
    let body = v.get("body").and_then(Value::as_str).unwrap_or("").to_string();
    let html_url = v.get("html_url").and_then(Value::as_str).ok_or("missing `html_url`")?.to_string();
    let published_at = v.get("published_at").and_then(Value::as_str).unwrap_or("").to_string();

    Ok(ReleaseInfo { tag_name, version, name, body, html_url, published_at, prerelease, draft })
}

/// Strips leading 'v' / 'V' and trims whitespace from a version tag string.
pub fn clean_version_str(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(stripped) = trimmed.strip_prefix('v').or_else(|| trimmed.strip_prefix('V')) { stripped.trim().to_string() } else { trimmed.to_string() }
}

/// Parses a string into a [`semver::Version`], extracting the version prefix if extra build
/// information is present (e.g. `0.6.0 (dev build)` -> `0.6.0`).
pub fn clean_semver(s: &str) -> Option<semver::Version> {
    let cleaned = clean_version_str(s);
    if let Ok(v) = semver::Version::parse(&cleaned) {
        return Some(v);
    }
    // Try taking the first whitespace or plus separated token
    let first_token = cleaned.split([' ', '+']).next().unwrap_or(&cleaned);
    semver::Version::parse(first_token).ok()
}

/// Checks if `candidate` represents a strictly newer semver version than `current`.
///
/// Pre-releases (such as `-rc.1`) in candidate versions are ignored.
/// Dev builds in `current` fall back to the workspace package version.
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    let Some(candidate_ver) = clean_semver(candidate) else {
        return false;
    };
    if !candidate_ver.pre.is_empty() {
        return false;
    }

    let current_ver = clean_semver(current).or_else(|| clean_semver(crate::build_info::VERSION));
    match current_ver {
        Some(curr) => candidate_ver > curr,
        None => false,
    }
}

/// Checks whether an automatic periodic check should run based on the last check time.
pub fn should_check_periodically(last_check_epoch_secs: Option<u64>, now_epoch_secs: u64) -> bool {
    match last_check_epoch_secs {
        None => true,
        Some(last) => now_epoch_secs >= last.saturating_add(MIN_CHECK_INTERVAL_SECS),
    }
}

/// Fetches the latest release from the specified URL.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_latest_release(url: &str, timeout_secs: u64) -> Result<ReleaseInfo, String> {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(timeout_secs)).user_agent(USER_AGENT).build();

    let response = agent.get(url).set("Accept", "application/vnd.github.v3+json").call().map_err(|e| format!("Failed to check for updates: {e}"))?;

    let json_val: Value = serde_json::from_reader(response.into_reader()).map_err(|e| format!("Invalid release response: {e}"))?;
    parse_release(&json_val)
}

/// WebAssembly stub: network fetching is unavailable.
#[cfg(target_arch = "wasm32")]
pub fn fetch_latest_release(_url: &str, _timeout_secs: u64) -> Result<ReleaseInfo, String> {
    Err("Update checking is not supported on WebAssembly".into())
}

/// Performs an update check against GitHub Releases feed.
///
/// Returns `Ok(Some(release))` if a newer version is available,
/// `Ok(None)` if up to date, or `Err(message)` if the check failed.
pub fn check_for_update(current_version: &str, timeout_secs: u64) -> Result<Option<ReleaseInfo>, String> {
    let release = fetch_latest_release(RELEASES_API_URL, timeout_secs)?;
    if is_newer_version(current_version, &release.version) { Ok(Some(release)) } else { Ok(None) }
}

fn always(_: &crate::Session) -> std::result::Result<(), String> {
    Ok(())
}

fn check_for_updates_cmd(_s: &mut crate::Session, p: &Value) -> crate::Result<Value> {
    let timeout = p.get("timeout").and_then(Value::as_u64).unwrap_or(10);
    let current_version = p.get("currentVersion").and_then(Value::as_str).unwrap_or(crate::build_info::VERSION);
    let outcome = check_for_update(current_version, timeout).map_err(crate::EngineError::Other)?;
    match outcome {
        Some(rel) => Ok(serde_json::json!({
            "updateAvailable": true,
            "release": rel,
        })),
        None => Ok(serde_json::json!({
            "updateAvailable": false,
            "version": current_version,
        })),
    }
}

pub fn specs() -> Vec<crate::commands::CommandSpec> {
    vec![crate::commands::CommandSpec {
        id: "help.checkForUpdates",
        label: "Check for Updates…",
        menu: &["Help"],
        shortcut: None,
        params: r#"{"timeout":10,"currentVersion":"0.6.0"}"#,
        enabled: always,
        run: check_for_updates_cmd,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clean_version_strips_v_prefix() {
        assert_eq!(clean_version_str("v0.6.0"), "0.6.0");
        assert_eq!(clean_version_str("V1.2.3"), "1.2.3");
        assert_eq!(clean_version_str("  v2.0.0  "), "2.0.0");
        assert_eq!(clean_version_str("0.6.0"), "0.6.0");
    }

    #[test]
    fn clean_semver_handles_dev_build_string() {
        assert_eq!(clean_semver("0.6.0 (dev build)"), Some(semver::Version::new(0, 6, 0)));
        assert_eq!(clean_semver("v0.6.0 (3f2a9c1d0, 2026-10-01)"), Some(semver::Version::new(0, 6, 0)));
        assert_eq!(clean_semver("1.0.0"), Some(semver::Version::new(1, 0, 0)));
        assert_eq!(clean_semver("not_a_version"), None);
    }

    #[test]
    fn version_comparison_logic() {
        assert!(is_newer_version("0.6.0", "0.6.1"));
        assert!(is_newer_version("0.6.0", "0.7.0"));
        assert!(is_newer_version("0.6.0", "1.0.0"));
        assert!(is_newer_version("v0.6.0", "v0.6.1"));
        assert!(is_newer_version("0.6.0 (dev build)", "0.6.1"));

        // Same version is not newer
        assert!(!is_newer_version("0.6.0", "0.6.0"));
        assert!(!is_newer_version("v0.6.0", "0.6.0"));

        // Older version is not newer
        assert!(!is_newer_version("0.6.0", "0.5.9"));
        assert!(!is_newer_version("1.0.0", "0.9.9"));

        // Pre-release candidates are ignored
        assert!(!is_newer_version("0.6.0", "0.6.1-rc.1"));
        assert!(!is_newer_version("0.6.0", "0.7.0-beta.2"));
    }

    #[test]
    fn parse_valid_github_release_fixture() {
        let fixture = json!({
            "tag_name": "v0.7.0",
            "name": "PhotoCraft 0.7.0",
            "body": "## What's Changed\n* Added Camera Raw enhancements\n* Improved performance",
            "html_url": "https://github.com/storytold/photocraft/releases/tag/v0.7.0",
            "published_at": "2026-10-15T12:00:00Z",
            "draft": false,
            "prerelease": false
        });

        let rel = parse_release(&fixture).expect("valid release");
        assert_eq!(rel.tag_name, "v0.7.0");
        assert_eq!(rel.version, "0.7.0");
        assert_eq!(rel.name, "PhotoCraft 0.7.0");
        assert!(rel.body.contains("Camera Raw"));
        assert_eq!(rel.html_url, "https://github.com/storytold/photocraft/releases/tag/v0.7.0");
    }

    #[test]
    fn parse_rejects_drafts_and_prereleases() {
        let draft = json!({
            "tag_name": "v0.7.0",
            "draft": true,
            "prerelease": false,
            "html_url": "https://example.com"
        });
        assert!(parse_release(&draft).is_err());

        let prerelease = json!({
            "tag_name": "v0.7.0-rc.1",
            "draft": false,
            "prerelease": true,
            "html_url": "https://example.com"
        });
        assert!(parse_release(&prerelease).is_err());

        let prerelease_tag = json!({
            "tag_name": "v0.7.0-rc.1",
            "draft": false,
            "prerelease": false,
            "html_url": "https://example.com"
        });
        assert!(parse_release(&prerelease_tag).is_err());
    }

    #[test]
    fn periodic_check_rate_limiting() {
        assert!(should_check_periodically(None, 1000));
        assert!(should_check_periodically(Some(1000), 1000 + MIN_CHECK_INTERVAL_SECS));
        assert!(should_check_periodically(Some(1000), 1000 + MIN_CHECK_INTERVAL_SECS + 10));
        assert!(!should_check_periodically(Some(1000), 1000 + 3600)); // only 1 hour later
        assert!(!should_check_periodically(Some(1000), 1000 + MIN_CHECK_INTERVAL_SECS - 1));
    }

    #[test]
    fn command_does_not_panic_on_arbitrary_params() {
        let mut s = crate::Session::new();
        // Malformed params must not panic
        let _ = check_for_updates_cmd(&mut s, &json!({ "timeout": -1, "currentVersion": [1, 2, 3] }));
        let _ = check_for_updates_cmd(&mut s, &json!("nonsense"));
        let _ = check_for_updates_cmd(&mut s, &json!(null));
    }
}
