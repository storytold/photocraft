use super::Http;
use crate::{AiError, AiRequest, Control, ProviderCapabilities, ProviderFuture, Result};
use reqwest::Method;

/// Model IDs are opaque JSON values, including proxy routing/tag syntax such as
/// `qwen3.6:35b@GPU-HOST` or `organization/model`. Never interpret them as paths.
pub fn validate_model(model: &str) -> Result<()> {
    if model.is_empty() || model.len() > 128 || model.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AiError::InvalidRequest("invalid model id".into()));
    }
    Ok(())
}

/// What an empty Generative Remove instruction asks for.
const REMOVE_PROMPT: &str = "Remove the marked object and reconstruct the surrounding background.";

/// Direct Images API provider. Original pixels remain protected by the engine's result mask.
pub struct OpenAiImages {
    http: Http,
    model: String,
    quality: String,
}
impl OpenAiImages {
    pub fn configured(endpoint: &str, key_variable: &str, key: Option<&crate::secret::Secret>, model: &str) -> Result<Self> {
        validate_model(model)?;
        Ok(Self { http: Http::configured(endpoint, key_variable, key)?, model: model.into(), quality: "low".into() })
    }
    pub fn new(endpoint: &str, key_variable: &str, model: &str) -> Result<Self> {
        validate_model(model)?;
        Ok(Self { http: Http::new(endpoint, key_variable, false)?, model: model.to_owned(), quality: "low".into() })
    }
    pub fn set_quality(&mut self, quality: &str) -> Result<()> {
        if !matches!(quality, "low" | "medium" | "high" | "xhigh" | "max" | "auto") {
            return Err(crate::AiError::InvalidRequest("unknown image quality".into()));
        }
        if matches!(quality, "xhigh" | "max") && !self.model.starts_with("gpt-image-2.5-") {
            return Err(crate::AiError::Unsupported("xhigh/max quality requires GPT Image 2.5".into()));
        }
        self.quality = quality.into();
        Ok(())
    }
    pub fn validate_request(&self, request: &AiRequest) -> Result<()> {
        use crate::ImageProvider;
        request.validate(&self.capabilities())?;
        if request.workflow.name != "openai-images" || request.workflow.revision != 1 {
            return Err(crate::AiError::Unsupported("unknown direct Images profile".into()));
        }
        crate::image_resolution::openai_size(&self.model, request.size)?;
        Ok(())
    }
}
impl crate::ImageProvider for OpenAiImages {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            operations: vec![crate::Operation::Generate, crate::Operation::Edit, crate::Operation::Fill, crate::Operation::Remove],
            mask_mode: crate::MaskMode::Native,
            max_references: 0,
            max_variants: 1,
            max_pixels: crate::image_resolution::OPENAI_MAX_PIXELS,
            max_input_bytes: crate::MAX_INPUT_BYTES,
            negative_prompt: false,
            cancellation: crate::Cancellation::LocalOnly,
        }
    }
    fn generate<'a>(&'a self, request: &'a AiRequest, control: &'a Control) -> ProviderFuture<'a, crate::AiResult> {
        Box::pin(async move {
            use crate::{AiError, AiResult, Artifact, Operation, RgbaImage};
            use base64::Engine;
            use serde_json::json;
            self.validate_request(request)?;
            let output_size = crate::image_resolution::openai_size(&self.model, request.size)?;
            let size_parameter = format!("{}x{}", output_size.width, output_size.height);
            let prompt = if request.operation == Operation::Remove && request.prompt.trim().is_empty() { REMOVE_PROMPT } else { request.prompt.as_str() };
            let builder = if request.operation == Operation::Generate {
                self.http
                    .request(Method::POST, "images/generations")
                    .json(&json!({"model":self.model,"prompt":prompt,"size":size_parameter,"quality":self.quality,"output_format":"png","n":1}))
            } else {
                let source = request.source.as_ref().ok_or(AiError::InvalidRequest("missing edit image".into()))?;
                let part = reqwest::multipart::Part::bytes(super::png(source)?)
                    .file_name("image.png")
                    .mime_str("image/png")
                    .map_err(|_| AiError::InvalidImage("PNG upload failed".into()))?;
                let mut form = reqwest::multipart::Form::new()
                    .text("model", self.model.clone())
                    .text("prompt", prompt.to_owned())
                    .text("size", size_parameter)
                    .text("quality", self.quality.clone())
                    .text("output_format", "png")
                    .text("n", "1")
                    .part("image[]", part);
                if let Some(mask) = &request.edit_mask {
                    let mask = RgbaImage::new(mask.size(), mask.values().iter().flat_map(|v| [0.0, 0.0, 0.0, if *v > 0.0 { 0.0 } else { 1.0 }]).collect())?;
                    let part = reqwest::multipart::Part::bytes(super::png(&mask)?)
                        .file_name("mask.png")
                        .mime_str("image/png")
                        .map_err(|_| AiError::InvalidImage("PNG mask upload failed".into()))?;
                    form = form.part("mask", part);
                }
                self.http.request(Method::POST, "images/edits").multipart(form)
            };
            let body = self.http.json(builder.timeout(std::time::Duration::from_secs(600)), 128 * 1024 * 1024, control).await.map_err(|e| match e {
                AiError::Unavailable | AiError::Timeout => AiError::OutcomeUnknown,
                other => other,
            })?;
            let data = body.get("data").and_then(serde_json::Value::as_array).filter(|v| v.len() == 1).ok_or(AiError::Rejected)?;
            let encoded = data
                .first()
                .and_then(|v| v.get("b64_json"))
                .and_then(serde_json::Value::as_str)
                .filter(|v| v.len() <= 96 * 1024 * 1024)
                .ok_or(AiError::Rejected)?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| AiError::Rejected)?;
            let mut image = super::decode_png(&bytes, output_size, false)?;
            if request.size != image.size() {
                let rgba: Vec<u8> = image.samples().iter().map(|v| (v * 255.0).round() as u8).collect();
                let raw =
                    image::RgbaImage::from_raw(output_size.width, output_size.height, rgba).ok_or(AiError::InvalidImage("invalid image output".into()))?;
                let resized = image::imageops::resize(&raw, request.size.width, request.size.height, image::imageops::FilterType::Triangle);
                image = RgbaImage::new(request.size, resized.into_raw().into_iter().map(|v| f32::from(v) / 255.0).collect())?;
            }
            Ok(AiResult { provider_job: None, artifacts: vec![Artifact::Image(image)] })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AiRequest, ImageSize, Operation, WorkflowId};
    use serde_json::json;
    fn request() -> AiRequest {
        AiRequest {
            operation: Operation::Generate,
            workflow: WorkflowId { name: "qwen-image21".into(), revision: 1 },
            prompt: "eine blaue Tasse".into(),
            negative_prompt: None,
            size: ImageSize { width: 32, height: 32 },
            variants: 1,
            seed: 0,
            source: None,
            edit_mask: None,
            references: vec![],
        }
    }
    #[test]
    fn model_ids_preserve_self_hosted_tags_and_routes() {
        for id in ["gpt-image-2.5-sunburst", "qwen3.6:35b@GPU-HOST", "organization/model"] {
            validate_model(id).unwrap();
        }
        for id in ["", "has space", "line\nbreak"] {
            assert!(validate_model(id).is_err());
        }
    }
    #[test]
    fn images_http_requests_png_and_resizes_square_output_to_exact_roi() {
        use base64::Engine;
        let image = crate::RgbaImage::new(ImageSize { width: 1024, height: 1024 }, vec![1.0; 1024 * 1024 * 4]).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(super::super::png(&image).unwrap());
        let body = serde_json::to_vec(&json!({"data":[{"b64_json":encoded}]})).unwrap();
        let (base, server) = super::super::tests::peer(200, body, |wire| {
            assert!(wire.starts_with("POST /proxy/v1/images/generations "));
            let body: serde_json::Value = serde_json::from_str(wire.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body["output_format"], "png");
            assert_eq!(body["n"], 1);
        });
        let provider =
            OpenAiImages { http: Http::with_key(&base, Some("test-token".into())).unwrap(), model: "test-image-model".into(), quality: "low".into() };
        let mut request = request();
        request.workflow.name = "openai-images".into();
        let result = super::super::block_on(crate::generate(&request, &provider, &Control::default())).unwrap();
        assert_eq!(result.artifacts.first().unwrap().size(), request.size);
        server.join().unwrap();
    }
    #[test]
    fn images_http_sends_native_four_k_landscape_and_quality() {
        use base64::Engine;
        let size = ImageSize { width: 3840, height: 2160 };
        let image = crate::RgbaImage::new(size, vec![1.0; size.pixels().unwrap() * 4]).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(super::super::png(&image).unwrap());
        drop(image);
        let body = serde_json::to_vec(&json!({"data":[{"b64_json":encoded}]})).unwrap();
        let (base, server) = super::super::tests::peer(200, body, |wire| {
            let body: serde_json::Value = serde_json::from_str(wire.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body["size"], "3840x2160");
            assert_eq!(body["quality"], "high");
        });
        let mut provider =
            OpenAiImages { http: Http::with_key(&base, Some("test-token".into())).unwrap(), model: "gpt-image-2.5-sunburst".into(), quality: "low".into() };
        provider.set_quality("high").unwrap();
        let mut request = request();
        request.workflow.name = "openai-images".into();
        request.size = size;
        let result = super::super::block_on(crate::generate(&request, &provider, &Control::default())).unwrap();
        assert_eq!(result.artifacts.first().unwrap().size(), size);
        assert!(provider.set_quality("unknown").is_err());
        server.join().unwrap();
    }
    #[test]
    fn images_edit_uploads_multipart_source_and_alpha_mask() {
        use base64::Engine;
        let output = crate::RgbaImage::new(ImageSize { width: 1024, height: 1024 }, vec![1.0; 1024 * 1024 * 4]).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(super::super::png(&output).unwrap());
        let body = serde_json::to_vec(&json!({"data":[{"b64_json":encoded}]})).unwrap();
        let (base, server) = super::super::tests::peer(200, body, |wire| {
            assert!(wire.starts_with("POST /proxy/v1/images/edits "));
            assert!(wire.contains("multipart/form-data; boundary="));
            assert!(wire.contains("name=\"image[]\"; filename=\"image.png\""));
            assert!(wire.contains("name=\"mask\"; filename=\"mask.png\""));
            assert!(wire.contains("name=\"model\""));
            assert!(wire.contains("name=\"output_format\""));
        });
        let provider =
            OpenAiImages { http: Http::with_key(&base, Some("test-token".into())).unwrap(), model: "test-image-model".into(), quality: "low".into() };
        let mut request = request();
        request.workflow.name = "openai-images".into();
        request.operation = Operation::Fill;
        request.source = Some(crate::RgbaImage::new(request.size, vec![0.5; 32 * 32 * 4]).unwrap());
        request.edit_mask = Some(crate::Mask::new(request.size, vec![1.0; 32 * 32]).unwrap());
        let result = super::super::block_on(crate::generate(&request, &provider, &Control::default())).unwrap();
        assert_eq!(result.artifacts.first().unwrap().size(), request.size);
        server.join().unwrap();
    }
}
