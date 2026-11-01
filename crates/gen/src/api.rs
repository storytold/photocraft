//! Explicit API-key image editing. FLUX additionally polls and downloads its signed result.
use std::{future::Future, io::Read, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    EditRequest, EditResult, Error, ImageProvider,
    api_config::{Profile, Protocol},
};

const MAX_IMAGE: usize = 64 * 1024 * 1024;
const MAX_INPUT: usize = 30 * 1024 * 1024;
const MAX_RESPONSE: usize = 90 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_777_216;
mod flux;
mod rejection;

fn error(message: &str) -> Error {
    Error::Provider(message.into())
}

impl Profile {
    pub fn validate(&self) -> Result<(), Error> {
        if self.model.is_empty() || self.model.len() > 200 || !self.model.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
            return Err(error("Enter a model ID using letters, numbers, dots, underscores or hyphens"));
        }
        if self.endpoint.len() > 2048 {
            return Err(error("API address is too long"));
        }
        let url = url::Url::parse(&self.request_url()).map_err(|_| error("Enter a complete HTTPS API address"))?;
        let template = url::Url::parse(&self.endpoint).map_err(|_| error("Model placeholders are allowed only in the API path"))?;
        if template.origin() != url.origin() {
            return Err(error("Model placeholders cannot change the API host"));
        }
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error("API address must use HTTPS without embedded credentials, query or fragment (HTTP is allowed only on loopback)"));
        }
        if matches!(self.protocol, Protocol::GeminiGenerateContent | Protocol::Flux) && !self.endpoint.contains("{model}") {
            return Err(error("The selected API address must contain {model}"));
        }
        Ok(())
    }

    fn request_url(&self) -> String {
        self.endpoint.replace("{model}", &self.model)
    }
}

pub fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| (33..=126).contains(&b)) {
        return Err(error("Enter a valid API key without whitespace"));
    }
    Ok(())
}

pub struct ApiProvider {
    profile: Profile,
    key: String,
    timeout: Duration,
}

impl ApiProvider {
    pub fn new(profile: Profile, key: String) -> Result<Self, Error> {
        profile.validate()?;
        validate_key(&key)?;
        Ok(Self { profile, key, timeout: Duration::from_secs(600) })
    }

    async fn request(&self, body: Value) -> Result<Vec<u8>, Error> {
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .user_agent("Photocraft-dev")
            .build()
            .map_err(|_| error("Could not initialize the image API client"))?;
        let mut auth = reqwest::header::HeaderValue::from_str(&match self.profile.protocol {
            Protocol::Seedream => format!("Bearer {}", self.key),
            _ => self.key.clone(),
        })
        .map_err(|_| error("Invalid API key"))?;
        auth.set_sensitive(true);
        if self.profile.protocol == Protocol::Flux {
            return tokio::time::timeout(self.timeout, flux::edit(self, &client, body, auth)).await.map_err(|_| Error::Timeout)?;
        }
        let header = if self.profile.protocol == Protocol::Seedream { "authorization" } else { "x-goog-api-key" };
        let mut response = client.post(self.profile.request_url()).header(header, auth).json(&body).send().await.map_err(network_error)?;
        if !response.status().is_success() {
            return Err(rejection::http(response, self.profile.protocol).await);
        }
        if response.content_length().is_some_and(|n| n > MAX_RESPONSE as u64) {
            return Err(error("Image API response exceeds 90 MiB"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                return Err(error("Image API response exceeds 90 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| error("Image API returned invalid JSON"))?;
        parse_result(self.profile.protocol, &value)
    }
}

fn network_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout
    } else {
        self::error("Could not reach the image API. Check the address, network and proxy; no automatic retry was made.")
    }
}

fn cancellable<T>(future: impl Future<Output = Result<T, Error>>, cancelled: &dyn Fn() -> bool) -> Result<T, Error> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| error("Could not start image API transport"))?;
    let result = runtime.block_on(async {
        let mut future = std::pin::pin!(future);
        loop {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            if let Ok(result) = tokio::time::timeout(Duration::from_millis(100), future.as_mut()).await {
                return result;
            }
        }
    });
    // Cancelling drops the HTTP future. An OS DNS lookup already in progress cannot be
    // interrupted, so don't let runtime teardown hold the editor waiting for that lookup.
    runtime.shutdown_timeout(Duration::from_millis(100));
    result
}

impl ImageProvider for ApiProvider {
    fn edit(&self, request: &EditRequest<'_>, cancelled: &dyn Fn() -> bool) -> Result<EditResult, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        if request.prompt.trim().is_empty() || request.prompt.len() > 16_000 {
            return Err(error("Enter an editing instruction under 16000 bytes"));
        }
        let mut input = Vec::new();
        std::fs::File::open(request.image)?.take(MAX_INPUT as u64 + 1).read_to_end(&mut input)?;
        if input.len() > MAX_INPUT {
            return Err(error("API input PNG exceeds 30 MiB; use a smaller canvas"));
        }
        let (width, height) = png_size(&input).ok_or_else(|| error("API input must be a PNG under 16 MP"))?;
        let body = request_body(&self.profile, request.prompt, &input, width, height)?;
        let image = cancellable(self.request(body), cancelled)?;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let extension = if image.starts_with(b"\x89PNG\r\n\x1a\n") {
            "png"
        } else if image.starts_with(b"\xff\xd8\xff") {
            "jpg"
        } else {
            return Err(error("Image API must return PNG or JPEG image data"));
        };
        let saved_path = request.workspace.join(format!("api-result.{extension}"));
        std::fs::write(&saved_path, image)?;
        Ok(EditResult { saved_path })
    }
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..8)? != b"\x89PNG\r\n\x1a\n" || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    (width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS).then_some((width, height))
}

