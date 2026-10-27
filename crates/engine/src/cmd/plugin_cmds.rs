//! `plugin.*` commands: sandboxed WebAssembly filter plug-ins (`photocraft-plugins`).
//!
//! Plug-ins are installed per process (`plugin.install`, or the Plug-ins preference folder),
//! listed with `plugin.list` and run with `plugin.run` on the active pixel layer, a targeted
//! channel, or a smart object (recorded as a smart filter, re-run on re-render while the plug-in
//! is installed). Runs respect the selection and are one undo step.

use std::collections::BTreeSet;

use photocraft_doc::{LayerContent, SmartFilter};
use photocraft_plugins::{Plugin, registry};
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The smart-filter command id plug-in runs are recorded under.
pub const RUN: &str = "plugin.run";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn err(e: photocraft_plugins::Error) -> EngineError {
    EngineError::Other(e.to_string())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// JSON description of an installed plug-in.
pub fn describe(p: &Plugin) -> Value {
    let m = p.manifest();
    json!({
        "id": m.id, "name": m.name, "version": m.version, "kind": m.kind, "author": m.author, "description": m.description,
        "params": m.params_notation(), "paramsSchema": serde_json::to_value(m).ok().and_then(|v| v.get("params").cloned()),
        "overlap": m.overlap, "area": m.area, "size": p.size(), "source": p.source(),
    })
}

/// The plug-in's own parameters from command params: `params` if given, else the top-level keys
/// minus the ones the command itself uses.
fn plugin_params(p: &Value) -> Value {
    if let Some(v) = p.get("params") {
        return v.clone();
    }
    match p {
        Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| !command_key(k)).map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => Value::Object(Map::new()),
    }
}

/// A top-level `plugin.run` key the command itself uses (or a dialog's private `__` key), not a
/// plug-in parameter.
fn command_key(k: &str) -> bool {
    matches!(k, "id" | "layer" | "target" | "channel") || k.starts_with("__")
}

/// Proxy factor for a live preview of `plugin.run` with `p` on a document reduced `k` times: the
/// plug-in's [`photocraft_plugins::Manifest::preview_factor`] (its `"unit": "px"` parameters set
/// the limit, not the built-in filters' parameter names). `k` when the plug-in isn't installed.
pub fn preview_factor(p: &Value, k: u32) -> u32 {
    let Ok(plugin) = plugin_by_id(RUN, p) else { return k };
    match plugin_params(p) {
        Value::Object(params) => plugin.manifest().preview_factor(&params, k),
        // `null` means the defaults (see `Manifest::resolve_params`).
        Value::Null => plugin.manifest().preview_factor(&Map::new(), k),
        _ => k,
    }
}

/// Rewrites `plugin.run` params `p` for a live preview on a copy of the document reduced `k`
/// times: the plug-in's pixel-sized parameters (`"unit": "px"`, under `params` or at the top
/// level, as [`plugin_params`] reads them) are divided by `k`, so the preview shows the lengths OK
/// applies (#7524). Nothing changes when the plug-in isn't installed (the run then fails as usual)
/// or for values of the wrong type (the run rejects them).
pub fn scale_for_preview(p: &mut Value, k: u32) {
    if k <= 1 {
        return;
    }
    let Ok(plugin) = plugin_by_id(RUN, p) else { return };
    let nested = p.get("params").is_some();
    let mut params = match plugin_params(p) {
        Value::Object(params) => params,
        Value::Null => Map::new(),
        _ => return,
    };
    plugin.manifest().scale_for_preview(&mut params, k);
    let Value::Object(o) = p else { return };
    if nested {
        o.insert("params".into(), Value::Object(params));
    } else {
        // Top-level form: write back only plug-in keys, never the command's own (`id`, `channel`…).
        o.extend(params.into_iter().filter(|(key, _)| !command_key(key)));
    }
}

