//! Provider-neutral AI contracts. Network runtimes are injected by the application;
//! this crate never mutates documents or implicitly contacts a provider.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod artifact;
#[cfg(target_arch = "wasm32")]
pub mod browser;
pub mod config;
pub mod dependencies;
pub mod fake;
pub mod gateway;
pub mod image_resolution;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
pub mod openai_prompt;
pub mod provider;
pub mod secret;
pub mod types;
pub mod vision;
pub mod workflow;

pub use artifact::ArtifactStore;
pub use provider::{Control, ImageProvider, PromptProvider, ProviderFuture, generate};
pub use types::*;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    #[error("AI provider is unavailable")]
    Unavailable,
    #[error("AI operation is unsupported: {0}")]
    Unsupported(String),
    #[error("Invalid AI request: {0}")]
    InvalidRequest(String),
    #[error("Invalid AI workflow: {0}")]
    InvalidWorkflow(String),
    #[error("Invalid AI image: {0}")]
    InvalidImage(String),
    #[error("AI authentication failed")]
    Authentication,
    #[error("OpenAI prompt authentication failed; check the prompt endpoint and its API key")]
    PromptAuthentication,
    #[error("API key environment variable '{0}' is missing or invalid; enter the API key in AI Settings or configure the optional environment variable")]
    MissingCredential(String),
    #[error("AI provider rate limit reached")]
    RateLimited,
    #[error("AI request was rejected")]
    Rejected,
    #[error("AI job was cancelled")]
    Cancelled,
    #[error("AI job timed out")]
    Timeout,
    #[error("AI submit outcome is unknown; do not automatically resubmit")]
    OutcomeUnknown,
    #[error("The target document has changed")]
    StaleDocument,
    #[error("AI artifact storage limit reached")]
    StorageFull,
}

pub type Result<T> = std::result::Result<T, AiError>;

#[cfg(test)]
mod tests;
