//! Document snapshots and explicit, undoable AI result application. No network access here.
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

use photocraft_ai::{AiError, AiRequest, AiResult, Artifact, ArtifactStore, ImageSize, Mask, Operation, RgbaImage};
use photocraft_cms::{Builtin, ColorSpace, Profile, transform};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::{DocState, EngineError, Result, Session, commands::CommandSpec};

const CACHE_BYTES: usize = 1024 * 1024 * 1024;
type UpscalePatch<'a> = dyn FnMut(&RgbaImage) -> Result<RgbaImage> + 'a;

mod identity;
mod recovery;
pub mod remote;
mod scaling;
mod upscale;

fn error(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

/// Composite includes visible layer effects; ActiveRaster reads the layer's raw pixels.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    Composite,
    ActiveRaster,
}

/// Cheap COW capture on the caller thread. `prepare` does the image work on a worker.
/// No numeric input can expand the working area beyond the bounded transport contract.
pub struct Snapshot {
    doc: Arc<Document>,
    incarnation: Weak<()>,
    revision: u64,
    source: Source,
    layer: Option<LayerId>,
    roi: Rect,
}

/// Prepared request plus its immutable destination. Providers never get access to a document.
pub struct PreparedInput {
    request: AiRequest,
    target: Target,
    format: PixelFormat,
    profile: Arc<Profile>,
    selection: Option<Surface>,
    foreground: Option<Surface>,
    original: Arc<Document>,
}

/// Fully validated preview and native layers, prepared on the same worker as the provider job.
pub struct PreparedResult {
    result: AiResult,
    target: Target,
}

struct Target {
    document: DocId,
    revision: u64,
    snapshot: Weak<Document>,
    original: Arc<Document>,
    incarnation: Weak<()>,
    comparison: std::sync::Mutex<Option<(Weak<Document>, bool)>>,
    roi: Rect,
    operation: Operation,
    layers: Vec<Layer>,
    canvas: Option<(ImageSize, u32, u32)>,
    previews: Vec<PreviewImage>,
    working_size: ImageSize,
    detail_upscaler: Option<&'static str>,
}

/// Display copy of the actual masked result, prepared off the UI thread in SDR sRGB.
pub struct PreviewImage {
    pub size: ImageSize,
    pub rgba: Vec<u8>,
}

impl Snapshot {
    /// Colour-managed, bounded composite for vision input; original pixels stay local.
    pub fn analysis_image(&self) -> Result<RgbaImage> {
        let size = scaling::working_size(self.size(), 1024)?;
        let buffer = photocraft_compose::render_reduced_rect(&self.doc, self.roi, size.width, size.height);
        let source: Vec<_> = buffer.px.into_iter().flatten().collect();
        let mut rgba = vec![0.0; size.pixels().map_err(error)?.checked_mul(4).ok_or_else(|| error("AI sample count overflow"))?];
        let profile = crate::color_cmds::composite_profile(&self.doc);
        transform::cached(&profile, Builtin::Srgb.profile(), Default::default()).map_err(error)?.convert_f32(&source, 4, &mut rgba, 4, true);
        for value in &mut rgba {
            if !value.is_finite() {
                return Err(error("AI source contains non-finite pixels"));
            }
            *value = value.clamp(0.0, 1.0);
        }
        RgbaImage::new(size, rgba).map_err(error)
    }

    pub fn size(&self) -> ImageSize {
        ImageSize { width: self.roi.width(), height: self.roi.height() }
    }

    pub fn roi(&self) -> Rect {
        self.roi
    }

    pub fn document(&self) -> DocId {
        self.doc.id
    }

    pub fn capture(state: &DocState, source: Source, margin: u32) -> Result<Self> {
        Self::capture_internal(state, source, margin, true)
    }

    pub fn capture_canvas(state: &DocState, source: Source) -> Result<Self> {
        Self::capture_internal(state, source, 0, false)
    }

