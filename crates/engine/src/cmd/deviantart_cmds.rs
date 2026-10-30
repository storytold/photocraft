//! Private Sta.sh submission through the shared engine dispatcher.
//!
//! The host supplies an authenticated transport, as it supplies a print spooler. Credentials
//! stay in that transport and are never command parameters or journal entries. Preparation,
//! validation and upload dispatch are shared by UI, control, CLI and MCP callers.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
use std::sync::Arc;

pub const COMMAND: &str = "file.submitDeviantArt";
pub const MAX_BYTES: u64 = 30 * 1024 * 1024;

/// One validated private upload. The transport must not log credentials or payload bytes.
pub struct Submission {
    pub name: String,
    pub title: String,
    pub comments: String,
    pub tags: Vec<String>,
    pub bytes: Vec<u8>,
}
/// An authenticated host transport; returns DeviantArt's response body or an explicit error.
pub type SubmitFn = Arc<dyn Fn(&Submission) -> std::result::Result<String, String> + Send + Sync>;

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}
fn unavailable() -> EngineError {
    EngineError::Other("DeviantArt submission is unavailable: use a supported desktop host with a connected account".into())
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") || session.deviantart_submit.is_none() { Err(unavailable().to_string()) } else { Ok(()) }
}
fn text<'a>(params: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    match params.get(key) {
        None => Ok(default),
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err(bad(format!("{key} must be a string"))),
    }
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let object = params.as_object().ok_or_else(|| bad("expected an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "path" | "format" | "title" | "tags" | "coalesce") {
            return Err(bad(format!("unknown parameter {key}")));
        }
    }
    let format = text(params, "format", "saved")?;
    if !matches!(format, "saved" | "png" | "psd") {
        return Err(bad("format must be saved, png or psd"));
    }
    let title = text(params, "title", "")?;
    let tags = text(params, "tags", "")?;
    if title.chars().take(201).count() > 200 || tags.len() > 4096 {
        return Err(bad("title exceeds 200 characters or tags exceed 4096 bytes"));
    }
    let path = text(params, "path", "")?;
    if path.len() > 32_768 || (format != "saved" && !path.is_empty()) {
        return Err(bad("path is only accepted for saved files (at most 32768 bytes)"));
    }
    let submit = session.deviantart_submit.clone().ok_or_else(unavailable)?;
    let (name, bytes, warnings) = prepare(session, format, path)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(bad("Sta.sh submissions must not exceed 30 MiB"));
    }
    let title = if title.is_empty() { stem(&name) } else { title.to_string() };
    let comments = comments(&name, bytes.len() as u64);
    let payload = Submission { name: name.clone(), title, comments, tags: clean_tags(tags), bytes };
    let body = submit(&payload).map_err(EngineError::Other)?;
    let response: Value = serde_json::from_str(&body).map_err(|_| EngineError::Other("DeviantArt returned an invalid submission response".into()))?;
    let item_id = response
        .get("itemid")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .ok_or_else(|| EngineError::Other("DeviantArt did not confirm a private Sta.sh item".into()))?;
    Ok(json!({"name": name, "itemId":item_id, "body": body, "warnings": warnings}))
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare(session: &Session, format: &str, path: &str) -> Result<(String, Vec<u8>, Vec<String>)> {
    use std::io::Read as _;
    if format == "saved" {
        let path = if path.is_empty() {
            session.active().and_then(|state| state.path.as_deref()).ok_or_else(|| bad("provide path or save the active document first"))?
        } else {
            path
        };
        // Reject non-files before opening: opening a FIFO for reading could otherwise wait
        // indefinitely for a writer. Re-check the opened handle below before reading.
        let before = std::fs::metadata(path).map_err(|error| EngineError::Other(error.to_string()))?;
        if !before.is_file() || before.len() > MAX_BYTES {
            return Err(bad("path must name a regular file no larger than 30 MiB"));
        }
        let file = open_regular_file(std::path::Path::new(path))?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|error| EngineError::Other(error.to_string()))?;
        let name = std::path::Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("file").to_string();
        return Ok((name, bytes, Vec::new()));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let name = format!("{}.{}", stem(&state.doc.name), format);
    let opts = photocraft_io::ExportOptions {
        xmp: photocraft_io::XmpEmbed::None,
        document_path: state.path.as_ref().map(std::path::PathBuf::from),
        // Match ordinary engine exports: trusted desktop saves embed linked content;
        // an untrusted session must not read the paths stored in a document.
        read_linked_files: !crate::jobs::authorization_installed(),
        ..Default::default()
    };
    let exported = photocraft_io::export(&state.doc, &name, &opts).map_err(|error| EngineError::Other(error.to_string()))?;
    Ok((name, exported.bytes, exported.warnings))
}
#[cfg(not(target_arch = "wasm32"))]
fn open_regular_file(path: &std::path::Path) -> Result<std::fs::File> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, open};
        // Metadata checked before open can race with replacement by a FIFO.
        let fd = open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(|error| EngineError::Other(error.to_string()))?;
        std::fs::File::from(fd)
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path).map_err(|error| EngineError::Other(error.to_string()))?;
    let meta = file.metadata().map_err(|error| EngineError::Other(error.to_string()))?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err(bad("path must name a regular file no larger than 30 MiB"));
    }
    Ok(file)
}
#[cfg(target_arch = "wasm32")]
fn prepare(_: &Session, _: &str, _: &str) -> Result<(String, Vec<u8>, Vec<String>)> {
    Err(unavailable())
}

/// Upload metadata deliberately contains only the file name, never its local path.
pub fn comments(name: &str, bytes: u64) -> String {
    let ext = std::path::Path::new(name).extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_uppercase();
    format!("File: {name}\nFormat: {ext}\nSize: {bytes} bytes")
}

pub fn stem(name: &str) -> String {
    let name = std::path::Path::new(name).file_stem().and_then(|name| name.to_str()).unwrap_or(name);
    let name: String = name.chars().take(50).collect();
    if name.is_empty() { "PhotoCraft".into() } else { name }
}
/// Normalize UI and agent tag input identically, without silently sending duplicate tags.
pub fn clean_tags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let tag: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(64).collect();
        if tag.is_empty() || out.iter().any(|have: &String| have.eq_ignore_ascii_case(&tag)) {
            continue;
        }
        out.push(tag);
        if out.len() == 30 {
            break;
        }
    }
    out
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Submit to DeviantArt…",
        menu: &["File"],
        shortcut: None,
        params: r#"{"format":"saved|png|psd"="saved","path":string? (saved only; otherwise active saved file),"title":string? (max 200 characters; defaults to file stem),"tags":string? (comma/whitespace separated; max 4096 bytes, 30 normalized tags)} → {name,itemId,body:Sta.sh response,warnings:[string]}. Private upload through the host's authenticated transport. PNG/PSD export the active document in memory; max upload 30 MiB. Unsupported hosts/platforms return an error. No automatic submission or public publishing."#,
        enabled,
        run,
        journal: false,
    }]
}

#[cfg(test)]
mod tests;
