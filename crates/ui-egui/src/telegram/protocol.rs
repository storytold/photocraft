//! Telegram API payloads, response parsing, and file helpers.

#[cfg(not(target_arch = "wasm32"))]
use super::MAX_BYTES;
use super::{PhotocraftApp, Value, export_temp_for, json, settings_json};

/// Chats that have talked to the bot, newest last, one row per id.
pub fn chats_from_updates(text: &str) -> Vec<Value> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_str::<Value>(text) else { return out };
    let Some(items) = v.get("result").and_then(Value::as_array) else { return out };
    for item in items {
        let chat = item.get("message").or_else(|| item.get("edited_message")).or_else(|| item.get("channel_post")).and_then(|m| m.get("chat"));
        let Some(chat) = chat else { continue };
        let Some(id) = chat.get("id").and_then(Value::as_i64) else { continue };
        let id = id.to_string();
        if !seen.insert(id.clone()) {
            continue;
        }
        let label = chat.get("title").or_else(|| chat.get("username")).or_else(|| chat.get("first_name")).and_then(Value::as_str).unwrap_or("").to_string();
        let label = if label.is_empty() { id.clone() } else { format!("{label} ({id})") };
        out.push(json!({"id": id, "label": label}));
    }
    out
}

/// Link to the sent message. A public chat gets `https://t.me/…`. A private chat has no web
/// page; Telegram's own app link still opens the message.
pub fn message_link(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let result = v.get("result")?;
    let message = result.as_array().and_then(|items| items.first()).unwrap_or(result);
    let message_id = message.get("message_id")?.as_i64()?;
    let chat = message.get("chat")?;
    if let Some(username) = chat.get("username").and_then(Value::as_str).filter(|name| !name.is_empty()) {
        return Some(format!("https://t.me/{username}/{message_id}"));
    }
    let id = chat.get("id")?.as_i64()?;
    if let Some(internal) = id.to_string().strip_prefix("-100").filter(|rest| !rest.is_empty()) {
        return Some(format!("https://t.me/c/{internal}/{message_id}"));
    }
    if id > 0 {
        Some(format!("tg://openmessage?user_id={id}&message_id={message_id}"))
    } else {
        Some(format!("tg://openmessage?chat_id={id}&message_id={message_id}"))
    }
}

/// Documents and photos in a successful send reply, newest first.
pub fn sent_files_from_reply(body: &str) -> Vec<Value> {
    let Ok(value) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let Some(result) = value.get("result") else { return Vec::new() };
    let messages: Vec<&Value> = result.as_array().map(|items| items.iter().collect()).unwrap_or_else(|| vec![result]);
    let mut out = Vec::new();
    for message in messages {
        push_message_file(message, &mut out);
    }
    out
}

fn push_message_file(message: &Value, out: &mut Vec<Value>) {
    let message_id = message.get("message_id").and_then(Value::as_i64).unwrap_or(0);
    if let Some(document) = message.get("document") {
        let file_id = document.get("file_id").and_then(Value::as_str).unwrap_or("");
        if file_id.is_empty() {
            return;
        }
        let name = document.get("file_name").and_then(Value::as_str).unwrap_or("file");
        out.push(json!({"id": file_id, "name": name, "label": name}));
        return;
    }
    let Some(photos) = message.get("photo").and_then(Value::as_array) else { return };
    let Some(photo) = photos.iter().max_by_key(|p| p.get("file_size").and_then(Value::as_u64).unwrap_or(0)) else { return };
    let file_id = photo.get("file_id").and_then(Value::as_str).unwrap_or("");
    if file_id.is_empty() {
        return;
    }
    let name = format!("telegram-{message_id}.jpg");
    out.push(json!({"id": file_id, "name": name, "label": name}));
}

pub(super) struct SendReport {
    pub(super) urls: Vec<String>,
    pub(super) files: Vec<Value>,
    pub(super) pin_failed: bool,
}

/// The send thread returns either the new JSON report or, from an older build, a bare link.
pub(super) fn send_report(body: &str) -> SendReport {
    if let Ok(value) = serde_json::from_str::<Value>(body)
        && let Some(urls) = value.get("urls").and_then(Value::as_array)
    {
        let urls = urls.iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|url| !url.is_empty()).collect();
        let files = value.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
        let pin_failed = value.get("pinFailed").and_then(Value::as_bool).unwrap_or(false);
        return SendReport { urls, files, pin_failed };
    }
    let (text, pin_failed) = body.split_once("\nPINFAIL").map(|(ok, _)| (ok.to_string(), true)).unwrap_or((body.to_string(), false));
    let urls = if text.starts_with("http") || text.starts_with("tg:") {
        text.lines().map(str::to_string).filter(|line| !line.is_empty()).collect()
    } else {
        Vec::new()
    };
    SendReport { urls, files: Vec::new(), pin_failed }
}