fn plugin_by_id(cmd: &str, p: &Value) -> Result<std::sync::Arc<Plugin>> {
    let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `id` (see plugin.list)"))?;
    registry::get(id).ok_or_else(|| err(photocraft_plugins::Error::NotFound(id.into())))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let plugin = plugin_by_id(RUN, p)?;
    let params = plugin_params(p);
    // Bad parameters fail before anything is touched.
    let resolved = Value::Object(plugin.manifest().resolve_params(&params).map_err(|e| bad(RUN, e.to_string()))?);
    let layer = crate::commands::layer_param(s, p)?;
    let label = plugin.manifest().name.clone();
    let id = plugin.id().to_string();
    s.edit(&label, |doc, _| {
        let selection = doc.selection.clone();
        let canvas = doc.bounds();
        if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, Some(layer), p)? {
            *surf = plugin.apply(surf, canvas, selection.as_ref(), &resolved).map_err(err)?;
            return Ok(());
        }
        crate::commands::check_pixels_unlocked(doc, layer)?;
        let keep_alpha = doc.effective_locks(layer).transparency;
        let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
        match &mut l.content {
            LayerContent::Raster(surf) => {
                // Lock Transparency: the plug-in may change colour but not alpha (as built-in filters).
                let before = keep_alpha.then(|| surf.clone());
                *surf = plugin.apply(surf, canvas, selection.as_ref(), &resolved).map_err(err)?;
                if let Some(before) = &before {
                    crate::filters::keep_alpha(before, surf, &|| false)?;
                }
                Ok(())
            }
            LayerContent::Smart(_) => {
                let sf = SmartFilter {
                    command: RUN.into(),
                    params: json!({"id": id, "params": resolved}),
                    blend: photocraft_color::BlendMode::Normal,
                    opacity: 1.0,
                    visible: true,
                };
                crate::smart_cmds::add_smart_filter(doc, layer, sf, selection.as_ref())
            }
            other => Err(EngineError::Other(format!("plug-in filters need a pixel layer (active layer is {} {} layer)", other.article(), other.kind_name()))),
        }
    })?;
    Ok(json!({"layer": layer.0, "plugin": id}))
}

/// Smart-filter re-render of a recorded `plugin.run` (see `filters::apply_filter_to_surface`).
/// `None` when `command` isn't a plug-in run or the plug-in isn't installed (the smart filter is
/// then skipped, as for other unknown filters) or fails.
pub(crate) fn apply_to_surface(command: &str, params: &Value, surf: &Surface, selection: Option<&Surface>, canvas: photocraft_geom::Rect) -> Option<Surface> {
    if command != RUN {
        return None;
    }
    let plugin = registry::get(params.get("id")?.as_str()?)?;
    plugin.apply(surf, canvas, selection, params.get("params").unwrap_or(&Value::Null)).ok()
}

/// Largest base64 payload `plugin.install` accepts (the module limit, encoded).
const MAX_DATA_CHARS: usize = (32 << 20) / 3 * 4 + 4;

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        }))
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    if clean.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3 + 2);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(bytes.get(..chunk.len() - 1)?);
    }
    Some(out)
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.install";
    let plugin = if let Some(data) = p.get("data") {
        let data = data.as_str().ok_or_else(|| bad(CMD, "`data` must be a base64 string"))?;
        if data.len() > MAX_DATA_CHARS {
            return Err(bad(CMD, "`data` is larger than the module limit"));
        }
        let bytes = b64_decode(data).ok_or_else(|| bad(CMD, "`data` is not valid base64"))?;
        Plugin::load(&bytes, photocraft_plugins::Limits::default()).map_err(err)?
    } else if let Some(path) = p.get("path") {
        let path = path.as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad(CMD, "`path` must be a file path"))?;
        load_path(path)?
    } else {
        return Err(bad(CMD, "give `path` (a .wasm file) or `data` (the module as base64)"));
    };
    let replace = p.get("replace").and_then(Value::as_bool).unwrap_or(true);
    if !replace && registry::get(plugin.id()).is_some() {
        return Err(bad(CMD, format!("a plug-in with id {:?} is already installed", plugin.id())));
    }
    s.plugin_folders.wasm.remove(plugin.id());
    Ok(describe(&registry::install(plugin)))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_path(path: &str) -> Result<Plugin> {
    registry::load_file(std::path::Path::new(path), photocraft_plugins::Limits::default()).map_err(err)
}

