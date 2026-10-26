use super::Http;
use crate::{AiRequest, Control, PreparedPrompt, PromptProvider, ProviderCapabilities, ProviderFuture, Result, openai_prompt};
use reqwest::Method;

pub struct OpenAiPrompt {
    http: Http,
    model: String,
}
impl OpenAiPrompt {
    pub async fn interpret(
        &self,
        source: &crate::RgbaImage,
        instruction: &str,
        language: &str,
        control: &crate::Control,
    ) -> Result<crate::vision::Interpretation> {
        use base64::Engine as _;
        if source.size().width > 1024 || source.size().height > 1024 {
            return Err(crate::AiError::InvalidRequest("image analysis must use a bounded working copy".into()));
        }
        let image = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(super::png(source)?));
        let body = crate::vision::request_body(&self.model, &image, instruction, language)?;
        let response = self.http.send(self.http.request(Method::POST, "responses").json(&body), control).await.map_err(|error| match error {
            crate::AiError::Authentication => crate::AiError::PromptAuthentication,
            crate::AiError::Rejected => crate::AiError::Unsupported(
                "The prompt endpoint/model rejected image analysis. Select a vision-capable model supporting Responses image input and structured output."
                    .into(),
            ),
            error => error,
        })?;
        crate::vision::parse_response(&self.http.bytes(response, 64 * 1024, control).await?)
    }
    pub fn configured(endpoint: &str, key_variable: &str, key: Option<&crate::secret::Secret>, model: &str) -> Result<Self> {
        openai_prompt::validate_model(model)?;
        Ok(Self { http: Http::configured(endpoint, key_variable, key)?, model: model.into() })
    }
    pub fn new(endpoint: &str, key_variable: &str, model: &str) -> Result<Self> {
        openai_prompt::validate_model(model)?;
        Ok(Self { http: Http::new(endpoint, key_variable, false)?, model: model.to_owned() })
    }
}
impl PromptProvider for OpenAiPrompt {
    fn prepare<'a>(&'a self, request: &'a AiRequest, caps: &'a ProviderCapabilities, control: &'a Control) -> ProviderFuture<'a, PreparedPrompt> {
        Box::pin(async move {
            let body = openai_prompt::request_body(&self.model, request, caps)?;
            let response = self.http.send(self.http.request(Method::POST, "responses").json(&body), control).await?;
            let bytes = self.http.bytes(response, 256 * 1024, control).await?;
            openai_prompt::parse_response(&self.model, &bytes, request, caps)
        })
    }
}

