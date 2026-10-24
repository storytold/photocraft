//! Files in and out of the page's automation ([`crate::command::OPEN_URL`],
//! [`crate::command::EXPORT`]): open fetched bytes as a document, and encode the active document
//! to bytes for the caller. The browser has no file system to point `file.open`/`file.saveACopy`
//! at, so these take and give bytes instead. Pure apart from the app; `web.rs` does the fetching.

use base64::Engine as _;
use photocraft_ui_egui::PhotocraftApp;
use serde_json::{Value, json};

use crate::command::Host;

/// The largest file `web.openUrl` reads (bytes): what the desktop opens comfortably, and well
/// under what a browser tab can hold twice (the download and the decoded document).
pub const MAX_OPEN_BYTES: usize = 512 * 1024 * 1024;

/// The formats `web.exportBytes` writes, with their media types. Layered: psd, tiff; the rest are
/// flattened composites.
const EXPORT_FORMATS: &[(&str, &str)] =
    &[("png", "image/png"), ("jpg", "image/jpeg"), ("webp", "image/webp"), ("psd", "image/vnd.adobe.photoshop"), ("tiff", "image/tiff")];

/// `{url, name?}` → (url, document name). The name defaults to the URL's last path segment,
/// whose extension tells the importer the format.
pub fn open_url_params(params: &Value) -> Result<(String, String), String> {
    let url = params.get("url").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()).ok_or("web.openUrl needs a `url`")?;
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    if let Some(s) = scheme.as_deref()
        && !matches!(s, "http" | "https" | "blob")
        && !url.starts_with('/')
    {
        return Err(format!("web.openUrl reads http(s) and blob URLs, not `{s}:`"));
    }
    let name = match params.get("name") {
        None | Some(Value::Null) => url_name(url),
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim().to_string(),
        Some(other) => return Err(format!("`name` must be a non-empty string, not {other}")),
    };
    Ok((url.to_string(), name))
}

/// The last path segment of `url`, without its query or fragment ("Untitled" when empty).
fn url_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let after_host = path.split_once("://").map_or(path, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    after_host.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("Untitled").to_string()
}

/// `{format, quality?, name?}` → (file name with the format's extension, media type, options).
fn export_params(params: &Value, doc_name: &str) -> Result<(String, &'static str, photocraft_io::ExportOptions), String> {
    let format = params.get("format").and_then(Value::as_str).ok_or("web.exportBytes needs a `format` (png, jpg, webp, psd or tiff)")?;
    let format = match format.to_ascii_lowercase().as_str() {
        "jpeg" => "jpg".to_string(),
        "tif" => "tiff".to_string(),
        f => f.to_string(),
    };
    let &(ext, media_type) =
        EXPORT_FORMATS.iter().find(|(e, _)| *e == format).ok_or_else(|| format!("web.exportBytes can't write `{format}`: use png, jpg, webp, psd or tiff"))?;
    let mut opts = photocraft_io::ExportOptions::default();
    match params.get("quality") {
        None | Some(Value::Null) => {}
        Some(q) => {
            let q = q.as_u64().filter(|q| (1..=100).contains(q)).ok_or_else(|| format!("`quality` must be a whole number 1–100, not {q}"))?;
            let q = u8::try_from(q).map_err(|e| e.to_string())?;
            opts.encode.jpeg_quality = q;
            opts.encode.webp_quality = q;
        }
    }
    let stem = match params.get("name") {
        None | Some(Value::Null) => doc_name.rsplit_once('.').map_or(doc_name, |(s, _)| s).to_string(),
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim().rsplit_once('.').map_or(n.trim(), |(s, _)| s).to_string(),
        Some(other) => return Err(format!("`name` must be a non-empty string, not {other}")),
    };
    let stem = if stem.is_empty() { "Untitled".to_string() } else { stem };
    Ok((format!("{stem}.{ext}"), media_type, opts))
}

