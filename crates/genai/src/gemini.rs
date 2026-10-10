//! Google Gemini image models ("Nano Banana") through the Gemini API's Interactions endpoint,
//! with the user's own API key.
//!
//! Gemini edits from instructions and reference images; it takes no mask parameter. Generative
//! Fill sends the mask as a second image and says what it means; the engine then keeps only the
//! masked pixels of the result, so anything the model changes outside the mask is discarded.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::{
    ApiKey, Capabilities, EditRequest, GenError, GenerateRequest, GeneratedImage, HttpRequest, HttpTransport, ImageProvider, ImageSize, ProviderInfo, Result,
    VariationRequest,
};

pub const ID: &str = "gemini";
pub const NAME: &str = "Google Gemini";
pub const HOST: &str = "generativelanguage.googleapis.com";
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";

/// Image models offered in settings, best default first: (id, description).
pub const MODELS: &[(&str, &str)] = &[
    ("gemini-nano-banana-2.1", "Nano Banana 2.1"),
    ("gemini-3-pro-image", "Nano Banana Pro"),
    ("gemini-3.1-flash-image", "Nano Banana 2"),
    ("gemini-3.1-flash-lite-image", "Nano Banana 2 Lite (fastest, 1K only)"),
];
pub const DEFAULT_MODEL: &str = "gemini-nano-banana-2.1";

/// Output aspect ratios the image models accept.
const RATIOS: &[(u32, u32)] = &[(1, 1), (3, 2), (2, 3), (3, 4), (4, 3), (4, 5), (5, 4), (9, 16), (16, 9), (21, 9)];

pub struct GeminiProvider<T: HttpTransport> {
    key: ApiKey,
    model: String,
    transport: T,
}

impl<T: HttpTransport> GeminiProvider<T> {
    /// `model` must look like a Gemini model id (letters, digits, `-`, `.`); it is not checked
    /// against [`MODELS`] so a newer model works without a PhotoCraft update.
    pub fn new(key: ApiKey, model: &str, transport: T) -> Result<Self> {
        let model = model.trim();
        let model = if model.is_empty() { DEFAULT_MODEL } else { model };
        if model.len() > 80 || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
            return Err(GenError::Invalid(format!("`{model}` is not a Gemini model id")));
        }
        Ok(Self { key, model: model.to_owned(), transport })
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn size_token(&self, size: ImageSize) -> &'static str {
        if self.model.contains("lite") {
            return "1K";
        }
        match size {
            ImageSize::Small => "1K",
            ImageSize::Medium => "2K",
            ImageSize::Large => "4K",
        }
    }

    fn request(&self, input: Vec<Value>, aspect: (u32, u32), size: ImageSize) -> Result<GeneratedImage> {
        let (a, b) = crate::nearest_ratio(aspect.0, aspect.1, RATIOS).unwrap_or((1, 1));
        let body = json!({
            "model": self.model,
            "input": input,
            "response_format": {
                "type": "image",
                // The Interactions API returns JPEG only ("Supported values: 'image/jpeg'").
                "mime_type": "image/jpeg",
                "aspect_ratio": format!("{a}:{b}"),
                "image_size": self.size_token(size),
            },
        });
        let body = serde_json::to_vec(&body).map_err(|_| GenError::Invalid("could not encode the request".into()))?;
        let response = self.transport.post(HttpRequest {
            url: ENDPOINT.into(),
            headers: vec![("x-goog-api-key", self.key.expose().to_owned()), ("content-type", "application/json".into())],
            body,
        })?;
        if response.body.len() > crate::MAX_RESPONSE {
            return Err(GenError::BadResponse("the response is too large"));
        }
        if !(200..300).contains(&response.status) {
            return Err(self.http_error(response.status, &response.body));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|_| GenError::BadResponse("not JSON"))?;
        if let Some(image) = find_image(&value)? {
            return Ok(image);
        }
        // No image: the model answered in text (a refusal or a safety block) or produced nothing.
        let text = output_text(&value);
        Err(GenError::Refused(if text.is_empty() { "it returned no image".into() } else { clip(&self.key.scrub(&text), 300) }))
    }

    fn http_error(&self, status: u16, body: &[u8]) -> GenError {
        let error = serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("error").cloned());
        let message = error.as_ref().and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("");
        let reason = error.as_ref().and_then(|e| e.get("status")).and_then(Value::as_str).unwrap_or("");
        let invalid_key = message.contains("API key") || message.contains("API_KEY") || reason == "UNAUTHENTICATED";
        match status {
            401 | 403 => GenError::Auth,
            400 if invalid_key => GenError::Auth,
            429 => GenError::Quota,
            _ => GenError::Http { status, message: if message.is_empty() { reason.to_owned() } else { clip(&self.key.scrub(message), 300) } },
        }
    }
}