#[cfg(target_arch = "wasm32")]
fn load_path(_: &str) -> Result<Plugin> {
    Err(EngineError::Other("installing from a path is not available on the web; pass the module as base64 `data`".into()))
}

/// What this session's preference folders registered, so a changed or disabled folder list
/// unregisters the plug-ins it no longer provides. Explicit installs are not tracked.
#[derive(Default)]
pub struct FolderPlugins {
    /// Folders of the last preference scan; `None` while the folders are off.
    folders: Option<Vec<String>>,
    /// The last scan's report, shown by `plugin.list`.
    report: Option<Value>,
    wasm: BTreeSet<String>,
    native: BTreeSet<String>,
}

impl FolderPlugins {
    /// Owns what `report` registered and unregisters what the previous preference scan
    /// registered but this one did not.
    fn adopt(&mut self, report: &Value) {
        let wasm = loaded_ids(report, "loaded", Value::as_str);
        let native = loaded_ids(report, "native_loaded", |v| v.get("id").and_then(Value::as_str));
        for id in self.wasm.difference(&wasm) {
            registry::remove(id);
        }
        #[cfg(not(target_arch = "wasm32"))]
        for id in self.native.difference(&native) {
            photocraft_plugins::native_8bf::remove(id);
        }
        self.wasm = wasm;
        self.native = native;
    }

    /// Plug-ins the user loaded explicitly stay installed when the preference folders change.
    fn disown(&mut self, report: &Value) {
        for id in loaded_ids(report, "loaded", Value::as_str) {
            self.wasm.remove(&id);
        }
        for id in loaded_ids(report, "native_loaded", |v| v.get("id").and_then(Value::as_str)) {
            self.native.remove(&id);
        }
    }

    /// An explicitly installed native filter stays installed when the preference folders change.
    pub(crate) fn disown_native(&mut self, id: &str) {
        self.native.remove(id);
    }
}

fn loaded_ids(report: &Value, key: &str, id: fn(&Value) -> Option<&str>) -> BTreeSet<String> {
    report.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(id).map(str::to_owned).collect()
}

/// Loads the configured folders when enabled and changed (startup and `prefs.set`), and
/// unregisters plug-ins from folders that were removed or turned off.
pub(crate) fn sync_prefs(s: &mut Session) {
    let enabled = s.prefs().plug_ins.use_additional_plugins_folder;
    let folders = s.prefs().plug_ins.effective_folders();
    let state = &mut s.plugin_folders;
    if !enabled || folders.is_empty() {
        if state.folders.take().is_some() {
            state.adopt(&Value::Null);
            state.report = None;
        }
        return;
    }
    if state.folders.as_ref() == Some(&folders) {
        return;
    }
    let report = load_folders(&folders);
    state.adopt(&report);
    state.folders = Some(folders);
    state.report = Some(report);
}