pub(super) fn remember_sent_files(app: &mut PhotocraftApp, fresh: &[Value]) {
    if fresh.is_empty() {
        return;
    }
    let mut kept = fresh.to_vec();
    kept.truncate(10);
    for old in app.telegram.sent_files.drain(..) {
        if kept.len() >= 10 {
            break;
        }
        let id = old.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() || kept.iter().any(|row| row.get("id").and_then(Value::as_str) == Some(id)) {
            continue;
        }
        kept.push(old);
    }
    app.telegram.sent_files = kept;
    let text = settings_json(&app.telegram);
    if let Some(save) = app.services.save_telegram.as_mut() {
        let _ = save(&text);
    }
}

pub(super) fn read_file(path: &str) -> Result<(String, Vec<u8>), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        return Err("Telegram sending needs the desktop app.".to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let meta = std::fs::metadata(path).map_err(|e| read_error(path, &e.to_string()))?;
        if !meta.is_file() {
            return Err(read_error(path, "not a file"));
        }
        if meta.len() > MAX_BYTES {
            return Err("That file is larger than Telegram accepts (50 MB).".to_string());
        }
        let bytes = std::fs::read(path).map_err(|e| read_error(path, &e.to_string()))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("That file is larger than Telegram accepts (50 MB).".to_string());
        }
        Ok((file_name(path), bytes))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_error(path: &str, error: &str) -> String {
    let name = file_name(path);
    format!("Could not read {name}: {}", redact(error, ""))
}

pub(super) fn file_secs(path: &str) -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
    }
}

pub(super) fn file_name(path: &str) -> String {
    let raw = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let name: String = raw.chars().filter(|c| !matches!(c, '"' | '\\' | '/' | '\n' | '\r')).take(180).collect();
    if name.is_empty() || name == "." || name == ".." { "file".to_string() } else { name }
}

const CAPTION_LIMIT: usize = 1024;

pub(super) fn caption(path: &str, name: &str, bytes: u64, secs: u64) -> String {
    let format = std::path::Path::new(path).extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_uppercase();
    let format = if format.is_empty() { "—".to_string() } else { format };
    let mut text = [
        crate::i18n::fmt(tl!("*File:* `{name}`"), &[("name", &markdown_code(name))]),
        crate::i18n::fmt(tl!("*Format:* {format}"), &[("format", &markdown_text(&format))]),
        crate::i18n::fmt(tl!("*Size:* {size}"), &[("size", &crate::sizing::human_bytes(bytes as f64))]),
        crate::i18n::fmt(tl!("*Time:* {time}"), &[("time", &utc_stamp(secs))]),
        crate::i18n::fmt(tl!("*Path:* `{path}`"), &[("path", &markdown_code(path))]),
    ]
    .join("\n");
    if text.chars().count() > CAPTION_LIMIT {
        text = text.chars().take(CAPTION_LIMIT).collect();
    }
    text
}

/// Text inside a Markdown code span. Backticks would end the span.
fn markdown_code(text: &str) -> String {
    text.replace('`', "'")
}

/// Markdown text outside a code span.
fn markdown_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c, '_' | '*' | '`' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `multipart/form-data` body and its `Content-Type` value.
pub fn multipart(chat_id: &str, caption: &str, filename: &str, bytes: &[u8], reply_to: Option<i64>) -> (String, Vec<u8>) {
    let boundary = boundary_for(bytes);
    let mut body = Vec::new();
    part(&mut body, &boundary, "chat_id", chat_id);
    part(&mut body, &boundary, "caption", caption);
    part(&mut body, &boundary, "parse_mode", "Markdown");
    if let Some(message_id) = reply_to {
        // The preview answers the document message, so the chat shows them as one pair.
        let reply = format!("{{\"message_id\":{message_id},\"allow_sending_without_reply\":true}}");
        part(&mut body, &boundary, "reply_parameters", &reply);
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"document\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn part(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
}

fn boundary_for(bytes: &[u8]) -> String {
    for n in 0..8u32 {
        let boundary = format!("PcTelegram{n:08x}");
        if !bytes.windows(boundary.len()).any(|w| w == boundary.as_bytes()) {
            return boundary;
        }
    }
    format!("PcTelegram{}", bytes.len())
}

pub fn validate_token(token: &str) -> Result<(), String> {
    let Some((id, secret)) = token.split_once(':') else {
        return Err("The bot token should look like 123456789:ABC…".to_string());
    };
    if (6..=12).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_digit())
        && (20..=80).contains(&secret.len())
        && secret.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !token.contains('/')
    {
        return Ok(());
    }
    Err("The bot token should look like 123456789:ABC…".to_string())
}

