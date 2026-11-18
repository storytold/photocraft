//! Plug-ins on the shared runtime, `craft-plugin` (RFC-0046): command plug-ins, which act on the
//! document only through the same commands as the menus, the CLI, the control channel and MCP,
//! and the installed plug-ins of both kinds PhotoCraft runs (the user's `Plug-ins` folder, or this
//! session only), installed from a `.craftplugin` bundle, a GitHub repository or a catalog.
//!
//! PhotoCraft's own kind, the pixel filter (`photocraft-plugins`, ABI 1), keeps its ABI and its
//! process-wide registry: a filter that comes in a bundle (`"kind": "filter"`) is checked and kept
//! here like a command plug-in, then registered there, so Filter › Plug-ins, smart filters and
//! previews find it as before. Bare `.wasm` filters (`plugin.install` with a `.wasm` path or base64
//! module, `plugin.reload`, the Additional Plug-ins Folder preference) work as they always did.
//!
//! This module is PhotoCraft's side of the shared runtime: which commands a plug-in may call
//! ([`policy`], [`refusal`]), one undo step per run rolled back when the run fails, read-only access
//! checked after every call, the plug-ins' settings (kept with the preferences, [`SavedPlugins`])
//! and the installed plug-ins ([`Runtime`]). The `plugin.*` commands are in `cmd::plugin_cmds`.
//! See `docs/plugins.md`.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

pub use craft_plugin;
use craft_plugin::fetch::{self, Fetch};
use craft_plugin::{API_VERSION, Access, AppInfo, Instance, KIND_COMMAND, Limits, Manifest, Policy, RunOutput, UiChange, bundle, catalog, github};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// PhotoCraft's own plug-in kind: a pixel filter (`photocraft-plugins`, ABI 1).
pub const KIND_FILTER: &str = "filter";

/// PhotoCraft as manifests name it: command plug-ins of every shared ABI version, and filters,
/// only from manifests that name `photocraft` in `apps`. A filter's `apps.photocraft.api` is the
/// filter ABI ([`photocraft_plugins::ABI_VERSION`], checked by [`target`]).
pub const APP: AppInfo<'static> =
    AppInfo { id: "photocraft", version: env!("CARGO_PKG_VERSION"), api: 1..=API_VERSION, kinds: &[KIND_COMMAND, KIND_FILTER], accept_legacy: false };

/// What one plug-in may keep in its settings (keys and JSON values, in bytes), and its longest key.
pub const MAX_SETTINGS_BYTES: usize = 64 * 1024;
pub const MAX_SETTING_KEY: usize = 128;
/// Catalogs a person may add.
pub const MAX_CATALOGS: usize = 32;
/// Messages a run may show (`notify`), and their length in characters.
const MAX_NOTES: usize = 20;
const MAX_NOTE_CHARS: usize = 1000;

/// PhotoCraft's commands no plug-in may call, on top of [`craft_plugin::ALWAYS_DENIED`] (plug-ins,
/// Undo and Redo, `file.*`, `app.*`, `pref.*`, `window.*`). A plug-in gets no path, no file, no
/// other document, no preferences, no undo history and no app or window state, and a run is
/// already one undo step, so nothing that runs other commands either.
const DENIED: &[&str] = &[
    // Preferences (PhotoCraft's are `prefs.*` and Edit › Preferences), and the settings dialogs
    // that are app state: Color Settings, shortcuts, menus, the toolbar.
    "prefs.*",
    "edit.preferences*",
    "edit.colorSettings",
    "edit.keyboardShortcuts*",
    "edit.menus*",
    "edit.toolbar*",
    "edit.presets*",
    "edit.remoteConnections*",
    // The undo history: History panel steps and snapshots, Purge, Step Backward/Forward.
    "history.*",
    "edit.stepBackward",
    "edit.stepForward",
    "edit.toggleLastState",
    "edit.purge*",
    // Other documents: switching, arranging and duplicating them, and the app's document presets
    // and recent files (`document.inspect`, `document.pixel` and `document.sampleColor` read the
    // active document and stay). Document Undo and Redo are the history.
    "document.activate",
    "document.move",
    "document.presets.*",
    "document.recent.*",
    "document.undo",
    "document.redo",
    "image.duplicate",
    // Running other commands inside, past this policy: actions, scripts, batches, droplets.
    "actions.*",
    "action.*",
    "automate.*",
    "script.*",
    "scripts.*",
    // The app, its windows, panels, view and workspace (view state, not the document).
    "view.*",
    "workspace.*",
    "panel.*",
    "panels.*",
    "help.*",
    "menus.*",
    "shortcuts.*",
    "tools.*",
    "tool.*",
    "ui.*",
    "interface.*",
    // The user's libraries, kept across sessions: brush, swatch, gradient, pattern, style and
    // shape presets (listing and applying them stay), and defining new ones from the document.
    "brush.presets.*",
    "brush.defineFromSelection",
    "edit.defineBrushPreset",
    "edit.defineCustomShape",
    "edit.definePattern",
    "swatches.*",
    "pattern.rename",
    "pattern.delete",
    "pattern.import",
    "pattern.export",
    "gradient.presets.new",
    "gradient.presets.edit",
    "gradient.presets.reset",
    "gradient.presets.importGrd",
    "pattern.presets.new",
    "pattern.presets.edit",
    "style.presets.new",
    "style.presets.edit",
    "style.presets.reset",
    "style.presets.importAsl",
    "style.presets.exportAsl",
    "shape.presets.new",
    "shape.presets.edit",
    "shape.presets.reset",
    "shape.presets.importCsh",
    "measurementLog.export",
    // The session: its other documents and background jobs, and test hooks.
    "session.*",
    "jobs.*",
    "test.*",
    // The clipboard belongs to the session and may hold another document.
    "edit.copy*",
    "edit.cut",
    "edit.paste*",
    "edit.clearClipboard",
    // Collaboration and sharing, and other network services.
    "share.*",
    "cloud.*",
    "libraries.*",
    "generator.*",
];