    fn capture_internal(state: &DocState, source: Source, margin: u32, selection: bool) -> Result<Self> {
        validate_document(&state.doc)?;
        if state.floating.is_some() {
            return Err(error("Drop the floating selection before starting an AI job"));
        }
        if margin > 1024 {
            return Err(error("AI context margin must be at most 1024 pixels"));
        }
        let bounds = state.doc.bounds();
        if let Some(selection) = &state.doc.selection {
            let limit = i32::MAX / photocraft_geom::TILE_SIZE - 1;
            if selection.channels() != 1
                || selection.tiles().any(|(c, _)| c.tx.checked_abs().is_none_or(|v| v > limit) || c.ty.checked_abs().is_none_or(|v| v > limit))
            {
                return Err(error("AI selection has an unsupported format or coordinate range"));
            }
        }
        let selected = if selection { state.doc.selection.as_ref().map_or(bounds, |s| s.content_bounds().intersect(&bounds)) } else { bounds };
        if selected.is_empty() {
            return Err(error("The AI selection is empty"));
        }
        let margin = i32::try_from(margin).map_err(error)?;
        let roi = Rect::new(
            selected.x0.saturating_sub(margin).max(bounds.x0),
            selected.y0.saturating_sub(margin).max(bounds.y0),
            selected.x1.saturating_add(margin).min(bounds.x1),
            selected.y1.saturating_add(margin).min(bounds.y1),
        );
        scaling::check_destination(roi, state.doc.pixel_format(), true)?;
        let layer = match source {
            Source::Composite => None,
            Source::ActiveRaster => {
                let id = state.active_layer.ok_or_else(|| error("No active raster layer"))?;
                let surface = state
                    .doc
                    .layer(id)
                    .and_then(|l| match &l.content {
                        LayerContent::Raster(s) => Some(s),
                        _ => None,
                    })
                    .ok_or_else(|| error("AI source must be a raster layer"))?;
                if surface.format().mode != state.doc.mode || surface.format().sample != state.doc.depth {
                    return Err(error("AI source format does not match the document"));
                }
                Some(id)
            }
        };
        Ok(Self { doc: state.doc.clone(), incarnation: Arc::downgrade(&state.incarnation), revision: state.revision, source, layer, roi })
    }

