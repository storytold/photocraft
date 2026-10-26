//! Client for the separately installed, authenticated model dependency service.
use super::Http;
use crate::{AiError, Control, Result, dependencies::Install, secret::Secret};
use reqwest::Method;
use serde_json::Value;
use std::time::{Duration, Instant};
pub struct Dependencies {
    http: Http,
}
impl Dependencies {
    pub fn configured(endpoint: &str, key: Option<&Secret>) -> Result<Self> {
        if endpoint.is_empty() {
            return Err(AiError::Unsupported("Configure the PhotoCraft ComfyUI dependency service in AI Settings".into()));
        }
        Ok(Self { http: Http::configured(endpoint, "", key)? })
    }
    pub async fn report(&self, control: &Control) -> Result<Value> {
        self.http.json(self.http.request(Method::GET, "models"), 64 * 1024, control).await
    }
    pub async fn install(&self, input: &Install, control: &Control) -> Result<Value> {
        input.selected()?;
        let started = self.http.json(self.http.request(Method::POST, "jobs").json(input), 4096, control).await?;
        let id = started
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
            .ok_or(AiError::Rejected)?;
        let path = format!("jobs/{id}");
        let start = Instant::now();
        loop {
            if control.check().is_err() || start.elapsed() > Duration::from_secs(24 * 3600) {
                let _ = self.http.json(self.http.request(Method::DELETE, &path), 4096, &Control::default()).await;
                return Err(if control.check().is_err() { AiError::Cancelled } else { AiError::Timeout });
            }
            let status = self.http.json(self.http.request(Method::GET, &path), 4096, control).await?;
            let bytes = status.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            let total = status.get("total").and_then(Value::as_u64).unwrap_or(1).max(1);
            control.report(crate::provider::Stage::Generating, (bytes as f64 / total as f64).clamp(0.0, 1.0) as f32);
            match status.get("state").and_then(Value::as_str) {
                Some("done") => return self.report(control).await,
                Some("failed") => {
                    return Err(AiError::InvalidRequest(
                        status.get("error").and_then(Value::as_str).unwrap_or("Dependency download failed").chars().take(1024).collect(),
                    ));
                }
                Some("running") => {}
                _ => return Err(AiError::Rejected),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
