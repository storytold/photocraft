//! Non-secret JSON settings and endpoint-bound keys in the OS credential store.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    Error,
    api::{ApiProvider, validate_key},
    api_config::{PROVIDERS, Profile, preset},
};

fn error(message: &str) -> Error {
    Error::Provider(message.into())
}

/// Desktop implementations must never fall back to plaintext storage.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, Error>;
    fn set(&self, account: &str, key: Option<&str>) -> Result<(), Error>;
}

struct NativeSecrets;
impl SecretStore for NativeSecrets {
    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let entry =
                keyring::Entry::new("ai.storyteller.photocraft.image-api", account).map_err(|_| error("Could not access the system credential store"))?;
            match entry.get_password() {
                Ok(key) => Ok(Some(key)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(error("Could not read the system credential store. Unlock it or use a session-only API key.")),
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            let _ = account;
            Err(error("System credential storage is unavailable. Use a session-only API key."))
        }
    }

    fn set(&self, account: &str, key: Option<&str>) -> Result<(), Error> {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let entry =
                keyring::Entry::new("ai.storyteller.photocraft.image-api", account).map_err(|_| error("Could not access the system credential store"))?;
            let result = match key {
                Some(key) => entry.set_password(key),
                None => entry.delete_credential(),
            };
            match result {
                Ok(()) => Ok(()),
                Err(keyring::Error::NoEntry) if key.is_none() => Ok(()),
                Err(_) => Err(error("Could not update the system credential store. Unlock it or choose a session-only API key.")),
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            let _ = (account, key);
            Err(error("System credential storage is unavailable. Use a session-only API key."))
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    remember_keys: BTreeSet<String>,
}

/// Keys and drafts are deliberately neither serializable nor Debug.
#[derive(Default)]
pub struct Settings {
    profiles: BTreeMap<String, Profile>,
    remember_keys: BTreeSet<String>,
    keys: BTreeMap<String, String>,
    path: Option<PathBuf>,
    namespace: String,
    secrets: Option<Arc<dyn SecretStore>>,
}

impl Settings {
    pub fn open(path: PathBuf, namespace: String) -> Result<Self, Error> {
        Self::with_store(path, namespace, Arc::new(NativeSecrets))
    }

    pub fn with_store(path: PathBuf, namespace: String, secrets: Arc<dyn SecretStore>) -> Result<Self, Error> {
        let mut settings = Self { path: Some(path), namespace, secrets: Some(secrets), ..Default::default() };
        let Some(path) = &settings.path else {
            return Ok(settings);
        };
        match std::fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 64 * 1024 {
                    return Err(error("AI provider settings exceed 64 KiB"));
                }
                let config: Config =
                    serde_json::from_slice(&bytes).map_err(|_| error("AI provider settings could not be read; the existing file was left unchanged"))?;
                if config.version != 1 {
                    return Err(error("Unsupported AI provider settings version"));
                }
                for (provider, profile) in &config.profiles {
                    if !PROVIDERS.contains(&provider.as_str()) {
                        return Err(error("Unknown AI provider in settings"));
                    }
                    profile.validate()?;
                }
                settings.profiles = config.profiles;
                if config.remember_keys.iter().any(|p| !PROVIDERS.contains(&p.as_str())) {
                    return Err(error("Unknown AI provider credential setting"));
                }
                settings.remember_keys = config.remember_keys;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error("Could not read AI provider settings; the existing file was left unchanged")),
        }
        Ok(settings)
    }

    pub fn persistent(&self) -> bool {
        self.path.is_some() && self.secrets.is_some()
    }

    pub fn profile(&self, provider: &str) -> Result<Profile, Error> {
        self.profiles.get(provider).cloned().or_else(|| preset(provider)).ok_or_else(|| error("Choose Gemini, Seedream or FLUX"))
    }

    fn account(&self, provider: &str, profile: &Profile) -> String {
        // The full address and protocol bind the secret to its intended destination. A model
        // change on the same endpoint does not need a new credential.
        let destination = blake3::hash(format!("{:?}:{}", profile.protocol, profile.endpoint).as_bytes());
        format!("{}:{provider}:{}", self.namespace, destination.to_hex())
    }

    fn key(&self, provider: &str, profile: &Profile) -> Result<Option<String>, Error> {
        let account = self.account(provider, profile);
        if let Some(key) = self.keys.get(&account) {
            return Ok(Some(key.clone()));
        }
        if !self.remember_keys.contains(provider) {
            return Ok(None);
        }
        match &self.secrets {
            Some(store) => store.get(&account),
            None => Ok(None),
        }
    }

