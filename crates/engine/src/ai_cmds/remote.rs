//! Portable prepared layers. Browser document identity remains local to its session.
use super::*;
use photocraft_ai::{PreparedPrompt, gateway};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    operation: Operation,
    roi: Rect,
    canvas: Option<(ImageSize, u32, u32)>,
    working_size: ImageSize,
    detail_upscaler: Option<String>,
    prepared_prompt: Option<PreparedPrompt>,
    preview_size: ImageSize,
}

/// Restrict expansion while decoding an authenticated but still untrusted response.
pub fn load_document(bytes: &[u8]) -> Result<Document> {
    if bytes.len() > gateway::MAX_DOCUMENT_BYTES {
        return Err(error(AiError::StorageFull));
    }
    let options = photocraft_format::LoadOptions {
        max_manifest_bytes: 8 * 1024 * 1024,
        max_blob_bytes: 256 * 1024 * 1024,
        max_total_bytes: CACHE_BYTES as u64,
        preserve_ids: false,
        ..Default::default()
    };
    let document = photocraft_format::load_from_bytes_with(bytes, &options).map_err(error)?;
    validate_document(&document)?;
    Ok(document)
}

impl Session {
    /// Platform-supplied default; explicit per-request model IDs retain precedence.
    pub fn configure_ai_prompt_model(&mut self, model: &str) -> Result<()> {
        photocraft_ai::openai_prompt::validate_model(model).map_err(error)?;
        self.ai.prompt_model = Some(model.into());
        Ok(())
    }
    pub fn ai_prompt_model(&self) -> &str {
        self.ai.prompt_model.as_deref().unwrap_or("gpt-4.1-mini")
    }
    /// Install a same-origin gateway from the browser shell; no request is made here.
    pub fn configure_web_ai_gateway(&mut self, endpoint: &str) -> Result<()> {
        photocraft_ai::config::validate_endpoint(endpoint).map_err(error)?;
        self.ai.gateway = Some(endpoint.trim_end_matches('/').into());
        Ok(())
    }
    /// Export one prepared native layer, not the original document or float image buffers.
    pub fn export_ai_gateway_result(&self, id: u64) -> Result<Vec<u8>> {
        let target = self.ai.targets.get(&id).ok_or_else(|| error("No such AI result"))?;
        let preview = target.previews.first().ok_or_else(|| error("Missing AI preview"))?;
        let layer = target.layers.first().ok_or_else(|| error("Missing AI layer"))?;
        let size = target.canvas.map_or(target.original.size, |(size, _, _)| photocraft_geom::Size::new(size.width, size.height));
        let mut doc = Document::new("AI gateway layer", size, target.original.mode, target.original.depth);
        doc.icc_profile = target.original.icc_profile.clone();
        doc.layers.push(layer.clone());
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).map_err(error)?;
        if bytes.len() > gateway::MAX_DOCUMENT_BYTES || preview.rgba.len() > gateway::MAX_PREVIEW_BYTES {
            return Err(error(AiError::StorageFull));
        }
        let metadata = Metadata {
            operation: target.operation,
            roi: target.roi,
            canvas: target.canvas,
            working_size: target.working_size,
            detail_upscaler: target.detail_upscaler.map(str::to_owned),
            prepared_prompt: self.ai.artifacts.get(id).and_then(|result| result.prepared_prompt.clone()),
            preview_size: preview.size,
        };
        let mut payload = preview.rgba.clone();
        payload.extend_from_slice(&bytes);
        gateway::encode(&metadata, &payload).map_err(error)
    }
}

