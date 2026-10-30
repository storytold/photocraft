//! DeviantArt OAuth and Sta.sh upload for File › Submit to DeviantArt.
//!
//! Only DeviantArt's authorize, token and stash submit calls are made. The refresh token is
//! stored by the caller; this module does not log it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

const REDIRECT_URI: &str = "http://127.0.0.1:43821/callback";
/// DeviantArt rejects clients that omit a User-Agent. gzip is requested by ureq's gzip feature.
const USER_AGENT: &str = "PhotoCraft";
const AUTHORIZE: &str = "https://www.deviantart.com/oauth2/authorize";
const TOKEN: &str = "https://www.deviantart.com/oauth2/token";
const SUBMIT: &str = "https://www.deviantart.com/api/v1/oauth2/stash/submit";

fn validated_scopes(scopes: &str) -> Result<String, String> {
    if scopes.len() > 256 {
        return Err("The requested DeviantArt permissions are invalid.".to_string());
    }
    let scopes: Vec<&str> = scopes.split_whitespace().collect();
    if scopes.is_empty() || scopes.iter().any(|scope| !matches!(*scope, "stash" | "gallery" | "browse" | "user" | "publish" | "user.manage")) {
        return Err("The requested DeviantArt permissions are invalid.".to_string());
    }
    Ok(scopes.join(" "))
}

/// Open the browser, wait for the loopback redirect, and return the refresh token.
pub fn connect(client_id: &str, scopes: &str) -> Result<String, String> {
    if client_id.is_empty() || !client_id.chars().all(|c| c.is_ascii_digit()) {
        return Err("The DeviantArt app id should be a number.".to_string());
    }
    let scope = validated_scopes(scopes)?;
    let verifier = verifier();
    let state = hex_token();
    let challenge = code_challenge(&verifier);
    let listener = TcpListener::bind("127.0.0.1:43821").map_err(|error| {
        if error.kind() == std::io::ErrorKind::AddrInUse {
            "Another PhotoCraft login is already using the DeviantArt callback port. Finish that login or close the other app instance, then try again."
                .to_string()
        } else {
            "Could not open the DeviantArt callback listener.".to_string()
        }
    })?;
    listener.set_nonblocking(true).map_err(|_| "listen".to_string())?;
    let url = format!(
        "{AUTHORIZE}?response_type=code&client_id={client_id}&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        encode(REDIRECT_URI),
        encode(&scope)
    );
    open::that(&url).map_err(|error| redact(&error.to_string()))?;
    let code = wait_for_oauth_code(&listener, &state, Instant::now() + Duration::from_secs(180))?;
    let body = format!(
        "grant_type=authorization_code&client_id={client_id}&code={}&redirect_uri={}&code_verifier={}",
        encode(&code),
        encode(REDIRECT_URI),
        encode(&verifier)
    );
    let text = post_form(TOKEN, &body)?;
    let refresh = refresh_of(&text).ok_or_else(|| "no-code".to_string())?;
    // This login replaces whatever the last upload rotated to.
    adopt_login(&refresh)?;
    Ok(refresh)
}

/// Refresh the login and submit one file. Returns DeviantArt's JSON body.
pub fn submit(refresh_token: &str, client_id: &str, title: &str, comments: &str, tags: &[String], filename: &str, bytes: &[u8]) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let (content_type, body) = photocraft_ui_egui::deviantart::multipart(title, comments, tags, filename, bytes);
    let response = agent(Duration::from_secs(60))
        .post(SUBMIT)
        .set("User-Agent", USER_AGENT)
        .set("Authorization", &format!("Bearer {access}"))
        .set("Content-Type", &content_type)
        .send_bytes(&body);
    interpret(response, &access)
}

