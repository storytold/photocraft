//! File › Save As through the XDG desktop portal on Linux (#6121, #4328, #4394).
//!
//! The GTK file chooser behind the portal keeps the typed name when the user picks another file
//! type, and rfd 0.17 reads only the file name from the portal's answer, so `image.psd` with PNG
//! picked came back as `image.psd` and was written as a PSD. The answer does say which type was
//! picked (`current_filter`), so Save As asks the portal itself and gives a name the dialog left
//! alone that type's extension ([`crate::services::with_chosen_type`]). rfd stays the fallback
//! (zenity, or no portal), and this can go once an rfd release reads the picked type
//! (PolyMeilex/rfd#341).
//!
//! File › Open asks the portal too (#14057): its `OpenFile` takes a checkbox (`choices`), which
//! rfd can't add, so the dialog itself offers Image Sequence, as Photoshop's Open dialog does.
//!
//! The dialog is attached to the window on X11. On Wayland it has no parent: naming the window to
//! the portal needs an xdg-foreign handle, which takes `unsafe` code the workspace forbids.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// One file type in the dialog: its label and extensions (`("PNG", ["png"])`).
pub type Filter = (String, Vec<String>);

/// A Save dialog to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveRequest {
    /// The portal's parent window identifier (`x11:<xid>`), or empty.
    pub parent: String,
    /// The name the dialog shows: the stem of the suggested file (`test`), see
    /// `services::dialog_file_name`.
    pub name: Option<String>,
    /// The suggested file's name (`test.psd`): a name that comes back with its extension still
    /// takes the extension of the type picked ([`crate::services::with_chosen_type`]).
    pub suggested: Option<String>,
    pub folder: Option<PathBuf>,
    pub filters: Vec<Filter>,
    /// The filter the dialog starts on.
    pub current: Option<usize>,
}

/// What the user chose: the path, as typed, and the label of the file type picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    pub filter: Option<String>,
}

/// File › Open's dialog to show (#14057).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenRequest {
    /// The portal's parent window identifier (`x11:<xid>`), or empty.
    pub parent: String,
    /// Where the dialog starts.
    pub folder: Option<PathBuf>,
    pub filters: Vec<Filter>,
    /// Several files may be picked.
    pub multiple: bool,
}

/// What the user picked in File › Open's dialog: the files, and whether Image Sequence was ticked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub paths: Vec<PathBuf>,
    pub image_sequence: bool,
}

/// The id of File › Open's Image Sequence checkbox among the dialog's `choices`.
const IMAGE_SEQUENCE: &str = "image_sequence";

/// The portal's parent window identifier for `window`: `x11:<xid>` on X11, empty elsewhere.
pub fn parent_id(window: Option<eframe::wgpu::rwh::RawWindowHandle>) -> String {
    use eframe::wgpu::rwh::RawWindowHandle;
    match window {
        Some(RawWindowHandle::Xlib(h)) => format!("x11:{:x}", h.window),
        Some(RawWindowHandle::Xcb(h)) => format!("x11:{:x}", h.window.get()),
        _ => String::new(),
    }
}

/// Shows the portal's Save dialog and waits for it. `Ok(None)` is Cancel; `Err` means the portal
/// couldn't show it (no portal, no FileChooser), so the caller falls back to rfd.
pub fn save_file(conn: &zbus::blocking::Connection, req: &SaveRequest) -> Result<Option<Saved>, String> {
    let (code, results) =
        ask(conn, |chooser, token| chooser.call("SaveFile", &(req.parent.as_str(), "Save As", options(req, token))).map_err(|e| e.to_string()))?;
    answer(code, &results)
}

/// Shows File › Open's dialog with the Image Sequence checkbox (`label`, off) and waits for it.
/// `Ok(None)` is Cancel; `Err` means the portal couldn't show it, so the caller falls back to rfd.
pub fn open_files(conn: &zbus::blocking::Connection, req: &OpenRequest, label: &str) -> Result<Option<Opened>, String> {
    let (code, results) =
        ask(conn, |chooser, token| chooser.call("OpenFile", &(req.parent.as_str(), "Open", open_options(req, label, token))).map_err(|e| e.to_string()))?;
    opened(code, &results)
}