/// Parameter names that name a file or folder on disk, refused at any depth whatever the command
/// (a plug-in gets the document's content, never a path): `path`, `folder` and the like.
const FILE_KEYS: &[&str] = &["path", "paths", "folder", "folders", "dir", "file", "files", "fileName", "mapPath", "imagesFolder", "url", "lutFile"];

/// The rules every plug-in run follows: [`craft_plugin::ALWAYS_DENIED`] and [`DENIED`].
pub fn policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(|| Policy::new(DENIED, &[]))
}

/// Why plug-in `m` may not call `command` with `params`, before it runs: a refused command, one
/// outside its manifest's `permissions.commands`, or parameters that name a file.
pub fn refusal(m: &Manifest, command: &str, params: &Value) -> Option<String> {
    if let Err(why) = craft_plugin::check_call(m, policy(), command) {
        return Some(why);
    }
    if names_a_file(params, 0) {
        return Some(format!("plug-ins can't name files or folders (`{command}` was given a path): pass the content itself, as base64 `data`"));
    }
    if names_a_document(command, params) {
        return Some(format!("plug-ins work on the active document only (`{command}` was given another document)"));
    }
    None
}

/// Whether `params` name another open document (or a new one): a `document`, `docs` or
/// `sourceDocument` key at any depth, or a key `command`'s params doc marks as a document
/// (`"source":doc`, Match Color's source).
fn names_a_document(command: &str, params: &Value) -> bool {
    let Some(map) = params.as_object() else { return false };
    has_document_key(params, 0) || crate::commands::find(command).is_some_and(|spec| map.keys().any(|k| spec.params.contains(&format!("\"{k}\":doc"))))
}

/// A key naming a document at any depth (Apply Image's `source.document`, `sourceDocument`).
fn has_document_key(params: &Value, depth: usize) -> bool {
    depth < 128
        && match params {
            Value::Object(m) => m
                .iter()
                .any(|(k, v)| matches!(k.as_str(), "document" | "documents" | "docs" | "sourceDocument" | "maskDocument") || has_document_key(v, depth + 1)),
            Value::Array(a) => a.iter().any(|v| has_document_key(v, depth + 1)),
            _ => false,
        }
}

/// Whether `params` hold a [`FILE_KEYS`] key at any depth, or a profile named by its file
/// (`"profile": "/x/y.icc"`).
fn names_a_file(params: &Value, depth: usize) -> bool {
    // serde_json refuses nesting past 128, so nothing deeper reaches here.
    depth < 128
        && match params {
            Value::Object(m) => m.iter().any(|(k, v)| {
                FILE_KEYS.contains(&k.as_str())
                    || (k.to_ascii_lowercase().ends_with("profile") && v.as_str().is_some_and(looks_like_a_file))
                    || names_a_file(v, depth + 1)
            }),
            Value::Array(a) => a.iter().any(|v| names_a_file(v, depth + 1)),
            _ => false,
        }
}

/// A profile value that is a file rather than a profile's name.
fn looks_like_a_file(v: &str) -> bool {
    let lower = v.to_ascii_lowercase();
    v.contains(['/', '\\', ':']) || lower.ends_with(".icc") || lower.ends_with(".icm")
}

/// Whether `app` can run plug-in `m`, and with which API: [`Manifest::target`], and for a filter
/// the filter ABI PhotoCraft runs.
pub fn target(m: &Manifest) -> std::result::Result<u32, String> {
    let api = m.target(&APP)?.api();
    if m.kind == KIND_FILTER && i64::from(api) != i64::from(photocraft_plugins::ABI_VERSION) {
        return Err(format!("it is a filter for plug-in ABI {api}, and PhotoCraft runs filter ABI {}", photocraft_plugins::ABI_VERSION));
    }
    Ok(api)
}

