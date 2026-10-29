//! Capability-based filesystem policy for untrusted automation paths.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use photocraft_format::atomic::RenameRetry;
use serde_json::Value;

use crate::AutomationError;

const DENIED: &str = "automation filesystem access is not granted";

#[derive(Clone)]
struct RootCapability {
    dir: Arc<Dir>,
    /// The root as an absolute path (trusted launch configuration), to recognise absolute request
    /// paths that name a file beneath it.
    path: PathBuf,
}

/// Separate directory capabilities for automation reads and writes.
///
/// Root paths are trusted launch-time configuration. Request paths are always
/// untrusted, forward-slash relative paths resolved by `cap-std` beneath the
/// held directory handle, or absolute paths whose text lies beneath the root
/// (translated to relative ones, see [`request_path`]). Parent traversal,
/// alternate separators, Windows prefixes and empty components are rejected
/// before I/O.
#[derive(Clone, Default)]
pub struct AuthorizedWorkspace {
    read: Option<RootCapability>,
    write: Option<RootCapability>,
}

impl AuthorizedWorkspace {
    /// Open the configured roots as capabilities. Either authority may be
    /// omitted; omitted authority fails closed.
    pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, AutomationError> {
        Ok(Self { read: open_root(read_root, "read")?, write: open_root(write_root, "write")? })
    }

    /// Read one regular file below the configured read root.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, AutomationError> {
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let mut file = root.dir.open(&relative).map_err(|e| file_error("read", path, e))?;
        let metadata = file.metadata().map_err(|e| file_error("read", path, e))?;
        if !metadata.is_file() {
            return Err(AutomationError::BadRequest(format!("automation read path is not a regular file: `{path}`")));
        }
        read_to_observed_length(&mut file, metadata.len())
    }

    /// Load one `.pcraft` document in directory form — the native on-disk layout — below the
    /// read root. `read` serves regular files only (the zip form), so a regular file at the
    /// path is reported as `Ok(None)` for the caller to fall back to.
    pub fn open_pcraft(&self, path: &str) -> Result<Option<photocraft_doc::Document>, AutomationError> {
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let dir = match root.dir.open_dir(&relative) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return Ok(None),
            Err(e) => return Err(file_error("read", path, e)),
        };
        let source = DirHandle { dir };
        Ok(Some(photocraft_format::load_bytes_source(&source, &photocraft_format::LoadOptions::default())?))
    }

    /// Create or replace one file below the configured write root, crash-safely: the bytes go to
    /// a temporary file beside the target, which is synced and renamed over it (the same steps
    /// as [`photocraft_format::atomic_write`], through the directory capability). On failure
    /// the previous file is untouched and the temporary file is removed.
    ///
    /// Replacing a file carries its extended attributes — macOS Finder tags, Linux `user.*` —
    /// onto the file that takes its place (#7095), through the two open descriptors rather than
    /// their paths, so nothing here reaches outside the write root to finish a write. The rest
    /// of what [`photocraft_format::atomic_write`] preserves does not follow: its permission
    /// step is tracked in #4963, and the Windows `ReplaceFileW` swap is path-based, which this
    /// path may not use.
    ///
    /// The parent directory must already exist. `cap-std` performs path
    /// resolution and file creation relative to the held directory handle, so
    /// a non-existent final target is supported without ambient path access.
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<(), AutomationError> {
        let relative = request_path(path, self.write.as_ref())?;
        let root = self.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        let leaf = relative.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // The bytes are an image or document whatever the name says (`format` picks them), so
        // only a file of a kind PhotoCraft writes is replaced: never source, settings or notes
        // that share the root (`src/main.rs`, `.env`; #9439). A new file may have any name, and
        // an image saved under one (`out` with `format: png`) can be saved over again.
        let exists = root.dir.symlink_metadata(&relative).is_ok();
        // Only a regular file is ever opened below (its head here, its attributes on Unix): a
        // FIFO would block that open until a writer appears, and with it every other request.
        // Links are followed for this check; a dangling one falls through to the name rule.
        if exists && root.dir.metadata(&relative).is_ok_and(|m| !m.is_file()) {
            return Err(AutomationError::BadRequest(format!("automation write refused: `{path}` already exists and is not a regular file")));
        }
        if exists && !written_kind(&leaf) && !holds_image(&root.dir, &relative) {
            return Err(AutomationError::BadRequest(format!(
                "automation write refused: `{path}` already exists and is not an image or document file PhotoCraft writes; save to a new name"
            )));
        }
        let parent = relative.parent().map(Path::to_path_buf).unwrap_or_default();
        let tmp = parent.join(photocraft_format::atomic::temp_name(&leaf));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let written = root.dir.open_with(&tmp, &options).and_then(|file| {
            let mut file = file.into_std();
            file.write_all(bytes)?;
            #[cfg(unix)]
            {
                use std::os::fd::AsFd;
                // Best effort, and only when there is a file being replaced: a save still
                // succeeds for a document whose Finder tag could not come along.
                if let Ok(old) = root.dir.open(&relative) {
                    let _ = photocraft_format::atomic::meta::copy_xattrs(old.as_fd(), file.as_fd());
                }
            }
            // Falls back to a plain fsync on shares that refuse a full flush (#1336).
            photocraft_format::atomic::sync_file(&file)
        });
        let renamed = written.and_then(|()| photocraft_format::atomic::retry_rename(RenameRetry::platform(), || root.dir.rename(&tmp, &root.dir, &relative)));
        if let Err(e) = renamed {
            let _ = root.dir.remove_file(&tmp);
            return Err(file_error("write", path, e));
        }
        // Flush the directory entry (Unix); best effort, the new bytes are already in place.
        #[cfg(unix)]
        {
            let dir = if parent.as_os_str().is_empty() { PathBuf::from(".") } else { parent };
            if let Ok(d) = root.dir.open(&dir) {
                let _ = photocraft_format::atomic::sync_file(&d.into_std());
            }
        }
        Ok(())
    }
}

