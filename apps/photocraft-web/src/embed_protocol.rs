//! What the embedding API (#1614) agrees with its host page, kept free of browser types so it is
//! tested natively: the `?embed=` origin, the `postMessage` calls and replies, the version info,
//! and the Photopea-compatible mode (a `#{"files":…,"server":…}` config in the URL hash, and
//! Photopea's save body: https://www.photopea.com/api/).

use serde_json::{Value, json};

/// The app and `.pcraft` format versions, for a host that keeps the CLI and the web build in step
/// (a newer `.pcraft` fails to open with `FormatError::TooNew`).
pub fn info() -> Value {
    json!({ "app": env!("CARGO_PKG_VERSION"), "format": photocraft_format::FORMAT_VERSION })
}

/// The page origin allowed to drive the app over `postMessage`: `?embed=<origin>` in the query
/// (`location.search`, with or without its `?`), percent-decoded and written as browsers write
/// `MessageEvent.origin` (lower case, no default port, no trailing `/`). Only an `http(s)` origin
/// (scheme, host and optional port) counts; anything else leaves the bridge off.
pub fn allowed_origin(query: &str) -> Option<String> {
    let raw = query.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("embed="))?;
    let origin = percent_decode(raw).to_ascii_lowercase();
    let origin = origin.strip_suffix('/').unwrap_or(&origin);
    let (scheme, rest) = origin.split_once("://")?;
    let default_port = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => return None,
    };
    let host = rest.strip_suffix(default_port).unwrap_or(rest);
    let valid = !host.is_empty() && !host.contains(['/', '?', '#', '@', ' ', '\\']);
    valid.then(|| format!("{scheme}://{host}"))
}

/// A call from the host page.
#[derive(Debug, PartialEq)]
pub enum Call {
    /// Open the document in the message's `data` (an ArrayBuffer) under `name`.
    Open {
        name: String,
    },
    /// The active document's bytes in `format` (an extension: `pcraft`, `psd`, `png`…).
    Save {
        format: String,
    },
    IsDirty,
    Info,
}

/// Reads a `postMessage` call: `{photocraft: <id>, method, …}` → (id, call). The id (a number or a
/// string) comes back in the reply. Messages without a `photocraft` id aren't for the app: `None`.
/// The control channel (`window.photocraft.request`) isn't offered here: the embedding page names
/// itself in `?embed=`, so any site could iframe the app that way and drive every command.
pub fn parse_call(msg: &Value) -> Option<(Value, Result<Call, String>)> {
    let id = msg.get("photocraft").filter(|v| v.is_number() || v.is_string())?.clone();
    let str_of = |k: &str| msg.get(k).and_then(Value::as_str).map(str::to_string);
    let call = match msg.get("method").and_then(Value::as_str) {
        Some("open") => Ok(Call::Open { name: str_of("name").unwrap_or_else(|| "document".into()) }),
        Some("save") => Ok(Call::Save { format: str_of("format").unwrap_or_else(|| photocraft_format::EXTENSION.into()) }),
        Some("isDirty") => Ok(Call::IsDirty),
        Some("info") => Ok(Call::Info),
        Some("request") => Err("`request` is only for the app's own page (window.photocraft.request)".to_string()),
        Some(other) => Err(format!("unknown method `{other}`")),
        None => Err("`method` is required".to_string()),
    };
    Some((id, call))
}

/// The reply to call `id`: `{photocraft: id, ok: true, result}` or `{…, ok: false, error}`.
pub fn reply(id: &Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({ "photocraft": id, "ok": true, "result": result }),
        Err(error) => json!({ "photocraft": id, "ok": false, "error": error }),
    }
}

/// The file name the active document is exported under for `format`: its name with the format's
/// extension. `format` is an extension, letters and digits only.
pub fn export_name(doc_name: &str, format: &str) -> Result<String, String> {
    let format = format.trim().trim_start_matches('.').to_ascii_lowercase();
    if format.is_empty() || !format.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("`{format}` isn't a file format"));
    }
    let stem = doc_name.rsplit_once('.').map_or(doc_name, |(stem, _)| stem);
    let stem = if stem.trim().is_empty() { "document" } else { stem };
    Ok(format!("{stem}.{format}"))
}