/// Checks a bundle's module before it is kept or installed (compiled, never run): a command
/// plug-in against the shared sandbox, a filter against the filter ABI, whose own embedded
/// manifest must name the same plug-in.
pub fn check_module(m: &Manifest, wasm: &[u8]) -> std::result::Result<(), String> {
    match m.kind.as_str() {
        KIND_COMMAND => bundle::check_command_module(m, wasm),
        KIND_FILTER => {
            let p = photocraft_plugins::Plugin::load(wasm, photocraft_plugins::Limits::default()).map_err(|e| e.to_string())?;
            if !p.id().eq_ignore_ascii_case(&m.id) {
                return Err(format!("its filter module calls itself `{}`, not `{}`", p.id(), m.id));
            }
            Ok(())
        }
        kind => Err(format!("PhotoCraft doesn't run `{kind}` plug-ins")),
    }
}

/// [`bundle::open_with`] with PhotoCraft's [`check_module`].
pub fn open_bundle(bytes: &[u8]) -> std::result::Result<(bundle::Bundle, Manifest), String> {
    bundle::open_with(bytes, &check_module)
}

/// What PhotoCraft keeps in its settings file for plug-ins (`[plugins]`): the catalogs a person
/// added and each plug-in's settings (`setting_set`), as JSON text by plug-in id and key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SavedPlugins {
    pub catalogs: Vec<String>,
    /// Plug-in id → setting key → the setting's JSON text (text, because the settings file has no
    /// null and a plug-in's JSON may hold one).
    pub settings: BTreeMap<String, BTreeMap<String, String>>,
}

/// An installed bundle: a command plug-in, or a filter (also in `photocraft_plugins::registry`).
#[derive(Clone, Debug)]
pub struct Plugin {
    pub manifest: Manifest,
    /// The SHA-256 of its bundle (lower-case hex).
    pub sha256: String,
    /// Where it came from: `{type: file, path}`, `{type: catalog, catalog, package}` or
    /// `{type: github, repo, tag, asset}` (craft-plugin's `store::Source`).
    pub source: Value,
    /// Kept in the user's `Plug-ins` folder; otherwise for this session only (the CLI, MCP).
    pub stored: bool,
    wasm: Arc<Vec<u8>>,
}

impl Plugin {
    /// Whether it is a command plug-in (else a filter).
    pub fn is_command(&self) -> bool {
        self.manifest.kind == KIND_COMMAND
    }

    /// What `plugin.list` says about it.
    pub fn describe(&self) -> Value {
        let mut v = serde_json::to_value(&self.manifest).unwrap_or_else(|_| json!({"id": self.manifest.id}));
        v["kind"] = json!(self.manifest.kind);
        v["sha256"] = json!(self.sha256);
        v["origin"] = self.source.clone();
        v["stored"] = json!(self.stored);
        v
    }

    /// Register a filter in the process-wide filter registry (Filter › Plug-ins, smart filters).
    fn register_filter(&self) -> std::result::Result<(), String> {
        let shown = match self.source.get("type").and_then(Value::as_str) {
            Some("github") => format!("github:{}", self.source["repo"].as_str().unwrap_or_default()),
            Some("catalog") => format!("catalog:{}", self.source["package"].as_str().unwrap_or_default()),
            _ => format!("bundle:{}", self.source["path"].as_str().unwrap_or_default()),
        };
        let p = photocraft_plugins::Plugin::load(&self.wasm, photocraft_plugins::Limits::default()).map_err(|e| e.to_string())?;
        photocraft_plugins::registry::install(p.with_source(shown));
        Ok(())
    }
}

/// A bundle checked and shown for review (`plugin.install {review: true}`), kept so the install
/// that follows doesn't download it again.
struct Pending {
    sha256: String,
    /// The `plugin.install` parameters that named it.
    request: Value,
    candidate: Candidate,
}

/// The session's bundle plug-ins, its HTTPS client and the plug-ins' saved settings.
#[derive(Default)]
pub struct Runtime {
    /// The user's `Plug-ins` folder (the app sets it; the CLI and MCP install for the session).
    #[cfg(not(target_arch = "wasm32"))]
    store: Option<craft_plugin::store::Store>,
    plugins: Vec<Plugin>,
    /// Plug-in folders that couldn't be read, or plug-ins this version can't run.
    problems: Vec<String>,
    /// HTTPS for catalogs and GitHub releases (the app sets its own client; without one, only
    /// files and `file://` catalogs install).
    pub fetch: Option<Box<dyn Fetch + Send>>,
    pending: Option<Pending>,
    /// The last command plug-in run (plug-in id, command id, params).
    pub last: Option<(String, String, Value)>,
    /// Catalogs and settings, kept with the preferences ([`crate::prefs::SavedPrefs::plugins`]).
    pub saved: SavedPlugins,
}