/// One id, or several separated by commas, semicolons or spaces.
pub fn parse_chats(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for part in text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        validate_chat(part)?;
        if !out.iter().any(|have: &String| have == part) {
            out.push(part.to_string());
        }
    }
    if out.is_empty() { Err("Enter a chat id (a number) or a public @channelname.".to_string()) } else { Ok(out) }
}

pub(super) fn chat_selected(current: &str, id: &str) -> bool {
    current.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).any(|part| part.trim() == id)
}

pub(super) fn append_chat(current: &str, id: &str) -> String {
    if chat_selected(current, id) {
        current.to_string()
    } else if current.trim().is_empty() {
        id.to_string()
    } else {
        format!("{}, {id}", current.trim().trim_end_matches([',', ';']))
    }
}

pub(super) fn png_preview_for(app: &mut PhotocraftApp, path: &str, source: Option<&photocraft_doc::Document>) -> Result<Option<String>, String> {
    if !app.telegram.also_png {
        return Ok(None);
    }
    let ext = std::path::Path::new(path).extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "psd" | "psb") {
        return Ok(None);
    }
    let Some(doc) = source else { return Ok(None) };
    export_temp_for(app, doc, "png").map(Some)
}

#[allow(clippy::too_many_arguments)]
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn send_to_chats(
    send: &crate::TelegramSendFn,
    token: &str,
    chats: &[String],
    caption: &str,
    name: &str,
    bytes: &[u8],
    preview: Option<&(String, Vec<u8>, String)>,
    pin: bool,
) -> Result<String, String> {
    let mut links = Vec::new();
    let mut files = Vec::new();
    let mut pin_failed = false;
    for chat in chats {
        // Always sendDocument (or a document album). sendPhoto re-encodes as JPEG,
        // drops transparency, and rejects files over 10 MB while MAX_BYTES allows 50 MB.
        let (method, content_type, body) = if let Some((preview_name, preview_bytes, _)) = preview {
            let (content_type, body) = media_group(chat, caption, &[(name, bytes), (preview_name.as_str(), preview_bytes.as_slice())]);
            ("sendMediaGroup", content_type, body)
        } else {
            let (content_type, body) = multipart(chat, caption, name, bytes, None);
            ("sendDocument", content_type, body)
        };
        let url = format!("https://api.telegram.org/bot{token}/{method}");
        let text = match send(&url, &content_type, &body) {
            Ok(text) => text,
            Err(error) => {
                let error = redact(&error, token);
                if links.is_empty() {
                    return Err(error);
                }
                return Err(format!("Sent to the earlier chats. The next chat failed: {error}"));
            }
        };
        files.extend(sent_files_from_reply(&text));
        if let Some(link) = message_link(&text) {
            links.push(link);
        }
        if pin {
            if let Some(message_id) = first_message_id(&text) {
                let pin_url = format!("https://api.telegram.org/bot{token}/pinChatMessage");
                let pin_body = format!("chat_id={chat}&message_id={message_id}");
                if send(&pin_url, "application/x-www-form-urlencoded", pin_body.as_bytes()).is_err() {
                    pin_failed = true;
                }
            } else {
                pin_failed = true;
            }
        }
    }
    Ok(json!({"urls": links, "files": files, "pinFailed": pin_failed}).to_string())
}

