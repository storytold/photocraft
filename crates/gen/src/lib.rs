//! Optional image generation provider. No credentials or response bodies appear in errors.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use base64::Engine as _;
use serde_json::{Value, json};

#[cfg(not(target_arch = "wasm32"))]
const BASE: &str = "https://api.openai.com/v1/images";
const MAX_RESPONSE: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("Generative AI is off: set OPENAI_API_KEY")]
    MissingKey,
    #[error("invalid image request: {0}")]
    Invalid(&'static str),
    #[error("image service unavailable: {0}")]
    Service(&'static str),
    #[error("image service returned HTTP {0}")]
    Http(u16),
}

pub type Result<T> = std::result::Result<T, GenError>;

/// All images and masks are PNG bytes; a mask's transparent pixels are repainted.
pub trait ImageProvider: Send + Sync {
    fn generate(&self, prompt: &str, size: &str) -> Result<Vec<u8>>;
    fn edit(&self, image: &[u8], mask: &[u8], prompt: &str, size: &str) -> Result<Vec<u8>>;
    fn variations(&self, image: &[u8], prompt: &str, size: &str) -> Result<Vec<u8>>;
}

/// Injectable transport; tests can inspect a request without sending it.
pub trait HttpTransport: Send + Sync {
    fn post(&self, path: &str, key: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>>;
}

/// Blocking HTTPS transport, intended for an engine background job.
pub struct UreqTransport;

#[cfg(not(target_arch = "wasm32"))]
impl HttpTransport for UreqTransport {
    fn post(&self, path: &str, key: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        use std::io::Read as _;
        use std::time::Duration;
        let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(180))).https_only(true).max_redirects(0).build();
        let agent = config.new_agent();
        let url = format!("{BASE}{path}");
        let mut response = match agent.post(&url).header("Authorization", format!("Bearer {key}")).header("Content-Type", content_type).send(body) {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => return Err(GenError::Http(code)),
            Err(_) => return Err(GenError::Service("HTTPS connection failed")),
        };
        let mut bytes = Vec::new();
        response.body_mut().as_reader().take((MAX_RESPONSE + 1) as u64).read_to_end(&mut bytes).map_err(|_| GenError::Service("could not read response"))?;
        if bytes.len() > MAX_RESPONSE {
            return Err(GenError::Service("response too large"));
        }
        Ok(bytes)
    }
}

#[cfg(target_arch = "wasm32")]
impl HttpTransport for UreqTransport {
    fn post(&self, _: &str, _: &str, _: &str, _: &[u8]) -> Result<Vec<u8>> {
        Err(GenError::Service("image generation requires the desktop app"))
    }
}

/// OpenAI implementation. `key` remains process memory only; never serialize this type.
pub struct OpenAiProvider<T: HttpTransport = UreqTransport> {
    key: String,
    model: String,
    transport: T,
}

impl OpenAiProvider<UreqTransport> {
    pub fn from_env(model: Option<&str>) -> Result<Self> {
        let key = std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.trim().is_empty()).ok_or(GenError::MissingKey)?;
        Self::new(key, model.unwrap_or("gpt-image-1"), UreqTransport)
    }
}

impl<T: HttpTransport> OpenAiProvider<T> {
    pub fn new(key: String, model: &str, transport: T) -> Result<Self> {
        if key.trim().is_empty() {
            return Err(GenError::MissingKey);
        }
        if !model.starts_with("gpt-image") || model.len() > 80 || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
            return Err(GenError::Invalid("invalid gpt-image model"));
        }
        Ok(Self { key, model: model.to_owned(), transport })
    }

    fn request(&self, path: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        let response = self.transport.post(path, &self.key, content_type, body)?;
        if response.len() > MAX_RESPONSE {
            return Err(GenError::Service("response too large"));
        }
        let value: Value = serde_json::from_slice(&response).map_err(|_| GenError::Service("invalid JSON response"))?;
        let data = value
            .get("data")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|v| v.get("b64_json"))
            .and_then(Value::as_str)
            .ok_or(GenError::Service("response missing b64_json image"))?;
        if data.len() > MAX_RESPONSE {
            return Err(GenError::Service("image too large"));
        }
        base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| GenError::Service("invalid base64 image"))
    }

    fn multipart(&self, image: &[u8], mask: Option<&[u8]>, prompt: &str, size: &str) -> Result<Vec<u8>> {
        validate(prompt, size)?;
        if image.is_empty() || image.len() > 20 * 1024 * 1024 {
            return Err(GenError::Invalid("PNG image is empty or too large"));
        }
        if mask.is_some_and(|m| m.is_empty() || m.len() > 20 * 1024 * 1024) {
            return Err(GenError::Invalid("PNG mask is empty or too large"));
        }
        // A boundary absent from binary input. A constant is safe after checking both buffers.
        let mut boundary = "photocraft-boundary-1".to_owned();
        while prompt.contains(&boundary)
            || image.windows(boundary.len()).any(|w| w == boundary.as_bytes())
            || mask.is_some_and(|m| m.windows(boundary.len()).any(|w| w == boundary.as_bytes()))
        {
            boundary.push('x');
        }
        let mut body = Vec::new();
        field(&mut body, &boundary, "model", self.model.as_bytes());
        field(&mut body, &boundary, "prompt", prompt.as_bytes());
        field(&mut body, &boundary, "size", size.as_bytes());
        field(&mut body, &boundary, "output_format", b"png");
        file(&mut body, &boundary, "image", image);
        if let Some(m) = mask {
            file(&mut body, &boundary, "mask", m);
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        self.request("/edits", &format!("multipart/form-data; boundary={boundary}"), &body)
    }
}

