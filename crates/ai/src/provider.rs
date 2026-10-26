use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::{AiError, AiRequest, AiResult, Operation, PreparedPrompt, PromptPreparation, ProviderCapabilities, Result};

#[cfg(not(target_arch = "wasm32"))]
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + 'a>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Preparing,
    Prompting,
    Generating,
    Ready,
}

/// Provider cancellation and progress, shared across both stages of a pipeline.
#[derive(Clone, Default)]
pub struct Control {
    cancelled: Arc<AtomicBool>,
    progress: Arc<Mutex<(Stage, f32)>>,
    cancelled_external: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    report_external: Option<Arc<dyn Fn(Stage, f32) + Send + Sync>>,
}

impl Control {
    /// Connect a provider pipeline to the host's job cancellation and progress.
    pub fn connected(cancelled: impl Fn() -> bool + Send + Sync + 'static, report: impl Fn(Stage, f32) + Send + Sync + 'static) -> Self {
        Self { cancelled_external: Some(Arc::new(cancelled)), report_external: Some(Arc::new(report)), ..Self::default() }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) || self.cancelled_external.as_ref().is_some_and(|f| f()) { Err(AiError::Cancelled) } else { Ok(()) }
    }
    pub fn report(&self, stage: Stage, fraction: f32) {
        if !fraction.is_finite() {
            return;
        }
        let mut state = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
        let fraction = fraction.clamp(0.0, 1.0);
        let notify = fraction >= state.1;
        if notify {
            *state = (stage, fraction);
        }
        drop(state);
        if notify && let Some(report) = &self.report_external {
            report(stage, fraction);
        }
    }
    pub fn progress(&self) -> (Stage, f32) {
        *self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Text-only prompt preparation. Implementations must not return executable graphs.
#[cfg(not(target_arch = "wasm32"))]
pub trait ProviderBounds: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> ProviderBounds for T {}
#[cfg(target_arch = "wasm32")]
pub trait ProviderBounds {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> ProviderBounds for T {}

pub trait PromptProvider: ProviderBounds {
    fn prepare<'a>(&'a self, request: &'a AiRequest, capabilities: &'a ProviderCapabilities, control: &'a Control) -> ProviderFuture<'a, PreparedPrompt>;
}

/// Image/segmentation role, independent of any UI or document representation.
pub trait ImageProvider: ProviderBounds {
    fn capabilities(&self) -> ProviderCapabilities;
    fn generate<'a>(&'a self, request: &'a AiRequest, prompt: Option<&'a PreparedPrompt>, control: &'a Control) -> ProviderFuture<'a, AiResult>;
}

/// One pipeline: validate capabilities before a paid text call, prepare once, validate
/// again, then submit images. Cancellation/errors never silently fall back or retry.
pub async fn generate(request: &AiRequest, images: &dyn ImageProvider, prompts: Option<&dyn PromptProvider>, control: &Control) -> Result<AiResult> {
    control.check()?;
    let caps = images.capabilities();
    request.validate(&caps)?;
    control.report(Stage::Preparing, 0.05);
    let prepared = if request.operation == Operation::SegmentSubject {
        None
    } else {
        let prompt = match request.prompt_preparation {
            PromptPreparation::Manual => PreparedPrompt {
                original: request.prompt.clone(),
                positive: if request.operation == Operation::Remove && request.prompt.trim().is_empty() {
                    "Remove the marked object and reconstruct the surrounding background.".into()
                } else {
                    request.prompt.clone()
                },
                negative: request.negative_prompt.clone(),
                workflow: request.workflow.clone(),
                text_model: None,
            },
            PromptPreparation::OpenAiText => {
                control.report(Stage::Prompting, 0.1);
                prompts
                    .ok_or(AiError::Unavailable)?
                    .prepare(request, &caps, control)
                    .await
                    .map_err(|error| if error == AiError::Authentication { AiError::PromptAuthentication } else { error })?
            }
        };
        control.check()?;
        prompt.validate(request, &caps)?;
        Some(prompt)
    };
    // Cancellation between stages must prevent the ComfyUI submit.
    control.check()?;
    control.report(Stage::Generating, 0.25);
    let mut result = images.generate(request, prepared.as_ref(), control).await?;
    control.check()?;
    // The pipeline owns provenance; untrusted image responses cannot replace it.
    result.prepared_prompt = prepared;
    result.validate(request)?;
    control.report(Stage::Ready, 1.0);
    Ok(result)
}