impl Runtime {
    /// Keep plug-ins in `<settings_dir>/Plug-ins` and read the ones installed there (filters are
    /// registered for Filter › Plug-ins at once).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_store(&mut self, settings_dir: &std::path::Path) {
        self.store = Some(craft_plugin::store::Store::new(craft_plugin::store::plugins_dir(settings_dir)));
        self.reload();
    }

    /// Read the store again (the session's own plug-ins stay).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn reload(&mut self) {
        let Some(store) = &self.store else { return };
        let (installed, problems) = store.list();
        self.plugins.retain(|p| !p.stored);
        self.problems = problems.into_iter().map(|p| format!("{}: {}", p.dir.display(), p.error)).collect();
        for i in installed {
            if let Err(why) = target(&i.manifest) {
                self.problems.push(format!("{}: {why}", i.manifest.name));
                continue;
            }
            let source = serde_json::to_value(&i.record.source).unwrap_or(Value::Null);
            let plugin = Plugin { manifest: i.manifest, sha256: i.record.sha256, source, stored: true, wasm: Arc::new(i.bundle.wasm) };
            if plugin.manifest.kind == KIND_FILTER
                && let Err(why) = plugin.register_filter()
            {
                self.problems.push(format!("{}: {why}", plugin.manifest.name));
                continue;
            }
            self.plugins.retain(|p| !p.manifest.id.eq_ignore_ascii_case(&plugin.manifest.id));
            self.plugins.push(plugin);
        }
    }

    /// The installed bundle plug-ins.
    pub fn list(&self) -> &[Plugin] {
        &self.plugins
    }

    /// The installed command plug-ins (the Plug-ins menu's).
    pub fn commands(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.iter().filter(|p| p.is_command())
    }

    /// Why plug-in folders couldn't be used.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Plug-in `id` (ignoring case, as the store does).
    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.manifest.id.eq_ignore_ascii_case(id))
    }

    /// Command plug-in `id`.
    pub fn command(&self, id: &str) -> Option<&Plugin> {
        self.get(id).filter(|p| p.is_command())
    }

    /// Whether plug-ins are kept in the user's folder (else for this session only).
    pub fn has_store(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.store.is_some();
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// The user's `Plug-ins` folder, when plug-ins are kept there.
    pub fn store_dir(&self) -> Option<String> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.store.as_ref().map(|s| s.root().display().to_string());
        #[cfg(target_arch = "wasm32")]
        None
    }
}

/// A fetcher for sessions without network access: `file://` catalogs still read.
struct NoNetwork;

impl Fetch for NoNetwork {
    fn get(&mut self, url: &str, _max: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        let shown: String = url.chars().take(120).collect();
        Err(format!("there is no network access here to read {shown}: install the plug-in from a file, or in the PhotoCraft app"))
    }
}

/// Run `f` with the session's HTTPS client (or [`NoNetwork`]).
fn with_fetch<T>(s: &mut Session, f: impl FnOnce(&mut dyn Fetch) -> T) -> T {
    match s.plugins.fetch.as_deref_mut() {
        Some(http) => f(http),
        None => f(&mut NoNetwork),
    }
}

/// A plug-in error as the engine reports it.
fn failed(name: &str, e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{name}: {e}"))
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

// ---------- running ----------

/// The sandbox's view of the session: every call is checked by [`refusal`], runs as its own
/// nested step, and is put back and refused when it opens, closes or switches documents or, from a
/// read-only plug-in, changes the document.
struct EngineHost<'s> {
    s: &'s mut Session,
    manifest: &'s Manifest,
    notes: Vec<String>,
}

/// The open documents' ids and the active one's.
fn documents(s: &Session) -> (Vec<photocraft_doc::DocId>, Option<photocraft_doc::DocId>) {
    (s.documents().iter().map(|d| d.doc.id).collect(), s.active().map(|d| d.doc.id))
}

/// Close the documents a call opened and make `active` active again; whether anything changed.
fn restore_documents(s: &mut Session, before: &(Vec<photocraft_doc::DocId>, Option<photocraft_doc::DocId>)) -> bool {
    let after = documents(s);
    if after == *before {
        return false;
    }
    while let Some(i) = s.documents().iter().rposition(|d| !before.0.contains(&d.doc.id)) {
        s.close(i);
    }
    if let Some(i) = before.1.and_then(|id| s.documents().iter().position(|d| d.doc.id == id)) {
        s.set_active(i);
    }
    true
}

/// Run one plug-in call: the command, then the checks that put it back.
fn call_checked(s: &mut Session, command: &str, params: Value) -> Result<Value> {
    let docs = documents(s);
    let r = s.execute(command, params);
    if restore_documents(s, &docs) {
        return Err(EngineError::Other(format!("`{command}` opens, closes or switches documents, which plug-ins can't")));
    }
    r
}