impl<T: HttpTransport> ImageProvider for GeminiProvider<T> {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: ID, name: NAME, model: self.model.clone(), host: HOST }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: true, edit: true, variations: true }
    }

    fn edit_aspect(&self, w: u32, h: u32) -> (u32, u32) {
        crate::nearest_ratio(w, h, RATIOS).unwrap_or((1, 1))
    }

    fn generate(&self, r: &GenerateRequest) -> Result<GeneratedImage> {
        crate::check_prompt(&r.prompt, true)?;
        self.request(vec![text(&r.prompt)], r.aspect, r.size)
    }

    fn edit(&self, r: &EditRequest) -> Result<GeneratedImage> {
        crate::check_prompt(&r.prompt, false)?;
        let size = crate::png_size(&r.image).ok_or_else(|| GenError::Invalid("the image must be a PNG".into()))?;
        if crate::png_size(&r.mask) != Some(size) {
            return Err(GenError::Invalid("the mask must be a PNG the size of the image".into()));
        }
        self.request(vec![text(&fill_instruction(&r.prompt)), png(&r.image), png(&r.mask)], size, r.size)
    }

    fn variations(&self, r: &VariationRequest) -> Result<GeneratedImage> {
        crate::check_prompt(&r.prompt, false)?;
        let size = crate::png_size(&r.image).ok_or_else(|| GenError::Invalid("the image must be a PNG".into()))?;
        let ask = if r.prompt.trim().is_empty() {
            "Create a variation of this image: the same subject, composition and style, with natural differences in detail.".to_owned()
        } else {
            format!("Create a variation of this image, keeping its composition and style: {}", r.prompt.trim())
        };
        self.request(vec![text(&ask), png(&r.image)], size, r.size)
    }
}

/// The instruction that stands in for a mask parameter. It asks the model to read the scene
/// first and to continue what crosses into the area; an empty prompt means "continue the image
/// through it", not "erase it".
pub fn fill_instruction(prompt: &str) -> String {
    let what = if prompt.trim().is_empty() {
        "Fill the white area by continuing the image through it. Everything that reaches into the white area from \
         around it (objects, people, edges, lines, patterns, textures, horizon, shadows, reflections) carries on inside \
         it the way it would plausibly continue, and anything partly covered is completed rather than erased. Do not \
         leave an empty or smeared patch, and do not add new subjects that nothing around it suggests."
            .to_owned()
    } else {
        format!(
            "In the white area, create: {}. Make it belong to the scene: the right size for its place in the perspective, \
             lit from the same direction, casting and receiving shadows and reflections, and interacting with whatever \
             touches it. Anything that crosses the edge of the white area continues into it naturally around the new content.",
            prompt.trim().trim_end_matches('.')
        )
    };
    format!(
        "Edit the first image. The second image is a mask the same size as the first: white marks the area to change, \
         black the area to keep. Before editing, study the whole first image: what the scene is, where the light comes \
         from, the perspective and depth, and which objects run into the white area. {what} Match the colour, focus, \
         noise and grain of the surroundings so the edit is seamless. Keep everything in the black area and the framing \
         exactly as they are. Reply with the edited image only."
    )
}

fn text(t: &str) -> Value {
    json!({"type": "text", "text": t})
}

fn png(bytes: &[u8]) -> Value {
    json!({"type": "image", "mime_type": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(bytes)})
}