/// The Photopea-compatible config in the URL hash: `files` opened at start, and saves in one of
/// `server.formats` POSTed to `server.url` instead of downloaded.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct PhotopeaConfig {
    pub files: Vec<String>,
    pub server_url: Option<String>,
    pub formats: Vec<String>,
}

impl PhotopeaConfig {
    /// Reads the URL hash (without its `#`); `None` when it is empty or isn't a JSON object.
    pub fn from_hash(hash: &str) -> Option<Self> {
        let raw = hash.trim_start_matches('#');
        if raw.is_empty() {
            return None;
        }
        let decoded = if raw.starts_with('{') { raw.to_string() } else { percent_decode(raw) };
        let v: Value = serde_json::from_str(&decoded).ok().filter(Value::is_object)?;
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        Some(Self {
            files: strings(v.get("files")),
            server_url: v.pointer("/server/url").and_then(Value::as_str).map(str::to_string),
            formats: strings(v.pointer("/server/formats")).into_iter().map(|f| f.to_lowercase()).collect(),
        })
    }

    /// Whether a save to `path` goes to the server rather than a download.
    pub fn saves(&self, path: &str) -> bool {
        self.server_url.is_some() && self.formats.iter().any(|f| *f == extension(path))
    }
}

/// The lower-case extension of `path` (empty when it has none).
pub fn extension(path: &str) -> String {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// The file name a URL points at, without its query, percent-decoded; "document" when it has none.
pub fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = percent_decode(path.rsplit('/').next().unwrap_or(path));
    if name.trim().is_empty() { "document".into() } else { name }
}

/// Photopea's save body: a JSON header `{"source","versions":[{"format","start","size"}]}` padded
/// with spaces to 2000 bytes, then the file.
pub fn save_body(source: &str, format: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    const HEADER_SIZE: usize = 2000;
    let header = json!({ "source": source, "versions": [{ "format": format, "start": 0, "size": bytes.len() }] }).to_string();
    if header.len() > HEADER_SIZE {
        return Err("the save header is longer than 2000 bytes".into());
    }
    let mut body = header.into_bytes();
    body.resize(HEADER_SIZE, b' ');
    body.extend_from_slice(bytes);
    Ok(body)
}

/// What to tell the page about a save the server answered: the text of an `app.echoToOE("…")` in
/// its JSON `script` (Photopea runs that script), else "saved" (2xx) or "error".
pub fn reply_message(ok: bool, reply: &str) -> String {
    let script = serde_json::from_str::<Value>(reply).ok().and_then(|v| v.get("script").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    if let Some(start) = script.find("echoToOE(") {
        let rest = script.get(start + "echoToOE(".len()..).unwrap_or("");
        if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'')
            && let Some(end) = rest.get(1..).and_then(|r| r.find(q))
        {
            return rest.get(1..1 + end).unwrap_or("").to_string();
        }
    }
    if ok { "saved".into() } else { "error".into() }
}

