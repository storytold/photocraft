//! Generative AI services for the desktop app (#41): API keys in the OS credential store and an
//! HTTPS transport for `photocraft-genai` providers. Nothing here runs until the user sets up a
//! provider in Generative AI Settings.

use std::io::Read as _;
use std::sync::Arc;
use std::time::Duration;

use photocraft_genai::{ApiKey, GenError, HttpRequest, HttpResponse, HttpTransport};
use photocraft_ui_egui::GenAiServices;

/// The credential store's service name; the account is the provider id (`gemini`).
#[cfg(any(target_os = "macos", target_os = "windows"))]
const SERVICE: &str = "ai.storyteller.photocraft.genai";
/// An image request can take a while (large outputs, busy service).
const TIMEOUT: Duration = Duration::from_secs(180);

/// Blocking HTTPS for provider requests, run from engine background jobs. HTTPS only, no
/// redirects (a redirect could carry the key header to another host), bounded response size.
struct Https {
    agent: ureq::Agent,
}

impl Https {
    fn new() -> Self {
        let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).https_only(true).max_redirects(0).http_status_as_error(false).build();
        Self { agent: config.new_agent() }
    }
}

impl HttpTransport for Https {
    fn post(&self, request: HttpRequest) -> photocraft_genai::Result<HttpResponse> {
        let mut call = self.agent.post(&request.url);
        for (name, value) in &request.headers {
            call = call.header(*name, value.as_str());
        }
        // ureq's error text names the host and the failure, never header values.
        let mut response = call.send(&request.body[..]).map_err(|e| GenError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(photocraft_genai::MAX_RESPONSE as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| GenError::Network(format!("reading the response failed: {e}")))?;
        Ok(HttpResponse { status, body })
    }
}

fn build(provider: &str, model: &str, key: ApiKey) -> Result<photocraft_engine::genai_cmds::SharedProvider, String> {
    match provider {
        photocraft_genai::gemini::ID => {
            let p = photocraft_genai::gemini::GeminiProvider::new(key, model, Https::new()).map_err(|e| e.to_string())?;
            Ok(Arc::new(p))
        }
        other => Err(format!("unknown generative AI provider `{other}`")),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod store {
    fn entry(provider: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(super::SERVICE, provider).map_err(|e| e.to_string())
    }
    pub fn load(provider: &str) -> Result<Option<String>, String> {
        match entry(provider)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("could not read the API key: {e}")),
        }
    }
    pub fn save(provider: &str, key: &str) -> Result<(), String> {
        entry(provider)?.set_password(key).map_err(|e| format!("could not save the API key: {e}"))
    }
    pub fn delete(provider: &str) -> Result<(), String> {
        match entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("could not remove the API key: {e}")),
        }
    }
    #[cfg(target_os = "macos")]
    pub const NAME: &str = "the macOS Keychain";
    #[cfg(target_os = "windows")]
    pub const NAME: &str = "Windows Credential Manager";
}

/// Elsewhere (Linux, the BSDs) the key lives in process memory only and is asked for again
/// after a restart, rather than written anywhere in plain text.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod store {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    fn keys() -> &'static Mutex<HashMap<String, String>> {
        static KEYS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        KEYS.get_or_init(Mutex::default)
    }
    pub fn load(provider: &str) -> Result<Option<String>, String> {
        Ok(keys().lock().map_err(|_| "key store unavailable")?.get(provider).cloned())
    }
    pub fn save(provider: &str, key: &str) -> Result<(), String> {
        keys().lock().map_err(|_| "key store unavailable")?.insert(provider.into(), key.into());
        Ok(())
    }
    pub fn delete(provider: &str) -> Result<(), String> {
        keys().lock().map_err(|_| "key store unavailable")?.remove(provider);
        Ok(())
    }
    pub const NAME: &str = "memory until PhotoCraft quits";
}

pub fn services() -> GenAiServices {
    GenAiServices {
        load_key: Box::new(store::load),
        save_key: Box::new(store::save),
        delete_key: Box::new(store::delete),
        provider: Box::new(build),
        key_store_name: store::NAME,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_gemini_and_rejects_unknown_providers_and_models() {
        let key = || ApiKey::new("k").unwrap();
        let p = build("gemini", "", key()).unwrap();
        assert_eq!(p.info().model, photocraft_genai::gemini::DEFAULT_MODEL);
        assert!(build("gemini", "../etc", key()).is_err());
        assert!(build("other", "", key()).is_err());
    }

    #[test]
    fn the_transport_refuses_plain_http_without_sending() {
        let r = Https::new().post(HttpRequest { url: "http://example.invalid/".into(), headers: vec![("x-goog-api-key", "secret".into())], body: Vec::new() });
        let e = r.err().unwrap();
        assert!(matches!(e, GenError::Network(_)), "{e:?}");
        assert!(!e.to_string().contains("secret"));
    }
}