impl Snapshot {
    pub fn gateway_document(&self) -> Result<Vec<u8>> {
        let bytes = photocraft_format::save_to_bytes(&self.doc, &Default::default()).map_err(error)?;
        if bytes.len() > gateway::MAX_DOCUMENT_BYTES {
            return Err(error("AI gateway document exceeds the upload budget"));
        }
        Ok(bytes)
    }
    /// Validate remote geometry against the requested local operation before retaining it.
    pub fn finish_gateway(self, bytes: &[u8], operation: Operation, roi: Rect, canvas: Option<(ImageSize, u32, u32)>) -> Result<PreparedResult> {
        let (metadata, payload): (Metadata, _) = gateway::decode(bytes).map_err(error)?;
        if metadata.operation != operation || metadata.roi != roi || metadata.canvas != canvas || roi.is_empty() {
            return Err(error("AI gateway returned unexpected destination geometry"));
        }
        let format = self.doc.pixel_format();
        scaling::check_destination(roi, format, true)?;
        metadata.working_size.pixels().map_err(error)?;
        if metadata.preview_size.width > 1024 || metadata.preview_size.height > 1024 {
            return Err(error("AI gateway preview exceeds its budget"));
        }
        let preview_bytes = metadata.preview_size.pixels().map_err(error)?.checked_mul(4).ok_or_else(|| error(AiError::StorageFull))?;
        let rgba = payload.get(..preview_bytes).ok_or_else(|| error("Truncated AI gateway preview"))?.to_vec();
        let layer_bytes = payload.get(preview_bytes..).ok_or_else(|| error("Missing AI gateway layer"))?;
        let mut document = load_document(layer_bytes)?;
        let expected_size = canvas.map_or(self.doc.size, |(size, _, _)| photocraft_geom::Size::new(size.width, size.height));
        if document.size != expected_size
            || document.mode != self.doc.mode
            || document.depth != self.doc.depth
            || document.icc_profile != self.doc.icc_profile
            || document.layers.len() != 1
        {
            return Err(error("AI gateway layer does not match the captured document format"));
        }
        let layer = document.layers.pop().ok_or_else(|| error("Missing AI gateway layer"))?;
        if !matches!(layer.content, LayerContent::Raster(_)) || layer.surface().is_none_or(|surface| surface.format() != format) {
            return Err(error("AI gateway must return a native raster layer"));
        }
        let detail_upscaler = match metadata.detail_upscaler.as_deref() {
            None => None,
            Some("realesrgan-x4plus") => Some("realesrgan-x4plus"),
            _ => return Err(error("Unknown AI gateway detail model")),
        };
        if let Some(prompt) = &metadata.prepared_prompt {
            prompt.workflow.validate().map_err(error)?;
            if prompt.original.len() > photocraft_ai::MAX_PROMPT_BYTES
                || prompt.positive.len() > photocraft_ai::MAX_PROMPT_BYTES
                || prompt.negative.as_ref().is_some_and(|p| p.len() > photocraft_ai::MAX_PROMPT_BYTES)
            {
                return Err(error("AI gateway prompt exceeds its budget"));
            }
            if let Some(model) = &prompt.text_model {
                photocraft_ai::openai_prompt::validate_model(model).map_err(error)?;
            }
        }
        let artifact = if operation == Operation::SegmentSubject {
            let mask = layer.mask.as_ref().ok_or_else(|| error("Missing AI foreground mask"))?;
            Artifact::ForegroundMask(scaling::transport_mask(&mask.surface, roi, metadata.working_size)?)
        } else {
            Artifact::Image(RgbaImage::new(metadata.preview_size, rgba.iter().map(|v| f32::from(*v) / 255.0).collect()).map_err(error)?)
        };
        let result = AiResult { provider_job: None, prepared_prompt: metadata.prepared_prompt, artifacts: vec![artifact] };
        let target = Target {
            document: self.doc.id,
            revision: self.revision,
            snapshot: Arc::downgrade(&self.doc),
            original: self.doc,
            incarnation: self.incarnation,
            comparison: std::sync::Mutex::new(None),
            roi,
            operation,
            layers: vec![layer],
            canvas,
            previews: vec![PreviewImage { size: metadata.preview_size, rgba }],
            working_size: metadata.working_size,
            detail_upscaler,
        };
        if target.bytes()?.checked_add(result.bytes().map_err(error)?).is_none_or(|n| n > CACHE_BYTES) {
            return Err(error(AiError::StorageFull));
        }
        Ok(PreparedResult { result, target })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_ai::{PromptPreparation, WorkflowId};
    #[test]
    fn portable_layers_preserve_depth_mask_pixels_identity_and_undo() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16] {
                let mut doc = Document::new("Browser source", photocraft_geom::Size::new(64, 64), mode, depth);
                let mut surface = Surface::new(doc.pixel_format());
                surface.fill_rect(doc.bounds(), if mode == ColorMode::Rgb { &[0.2, 0.4, 0.6, 1.0] } else { &[0.4, 1.0] });
                doc.layers.push(Layer::new("Original", LayerContent::Raster(surface)));
                let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                selection.fill_rect(Rect::from_xywh(12, 12, 40, 40), &[0.5]);
                doc.selection = Some(selection);
                let mut server = Session::new();
                server.add_document(doc.clone(), None);
                let mut browser = Session::new();
                browser.add_document(doc, None);
                let original = browser.active().unwrap().doc.clone();
                let snapshot = Snapshot::capture(browser.active().unwrap(), Source::Composite, 0).unwrap();
                let input = Snapshot::capture(server.active().unwrap(), Source::Composite, 0)
                    .unwrap()
                    .prepare(AiRequest {
                        operation: Operation::Fill,
                        workflow: WorkflowId { name: "test".into(), revision: 1 },
                        prompt: "blue".into(),
                        negative_prompt: None,
                        size: ImageSize { width: 40, height: 40 },
                        variants: 1,
                        seed: 0,
                        source: None,
                        edit_mask: None,
                        references: vec![],
                        prompt_preparation: PromptPreparation::Manual,
                    })
                    .unwrap();
                let ready = input
                    .finish(AiResult {
                        provider_job: None,
                        prepared_prompt: None,
                        artifacts: vec![Artifact::Image(RgbaImage::new(ImageSize { width: 40, height: 40 }, [0.1, 0.2, 0.8, 1.0].repeat(40 * 40)).unwrap())],
                    })
                    .unwrap();
                let server_id = server.retain_ai_result(ready).unwrap();
                let packet = server.export_ai_gateway_result(server_id).unwrap();
                assert!(Arc::ptr_eq(&browser.active().unwrap().doc, &original));
                let local = snapshot.finish_gateway(&packet, Operation::Fill, Rect::from_xywh(12, 12, 40, 40), None).unwrap();
                let browser_id = browser.retain_ai_result(local).unwrap();
                browser.execute("ai.result.apply", json!({"result":browser_id})).unwrap();
                server.execute("ai.result.apply", json!({"result":server_id})).unwrap();
                let left = browser.active().unwrap().doc.layers.last().unwrap();
                let right = server.active().unwrap().doc.layers.last().unwrap();
                assert_ne!(left.id, right.id);
                assert_eq!(left.surface(), right.surface());
                assert_eq!(left.mask, right.mask);
                assert_eq!(browser.active().unwrap().doc.layers.first(), original.layers.first());
                assert!(browser.undo());
                assert_eq!(*browser.active().unwrap().doc, *original);
                assert!(browser.redo());
                let wrong = Snapshot::capture(browser.active().unwrap(), Source::Composite, 0).unwrap();
                assert!(wrong.finish_gateway(&packet, Operation::Generate, Rect::from_xywh(0, 0, 64, 64), None).is_err());
                assert!(load_document(b"not a native bundle").is_err());
            }
        }
    }
}