/// Asks the portal's FileChooser through `call` (given the chooser and the request's handle
/// token) and waits for the dialog's answer: the response code and results.
fn ask(
    conn: &zbus::blocking::Connection,
    call: impl FnOnce(&zbus::blocking::Proxy<'_>, &str) -> Result<OwnedObjectPath, String>,
) -> Result<(u32, HashMap<String, OwnedValue>), String> {
    use futures_lite::StreamExt;
    use zbus::blocking::Proxy;
    let unique = conn.unique_name().ok_or("Portal connection has no unique name")?.as_str().trim_start_matches(':').replace('.', "_");
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let token = format!("photocraft_dialog_{}_{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
    // Listen before asking, so a quick answer isn't missed.
    let request = Proxy::new(conn, "org.freedesktop.portal.Desktop", path.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(|e| e.to_string())?;
    let chooser = Proxy::new(conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser")
        .map_err(|e| e.to_string())?;
    let handle = call(&chooser, &token)?;
    // A portal older than 0.9 ignores `handle_token` and answers on a request path of its own,
    // which it returns: listen there instead, as the Request documentation asks.
    let reply = if handle.as_str() == path {
        futures_lite::future::block_on(replies.next())
    } else {
        let request = Proxy::new(conn, "org.freedesktop.portal.Desktop", handle.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
        let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(|e| e.to_string())?;
        futures_lite::future::block_on(replies.next())
    }
    .ok_or("The file dialog closed without an answer")?;
    reply.body().deserialize().map_err(|e| e.to_string())
}

/// The `SaveFile` options for `req`.
fn options<'a>(req: &'a SaveRequest, token: &'a str) -> HashMap<&'static str, Value<'a>> {
    let mut o: HashMap<&'static str, Value<'a>> = HashMap::from([("handle_token", Value::from(token)), ("modal", Value::from(true))]);
    if let Some(name) = &req.name {
        o.insert("current_name", Value::from(name.as_str()));
    }
    if let Some(folder) = &req.folder {
        o.insert("current_folder", folder_bytes(folder));
    }
    if !req.filters.is_empty() {
        o.insert("filters", Value::from(req.filters.iter().map(wire).collect::<Vec<_>>()));
    }
    if let Some(f) = req.current.and_then(|i| req.filters.get(i)) {
        o.insert("current_filter", Value::from(wire(f)));
    }
    o
}

/// The `OpenFile` options for File › Open's dialog: its folder and file types, and the Image
/// Sequence checkbox (`label`), off each time the dialog opens.
fn open_options<'a>(req: &'a OpenRequest, label: &'a str, token: &'a str) -> HashMap<&'static str, Value<'a>> {
    let mut o: HashMap<&'static str, Value<'a>> =
        HashMap::from([("handle_token", Value::from(token)), ("modal", Value::from(true)), ("multiple", Value::from(req.multiple))]);
    if let Some(folder) = &req.folder {
        o.insert("current_folder", folder_bytes(folder));
    }
    if !req.filters.is_empty() {
        o.insert("filters", Value::from(req.filters.iter().map(wire).collect::<Vec<_>>()));
    }
    // A choice without options is a checkbox; its initial state is "false".
    let choices: Vec<(&str, &str, Vec<(&str, &str)>, &str)> = vec![(IMAGE_SEQUENCE, label, Vec::new(), "false")];
    o.insert("choices", Value::from(choices));
    o
}

/// A folder as the portal takes it: a byte string with its terminating NUL.
fn folder_bytes(folder: &Path) -> Value<'static> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = folder.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    Value::from(bytes)
}

/// A file type as the portal takes it: its label and `*.ext` globs.
fn wire((label, exts): &Filter) -> (String, Vec<(u32, String)>) {
    (label.clone(), exts.iter().map(|e| (0u32, format!("*.{e}"))).collect())
}

/// Reads the portal's answer to `OpenFile`: the files picked, all local, and the state of the
/// Image Sequence checkbox. Codes 1 and 2 are Cancel, as for Save ([`answer`]).
fn opened(code: u32, results: &HashMap<String, OwnedValue>) -> Result<Option<Opened>, String> {
    match code {
        0 => {}
        1 | 2 => return Ok(None),
        _ => return Err(format!("the open dialog failed (portal response {code})")),
    }
    let uris: Vec<String> = results
        .get("uris")
        .ok_or("no file in the portal's answer")?
        .try_clone()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: zbus::zvariant::Error| e.to_string())?;
    if uris.is_empty() {
        return Ok(None);
    }
    let paths = uris.iter().map(|uri| file_uri_path(uri).ok_or_else(|| format!("not a local file: {uri}"))).collect::<Result<Vec<_>, _>>()?;
    let choices: Vec<(String, String)> = results.get("choices").and_then(|v| v.try_clone().ok()?.try_into().ok()).unwrap_or_default();
    let image_sequence = choices.iter().any(|(id, value)| id == IMAGE_SEQUENCE && value == "true");
    Ok(Some(Opened { paths, image_sequence }))
}

/// Reads the portal's `Response`: code 0 with the first `uris` entry. 1 is Cancel, and 2 a dialog
/// closed some other way, such as with Esc in GNOME's: no file either, and not a reason to show
/// rfd's dialog instead (it did, so Esc opened a second Save As).
fn answer(code: u32, results: &HashMap<String, OwnedValue>) -> Result<Option<Saved>, String> {
    match code {
        0 => {}
        1 | 2 => return Ok(None),
        _ => return Err(format!("the save dialog failed (portal response {code})")),
    }
    let uris: Vec<String> = results
        .get("uris")
        .ok_or("no file in the portal's answer")?
        .try_clone()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: zbus::zvariant::Error| e.to_string())?;
    let uri = uris.first().ok_or("no file in the portal's answer")?;
    let path = file_uri_path(uri).ok_or_else(|| format!("not a local file: {uri}"))?;
    let filter = results.get("current_filter").and_then(|v| {
        let (label, _globs): (String, Vec<(u32, String)>) = v.try_clone().ok()?.try_into().ok()?;
        Some(label)
    });
    Ok(Some(Saved { path, filter }))
}