#[cfg(not(target_arch = "wasm32"))]
fn load_folders(folders: &[String]) -> Value {
    let paths: Vec<std::path::PathBuf> = folders.iter().map(std::path::PathBuf::from).collect();
    let folder = if folders.len() == 1 { folders.first().cloned() } else { None };
    // Preserve an error for an entirely missing configuration, while permitting partial scans.
    if !paths.iter().any(|path| path.is_dir()) {
        return json!({"folder": folder, "folders": folders, "error": "none of the plug-in folders is an accessible directory"});
    }
    match registry::load_folders(&paths) {
        Ok(r) => {
            // Native discovery only inspects PE/PiPL metadata; it never starts a worker.
            match photocraft_plugins::native_8bf::load_folders(&paths) {
                Ok(native) => {
                    json!({"folder": folder, "folders": folders, "loaded": r.loaded, "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>(),
                    "native_loaded": native.loaded, "native_failed": native.failed.iter().map(|(f,e)|json!({"file":f,"error":e})).collect::<Vec<_>>()})
                }
                Err(e) => {
                    json!({"folder": folder, "folders": folders, "loaded": r.loaded, "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>(), "native_error": e.to_string()})
                }
            }
        }
        Err(e) => json!({"folder": folder, "folders": folders, "error": e.to_string()}),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_folders(folders: &[String]) -> Value {
    let folder = if folders.len() == 1 { folders.first().cloned() } else { None };
    json!({"folder": folder, "folders": folders, "error": "plug-in folders are not available on the web"})
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.reload";
    if p.get("path").is_some() && p.get("paths").is_some() {
        return Err(bad(CMD, "give either `path` or `paths`, not both"));
    }
    let folders = if let Some(paths) = p.get("paths") {
        let paths = paths.as_array().filter(|paths| !paths.is_empty()).ok_or_else(|| bad(CMD, "`paths` must be a nonempty array of folders"))?;
        // Validate the complete argument before any discovery can mutate the registries.
        let paths = paths
            .iter()
            .map(|v| v.as_str().filter(|f| !f.trim().is_empty()).map(str::trim).ok_or_else(|| bad(CMD, "every `paths` entry must be a folder")))
            .collect::<Result<Vec<_>>>()?;
        let mut folders = Vec::new();
        for path in paths {
            if !folders.iter().any(|f| f == path) {
                folders.push(path.to_string());
            }
        }
        folders
    } else if let Some(path) = p.get("path") {
        vec![path.as_str().filter(|f| !f.trim().is_empty()).ok_or_else(|| bad(CMD, "`path` must be a folder"))?.trim().to_string()]
    } else {
        s.prefs().plug_ins.effective_folders()
    };
    let from_prefs = p.get("path").is_none() && p.get("paths").is_none() && s.prefs().plug_ins.use_additional_plugins_folder;
    if folders.is_empty() {
        return Err(bad(CMD, "no folders: pass `path` or `paths`, or add folders in Preferences › Plug-ins"));
    }
    let report = load_folders(&folders);
    let state = &mut s.plugin_folders;
    if from_prefs {
        // A rescan of the enabled preference folders replaces what they provided before.
        state.adopt(&report);
        state.folders = report.get("error").is_none().then_some(folders);
    } else {
        state.disown(&report);
    }
    if let Some(e) = report.get("error").and_then(Value::as_str) {
        return Err(EngineError::Other(e.to_string()));
    }
    state.report = Some(report.clone());
    Ok(report)
}

/// The plug-in command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "plugin.list",
            label: "List Plug-ins",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| {
                let folder = s.plugin_folders.report.clone();
                Ok(json!({"plugins": registry::list().iter().map(|p| describe(p)).collect::<Vec<_>>(), "folder": folder}))
            },
            journal: false,
        },
        CommandSpec {
            id: "plugin.install",
            label: "Install Plug-in…",
            menu: &["Filter", "Plug-ins"],
            shortcut: None,
            params: r##"{"path":text,"data":json,"replace":bool=true} (path: a .wasm file, native only; data: the module as base64)"##,
            enabled: always,
            run: install,
            journal: false,
        },
        CommandSpec {
            id: "plugin.remove",
            label: "Remove Plug-in",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text}"##,
            enabled: always,
            run: |_, p| {
                let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad("plugin.remove", "missing `id`"))?;
                if registry::remove(id) { Ok(json!({"removed": id})) } else { Err(err(photocraft_plugins::Error::NotFound(id.into()))) }
            },
            journal: false,
        },
        CommandSpec {
            id: "plugin.reload",
            label: "Reload Plug-ins",
            menu: &[],
            shortcut: None,
            params: r##"{"path":text,"paths":json} (one folder or an array; includes subfolders; default: all Plug-ins preference folders)"##,
            enabled: always,
            run: reload,
            journal: false,
        },
        CommandSpec {
            id: RUN,
            label: "Run Plug-in",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"params":json} (plug-in parameters under "params" or at the top level; see plugin.list)"##,
            enabled: crate::filters::has_filterable_layer,
            run,
            journal: true,
        },
    ]
}

#[cfg(test)]
#[path = "plugin_cmds/tests.rs"]
mod tests;