/// The first image in a response: `output_image`, else an image block in `steps[].content[]` or
/// `outputs[]`.
fn find_image(v: &Value) -> Result<Option<GeneratedImage>> {
    let mut candidates: Vec<&Value> = Vec::new();
    candidates.extend(v.get("output_image"));
    for key in ["steps", "outputs"] {
        for item in v.get(key).and_then(Value::as_array).into_iter().flatten() {
            candidates.push(item);
            candidates.extend(item.get("content").and_then(Value::as_array).into_iter().flatten());
        }
    }
    for c in candidates {
        let is_image = c.get("type").and_then(Value::as_str).is_none_or(|t| t == "image");
        let Some(data) = c.get("data").and_then(Value::as_str).filter(|_| is_image) else { continue };
        if data.len() > crate::MAX_RESPONSE {
            return Err(GenError::BadResponse("the image is too large"));
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| GenError::BadResponse("the image data is not base64"))?;
        let mime = c.get("mime_type").and_then(Value::as_str).unwrap_or("image/jpeg").to_owned();
        return Ok(Some(GeneratedImage { bytes, mime }));
    }
    Ok(None)
}

fn output_text(v: &Value) -> String {
    if let Some(t) = v.get("output_text").and_then(Value::as_str) {
        return t.trim().to_owned();
    }
    let mut out = Vec::new();
    for step in v.get("steps").and_then(Value::as_array).into_iter().flatten() {
        for c in step.get("content").and_then(Value::as_array).into_iter().flatten() {
            if c.get("type").and_then(Value::as_str) == Some("text")
                && let Some(t) = c.get("text").and_then(Value::as_str)
            {
                out.push(t.trim());
            }
        }
    }
    out.join(" ")
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpResponse;
    use std::sync::Mutex;

    /// A request as sent: URL, headers, JSON body.
    type Sent = (String, Vec<(&'static str, String)>, Value);

    /// Records each request and answers with the next canned response.
    struct Fake {
        sent: Mutex<Vec<Sent>>,
        replies: Mutex<Vec<Result<HttpResponse>>>,
    }

    impl Fake {
        fn new(replies: Vec<Result<HttpResponse>>) -> Self {
            Self { sent: Mutex::new(Vec::new()), replies: Mutex::new(replies) }
        }
        fn ok(body: Value) -> Result<HttpResponse> {
            Ok(HttpResponse { status: 200, body: serde_json::to_vec(&body).unwrap() })
        }
        fn status(status: u16, body: Value) -> Result<HttpResponse> {
            Ok(HttpResponse { status, body: serde_json::to_vec(&body).unwrap() })
        }
    }

    impl HttpTransport for Fake {
        fn post(&self, r: HttpRequest) -> Result<HttpResponse> {
            let body = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            self.sent.lock().unwrap().push((r.url, r.headers, body));
            self.replies.lock().unwrap().remove(0)
        }
    }

    fn png_of(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(b"rest");
        v
    }

    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn provider(replies: Vec<Result<HttpResponse>>) -> GeminiProvider<Fake> {
        GeminiProvider::new(ApiKey::new("AIza-test-secret").unwrap(), DEFAULT_MODEL, Fake::new(replies)).unwrap()
    }

    fn edit(prompt: &str, w: u32, h: u32) -> EditRequest {
        EditRequest { image: png_of(w, h), mask: png_of(w, h), prompt: prompt.into(), size: ImageSize::Medium }
    }

    #[test]
    fn edit_sends_prompt_image_and_mask_with_the_key_in_a_header() {
        let p = provider(vec![Fake::ok(json!({"output_image": {"mime_type": "image/png", "data": b64(b"result")}}))]);
        let out = p.edit(&edit("a red balloon", 1600, 900)).unwrap();
        assert_eq!(out, GeneratedImage { bytes: b"result".to_vec(), mime: "image/png".into() });
        let sent = p.transport().sent.lock().unwrap();
        let (url, headers, body) = &sent[0];
        assert_eq!(url, ENDPOINT);
        assert!(!url.contains("secret"), "the key goes in a header, never the URL");
        assert!(headers.contains(&("x-goog-api-key", "AIza-test-secret".into())));
        assert_eq!(body["model"], DEFAULT_MODEL);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert!(input[0]["text"].as_str().unwrap().contains("create: a red balloon."));
        assert_eq!(input[1]["data"], b64(&png_of(1600, 900)));
        assert_eq!(input[2]["type"], "image");
        assert_eq!(body["response_format"]["aspect_ratio"], "16:9");
        assert_eq!(body["response_format"]["image_size"], "2K");
        assert_eq!(body["response_format"]["mime_type"], "image/jpeg", "the only output format the API accepts");
    }

    #[test]
    fn an_empty_prompt_asks_for_removal() {
        let empty = fill_instruction("");
        assert!(empty.contains("continuing the image through it") && empty.contains("completed rather than erased"), "{empty}");
        assert!(!empty.contains("Remove"));
        assert!(fill_instruction("  a boat. ").contains("create: a boat."));
    }

    #[test]
    fn images_are_found_in_steps_too_and_lite_models_stay_at_1k() {
        let reply = json!({"steps": [{"type": "model_output", "content": [{"type": "text", "text": "Here"}, {"type": "image", "mime_type": "image/jpeg", "data": b64(b"jpg")}]}]});
        let p = GeminiProvider::new(ApiKey::new("k").unwrap(), "gemini-3.1-flash-lite-image", Fake::new(vec![Fake::ok(reply)])).unwrap();
        let out = p.edit(&EditRequest { size: ImageSize::Large, ..edit("", 100, 100) }).unwrap();
        assert_eq!(out.mime, "image/jpeg");
        assert_eq!(p.transport().sent.lock().unwrap()[0].2["response_format"]["image_size"], "1K");
    }

    #[test]
    fn errors_map_to_actionable_messages_without_the_key() {
        let cases = [
            (Fake::status(400, json!({"error": {"message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}})), GenError::Auth),
            (Fake::status(403, json!({})), GenError::Auth),
            (Fake::status(429, json!({"error": {"message": "Resource exhausted"}})), GenError::Quota),
            (Fake::status(500, json!({"error": {"message": "boom AIza-test-secret"}})), GenError::Http { status: 500, message: "boom [key]".into() }),
            (Fake::ok(json!({"output_text": "I can't help with that."})), GenError::Refused("I can't help with that.".into())),
            (Fake::ok(json!({})), GenError::Refused("it returned no image".into())),
            (Ok(HttpResponse { status: 200, body: b"<html>".to_vec() }), GenError::BadResponse("not JSON")),
            (Fake::ok(json!({"output_image": {"data": "%%%"}})), GenError::BadResponse("the image data is not base64")),
            (Err(GenError::Network("timed out".into())), GenError::Network("timed out".into())),
        ];
        for (reply, want) in cases {
            let p = provider(vec![reply]);
            let got = p.edit(&edit("x", 64, 64)).unwrap_err();
            assert_eq!(got, want);
            assert!(!format!("{got} {got:?}").contains("AIza-test-secret"));
        }
    }

    #[test]
    fn bad_input_fails_before_anything_is_sent() {
        let p = provider(Vec::new());
        assert!(matches!(p.edit(&EditRequest { image: b"jpeg".to_vec(), ..edit("x", 8, 8) }), Err(GenError::Invalid(_))));
        assert!(matches!(p.edit(&EditRequest { mask: png_of(9, 8), ..edit("x", 8, 8) }), Err(GenError::Invalid(_))));
        assert!(matches!(p.generate(&GenerateRequest { prompt: " ".into(), aspect: (1, 1), size: ImageSize::Small }), Err(GenError::Invalid(_))));
        assert!(p.transport().sent.lock().unwrap().is_empty());
        assert!(GeminiProvider::new(ApiKey::new("k").unwrap(), "bad model/../x", Fake::new(Vec::new())).is_err());
        assert_eq!(GeminiProvider::new(ApiKey::new("k").unwrap(), " ", Fake::new(Vec::new())).unwrap().info().model, DEFAULT_MODEL);
    }

    #[test]
    fn generate_and_variations_shape() {
        let p = provider(vec![Fake::ok(json!({"output_image": {"data": b64(b"a")}})), Fake::ok(json!({"output_image": {"data": b64(b"b")}}))]);
        p.generate(&GenerateRequest { prompt: "a fox".into(), aspect: (3, 4), size: ImageSize::Small }).unwrap();
        p.variations(&VariationRequest { image: png_of(500, 500), prompt: String::new(), size: ImageSize::Small }).unwrap();
        let sent = p.transport().sent.lock().unwrap();
        assert_eq!(sent[0].2["input"].as_array().unwrap().len(), 1);
        assert_eq!(sent[0].2["response_format"]["aspect_ratio"], "3:4");
        assert_eq!(sent[1].2["input"].as_array().unwrap().len(), 2);
        assert_eq!(p.capabilities(), Capabilities { generate: true, edit: true, variations: true });
        assert_eq!(p.edit_aspect(1920, 1080), (16, 9));
    }
}