/// Whether `name`'s extension is one of the formats PhotoCraft saves or exports.
fn written_kind(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    let ext = ext.to_ascii_lowercase();
    ["psd", "psb", "ora", "pdf", photocraft_format::EXTENSION].contains(&ext.as_str()) || photocraft_codecs::from_extension(&ext).is_some()
}

/// Whether the file at `relative` starts like an image PhotoCraft reads (PSD/PSB or a codec's
/// magic). Containers that also hold other things (zip, PDF) are judged by name only.
fn holds_image(dir: &Dir, relative: &Path) -> bool {
    let mut head = [0u8; 64];
    let Ok(mut file) = dir.open(relative) else { return false };
    let Ok(n) = file.read(&mut head) else { return false };
    let head = &head[..n];
    head.starts_with(b"8BPS") || photocraft_codecs::detect(head).is_some()
}

fn read_to_observed_length(reader: &mut impl Read, observed_length: u64) -> Result<Vec<u8>, AutomationError> {
    // Do not reserve from a potentially stale or attacker-controlled metadata length. Read in
    // chunks, and reject growth beyond the length observed on the opened file before appending.
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 << 10];
    loop {
        let remaining = observed_length.saturating_sub(bytes.len() as u64);
        let ask = chunk.len().min(usize::try_from(remaining.saturating_add(1)).unwrap_or(chunk.len()));
        let n = match reader.read(&mut chunk[..ask]) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(AutomationError::Io(format!("automation read failed: {e}"))),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if n as u64 > remaining {
            return Err(AutomationError::BadRequest(format!("automation file grew beyond its observed length of {observed_length} bytes")));
        }
        let needed = bytes.len().checked_add(n).ok_or_else(|| AutomationError::BadRequest("automation file is too large to read".into()))?;
        if needed > bytes.capacity() {
            let target = bytes.capacity().saturating_add(bytes.capacity() / 2).max(needed).min(usize::try_from(observed_length).unwrap_or(usize::MAX));
            bytes.try_reserve_exact(target - bytes.len()).map_err(|e| AutomationError::Io(format!("automation read allocation failed: {e}")))?;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

fn open_root(path: Option<&Path>, authority: &str) -> Result<Option<RootCapability>, AutomationError> {
    let Some(path) = path else { return Ok(None) };
    if path.as_os_str().is_empty() {
        return Err(AutomationError::BadRequest(format!("automation {authority} root is empty")));
    }
    let dir = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|e| AutomationError::Io(format!("cannot open automation {authority} root `{}`: {e}", path.display())))?;
    // Textual (no link resolution), like the request paths it is compared with.
    let absolute =
        std::path::absolute(path).map_err(|e| AutomationError::Io(format!("cannot resolve automation {authority} root `{}`: {e}", path.display())))?;
    Ok(Some(RootCapability { dir: Arc::new(dir), path: absolute }))
}

/// A request path as the relative path the root's capability resolves. An absolute path is
/// accepted only when its text names a file beneath `root`: the root's components are
/// stripped (the drive letter compared without case) and the rest must pass
/// [`relative_path`], so `..`, prefixes and device names are refused as before. Nothing is
/// resolved on the filesystem; the held directory handle still does all the opening, so
/// links cannot escape the root.
fn request_path(raw: &str, root: Option<&RootCapability>) -> Result<PathBuf, AutomationError> {
    let Some(root) = root.filter(|_| Path::new(raw).is_absolute()) else { return relative_path(raw) };
    let mut request = Path::new(raw).components();
    for want in root.path.components() {
        let same = match (request.next(), want) {
            (Some(Component::Prefix(a)), Component::Prefix(b)) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
            (Some(got), want) => got == want,
            (None, _) => false,
        };
        if !same {
            return Err(path_error(raw, "absolute paths must be inside the automation root"));
        }
    }
    let rest: Vec<&str> = request.map(|c| c.as_os_str().to_str().unwrap_or("\u{FFFD}")).collect();
    relative_path(&rest.join("/"))
}

fn relative_path(raw: &str) -> Result<PathBuf, AutomationError> {
    if raw.is_empty() {
        return Err(path_error(raw, "path is empty"));
    }
    if raw.contains('\\') {
        return Err(path_error(raw, "alternate separators are not allowed; use `/`"));
    }
    if raw.starts_with('/') {
        return Err(path_error(raw, "absolute paths are not allowed"));
    }
    if raw.contains(':') {
        return Err(path_error(raw, "drive, device and stream prefixes are not allowed"));
    }
    for component in raw.split('/') {
        if component.is_empty() {
            return Err(path_error(raw, "empty path components are not allowed"));
        }
        if component == "." {
            return Err(path_error(raw, "`.` path components are not allowed"));
        }
        if component == ".." {
            return Err(path_error(raw, "parent traversal is not allowed"));
        }
        if component.ends_with(['.', ' ']) {
            return Err(path_error(raw, "path components ending in a dot or space are not allowed"));
        }
        if is_windows_device_name(component) {
            return Err(path_error(raw, "reserved device names are not allowed"));
        }
    }
    Ok(PathBuf::from(raw))
}

/// At least every name cap-std refuses to open on Windows (`COM0`–`COM9`, `COM¹`–`COM³`, the same
/// for `LPT`, spaces before the extension ignored), so nothing written is unreadable (#9424).
fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end().to_ascii_uppercase();
    let port = |n: &str| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³");
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
        || stem.strip_prefix("COM").is_some_and(port)
        || stem.strip_prefix("LPT").is_some_and(port)
}