impl craft_plugin::Host for EngineHost<'_> {
    fn call(&mut self, command: &str, params: &str) -> std::result::Result<String, String> {
        let params: Value = serde_json::from_str(params).map_err(|e| format!("the params for `{command}` aren't JSON: {e}"))?;
        if !(params.is_object() || params.is_null()) {
            return Err(format!("the params for `{command}` must be a JSON object"));
        }
        if let Some(why) = refusal(self.manifest, command, &params) {
            return Err(why);
        }
        let m = self.manifest;
        let s = &mut *self.s;
        let r = if s.active().is_some() {
            // Its own nested step: a call that fails, or that a read-only plug-in edits with, is
            // put back whole (document, targeted layers, channel view, the journal). The run's
            // own step is settled when it ends.
            crate::analysis_cmds::compound(s, command, |s| {
                let before = s.active().map(|d| d.doc.clone());
                let v = call_checked(s, command, params)?;
                let changed = match (before, s.active()) {
                    (Some(b), Some(d)) => !Arc::ptr_eq(&b, &d.doc) && *b != *d.doc,
                    _ => false,
                };
                if changed && m.permissions.access == Access::Read {
                    return Err(EngineError::Other(craft_plugin::check_read_only(m, command, true).err().unwrap_or_default()));
                }
                Ok(v)
            })
        } else {
            call_checked(s, command, params)
        };
        r.map(|v| v.to_string()).map_err(|e| e.to_string())
    }

    fn selection(&mut self) -> String {
        let ids: Vec<u64> = self.s.active().map(|d| d.selected_layers().iter().map(|l| l.0).collect()).unwrap_or_default();
        json!(ids).to_string()
    }

    fn notify(&mut self, message: &str) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(message.chars().take(MAX_NOTE_CHARS).collect());
        }
    }

    fn setting(&mut self, key: &str) -> Option<String> {
        self.s.plugins.saved.settings.get(&self.manifest.id)?.get(key).cloned()
    }

    fn set_setting(&mut self, key: &str, value: Option<&str>) -> std::result::Result<(), String> {
        set_setting(&mut self.s.plugins.saved.settings, &self.manifest.id, key, value)?;
        // The preferences revision moves, so the app saves the settings file.
        self.s.prefs.edit(|_| ());
        Ok(())
    }
}

/// Store (`Some`) or forget (`None`) plug-in `id`'s setting `key` in `all`, within
/// [`MAX_SETTINGS_BYTES`] and [`MAX_SETTING_KEY`].
fn set_setting(all: &mut BTreeMap<String, BTreeMap<String, String>>, id: &str, key: &str, value: Option<&str>) -> std::result::Result<(), String> {
    let Some(text) = value else {
        if let Some(mine) = all.get_mut(id) {
            mine.remove(key);
            if mine.is_empty() {
                all.remove(id);
            }
        }
        return Ok(());
    };
    if key.is_empty() || key.chars().count() > MAX_SETTING_KEY || key.chars().any(char::is_control) {
        return Err(format!("a setting's key must be 1–{MAX_SETTING_KEY} characters without control characters"));
    }
    let too_big = || format!("a plug-in's settings are limited to {} KB", MAX_SETTINGS_BYTES / 1024);
    if key.len() + text.len() > MAX_SETTINGS_BYTES {
        return Err(too_big());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("a setting must be JSON: {e}"))?;
    let text = v.to_string();
    let others: usize = all.get(id).map_or(0, |mine| mine.iter().filter(|(k, _)| k.as_str() != key).map(|(k, v)| k.len() + v.len()).sum());
    if others + key.len() + text.len() > MAX_SETTINGS_BYTES {
        return Err(too_big());
    }
    all.entry(id.to_string()).or_default().insert(key.to_string(), text);
    Ok(())
}

