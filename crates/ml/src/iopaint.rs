//! Provider for a local IOPaint helper (<https://github.com/Sanster/IOPaint>, Apache-2.0).
//!
//! Protocol (IOPaint 1.6): `POST /api/v1/inpaint` with JSON `{image, mask, ...}` where both are
//! base64 PNGs and white mask pixels mean "fill"; the reply body is the result image in the same
//! format as the input (PNG here). `GET /api/v1/server-config` answers 200 when the helper is up.
//! The model is whatever the helper was started with (`iopaint start --model=lama`), so the
//! provider never sends a prompt and reports `accepts_prompt() == false`.

use base64::Engine as _;

use crate::png_codec::{decode_png_rgba, encode_png_rgba};
use crate::transport::Transport;
use crate::{Error, GenerateOptions, GenerativeProvider, Mask, ProviderInfo, RgbaImage};

/// Default helper address: loopback only, IOPaint's default port.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8080";

pub struct IoPaintProvider {
    base_url: String,
    transport: Box<dyn Transport>,
}

impl IoPaintProvider {
    /// `base_url` like `http://127.0.0.1:8080` (a trailing slash is fine).
    pub fn new(base_url: &str, transport: Box<dyn Transport>) -> Self {
        Self { base_url: base_url.trim_end_matches('/').to_owned(), transport }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn request_json(image: &RgbaImage, mask: &Mask) -> Result<Vec<u8>, Error> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let image_png = encode_png_rgba(image)?;
        // IOPaint reads the mask as an image file too; send it as an opaque grey PNG.
        let mask_rgba = RgbaImage { width: mask.width, height: mask.height, data: mask.data.iter().flat_map(|&v| [v, v, v, 255]).collect() };
        let mask_png = encode_png_rgba(&mask_rgba)?;
        let body = serde_json::json!({
            "image": b64.encode(image_png),
            "mask": b64.encode(mask_png),
            // Erase-model (LaMa) preprocessing: work on a crop around the mask when the image is
            // big, so time stays proportional to the area being filled, not the whole canvas.
            "hd_strategy": "Crop",
            "hd_strategy_crop_margin": 128,
            "hd_strategy_crop_trigger_size": 800,
            "hd_strategy_resize_limit": 1280,
        });
        serde_json::to_vec(&body).map_err(|e| Error::BadReply(e.to_string()))
    }
}