impl Host for PhotocraftApp {
    fn open(&mut self, name: &str, bytes: &[u8]) -> Result<Value, String> {
        let before = self.session.documents().len();
        let warnings = self.open_bytes(name, bytes)?;
        // Brushes and gradients go to the preset libraries; a colour-profile question (Edit ›
        // Color Settings › Ask When Opening) holds the file back until it is answered.
        if self.session.documents().len() <= before {
            return Err(format!("{name} did not open as a new document (it may be a preset file, or PhotoCraft is asking about it)"));
        }
        let doc = &self.session.active().ok_or("no document")?.doc;
        Ok(json!({"name": doc.name, "width": doc.size.width, "height": doc.size.height, "warnings": warnings}))
    }

    fn export(&mut self, params: &Value) -> Result<Value, String> {
        let doc = &self.session.active().ok_or("no document is open to export")?.doc;
        let (name, media_type, opts) = export_params(params, &doc.name)?;
        let r = photocraft_io::export(doc, &name, &opts).map_err(|e| e.to_string())?;
        Ok(json!({
            "name": name,
            "mediaType": media_type,
            "size": r.bytes.len(),
            "base64": base64::engine::general_purpose::STANDARD.encode(&r.bytes),
            "warnings": r.warnings,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_params_take_the_name_from_the_url_or_the_caller() {
        let p = |v: Value| open_url_params(&v);
        assert_eq!(
            p(json!({"url": "https://files.example/a/b/poster.psd?sig=1#x"})),
            Ok(("https://files.example/a/b/poster.psd?sig=1#x".into(), "poster.psd".into()))
        );
        assert_eq!(
            p(json!({"url": "https://files.example/api/storage/9f1c", "name": "harbor.png"})),
            Ok(("https://files.example/api/storage/9f1c".into(), "harbor.png".into()))
        );
        assert_eq!(p(json!({"url": "https://files.example/"})), Ok(("https://files.example/".into(), "Untitled".into())));
        assert_eq!(p(json!({"url": "/samples/sky.jpg"})), Ok(("/samples/sky.jpg".into(), "sky.jpg".into())));
        assert_eq!(p(json!({})), Err("web.openUrl needs a `url`".into()));
        assert_eq!(p(json!({"url": "file:///etc/passwd"})), Err("web.openUrl reads http(s) and blob URLs, not `file:`".into()));
        assert_eq!(p(json!({"url": "javascript:alert(1)"})), Err("web.openUrl reads http(s) and blob URLs, not `javascript:`".into()));
        assert_eq!(p(json!({"url": "https://x/y.png", "name": 3})), Err("`name` must be a non-empty string, not 3".into()));
    }

    #[test]
    fn export_params_name_the_file_and_check_the_format() {
        let p = |v: Value| export_params(&v, "Untitled-1.psd").map(|(n, m, o)| (n, m, o.encode.jpeg_quality));
        let default_q = photocraft_io::ExportOptions::default().encode.jpeg_quality;
        assert_eq!(p(json!({"format": "png"})), Ok(("Untitled-1.png".into(), "image/png", default_q)));
        assert_eq!(p(json!({"format": "JPEG", "quality": 80, "name": "tiktok-ad"})), Ok(("tiktok-ad.jpg".into(), "image/jpeg", 80)));
        assert_eq!(p(json!({"format": "tif", "name": "x.png"})), Ok(("x.tiff".into(), "image/tiff", default_q)));
        assert_eq!(p(json!({"format": "gif"})), Err("web.exportBytes can't write `gif`: use png, jpg, webp, psd or tiff".into()));
        assert_eq!(p(json!({})), Err("web.exportBytes needs a `format` (png, jpg, webp, psd or tiff)".into()));
        assert_eq!(p(json!({"format": "jpg", "quality": 0})), Err("`quality` must be a whole number 1–100, not 0".into()));
        assert_eq!(p(json!({"format": "jpg", "quality": 101})), Err("`quality` must be a whole number 1–100, not 101".into()));
    }
}