/// Album of two documents. Telegram will not mix a document with a photo.
pub fn media_group(chat_id: &str, caption: &str, files: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = format!("PcTelegramAlbum{}", files.iter().map(|(_, bytes)| bytes.len()).sum::<usize>());
    let mut media = String::from("[");
    for (index, _) in files.iter().enumerate() {
        if index > 0 {
            media.push(',');
        }
        if index == 0 {
            media.push_str(&format!(
                "{{\"type\":\"document\",\"media\":\"attach://f{index}\",\"caption\":{},\"parse_mode\":\"Markdown\"}}",
                serde_json::to_string(caption).unwrap_or_else(|_| "\"\"".to_string())
            ));
        } else {
            media.push_str(&format!("{{\"type\":\"document\",\"media\":\"attach://f{index}\"}}"));
        }
    }
    media.push(']');
    let mut body = Vec::new();
    part(&mut body, &boundary, "chat_id", chat_id);
    part(&mut body, &boundary, "media", &media);
    for (index, (name, bytes)) in files.iter().enumerate() {
        let safe = file_name(name);
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"f{index}\"; filename=\"{safe}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// The 10 newest photos and documents a bot was sent. Newest first.
pub fn files_from_updates(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(value) = serde_json::from_str::<Value>(text) else { return out };
    let Some(items) = value.get("result").and_then(Value::as_array) else { return out };
    for item in items.iter().rev() {
        let Some(message) = item.get("message").or_else(|| item.get("channel_post")) else { continue };
        let message_id = message.get("message_id").and_then(Value::as_i64).unwrap_or(0);
        if let Some(document) = message.get("document") {
            let file_id = document.get("file_id").and_then(Value::as_str).unwrap_or("");
            let size = document.get("file_size").and_then(Value::as_u64).unwrap_or(0);
            if file_id.is_empty() || size > 20 * 1024 * 1024 {
                continue;
            }
            let name = file_name(document.get("file_name").and_then(Value::as_str).unwrap_or("file"));
            out.push(json!({"id": file_id, "name": name, "label": format!("{name} ({message_id})")}));
        } else if let Some(photos) = message.get("photo").and_then(Value::as_array) {
            let photo = photos.iter().max_by_key(|p| p.get("file_size").and_then(Value::as_u64).unwrap_or(0));
            let Some(photo) = photo else { continue };
            let file_id = photo.get("file_id").and_then(Value::as_str).unwrap_or("");
            let size = photo.get("file_size").and_then(Value::as_u64).unwrap_or(0);
            if file_id.is_empty() || size > 20 * 1024 * 1024 {
                continue;
            }
            let name = format!("telegram-{message_id}.jpg");
            out.push(json!({"id": file_id, "name": name, "label": name}));
        }
        if out.len() == 10 {
            break;
        }
    }
    out
}

pub fn first_message_id(body: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(body).ok()?;
    let result = value.get("result")?;
    result
        .get("message_id")
        .and_then(Value::as_i64)
        .or_else(|| result.as_array().and_then(|items| items.first()).and_then(|item| item.get("message_id")).and_then(Value::as_i64))
}

/// `message_id` from a successful `sendDocument` reply. Absent when the body is not that shape.
pub fn sent_message_id(body: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(body).ok()?;
    value.get("result")?.get("message_id")?.as_i64()
}

/// Split a settings document into the part safe to store on disk and the bot token.
pub fn without_token(text: &str) -> Result<(String, String), String> {
    let mut value: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    let token = value.get("botToken").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(object) = value.as_object_mut() {
        object.remove("botToken");
    }
    Ok((serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string()), token))
}

/// Put `token` back into a settings document that was stored without it.
pub fn with_token(text: &str, token: &str) -> String {
    let mut value: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("botToken".into(), json!(token));
    }
    serde_json::to_string(&value).unwrap_or_else(|_| text.to_string())
}

pub fn validate_chat(chat: &str) -> Result<(), String> {
    let ok = if let Some(name) = chat.strip_prefix('@') {
        (5..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    } else {
        let digits = chat.strip_prefix('-').unwrap_or(chat);
        (1..=20).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()) && chat != "-"
    };
    if ok { Ok(()) } else { Err("Enter a chat id (a number) or a public @channelname.".to_string()) }
}

/// Drop the bot token if a transport error repeated the request URL.
pub fn redact(text: &str, token: &str) -> String {
    if token.len() < 8 { text.to_string() } else { text.replace(token, "…") }
}

/// UTC `YYYY-MM-DD HH:MM` from seconds since the Unix epoch.
pub fn utc_stamp(secs: u64) -> String {
    use chrono::Datelike;
    // Gregorian dates repeat every 400 years. Reduce to chrono's supported range.
    const CYCLE_SECONDS: u64 = 146_097 * 86_400;
    let cycles = secs / CYCLE_SECONDS;
    let remainder = secs % CYCLE_SECONDS;
    let Some(date) = chrono::DateTime::<chrono::Utc>::from_timestamp(remainder as i64, 0) else {
        return "Unknown UTC time".into();
    };
    let year = (date.year() as u64).saturating_add(cycles.saturating_mul(400));
    format!("{year:04}-{} UTC", date.format("%m-%d %H:%M"))
}