fn request_body(profile: &Profile, prompt: &str, input: &[u8], width: u32, height: u32) -> Result<Value, Error> {
    let encoded = STANDARD.encode(input);
    let prompt = format!("{prompt}\nPreserve the full input canvas and its {width}:{height} aspect ratio. Return one edited image.");
    let mut body = match profile.protocol {
        Protocol::Flux => json!({"prompt":prompt,"input_image":encoded,"output_format":"png"}),
        Protocol::GeminiInteractions => json!({
            "model":profile.model,"store":false,"stream":false,
            "input":[{"type":"image","mime_type":"image/png","data":encoded},{"type":"text","text":prompt}],
            "response_format":{"type":"image","mime_type":"image/png"}
        }),
        Protocol::GeminiGenerateContent => json!({
            "contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"image/png","data":encoded}},{"text":prompt}]}],
            "generationConfig":{"responseModalities":["TEXT","IMAGE"]}
        }),
        Protocol::Seedream => {
            // A ~4 MP result fits both the 4.5 minimum and the 5.0 flash/pro maximum.
            // Scale both axes together; the engine fits a same-aspect result back to the canvas.
            let ratio = f64::from(width) / f64::from(height);
            if !(1.0 / 16.0..=16.0).contains(&ratio) {
                return Err(error("Seedream requires a canvas aspect ratio between 1:16 and 16:1"));
            }
            let scale = (4_000_000.0 / (f64::from(width) * f64::from(height))).sqrt();
            let w = (f64::from(width) * scale).round() as u32;
            let h = (f64::from(height) * scale).round() as u32;
            json!({"model":profile.model,"prompt":prompt,"image":format!("data:image/png;base64,{encoded}"),
                // Non-streaming is the API default. Flash/pro do not support the stream
                // option, so omit it rather than sending a field for other model versions.
                "size":format!("{w}x{h}"),"response_format":"b64_json"})
        }
    };
    if let Some(ratio) = standard_aspect(width, height) {
        match profile.protocol {
            Protocol::GeminiInteractions => body["response_format"]["aspect_ratio"] = json!(ratio),
            Protocol::GeminiGenerateContent => body["generationConfig"]["imageConfig"] = json!({"aspectRatio":ratio}),
            Protocol::Seedream | Protocol::Flux => {}
        }
    }
    Ok(body)
}

