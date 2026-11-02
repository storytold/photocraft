use super::*;
use std::sync::{Mutex, PoisonError};

#[derive(Default)]
struct Vault(Mutex<BTreeMap<String, String>>);
impl SecretStore for Vault {
    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).get(account).cloned())
    }
    fn set(&self, account: &str, key: Option<&str>) -> Result<(), Error> {
        let mut keys = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match key {
            Some(key) => {
                keys.insert(account.into(), key.into());
            }
            None => {
                keys.remove(account);
            }
        }
        Ok(())
    }
}

#[test]
fn remembered_key_survives_restart_only_in_vault_and_is_bound_to_address() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("ai-providers.json");
    let vault = Arc::new(Vault::default());
    let mut settings = Settings::with_store(path.clone(), "test-profile".into(), vault.clone()).unwrap();
    let profile = preset("gemini").unwrap();
    settings.save("gemini", profile.clone(), Some("private-test-key"), true).unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("private-test-key"));
    let mut restarted = Settings::with_store(path.clone(), "test-profile".into(), vault).unwrap();
    assert!(restarted.has_key("gemini").unwrap());
    assert!(restarted.provider("gemini").is_ok());
    let different = Profile { endpoint: "https://another.example/interactions".into(), ..profile };
    assert!(restarted.save("gemini", different, None, true).is_err());
    assert!(restarted.has_key("gemini").unwrap());
    restarted.remove_key("gemini").unwrap();
    assert!(!restarted.has_key("gemini").unwrap());
    assert!(restarted.provider("gemini").is_err());
}

#[test]
fn session_key_is_not_persisted_and_model_change_can_keep_same_key() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("settings.json");
    let vault = Arc::new(Vault::default());
    let mut settings = Settings::with_store(path.clone(), "isolated".into(), vault.clone()).unwrap();
    let profile = preset("seedream").unwrap();
    settings.save("seedream", profile.clone(), Some("session-key"), false).unwrap();
    assert!(settings.has_key("seedream").unwrap());
    let changed = Profile { model: "another-model".into(), ..profile };
    settings.save("seedream", changed.clone(), None, false).unwrap();
    let restarted = Settings::with_store(path.clone(), "isolated".into(), vault.clone()).unwrap();
    assert_eq!(restarted.profile("seedream").unwrap(), changed);
    assert!(!restarted.has_key("seedream").unwrap());
    assert!(vault.0.lock().unwrap().is_empty());
    assert!(!std::fs::read_to_string(path).unwrap().contains("session-key"));
}

#[test]
fn changing_remember_option_migrates_existing_key_and_forgetting_survives_restart() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("settings.json");
    let vault = Arc::new(Vault::default());
    let mut settings = Settings::with_store(path.clone(), "test".into(), vault.clone()).unwrap();
    let profile = preset("gemini").unwrap();
    settings.save("gemini", profile.clone(), Some("test-key"), false).unwrap();
    settings.save("gemini", profile.clone(), None, true).unwrap();
    let mut restarted = Settings::with_store(path.clone(), "test".into(), vault.clone()).unwrap();
    assert!(restarted.has_key("gemini").unwrap());
    restarted.save("gemini", profile, None, false).unwrap();
    assert!(restarted.has_key("gemini").unwrap(), "still available in this session");
    let restarted = Settings::with_store(path, "test".into(), vault.clone()).unwrap();
    assert!(!restarted.has_key("gemini").unwrap());
    assert!(vault.0.lock().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn windows_settings_save_retries_a_short_sharing_violation() {
    use std::{os::windows::fs::OpenOptionsExt, time::Duration};
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("settings.json");
    let vault = Arc::new(Vault::default());
    let mut settings = Settings::with_store(path.clone(), "sharing-test".into(), vault).unwrap();
    let mut profile = preset("gemini").unwrap();
    settings.save("gemini", profile.clone(), Some("test-key"), false).unwrap();
    // Read/write sharing, deliberately without FILE_SHARE_DELETE: an indexer's short read.
    let locked = std::fs::OpenOptions::new().read(true).share_mode(3).open(&path).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(locked);
    });
    profile.model = "another-model".into();
    settings.save("gemini", profile.clone(), None, false).unwrap();
    release.join().unwrap();
    let restarted = Settings::with_store(path, "sharing-test".into(), Arc::new(Vault::default())).unwrap();
    assert_eq!(restarted.profile("gemini").unwrap(), profile);
}

#[cfg(windows)]
#[test]
fn sharing_violation_keeps_the_previous_configuration_when_retries_are_exhausted() {
    use std::os::windows::fs::OpenOptionsExt;
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("settings.json");
    let mut settings = Settings::with_store(path.clone(), "sharing-test".into(), Arc::new(Vault::default())).unwrap();
    let profile = preset("gemini").unwrap();
    settings.save("gemini", profile.clone(), Some("test-key"), false).unwrap();
    let previous = std::fs::read(&path).unwrap();
    let locked = std::fs::OpenOptions::new().read(true).share_mode(3).open(&path).unwrap();
    let changed = Profile { model: "another-model".into(), ..profile.clone() };
    assert!(settings.save("gemini", changed, None, false).is_err());
    assert_eq!(settings.profile("gemini").unwrap(), profile);
    assert_eq!(std::fs::read(path).unwrap(), previous);
    assert!(settings.has_key("gemini").unwrap());
    drop(locked);
}

#[test]
fn corrupt_or_secret_containing_config_is_rejected_without_rewriting() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("settings.json");
    for text in ["bad json", r#"{"version":99,"profiles":{}}"#, r#"{"version":1,"profiles":{},"apiKey":"private"}"#] {
        std::fs::write(&path, text).unwrap();
        assert!(Settings::with_store(path.clone(), "test".into(), Arc::new(Vault::default())).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

#[test]
#[ignore = "explicitly enabled smoke test of a synthetic OS credential"]
fn native_vault_round_trip_uses_only_a_temporary_synthetic_entry() {
    assert_eq!(std::env::var("PHOTOCRAFT_TEST_OS_VAULT").as_deref(), Ok("1"));
    let work = tempfile::tempdir().unwrap();
    let name = format!("photocraft-api-test-{}", blake3::hash(work.path().to_string_lossy().as_bytes()).to_hex());
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = NativeSecrets.set(&self.0, None);
        }
    }
    let _cleanup = Cleanup(name.clone());
    NativeSecrets.set(&name, Some("synthetic-not-an-api-key")).unwrap();
    assert_eq!(NativeSecrets.get(&name).unwrap().as_deref(), Some("synthetic-not-an-api-key"));
    NativeSecrets.set(&name, None).unwrap();
    assert!(NativeSecrets.get(&name).unwrap().is_none());
}
