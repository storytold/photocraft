//! Native Gemini API image generation (`gemini-2.5-flash-image`) and local API key storage.
//!
//! Uses `curl` with the API key passed via stdin (`-K -`) so the credential never appears in
//! process arguments or logs, and avoids adding new HTTP/TLS crate dependencies to `ui-egui`.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::connector;

const GEMINI_IMAGE_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-image:generateContent";

/// Outcome of a background Gemini image generation request.
#[derive(Debug)]
pub enum ImageGenResult {
    Ready { bytes: Vec<u8>, ext: &'static str, caption: Option<String> },
    Error(String),
}

/// Path to the user's local `0600` AI key store (`~/.config/photocraft/ai_keys.json`).
pub fn key_store_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("photocraft").join("ai_keys.json"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| connector::home_dir().map(|h| h.join(".config")));
        base.map(|b| b.join("photocraft/ai_keys.json"))
    }
}

/// Locally stored optional API keys (`0600` in `~/.config/photocraft/ai_keys.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiKeys {
    pub gemini_api_key: String,
    pub anthropic_api_key: String,
}

/// Load `GEMINI_API_KEY` and `ANTHROPIC_API_KEY` from the environment or the local `0600` key store.
pub fn load_keys() -> AiKeys {
    let file_val = key_store_path().and_then(|p| fs::read_to_string(p).ok()).and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let from_file = |field: &str| file_val.as_ref().and_then(|v| v.get(field)).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let env_or_file =
        |env_name: &str, field: &str| std::env::var(env_name).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).unwrap_or_else(|| from_file(field));
    AiKeys { gemini_api_key: env_or_file("GEMINI_API_KEY", "geminiApiKey"), anthropic_api_key: env_or_file("ANTHROPIC_API_KEY", "anthropicApiKey") }
}

/// Persist both optional API keys to the local `0600` key store.
pub fn save_keys(keys: &AiKeys) -> Result<PathBuf, String> {
    let path = key_store_path().ok_or_else(|| "Could not locate config directory for API key storage.".to_owned())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    let payload = json!({
        "geminiApiKey": keys.gemini_api_key.trim(),
        "anthropicApiKey": keys.anthropic_api_key.trim(),
    });
    let text = serde_json::to_string_pretty(&payload).map_err(|e| format!("Failed to encode key file: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, format!("{text}\n")).map_err(|e| format!("Failed to write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, &path).map_err(|e| format!("Failed to save {}: {e}", path.display()))?;
    Ok(path)
}

/// Read plain text from the OS clipboard (used by Paste buttons when native menu shortcuts intercept Cmd+V).
pub fn read_clipboard_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    let out = Command::new("pbpaste").output().ok()?;
    #[cfg(target_os = "windows")]
    let out = Command::new("powershell").args(["-NoProfile", "-Command", "Get-Clipboard"]).output().ok()?;
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let out = Command::new("wl-paste").arg("--no-newline").output().or_else(|_| Command::new("xclip").args(["-selection", "clipboard", "-o"]).output()).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if text.is_empty() { None } else { Some(text) }
}

/// Spawn a background worker that calls `gemini-2.5-flash-image` and returns decoded image bytes.
pub fn spawn_generate(prompt: String, api_key: String, ctx: egui::Context) -> Receiver<ImageGenResult> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().name("gemini-image-gen".into()).spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| generate_blocking(&prompt, &api_key)));
        let res = match outcome {
            Ok(r) => r,
            Err(_) => ImageGenResult::Error("Gemini image worker panicked.".to_owned()),
        };
        let _ = tx.send(res);
        ctx.request_repaint();
    });
    rx
}

