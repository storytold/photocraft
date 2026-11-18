//! `plugin.*` commands: sandboxed WebAssembly plug-ins.
//!
//! Filter plug-ins (`photocraft-plugins`, PhotoCraft's own kind) are installed per process
//! (`plugin.install` with a `.wasm` module, the Plug-ins preference folder, or a `"kind": "filter"`
//! bundle from the user's `Plug-ins` folder), listed with `plugin.list` and run with `plugin.run`
//! on the active pixel layer, a targeted channel, or a smart object (recorded as a smart filter,
//! re-run on re-render while the plug-in is installed). Runs respect the selection and are one
//! undo step.
//!
//! Command plug-ins (the kind every Craft app shares, RFC-0046) come in `.craftplugin` bundles
//! from a file, a GitHub repository or a catalog, and `plugin.run` runs one as one undo step that
//! calls other commands; see [`crate::plugin`]. `plugin.update` and `plugin.catalog.*` manage
//! bundles.

use std::sync::Mutex;

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

/// Whether `plugin.run` params `p` name an installed command plug-in (not a filter).
pub fn names_command_plugin(s: &Session, p: &Value) -> bool {
    p.get("id").and_then(Value::as_str).is_some_and(|id| s.plugins.command(id).is_some())
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

/// A top-level `plugin.run` key of a command plug-in's run (its command, budget, coalescing key),
/// not one of its parameters.
fn command_plugin_key(k: &str) -> bool {
    command_key(k) || matches!(k, "command" | "fuel" | "coalesce")
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
    if names_command_plugin(s, p) {
        return run_command_plugin(s, p);
    }
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

/// `plugin.run` of a command plug-in: `command` (default: its first) with its parameters under
/// `params` or at the top level, and an optional lower instruction budget `fuel`.
fn run_command_plugin(s: &mut Session, p: &Value) -> Result<Value> {
    let id = p.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let command = match p.get("command") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad(RUN, "`command` must be text (a command of the plug-in)"))?.to_string()),
    };
    let fuel = p.get("fuel").and_then(Value::as_f64).filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64);
    let params = match p.get("params") {
        Some(v) => v.clone(),
        None => match p {
            Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| !command_plugin_key(k)).map(|(k, v)| (k.clone(), v.clone())).collect()),
            _ => Value::Null,
        },
    };
    crate::plugin::run(s, &id, command.as_deref(), &params, fuel)
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
    // plugin.install historically accepts both alphabet variants, including mixed input.
    // Keep that input policy while the library validates padding and decodes the bytes.
    photocraft_paint::tile::b64_decode(&s.replace('-', "+").replace('_', "/"))
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.install";
    let plugin = if let Some(data) = p.get("data") {
        let data = data.as_str().ok_or_else(|| bad(CMD, "`data` must be a base64 string"))?;
        if data.len() > MAX_DATA_CHARS {
            return Err(bad(CMD, "`data` is larger than the module limit"));
        }
        let bytes = b64_decode(data).ok_or_else(|| bad(CMD, "`data` is not valid base64"))?;
        if crate::plugin::is_bundle(&bytes) {
            return install_bundle(s, p, Some(bytes));
        }
        Plugin::load(&bytes, photocraft_plugins::Limits::default()).map_err(err)?
    } else if let Some(path) = p.get("path") {
        let path = path.as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad(CMD, "`path` must be a file path"))?;
        if crate::plugin::is_bundle_path(path) {
            return install_bundle(s, p, None);
        }
        load_path(path)?
    } else if p.get("repo").is_some() || p.get("catalog").is_some() {
        return install_bundle(s, p, None);
    } else {
        return Err(bad(
            CMD,
            "give `path` (a .wasm or .craftplugin file), `data` (a module or bundle as base64), `repo` (owner/repo or owner/repo@tag) or `catalog` with `id`",
        ));
    };
    let replace = p.get("replace").and_then(Value::as_bool).unwrap_or(true);
    if !replace && registry::get(plugin.id()).is_some() {
        return Err(bad(CMD, format!("a plug-in with id {:?} is already installed", plugin.id())));
    }
    Ok(describe(&registry::install(plugin)))
}