fn path_error(path: &str, reason: &str) -> AutomationError {
    AutomationError::BadRequest(format!("automation path rejected: {reason}: `{path}`"))
}

/// A `.pcraft` directory behind the read-root capability: bounded reads, no ambient paths.
struct DirHandle {
    dir: Dir,
}

impl photocraft_format::ByteSource for DirHandle {
    fn get(&self, path: &str, max: usize) -> photocraft_format::Result<Vec<u8>> {
        let missing = || photocraft_format::FormatError::Corrupt(format!("missing `{path}`"));
        let mut file = self.dir.open(path).map_err(|_| missing())?;
        let metadata = file.metadata().map_err(|_| missing())?;
        if !metadata.is_file() {
            // A FIFO or device would block the open and stream without end.
            return Err(photocraft_format::FormatError::Corrupt(format!("`{path}` is not a regular file")));
        }
        photocraft_format::read_entry(&mut file, metadata.len(), max, path)
    }
}

fn file_error(operation: &str, path: &str, error: std::io::Error) -> AutomationError {
    let kind = error.kind();
    AutomationError::Io(format!("automation {operation} `{path}` failed ({kind:?}): {error}"))
}

/// Filesystem-bearing engine commands have not yet been converted to consume
/// directory capabilities. Automation must use `doc.open`, `doc.save` and
/// `doc.render` for file effects until those commands are migrated.
pub fn authorize_engine_command(id: &str, params: &Value) -> Result<(), AutomationError> {
    if photocraft_engine::untrusted::reaches_outside(id, params) { Err(command_error(id)) } else { Ok(()) }
}

