use super::{Http, decode_png, png};
use crate::provider::Stage;
use crate::{
    AiError, AiRequest, AiResult, Artifact, Cancellation, Control, ImageProvider, Mask, MaskMode, Operation, PreparedPrompt, ProviderCapabilities,
    ProviderFuture, Result, RgbaImage, WorkflowId,
    workflow::{ApiNode, ModelBinding, WorkflowPackage},
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

pub struct ComfyUi {
    http: Http,
    sampling_steps: Option<u32>,
}
impl ComfyUi {
    pub fn new(endpoint: &str, key_variable: &str) -> Result<Self> {
        Ok(Self { http: Http::new(endpoint, key_variable, false)?, sampling_steps: None })
    }
    pub fn configured(endpoint: &str, key_variable: &str, key: Option<&crate::secret::Secret>) -> Result<Self> {
        Ok(Self { http: Http::configured(endpoint, key_variable, key)?, sampling_steps: None })
    }
    pub async fn dependency_nodes(&self, control: &Control) -> Result<Value> {
        let info = self.http.json(self.http.request(Method::GET, "object_info"), 8 * 1024 * 1024, control).await?;
        let mut nodes = BTreeSet::new();
        for source in [include_str!("t2i.json"), include_str!("edit.json"), include_str!("maskedit.json"), include_str!("foreground.json")] {
            let graph: BTreeMap<String, ApiNode> = serde_json::from_str(source).map_err(|_| AiError::Rejected)?;
            nodes.extend(graph.into_values().map(|n| n.class_type));
        }
        nodes.extend(["UpscaleModelLoader", "ImageUpscaleWithModel", "VAEDecodeTiled", "EmptySD3LatentImage"].map(str::to_owned));
        let missing: Vec<_> = nodes.iter().filter(|node| info.get(*node).is_none()).cloned().collect();
        Ok(
            json!({"required":nodes,"missing":missing,"note":"Missing core nodes require a compatible ComfyUI update. The model downloader does not install code or restart ComfyUI."}),
        )
    }
    pub fn set_sampling_steps(&mut self, steps: u32) -> Result<()> {
        if !(1..=100).contains(&steps) {
            return Err(AiError::InvalidRequest("sampling steps must be between 1 and 100".into()));
        }
        self.sampling_steps = Some(steps);
        Ok(())
    }
    pub async fn upscale_preflight(&self, control: &Control) -> Result<()> {
        let info = self.http.json(self.http.request(Method::GET, "object_info"), 8 * 1024 * 1024, control).await?;
        upscale_package("photocraft-preflight", "input.png")?.validate_server(&info)
    }
    /// One bounded x4 patch. The engine assembles overlapping patches at native resolution.
    pub async fn upscale(&self, source: &RgbaImage, control: &Control) -> Result<RgbaImage> {
        let output = crate::ImageSize {
            width: source.size().width.checked_mul(4).ok_or(AiError::StorageFull)?,
            height: source.size().height.checked_mul(4).ok_or(AiError::StorageFull)?,
        };
        output.pixels()?;
        if source.bytes() > 32 * 1024 * 1024 {
            return Err(AiError::StorageFull);
        }
        self.upscale_preflight(control).await?;
        let filename = self.upload(source, control).await?;
        let prefix = format!("photocraft-{}", uuid::Uuid::new_v4());
        let graph = upscale_package(&prefix, &filename)?;
        control.check()?;
        let response = self
            .http
            .json(
                self.http.request(Method::POST, "prompt").json(&json!({"prompt":graph.graph,"client_id":uuid::Uuid::new_v4().to_string()})),
                256 * 1024,
                control,
            )
            .await
            .map_err(|e| match e {
                AiError::Unavailable | AiError::Timeout | AiError::Cancelled => AiError::OutcomeUnknown,
                other => other,
            })?;
        let id = response.get("prompt_id").and_then(Value::as_str).filter(|id| uuid::Uuid::parse_str(id).is_ok()).ok_or(AiError::OutcomeUnknown)?;
        let request = AiRequest {
            operation: Operation::Edit,
            workflow: WorkflowId { name: "realesrgan-x4plus".into(), revision: 1 },
            prompt: "detail upscale".into(),
            negative_prompt: None,
            size: output,
            variants: 1,
            seed: 0,
            source: Some(source.clone()),
            edit_mask: None,
            references: vec![],
            prompt_preparation: crate::PromptPreparation::Manual,
        };
        let result = self.wait(&request, id, &prefix, &graph.outputs, control).await;
        if result.is_err() {
            self.cancel_own(id).await;
        }
        match result?.artifacts.into_iter().next() {
            Some(Artifact::Image(image)) => Ok(image),
            _ => Err(AiError::InvalidImage("missing upscale output".into())),
        }
    }
    /// Validate nodes and model dropdowns before any upload or paid prompt stage.
    pub async fn preflight(&self, request: &AiRequest, control: &Control) -> Result<()> {
        request.validate(&self.capabilities())?;
        validate_request(request)?;
        let info = self.http.json(self.http.request(Method::GET, "object_info"), 8 * 1024 * 1024, control).await?;
        package(request.operation)?.validate_server(&info)
    }
    async fn upload(&self, image: &RgbaImage, control: &Control) -> Result<String> {
        let name = format!("photocraft-{}.png", uuid::Uuid::new_v4());
        let part = reqwest::multipart::Part::bytes(png(image)?)
            .file_name(name)
            .mime_str("image/png")
            .map_err(|_| AiError::InvalidImage("invalid PNG upload".into()))?;
        let form = reqwest::multipart::Form::new().part("image", part).text("type", "input").text("overwrite", "false");
        let result = self.http.json(self.http.request(Method::POST, "upload/image").multipart(form), 64 * 1024, control).await?;
        let name = result.get("name").and_then(Value::as_str).ok_or(AiError::Rejected)?;
        if !safe_filename(name) || result.get("subfolder").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
            return Err(AiError::Rejected);
        }
        Ok(name.to_owned())
    }
    async fn cancel_own(&self, id: &str) {
        // Never call /interrupt or clear the queue: other clients may be rendering.
        let _ = self
            .http
            .json(
                self.http.request(Method::POST, &format!("api/jobs/{id}/cancel")).timeout(Duration::from_secs(5)).json(&json!({})),
                64 * 1024,
                &Control::default(),
            )
            .await;
    }
    async fn render(&self, request: &AiRequest, prompt: Option<&PreparedPrompt>, control: &Control) -> Result<AiResult> {
        request.validate(&self.capabilities())?;
        validate_request(request)?;
        let mut package = package(request.operation)?;
        let prefix = format!("photocraft-{}", uuid::Uuid::new_v4());
        for node in package.graph.values_mut() {
            if node.class_type == "SaveImage" {
                node.inputs.insert("filename_prefix".into(), json!(prefix));
            }
        }
        if let Some(prompt) = prompt {
            set(&mut package, "5", "prompt", json!(prompt.positive))?;
            set(&mut package, "5", "negative_prompt", json!(prompt.negative.as_deref().unwrap_or("")))?;
            set(&mut package, "5", "resolution", json!(0))?;
            set(&mut package, "7", "seed", json!(request.seed))?;
        }
        if let Some(source) = &request.source {
            let name = self.upload(source, control).await?;
            set(&mut package, if request.operation == Operation::SegmentSubject { "1" } else { "10" }, "image", json!(name))?;
        }
        if let Some(mask) = &request.edit_mask {
            let samples = mask.values().iter().flat_map(|v| [*v, *v, *v, 1.0]).collect();
            let name = self.upload(&RgbaImage::new(mask.size(), samples)?, control).await?;
            set(&mut package, "11", "image", json!(name))?;
        }
        if request.operation == Operation::Generate {
            set(&mut package, "12", "width", json!(request.size.width.div_ceil(32) * 32))?;
            set(&mut package, "12", "height", json!(request.size.height.div_ceil(32) * 32))?;
        }
        if request.operation != Operation::SegmentSubject {
            set(&mut package, "7", "steps", json!(self.sampling_steps.unwrap_or(if request.operation == Operation::Generate { 25 } else { 8 })))?;
        }
        control.check()?;
        let response = self
            .http
            .json(
                self.http.request(Method::POST, "prompt").json(&json!({"prompt":package.graph,"client_id":uuid::Uuid::new_v4().to_string()})),
                256 * 1024,
                control,
            )
            .await
            .map_err(|e| match e {
                AiError::Unavailable | AiError::Timeout | AiError::Cancelled => AiError::OutcomeUnknown,
                other => other,
            })?;
        let id = response.get("prompt_id").and_then(Value::as_str).filter(|id| uuid::Uuid::parse_str(id).is_ok()).ok_or(AiError::OutcomeUnknown)?.to_owned();
        let result = self.wait(request, &id, &prefix, &package.outputs, control).await;
        if result.is_err() {
            self.cancel_own(&id).await;
        }
        result
    }
    async fn wait(&self, request: &AiRequest, id: &str, prefix: &str, outputs: &BTreeSet<String>, control: &Control) -> Result<AiResult> {
        let started = Instant::now();
        loop {
            control.check()?;
            if started.elapsed() > Duration::from_secs(if request.operation == Operation::Generate { 3600 } else { 900 }) {
                return Err(AiError::Timeout);
            }
            let history = self.http.json(self.http.request(Method::GET, &format!("history/{id}")), 2 * 1024 * 1024, control).await?;
            if let Some(entry) = history.get(id) {
                if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                    if entry.pointer("/status/messages").and_then(Value::as_array).is_some_and(|messages| {
                        messages.iter().any(|message| {
                            message
                                .get(1)
                                .and_then(|data| data.get("exception_message"))
                                .and_then(Value::as_str)
                                .is_some_and(|text| text.contains("VRAM grow failed") || text.to_ascii_lowercase().contains("out of memory"))
                        })
                    }) {
                        return Err(AiError::Unsupported(
                            "ComfyUI GPU memory is insufficient for this resolution; choose a smaller direct size or use AI Upscale separately".into(),
                        ));
                    }
                    return Err(AiError::Rejected);
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    let mut artifacts = Vec::new();
                    for output in outputs {
                        let images = entry
                            .get("outputs")
                            .and_then(|v| v.get(output))
                            .and_then(|v| v.get("images"))
                            .and_then(Value::as_array)
                            .ok_or(AiError::Rejected)?;
                        if images.len() != 1 {
                            return Err(AiError::Rejected);
                        }
                        let metadata = images.first().ok_or(AiError::Rejected)?;
                        let filename =
                            metadata.get("filename").and_then(Value::as_str).filter(|f| safe_filename(f) && f.starts_with(prefix)).ok_or(AiError::Rejected)?;
                        let subfolder = metadata.get("subfolder").and_then(Value::as_str).unwrap_or("");
                        if !subfolder.is_empty() || metadata.get("type").and_then(Value::as_str) != Some("output") {
                            return Err(AiError::Rejected);
                        }
                        let response = self
                            .http
                            .send(
                                self.http.request(Method::GET, "view").query(&[("filename", filename), ("subfolder", subfolder), ("type", "output")]),
                                control,
                            )
                            .await?;
                        let bytes = self.http.bytes(response, 256 * 1024 * 1024, control).await?;
                        let image = decode_png(&bytes, request.size, request.workflow.name != "realesrgan-x4plus")?;
                        artifacts.push(if request.operation == Operation::SegmentSubject {
                            Artifact::ForegroundMask(Mask::new(
                                request.size,
                                image.samples().chunks_exact(4).map(|p| p.first().copied().unwrap_or(0.0)).collect(),
                            )?)
                        } else {
                            Artifact::Image(image)
                        });
                    }
                    let result = AiResult { provider_job: Some(id.to_owned()), prepared_prompt: None, artifacts };
                    result.validate(request)?;
                    return Ok(result);
                }
            }
            control.report(Stage::Generating, (0.3 + started.elapsed().as_secs_f32() / 1200.0).min(0.9));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
impl ImageProvider for ComfyUi {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            operations: vec![Operation::Generate, Operation::Edit, Operation::Fill, Operation::Remove, Operation::Expand, Operation::SegmentSubject],
            mask_mode: MaskMode::LatentWithComposite,
            max_references: 0,
            max_variants: 1,
            max_pixels: crate::MAX_PIXELS,
            max_input_bytes: 32 * 1024 * 1024,
            negative_prompt: true,
            cancellation: Cancellation::OwnJob,
        }
    }
    fn generate<'a>(&'a self, request: &'a AiRequest, prompt: Option<&'a PreparedPrompt>, control: &'a Control) -> ProviderFuture<'a, AiResult> {
        Box::pin(self.render(request, prompt, control))
    }
}
fn upscale_package(prefix: &str, image: &str) -> Result<WorkflowPackage> {
    let graph = BTreeMap::from([
        ("1".into(), ApiNode { class_type: "LoadImage".into(), inputs: BTreeMap::from([("image".into(), json!(image))]) }),
        ("2".into(), ApiNode { class_type: "UpscaleModelLoader".into(), inputs: BTreeMap::from([("model_name".into(), json!("RealESRGAN_x4plus.pth"))]) }),
        (
            "3".into(),
            ApiNode {
                class_type: "ImageUpscaleWithModel".into(),
                inputs: BTreeMap::from([("upscale_model".into(), json!(["2", 0])), ("image".into(), json!(["1", 0]))]),
            },
        ),
        (
            "4".into(),
            ApiNode { class_type: "SaveImage".into(), inputs: BTreeMap::from([("images".into(), json!(["3", 0])), ("filename_prefix".into(), json!(prefix))]) },
        ),
    ]);
    let package = WorkflowPackage {
        schema_version: 1,
        id: WorkflowId { name: "realesrgan-x4plus".into(), revision: 1 },
        required_nodes: graph.values().map(|n| n.class_type.clone()).collect(),
        graph,
        models: vec![ModelBinding { node: "2".into(), input: "model_name".into(), filename: "RealESRGAN_x4plus.pth".into() }],
        bindings: vec![],
        outputs: BTreeSet::from(["4".into()]),
    };
    package.validate()?;
    Ok(package)
}
fn safe_filename(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && !name.contains(['/', '\\']) && !name.contains("..") && !name.chars().any(char::is_control)
}
fn validate_request(request: &AiRequest) -> Result<()> {
    if request.source.as_ref().is_some_and(|source| source.size() != request.size) {
        return Err(AiError::InvalidRequest("source must match the working area".into()));
    }
    let expected = if request.operation == Operation::SegmentSubject { "birefnet" } else { "qwen-image21" };
    if request.workflow.name != expected || request.workflow.revision != 1 {
        return Err(AiError::InvalidWorkflow("unsupported built-in workflow".into()));
    }
    let maximum = if request.operation == Operation::Generate { 8192 } else { 1024 };
    if request.size.width < 32 || request.size.height < 32 || request.size.width > maximum || request.size.height > maximum {
        return Err(AiError::Unsupported(format!("workflow supports 32–{maximum} pixels per side")));
    }
    Ok(())
}
fn set(package: &mut WorkflowPackage, node: &str, input: &str, value: Value) -> Result<()> {
    let slot = package.graph.get_mut(node).and_then(|n| n.inputs.get_mut(input)).ok_or_else(|| AiError::InvalidWorkflow("missing built-in binding".into()))?;
    *slot = value;
    Ok(())
}
fn package(operation: Operation) -> Result<WorkflowPackage> {
    let text = match operation {
        Operation::Generate => include_str!("t2i.json"),
        Operation::Edit => include_str!("edit.json"),
        Operation::Fill | Operation::Remove | Operation::Expand => include_str!("maskedit.json"),
        Operation::SegmentSubject => include_str!("foreground.json"),
        Operation::Upscale => return Err(AiError::Unsupported("use the dedicated upscale workflow".into())),
    };
    let mut graph: BTreeMap<String, ApiNode> = serde_json::from_str(text).map_err(|_| AiError::InvalidWorkflow("invalid built-in graph".into()))?;
    if matches!(operation, Operation::Fill | Operation::Remove | Operation::Expand) {
        // Return raw generated pixels; the engine applies the soft edit mask exactly once.
        graph.remove("14");
        let node = graph.get_mut("9").ok_or_else(|| AiError::InvalidWorkflow("missing output".into()))?;
        node.inputs.insert("images".into(), json!(["8", 0]));
    }
    if operation == Operation::Generate {
        graph.insert(
            "8".into(),
            ApiNode {
                class_type: "VAEDecodeTiled".into(),
                inputs: BTreeMap::from([
                    ("samples".into(), json!(["7", 0])),
                    ("vae".into(), json!(["3", 0])),
                    ("tile_size".into(), json!(512)),
                    ("overlap".into(), json!(64)),
                    ("temporal_size".into(), json!(64)),
                    ("temporal_overlap".into(), json!(8)),
                ]),
            },
        );
        graph.insert(
            "12".into(),
            ApiNode {
                class_type: "EmptySD3LatentImage".into(),
                inputs: BTreeMap::from([("width".into(), json!(512)), ("height".into(), json!(512)), ("batch_size".into(), json!(1))]),
            },
        );
        graph.get_mut("7").ok_or_else(|| AiError::InvalidWorkflow("missing sampler".into()))?.inputs.insert("latent_image".into(), json!(["12", 0]));
    }
    let required_nodes = graph.values().map(|n| n.class_type.clone()).collect();
    let models = graph
        .iter()
        .flat_map(|(id, n)| {
            n.inputs
                .iter()
                .filter(|(key, _)| ["unet_name", "clip_name", "vae_name", "bg_removal_name"].contains(&key.as_str()))
                .filter_map(move |(key, v)| v.as_str().map(|filename| ModelBinding { node: id.clone(), input: key.clone(), filename: filename.to_owned() }))
        })
        .collect();
    let package = WorkflowPackage {
        schema_version: 1,
        id: WorkflowId { name: if operation == Operation::SegmentSubject { "birefnet" } else { "qwen-image21" }.into(), revision: 1 },
        graph,
        required_nodes,
        models,
        bindings: vec![],
        outputs: BTreeSet::from([if operation == Operation::SegmentSubject { "5" } else { "9" }.into()]),
    };
    package.validate()?;
    Ok(package)
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_generation_graph_decodes_in_tiles_without_an_upscale_node() {
        let graph = super::package(crate::Operation::Generate).unwrap();
        assert_eq!(graph.graph["8"].class_type, "VAEDecodeTiled");
        assert_eq!(graph.graph["12"].class_type, "EmptySD3LatentImage");
        assert!(!graph.graph.values().any(|node| node.class_type.contains("Upscale")));
        let mut backend = super::ComfyUi::new("http://127.0.0.1:1", "").unwrap();
        backend.set_sampling_steps(25).unwrap();
        assert!(backend.set_sampling_steps(0).is_err());
        assert!(backend.set_sampling_steps(101).is_err());
        assert_eq!(crate::ImageSize { width: 7680, height: 4320 }.pixels().unwrap(), 33_177_600);
    }
    #[test]
    fn detail_profile_requires_the_explicit_model_and_bounded_graph() {
        let graph = super::upscale_package("photocraft-own-prefix", "input.png").unwrap();
        graph.validate().unwrap();
        assert_eq!(graph.models[0].filename, "RealESRGAN_x4plus.pth");
        assert_eq!(graph.graph.len(), 4);
        assert!(graph.validate_server(&serde_json::json!({})).is_err());
    }
    use super::*;
    #[test]
    fn masked_workflows_return_raw_pixels_and_manifest_is_complete() {
        for op in [Operation::Generate, Operation::Edit, Operation::Fill, Operation::Remove, Operation::Expand, Operation::SegmentSubject] {
            let p = package(op).unwrap();
            p.validate().unwrap();
            if matches!(op, Operation::Fill | Operation::Remove | Operation::Expand) {
                assert!(!p.graph.contains_key("14"));
                assert_eq!(p.graph.get("9").unwrap().inputs.get("images"), Some(&json!(["8", 0])));
            }
        }
    }
    #[test]
    fn dropdown_models_support_classic_and_v3_combo_nodes() {
        let p = package(Operation::SegmentSubject).unwrap();
        let mut info = json!({"LoadImage":{},"RemoveBackground":{},"MaskToImage":{},"SaveImage":{},"LoadBackgroundRemovalModel":{"input":{"required":{"bg_removal_name":["COMBO",{"options":["birefnet.safetensors"]}]}}}});
        p.validate_server(&info).unwrap();
        info["LoadBackgroundRemovalModel"]["input"]["required"]["bg_removal_name"] = json!([["birefnet.safetensors"]]);
        p.validate_server(&info).unwrap();
        info["LoadBackgroundRemovalModel"]["input"]["required"]["bg_removal_name"] = json!([["other.safetensors"]]);
        assert!(p.validate_server(&info).is_err());
    }
    #[test]
    fn remote_metadata_never_becomes_a_path_or_url() {
        for name in ["../other.png", "/tmp/other.png", "other\\secret", "http://other/image", "bad\nname", ""] {
            assert!(!safe_filename(name));
        }
        assert!(safe_filename("photocraft-uuid_00001_.png"));
    }
}