/// One JSON request from the dialog. The refresh token is an argument, never a URL.
pub fn call(refresh_token: &str, client_id: &str, request: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(request).map_err(|_| "DeviantArt request was rejected".to_string())?;
    let op = value.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "publish" => {
            let item_id = value.get("itemId").and_then(|v| v.as_i64()).ok_or_else(|| "Enter the Sta.sh item id.".to_string())?;
            let mature = value.get("mature").and_then(|v| v.as_bool()).unwrap_or(false);
            let level = value.get("level").and_then(|v| v.as_str()).unwrap_or("moderate");
            let classes = string_list(value.get("classes"));
            let creative_commons = value.get("creativeCommons").and_then(|v| v.as_bool()).unwrap_or(false);
            let commercial = value.get("commercial").and_then(|v| v.as_bool()).unwrap_or(false);
            let modify = value.get("modify").and_then(|v| v.as_str()).unwrap_or("no");
            publish(refresh_token, client_id, item_id, mature, level, &classes, creative_commons, commercial, modify)
        }
        "edit" => {
            let id = value.get("deviationId").and_then(|v| v.as_str()).unwrap_or("");
            let title = value.get("title").and_then(|v| v.as_str()).unwrap_or("");
            edit_deviation(refresh_token, client_id, id, title, &string_list(value.get("tags")))
        }
        "gallery" => own_gallery(refresh_token, client_id),
        "stash" => stash_list(refresh_token, client_id),
        "stash-open" => {
            let item_id = value.get("itemId").and_then(|v| v.as_i64()).ok_or_else(|| "Enter the Sta.sh item id.".to_string())?;
            open_stash(refresh_token, client_id, item_id)
        }
        "download" => {
            let id = value.get("deviationId").and_then(|v| v.as_str()).unwrap_or("");
            let (name, bytes) = download_deviation(refresh_token, client_id, id)?;
            let path = write_download_temp(&name, &bytes)?;
            Ok(format!("OPEN\t{}", path.display()))
        }
        "avatar" => {
            let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let file = std::path::Path::new(path);
            let metadata = std::fs::symlink_metadata(file).map_err(|_| "DeviantArt address was rejected".to_string())?;
            let canonical = file.canonicalize().map_err(|_| "DeviantArt address was rejected".to_string())?;
            let temp = std::env::temp_dir().canonicalize().map_err(|_| "DeviantArt address was rejected".to_string())?;
            let parent = canonical.parent();
            let private_temp =
                parent.and_then(|parent| parent.file_name()).and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("photocraft-deviantart-"));
            if metadata.file_type().is_symlink() || !metadata.is_file() || !canonical.starts_with(temp) || !private_temp {
                return Err("DeviantArt address was rejected".to_string());
            }
            let bytes = std::fs::read(&canonical).map_err(|error| error.to_string())?;
            set_avatar(refresh_token, client_id, &file_name_of(path), &bytes)
        }
        _ => Err("DeviantArt request was rejected".to_string()),
    }
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value.and_then(|v| v.as_array()).map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn file_name_of(path: &str) -> String {
    std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("avatar.png").to_string()
}

/// Basename only, ASCII-safe, so a Hostile DeviantArt filename cannot leave temp_dir.
fn safe_download_name(name: &str) -> String {
    let normalized = name.replace('\\', "/");
    let base = std::path::Path::new(&normalized).file_name().and_then(|n| n.to_str()).unwrap_or("deviantart");
    let cleaned: String = base.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." { "deviantart".to_string() } else { cleaned }
}

fn write_download_temp(name: &str, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    let directory = tempfile::Builder::new().prefix("photocraft-deviantart-in-").tempdir().map_err(|error| error.to_string())?;
    let path = directory.path().join(safe_download_name(name));
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    let _ = directory.keep();
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
pub fn publish(
    refresh_token: &str,
    client_id: &str,
    item_id: i64,
    mature: bool,
    level: &str,
    classes: &[String],
    creative_commons: bool,
    commercial: bool,
    modify: &str,
) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let mut body = format!("itemid={item_id}&is_mature={}&allow_comments=true", if mature { "true" } else { "false" });
    if mature {
        body.push_str(&format!("&mature_level={}", encode(level)));
        for (index, class) in classes.iter().enumerate() {
            body.push_str(&format!("&mature_classification[{index}]={}", encode(class)));
        }
    }
    body.push_str(&format!(
        "&license_options[creative_commons]={}&license_options[commercial]={}&license_options[modify]={}",
        if creative_commons { "true" } else { "false" },
        if commercial { "true" } else { "false" },
        encode(modify)
    ));
    post_bearer("https://www.deviantart.com/api/v1/oauth2/stash/publish", &access, &body)
}