/// Desktop sessions may already contain user-configured ambient colour-profile
/// paths. Commands which implicitly resolve those settings are denied even
/// when their request parameters contain no path. Fresh headless sessions
/// cannot acquire such settings through the automation interface.
pub fn authorize_desktop_engine_command(id: &str, params: &Value) -> Result<(), AutomationError> {
    if photocraft_engine::untrusted::reaches_outside_desktop(id, params) { Err(command_error(id)) } else { Ok(()) }
}

/// [`authorize_engine_command`] as the function pointer [`photocraft_engine::Session::authorize`]
/// stores. `actions.play` calls it for every nested step.
pub fn authorize_engine_step(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    authorize_engine_command(id, params).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
}

/// [`authorize_desktop_engine_command`] as a [`photocraft_engine::Session::authorize`] hook.
pub fn authorize_desktop_engine_step(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    authorize_desktop_engine_command(id, params).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
}

fn command_error(id: &str) -> AutomationError {
    AutomationError::BadRequest(format!("automation command `{id}` uses ambient filesystem paths and is disabled; use capability-scoped document methods"))
}

/// An export owns only files it created through this held directory capability. Refuse
/// collisions (including symlinks) so cancellation cannot delete or replace an earlier export.
pub(crate) struct ExportSequence {
    dir: Dir,
    created: Vec<String>,
}

impl AuthorizedWorkspace {
    pub(crate) fn export_sequence(&self, path: &str) -> Result<ExportSequence, AutomationError> {
        let relative = request_path(path, self.write.as_ref())?;
        let root = self.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        root.dir.create_dir_all(&relative).map_err(|e| file_error("mkdir", path, e))?;
        let dir = root.dir.open_dir(&relative).map_err(|e| file_error("open export directory", path, e))?;
        Ok(ExportSequence { dir, created: Vec::new() })
    }
}

impl ExportSequence {
    pub(crate) fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), AutomationError> {
        let relative = relative_path(name)?;
        if relative.components().count() != 1 {
            return Err(path_error(name, "export filename must be a single component"));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self.dir.open_with(&relative, &options).map_err(|e| file_error("create export", name, e))?;
        // Record immediately after creation, before any fallible write, including partial writes.
        self.created.push(name.to_owned());
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| file_error("write export", name, e))
    }

    pub(crate) fn commit(&mut self) {
        self.created.clear();
    }

    pub(crate) fn cleanup(&mut self) -> Result<(), AutomationError> {
        let mut failed = Vec::new();
        self.created.retain(|name| match self.dir.remove_file(name) {
            Ok(()) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                true
            }
        });
        if failed.is_empty() { Ok(()) } else { Err(AutomationError::Io(format!("export cleanup failed: {}", failed.join("; ")))) }
    }
}

impl Drop for ExportSequence {
    fn drop(&mut self) {
        // Also cover an escaped rendering panic; normal errors report cleanup failures above.
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests;