/// Run command plug-in `id`'s `command` (default: its first) with `params` as one undo step named
/// after the plug-in. A run that fails (a trap, an exhausted budget, a refusal it passes on, a
/// non-zero status) leaves the document, its targeted layers and the undo history as they were;
/// a run that changed nothing leaves no step.
pub fn run(s: &mut Session, id: &str, command: Option<&str>, params: &Value, fuel: Option<u64>) -> Result<Value> {
    let cmd = "plugin.run";
    let p = s.plugins.command(id).cloned().ok_or_else(|| bad(cmd, format!("no command plug-in `{id}` is installed (see plugin.list)")))?;
    let m = &p.manifest;
    let api = target(m).map_err(|e| bad(cmd, format!("{} can't run here: {e}", m.name)))?;
    let c = m.command(command).ok_or_else(|| bad(cmd, format!("{} has no command `{}`", m.name, command.unwrap_or_default())))?;
    let params = c.resolve_params(params).map_err(|e| bad(cmd, e))?;
    let mut limits = Limits::default();
    if let Some(f) = fuel {
        limits.fuel = f.clamp(1000, limits.fuel);
    }
    let mut instance = Instance::new(&p.wasm, &limits).map_err(|e| failed(&m.name, e))?;
    let selection: Vec<u64> = s.active().map(|d| d.selected_layers().iter().map(|l| l.0).collect()).unwrap_or_default();
    let input = json!({"api": api, "command": c.id, "params": params, "selection": selection}).to_string();
    s.plugins.last = Some((m.id.clone(), c.id.clone(), params));
    let label = if m.commands.len() > 1 { format!("{}: {}", m.name, c.label.trim_end_matches(['…', '.'])) } else { m.name.clone() };

    let mut notes = Vec::new();
    let mut call = |s: &mut Session| -> Result<RunOutput> {
        let mut host = EngineHost { s, manifest: m, notes: Vec::new() };
        let ran = instance.call(&input, &mut host);
        notes = std::mem::take(&mut host.notes);
        ran.map_err(|e| failed(&m.name, e))
    };
    let out = if s.active().is_some() {
        // The run's commands record steps of their own; they become one, named after the plug-in,
        // and all of it is put back when the run fails.
        crate::analysis_cmds::compound_with(s, &label, false, call)?
    } else {
        call(s)?
    };
    let output = serde_json::from_str::<Value>(&out.output).unwrap_or(Value::String(out.output));
    // Panels (`ui_show`) aren't shown in PhotoCraft yet: say so rather than drop it silently.
    let panel = matches!(out.ui, Some(UiChange::Show(_))).then_some("PhotoCraft doesn't show plug-in panels yet");
    Ok(json!({"plugin": m.id, "command": c.id, "output": output, "notes": notes, "logs": out.logs, "calls": out.calls, "fuel": out.fuel, "panel": panel}))
}

// ---------- installing ----------

/// A bundle obtained for installing: its bytes, where it came from and, for a repository,
/// whether a catalog in use lists it.
#[derive(Clone)]
pub(crate) struct Candidate {
    pub bytes: Vec<u8>,
    pub source: Value,
    /// `Some(true)` from a catalog; for a repository, whether a catalog in use lists it.
    pub in_catalog: Option<bool>,
}

/// Read at most `max` bytes of the regular file `path`.
#[cfg(not(target_arch = "wasm32"))]
fn read_capped(path: &str, max: usize) -> std::result::Result<Vec<u8>, String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = f.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    let mut bytes = Vec::new();
    f.take(max as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err(format!("larger than {} MB", max >> 20));
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn read_capped(_path: &str, _max: usize) -> std::result::Result<Vec<u8>, String> {
    Err("installing from a path is not available on the web; pass the bundle as base64 `data`".into())
}

/// Whether `bytes` are a `.craftplugin` bundle (rather than a bare filter module).
pub fn is_bundle(bytes: &[u8]) -> bool {
    bytes.starts_with(bundle::MAGIC)
}

/// Whether a `plugin.install` `path` names a bundle (`.craftplugin`; anything else is a bare
/// filter module, read as before).
pub fn is_bundle_path(path: &str) -> bool {
    std::path::Path::new(path).extension().is_some_and(|x| x.eq_ignore_ascii_case(bundle::BUNDLE_EXTENSION))
}

/// Read the bundle `p` names: `path` (a `.craftplugin`), `data` (a base64 bundle, already
/// decoded as `data`), `repo` (`owner/repo[@tag]`) or `catalog` + `id` (and optionally `version`).
pub(crate) fn obtain(s: &mut Session, cmd: &str, p: &Value, data: Option<Vec<u8>>) -> Result<Candidate> {
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    if let Some(bytes) = data {
        let name: String = text("name").unwrap_or("data").chars().take(200).collect();
        return Ok(Candidate { bytes, source: json!({"type": "file", "path": name}), in_catalog: None });
    }
    if let Some(path) = text("path") {
        let bytes = read_capped(path, bundle::MAX_BUNDLE_BYTES).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
        return Ok(Candidate { bytes, source: json!({"type": "file", "path": path}), in_catalog: None });
    }
    if let Some(spec) = text("repo") {
        let repo = github::parse_repo_spec(spec).map_err(|e| bad(cmd, e))?;
        let catalogs = s.plugins.saved.catalogs.clone();
        let (plan, bytes) = with_fetch(s, |f| {
            let plan = fetch::resolve_repo(f, &repo)?;
            let bytes = fetch::download_repo(f, &plan)?;
            Ok::<_, String>((plan, bytes))
        })
        .map_err(|e| bad(cmd, e))?;
        let source = json!({"type": "github", "repo": plan.repo.slug(), "tag": plan.repo.tag.clone().unwrap_or_default(), "asset": plan.asset});
        let local_ok = !untrusted(s);
        let in_catalog = with_fetch(s, |f| {
            catalogs.iter().filter(|url| local_ok || !is_file_url(url)).any(|url| {
                fetch::fetch_catalog(f, url)
                    .is_ok_and(|(cat, _)| cat.packages.iter().any(|pkg| pkg.repo_ref().is_some_and(|r| r.same_repo(&repo.owner, &repo.name))))
            })
        });
        return Ok(Candidate { bytes, source, in_catalog: Some(in_catalog) });
    }
    if let Some(url) = text("catalog") {
        let id = text("id").ok_or_else(|| bad(cmd, "`id` required with `catalog` (a package of the catalog)"))?;
        let version = text("version");
        local_refused(s, url).map_err(|e| bad(cmd, e))?;
        let bytes = with_fetch(s, |f| {
            let (cat, _) = fetch::fetch_catalog(f, url)?;
            let package = cat.package(id).ok_or_else(|| format!("{url} has no package `{id}`"))?;
            let release = match version {
                Some(v) => package.release(v).ok_or_else(|| format!("{id} has no version {v} in {url}"))?,
                None => package.latest_for(&APP).ok_or_else(|| format!("{id} has no release for PhotoCraft {}", APP.version))?,
            };
            release.target(&package.kind, &APP)?;
            let bytes = fetch::download_release(f, url, package, release)?;
            let (_, manifest) = open_bundle(&bytes)?;
            package.check_manifest(release, &manifest)?;
            Ok::<_, String>(bytes)
        })
        .map_err(|e| bad(cmd, e))?;
        return Ok(Candidate { bytes, source: json!({"type": "catalog", "catalog": url, "package": id}), in_catalog: Some(true) });
    }
    Err(bad(cmd, "give `path` (a .wasm or .craftplugin file), `data` (base64), `repo` (owner/repo or owner/repo@tag) or `catalog` with `id`"))
}

/// What an install confirmation shows about `c`: the manifest's permissions, kind and commands,
/// the source, the SHA-256, whether a catalog lists it and what it replaces. An error when the
/// bundle isn't a plug-in PhotoCraft can run.
pub(crate) fn review(s: &Session, cmd: &str, c: &Candidate) -> Result<(Manifest, Value)> {
    let (_, manifest) = open_bundle(&c.bytes).map_err(|e| bad(cmd, e))?;
    target(&manifest).map_err(|e| bad(cmd, format!("{} can't be installed: {e}", manifest.name)))?;
    let old = s.plugins.get(&manifest.id);
    let mut v = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({"id": manifest.id}));
    v["kind"] = json!(manifest.kind);
    v["sha256"] = json!(bundle::sha256_hex(&c.bytes));
    v["origin"] = c.source.clone();
    v["inCatalog"] = json!(c.in_catalog);
    v["replaces"] = json!(old.map(|o| &o.manifest.version));
    v["permissionsChanged"] = json!(old.is_some_and(|o| craft_plugin::permissions::changed(&o.manifest, &manifest)));
    v["stored"] = json!(s.plugins.has_store() && !untrusted(s));
    Ok((manifest, v))
}