pub fn edit_deviation(refresh_token: &str, client_id: &str, deviation_id: &str, title: &str, tags: &[String]) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let mut body = format!("title={}", encode(title));
    for (index, tag) in tags.iter().enumerate() {
        body.push_str(&format!("&tags[{index}]={}", encode(tag)));
    }
    let url = format!("https://www.deviantart.com/api/v1/oauth2/deviation/edit/{}", encode_path(deviation_id));
    post_bearer(&url, &access, &body)
}

pub fn own_gallery(refresh_token: &str, client_id: &str) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let who = get_bearer("https://www.deviantart.com/api/v1/oauth2/user/whoami", &access)?;
    let username = serde_json::from_str::<serde_json::Value>(&who)
        .ok()
        .and_then(|v| v.get("username").and_then(|n| n.as_str()).map(str::to_string))
        .ok_or_else(|| "DeviantArt did not name the account.".to_string())?;
    get_bearer(&format!("https://www.deviantart.com/api/v1/oauth2/gallery/all?username={}&limit=24", encode(&username)), &access)
}

fn stash_list(refresh_token: &str, client_id: &str) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let text = get_bearer("https://www.deviantart.com/api/v1/oauth2/stash/delta?ext_submission=true", &access)?;
    Ok(serde_json::json!({"rows": stash_rows(&text)}).to_string())
}

fn open_stash(refresh_token: &str, client_id: &str, item_id: i64) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let text = get_bearer("https://www.deviantart.com/api/v1/oauth2/stash/delta?ext_submission=true", &access)?;
    let src = stash_src(&text, item_id).ok_or_else(|| "DeviantArt did not offer a download.".to_string())?;
    if !stash_src_allowed(&src) {
        return Err("DeviantArt address was rejected".to_string());
    }
    let bytes = download_https(&src, &access)?;
    let name = format!("stash-{item_id}.png");
    let path = write_download_temp(&name, &bytes)?;
    Ok(format!("OPEN\t{}", path.display()))
}

/// Recent Sta.sh drafts. A missing file address still keeps the row so the dialog can explain it.
pub fn stash_rows(text: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return out };
    let Some(entries) = value.get("entries").and_then(|v| v.as_array()) else { return out };
    for entry in entries {
        let Some(item_id) = entry.get("itemid").and_then(|v| v.as_i64()) else { continue };
        let title = entry
            .get("title")
            .or_else(|| entry.get("submission").and_then(|s| s.get("title")))
            .and_then(|v| v.as_str())
            .filter(|title| !title.is_empty())
            .unwrap_or("Sta.sh");
        out.push(serde_json::json!({"itemId": item_id, "title": title}));
        if out.len() == 10 {
            break;
        }
    }
    out
}

fn stash_src(text: &str, item_id: i64) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let entries = value.get("entries")?.as_array()?;
    let entry = entries.iter().find(|entry| entry.get("itemid").and_then(|v| v.as_i64()) == Some(item_id))?;
    let files = entry.get("files").or_else(|| entry.get("submission").and_then(|s| s.get("files")))?;
    files.as_array()?.iter().find_map(|file| file.get("src").and_then(|v| v.as_str()).filter(|src| stash_src_allowed(src)).map(str::to_string))
}

fn stash_src_allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, "443")) => host,
        Some(_) => return false,
        None => authority,
    };
    if host.is_empty() || !host.is_ascii() || host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return false;
    }
    let labels_are_valid = host.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|character| character.is_ascii_alphanumeric() || character == '-')
    });
    let host = host.to_ascii_lowercase();
    labels_are_valid && (host == "deviantart.com" || host.ends_with(".deviantart.com") || host.ends_with(".deviantart.net") || host.ends_with(".wixmp.com"))
}

pub fn download_deviation(refresh_token: &str, client_id: &str, deviation_id: &str) -> Result<(String, Vec<u8>), String> {
    let access = bearer(client_id, refresh_token)?;
    let info = get_bearer(&format!("https://www.deviantart.com/api/v1/oauth2/deviation/download/{}", encode_path(deviation_id)), &access)?;
    let value: serde_json::Value = serde_json::from_str(&info).map_err(|_| "DeviantArt did not offer a download.".to_string())?;
    let src = value.get("src").and_then(|v| v.as_str()).ok_or_else(|| "DeviantArt did not offer a download.".to_string())?;
    let filename = value.get("filename").and_then(|v| v.as_str()).unwrap_or("deviantart").to_string();
    if !stash_src_allowed(src) {
        return Err("DeviantArt address was rejected".to_string());
    }
    let bytes = download_https(src, &access)?;
    Ok((filename, bytes))
}

