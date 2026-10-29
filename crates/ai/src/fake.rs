//! Deterministic in-process provider for integration tests. Never a cloud fallback
//! and never registered as a user's configured provider automatically.
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{AiRequest, AiResult, Artifact, Cancellation, Control, ImageProvider, MaskMode, Operation, ProviderCapabilities, ProviderFuture, RgbaImage};

#[derive(Default)]
pub struct FakeProvider {
    submissions: AtomicUsize,
}

impl FakeProvider {
    pub fn submissions(&self) -> usize {
        self.submissions.load(Ordering::Relaxed)
    }
}

impl ImageProvider for FakeProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            operations: vec![Operation::Generate],
            mask_mode: MaskMode::None,
            max_references: 0,
            max_variants: 4,
            max_pixels: 1024 * 1024,
            max_input_bytes: crate::MAX_INPUT_BYTES,
            negative_prompt: true,
            cancellation: Cancellation::OwnJob,
        }
    }

    fn generate<'a>(&'a self, request: &'a AiRequest, control: &'a Control) -> ProviderFuture<'a, AiResult> {
        Box::pin(async move {
            control.check()?;
            request.validate(&self.capabilities())?;
            self.submissions.fetch_add(1, Ordering::Relaxed);
            let mut artifacts = Vec::new();
            let count = request.size.pixels()?;
            for variant in 0..request.variants {
                control.check()?;
                let mut samples = Vec::with_capacity(count.saturating_mul(4));
                for pixel in 0..count {
                    if pixel % 4096 == 0 {
                        control.check()?;
                    }
                    let value = ((request.seed.wrapping_add(u64::from(variant)).wrapping_add(pixel as u64)) % 256) as f32 / 255.0;
                    samples.extend_from_slice(&[value, 0.25, 1.0 - value, 1.0]);
                }
                artifacts.push(Artifact::Image(RgbaImage::new(request.size, samples)?));
            }
            Ok(AiResult { provider_job: None, artifacts })
        })
    }
}