/// `%XX` escapes decoded as UTF-8 (invalid sequences kept as they are).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |j: usize| bytes.get(j).and_then(|c| (*c as char).to_digit(16));
        if b == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_gives_the_app_and_pcraft_format_versions() {
        assert_eq!(info(), json!({"app": env!("CARGO_PKG_VERSION"), "format": photocraft_format::FORMAT_VERSION}));
    }

    #[test]
    fn the_embed_origin_is_an_http_origin_or_nothing() {
        assert_eq!(allowed_origin("embed=https%3A%2F%2Fadmin.example.com"), Some("https://admin.example.com".into()));
        assert_eq!(allowed_origin("?embed=https://admin.example.com"), Some("https://admin.example.com".into()), "location.search keeps its ?");
        assert_eq!(allowed_origin("embed=https://Admin.Example.com:443/"), Some("https://admin.example.com".into()), "as MessageEvent.origin writes it");
        assert_eq!(allowed_origin("embed=http://host:8443"), Some("http://host:8443".into()));
        assert_eq!(allowed_origin("webgl&embed=http://localhost:8080"), Some("http://localhost:8080".into()));
        assert_eq!(allowed_origin("embed=https://example.com/path"), None, "an origin has no path");
        assert_eq!(allowed_origin("embed=*"), None);
        assert_eq!(allowed_origin("embed=javascript:alert(1)"), None);
        assert_eq!(allowed_origin("cpu"), None);
    }

    #[test]
    fn calls_are_read_from_messages_with_an_id() {
        let call = |v: Value| parse_call(&v).map(|(id, c)| (id, c.map_err(|_| ())));
        assert_eq!(call(json!({"type": "something else"})), None, "not for the app");
        assert_eq!(call(json!({"photocraft": 1, "method": "save", "format": "psd"})), Some((json!(1), Ok(Call::Save { format: "psd".into() }))));
        assert_eq!(call(json!({"photocraft": "a", "method": "save"})), Some((json!("a"), Ok(Call::Save { format: "pcraft".into() }))));
        assert_eq!(call(json!({"photocraft": 2, "method": "open", "name": "x.psd"})), Some((json!(2), Ok(Call::Open { name: "x.psd".into() }))));
        assert_eq!(
            call(json!({"photocraft": 3, "method": "request", "request": "ui.inspect"})),
            Some((json!(3), Err(()))),
            "no control channel over postMessage"
        );
        assert_eq!(call(json!({"photocraft": 5, "method": "rm -rf"})), Some((json!(5), Err(()))));
        assert_eq!(call(json!({"photocraft": {"x": 1}, "method": "info"})), None, "the id is a number or a string");
        assert_eq!(reply(&json!(7), Ok(json!(true))), json!({"photocraft": 7, "ok": true, "result": true}));
        assert_eq!(reply(&json!(7), Err("no".into())), json!({"photocraft": 7, "ok": false, "error": "no"}));
    }

    #[test]
    fn exports_take_the_documents_name_and_the_formats_extension() {
        assert_eq!(export_name("Poster.psd", "png"), Ok("Poster.png".into()));
        assert_eq!(export_name("Untitled-1", ".PSD"), Ok("Untitled-1.psd".into()));
        assert_eq!(export_name("", "pcraft"), Ok("document.pcraft".into()));
        assert!(export_name("a.psd", "../x").is_err());
        assert!(export_name("a.psd", "").is_err());
    }

    #[test]
    fn the_photopea_config_comes_from_the_hash() {
        let raw = r#"{"files":["https://s.test/a.psd"],"server":{"url":"https://s.test/save","formats":["PSD"]}}"#;
        let encoded: String = raw.bytes().map(|b| format!("%{b:02X}")).collect();
        let c = PhotopeaConfig::from_hash(&format!("#{encoded}")).expect("a config");
        assert_eq!(c, PhotopeaConfig::from_hash(raw).expect("a config"), "encoded or not");
        assert_eq!(c.files, ["https://s.test/a.psd"]);
        assert!(c.saves("a.psd") && !c.saves("a.png"));
        assert_eq!(PhotopeaConfig::from_hash(""), None);
        assert_eq!(PhotopeaConfig::from_hash("#section"), None);
        assert!(!PhotopeaConfig::from_hash(r#"{"files":[]}"#).expect("a config").saves("a.psd"), "no server, no upload");
    }

    #[test]
    fn photopeas_save_body_and_reply() {
        let body = save_body("https://s.test/a.psd", "psd", b"8BPS").expect("a body");
        assert_eq!(body.len(), 2004);
        let header: Value = serde_json::from_slice(body.get(..2000).unwrap_or_default().trim_ascii_end()).expect("JSON header");
        assert_eq!(header, json!({"source": "https://s.test/a.psd", "versions": [{"format": "psd", "start": 0, "size": 4}]}));
        assert_eq!(body.get(2000..), Some(&b"8BPS"[..]));
        assert!(save_body(&"x".repeat(2000), "psd", b"").is_err());
        assert_eq!(reply_message(true, r#"{"script":"app.echoToOE(\"stored 12\")"}"#), "stored 12");
        assert_eq!(reply_message(true, r#"{"script":"app.echoToOE('ok')"}"#), "ok");
        assert_eq!(reply_message(true, "not json"), "saved");
        assert_eq!(reply_message(false, ""), "error");
        assert_eq!(file_name("https://s.test/dir/My%20File.psd?v=2"), "My File.psd");
        assert_eq!(file_name("https://s.test/"), "document");
    }
}