fn standard_aspect(width: u32, height: u32) -> Option<&'static str> {
    [(1, 1, "1:1"), (2, 3, "2:3"), (3, 2, "3:2"), (3, 4, "3:4"), (4, 3, "4:3"), (4, 5, "4:5"), (5, 4, "5:4"), (9, 16, "9:16"), (16, 9, "16:9"), (21, 9, "21:9")]
        .into_iter()
        .find_map(|(w, h, label)| {
            let a = u64::from(width) * h;
            let b = u64::from(height) * w;
            (a > 0 && b > 0 && a.abs_diff(b).saturating_mul(100) <= a.max(b)).then_some(label)
        })
}

fn decode_image(data: &str) -> Result<Vec<u8>, Error> {
    if data.len() > MAX_IMAGE.saturating_mul(4).div_ceil(3).saturating_add(4) {
        return Err(error("Generated image exceeds 64 MiB"));
    }
    let bytes = STANDARD.decode(data).map_err(|_| error("Image API returned invalid Base64 image data"))?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE {
        return Err(error("Generated image is empty or exceeds 64 MiB"));
    }
    Ok(bytes)
}

fn parse_result(protocol: Protocol, value: &Value) -> Result<Vec<u8>, Error> {
    if value.get("error").is_some_and(|e| !e.is_null()) {
        if protocol == Protocol::Seedream
            && let Some(detail) = rejection::detail(value)
        {
            return Err(error(&format!("Image service rejected the request. {detail}. No layer was added.")));
        }
        return Err(error("Image service rejected the request; check the model and provider console. No layer was added."));
    }
    let images = match protocol {
        Protocol::Flux => return Err(error("FLUX requires its asynchronous result workflow")),
        Protocol::Seedream => seedream_images(value)?,
        Protocol::GeminiGenerateContent => candidate_images(value)?,
        Protocol::GeminiInteractions => interaction_images(value)?,
    };
    let [data] = images.as_slice() else {
        return Err(error(
            "Expected one completed image with inline Base64 data. Check the model and protocol; URL-only and multiple-image responses are not supported.",
        ));
    };
    decode_image(data)
}

fn array<'a>(value: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    value.get(field).and_then(Value::as_array).into_iter().flatten()
}

fn seedream_images(value: &Value) -> Result<Vec<&str>, Error> {
    if array(value, "data").any(|item| item.get("error").is_some_and(|e| !e.is_null())) {
        if let Some(detail) = array(value, "data").find_map(rejection::detail) {
            return Err(error(&format!("Image service reported a failed image. {detail}. No layer was added.")));
        }
        return Err(error("Image service reported a failed image; no layer was added"));
    }
    Ok(array(value, "data").filter_map(|item| item.get("b64_json").and_then(Value::as_str)).collect())
}

fn candidate_images(value: &Value) -> Result<Vec<&str>, Error> {
    if value.pointer("/promptFeedback/blockReason").is_some() {
        return Err(error("Image request was blocked by the provider; no layer was added"));
    }
    Ok(array(value, "candidates")
        .filter(|candidate| candidate.get("finishReason").and_then(Value::as_str).is_none_or(|s| s == "STOP"))
        .flat_map(|candidate| candidate.pointer("/content/parts").and_then(Value::as_array).into_iter().flatten())
        // Thinking images are not the final edited result.
        .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
        .filter_map(|part| part.get("inlineData").or_else(|| part.get("inline_data")).and_then(|v| v.get("data")).and_then(Value::as_str))
        .collect())
}

fn image_data(block: &Value) -> Option<&str> {
    if block.get("type").and_then(Value::as_str) == Some("image") { block.get("data").and_then(Value::as_str) } else { None }
}

fn interaction_images(value: &Value) -> Result<Vec<&str>, Error> {
    if value.get("status").and_then(Value::as_str) != Some("completed") {
        return Err(error("Gemini interaction did not complete; no layer was added"));
    }
    if let Some(data) = value.pointer("/output_image/data").and_then(Value::as_str) {
        return Ok(vec![data]);
    }
    let images: Vec<&str> = array(value, "steps")
        .filter(|step| step.get("type").and_then(Value::as_str) == Some("model_output"))
        .flat_map(|step| array(step, "content"))
        .filter_map(image_data)
        .collect();
    if !images.is_empty() {
        return Ok(images);
    }
    // Earlier Interactions revisions used outputs rather than steps.
    Ok(array(value, "outputs").filter_map(image_data).collect())
}
#[cfg(test)]
mod tests;