/// `plugin.install` of a `.craftplugin` bundle (a command plug-in or a filter): checked and
/// described without installing when `review` is set (the install confirmation), installed only as
/// the reviewed bundle when `sha256` is given; `data` is the decoded `data` parameter.
fn install_bundle(s: &mut Session, p: &Value, data: Option<Vec<u8>>) -> Result<Value> {
    const CMD: &str = "plugin.install";
    let mut request: Map<String, Value> = p.as_object().cloned().unwrap_or_default();
    request.remove("review");
    request.remove("sha256");
    let request = Value::Object(request);
    let wanted = match p.get("sha256") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad(CMD, "`sha256` must be text (the reviewed bundle's SHA-256)"))?.to_string()),
    };
    let review = p.get("review").and_then(Value::as_bool).unwrap_or(false);
    let c = match wanted.as_deref().and_then(|h| crate::plugin::take_held(s, h, &request)) {
        Some(c) => c,
        None => crate::plugin::obtain(s, CMD, p, data)?,
    };
    let (manifest, mut info) = crate::plugin::review(s, CMD, &c)?;
    let sha256 = info["sha256"].as_str().unwrap_or_default().to_string();
    if let Some(w) = &wanted
        && !w.eq_ignore_ascii_case(&sha256)
    {
        return Err(bad(CMD, format!("the bundle's SHA-256 is {sha256}, not the {w} that was reviewed: review it again")));
    }
    let replace = p.get("replace").and_then(Value::as_bool).unwrap_or(true);
    if !replace && (s.plugins.get(&manifest.id).is_some() || registry::get(&manifest.id).is_some()) {
        return Err(bad(CMD, format!("a plug-in with id {:?} is already installed", manifest.id)));
    }
    if review {
        crate::plugin::hold(s, &sha256, request, c);
        info["installed"] = json!(false);
        return Ok(info);
    }
    crate::plugin::install(s, CMD, c)?;
    info["installed"] = json!(true);
    Ok(info)
}

#[cfg(not(target_arch = "wasm32"))]
fn load_path(path: &str) -> Result<Plugin> {
    registry::load_file(std::path::Path::new(path), photocraft_plugins::Limits::default()).map_err(err)
}

#[cfg(target_arch = "wasm32")]
fn load_path(_: &str) -> Result<Plugin> {
    Err(EngineError::Other("installing from a path is not available on the web; pass the module as base64 `data`".into()))
}

/// The plug-in folder last loaded from the preferences, and what happened.
static FOLDER: Mutex<Option<(String, Value)>> = Mutex::new(None);

/// Loads the Plug-ins preference folder (`*.wasm`) when it is enabled and has changed since the
/// last load. Called whenever preferences are applied (startup, `prefs.set`). Native only.
pub(crate) fn sync_prefs(s: &Session) {
    let pi = &s.prefs().plug_ins;
    let folder = pi.additional_plugins_folder.trim();
    if !pi.use_additional_plugins_folder || folder.is_empty() {
        return;
    }
    let mut last = FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref().is_some_and(|(f, _)| f == folder) {
        return;
    }
    *last = Some((folder.to_string(), load_folder(folder)));
}