    /// Prepare a bounded provider copy; the destination and its selection stay at native size.
    pub fn prepare(self, mut request: AiRequest) -> Result<PreparedInput> {
        if request.operation == Operation::Expand {
            return Err(error(AiError::Unsupported("canvas expansion mapping".into())));
        }
        if request.size.width > self.roi.width() || request.size.height > self.roi.height() || request.source.is_some() || request.edit_mask.is_some() {
            return Err(error("AI working size must fit the captured ROI without a caller-supplied source or mask"));
        }
        // Bound the full outgoing payload before allocating the transport buffers.
        let pixels = request.size.pixels().map_err(error)?;
        if request.variants == 0 || request.variants > photocraft_ai::MAX_VARIANTS {
            return Err(error("AI variant count exceeds the permitted range"));
        }
        request.workflow.validate().map_err(error)?;
        let mask_bytes = usize::from(self.doc.selection.is_some() || request.operation == Operation::SegmentSubject) * 4;
        let native_tile_bytes = self
            .roi
            .tiles()
            .count()
            .checked_mul(photocraft_geom::TILE_SIZE as usize * photocraft_geom::TILE_SIZE as usize)
            .and_then(|n| n.checked_mul(self.doc.pixel_format().bytes_per_pixel() + mask_bytes));
        let ready_bytes = pixels
            .checked_mul(20)
            .and_then(|n| n.checked_add(native_tile_bytes?))
            .and_then(|n| n.checked_mul(request.variants as usize))
            .and_then(|n| n.checked_add(3 * photocraft_ai::MAX_PROMPT_BYTES + 512));
        if ready_bytes.is_none_or(|n| n > CACHE_BYTES) {
            return Err(error("AI preview byte limit exceeded; reduce the working area or variant count"));
        }
        let channels = usize::from(request.operation != Operation::Generate) * 4
            + usize::from(self.doc.selection.is_some() && request.operation != Operation::SegmentSubject && request.operation != Operation::Generate);
        let bytes = pixels
            .checked_mul(channels)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| request.references.iter().try_fold(n, |total, image| total.checked_add(image.bytes())));
        if bytes.is_none_or(|n| n > photocraft_ai::MAX_INPUT_BYTES) {
            return Err(error("AI input byte limit exceeded; reduce the working area"));
        }
        let selection = self
            .doc
            .selection
            .as_ref()
            .map(|surface| {
                if surface.channels() != 1 {
                    return Err(error("AI selection must have one coverage channel"));
                }
                scaling::native_selection(surface, self.roi)
            })
            .transpose()?;
        if selection.as_ref().is_some_and(|s| s.content_bounds().intersect(&self.roi).is_empty()) {
            return Err(error("The AI selection is empty"));
        }
        if matches!(request.operation, Operation::Fill | Operation::Remove) && selection.is_none() {
            return Err(error("AI fill/removal requires a nonempty selection"));
        }
        let mut foreground = None;
        if request.operation != Operation::Generate {
            let (raw, stride, profile) = match self.source {
                Source::ActiveRaster => {
                    let surface = self.layer.and_then(|id| self.doc.layer(id)).and_then(Layer::surface).ok_or_else(|| error("AI source layer is missing"))?;
                    let fmt = surface.format();
                    let reduced = scaling::reduce_surface(surface, self.roi, request.size)?;
                    let mut raw = reduced.read_region(Rect::from_xywh(0, 0, request.size.width, request.size.height));
                    if !fmt.alpha {
                        raw = raw.chunks_exact(fmt.channels()).flat_map(|p| p.iter().copied().chain(std::iter::once(1.0))).collect();
                    }
                    if request.operation == Operation::SegmentSubject {
                        foreground = Some(surface.translated(0, 0, self.roi));
                    }
                    (raw, self.doc.pixel_format().channels(), document_profile(&self.doc)?)
                }
                Source::Composite => {
                    let buf = photocraft_compose::render_reduced_rect(&self.doc, self.roi, request.size.width, request.size.height);
                    let raw: Vec<_> = buf.px.into_iter().flatten().collect();
                    if request.operation == Operation::SegmentSubject {
                        foreground = Some(scaling::native_composite(&self.doc, self.roi)?);
                    }
                    (raw, 4, crate::color_cmds::composite_profile(&self.doc))
                }
            };
            let mut rgba = vec![0.0; request.size.pixels().map_err(error)?.checked_mul(4).ok_or_else(|| error("AI sample count overflow"))?];
            let t = transform::cached(&profile, Builtin::Srgb.profile(), Default::default()).map_err(error)?;
            t.convert_f32(&raw, stride, &mut rgba, 4, true);
            for value in &mut rgba {
                if !value.is_finite() {
                    return Err(error("AI source contains non-finite pixels"));
                }
                // SDR gamut mapping of the transport copy only; original pixels are untouched.
                *value = value.clamp(0.0, 1.0);
            }
            request.source = Some(RgbaImage::new(request.size, rgba).map_err(error)?);
            if request.operation != Operation::SegmentSubject {
                request.edit_mask = selection.as_ref().map(|s| scaling::transport_mask(s, self.roi, request.size)).transpose()?;
            }
        }
        let target = Target {
            document: self.doc.id,
            revision: self.revision,
            snapshot: Arc::downgrade(&self.doc),
            original: self.doc.clone(),
            incarnation: self.incarnation.clone(),
            comparison: std::sync::Mutex::new(None),
            roi: self.roi,
            operation: request.operation,
            layers: vec![],
            canvas: None,
            previews: vec![],
            working_size: request.size,
            detail_upscaler: None,
        };
        request
            .validate(&photocraft_ai::ProviderCapabilities {
                operations: vec![Operation::Generate, Operation::Edit, Operation::Fill, Operation::Remove, Operation::SegmentSubject],
                mask_mode: photocraft_ai::MaskMode::Native,
                max_references: photocraft_ai::MAX_REFERENCES,
                max_variants: photocraft_ai::MAX_VARIANTS,
                max_pixels: photocraft_ai::MAX_PIXELS,
                max_input_bytes: photocraft_ai::MAX_INPUT_BYTES,
                negative_prompt: true,
                cancellation: photocraft_ai::Cancellation::LocalOnly,
            })
            .map_err(error)?;
        Ok(PreparedInput {
            request,
            target,
            format: self.doc.pixel_format(),
            profile: document_profile(&self.doc)?,
            selection,
            foreground,
            original: self.doc.clone(),
        })
    }
}

