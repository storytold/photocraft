use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{AiError, Result};

pub const MAX_PROMPT_BYTES: usize = 16 * 1024;
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;
pub const MAX_VARIANTS: u32 = 4;
pub const MAX_REFERENCES: usize = 16;
pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Generate,
    Edit,
    Fill,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultKind {
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    None,
    Native,
    LatentWithComposite,
    LocalComposite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cancellation {
    LocalOnly,
    OwnJob,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

impl ImageSize {
    pub fn pixels(self) -> Result<usize> {
        let count = u64::from(self.width) * u64::from(self.height);
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 || count > MAX_PIXELS {
            return Err(AiError::InvalidImage("dimensions exceed the permitted range".into()));
        }
        usize::try_from(count).map_err(|_| AiError::InvalidImage("dimensions exceed addressable memory".into()))
    }
}

/// Explicit SDR sRGB transport copy, independent of the document's storage depth.
/// The engine must perform ICC conversion before constructing this value.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbaImage {
    size: ImageSize,
    samples: Arc<Vec<f32>>,
}

impl RgbaImage {
    pub fn new(size: ImageSize, samples: Vec<f32>) -> Result<Self> {
        let expected = size.pixels()?.checked_mul(4).ok_or_else(|| AiError::InvalidImage("sample count overflow".into()))?;
        if samples.len() != expected || samples.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(AiError::InvalidImage("expected finite SDR RGBA samples".into()));
        }
        Ok(Self { size, samples: Arc::new(samples) })
    }

    pub fn size(&self) -> ImageSize {
        self.size
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
    pub fn bytes(&self) -> usize {
        self.samples.len().saturating_mul(std::mem::size_of::<f32>())
    }
}

/// One means foreground (segmentation) or editable (generation), zero means absent/preserved.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    size: ImageSize,
    values: Arc<Vec<f32>>,
}

impl Mask {
    pub fn new(size: ImageSize, values: Vec<f32>) -> Result<Self> {
        if values.len() != size.pixels()? || values.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(AiError::InvalidImage("expected finite mask coverage in 0..1".into()));
        }
        Ok(Self { size, values: Arc::new(values) })
    }

    pub fn size(&self) -> ImageSize {
        self.size
    }
    pub fn values(&self) -> &[f32] {
        &self.values
    }
    pub fn bytes(&self) -> usize {
        self.values.len().saturating_mul(std::mem::size_of::<f32>())
    }
    pub fn is_empty(&self) -> bool {
        self.values.iter().all(|v| *v == 0.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowId {
    pub name: String,
    pub revision: u32,
}

impl WorkflowId {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.name.len() > 128
            || self.revision == 0
            || !self.name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(AiError::InvalidWorkflow("invalid workflow identity".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AiRequest {
    pub operation: Operation,
    pub workflow: WorkflowId,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub size: ImageSize,
    pub variants: u32,
    pub seed: u64,
    pub source: Option<RgbaImage>,
    pub edit_mask: Option<Mask>,
    pub references: Vec<RgbaImage>,
}

impl AiRequest {
    pub fn validate(&self, caps: &ProviderCapabilities) -> Result<()> {
        self.workflow.validate()?;
        let pixels = self.size.pixels()?;
        let source_bytes = self.source.as_ref().map_or(0, RgbaImage::bytes);
        let mask_bytes = self.edit_mask.as_ref().map_or(0, Mask::bytes);
        let input_bytes =
            source_bytes.checked_add(mask_bytes).and_then(|start| self.references.iter().try_fold(start, |total, image| total.checked_add(image.bytes())));
        if input_bytes.is_none_or(|bytes| bytes > caps.max_input_bytes.min(MAX_INPUT_BYTES)) {
            return Err(AiError::InvalidRequest("input byte limit exceeded".into()));
        }
        if !caps.operations.contains(&self.operation) {
            return Err(AiError::Unsupported("operation".into()));
        }
        if caps.max_pixels == 0
            || pixels as u64 > caps.max_pixels.min(MAX_PIXELS)
            || self.variants == 0
            || self.variants > caps.max_variants.min(MAX_VARIANTS)
            || self.references.len() > caps.max_references.min(MAX_REFERENCES)
        {
            return Err(AiError::InvalidRequest("provider size, variant or reference limit exceeded".into()));
        }
        if self.prompt.len() > MAX_PROMPT_BYTES || self.negative_prompt.as_ref().is_some_and(|v| v.len() > MAX_PROMPT_BYTES) {
            return Err(AiError::InvalidRequest("prompt is too long".into()));
        }
        if self.negative_prompt.is_some() && !caps.negative_prompt {
            return Err(AiError::Unsupported("negative prompt".into()));
        }
        if matches!(self.operation, Operation::Generate | Operation::Edit | Operation::Fill) && self.prompt.trim().is_empty() {
            return Err(AiError::InvalidRequest("a prompt is required".into()));
        }
        if self.operation != Operation::Generate && self.source.is_none() {
            return Err(AiError::InvalidRequest("source image is required".into()));
        }
        if self.operation == Operation::Generate && (self.source.is_some() || self.edit_mask.is_some()) {
            return Err(AiError::InvalidRequest("generation cannot contain an edit source or mask".into()));
        }
        if matches!(self.operation, Operation::Fill | Operation::Remove) && self.edit_mask.is_none() {
            return Err(AiError::InvalidRequest("an edit mask is required".into()));
        }
        if let Some(mask) = &self.edit_mask {
            if caps.mask_mode == MaskMode::None {
                return Err(AiError::Unsupported("edit mask".into()));
            }
            if self.source.as_ref().is_none_or(|s| s.size() != mask.size()) || mask.size() != self.size || mask.is_empty() {
                return Err(AiError::InvalidRequest("mask and source must match the target size and contain editable pixels".into()));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub operations: Vec<Operation>,
    pub mask_mode: MaskMode,
    pub max_references: usize,
    pub max_variants: u32,
    pub max_pixels: u64,
    pub max_input_bytes: usize,
    pub negative_prompt: bool,
    pub cancellation: Cancellation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Artifact {
    Image(RgbaImage),
}

impl Artifact {
    pub fn kind(&self) -> ResultKind {
        match self {
            Self::Image(_) => ResultKind::Image,
        }
    }
    pub fn size(&self) -> ImageSize {
        match self {
            Self::Image(v) => v.size(),
        }
    }
    pub fn bytes(&self) -> usize {
        match self {
            Self::Image(v) => v.bytes(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AiResult {
    pub provider_job: Option<String>,
    pub artifacts: Vec<Artifact>,
}

impl AiResult {
    /// Account for bounded image data and textual provenance in the artifact store.
    pub fn bytes(&self) -> Result<usize> {
        if self.artifacts.is_empty() || self.artifacts.len() > MAX_VARIANTS as usize || self.provider_job.as_ref().is_some_and(|id| id.len() > 256) {
            return Err(AiError::InvalidImage("invalid result count or metadata".into()));
        }
        let bytes = self.provider_job.as_ref().map_or(0, String::len);
        self.artifacts.iter().try_fold(bytes, |n, a| n.checked_add(a.bytes())).ok_or(AiError::StorageFull)
    }

    pub fn validate(&self, request: &AiRequest) -> Result<()> {
        self.bytes()?;
        if self.artifacts.len() != request.variants as usize {
            return Err(AiError::InvalidImage("unexpected variant count".into()));
        }
        if self.artifacts.iter().any(|a| a.kind() != ResultKind::Image || a.size() != request.size) {
            return Err(AiError::InvalidImage("unexpected output kind or dimensions".into()));
        }
        Ok(())
    }
}