#[cfg(not(target_arch = "wasm32"))]
fn load_folder(folder: &str) -> Value {
    match registry::load_folder(std::path::Path::new(folder)) {
        Ok(r) => json!({"folder": folder, "loaded": r.loaded, "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>()}),
        Err(e) => json!({"folder": folder, "error": e.to_string()}),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_folder(folder: &str) -> Value {
    json!({"folder": folder, "error": "plug-in folders are not available on the web"})
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    let folder = match p.get("path") {
        Some(v) => v.as_str().filter(|f| !f.trim().is_empty()).ok_or_else(|| bad("plugin.reload", "`path` must be a folder"))?.trim().to_string(),
        None => s.prefs().plug_ins.additional_plugins_folder.trim().to_string(),
    };
    if folder.is_empty() {
        return Err(bad("plugin.reload", "no folder: pass `path` or set Preferences › Plug-ins › Additional Plug-ins Folder"));
    }
    let report = load_folder(&folder);
    if let Some(e) = report.get("error").and_then(Value::as_str) {
        return Err(EngineError::Other(e.to_string()));
    }
    *FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder, report.clone()));
    Ok(report)
}

/// `plugin.list`: the filters (the registry), then the command plug-ins; bundle plug-ins say
/// where they came from and their SHA-256.
fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let folder = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, r)| r.clone());
    let mut plugins: Vec<Value> = registry::list()
        .iter()
        .map(|p| {
            let mut v = describe(p);
            if let Some(b) = s.plugins.get(p.id()).filter(|b| !b.is_command()) {
                v["sha256"] = json!(b.sha256);
                v["origin"] = b.source.clone();
                v["stored"] = json!(b.stored);
            }
            v
        })
        .collect();
    plugins.extend(s.plugins.commands().map(crate::plugin::Plugin::describe));
    let last = s.plugins.last.as_ref().map(|(id, command, _)| json!({"id": id, "command": command}));
    Ok(json!({
        "plugins": plugins,
        "folder": folder,
        "problems": s.plugins.problems(),
        "catalogs": s.plugins.saved.catalogs,
        "store": s.plugins.store_dir(),
        "last": last,
    }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.remove";
    let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad(CMD, "missing `id`"))?.to_string();
    if crate::plugin::remove(s, CMD, &id)? {
        return Ok(json!({"removed": id}));
    }
    if registry::remove(&id) { Ok(json!({"removed": id})) } else { Err(err(photocraft_plugins::Error::NotFound(id))) }
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.update";
    let only = match p.get("id") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad(CMD, "`id` must be text"))?.to_string()),
    };
    if let Some(id) = &only
        && s.plugins.get(id).is_none()
    {
        return Err(bad(CMD, format!("no plug-in `{id}` was installed from a bundle (see plugin.list)")));
    }
    let apply = p.get("apply").and_then(Value::as_bool).unwrap_or(false);
    let targets: Vec<crate::plugin::Plugin> =
        s.plugins.list().iter().filter(|x| only.as_deref().is_none_or(|id| x.manifest.id.eq_ignore_ascii_case(id))).cloned().collect();
    let (mut updates, mut updated, mut errors) = (vec![], vec![], vec![]);
    for old in targets {
        let id = old.manifest.id.clone();
        let (params, available) = match crate::plugin::update_for(s, &old) {
            Ok(Some(found)) => found,
            Ok(None) => continue,
            Err(e) => {
                errors.push(json!({"id": id, "error": e}));
                continue;
            }
        };
        let mut entry = json!({"id": id, "installed": old.manifest.version, "available": available, "install": params});
        if apply {
            // An update that asks for other permissions waits for its confirmation (`plugin.install`).
            let done = crate::plugin::obtain(s, CMD, &params, None).and_then(|c| {
                let (_, info) = crate::plugin::review(s, CMD, &c)?;
                if info["permissionsChanged"].as_bool().unwrap_or(true) {
                    return Ok(false);
                }
                crate::plugin::install(s, CMD, c).map(|_| true)
            });
            match done {
                Ok(true) => updated.push(json!(id)),
                Ok(false) => entry["permissionsChanged"] = json!(true),
                Err(e) => errors.push(json!({"id": id, "error": e.to_string()})),
            }
        }
        updates.push(entry);
    }
    Ok(json!({"updates": updates, "updated": updated, "errors": errors}))
}

fn catalog_list(s: &mut Session, p: &Value) -> Result<Value> {
    let urls = match p.get("url") {
        None | Some(Value::Null) => s.plugins.saved.catalogs.clone(),
        Some(v) => vec![v.as_str().ok_or_else(|| bad("plugin.catalog.list", "`url` must be text"))?.to_string()],
    };
    let (mut packages, mut warnings) = (vec![], vec![]);
    for url in &urls {
        match crate::plugin::catalog_packages(s, url) {
            Ok((found, w)) => {
                packages.extend(found);
                warnings.extend(w.into_iter().map(|w| format!("{url}: {w}")));
            }
            Err(e) => warnings.push(format!("{url}: {e}")),
        }
    }
    Ok(json!({"catalogs": urls, "packages": packages, "warnings": warnings}))
}

