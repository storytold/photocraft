//! Native HTTP adapters. Construct and execute only on a background worker.
use crate::{AiError, Control, ImageSize, Result, RgbaImage};
use image::{ImageDecoder, ImageEncoder};
use reqwest::{Client, Method, Response};
use serde_json::Value;
use std::{future::Future, io::Cursor, time::Duration};

pub mod openai;

pub fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| AiError::Unavailable)?.block_on(future)
}

/// Deliberately no Debug implementation: this owns a credential resolved on the worker.
pub struct Http {
    client: Client,
    base: String,
    key: Option<String>,
}
impl Http {
    pub fn configured(base: &str, key_variable: &str, key: Option<&crate::secret::Secret>) -> Result<Self> {
        if let Some(key) = key.filter(|key| !key.expose().is_empty()) {
            key.validate()?;
            return Self::with_key(base, Some(key.expose().to_owned()));
        }
        Self::new(base, key_variable, false)
    }
    pub fn new(base: &str, key_variable: &str, required: bool) -> Result<Self> {
        crate::config::validate_endpoint(base)?;
        crate::config::validate_key_reference(key_variable, !required)?;
        let key = if key_variable.is_empty() {
            None
        } else {
            Some(
                std::env::var(key_variable)
                    .ok()
                    .filter(|v| !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control))
                    .ok_or_else(|| AiError::MissingCredential(key_variable.into()))?,
            )
        };
        Self::with_key(base, key)
    }
    fn with_key(base: &str, key: Option<String>) -> Result<Self> {
        crate::config::validate_endpoint(base)?;
        if key.is_some() && base.starts_with("http://") && !crate::config::cleartext_credentials_allowed(base) {
            return Err(AiError::InvalidRequest("An API key is only sent over HTTPS, or over HTTP to a loopback, private-network or .local address".into()));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .user_agent("PhotoCraft")
            .build()
            .map_err(|_| AiError::Unavailable)?;
        Ok(Self { client, base: base.trim_end_matches('/').to_owned(), key })
    }
    pub fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let request = self.client.request(method, format!("{}/{path}", self.base));
        if let Some(key) = &self.key { request.bearer_auth(key) } else { request }
    }
    pub async fn send(&self, request: reqwest::RequestBuilder, control: &Control) -> Result<Response> {
        control.check()?;
        let future = request.send();
        tokio::pin!(future);
        loop {
            tokio::select! {
                response = &mut future => {
                    let response = response.map_err(|e| if e.is_timeout() { AiError::Timeout } else { AiError::Unavailable })?;
                    return match response.status().as_u16() {
                        200..=299 => Ok(response), 401 | 403 => Err(AiError::Authentication), 429 => Err(AiError::RateLimited), _ => Err(AiError::Rejected),
                    };
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => { control.check()?; }
            }
        }
    }
    pub async fn bytes(&self, mut response: Response, limit: usize, control: &Control) -> Result<Vec<u8>> {
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(AiError::StorageFull);
        }
        let mut bytes = Vec::new();
        loop {
            control.check()?;
            let chunk = response.chunk();
            tokio::pin!(chunk);
            let data = loop {
                tokio::select! {
                    data = &mut chunk => { break data.map_err(|_| AiError::Unavailable)?; }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => { control.check()?; }
                }
            };
            let Some(data) = data else {
                return Ok(bytes);
            };
            if bytes.len().checked_add(data.len()).is_none_or(|n| n > limit) {
                return Err(AiError::StorageFull);
            }
            bytes.extend_from_slice(&data);
        }
    }
    pub async fn json(&self, request: reqwest::RequestBuilder, limit: usize, control: &Control) -> Result<Value> {
        let response = self.send(request, control).await?;
        serde_json::from_slice(&self.bytes(response, limit, control).await?).map_err(|_| AiError::Rejected)
    }
}

pub fn png(image: &RgbaImage) -> Result<Vec<u8>> {
    let bytes: Vec<u8> = image.samples().iter().map(|v| (v * 255.0).round() as u8).collect();
    let mut output = Vec::new();
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(&bytes, image.size().width, image.size().height, image::ExtendedColorType::Rgba8)
        .map_err(|_| AiError::InvalidImage("PNG encoding failed".into()))?;
    Ok(output)
}

