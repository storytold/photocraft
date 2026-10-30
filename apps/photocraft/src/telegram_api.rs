//! HTTPS POST for File › Send to Telegram. The bot token is in the URL; it is stripped from
//! any error returned to the shell.

use std::time::Duration;

/// Post a `sendDocument` body. Rejects any URL that is not Telegram's bot method, so this
/// cannot be reused as a general request helper.
pub fn post_send_document(url: &str, content_type: &str, body: &[u8]) -> Result<String, String> {
    let token = bot_token(url).ok_or_else(|| "Telegram address was rejected".to_string())?;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    match agent.post(url).set("Content-Type", content_type).send_bytes(body) {
        Ok(response) => {
            let status = response.status();
            let text = response.into_string().unwrap_or_default();
            interpret(status, &text, token)
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            interpret(status, &text, token)
        }
        Err(error) => Err(redact(&error.to_string(), token)),
    }
}

/// GET `getUpdates`. Same host check as [`post_send_document`].
pub fn get(url: &str) -> Result<String, String> {
    let token = bot_token(url).ok_or_else(|| "Telegram address was rejected".to_string())?;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    match agent.get(url).call() {
        Ok(response) => {
            let status = response.status();
            let text = response.into_string().unwrap_or_default();
            interpret(status, &text, token)
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            interpret(status, &text, token)
        }
        Err(error) => Err(redact(&error.to_string(), token)),
    }
}

fn bot_token(url: &str) -> Option<&str> {
    if let Some(rest) = url.strip_prefix("https://api.telegram.org/file/bot") {
        let (token, path) = rest.split_once('/')?;
        if !token.is_empty() && !path.is_empty() && !path.contains("..") {
            return Some(token);
        }
        return None;
    }
    let rest = url.strip_prefix("https://api.telegram.org/bot")?;
    let (token, method) = rest.split_once('/')?;
    let method = method.split('?').next().unwrap_or(method);
    if matches!(method, "sendDocument" | "sendPhoto" | "sendMediaGroup" | "pinChatMessage" | "getUpdates" | "getFile") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

const DOWNLOAD_LIMIT: usize = 20 * 1024 * 1024;

/// `getFile` then the file bytes. The file id is not a URL.
pub fn download_file_id(token: &str, file_id: &str) -> Result<Vec<u8>, String> {
    let info_url = format!("https://api.telegram.org/bot{token}/getFile?file_id={}", encode_query(file_id));
    let info = get(&info_url)?;
    let value: serde_json::Value = serde_json::from_str(&info).map_err(|_| "Telegram did not name the file.".to_string())?;
    let path = value.get("result").and_then(|r| r.get("file_path")).and_then(|p| p.as_str()).ok_or_else(|| "Telegram did not name the file.".to_string())?;
    let size = value.get("result").and_then(|r| r.get("file_size")).and_then(|n| n.as_u64()).unwrap_or(0);
    if size > DOWNLOAD_LIMIT as u64 {
        return Err("That file is larger than Telegram will let a bot download (20 MB).".to_string());
    }
    if path.contains("..") || path.contains('\\') {
        return Err("Telegram address was rejected".to_string());
    }
    download(&format!("https://api.telegram.org/file/bot{token}/{path}"))
}

fn encode_query(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Download a file Telegram already named in `getFile`. Caps the read at 20 MB.
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let token = bot_token(url).filter(|_| url.contains("/file/bot")).ok_or_else(|| "Telegram address was rejected".to_string())?;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    let response = agent.get(url).call().map_err(|error| redact(&error.to_string(), token))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut reader = response.into_reader();
    loop {
        let n = std::io::Read::read(&mut reader, &mut chunk).map_err(|error| redact(&error.to_string(), token))?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > DOWNLOAD_LIMIT {
            return Err("That file is larger than Telegram will let a bot download (20 MB).".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

fn interpret(status: u16, text: &str, token: &str) -> Result<String, String> {
    if (200..300).contains(&status) {
        let ok = serde_json::from_str::<serde_json::Value>(text).ok().and_then(|v| v.get("ok").and_then(|b| b.as_bool()));
        if ok != Some(false) {
            return Ok(text.to_string());
        }
    }
    let description = serde_json::from_str::<serde_json::Value>(text).ok().and_then(|v| v.get("description").and_then(|d| d.as_str()).map(str::to_string));
    Err(redact(&description.unwrap_or_else(|| format!("HTTP {status}")), token))
}

fn redact(text: &str, token: &str) -> String {
    if token.len() < 8 { text.to_string() } else { text.replace(token, "…") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_send_document_url_is_accepted() {
        assert!(bot_token("https://api.telegram.org/bot123:abc/sendDocument").is_some());
        assert!(bot_token("https://api.telegram.org/bot123:abc/sendPhoto").is_some());
        assert!(bot_token("https://api.telegram.org/bot123:abc/sendMediaGroup").is_some());
        assert!(bot_token("https://api.telegram.org/bot123:abc/pinChatMessage").is_some());
        assert!(bot_token("https://api.telegram.org/bot123:abc/getFile?file_id=abc").is_some());
        assert!(bot_token("https://api.telegram.org/file/bot123:abc/documents/file.png").is_some());
        assert!(bot_token("https://api.telegram.org/bot123:abc/getUpdates").is_some());
        assert!(bot_token("https://example.com/bot123:abc/sendDocument").is_none());
        assert!(bot_token("https://api.telegram.org/file/bot123:abc/../secret").is_none());
        assert!(bot_token("https://api.telegram.org/bot123:abc/getMe").is_none());
    }

    #[test]
    fn errors_drop_the_token() {
        let token = "123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        assert_eq!(redact(&format!("refused https://api.telegram.org/bot{token}/sendDocument"), token), "refused https://api.telegram.org/bot…/sendDocument");
    }
}