pub fn set_avatar(refresh_token: &str, client_id: &str, filename: &str, bytes: &[u8]) -> Result<String, String> {
    let access = bearer(client_id, refresh_token)?;
    let (content_type, body) = photocraft_ui_egui::deviantart::multipart("avatar", "", &[], filename, bytes);
    let response = agent(Duration::from_secs(60))
        .post("https://www.deviantart.com/api/v1/oauth2/user/profile/avatar/update")
        .set("User-Agent", USER_AGENT)
        .set("Authorization", &format!("Bearer {access}"))
        .set("Content-Type", &content_type)
        .send_bytes(&body);
    interpret(response, &access)
}

fn post_bearer(url: &str, access: &str, body: &str) -> Result<String, String> {
    if !url.starts_with("https://www.deviantart.com/api/v1/oauth2/") {
        return Err("DeviantArt address was rejected".to_string());
    }
    let response = agent(Duration::from_secs(30))
        .post(url)
        .set("User-Agent", USER_AGENT)
        .set("Authorization", &format!("Bearer {access}"))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(body);
    interpret(response, access)
}

fn get_bearer(url: &str, access: &str) -> Result<String, String> {
    if !url.starts_with("https://www.deviantart.com/api/v1/oauth2/") {
        return Err("DeviantArt address was rejected".to_string());
    }
    let response = agent(Duration::from_secs(30)).get(url).set("User-Agent", USER_AGENT).set("Authorization", &format!("Bearer {access}")).call();
    interpret(response, access)
}

fn download_https(url: &str, secret: &str) -> Result<Vec<u8>, String> {
    if !stash_src_allowed(url) {
        return Err("DeviantArt address was rejected".to_string());
    }
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .redirects(0)
        .build()
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| redact(&error.to_string()).replace(secret, "…"))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut reader = response.into_reader();
    loop {
        let n = std::io::Read::read(&mut reader, &mut chunk).map_err(|error| error.to_string())?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > 30 * 1024 * 1024 {
            return Err("That file is larger than DeviantArt accepts (30 MB).".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

fn encode_path(text: &str) -> String {
    encode(text)
}

/// One DeviantArt login for this process. The access token is reused until it is about to
/// expire, so an upload does not rotate the refresh token. When a refresh does happen, the new
/// refresh token replaces the old one before the lock is released.
struct Login {
    refresh: String,
    access: String,
    until: Instant,
    generation: u64,
    refreshing: bool,
    rejected_refresh: String,
}

fn login() -> &'static Mutex<Login> {
    static LOGIN: std::sync::OnceLock<Mutex<Login>> = std::sync::OnceLock::new();
    LOGIN.get_or_init(|| {
        Mutex::new(Login {
            refresh: String::new(),
            access: String::new(),
            until: Instant::now(),
            generation: 0,
            refreshing: false,
            rejected_refresh: String::new(),
        })
    })
}

fn login_changed() -> &'static Condvar {
    static CHANGED: std::sync::OnceLock<Condvar> = std::sync::OnceLock::new();
    CHANGED.get_or_init(Condvar::new)
}

fn login_mut() -> std::sync::MutexGuard<'static, Login> {
    login().lock().unwrap_or_else(PoisonError::into_inner)
}

/// Browser login succeeded. Drop the previous access token so the next call uses this refresh token.
pub fn adopt_login(token: &str) -> Result<(), String> {
    let mut session = login_mut();
    session.generation = session.generation.wrapping_add(1);
    session.rejected_refresh.clear();
    session.refresh = token.to_string();
    session.access.clear();
    session.until = Instant::now();
    drop(session);
    // The UI persists the token to its restricted settings-file fallback if the keychain is
    // unavailable. Never hold the login mutex during keychain I/O.
    let _ = crate::services::store_deviantart_token(token);
    login_changed().notify_all();
    Ok(())
}

/// The user disconnected, or DeviantArt rejected the refresh token.
pub fn clear_login() -> Result<(), String> {
    clear_login_with(|| crate::services::store_deviantart_token(""))
}