    pub fn has_key(&self, provider: &str) -> Result<bool, Error> {
        Ok(self.key(provider, &self.profile(provider)?)?.is_some())
    }

    pub fn remembered(&self, provider: &str) -> bool {
        self.remember_keys.contains(provider)
    }

    pub fn save(&mut self, provider: &str, profile: Profile, new_key: Option<&str>, remember: bool) -> Result<(), Error> {
        if !PROVIDERS.contains(&provider) {
            return Err(error("Choose Gemini, Seedream or FLUX"));
        }
        profile.validate()?;
        if let Some(key) = new_key {
            validate_key(key)?;
        }
        let old = self.profile(provider)?;
        let account = self.account(provider, &profile);
        let key = match new_key {
            Some(key) => key.to_owned(),
            None => self.key(provider, &profile)?.ok_or_else(|| error("Enter an API key for this address before saving"))?,
        };
        if remember {
            let store = self.secrets.as_ref().ok_or_else(|| error("Persistent credential storage is unavailable. Turn off Remember API key."))?;
            store.set(&account, Some(&key))?;
        }
        let was_remembered = self.remember_keys.contains(provider);
        if was_remembered
            && (!remember || old.endpoint != profile.endpoint || old.protocol != profile.protocol)
            && let Some(store) = &self.secrets
        {
            store.set(&self.account(provider, &old), None)?;
        }
        if remember {
            self.remember_keys.insert(provider.into());
        } else {
            self.remember_keys.remove(provider);
        }
        let previous = self.profiles.insert(provider.into(), profile.clone());
        if let Err(e) = self.persist() {
            if was_remembered {
                self.remember_keys.insert(provider.into());
            } else {
                self.remember_keys.remove(provider);
            }
            match previous {
                Some(profile) => {
                    self.profiles.insert(provider.into(), profile);
                }
                None => {
                    self.profiles.remove(provider);
                }
            }
            return Err(e);
        }
        self.keys.insert(account, key);
        // Never carry the old key in memory to a different address.
        if old.endpoint != profile.endpoint || old.protocol != profile.protocol {
            self.keys.remove(&self.account(provider, &old));
        }
        Ok(())
    }

    pub fn remove_key(&mut self, provider: &str) -> Result<(), Error> {
        let account = self.account(provider, &self.profile(provider)?);
        if self.remember_keys.contains(provider)
            && let Some(store) = &self.secrets
        {
            store.set(&account, None)?;
        }
        self.keys.remove(&account);
        Ok(())
    }

    pub fn provider(&self, provider: &str) -> Result<ApiProvider, Error> {
        let profile = self.profile(provider)?;
        let key = self.key(provider, &profile)?.ok_or_else(|| error("Configure an API key for the selected service first"))?;
        ApiProvider::new(profile, key)
    }

    fn persist(&self) -> Result<(), Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or_else(|| error("Invalid AI settings path"))?;
        std::fs::create_dir_all(parent).map_err(|_| error("Could not create the AI settings folder"))?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| error("Could not write AI provider settings"))?;
        let bytes = serde_json::to_vec_pretty(&Config { version: 1, profiles: self.profiles.clone(), remember_keys: self.remember_keys.clone() })
            .map_err(|_| error("Could not serialize AI provider settings"))?;
        file.write_all(&bytes).and_then(|_| file.as_file().sync_all()).map_err(|_| error("Could not save AI provider settings"))?;
        persist_settings(file, path)
    }
}

fn persist_settings(mut file: tempfile::NamedTempFile, path: &Path) -> Result<(), Error> {
    let mut retries = 0u8;
    let mut delay = std::time::Duration::from_millis(10);
    loop {
        match file.persist(path) {
            Ok(_) => return Ok(()),
            Err(failed) => {
                // Indexers and virus scanners briefly hold Windows files without delete
                // sharing. Retry only the local atomic rename, never an image API request.
                let transient =
                    cfg!(windows) && (failed.error.kind() == std::io::ErrorKind::PermissionDenied || matches!(failed.error.raw_os_error(), Some(32 | 33)));
                if !transient || retries >= 7 {
                    return Err(error(&format!(
                        "Could not replace AI provider settings ({:?}); the configuration file was left unchanged",
                        failed.error.kind()
                    )));
                }
                file = failed.file;
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2);
                retries = retries.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests;
