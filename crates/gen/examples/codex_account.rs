//! Explicit local authentication probe. Never performs image generation or prints credentials.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_gen::{
        Error,
        codex::{AccountInfo, AccountService, CodexProvider},
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    let provider = CodexProvider { timeout: std::time::Duration::from_secs(30), ..Default::default() };
    if std::env::args().any(|arg| arg == "--cancel-login") {
        if std::env::var_os("PHOTOCRAFT_CODEX_HOME").filter(|home| !home.is_empty()).is_none() {
            return Err("An explicit isolated PHOTOCRAFT_CODEX_HOME is required for the cancellation probe".into());
        }
        let cancel = AtomicBool::new(false);
        let result = provider.login(
            &mut |_| {
                cancel.store(true, Ordering::Release);
                Ok(())
            },
            &|| cancel.load(Ordering::Acquire),
        );
        match result {
            Err(Error::Cancelled) if cancel.load(Ordering::Acquire) => println!("{{\"loginUrlReceived\":true,\"cancelled\":true,\"browserOpened\":false}}"),
            Err(error) => return Err(error.into()),
            Ok(_) => return Err("Unexpected completed login in cancellation probe".into()),
        }
    } else {
        let status = match provider.check(&|| false)? {
            AccountInfo::SignedOut => "signedOut",
            AccountInfo::ChatGpt { .. } => "connected",
            AccountInfo::Unsupported => "unsupported",
        };
        println!("{{\"state\":\"{status}\"}}");
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