fn clear_login_with(clear: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let mut session = login_mut();
    session.generation = session.generation.wrapping_add(1);
    session.rejected_refresh = session.refresh.clone();
    session.refresh.clear();
    session.access.clear();
    session.until = Instant::now();
    drop(session);
    login_changed().notify_all();
    clear()
}

/// The refresh token the next save must not overwrite.
pub fn current_refresh() -> String {
    login_mut().refresh.clone()
}

/// Which refresh token to keep after DeviantArt answers a refresh.
fn kept_refresh(sent: &str, returned: Option<&str>) -> String {
    returned.filter(|token| !token.is_empty()).unwrap_or(sent).to_string()
}

fn bearer(client_id: &str, refresh_from_caller: &str) -> Result<String, String> {
    loop {
        let mut session = login_mut();
        if session.refresh.is_empty() && !refresh_from_caller.is_empty() {
            if session.rejected_refresh == refresh_from_caller {
                return Err("The DeviantArt login expired. Connect again.".to_string());
            }
            session.generation = session.generation.wrapping_add(1);
            session.refresh = refresh_from_caller.to_string();
        }
        if !session.access.is_empty() && Instant::now() + Duration::from_secs(60) < session.until {
            return Ok(session.access.clone());
        }
        if session.refresh.is_empty() {
            return Err("The DeviantArt login expired. Connect again.".to_string());
        }
        if session.refreshing {
            session = login_changed().wait(session).unwrap_or_else(PoisonError::into_inner);
            drop(session);
            continue;
        }
        session.refreshing = true;
        let generation = session.generation;
        let refresh = session.refresh.clone();
        drop(session);

        let result = refresh_request(client_id, &refresh);
        let mut session = login_mut();
        session.refreshing = false;
        if session.generation != generation || session.refresh != refresh {
            drop(session);
            login_changed().notify_all();
            return Err("The DeviantArt login changed. Connect again.".to_string());
        }
        match result {
            Ok((access, returned, seconds)) => {
                let next = kept_refresh(&refresh, returned.as_deref());
                session.refresh = next.clone();
                session.access = access.clone();
                let lifetime = seconds.saturating_sub(60).max(30);
                session.until = Instant::now() + Duration::from_secs(lifetime);
                drop(session);
                let _ = crate::services::store_deviantart_token(&next);
                login_changed().notify_all();
                return Ok(access);
            }
            Err(error) => {
                let clear_stored_token = if login_rejected(&error) {
                    session.generation = session.generation.wrapping_add(1);
                    session.rejected_refresh = session.refresh.clone();
                    session.refresh.clear();
                    session.access.clear();
                    true
                } else {
                    false
                };
                drop(session);
                if clear_stored_token {
                    let _ = crate::services::store_deviantart_token("");
                }
                login_changed().notify_all();
                return Err(if clear_stored_token { format!("refresh-grant-rejected: {error}") } else { error });
            }
        }
    }
}

fn login_rejected(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("invalid_grant") || lower.contains("invalid_token") || lower.contains("refresh token") || lower.contains("refresh_token")
}

fn refresh_request(client_id: &str, refresh_token: &str) -> Result<(String, Option<String>, u64), String> {
    let body = format!("grant_type=refresh_token&client_id={client_id}&refresh_token={}", encode(refresh_token));
    let text = post_form(TOKEN, &body)?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| "no-code".to_string())?;
    let access = value.get("access_token").and_then(|v| v.as_str()).ok_or_else(|| "no-code".to_string())?.to_string();
    let refresh = value.get("refresh_token").and_then(|v| v.as_str()).map(str::to_string);
    let seconds = value.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
    Ok((access, refresh, seconds))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

fn post_form(url: &str, body: &str) -> Result<String, String> {
    if url != TOKEN {
        return Err("DeviantArt address was rejected".to_string());
    }
    let response =
        agent(Duration::from_secs(30)).post(url).set("User-Agent", USER_AGENT).set("Content-Type", "application/x-www-form-urlencoded").send_string(body);
    interpret(response, body)
}