/// `file:///home/me/A%20b.psd` → `/home/me/A b.psd`; `None` for other schemes.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Only local files: `file:///…` or `file://localhost/…`.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Which file type's extensions belong on the name the dialog returned: the one it reported, or,
/// when the dialog offered only one type, that one. It reports no pick then, and the name it was
/// given is the stem (`services::dialog_file_name`), so a bare `Swatches` has to come back as
/// `Swatches.aco`. `None` when the answer names no type this dialog offered.
fn chosen_type(filters: &[Filter], picked: Option<usize>) -> Option<usize> {
    match picked {
        Some(i) if i < filters.len() => Some(i),
        None if filters.len() == 1 => Some(0),
        _ => None,
    }
}

/// Save As on Linux: the portal's dialog, with the name given the extension of the file type
/// picked. When that changes the name to one that already exists, the dialog opens again under
/// the new name, so the portal asks before replacing it (it asked only about the typed name).
pub fn save_as(conn: &zbus::blocking::Connection, mut req: SaveRequest) -> Result<Option<PathBuf>, String> {
    loop {
        let Some(saved) = save_file(conn, &req)? else { return Ok(None) };
        let picked = saved.filter.as_deref().and_then(|label| req.filters.iter().position(|(l, _)| l == label));
        // A typed extension is the user's own: `with_chosen_type` keeps one it recognises.
        let exts: &[String] = match chosen_type(&req.filters, picked) {
            Some(i) => &req.filters[i].1,
            None => &[],
        };
        let path = crate::services::with_chosen_type(&saved.path, req.suggested.as_deref(), exts);
        if path == saved.path || !path.exists() {
            return Ok(Some(path));
        }
        req.name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        req.suggested = req.name.clone();
        req.folder = path.parent().map(Path::to_path_buf);
        req.current = picked;
    }
}

#[cfg(test)]
mod tests;