impl PreparedInput {
    pub fn request(&self) -> &AiRequest {
        &self.request
    }

    /// Do before delivering the external-job completion. All image scans, ICC conversions
    /// and mask creation happen here, never in `retain_ai_result` or `ai.result.apply`.
    pub fn finish(self, result: AiResult) -> Result<PreparedResult> {
        self.finish_mapped(result, None)
    }
    pub fn finish_with_upscaler(self, result: AiResult, mut upscale: impl FnMut(&RgbaImage) -> Result<RgbaImage>) -> Result<PreparedResult> {
        self.finish_mapped(result, Some(&mut upscale))
    }
    fn finish_mapped(mut self, result: AiResult, mut upscale: Option<&mut UpscalePatch<'_>>) -> Result<PreparedResult> {
        result.validate(&self.request).map_err(error)?;
        let mask_bytes = usize::from(self.selection.is_some() || self.request.operation == Operation::SegmentSubject) * 4;
        let result_bytes = result.bytes().map_err(error)?;
        let native_bytes = Some(scaling::check_destination(self.target.roi, self.format, mask_bytes > 0)?)
            .and_then(|n| n.checked_mul(result.artifacts.len()))
            .and_then(|n| n.checked_add(result_bytes));
        if native_bytes.is_none_or(|n| n > CACHE_BYTES) {
            return Err(error(AiError::StorageFull));
        }
        if result.prepared_prompt.as_ref().is_some_and(|p| p.workflow != self.request.workflow || p.original != self.request.prompt) {
            return Err(error("AI result prompt provenance does not match the captured request"));
        }
        for artifact in &result.artifacts {
            let surface = match artifact {
                Artifact::Image(image) => {
                    if let Some(upscale) = upscale.as_deref_mut() {
                        self.target.detail_upscaler = Some("realesrgan-x4plus");
                        scaling::detail_surface(self.format, &self.profile, self.target.roi, image, upscale)?
                    } else {
                        scaling::mapped_surface(self.format, &self.profile, self.target.roi, image)?
                    }
                }
                Artifact::ForegroundMask(_) => self.foreground.clone().ok_or_else(|| error("AI foreground source is missing"))?,
            };
            let coverage = match artifact {
                Artifact::Image(_) => self.selection.clone(),
                Artifact::ForegroundMask(m) => {
                    let mask = scaling::mapped_mask(m, self.target.roi)?;
                    Some(if let Some(selection) = &self.selection { scaling::multiply_masks(mask, selection, self.target.roi) } else { mask })
                }
            };
            let mut layer = Layer::new("AI Result", LayerContent::Raster(surface));
            if let Some(mask) = coverage {
                layer.mask = Some(LayerMask { surface: mask, ..LayerMask::hide_all() });
            }
            let mut preview = (*self.original).clone();
            if let Some((size, left, top)) = self.target.canvas {
                crate::image_cmds::translate_doc(&mut preview, "ai.preview", left as i32, top as i32)?;
                preview.size = photocraft_geom::Size::new(size.width, size.height);
                crate::canvas_geom::refresh(&mut preview, crate::canvas_geom::Refresh::Shapes);
            }
            if self.request.operation == Operation::SegmentSubject {
                preview.layers.clear();
            }
            preview.layers.push(layer.clone());
            self.target.previews.push(scaling::preview(&preview, self.target.roi)?);
            self.target.layers.push(layer);
        }
        if self.target.bytes()?.checked_add(result.bytes().map_err(error)?).is_none_or(|n| n > CACHE_BYTES) {
            return Err(error(AiError::StorageFull));
        }
        Ok(PreparedResult { result, target: self.target })
    }
}

#[cfg(test)]
fn size(roi: Rect) -> Result<ImageSize> {
    let size = ImageSize { width: roi.width(), height: roi.height() };
    if size.width == 0 || size.height == 0 {
        return Err(error("AI dimensions must be positive"));
    }
    Ok(size)
}

