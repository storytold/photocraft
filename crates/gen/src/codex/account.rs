//! Codex owns the browser callback, credentials and token refresh. No tokens leave its RPC.
use super::{CodexProvider, Rpc};
use crate::Error;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountInfo {
    SignedOut,
    ChatGpt { plan: Option<String> },
    Unsupported,
}

/// Blocking transport seam; callers run it on a worker, never the UI thread.
/// Implementations must poll cancellation promptly and close their subprocesses before returning.
pub trait AccountService: Send + Sync {
    fn check(&self, cancelled: &dyn Fn() -> bool) -> Result<AccountInfo, Error>;
    fn login(&self, on_url: &mut dyn FnMut(String) -> Result<(), Error>, cancelled: &dyn Fn() -> bool) -> Result<AccountInfo, Error>;
}

pub fn valid_login_url(url: &str) -> bool {
    url.len() <= 8192 && !url.chars().any(char::is_control) && (url.starts_with("https://auth.openai.com/") || url.starts_with("https://chatgpt.com/"))
}

fn account_info(value: &Value) -> Result<AccountInfo, Error> {
    match value.get("account") {
        Some(Value::Null) => Ok(AccountInfo::SignedOut),
        Some(account) => match account.get("type").and_then(Value::as_str) {
            Some("chatgpt") => {
                // No email, account ids or arbitrary server strings are retained by PhotoCraft.
                let plan = account
                    .get("planType")
                    .and_then(Value::as_str)
                    .filter(|plan| !plan.is_empty() && plan.len() <= 64 && plan.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
                    .map(str::to_owned);
                Ok(AccountInfo::ChatGpt { plan })
            }
            Some(_) => Ok(AccountInfo::Unsupported),
            None => Err(Error::Provider("Codex returned invalid account information".into())),
        },
        None => Err(Error::Provider("Codex returned no account information".into())),
    }
}

trait Transport {
    fn request(&mut self, id: u64, method: &str, params: Value, cancel: &dyn Fn() -> bool) -> Result<Value, Error>;
    fn next(&mut self, cancel: &dyn Fn() -> bool) -> Result<Value, Error>;
    fn send(&mut self, value: Value) -> Result<(), Error>;
}

impl Transport for Rpc {
    fn request(&mut self, id: u64, method: &str, params: Value, cancel: &dyn Fn() -> bool) -> Result<Value, Error> {
        Rpc::request(self, id, method, params, cancel)
    }
    fn next(&mut self, cancel: &dyn Fn() -> bool) -> Result<Value, Error> {
        let value = Rpc::next(self, cancel)?;
        self.reject_request(&value)?;
        Ok(value)
    }
    fn send(&mut self, value: Value) -> Result<(), Error> {
        Rpc::send(self, value)
    }
}

fn initialize(rpc: &mut impl Transport, cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    rpc.request(1, "initialize", json!({"clientInfo":{"name":"photocraft_ai","title":"PhotoCraft AI","version":env!("CARGO_PKG_VERSION")}}), cancel)?;
    rpc.send(json!({"method":"initialized"}))
}

fn managed_login(rpc: &mut impl Transport, on_url: &mut dyn FnMut(String) -> Result<(), Error>, cancel: &dyn Fn() -> bool) -> Result<AccountInfo, Error> {
    let started = rpc.request(2, "account/login/start", json!({"type":"chatgpt"}), cancel)?;
    let login_id = started
        .get("loginId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .ok_or_else(|| Error::Provider("Codex returned no login id".into()))?
        .to_owned();
    let result = (|| {
        let url = started
            .get("authUrl")
            .and_then(Value::as_str)
            .filter(|url| valid_login_url(url))
            .ok_or_else(|| Error::Provider("Codex returned an unsupported sign-in URL".into()))?;
        on_url(url.to_owned())?;
        loop {
            let value = rpc.next(cancel)?;
            if value.get("method").and_then(Value::as_str) != Some("account/login/completed")
                || value.pointer("/params/loginId").and_then(Value::as_str) != Some(login_id.as_str())
            {
                continue;
            }
            if value.pointer("/params/success").and_then(Value::as_bool) != Some(true) {
                return Err(Error::Provider("ChatGPT sign-in was cancelled or failed. Try signing in again.".into()));
            }
            let account = account_info(&rpc.request(3, "account/read", json!({"refreshToken":false}), cancel)?)?;
            if !matches!(account, AccountInfo::ChatGpt { .. }) {
                return Err(Error::Provider("ChatGPT sign-in completed without a ChatGPT account".into()));
            }
            return Ok(account);
        }
    })();
    if result.is_err() {
        // Best effort before the private server closes its callback listener. Never sign out
        // or delete credentials: a browser callback may already have completed concurrently.
        let _ = rpc.send(json!({"id":4,"method":"account/login/cancel","params":{"loginId":login_id}}));
    }
    result
}

impl AccountService for CodexProvider {
    fn check(&self, cancelled: &dyn Fn() -> bool) -> Result<AccountInfo, Error> {
        let provider = Self { timeout: std::time::Duration::from_secs(30), ..self.clone() };
        // Merely checking an account must not enforce a different login method and log out
        // an existing API-key user. The image path separately requires ChatGPT before inference.
        let mut rpc = Rpc::start(&provider, &std::env::temp_dir(), false)?;
        initialize(&mut rpc, cancelled)?;
        let value = rpc.request(2, "account/read", json!({"refreshToken":true}), cancelled)?;
        account_info(&value)
    }

    fn login(&self, on_url: &mut dyn FnMut(String) -> Result<(), Error>, cancelled: &dyn Fn() -> bool) -> Result<AccountInfo, Error> {
        let mut rpc = Rpc::start(self, &std::env::temp_dir(), false)?;
        initialize(&mut rpc, cancelled)?;
        managed_login(&mut rpc, on_url, cancelled).map_err(|error| match error {
            Error::Timeout => Error::Provider("ChatGPT sign-in timed out. Start sign-in again.".into()),
            other => other,
        })
    }
}

#[cfg(test)]
mod tests;