fn interpret(response: Result<ureq::Response, ureq::Error>, secret: &str) -> Result<String, String> {
    match response {
        Ok(response) => {
            let status = response.status();
            let text = response.into_string().unwrap_or_default();
            finish(status, &text, secret)
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            finish(status, &text, secret)
        }
        Err(error) => Err(redact(&error.to_string())),
    }
}

fn finish(status: u16, text: &str, secret: &str) -> Result<String, String> {
    if (200..300).contains(&status) {
        return Ok(redact(text));
    }
    Err(redact(&oauth_error(status, text)).replace(secret, "…"))
}

/// Keep DeviantArt's error code. `invalid_request` still means a dead refresh token when the
/// description says so, and the code alone is enough when the description is missing.
fn oauth_error(status: u16, text: &str) -> String {
    let value = serde_json::from_str::<serde_json::Value>(text).ok();
    let code = value.as_ref().and_then(|v| v.get("error")).and_then(|v| v.as_str()).unwrap_or("");
    let description = value.as_ref().and_then(|v| v.get("error_description")).and_then(|v| v.as_str()).unwrap_or("");
    if code.is_empty() && description.is_empty() {
        format!("HTTP {status}")
    } else if description.is_empty() {
        code.to_string()
    } else if code.is_empty() {
        description.to_string()
    } else {
        format!("{code}: {description}")
    }
}

fn refresh_of(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value.get("refresh_token").and_then(|v| v.as_str()).map(str::to_string)
}

fn accept_for(listener: &TcpListener, limit: Duration) -> Result<TcpStream, String> {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).map_err(|_| "listen".to_string())?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > limit {
                    return Err("timeout".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return Err("listen".to_string()),
        }
    }
}

/// Keep accepting until `/callback` with the expected state arrives, or the deadline passes.
fn wait_for_oauth_code(listener: &TcpListener, expected_state: &str, deadline: Instant) -> Result<String, String> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timeout".to_string());
        }
        // accept_for can time out every few seconds; that is not the overall deadline.
        let mut stream = match accept_for(listener, remaining.min(Duration::from_secs(5))) {
            Ok(stream) => stream,
            Err(error) if error == "timeout" => continue,
            Err(error) => return Err(error),
        };
        let head = match read_head(&mut stream) {
            Ok(head) => head,
            Err(_) => {
                let _ = write_page(&mut stream, false);
                continue;
            }
        };
        if request_path(&head).as_deref() != Some("/callback") {
            let _ = write_page(&mut stream, false);
            continue;
        }
        let Some(query) = request_query(&head) else {
            let _ = write_page(&mut stream, false);
            continue;
        };
        if query_value(&query, "error").is_some() {
            let _ = write_page(&mut stream, false);
            return Err("denied".to_string());
        }
        if query_value(&query, "state").unwrap_or_default() != expected_state {
            let _ = write_page(&mut stream, false);
            continue;
        }
        let Some(code) = query_value(&query, "code").filter(|code| !code.is_empty()) else {
            let _ = write_page(&mut stream, false);
            continue;
        };
        let _ = write_page(&mut stream, true);
        return Ok(code);
    }
}

fn read_head(stream: &mut TcpStream) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if buf.len() >= 8192 {
            return Err("no-code".to_string());
        }
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return Err("timeout".to_string()),
        }
    }
    String::from_utf8(buf).map_err(|_| "no-code".to_string())
}

fn write_page(stream: &mut TcpStream, ok: bool) -> Result<(), String> {
    let body = if ok {
        "PhotoCraft ist verbunden. Du kannst dieses Fenster schließen.\nPhotoCraft is connected. You can close this window."
    } else {
        "PhotoCraft konnte nicht verbinden. Du kannst dieses Fenster schließen.\nPhotoCraft could not connect. You can close this window."
    };
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).map_err(|error| error.to_string())?;
    stream.write_all(body.as_bytes()).map_err(|error| error.to_string())
}

fn request_path(head: &str) -> Option<String> {
    let line = head.lines().next()?;
    let path = line.split_whitespace().nth(1)?;
    Some(path.split('?').next().unwrap_or(path).to_string())
}

fn request_query(head: &str) -> Option<String> {
    let line = head.lines().next()?;
    let path = line.split_whitespace().nth(1)?;
    path.split_once('?').map(|(_, query)| query.to_string())
}

fn query_value(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else { continue };
        if name == key {
            return Some(decode(value));
        }
    }
    None
}

