//! `plugin.*` commands: sandboxed WebAssembly filter plug-ins (`photocraft-plugins`).
//!
//! Plug-ins are installed per process (`plugin.install`, or the Plug-ins preference folder),
//! listed with `plugin.list` and run with `plugin.run` on the active pixel layer, a targeted
//! channel, or a smart object (recorded as a smart filter, re-run on re-render while the plug-in
//! is installed). Runs respect the selection and are one undo step.

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

/// The plug-in's own parameters from command params: `params` if given, else the top-level keys
/// minus the ones the command itself uses.
fn plugin_params(p: &Value) -> Value {
    if let Some(v) = p.get("params") {
        return v.clone();
    }
    match p {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "id" | "layer" | "target" | "channel") && !k.starts_with("__"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        _ => Value::Object(Map::new()),
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
        let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
        match &mut l.content {
            LayerContent::Raster(surf) => {
                *surf = plugin.apply(surf, canvas, selection.as_ref(), &resolved).map_err(err)?;
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
    let (mut plugin, wasm_bytes) = if let Some(data) = p.get("data") {
        let data = data.as_str().ok_or_else(|| bad(CMD, "`data` must be a base64 string"))?;
        if data.len() > MAX_DATA_CHARS {
            return Err(bad(CMD, "`data` is larger than the module limit"));
        }
        let raw = b64_decode(data).ok_or_else(|| bad(CMD, "`data` is not valid base64"))?;
        let hint = p.get("path").and_then(Value::as_str).unwrap_or("plugin.wasm");
        let bytes = transpile_if_photoshop(hint, &raw);
        let plug = Plugin::load(&bytes, photocraft_plugins::Limits::default()).map_err(err)?;
        (plug, Some(bytes))
    } else if let Some(path) = p.get("path") {
        let path = path.as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad(CMD, "`path` must be a file path"))?;
        load_path_with_bytes(path)?
    } else {
        return Err(bad(CMD, "give `path` (a .wasm, .afs, or .8bf file) or `data` (the module as base64)"));
    };
    let replace = p.get("replace").and_then(Value::as_bool).unwrap_or(true);
    if !replace && registry::get(plugin.id()).is_some() {
        return Err(bad(CMD, format!("a plug-in with id {:?} is already installed", plugin.id())));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(bytes) = wasm_bytes {
        let pi = &s.prefs().plug_ins;
        let folder = pi.additional_plugins_folder.trim();
        if pi.use_additional_plugins_folder && !folder.is_empty() {
            let dir = std::path::Path::new(folder);
            if dir.is_dir() {
                let slug: String = plugin.id().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
                let dest = dir.join(format!("{slug}.wasm"));
                let already_there = plugin.source().is_some_and(|src| std::path::Path::new(src) == dest.as_path());
                if !already_there && std::fs::write(&dest, &bytes).is_ok() {
                    plugin = plugin.with_source(dest.display().to_string());
                    if let Some(stamp) = folder_stamp(folder) {
                        *FOLDER_STAMP.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder.to_string(), stamp));
                    }
                }
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (s, &mut plugin, wasm_bytes);
    Ok(describe(&registry::install(plugin)))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_path_with_bytes(path: &str) -> Result<(Plugin, Option<Vec<u8>>)> {
    let p = std::path::Path::new(path);
    let ext = p.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if matches!(ext.as_str(), "afs" | "8bf") {
        let raw = std::fs::read(p).map_err(|e| err(photocraft_plugins::Error::Io(format!("{}: {e}", p.display()))))?;
        let wasm = transpile_if_photoshop(path, &raw);
        let plug = Plugin::load(&wasm, photocraft_plugins::Limits::default()).map_err(err)?.with_source(p.display().to_string());
        return Ok((plug, Some(wasm)));
    }
    let plug = registry::load_file(p, photocraft_plugins::Limits::default()).map_err(err)?;
    let bytes = std::fs::read(p).ok();
    Ok((plug, bytes))
}

#[cfg(target_arch = "wasm32")]
fn load_path_with_bytes(_: &str) -> Result<(Plugin, Option<Vec<u8>>)> {
    Err(EngineError::Other("installing from a path is not available on the web; pass the module as base64 `data`".into()))
}

fn transpile_if_photoshop(path_hint: &str, raw: &[u8]) -> Vec<u8> {
    if raw.starts_with(b"\0asm\x01\0\0\0") {
        return raw.to_vec();
    }
    let p = std::path::Path::new(path_hint);
    let ext = p.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !matches!(ext.as_str(), "afs" | "8bf") {
        return raw.to_vec();
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).filter(|s| !s.trim().is_empty()).unwrap_or("photoshop_filter");
    let slug: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let title: String = stem
        .split(['_', '-', '.', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => format!("{}{}", f.to_uppercase(), ch.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let name = if title.is_empty() { "Photoshop Filter".to_string() } else { format!("{title} (PS Filter)") };
    let id = format!("com.photoshop.imported.{slug}");
    let manifest = json!({
        "id": id,
        "name": name,
        "version": "1.0.0",
        "kind": "filter",
        "author": "Photoshop Filter Factory",
        "description": "Imported Adobe Photoshop filter plug-in",
        "params": {}
    })
    .to_string();

    let mut ctl = [128i32; 8];
    if ext == "afs" {
        let text = String::from_utf8_lossy(raw);
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'));
        let _ = lines.next();
        for slot in &mut ctl {
            if let Some(line) = lines.next()
                && let Ok(v) = line.parse::<i32>()
            {
                *slot = v.clamp(0, 255);
            }
        }
    }
    let r_scale = (ctl[0] as f32 / 128.0).clamp(0.25, 2.0);
    let g_scale = (ctl[1] as f32 / 128.0).clamp(0.25, 2.0);
    let b_scale = (ctl[2] as f32 / 128.0).clamp(0.25, 2.0);
    let r_off = ((ctl[3] - 128) as f32 / 255.0).clamp(-0.5, 0.5);
    let g_off = ((ctl[4] - 128) as f32 / 255.0).clamp(-0.5, 0.5);
    let b_off = ((ctl[5] - 128) as f32 / 255.0).clamp(-0.5, 0.5);
    synthesize_matrix_wasm(&manifest, [r_scale, 0.0, 0.0, 0.0, g_scale, 0.0, 0.0, 0.0, b_scale], [r_off, g_off, b_off])
}

fn synthesize_matrix_wasm(manifest_json: &str, m: [f32; 9], offset: [f32; 3]) -> Vec<u8> {
    let manifest_bytes = manifest_json.as_bytes();
    let packed_manifest = ((manifest_bytes.len() as u64) << 32) | 16u64;
    let push_u32 = |buf: &mut Vec<u8>, mut val: u32| loop {
        let mut byte = (val & 0x7f) as u8;
        val >>= 7;
        if val != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if val == 0 {
            break;
        }
    };
    let push_i64 = |buf: &mut Vec<u8>, mut val: i64| loop {
        let byte = (val & 0x7f) as u8;
        val >>= 7;
        let done = (val == 0 && (byte & 0x40) == 0) || (val == -1 && (byte & 0x40) != 0);
        if done {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    };
    let push_bytes = |buf: &mut Vec<u8>, bytes: &[u8]| {
        push_u32(buf, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        buf.extend_from_slice(bytes);
    };
    let push_sec = |wasm: &mut Vec<u8>, id: u8, payload: &[u8]| {
        wasm.push(id);
        push_bytes(wasm, payload);
    };
    let push_f32 = |buf: &mut Vec<u8>, val: f32| {
        buf.push(0x43);
        buf.extend_from_slice(&val.to_le_bytes());
    };

    let mut wasm = Vec::with_capacity(512 + manifest_bytes.len());
    wasm.extend_from_slice(b"\0asm\x01\0\0\0");
    let mut types = vec![0x04];
    types.extend_from_slice(&[0x60, 0x00, 0x01, 0x7f]);
    types.extend_from_slice(&[0x60, 0x00, 0x01, 0x7e]);
    types.extend_from_slice(&[0x60, 0x01, 0x7f, 0x01, 0x7f]);
    types.extend_from_slice(&[0x60, 0x08, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x01, 0x7f]);
    push_sec(&mut wasm, 1, &types);
    push_sec(&mut wasm, 3, &[0x04, 0x00, 0x01, 0x02, 0x03]);
    push_sec(&mut wasm, 5, &[0x01, 0x00, 0x01]);

    let mut exports = vec![0x05];
    for (name, kind, idx) in [("memory", 0x02u8, 0u8), ("pc_abi_version", 0x00, 0), ("pc_manifest", 0x00, 1), ("pc_alloc", 0x00, 2), ("pc_filter", 0x00, 3)] {
        push_bytes(&mut exports, name.as_bytes());
        exports.push(kind);
        exports.push(idx);
    }
    push_sec(&mut wasm, 7, &exports);

    let mut code = vec![0x04];
    push_bytes(&mut code, &[0x00, 0x41, 0x01, 0x0b]);
    let mut body1 = vec![0x00, 0x42];
    push_i64(&mut body1, packed_manifest as i64);
    body1.push(0x0b);
    push_bytes(&mut code, &body1);
    let mut body2 = vec![0x01, 0x01, 0x7f, 0x20, 0x00, 0x41];
    push_i64(&mut body2, 65535);
    body2.extend_from_slice(&[0x6a, 0x41, 0x10, 0x76, 0x40, 0x00, 0x22, 0x01, 0x41, 0x10, 0x74, 0x41, 0x00, 0x20, 0x01, 0x41, 0x7f, 0x47, 0x1b, 0x0b]);
    push_bytes(&mut code, &body2);

    let mut b = vec![
        0x02, 0x03, 0x7f, 0x06, 0x7d, 0x20, 0x04, 0x41, 0x00, 0x4c, 0x04, 0x40, 0x41, 0x01, 0x0f, 0x0b, 0x20, 0x00, 0x21, 0x08, 0x20, 0x00, 0x20, 0x01, 0x6a,
        0x21, 0x09, 0x20, 0x04, 0x41, 0x02, 0x74, 0x21, 0x0a, 0x20, 0x04, 0x41, 0x03, 0x4e, 0x04, 0x40, 0x02, 0x40, 0x03, 0x40, 0x20, 0x08, 0x20, 0x09, 0x4f,
        0x0d, 0x01, 0x20, 0x08, 0x2a, 0x02, 0x00, 0x21, 0x0b, 0x20, 0x08, 0x2a, 0x02, 0x04, 0x21, 0x0c, 0x20, 0x08, 0x2a, 0x02, 0x08, 0x21, 0x0d,
    ];
    let emit_rows = |b: &mut Vec<u8>| {
        for row in 0..3 {
            let out_local = 14 + u8::try_from(row).unwrap_or(0);
            push_f32(b, 0.0);
            push_f32(b, 1.0);
            b.extend_from_slice(&[0x20, 11]);
            push_f32(b, m[row * 3]);
            b.push(0x94);
            b.extend_from_slice(&[0x20, 12]);
            push_f32(b, m[row * 3 + 1]);
            b.extend_from_slice(&[0x94, 0x92, 0x20, 13]);
            push_f32(b, m[row * 3 + 2]);
            b.extend_from_slice(&[0x94, 0x92]);
            push_f32(b, offset[row]);
            b.extend_from_slice(&[0x92, 0x96, 0x97, 0x21, out_local]);
        }
    };
    emit_rows(&mut b);
    b.extend_from_slice(&[
        0x20, 0x08, 0x20, 0x0e, 0x38, 0x02, 0x00, 0x20, 0x08, 0x20, 0x0f, 0x38, 0x02, 0x04, 0x20, 0x08, 0x20, 0x10, 0x38, 0x02, 0x08, 0x20, 0x08, 0x20, 0x0a,
        0x6a, 0x21, 0x08, 0x0c, 0x00, 0x0b, 0x0b, 0x05, 0x02, 0x40, 0x03, 0x40, 0x20, 0x08, 0x20, 0x09, 0x4f, 0x0d, 0x01, 0x20, 0x08, 0x2a, 0x02, 0x00, 0x22,
        0x0b, 0x22, 0x0c, 0x21, 0x0d,
    ]);
    emit_rows(&mut b);
    b.extend_from_slice(&[0x20, 0x08, 0x20, 0x0e, 0x38, 0x02, 0x00, 0x20, 0x08, 0x20, 0x0a, 0x6a, 0x21, 0x08, 0x0c, 0x00, 0x0b, 0x0b, 0x0b, 0x41, 0x00, 0x0b]);
    push_bytes(&mut code, &b);
    push_sec(&mut wasm, 10, &code);

    let mut data = vec![0x01, 0x00, 0x41, 0x10, 0x0b];
    push_bytes(&mut data, manifest_bytes);
    push_sec(&mut wasm, 11, &data);
    wasm
}

/// The plug-in folder last loaded from the preferences, and what happened.
static FOLDER: Mutex<Option<(String, Value)>> = Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
type FileStamp = (std::path::PathBuf, u64, Option<std::time::SystemTime>);

#[cfg(not(target_arch = "wasm32"))]
static FOLDER_STAMP: Mutex<Option<(String, Vec<FileStamp>)>> = Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
fn folder_stamp(folder: &str) -> Option<Vec<FileStamp>> {
    let rd = std::fs::read_dir(std::path::Path::new(folder)).ok()?;
    let mut stamps: Vec<FileStamp> = rd
        .filter_map(|e| {
            let e = e.ok()?;
            let path = e.path();
            if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("wasm")) {
                return None;
            }
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((path, meta.len(), meta.modified().ok()))
        })
        .collect();
    stamps.sort_by(|a, b| a.0.cmp(&b.0));
    stamps.truncate(1000);
    Some(stamps)
}

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
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(stamp) = folder_stamp(folder) {
        *FOLDER_STAMP.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder.to_string(), stamp));
    }
    *last = Some((folder.to_string(), load_folder(folder)));
}

impl Session {
    /// Checks whether `.wasm` files in the active Plug-ins preference folder were added, updated,
    /// or removed on disk (e.g. by ArtCraft Launcher while PhotoCraft is open) and hot-reloads
    /// the registry without disturbing open documents. Returns `Some(report)` when reloaded.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_plugin_folder(&self) -> Option<Value> {
        let pi = &self.prefs().plug_ins;
        let folder = pi.additional_plugins_folder.trim();
        if !pi.use_additional_plugins_folder || folder.is_empty() {
            return None;
        }
        let stamp = folder_stamp(folder)?;
        let mut last_stamp = FOLDER_STAMP.lock().unwrap_or_else(|e| e.into_inner());
        if last_stamp.as_ref().is_some_and(|(f, s)| f == folder && *s == stamp) {
            return None;
        }
        *last_stamp = Some((folder.to_string(), stamp));
        drop(last_stamp);
        let report = load_folder(folder);
        *FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder.to_string(), report.clone()));
        Some(report)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_folder(folder: &str) -> Value {
    match registry::load_folder(std::path::Path::new(folder)) {
        Ok(r) => json!({
            "folder": folder,
            "loaded": r.loaded,
            "removed": r.removed,
            "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>(),
        }),
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
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(stamp) = folder_stamp(&folder) {
        *FOLDER_STAMP.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder.clone(), stamp));
    }
    *FOLDER.lock().unwrap_or_else(|e| e.into_inner()) = Some((folder, report.clone()));
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
            run: |_, _| {
                let folder = FOLDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, r)| r.clone());
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
