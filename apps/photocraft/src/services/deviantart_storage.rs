//! Persistent DeviantArt credentials and explicit disconnect state.

use super::{config_dir, write_atomic};
use std::path::Path;

const DEVIANTART_KEYCHAIN: &str = "ai.storyteller.photocraft";
const DEVIANTART_KEYCHAIN_USER: &str = "deviantart-refresh-token";

/// App id, title and switches on disk. A settings save must not put an older refresh token
/// back over the one DeviantArt just issued.
pub(super) fn save_deviantart_file(text: &str) -> Result<(), String> {
    let path = config_dir().map(|dir| dir.join("deviantart.json"));
    let text = current_settings(text, &crate::deviantart_api::current_refresh())?;
    save_with(&text, path.as_deref(), store_deviantart_token, crate::deviantart_api::clear_login)
}

fn current_settings(text: &str, live: &str) -> Result<String, String> {
    let (_, requested) = photocraft_ui_egui::deviantart::without_token(text)?;
    if requested.is_empty() || live.is_empty() {
        return Ok(text.to_string());
    }
    Ok(photocraft_ui_egui::deviantart::with_token(text, live))
}

fn save_with(
    text: &str,
    path: Option<&Path>,
    store: impl FnOnce(&str) -> Result<(), String>,
    clear: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let (rest, token) = photocraft_ui_egui::deviantart::without_token(text)?;
    let mut state: serde_json::Value = serde_json::from_str(&rest).map_err(|e| e.to_string())?;
    let object = state.as_object_mut().ok_or_else(|| "invalid DeviantArt settings".to_string())?;
    object.insert("disconnected".into(), serde_json::json!(token.is_empty()));
    let rest = serde_json::to_string(&state).map_err(|e| e.to_string())?;
    if token.is_empty() {
        // Persist the user's intent before deleting credentials. Clear the live login even
        // if persistence fails; neither error may short-circuit the other operation.
        let written = path.ok_or_else(|| "no config directory".to_string()).and_then(|path| write_atomic(path, rest.as_bytes()));
        let cleared = clear();
        written?;
        return cleared;
    }
    let path = path.ok_or_else(|| "no config directory".to_string())?;
    if store(&token).is_ok() {
        write_atomic(path, rest.as_bytes())
    } else {
        let fallback = photocraft_ui_egui::deviantart::with_token(&rest, &token);
        write_secret_atomic(path, fallback.as_bytes())
    }
}

fn write_secret_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| "no config directory".to_string())?;
    let mut file = tempfile::Builder::new().prefix(".deviantart-").tempfile_in(parent).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file().set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())?;
    }
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.as_file().sync_all().map_err(|error| error.to_string())?;
    file.persist(path).map_err(|error| error.error.to_string())?;
    Ok(())
}

pub(super) fn load_deviantart_file() -> Option<String> {
    let text = std::fs::read_to_string(config_dir()?.join("deviantart.json")).ok()?;
    let (restored, migrate) = restored_settings(&text, load_deviantart_token)?;
    if migrate {
        let _ = save_deviantart_file(&restored);
    }
    Some(restored)
}

fn restored_settings(text: &str, load: impl FnOnce() -> Result<String, String>) -> Option<(String, bool)> {
    let state: serde_json::Value = serde_json::from_str(text).ok()?;
    if state.get("disconnected").and_then(serde_json::Value::as_bool) == Some(true) {
        return Some((photocraft_ui_egui::deviantart::with_token(text, ""), false));
    }
    let (_, embedded) = photocraft_ui_egui::deviantart::without_token(text).ok()?;
    let stored = match load() {
        Ok(token) => token,
        Err(_) => return Some((photocraft_ui_egui::deviantart::with_token(text, &embedded), false)),
    };
    let token = if embedded.is_empty() { &stored } else { &embedded };
    Some((photocraft_ui_egui::deviantart::with_token(text, token), !embedded.is_empty()))
}

pub(crate) fn store_deviantart_token(token: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(DEVIANTART_KEYCHAIN, DEVIANTART_KEYCHAIN_USER).map_err(|error| error.to_string())?;
    if token.is_empty() {
        return match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    entry.set_password(token).map_err(|error| error.to_string())
}

fn load_deviantart_token() -> Result<String, String> {
    let entry = keyring::Entry::new(DEVIANTART_KEYCHAIN, DEVIANTART_KEYCHAIN_USER).map_err(|error| error.to_string())?;
    match entry.get_password() {
        Ok(token) => Ok(token),
        Err(keyring::Error::NoEntry) => Ok(String::new()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn deviantart_disconnect_survives_failed_keychain_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deviantart.json");
        let result = save_with(
            r#"{"clientId":"42"}"#,
            Some(&path),
            |_| panic!("must not store"),
            || {
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(text.contains("\"disconnected\":true"));
                Err("keychain unavailable".into())
            },
        );
        assert!(result.is_err());
        let text = std::fs::read_to_string(path).unwrap();
        let (loaded, _) = restored_settings(&text, || panic!("must not resurrect keychain token")).unwrap();
        assert!(photocraft_ui_egui::deviantart::settings_from_json(&loaded).refresh_token.is_empty());
    }

    #[test]
    fn deviantart_disconnect_clears_login_even_when_settings_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let path = blocked.join("settings.json");
        let cleared = Cell::new(false);
        let result = save_with(
            "{}",
            Some(&path),
            |_| panic!("must not store"),
            || {
                cleared.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(cleared.get());
    }

    #[test]
    fn deviantart_reconnect_clears_marker_and_protects_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deviantart.json");
        save_with(r#"{"disconnected":true,"refreshToken":"new"}"#, Some(&path), |_| Err("offline".into()), || panic!("must not clear")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"disconnected\":false"));
        let (loaded, migrate) = restored_settings(&text, || Err("offline".into())).unwrap();
        assert!(!migrate);
        assert_eq!(photocraft_ui_egui::deviantart::settings_from_json(&loaded).refresh_token, "new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn deviantart_tokenless_settings_are_not_a_disconnect_signal() {
        let (loaded, _) = restored_settings("{}", || Ok("from-keychain".into())).unwrap();
        assert_eq!(photocraft_ui_egui::deviantart::settings_from_json(&loaded).refresh_token, "from-keychain");
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    #[test]
    fn embedded_fallback_wins_over_stale_keychain() {
        let (text, migrate) = restored_settings(r#"{"refreshToken":"rotated"}"#, || Ok("stale".into())).unwrap();
        assert!(migrate);
        assert_eq!(photocraft_ui_egui::deviantart::settings_from_json(&text).refresh_token, "rotated");
    }
    #[test]
    fn live_token_wins_over_stale_ui_but_never_cancels_disconnect() {
        let text = current_settings(r#"{"refreshToken":"stale"}"#, "rotated").unwrap();
        assert_eq!(photocraft_ui_egui::deviantart::settings_from_json(&text).refresh_token, "rotated");
        let text = current_settings(r#"{"refreshToken":""}"#, "rotated").unwrap();
        assert!(photocraft_ui_egui::deviantart::settings_from_json(&text).refresh_token.is_empty());
    }
}