fn validate_document(doc: &Document) -> Result<()> {
    if doc.max_group_depth() > 64 {
        return Err(error("AI document nesting exceeds the supported limit"));
    }
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(doc.depth, SampleType::U8 | SampleType::U16) {
        return Err(error(AiError::Unsupported("initial AI bridge supports RGB/Gray at 8/16 bits; HDR and other modes need verified mappings".into())));
    }
    document_profile(doc)?;
    Ok(())
}

fn document_profile(doc: &Document) -> Result<Arc<Profile>> {
    let profile = match &doc.icc_profile {
        Some(bytes) => crate::color_cmds::profile_from_bytes(bytes).map_err(error)?,
        None => crate::color_cmds::working_profile(doc.mode),
    };
    let expected = if doc.mode == ColorMode::Rgb { ColorSpace::Rgb } else { ColorSpace::Gray };
    if profile.color_space != expected {
        return Err(error("AI document profile does not match its colour mode"));
    }
    Ok(profile)
}

fn to_surface(fmt: PixelFormat, profile: &Profile, roi: Rect, pixels: &[f32], from: &Profile) -> Result<Surface> {
    let count = (roi.width() as usize)
        .checked_mul(roi.height() as usize)
        .filter(|n| *n <= photocraft_ai::MAX_PIXELS as usize)
        .ok_or_else(|| error("AI conversion band exceeds the scratch budget"))?;
    if pixels.len() != count.checked_mul(4).ok_or_else(|| error("AI sample count overflow"))? {
        return Err(error("AI result dimensions do not match its pixels"));
    }
    let t = transform::cached(from, profile, Default::default()).map_err(error)?;
    let mut raw = vec![0.0; count.checked_mul(fmt.channels()).ok_or_else(|| error("AI sample count overflow"))?];
    t.convert_f32(pixels, 4, &mut raw, fmt.channels(), true);
    if raw.iter().any(|v| !v.is_finite()) {
        return Err(error("AI colour conversion produced non-finite pixels"));
    }
    let mut surface = Surface::new(fmt);
    surface.write_region(roi, &raw);
    Ok(surface)
}

impl Target {
    fn stale(&self, state: &DocState) -> bool {
        if state.revision != self.revision || !Weak::ptr_eq(&self.incarnation, &Arc::downgrade(&state.incarnation)) {
            return true;
        }
        if Weak::ptr_eq(&self.snapshot, &Arc::downgrade(&state.doc)) {
            return false;
        }
        let mut cache = self.comparison.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = Arc::downgrade(&state.doc);
        if let Some((identity, changed)) = &*cache
            && Weak::ptr_eq(identity, &current)
        {
            return *changed;
        }
        let changed = !identity::same_content(&self.original, &state.doc);
        *cache = Some((current, changed));
        changed
    }

    fn bytes(&self) -> Result<usize> {
        let previews = self.previews.iter().try_fold(0usize, |n, preview| n.checked_add(preview.rgba.len()).ok_or_else(|| error(AiError::StorageFull)))?;
        self.layers
            .iter()
            .flat_map(|l| l.surface().into_iter().chain(l.mask.as_ref().map(|m| &m.surface)))
            .flat_map(Surface::tiles)
            .try_fold(previews, |n, (_, t)| n.checked_add(t.bytes().len()).ok_or_else(|| error(AiError::StorageFull)))
    }
}

pub(crate) struct AiState {
    pub(crate) gateway: Option<String>,
    prompt_model: Option<String>,
    artifacts: ArtifactStore,
    targets: BTreeMap<u64, Target>,
    target_bytes: usize,
    next: u64,
}
impl Default for AiState {
    fn default() -> Self {
        Self { gateway: None, prompt_model: None, artifacts: ArtifactStore::new(CACHE_BYTES, 16), targets: BTreeMap::new(), target_bytes: 0, next: 0 }
    }
}

impl AiState {
    pub(crate) fn forget_document(&mut self, document: DocId) {
        let ids: Vec<_> = self.targets.iter().filter(|(_, t)| t.document == document).map(|(id, _)| *id).collect();
        for id in ids {
            self.discard(id);
        }
    }