impl<T: HttpTransport> ImageProvider for OpenAiProvider<T> {
    fn generate(&self, prompt: &str, size: &str) -> Result<Vec<u8>> {
        validate(prompt, size)?;
        let body = json!({"model":self.model,"prompt":prompt,"size":size,"output_format":"png"});
        let bytes = serde_json::to_vec(&body).map_err(|_| GenError::Service("could not prepare request"))?;
        self.request("/generations", "application/json", &bytes)
    }
    fn edit(&self, image: &[u8], mask: &[u8], prompt: &str, size: &str) -> Result<Vec<u8>> {
        self.multipart(image, Some(mask), prompt, size)
    }
    fn variations(&self, image: &[u8], prompt: &str, size: &str) -> Result<Vec<u8>> {
        // GPT image models support prompt-guided variations through edits.
        self.multipart(image, None, prompt, size)
    }
}

fn validate(prompt: &str, size: &str) -> Result<()> {
    if prompt.trim().is_empty() || prompt.len() > 32_000 {
        return Err(GenError::Invalid("prompt is empty or too long"));
    }
    if !matches!(size, "1024x1024" | "1024x1536" | "1536x1024" | "auto") {
        return Err(GenError::Invalid("unsupported size"));
    }
    Ok(())
}

fn field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &[u8]) {
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes());
    body.extend_from_slice(value);
    body.extend_from_slice(b"\r\n");
}

fn file(body: &mut Vec<u8>, boundary: &str, name: &str, value: &[u8]) {
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"image.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value);
    body.extend_from_slice(b"\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    type Request = (String, String, String, Vec<u8>);
    struct Mock {
        requests: Mutex<Vec<Request>>,
        replies: Mutex<Vec<Result<Vec<u8>>>>,
    }
    impl Mock {
        fn new(replies: Vec<Result<Vec<u8>>>) -> Self {
            Self { requests: Mutex::new(Vec::new()), replies: Mutex::new(replies) }
        }
    }
    impl HttpTransport for Mock {
        fn post(&self, path: &str, key: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
            self.requests.lock().map_err(|_| GenError::Service("test lock"))?.push((path.into(), key.into(), content_type.into(), body.to_vec()));
            let mut replies = self.replies.lock().map_err(|_| GenError::Service("test lock"))?;
            if replies.is_empty() { Ok(br#"{"data":[{"b64_json":"AQID"}]}"#.to_vec()) } else { replies.remove(0) }
        }
    }
    #[test]
    fn requests_are_mockable_and_have_expected_shapes() {
        let p = OpenAiProvider::new("secret".into(), "gpt-image-1", Mock::new(Vec::new())).unwrap();
        assert_eq!(p.generate("cat", "1024x1024").unwrap(), [1, 2, 3]);
        p.edit(b"png", b"mask", "fill", "1024x1024").unwrap();
        p.variations(b"png", "similar", "1024x1024").unwrap();
        let r = p.transport.requests.lock().unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].0, "/generations");
        assert_eq!(r[0].1, "secret");
        assert_eq!(r[0].2, "application/json");
        assert_eq!(serde_json::from_slice::<Value>(&r[0].3).unwrap(), json!({"model":"gpt-image-1","prompt":"cat","size":"1024x1024","output_format":"png"}));
        assert!(r[1].2.starts_with("multipart/form-data; boundary="));
        assert!(r[1].3.windows(b"name=\"image\"".len()).any(|w| w == b"name=\"image\""));
        assert!(r[1].3.windows(b"name=\"mask\"".len()).any(|w| w == b"name=\"mask\""));
        assert!(r[1].3.windows(b"1024x1024".len()).any(|w| w == b"1024x1024"));
        assert_eq!(r[2].0, "/edits");
        assert!(!r[2].3.windows(b"name=\"mask\"".len()).any(|w| w == b"name=\"mask\""));
    }

    #[test]
    fn response_failures_and_credentials_are_redacted() {
        let secret = "sentinel-test-secret";
        for reply in [
            Err(GenError::Http(401)),
            Err(GenError::Http(429)),
            Err(GenError::Http(500)),
            Ok(b"{".to_vec()),
            Ok(br#"{"data":[{"b64_json":"%%%"}]}"#.to_vec()),
            Ok(br#"{"data":[]}"#.to_vec()),
        ] {
            let p = OpenAiProvider::new(secret.into(), "gpt-image-1", Mock::new(vec![reply])).unwrap();
            let e = p.generate("cat", "1024x1024").unwrap_err();
            assert!(!format!("{e} {e:?}").contains(secret));
        }
        let missing = OpenAiProvider::new(" ".into(), "gpt-image-1", Mock::new(Vec::new())).err().unwrap();
        assert!(matches!(missing, GenError::MissingKey));
        assert!(missing.to_string().starts_with("Generative AI is off"));
        let p = OpenAiProvider::new(secret.into(), "gpt-image-1", Mock::new(Vec::new())).unwrap();
        assert!(matches!(p.generate(" ", "1024x1024"), Err(GenError::Invalid(_))));
        assert!(matches!(p.generate("cat", "99x99"), Err(GenError::Invalid(_))));
        assert!(p.transport.requests.lock().unwrap().is_empty());
    }
}
