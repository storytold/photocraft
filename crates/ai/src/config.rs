//! API configuration validation and private native configuration persistence.

use crate::{AiError, Result};

/// Validate a base URL while preserving reverse-proxy path prefixes. Credentials belong in
/// a separate credential field, never in the URL.
pub fn validate_endpoint(value: &str) -> Result<()> {
    let invalid = || AiError::InvalidRequest("API base URL must be an absolute HTTP(S) URL without credentials, query or fragment".into());
    if value.len() > 2048
        || !(value.starts_with("http://") || value.starts_with("https://"))
        || value.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid());
    }
    Ok(())
}

/// A key may travel over plain HTTP only to a machine on the user's own network: loopback,
/// private/link-local addresses, single-label and `.local`/`.lan`/`.internal` names.
pub fn cleartext_credentials_allowed(base: &str) -> bool {
    let Ok(url) = url::Url::parse(base) else { return false };
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            !name.contains('.') || [".local", ".lan", ".internal", ".home.arpa"].iter().any(|suffix| name.ends_with(suffix))
        }
        None => false,
    }
}

/// Name of an environment variable containing a key; an empty name means unauthenticated.
/// Only the name is persisted, so setting preferences cannot expose a key in jobs or logs.
pub fn validate_key_reference(value: &str, optional: bool) -> Result<()> {
    if optional && value.is_empty() {
        return Ok(());
    }
    let mut chars = value.chars();
    if value.len() > 128 || !chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AiError::InvalidRequest("API key reference must be an environment variable name, such as OPENAI_API_KEY".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_accept_local_hosts_ipv6_and_proxy_prefixes() {
        for url in ["http://127.0.0.1:8188", "http://127.0.0.1:8188", "http://[::1]:8188/", "https://example.com/comfy/", "https://api.openai.com/v1"] {
            assert!(validate_endpoint(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn endpoints_reject_malformed_and_secret_bearing_urls() {
        for url in [
            "",
            "localhost:8188",
            "ftp://example.com",
            "https:///",
            "https://user:secret@example.com",
            "https://example.com?key=secret",
            "https://example.com#secret",
            " http://localhost:8188",
            "http://localhost:bad",
            "http://example.com\\secret",
            "https://example.com/\n",
        ] {
            assert!(validate_endpoint(url).is_err(), "{url:?}");
        }
        assert!(validate_endpoint(&format!("https://example.com/{}", "a".repeat(2048))).is_err());
    }

    #[test]
    fn keys_do_not_travel_in_clear_over_the_internet() {
        for url in ["http://127.0.0.1:8188", "http://192.168.1.5:8188", "http://[::1]:8188", "http://comfy-box:8188", "http://gpu.local/v1"] {
            assert!(cleartext_credentials_allowed(url), "{url}");
        }
        for url in ["http://api.example.com/v1", "http://8.8.8.8/v1", "http://[2001:db8::1]/v1", "http://localhost.evil.com/v1"] {
            assert!(!cleartext_credentials_allowed(url), "{url}");
        }
    }

    #[test]
    fn key_references_are_names_without_values_or_shell_expansion() {
        for name in ["OPENAI_API_KEY", "COMFYUI_API_KEY", "_local1"] {
            assert!(validate_key_reference(name, false).is_ok());
        }
        assert!(validate_key_reference("", true).is_ok());
        for name in ["", "sk-secret", "1KEY", "$OPENAI_API_KEY", "KEY=value", " KEY", "KEY\n", "$(secret)"] {
            assert!(validate_key_reference(name, false).is_err(), "{name:?}");
        }
    }
}