/// PNG only, bounded before allocation; small VAE rounding is mapped back to the exact ROI.
pub fn decode_png(bytes: &[u8], size: ImageSize, allow_resize: bool) -> Result<RgbaImage> {
    size.pixels()?;
    let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(|_| AiError::InvalidImage("invalid PNG".into()))?;
    let (width, height) = decoder.dimensions();
    ImageSize { width, height }.pixels()?;
    if (width != size.width || height != size.height) && (!allow_resize || width.abs_diff(size.width) > 32 || height.abs_diff(size.height) > 32) {
        return Err(AiError::InvalidImage("unexpected output dimensions".into()));
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    decoder.set_limits(limits).map_err(|_| AiError::StorageFull)?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| AiError::InvalidImage("PNG decoding failed".into()))?.into_rgba8();
    if width != size.width || height != size.height {
        if !allow_resize || width.abs_diff(size.width) > 32 || height.abs_diff(size.height) > 32 {
            return Err(AiError::InvalidImage("unexpected output dimensions".into()));
        }
        image = image::imageops::resize(&image, size.width, size.height, image::imageops::FilterType::Triangle);
    }
    RgbaImage::new(size, image.into_raw().into_iter().map(|v| f32::from(v) / 255.0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    /// A real HTTP peer: checks the wire request, with one response and no external account.
    pub(super) fn peer(status: u16, body: Vec<u8>, inspect: impl FnOnce(String) + Send + 'static) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut input = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                input.extend_from_slice(&buffer[..count]);
                assert!(input.len() < 256 * 1024);
                if let Some(end) = input.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&input[..end]).to_lowercase();
                    let length = header.lines().find_map(|l| l.strip_prefix("content-length: ")).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                    if input.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            inspect(String::from_utf8_lossy(&input).into_owned());
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        });
        (format!("http://{address}/proxy/v1"), handle)
    }
    #[test]
    fn transport_preserves_proxy_prefix_and_maps_auth_without_echoing_secrets() {
        let (base, server) = peer(401, b"secret-provider-details".to_vec(), |request| {
            assert!(request.starts_with("GET /proxy/v1/test "));
            assert!(request.to_lowercase().contains("authorization: bearer synthetic-test-token"));
        });
        let result = block_on(async {
            let http = Http::with_key(&base, Some("synthetic-test-token".into()))?;
            http.json(http.request(Method::GET, "test"), 1024, &Control::default()).await
        });
        assert_eq!(result.unwrap_err(), AiError::Authentication);
        server.join().unwrap();
    }
    #[test]
    fn transport_bounds_even_successful_response_bodies() {
        let (base, server) = peer(200, vec![42; 1024], |_| {});
        let result = block_on(async {
            let http = Http::with_key(&base, None)?;
            let response = http.send(http.request(Method::GET, "test"), &Control::default()).await?;
            http.bytes(response, 32, &Control::default()).await
        });
        assert_eq!(result.unwrap_err(), AiError::StorageFull);
        server.join().unwrap();
    }
    #[test]
    fn cancelled_transport_never_opens_a_connection() {
        let control = Control::default();
        control.cancel();
        let result = block_on(async {
            let http = Http::with_key("http://127.0.0.1:1", None)?;
            http.send(http.request(Method::GET, "test"), &control).await
        });
        assert!(matches!(result, Err(AiError::Cancelled)));
    }
    #[test]
    fn png_roundtrip_and_dimension_validation() {
        let size = ImageSize { width: 32, height: 32 };
        let source = RgbaImage::new(size, vec![1.0; 32 * 32 * 4]).unwrap();
        let bytes = png(&source).unwrap();
        assert_eq!(decode_png(&bytes, size, false).unwrap(), source);
        assert!(decode_png(&bytes, ImageSize { width: 128, height: 128 }, true).is_err());
        assert!(decode_png(b"not PNG", size, false).is_err());
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn stored_key_overrides_a_missing_environment_variable() {
        let key = crate::secret::Secret::new("synthetic-config-key".into()).unwrap();
        let http = Http::configured("http://localhost:8188", "PHOTOCRAFT_TEST_NONEXISTENT_ENV_KEY", Some(&key)).unwrap();
        let request = http.request(Method::GET, "object_info").build().unwrap();
        assert_eq!(request.headers().get("authorization").unwrap(), "Bearer synthetic-config-key");
        assert!(matches!(Http::configured("http://localhost:8188", "PHOTOCRAFT_TEST_NONEXISTENT_ENV_KEY", None), Err(AiError::MissingCredential(_))));
        let anonymous = Http::configured("http://localhost:8188", "", None).unwrap().request(Method::GET, "object_info").build().unwrap();
        assert!(anonymous.headers().get("authorization").is_none());
    }
}