/// Install `c` (already reviewed): into the user's folder when the session has one, else for the
/// session. Replaces an installed plug-in with the same id; its settings stay. A filter is
/// registered for Filter › Plug-ins.
pub(crate) fn install(s: &mut Session, cmd: &str, c: Candidate) -> Result<Plugin> {
    let (manifest, _) = review(s, cmd, &c)?;
    let sha256 = bundle::sha256_hex(&c.bytes);
    // Automation installs for the session only: an agent never adds to the person's Plug-ins
    // folder, which runs at every launch.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(store) = s.plugins.store.as_ref().filter(|_| !untrusted(s)) {
        let source: craft_plugin::store::Source = serde_json::from_value(c.source.clone()).map_err(|e| bad(cmd, e.to_string()))?;
        let i = store.install_with(&c.bytes, source, &check_module).map_err(|e| bad(cmd, e))?;
        let plugin = Plugin { manifest: i.manifest, sha256: i.record.sha256, source: c.source, stored: true, wasm: Arc::new(i.bundle.wasm) };
        return keep(s, plugin).map_err(|e| bad(cmd, e));
    }
    let (b, _) = open_bundle(&c.bytes).map_err(|e| bad(cmd, e))?;
    keep(s, Plugin { manifest, sha256, source: c.source, stored: false, wasm: Arc::new(b.wasm) }).map_err(|e| bad(cmd, e))
}

/// Put `plugin` in the session's list, in place of one with the same id (a filter in the filter
/// registry too).
fn keep(s: &mut Session, plugin: Plugin) -> std::result::Result<Plugin, String> {
    if plugin.manifest.kind == KIND_FILTER {
        plugin.register_filter()?;
    }
    let list = &mut s.plugins.plugins;
    match list.iter_mut().find(|p| p.manifest.id.eq_ignore_ascii_case(&plugin.manifest.id)) {
        Some(slot) => *slot = plugin.clone(),
        None => list.push(plugin.clone()),
    }
    Ok(plugin)
}

