//! Same-origin browser gateway. Credentials are sent only to authenticated server configuration.
use crate::{AiError, Control, Result, gateway};
use reqwest::{Client, Method, Response};
use std::time::Duration;

pub fn job_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub struct Gateway {
    base: String,
    client: Client,
}
impl Gateway {
    pub fn new(base: &str) -> Result<Self> {
        crate::config::validate_endpoint(base)?;
        Ok(Self { base: base.trim_end_matches('/').into(), client: Client::new() })
    }
    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.client.request(method, format!("{}/{path}", self.base)).fetch_credentials_same_origin().timeout(Duration::from_secs(120))
    }
    async fn bytes(response: Response, maximum: usize) -> Result<Vec<u8>> {
        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(AiError::Authentication),
            429 => return Err(AiError::RateLimited),
            _ => return Err(AiError::Rejected),
        }
        if response.content_length().is_none_or(|n| n > maximum as u64) {
            return Err(AiError::StorageFull);
        }
        let data = response.bytes().await.map_err(|_| AiError::Unavailable)?;
        if data.len() > maximum {
            return Err(AiError::StorageFull);
        }
        Ok(data.to_vec())
    }
    pub async fn configuration(&self) -> Result<serde_json::Value> {
        let response = self.request(Method::GET, "config").send().await.map_err(|_| AiError::Unavailable)?;
        serde_json::from_slice(&Self::bytes(response, 64 * 1024).await?).map_err(|_| AiError::Rejected)
    }
    pub async fn save_configuration(&self, providers: &serde_json::Value) -> Result<()> {
        let response = self.request(Method::PUT, "config").json(providers).send().await.map_err(|_| AiError::Unavailable)?;
        Self::bytes(response, 64 * 1024).await?;
        Ok(())
    }
    async fn cancel(&self, id: &str) {
        let _ = self.request(Method::POST, &format!("jobs/{id}/cancel")).header("content-type", "application/json").body("{}").send().await;
    }
    pub async fn run(&self, request: &gateway::Request, document: &[u8], control: &Control, progress: impl Fn(&gateway::Status)) -> Result<Vec<u8>> {
        control.check()?;
        if !gateway::COMMANDS.contains(&request.command.as_str()) || uuid::Uuid::parse_str(&request.id).is_err() || document.len() > gateway::MAX_DOCUMENT_BYTES
        {
            return Err(AiError::InvalidRequest("gateway request".into()));
        }
        let packet = gateway::encode(request, document)?;
        let submitted = self.request(Method::POST, "jobs").header("content-type", "application/x-photocraft-ai").body(packet).send().await;
        let response = match submitted {
            Ok(response) => response,
            Err(_) => {
                self.cancel(&request.id).await;
                return Err(AiError::OutcomeUnknown);
            }
        };
        Self::bytes(response, 64 * 1024).await?;
        let started = js_sys::Date::now();
        loop {
            if control.check().is_err() {
                self.cancel(&request.id).await;
                return Err(AiError::Cancelled);
            }
            if js_sys::Date::now() - started > 3_600_000.0 {
                self.cancel(&request.id).await;
                return Err(AiError::Timeout);
            }
            let response = self.request(Method::GET, &format!("jobs/{}", request.id)).send().await.map_err(|_| AiError::Unavailable)?;
            let status: gateway::Status = serde_json::from_slice(&Self::bytes(response, 64 * 1024).await?).map_err(|_| AiError::Rejected)?;
            control.report(crate::provider::Stage::Generating, status.progress);
            progress(&status);
            match status.state.as_str() {
                "done" => break,
                "cancelled" => return Err(AiError::Cancelled),
                "failed" => return Err(AiError::Unsupported(status.error.unwrap_or_else(|| "gateway job failed".into()))),
                "running" => {}
                _ => return Err(AiError::Rejected),
            }
            pause().await?;
        }
        control.check()?;
        let response = self.request(Method::GET, &format!("jobs/{}/result", request.id)).send().await.map_err(|_| AiError::Unavailable)?;
        let bytes = Self::bytes(response, gateway::MAX_PACKET_BYTES).await?;
        control.check()?;
        let _ = self.request(Method::DELETE, &format!("jobs/{}", request.id)).send().await;
        Ok(bytes)
    }
}
async fn pause() -> Result<()> {
    let window = web_sys::window().ok_or(AiError::Unavailable)?;
    let mut failed = false;
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        failed = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 500).is_err();
    });
    if failed {
        return Err(AiError::Unavailable);
    }
    wasm_bindgen_futures::JsFuture::from(promise).await.map(|_| ()).map_err(|_| AiError::Unavailable)
}

#[derive(Default)]
struct ConfigurationQueue {
    pending: Option<serde_json::Value>,
    running: bool,
    error: Option<AiError>,
}
thread_local! {
    static CONFIGURATION: std::cell::RefCell<ConfigurationQueue> = std::cell::RefCell::new(ConfigurationQueue::default());
}
/// Save only through the same-origin authenticated API; never use browser storage for keys.
pub fn queue_configuration(providers: serde_json::Value) -> Result<()> {
    let origin = web_sys::window().ok_or(AiError::Unavailable)?.location().origin().map_err(|_| AiError::Unavailable)?;
    let gateway = Gateway::new(&format!("{origin}/api/ai"))?;
    let start = CONFIGURATION.with(|state| {
        let mut state = state.borrow_mut();
        state.pending = Some(providers);
        state.error = None;
        if state.running {
            false
        } else {
            state.running = true;
            true
        }
    });
    if start {
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let pending = CONFIGURATION.with(|state| state.borrow_mut().pending.take());
                let Some(providers) = pending else {
                    break;
                };
                if let Err(error) = gateway.save_configuration(&providers).await {
                    CONFIGURATION.with(|state| state.borrow_mut().error = Some(error.clone()));
                    if let Some(window) = web_sys::window() {
                        let _ = window.alert_with_message(&format!(
                            "PhotoCraft: API configuration was not saved on the server. Reopen AI Settings and apply again. {error}"
                        ));
                    }
                }
            }
            CONFIGURATION.with(|state| state.borrow_mut().running = false);
        });
    }
    Ok(())
}
pub fn configuration_pending() -> bool {
    CONFIGURATION.with(|state| state.borrow().running)
}
pub async fn flush_configuration() -> Result<()> {
    while CONFIGURATION.with(|state| state.borrow().running) {
        pause().await?;
    }
    CONFIGURATION.with(|state| state.borrow().error.clone().map_or(Ok(()), Err))
}