fn generate_blocking(prompt: &str, api_key: &str) -> ImageGenResult {
    let body = json!({
        "contents": [{ "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "responseModalities": ["TEXT", "IMAGE"]
        }
    });
    let Ok(body_bytes) = serde_json::to_vec(&body) else {
        return ImageGenResult::Error("Failed to encode Gemini image request.".to_owned());
    };
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let body_path = std::env::temp_dir().join(format!("photocraft-gemini-req-{}-{stamp}.json", std::process::id()));
    if let Err(e) = fs::write(&body_path, &body_bytes) {
        return ImageGenResult::Error(format!("Failed to write request body: {e}"));
    }

    let mut child = match Command::new("curl")
        .arg("-sS")
        .arg("--max-time")
        .arg("120")
        .arg("-X")
        .arg("POST")
        .arg(GEMINI_IMAGE_ENDPOINT)
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-K")
        .arg("-")
        .arg("--data-binary")
        .arg(format!("@{}", body_path.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&body_path);
            return ImageGenResult::Error(format!("Failed to launch curl: {e}"));
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "header = \"x-goog-api-key: {}\"", api_key.trim());
    }
    let output = child.wait_with_output();
    let _ = fs::remove_file(&body_path);

    let output = match output {
        Ok(o) => o,
        Err(e) => return ImageGenResult::Error(format!("curl failed: {e}")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return ImageGenResult::Error(format!("Gemini request failed: {}", stderr.trim()));
    }
    parse_response(&output.stdout)
}

/// Extract the first inline image (and optional text caption) from a Gemini `generateContent` JSON reply.
pub fn parse_response(raw: &[u8]) -> ImageGenResult {
    let val: Value = match serde_json::from_slice(raw) {
        Ok(v) => v,
        Err(e) => return ImageGenResult::Error(format!("Invalid JSON from Gemini API: {e}")),
    };
    if let Some(err) = val.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("Unknown Gemini API error");
        return ImageGenResult::Error(format!("Gemini API error: {msg}"));
    }
    let Some(parts) = val
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
    else {
        return ImageGenResult::Error("Gemini returned no image candidates.".to_owned());
    };
    let mut caption = None;
    for part in parts {
        if let Some(t) = part.get("text").and_then(Value::as_str)
            && !t.trim().is_empty()
        {
            caption = Some(t.trim().to_owned());
        }
    }
    for part in parts {
        let Some(inline) = part.get("inlineData").or_else(|| part.get("inline_data")) else { continue };
        let Some(b64) = inline.get("data").and_then(Value::as_str) else { continue };
        let mime = inline.get("mimeType").or_else(|| inline.get("mime_type")).and_then(Value::as_str).unwrap_or("image/png");
        let ext = if mime.contains("jpeg") || mime.contains("jpg") {
            "jpg"
        } else if mime.contains("webp") {
            "webp"
        } else {
            "png"
        };
        match decode_base64(b64) {
            Ok(bytes) if !bytes.is_empty() => return ImageGenResult::Ready { bytes, ext, caption },
            Ok(_) => return ImageGenResult::Error("Gemini returned empty image bytes.".to_owned()),
            Err(e) => return ImageGenResult::Error(format!("Base64 decode failed: {e}")),
        }
    }
    let detail = caption.unwrap_or_else(|| "Gemini replied without an image.".to_owned());
    ImageGenResult::Error(detail)
}

/// Pure-Rust standard Base64 decoder (ignores ASCII whitespace).
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for b in input.bytes() {
        if b.is_ascii_whitespace() || b == b'=' {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("invalid base64 byte {b}")),
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_standard_base64() {
        assert_eq!(decode_base64("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(decode_base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
    }

    #[test]
    fn parses_gemini_inline_image_response() {
        let raw = br#"{"candidates":[{"content":{"parts":[{"text":"Here is your bird."},{"inlineData":{"mimeType":"image/png","data":"iVBORw0K"}}]}}]}"#;
        match parse_response(raw) {
            ImageGenResult::Ready { bytes, ext, caption } => {
                assert_eq!(ext, "png");
                assert_eq!(caption.as_deref(), Some("Here is your bird."));
                assert_eq!(bytes, vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a]);
            }
            ImageGenResult::Error(e) => panic!("unexpected error: {e}"),
        }
    }
}