/// Keep `c`, named by the install parameters `request`, for the install that follows its review
/// (one at a time).
pub(crate) fn hold(s: &mut Session, sha256: &str, request: Value, candidate: Candidate) {
    s.plugins.pending = Some(Pending { sha256: sha256.into(), request, candidate });
}

/// The bundle reviewed last, when it is `sha256` and `request` named it.
pub(crate) fn take_held(s: &mut Session, sha256: &str, request: &Value) -> Option<Candidate> {
    let held = s.plugins.pending.take_if(|h| h.sha256.eq_ignore_ascii_case(sha256) && h.request == *request)?;
    Some(held.candidate)
}

/// Remove bundle plug-in `id` (a filter from the filter registry too) and its settings; whether it
/// was installed.
pub(crate) fn remove(s: &mut Session, cmd: &str, id: &str) -> Result<bool> {
    let Some(p) = s.plugins.get(id).cloned() else { return Ok(false) };
    #[cfg(not(target_arch = "wasm32"))]
    if p.stored
        && let Some(store) = &s.plugins.store
    {
        store.remove(&p.manifest.id).map_err(|e| bad(cmd, e))?;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = cmd;
    s.plugins.plugins.retain(|x| !x.manifest.id.eq_ignore_ascii_case(id));
    if p.manifest.kind == KIND_FILTER {
        photocraft_plugins::registry::remove(&p.manifest.id);
    }
    if s.plugins.saved.settings.remove(&p.manifest.id).is_some() {
        s.prefs.edit(|_| ());
    }
    if s.plugins.last.as_ref().is_some_and(|(last, ..)| last.eq_ignore_ascii_case(id)) {
        s.plugins.last = None;
    }
    Ok(true)
}

/// The catalog `url`'s packages PhotoCraft can run, with their newest release for it.
pub(crate) fn catalog_packages(s: &mut Session, url: &str) -> std::result::Result<(Vec<Value>, Vec<String>), String> {
    local_refused(s, url)?;
    let (cat, warnings) = with_fetch(s, |f| fetch::fetch_catalog(f, url))?;
    let packages = cat
        .packages
        .iter()
        .filter_map(|pkg| {
            let release = pkg.latest_for(&APP)?;
            let installed = s.plugins.get(&pkg.id).map(|p| p.manifest.version.clone());
            Some(json!({
                "catalog": url, "id": pkg.id, "name": pkg.name, "version": release.version, "author": pkg.author,
                "description": pkg.description, "license": pkg.license, "repo": pkg.repo, "kind": pkg.kind,
                "sha256": release.sha256, "installed": installed,
            }))
        })
        .collect();
    Ok((packages, warnings))
}

/// Whether the session runs requests from automation (MCP, the control channel), whose file
/// access is confined: it gets no `file://` catalog.
fn untrusted(s: &Session) -> bool {
    s.authorize.is_some()
}

fn is_file_url(url: &str) -> bool {
    url.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("file://"))
}

/// Why the catalog `url` can't be read here: a `file://` catalog (and the files it names) is read
/// straight from the disk, past automation's confined file access, so automation can't use one.
fn local_refused(s: &Session, url: &str) -> std::result::Result<(), String> {
    if is_file_url(url) && untrusted(s) {
        return Err("file:// catalogs can't be read by automation, whose file access is confined".into());
    }
    Ok(())
}

/// Whether `url` is a catalog address PhotoCraft reads (`https://` on GitHub's hosts, or
/// `file://`), or why not.
pub(crate) fn check_catalog_url(s: &Session, url: &str) -> std::result::Result<(), String> {
    local_refused(s, url)?;
    catalog::check_url(url, None).map(|_| ())
}

/// Where plug-in `p` has a newer release: the install parameters that get it and its version (a
/// tag, for a repository). `None` when it is up to date or came from a file.
pub(crate) fn update_for(s: &mut Session, p: &Plugin) -> std::result::Result<Option<(Value, String)>, String> {
    let text = |k: &str| p.source.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match p.source.get("type").and_then(Value::as_str) {
        Some("catalog") => {
            let (url, id) = (text("catalog"), text("package"));
            local_refused(s, &url)?;
            let (cat, _) = with_fetch(s, |f| fetch::fetch_catalog(f, &url))?;
            let release = cat.package(&id).and_then(|pkg| pkg.latest_for(&APP));
            Ok(release
                .filter(|r| bundle::newer(&r.version, &p.manifest.version))
                .map(|r| (json!({"catalog": url, "id": id, "version": r.version}), r.version.clone())))
        }
        Some("github") => {
            let mut repo = github::parse_repo_spec(&text("repo"))?;
            repo.tag = None;
            let plan = with_fetch(s, |f| fetch::resolve_repo(f, &repo))?;
            let tag = plan.repo.tag.clone().unwrap_or_default();
            Ok((tag != text("tag")).then(|| (json!({"repo": format!("{}@{tag}", plan.repo.slug())}), tag)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests;