    fn discard(&mut self, id: u64) -> bool {
        let Some(target) = self.targets.remove(&id) else { return false };
        self.target_bytes = self.target_bytes.saturating_sub(target.bytes().unwrap_or(CACHE_BYTES));
        self.artifacts.discard(id);
        true
    }
}

impl Session {
    /// Read-only preview for a frontend. It cannot alter the destination or prepared layers.
    pub fn ai_result(&self, id: u64) -> Option<&AiResult> {
        self.ai.artifacts.get(id)
    }
    pub fn ai_preview(&self, id: u64, variant: usize) -> Option<&PreviewImage> {
        self.ai.targets.get(&id)?.previews.get(variant)
    }

    /// Validate completion and retain a preview. Safe to call from an external job's inbox:
    /// edits made while it ran do not discard the preview, but will prevent later application.
    pub fn retain_ai_result(&mut self, prepared: PreparedResult) -> Result<u64> {
        let PreparedResult { result, target } = prepared;
        let state = self.docs.iter().find(|s| s.doc.id == target.document).ok_or_else(|| error("AI target document is closed"))?;
        // Reopening a file can reuse its persisted DocId and reset the revision counter.
        // Preserve genuinely edited previews, but never mistake a new incarnation for it.
        if !Weak::ptr_eq(&target.incarnation, &Arc::downgrade(&state.incarnation))
            || state.revision < target.revision
            || (state.revision == target.revision && target.stale(state))
        {
            return Err(error(AiError::StaleDocument));
        }
        let target_bytes = target.bytes()?;
        let result_bytes = result.bytes().map_err(error)?;
        let used = self
            .ai
            .target_bytes
            .checked_add(self.ai.artifacts.bytes())
            .and_then(|n| n.checked_add(target_bytes))
            .and_then(|n| n.checked_add(result_bytes))
            .ok_or_else(|| error(AiError::StorageFull))?;
        if used > CACHE_BYTES {
            return Err(error(AiError::StorageFull));
        }
        let id = self.ai.next.checked_add(1).ok_or_else(|| error("AI result id limit reached"))?;
        self.ai.artifacts.insert(id, result).map_err(error)?;
        self.ai.targets.insert(id, target);
        self.ai.target_bytes += target_bytes;
        self.ai.next = id;
        Ok(id)
    }
}

fn id(p: &Value, cmd: &str) -> Result<u64> {
    p.get("result")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "result must be a positive integer".into() })
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows: Vec<_> =
        s.ai.targets
            .iter()
            .map(|(id, t)| {
                let state = s.docs.iter().find(|d| d.doc.id == t.document);
                let variants = s.ai.artifacts.get(*id).map_or(0, |r| r.artifacts.len());
                json!({"result":id,"document":t.document.0,"revision":t.revision,"operation":t.operation,
            "workingSize":t.working_size,"outputSize":{"width":t.roi.width(),"height":t.roi.height()},
            "detailUpscaler":t.detail_upscaler,
            "resampled":t.working_size.width != t.roi.width() || t.working_size.height != t.roi.height(),
            "roi":[t.roi.x0,t.roi.y0,t.roi.width(),t.roi.height()],"variants":variants,
                "stale":state.is_none_or(|d| t.stale(d))})
            })
            .collect();
    Ok(json!({"results":rows,"bytes":s.ai.target_bytes + s.ai.artifacts.bytes(),"byteLimit":CACHE_BYTES}))
}

