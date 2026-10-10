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
#[cfg(not(target_arch = "wasm32"))]
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

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
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

impl ReleaseInfo {
    /// Prefer the native installer; package-managed installations use their package manager.
    pub fn download_url(&self, os: &str, arch: &str, appimage: bool) -> Option<&str> {
        let suffix = match (os, arch) {
            ("macos", "aarch64" | "x86_64") => "macos-universal.dmg",
            ("windows", "aarch64") => "windows-arm64.msi",
            ("windows", "x86_64") => "windows-x64.msi",
            ("windows", "x86") => "windows-x86.msi",
            ("linux", "aarch64") if appimage => "linux-aarch64.AppImage",
            ("linux", "x86_64") if appimage => "linux-x86_64.AppImage",
            _ => return None,
        };
        let name = format!("photocraft-{}-{suffix}", self.version);
        self.assets.iter().find(|a| a.name == name && trusted_download_url(&a.browser_download_url, &self.tag_name)).map(|a| a.browser_download_url.as_str())
    }
}

pub fn trusted_release_url(url: &str) -> bool {
    url.strip_prefix("https://github.com/storytold/photocraft/releases/tag/").is_some_and(safe_path_component)
}

fn safe_path_component(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

pub fn trusted_download_url(url: &str, tag: &str) -> bool {
    safe_path_component(tag) && url.strip_prefix(&format!("https://github.com/storytold/photocraft/releases/download/{tag}/")).is_some_and(safe_path_component)
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
    let parsed_semver = semver::Version::parse(&version).map_err(|_| format!("invalid semver: {tag_name}"))?;
    if !safe_path_component(&tag_name) {
        return Err("invalid release tag".into());
    }
    if !parsed_semver.pre.is_empty() {
        return Err("candidate version is a pre-release".into());
    }

    let name = v.get("name").and_then(Value::as_str).unwrap_or(&tag_name).to_string();
    let body = v.get("body").and_then(Value::as_str).unwrap_or("").to_string();
    let html_url = v.get("html_url").and_then(Value::as_str).ok_or("missing `html_url`")?.to_string();
    if !trusted_release_url(&html_url) || html_url != format!("https://github.com/storytold/photocraft/releases/tag/{tag_name}") {
        return Err("untrusted release URL".into());
    }
    let published_at = v.get("published_at").and_then(Value::as_str).unwrap_or("").to_string();
    let assets = v
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            let url = asset.get("browser_download_url")?.as_str()?;
            (safe_path_component(name) && trusted_download_url(url, &tag_name) && url.ends_with(&format!("/{name}")))
                .then(|| ReleaseAsset { name: name.into(), browser_download_url: url.into() })
        })
        .collect();

    Ok(ReleaseInfo { tag_name, version, name, body, html_url, published_at, prerelease, draft, assets })
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
    let Ok(candidate_ver) = semver::Version::parse(&clean_version_str(candidate)) else {
        return false;
    };
    if !candidate_ver.pre.is_empty() {
        return false;
    }

    let current_ver = clean_semver(current).or_else(|| clean_semver(crate::build_info::VERSION));
    match current_ver {
        Some(curr) => candidate_ver.cmp_precedence(&curr).is_gt(),
        None => false,
    }
}

/// Checks whether an automatic periodic check should run based on the last check time.
pub fn should_check_periodically(last_check_epoch_secs: Option<u64>, now_epoch_secs: u64) -> bool {
    match last_check_epoch_secs {
        None => true,
        Some(last) => now_epoch_secs < last || now_epoch_secs >= last.saturating_add(MIN_CHECK_INTERVAL_SECS),
    }
}

/// Fetches the latest release from the specified URL.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_latest_release(url: &str, timeout_secs: u64) -> Result<ReleaseInfo, String> {
    if url != RELEASES_API_URL {
        return Err("untrusted update feed".into());
    }
    if !(1..=30).contains(&timeout_secs) {
        return Err("update timeout must be between 1 and 30 seconds".into());
    }
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(std::time::Duration::from_secs(timeout_secs)).user_agent(USER_AGENT).build();

    let response = agent.get(url).set("Accept", "application/vnd.github.v3+json").call().map_err(|e| format!("Failed to check for updates: {e}"))?;

    read_release_response(response.into_reader())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_release_response(reader: impl std::io::Read) -> Result<ReleaseInfo, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader.take(MAX_RESPONSE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("Failed to read release response: {e}"))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("release response exceeds 1 MiB".into());
    }
    let json_val: Value = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid release response: {e}"))?;
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
        assert!(!is_newer_version("0.6.0+local", "0.6.0+release"));

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
        assert!(should_check_periodically(Some(2000), 1000));
    }

    #[test]
    fn installer_selection_and_url_validation() {
        let names = ["windows-x64.msi", "windows-arm64.msi", "windows-x86.msi", "macos-universal.dmg", "linux-aarch64.AppImage", "linux-x86_64.AppImage"];
        let assets: Vec<Value> = names
            .iter()
            .map(|suffix| {
                let name = format!("photocraft-0.7.0-{suffix}");
                json!({"name":name,"browser_download_url":format!("https://github.com/storytold/photocraft/releases/download/v0.7.0/{name}")})
            })
            .collect();
        let mut fixture = json!({"tag_name":"v0.7.0", "html_url":"https://github.com/storytold/photocraft/releases/tag/v0.7.0", "assets":assets});
        let release = parse_release(&fixture).unwrap();
        for (os, arch, appimage, suffix) in [
            ("windows", "x86_64", false, names[0]),
            ("windows", "aarch64", false, names[1]),
            ("windows", "x86", false, names[2]),
            ("macos", "aarch64", false, names[3]),
            ("macos", "x86_64", false, names[3]),
            ("linux", "aarch64", true, names[4]),
            ("linux", "x86_64", true, names[5]),
        ] {
            assert!(release.download_url(os, arch, appimage).unwrap().ends_with(suffix));
        }
        assert!(release.download_url("linux", "x86_64", false).is_none());
        assert!(release.download_url("windows", "riscv64", false).is_none());
        for url in [
            "file:///etc/passwd",
            "https://github.com.evil.test/storytold/photocraft/releases/tag/v0.7.0",
            "https://github.com/storytold/photocraft/releases/tag/../evil",
            "https://github.com/storytold/photocraft/releases/tag/v0.7.0?redirect=evil",
        ] {
            fixture["html_url"] = json!(url);
            assert!(parse_release(&fixture).is_err(), "{url}");
        }
        assert!(!trusted_download_url("https://github.com/storytold/photocraft/releases/download/v0.7.0/../../evil", "v0.7.0"));
        assert!(!is_newer_version("0.6.0", "0.7.0 injected"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn invalid_fetch_parameters_fail_without_network() {
        assert!(fetch_latest_release("http://localhost/", 10).is_err());
        for timeout in [0, 31, u64::MAX] {
            assert!(fetch_latest_release(RELEASES_API_URL, timeout).is_err());
        }
        let oversized = vec![b' '; MAX_RESPONSE_BYTES as usize + 1];
        assert_eq!(read_release_response(oversized.as_slice()).unwrap_err(), "release response exceeds 1 MiB");
        assert!(read_release_response(b"invalid json".as_slice()).is_err());
    }
}