fn catalog_add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.catalog.add";
    let url = p.get("url").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| bad(CMD, "`url` required"))?.to_string();
    let remove = p.get("remove").and_then(Value::as_bool).unwrap_or(false);
    let listed = s.plugins.saved.catalogs.contains(&url);
    if remove && listed {
        s.plugins.saved.catalogs.retain(|u| *u != url);
        s.prefs.edit(|_| ());
    } else if !remove && !listed {
        crate::plugin::check_catalog_url(s, &url).map_err(|e| bad(CMD, e))?;
        if s.plugins.saved.catalogs.len() >= crate::plugin::MAX_CATALOGS {
            return Err(bad(CMD, format!("at most {} catalogs", crate::plugin::MAX_CATALOGS)));
        }
        s.plugins.saved.catalogs.push(url);
        // The preferences revision moves, so the app saves the list.
        s.prefs.edit(|_| ());
    }
    Ok(json!({"catalogs": s.plugins.saved.catalogs}))
}

/// The plug-in command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "plugin.list",
            label: "List Plug-ins",
            menu: &[],
            shortcut: None,
            params: r##"{} → {plugins: [filters {id, name, version, kind, params, paramsSchema, overlap, area, size, source, sha256?, origin?, stored?}, then command plug-ins {id, name, version, kind: "command", permissions, commands, sha256, origin, stored}], folder (the preference folder's last load), problems, catalogs, store (the Plug-ins folder, or null: this session only), last}"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "plugin.install",
            label: "Install Plug-in…",
            menu: &["Filter", "Plug-ins"],
            shortcut: None,
            params: r##"{"path":text,"data":json,"replace":bool=true} (path: a .wasm filter module, or a .craftplugin bundle, native only; data: either as base64) | {"repo":text} (owner/repo or owner/repo@tag: its release's .craftplugin, checked against SHA256SUMS) | {"catalog":text,"id":text,"version":text} (an index.json URL and one of its packages; default: the newest for PhotoCraft) — a bundle also takes "review":bool (check and describe it without installing: the confirmation), "sha256":text (install only that reviewed bundle) and "name":text (with data) → a .wasm: the filter's description; a bundle: {id, name, version, kind, permissions, commands, sha256, origin, inCatalog, replaces, permissionsChanged, stored, installed}"##,
            enabled: always,
            run: install,
            journal: false,
        },
        CommandSpec {
            id: "plugin.update",
            label: "Update Plug-ins",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"apply":bool=false} (id: one bundle plug-in, default all; apply: install the updates that ask for no other permissions) → {updates: [{id, installed, available, install (plugin.install params), permissionsChanged?}], updated, errors}"##,
            enabled: always,
            run: update,
            journal: false,
        },
        CommandSpec {
            id: "plugin.catalog.list",
            label: "Plug-in Catalogs",
            menu: &[],
            shortcut: None,
            params: r##"{"url":text} (one catalog; default: every catalog added) → {catalogs, packages: [{catalog, id, name, version, author, description, license, repo, kind, sha256, installed}], warnings}"##,
            enabled: always,
            run: catalog_list,
            journal: false,
        },
        CommandSpec {
            id: "plugin.catalog.add",
            label: "Add Plug-in Catalog",
            menu: &[],
            shortcut: None,
            params: r##"{"url":text,"remove":bool=false} (an index.json on GitHub over https://, or file://) → {catalogs}"##,
            enabled: always,
            run: catalog_add,
            journal: false,
        },
        CommandSpec {
            id: "plugin.remove",
            label: "Remove Plug-in",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text} (a bundle plug-in leaves the Plug-ins folder, with its settings)"##,
            enabled: always,
            run: remove,
            journal: false,
        },
        CommandSpec {
            id: "plugin.reload",
            label: "Reload Plug-ins",
            menu: &[],
            shortcut: None,
            params: r##"{"path":text} (a folder of .wasm plug-ins; default: the Plug-ins preference folder)"##,
            enabled: always,
            run: reload,
            journal: false,
        },
        CommandSpec {
            id: RUN,
            label: "Run Plug-in",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"params":json,"command":text,"fuel":int} (plug-in parameters under "params" or at the top level; see plugin.list. A command plug-in runs `command`, default its first, as one undo step, rolled back when it fails, optionally with a lower instruction budget `fuel` → {plugin, command, output, notes, logs, calls, fuel, panel})"##,
            enabled: crate::filters::has_filterable_layer,
            run,
            journal: true,
        },
    ]
}

#[cfg(test)]
#[path = "plugin_cmds/tests.rs"]
mod tests;