/// Direct Images API provider. Original pixels remain protected by the engine's result mask.
pub struct OpenAiImages {
    http: Http,
    model: String,
    quality: String,
}
impl OpenAiImages {
    pub fn configured(endpoint: &str, key_variable: &str, key: Option<&crate::secret::Secret>, model: &str) -> Result<Self> {
        openai_prompt::validate_model(model)?;
        Ok(Self { http: Http::configured(endpoint, key_variable, key)?, model: model.into(), quality: "low".into() })
    }
    pub fn new(endpoint: &str, key_variable: &str, model: &str) -> Result<Self> {
        openai_prompt::validate_model(model)?;
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
    fn generate<'a>(&'a self, request: &'a AiRequest, prompt: Option<&'a PreparedPrompt>, control: &'a Control) -> ProviderFuture<'a, crate::AiResult> {
        Box::pin(async move {
            use crate::{AiError, AiResult, Artifact, Operation, RgbaImage};
            use base64::Engine;
            use serde_json::json;
            self.validate_request(request)?;
            let output_size = crate::image_resolution::openai_size(&self.model, request.size)?;
            let size_parameter = format!("{}x{}", output_size.width, output_size.height);
            let prompt = prompt.ok_or(AiError::InvalidRequest("missing image prompt".into()))?;
            let builder = if request.operation == Operation::Generate {
                self.http
                    .request(Method::POST, "images/generations")
                    .json(&json!({"model":self.model,"prompt":prompt.positive,"size":size_parameter,"quality":self.quality,"output_format":"png","n":1}))
            } else {
                let source = request.source.as_ref().ok_or(AiError::InvalidRequest("missing edit image".into()))?;
                let part = reqwest::multipart::Part::bytes(super::png(source)?)
                    .file_name("image.png")
                    .mime_str("image/png")
                    .map_err(|_| AiError::InvalidImage("PNG upload failed".into()))?;
                let mut form = reqwest::multipart::Form::new()
                    .text("model", self.model.clone())
                    .text("prompt", prompt.positive.clone())
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
            Ok(AiResult { provider_job: None, prepared_prompt: None, artifacts: vec![Artifact::Image(image)] })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AiRequest, ImageProvider, ImageSize, Operation, PromptPreparation, WorkflowId};
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
            prompt_preparation: PromptPreparation::OpenAiText,
        }
    }
    #[test]
    fn anonymous_selfhosted_prompt_is_explicit_and_missing_named_keys_are_actionable() {
        let body=serde_json::to_vec(&json!({"status":"completed","output":[{"type":"message","status":"completed","content":[{"type":"output_text","text":"{\"positive_prompt\":\"a toy astronaut\",\"negative_prompt\":null}"}]}]})).unwrap();
        let (base, server) = super::super::tests::peer(200, body, |wire| {
            assert!(wire.starts_with("POST /proxy/v1/responses "));
            assert!(!wire.to_ascii_lowercase().contains("authorization:"));
        });
        let prompt = OpenAiPrompt::new(&base, "", "qwen3.6:35b@GPU-HOST").unwrap();
        let caps = crate::native::comfy::ComfyUi::new("http://127.0.0.1:8188", "").unwrap().capabilities();
        let result = super::super::block_on(prompt.prepare(&request(), &caps, &Control::default())).unwrap();
        assert_eq!(result.positive, "a toy astronaut");
        server.join().unwrap();
        let variable = format!("PHOTOCRAFT_TEST_MISSING_{}", uuid::Uuid::new_v4().simple());
        let error = OpenAiPrompt::new(&base, &variable, "qwen3.6:35b@GPU-HOST").err().unwrap();
        assert_eq!(error, crate::AiError::MissingCredential(variable.clone()));
        assert!(error.to_string().contains(&variable));
    }
    #[test]
    fn prompt_authentication_is_identified_before_image_submission() {
        let (base, server) =
            super::super::tests::peer(401, b"private-provider-details".to_vec(), |wire| assert!(wire.starts_with("POST /proxy/v1/responses ")));
        let prompt = OpenAiPrompt::new(&base, "", "qwen3.6:35b@GPU-HOST").unwrap();
        let images = crate::native::comfy::ComfyUi::new("http://127.0.0.1:1", "").unwrap();
        let error = super::super::block_on(crate::generate(&request(), &images, Some(&prompt), &Control::default())).unwrap_err();
        assert_eq!(error, crate::AiError::PromptAuthentication);
        assert!(!error.to_string().contains("private-provider-details"));
        server.join().unwrap();
    }
    #[test]
    fn responses_http_uses_strict_text_schema_and_preserves_prompt_provenance() {
        let body=serde_json::to_vec(&json!({"status":"completed","output":[{"type":"message","status":"completed","content":[{"type":"output_text","text":"{\"positive_prompt\":\"a blue ceramic mug\",\"negative_prompt\":null}"}]}]})).unwrap();
        let (base, server) = super::super::tests::peer(200, body, |wire| {
            assert!(wire.starts_with("POST /proxy/v1/responses "));
            let body = wire.split("\r\n\r\n").nth(1).unwrap();
            let value: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(value["store"], false);
            assert_eq!(value["model"], "qwen3.6:35b@GPU-HOST");
            assert_eq!(value["text"]["format"]["strict"], true);
            assert!(!body.contains("image_url"));
        });
        openai_prompt::validate_model("qwen3.6:35b@GPU-HOST").unwrap();
        let prompt = OpenAiPrompt { http: Http::with_key(&base, Some("test-token".into())).unwrap(), model: "qwen3.6:35b@GPU-HOST".into() };
        let request = request();
        let caps = crate::native::comfy::ComfyUi::new("http://127.0.0.1:8188", "").unwrap().capabilities();
        let result = super::super::block_on(prompt.prepare(&request, &caps, &Control::default())).unwrap();
        assert_eq!(result.original, request.prompt);
        assert_eq!(result.positive, "a blue ceramic mug");
        assert_eq!(result.text_model.as_deref(), Some("qwen3.6:35b@GPU-HOST"));
        server.join().unwrap();
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
        request.prompt_preparation = PromptPreparation::Manual;
        let result = super::super::block_on(crate::generate(&request, &provider, None, &Control::default())).unwrap();
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
        request.prompt_preparation = PromptPreparation::Manual;
        let result = super::super::block_on(crate::generate(&request, &provider, None, &Control::default())).unwrap();
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
        request.prompt_preparation = PromptPreparation::Manual;
        request.source = Some(crate::RgbaImage::new(request.size, vec![0.5; 32 * 32 * 4]).unwrap());
        request.edit_mask = Some(crate::Mask::new(request.size, vec![1.0; 32 * 32]).unwrap());
        let result = super::super::block_on(crate::generate(&request, &provider, None, &Control::default())).unwrap();
        assert_eq!(result.artifacts.first().unwrap().size(), request.size);
        server.join().unwrap();
    }
    #[test]
    fn vision_uses_images_and_private_auth_without_generation() {
        let answer = json!({"description":"A blue square", "positive_prompt":"A blue square on a white background", "negative_prompt":null});
        let body = serde_json::to_vec(
            &json!({"status":"completed","output":[{"type":"message","status":"completed","content":[{"type":"output_text","text":answer.to_string()}]}]}),
        )
        .unwrap();
        let (base, server) = super::super::tests::peer(200, body, |wire| {
            assert!(wire.starts_with("POST /proxy/v1/responses "));
            assert!(wire.to_lowercase().contains("authorization: bearer synthetic-vision-key"));
            assert!(wire.contains("data:image/png;base64,"));
            assert!(wire.contains("input_image"));
            assert!(!wire.contains("images/generations"));
            assert!(wire.contains("qwen3.6:35b@GPU-HOST"));
        });
        let key = crate::secret::Secret::new("synthetic-vision-key".into()).unwrap();
        let provider = OpenAiPrompt::configured(&base, "", Some(&key), "qwen3.6:35b@GPU-HOST").unwrap();
        let image = crate::RgbaImage::new(ImageSize { width: 2, height: 2 }, vec![1.0; 16]).unwrap();
        let result = super::super::block_on(provider.interpret(&image, "Recreate composition", "en", &Control::default())).unwrap();
        assert_eq!(result.description, "A blue square");
        server.join().unwrap();
    }
}