fn verifier() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut bytes = [0u8; 64];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    bytes.iter().map(|b| CHARS[(*b as usize) % CHARS.len()] as char).collect()
}

fn hex_token() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn encode(text: &str) -> String {
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

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn redact(text: &str) -> String {
    text.replace(REDIRECT_URI, "…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_rows_keep_the_title_and_only_deviantart_files_download() {
        let text = r#"{"entries":[{"itemid":7,"submission":{"title":"Hill","files":[{"src":"https://images-wixmp.wixmp.com/hill.png"}]}},{"itemid":8,"title":"Plain"}]}"#;
        let rows = stash_rows(text);
        assert_eq!(rows[0].get("title").and_then(|v| v.as_str()), Some("Hill"));
        assert_eq!(stash_src(text, 7).as_deref(), Some("https://images-wixmp.wixmp.com/hill.png"));
        assert!(stash_src(r#"{"entries":[{"itemid":7,"files":[{"src":"https://evil.example/x"}]}]}"#, 7).is_none());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn deviantart_clear_login_discards_live_tokens_when_keychain_fails() {
        {
            let mut session = login_mut();
            session.refresh = "old-refresh".into();
            session.access = "old-access".into();
        }
        assert!(clear_login_with(|| Err("keychain unavailable".into())).is_err());
        let session = login_mut();
        assert!(session.refresh.is_empty());
        assert!(session.access.is_empty());
    }

    #[test]
    fn a_rotated_refresh_token_replaces_the_one_that_was_sent() {
        assert_eq!(kept_refresh("old", Some("new")), "new");
        assert_eq!(kept_refresh("old", None), "old");
        assert_eq!(kept_refresh("old", Some("")), "old");
    }

    #[test]
    fn oauth_error_keeps_the_code_and_the_description() {
        let text = r#"{"error":"invalid_request","error_description":"The refresh token is invalid."}"#;
        assert_eq!(oauth_error(400, text), "invalid_request: The refresh token is invalid.");
    }

    #[test]
    fn operation_scopes_are_validated_without_starting_a_login() {
        for scope in ["stash", "stash publish", "stash browse user", "stash browse", "stash browse user.manage"] {
            assert_eq!(validated_scopes(scope).unwrap(), scope);
        }
        for scope in ["", "admin", "stash user.admin", "stash,publish"] {
            assert!(validated_scopes(scope).is_err());
        }
        assert!(validated_scopes(&" ".repeat(257)).is_err());
    }

    #[test]
    fn pkce_matches_the_published_vector() {
        assert_eq!(code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn callback_query_reads_the_code() {
        let head = "GET /callback?code=abc%2F1&state=ss HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let query = request_query(head).unwrap();
        assert_eq!(query_value(&query, "code").as_deref(), Some("abc/1"));
        assert_eq!(query_value(&query, "state").as_deref(), Some("ss"));
        assert_eq!(request_path(head).as_deref(), Some("/callback"));
        assert_eq!(request_path("GET / HTTP/1.1\r\n\r\n").as_deref(), Some("/"));
    }

    #[test]
    fn download_names_stay_inside_temp() {
        assert_eq!(safe_download_name("../../evil.png"), "evil.png");
        assert_eq!(safe_download_name(r"..\..\evil.png"), "evil.png");
        assert_eq!(safe_download_name("my \"best\".png"), "mybest.png");
        assert_eq!(safe_download_name(".."), "deviantart");
        assert_eq!(safe_download_name(""), "deviantart");
    }
    #[test]
    fn avatar_rejects_a_file_outside_the_private_export_directory() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let request = serde_json::json!({"op":"avatar", "path":file.path()}).to_string();
        assert_eq!(call("", "", &request), Err("DeviantArt address was rejected".into()));
    }

    #[cfg(unix)]
    #[test]
    fn avatar_rejects_a_symlink_inside_the_private_export_directory() {
        let dir = tempfile::Builder::new().prefix("photocraft-deviantart-").tempdir().unwrap();
        let target = dir.path().join("image.png");
        std::fs::write(&target, b"test").unwrap();
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let request = serde_json::json!({"op":"avatar", "path":link}).to_string();
        assert_eq!(call("", "", &request), Err("DeviantArt address was rejected".into()));
    }
}