impl GenerativeProvider for IoPaintProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { name: "IOPaint (local helper)".into(), destination: format!("a helper program at {}", self.base_url) }
    }

    fn accepts_prompt(&self) -> bool {
        false
    }

    fn ping(&self) -> Result<(), Error> {
        let r = self.transport.get(&self.url("/api/v1/server-config"))?;
        if r.status == 200 { Ok(()) } else { Err(Error::HttpStatus { status: r.status, detail: String::from_utf8_lossy(&r.body).into_owned() }) }
    }

    fn generate(&self, image: &RgbaImage, mask: &Mask, _options: &GenerateOptions, cancelled: &dyn Fn() -> bool) -> Result<RgbaImage, Error> {
        if !image.is_consistent() || !mask.is_consistent() {
            return Err(Error::InvalidBuffer);
        }
        if image.width != mask.width || image.height != mask.height {
            return Err(Error::SizeMismatch { image_w: image.width, image_h: image.height, mask_w: mask.width, mask_h: mask.height });
        }
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let body = Self::request_json(image, mask)?;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let r = self.transport.post(&self.url("/api/v1/inpaint"), "application/json", &body)?;
        if cancelled() {
            return Err(Error::Cancelled);
        }
        if r.status != 200 {
            let detail = String::from_utf8_lossy(&r.body).chars().take(400).collect();
            return Err(Error::HttpStatus { status: r.status, detail });
        }
        let out = decode_png_rgba(&r.body).map_err(|e| Error::BadReply(e.to_string()))?;
        if out.width != image.width || out.height != image.height {
            return Err(Error::WrongSize { want_w: image.width, want_h: image.height, got_w: out.width, got_h: out.height });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FakeTransport;
    use std::sync::Arc;

    fn fixture() -> Result<(RgbaImage, Mask), Error> {
        let image = RgbaImage::filled(4, 3, [9, 8, 7, 255]).ok_or(Error::InvalidBuffer)?;
        let mut mask = Mask { width: 4, height: 3, data: vec![0; 12] };
        mask.data[3] = 255;
        mask.data[7] = 255;
        mask.data[11] = 255;
        Ok((image, mask))
    }

    struct Shared(Arc<FakeTransport>);
    impl Transport for Shared {
        fn get(&self, url: &str) -> Result<crate::HttpResponse, Error> {
            self.0.get(url)
        }
        fn post(&self, url: &str, ct: &str, body: &[u8]) -> Result<crate::HttpResponse, Error> {
            self.0.post(url, ct, body)
        }
    }

    fn provider() -> (IoPaintProvider, Arc<FakeTransport>) {
        let fake = Arc::new(FakeTransport::new());
        let p = IoPaintProvider::new("http://127.0.0.1:8080/", Box::new(Shared(fake.clone())));
        (p, fake)
    }

    #[test]
    fn sends_png_image_and_mask_and_decodes_reply() -> Result<(), Error> {
        let (image, mask) = fixture()?;
        let (p, fake) = provider();
        let reply = RgbaImage::filled(4, 3, [1, 2, 3, 255]).ok_or(Error::InvalidBuffer)?;
        fake.push_ok("image/png", encode_png_rgba(&reply)?);

        let out = p.generate(&image, &mask, &GenerateOptions::default(), &|| false)?;
        assert_eq!(out, reply);

        let reqs = fake.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].url, "http://127.0.0.1:8080/api/v1/inpaint");
        assert_eq!(reqs[0].content_type.as_deref(), Some("application/json"));
        let v: serde_json::Value = serde_json::from_slice(&reqs[0].body).map_err(|e| Error::BadReply(e.to_string()))?;
        let b64 = base64::engine::general_purpose::STANDARD;
        let img_png = b64.decode(v["image"].as_str().unwrap_or("")).map_err(|e| Error::BadReply(e.to_string()))?;
        let sent = decode_png_rgba(&img_png)?;
        assert_eq!(sent, image, "image goes over the wire unchanged");
        let mask_png = b64.decode(v["mask"].as_str().unwrap_or("")).map_err(|e| Error::BadReply(e.to_string()))?;
        let sent_mask = decode_png_rgba(&mask_png)?;
        assert_eq!((sent_mask.width, sent_mask.height), (4, 3));
        assert_eq!(sent_mask.data[3 * 4], 255, "white where the mask says fill");
        assert_eq!(sent_mask.data[0], 0, "black where the mask says keep");
        assert_eq!(v["hd_strategy"], "Crop");
        assert!(v.get("prompt").is_none(), "LaMa provider never sends a prompt");
        Ok(())
    }

    #[test]
    fn http_error_status_is_reported_not_panicked() -> Result<(), Error> {
        let (image, mask) = fixture()?;
        let (p, fake) = provider();
        fake.push(Ok(crate::HttpResponse { status: 500, content_type: None, body: b"model exploded".to_vec() }));
        let err = p.generate(&image, &mask, &GenerateOptions::default(), &|| false);
        assert!(matches!(err, Err(Error::HttpStatus { status: 500, .. })), "{err:?}");
        Ok(())
    }

    #[test]
    fn unreachable_helper_and_bad_reply() -> Result<(), Error> {
        let (image, mask) = fixture()?;
        let (p, fake) = provider();
        // Nothing queued = connection refused.
        assert!(matches!(p.ping(), Err(Error::Unreachable { .. })));
        fake.push_ok("text/html", b"<html>not a png</html>".to_vec());
        let err = p.generate(&image, &mask, &GenerateOptions::default(), &|| false);
        assert!(matches!(err, Err(Error::BadReply(_))), "{err:?}");
        Ok(())
    }

    #[test]
    fn wrong_size_reply_is_rejected() -> Result<(), Error> {
        let (image, mask) = fixture()?;
        let (p, fake) = provider();
        let small = RgbaImage::filled(2, 2, [0, 0, 0, 255]).ok_or(Error::InvalidBuffer)?;
        fake.push_ok("image/png", encode_png_rgba(&small)?);
        let err = p.generate(&image, &mask, &GenerateOptions::default(), &|| false);
        assert!(matches!(err, Err(Error::WrongSize { want_w: 4, want_h: 3, got_w: 2, got_h: 2 })), "{err:?}");
        Ok(())
    }

    #[test]
    fn cancel_before_sending_sends_nothing() -> Result<(), Error> {
        let (image, mask) = fixture()?;
        let (p, fake) = provider();
        let err = p.generate(&image, &mask, &GenerateOptions::default(), &|| true);
        assert!(matches!(err, Err(Error::Cancelled)));
        assert!(fake.requests().is_empty());
        Ok(())
    }

    #[test]
    fn size_mismatch_is_checked_first() -> Result<(), Error> {
        let (image, _) = fixture()?;
        let (p, fake) = provider();
        let mask = Mask { width: 5, height: 3, data: vec![0; 15] };
        let err = p.generate(&image, &mask, &GenerateOptions::default(), &|| false);
        assert!(matches!(err, Err(Error::SizeMismatch { .. })));
        assert!(fake.requests().is_empty());
        Ok(())
    }

    #[test]
    fn ping_and_info() -> Result<(), Error> {
        let (p, fake) = provider();
        fake.push_ok("application/json", b"{}".to_vec());
        p.ping()?;
        assert_eq!(fake.requests()[0].url, "http://127.0.0.1:8080/api/v1/server-config");
        assert!(!p.accepts_prompt());
        assert!(p.info().destination.contains("127.0.0.1:8080"));
        Ok(())
    }
}