fn discard(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id(p, "ai.result.discard")?;
    if !s.ai.discard(id) {
        return Err(error("No such AI result"));
    }
    Ok(json!({"discarded":id}))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id(p, "ai.result.apply")?;
    let hide_originals = match p.get("hideOriginals") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| error("hideOriginals must be a boolean"))?,
    };
    let variant = match p.get("variant") {
        None => 0,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| error("AI variant must be a nonnegative integer"))?,
    };
    let target = s.ai.targets.get(&id).ok_or_else(|| error("No such AI result"))?;
    if hide_originals && !matches!(target.operation, Operation::SegmentSubject | Operation::Upscale) {
        return Err(error("hideOriginals is only supported for foreground/upscale results"));
    }
    let index = s.docs.iter().position(|d| d.doc.id == target.document).ok_or_else(|| error("AI target document is closed"))?;
    let st = s.docs.get(index).ok_or(EngineError::NoDocument)?;
    if target.stale(st) {
        return Err(error(AiError::StaleDocument));
    }
    if s.job_on(target.document).is_some() || st.floating.is_some() {
        return Err(error("AI target has an exclusive job or floating selection"));
    }
    validate_document(&st.doc)?;
    let layer = target.layers.get(variant).ok_or_else(|| error("No such AI variant"))?.clone();
    let layer_id = layer.id;
    let canvas = target.canvas;
    // Targeted transaction: never change the selected tab; never coalesce with another edit.
    let st = s.docs.get_mut(index).ok_or(EngineError::NoDocument)?;
    let revision = st.revision.checked_add(1).ok_or_else(|| error("Document revision limit reached"))?;
    let before = st.doc.clone();
    let mut doc = (*before).clone();
    if hide_originals {
        for original in &mut doc.layers {
            original.visible = false;
        }
    }
    if let Some((size, left, top)) = canvas {
        crate::image_cmds::translate_doc(&mut doc, "ai.result.apply", left as i32, top as i32)?;
        doc.size = photocraft_geom::Size::new(size.width, size.height);
        crate::canvas_geom::refresh(&mut doc, crate::canvas_geom::Refresh::Shapes);
    }
    doc.layers.push(layer);
    st.doc = Arc::new(doc);
    st.active_layer = Some(layer_id);
    crate::fix_selection(st);
    crate::channel_cmds::fix_view(st);
    st.history.record("Apply AI Result", before, st.layer_target());
    st.history.trim(&st.doc);
    st.coalesce = None;
    st.revision = revision;
    st.last_damage = Some(target.roi);
    Ok(json!({"result":id,"variant":variant,"document":target.document.0,"layer":layer_id.0}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "ai.result.list",
            label: "List AI Results",
            menu: &["Window"],
            shortcut: None,
            params: r#"{} → {results:[{result,document,revision,operation,roi,variants,stale}],bytes,byteLimit}"#,
            enabled: crate::commands::always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "ai.result.discard",
            label: "Discard AI Result",
            menu: &[],
            shortcut: None,
            params: r#"{"result":id} → {discarded}"#,
            enabled: crate::commands::always,
            run: discard,
            journal: false,
        },
        CommandSpec {
            id: "ai.result.openDocument",
            label: "Open AI Result as New Document",
            menu: &[],
            shortcut: None,
            params: r#"{"result":id,"variant":index=0,"name":string?} → {document,layer}; opens the captured composition without changing the original, including stale results"#,
            enabled: crate::commands::always,
            run: recovery::open_document,
            journal: true,
        },
        CommandSpec {
            id: "ai.result.apply",
            label: "Apply AI Result",
            menu: &[],
            shortcut: None,
            params: r#"{"result":id,"variant":index=0,"hideOriginals":bool=false} → {result,variant,document,layer}; hideOriginals only for foreground masks, explicit target, one undo step"#,
            enabled: crate::commands::always,
            run: apply,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod resolution_tests;
#[cfg(test)]
mod tests;

/// Expand all four canvas edges. The original image is shifted only in the confirmed transaction.
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Expansion {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
impl Default for Expansion {
    fn default() -> Self {
        Self { left: 0, top: 0, right: 128, bottom: 0 }
    }
}
impl Snapshot {
    pub fn working_size(target: ImageSize, longest: u32) -> Result<ImageSize> {
        scaling::working_size(target, longest)
    }

    pub fn prepare_generation(mut self, request: AiRequest, output: ImageSize) -> Result<PreparedInput> {
        if request.operation != Operation::Generate || output.width > i32::MAX as u32 || output.height > i32::MAX as u32 {
            return Err(error("Invalid AI output dimensions or operation"));
        }
        let roi = Rect::from_xywh(0, 0, output.width, output.height);
        scaling::check_destination(roi, self.doc.pixel_format(), false)?;
        let original = self.doc.clone();
        let mut copy = (*original).clone();
        copy.selection = None;
        self.doc = Arc::new(copy);
        self.roi = roi;
        let mut input = self.prepare(request)?;
        input.target.snapshot = Arc::downgrade(&original);
        input.target.original = original.clone();
        input.original = original;
        input.target.canvas = Some((output, 0, 0));
        Ok(input)
    }

    pub fn expansion_size(&self, edges: Expansion) -> Result<ImageSize> {
        let width = self.doc.size.width.checked_add(edges.left).and_then(|n| n.checked_add(edges.right)).ok_or_else(|| error("AI expansion overflow"))?;
        let height = self.doc.size.height.checked_add(edges.top).and_then(|n| n.checked_add(edges.bottom)).ok_or_else(|| error("AI expansion overflow"))?;
        let size = ImageSize { width, height };
        if width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(error("AI expansion coordinate overflow"));
        }
        scaling::check_destination(Rect::from_xywh(0, 0, width, height), self.doc.pixel_format(), true)?;
        if [edges.left, edges.top, edges.right, edges.bottom].iter().all(|v| *v == 0) {
            return Err(error("AI expansion requires at least one nonzero edge"));
        }
        Ok(size)
    }
    pub fn prepare_expansion(mut self, mut request: AiRequest, edges: Expansion) -> Result<PreparedInput> {
        let expanded = self.expansion_size(edges)?;
        if request.operation != Operation::Expand {
            return Err(error("Unsupported AI expansion operation"));
        }
        let original = self.doc.clone();
        let mut copy = (*original).clone();
        crate::image_cmds::translate_doc(&mut copy, "ai.prepare", edges.left as i32, edges.top as i32)?;
        copy.size = photocraft_geom::Size::new(expanded.width, expanded.height);
        let roi = copy.bounds();
        let mut coverage = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        coverage.fill_rect(roi, &[1.0]);
        coverage.fill_rect(original.bounds().translate(edges.left as i32, edges.top as i32), &[0.0]);
        coverage.prune();
        copy.selection = Some(coverage);
        self.doc = Arc::new(copy);
        self.roi = roi;
        request.operation = Operation::Fill;
        let mut input = self.prepare(request)?;
        input.request.operation = Operation::Expand;
        input.target.snapshot = Arc::downgrade(&original);
        input.target.original = original.clone();
        input.original = original;
        input.target.operation = Operation::Expand;
        input.target.canvas = Some((expanded, edges.left, edges.top));
        Ok(input)
    }
}

impl Snapshot {
    pub fn canvas_size(&self) -> ImageSize {
        ImageSize { width: self.doc.size.width, height: self.doc.size.height }
    }
    pub fn prepare_background(mut self, request: AiRequest) -> Result<PreparedInput> {
        let original = self.doc.clone();
        let mut copy = (*original).clone();
        copy.selection = None;
        self.doc = Arc::new(copy);
        self.roi = self.doc.bounds();
        let mut input = self.prepare(request)?;
        input.target.snapshot = Arc::downgrade(&original);
        input.target.original = original.clone();
        input.original = original;
        Ok(input)
    }
}
impl PreparedInput {
    pub fn background_mask_request(&self) -> AiRequest {
        let mut request = self.request.clone();
        request.operation = Operation::SegmentSubject;
        request.workflow = photocraft_ai::WorkflowId { name: "birefnet".into(), revision: 1 };
        request.prompt = String::new();
        request.negative_prompt = None;
        request.prompt_preparation = photocraft_ai::PromptPreparation::Manual;
        request.edit_mask = None;
        request
    }
    pub fn use_background_mask(&mut self, foreground: &Mask) -> Result<()> {
        if foreground.size() != self.request.size {
            return Err(error("AI foreground dimensions do not match the background input"));
        }
        let mask = Mask::new(foreground.size(), foreground.values().iter().map(|v| 1.0 - *v).collect()).map_err(error)?;
        if mask.is_empty() {
            return Err(error("No background was found"));
        }
        self.request.operation = Operation::Fill;
        self.request.edit_mask = Some(mask.clone());
        self.selection = Some(scaling::mapped_mask(&mask, self.target.roi)?);
        self.target.operation = Operation::Fill;
        Ok(())
    }
}
