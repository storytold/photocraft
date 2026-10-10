//! Programmatic control of the running app, for agents, tests and (soon) MCP.
//!
//! Transport-agnostic: a transport thread (TCP in the desktop app, a channel in tests) sends
//! [`ControlRequest`]s; the UI thread handles them between frames and replies with JSON.
//!
//! Methods:
//! - `engine.execute {command, params}`: run any engine or UI command by id
//! - `engine.commands`: list commands with enablement
//! - `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size); the menu
//!   tree is `ui.menu.list`
//! - `ui.set {tool?, panels?, dock?, dockTabs?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, selectionMode?, zoom?, center?, rotation?, fit?, theme?, brushSection?, brushTab?, brushesView?, brushPicker?, brushPickerView?, brushSize?}`:
//!   change UI state; any other field is an error ([`UI_SET_FIELDS`])
//! - `ui.dialog.open {kind, fields?}` (kinds: newDocument, about, layerStyle {effect?}, colorPicker {target: foreground|background}, command {command}) / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`
//! - `ui.dialog.apply {dialog}`: commit Preferences changes without closing the dialog
//! - `ui.window.open {document?}` / `ui.window.close {window}`: extra document windows
//! - `ui.pointer {events: [{kind: down|move|up, x, y, pressure?, tiltX?, tiltY?, rotation?}], modifiers?, button?}`: drive the active tool in document coordinates (`button: "secondary"` opens the tool's canvas context menu or Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase)
//! - `ui.click {x, y, button?, count?}` / `ui.move {x, y}`: synthetic pointer input in screen points
//!   (`count` at most [`MAX_CLICKS`])
//! - `ui.key {key, command?, shift?, alt?, ctrl?}` / `ui.type {text}`: synthetic keyboard input
//! - `ui.resize {width, height}`: resize the main window
//! - `ui.gpu.simulateLoss {error?}`: act as if the wgpu device was lost (or, with `error: true`,
//!   reported an error): the app switches to the CPU renderer for the rest of the session, as on a
//!   real loss. For testing the fallback; returns whether a GPU canvas was active
//! - `ui.screenshot {path?, focus?}`: capture the main window (PNG). Raises the window first (default)
//!   because occluded macOS windows stop rendering
//! - `ui.focus`: bring the main window to the front
//! - `app.open {path}` / `app.save {path}`: relative file I/O under the automation roots; reply with `warnings`
//! - `app.quit`
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs (#210) with progress; cancel one (or all).
//!   `engine.execute`, `ui.menu.invoke` and `ui.dialog.confirm` wait for a command that runs as a
//!   job unless `wait: false` (then the reply is `{job, pending: true}`)

use std::sync::mpsc::Sender;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

pub type ControlResponse = Value;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<ControlResponse>,
    /// Requests still queued at this instant are rejected; already dispatched work continues.
    pub deadline: Option<std::time::Instant>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx, deadline: None }, rx)
    }
}

pub enum Outcome {
    Done(Value),
    Screenshot {
        token: u64,
        path: Option<String>,
    },
    /// Synthetic input queued: reply once the app has processed all of it (so a following
    /// `ui.inspect`/`engine.execute` observes the effect).
    AfterInput,
    /// A command started a background job and the caller waits for it: reply with its result
    /// once it has been applied (or with its error / "cancelled").
    AfterJob(photocraft_engine::jobs::JobId),
}

/// The fields `ui.set` reads. Anything else is rejected before a field is applied, and every
/// field's value is validated before the first one is applied, so a typo, an unknown field, a
/// bad value or a bad nested key can't reply with success while nothing — or only half of it —
/// changed (#412).
pub const UI_SET_FIELDS: [&str; 29] = [
    "tool",
    "panels",
    "dock",
    "dockTabs",
    "dockWidth",
    "colorPanel",
    "maskTarget",
    "vectorMaskTarget",
    "selectionMode",
    "zoom",
    "center",
    "rotation",
    "fit",
    "theme",
    "brushSection",
    "brushTab",
    "brushesView",
    "brushPicker",
    "brushPickerView",
    "brushSize",
    "gradientBlendMode",
    "gradientClassic",
    "eyedropperSampleSize",
    "eyedropperSample",
    "eyedropperRing",
    "cropOverlay",
    "cropOverlayShow",
    "cropOverlayOrientation",
    "cropShield",
];

/// Most clicks one `ui.click` may queue (#982). Each click is a press and a release that the app
/// feeds in its own frame before replying, so `count` sizes both the input queue and the wait;
/// the same ceiling as the steps of one batch request.
pub const MAX_CLICKS: u64 = 256;

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}
fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

/// A `ui.set` boolean field: `Ok(None)` when absent, an error when present with another type.
fn bool_field(p: &Value, key: &str) -> std::result::Result<Option<bool>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("{key} must be a boolean")),
    }
}

/// A `ui.set` integer field: `Ok(None)` when absent; whole-number floats (`12.0`, as UIs send
/// them) are accepted, anything else is an error.
fn uint_field(p: &Value, key: &str) -> std::result::Result<Option<u64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) if v.is_u64() => Ok(v.as_u64()),
        Some(v) => v.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0).map(|f| Some(f as u64)).ok_or_else(|| format!("{key} must be a non-negative integer")),
    }
}

/// A `ui.set` numeric field: `Ok(None)` when absent, finite numbers only.
fn num_field(p: &Value, key: &str) -> std::result::Result<Option<f64>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("{key} must be a finite number")),
    }
}

/// A `ui.set` object field merged into the current state (e.g. `panels`): the patch's keys are
/// merged into the serialized current value and parsed back, so a non-object value or an
/// unknown nested key is an error — a nested typo must not reply ok while changing nothing.
fn merged_object<T: serde::Serialize + serde::de::DeserializeOwned>(current: &T, patch: Option<&Value>, field: &str) -> std::result::Result<Option<T>, String> {
    let Some(patch) = patch else { return Ok(None) };
    let Some(patch) = patch.as_object() else { return Err(format!("{field} must be an object")) };
    let mut cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let known: std::collections::HashSet<String> = cur.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    for (k, v) in patch {
        if !known.contains(k) {
            return Err(format!("unknown {field} field `{k}`"));
        }
        if let Some(c) = cur.as_object_mut() {
            c.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(cur).map(Some).map_err(|e| format!("{field}: {e}"))
}

/// A `ui.set` object field applied by whole-value replacement (e.g. `dock`, `colorPanel`):
/// unknown nested keys are rejected like [`merged_object`] does, but the value replaces the
/// state instead of merging into it.
fn whole_object<T: serde::Serialize + serde::de::DeserializeOwned>(current: &T, v: Option<&Value>, field: &str) -> std::result::Result<Option<T>, String> {
    let Some(v) = v else { return Ok(None) };
    let Some(patch) = v.as_object() else { return Err(format!("{field} must be an object")) };
    let cur = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let known: std::collections::HashSet<&str> = cur.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    for k in patch.keys() {
        if !known.contains(k.as_str()) {
            return Err(format!("unknown {field} field `{k}`"));
        }
    }
    serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("{field}: {e}"))
}
fn wrap(r: Result<Value, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

/// Run a command the way automation does (script events off). Long commands may run as
/// background jobs: by default the reply waits for the job's result (backward compatible); with
/// `wait` false it is `{job, pending: true}` at once.
fn run_waiting(app: &mut PhotocraftApp, wait: bool, run: impl FnOnce(&mut PhotocraftApp) -> Result<Value, String>) -> Outcome {
    let events_enabled = app.session.prefs().script_events.enabled;
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
    }
    app.jobs.last_started = None;
    let result = run(app);
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    }
    match (result, app.jobs.last_started.take()) {
        (Ok(_), Some(job)) if wait => Outcome::AfterJob(job),
        (result, _) => wrap(result),
    }
}

/// Screen position of document point (x, y) on the main canvas, where a `ui.pointer` right-click
/// opens its menu (the mouse's opens at the pointer); the canvas centre when it can't be mapped.
fn screen_point(app: &PhotocraftApp, x: f64, y: f64) -> [f32; 2] {
    let p = crate::canvas::ViewXform::active(app)
        .map(|xf| xf.to_screen(x as f32, y as f32))
        .filter(|p| p.is_finite())
        .unwrap_or_else(|| app.last_canvas_rect.center());
    [p.x, p.y]
}

/// The `dialog` id of the shell's own dialog (`workspace_ui`) in `ui.inspect` and `ui.dialog.*`.
const SHELL_DIALOG: &str = "shell";

pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let saved = app.session.authorize;
    if let Some(gate) = app.services.automation_authorize {
        app.session.authorize = Some(gate);
    }
    let outcome = dispatch(app, ctx, req);
    app.session.authorize = saved;
    outcome
}

fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let u = |k: &str| p.get(k).and_then(Value::as_u64);
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
    match req.method.as_str() {
        "ui.context.choose" => {
            let Some(id) = s("id") else { return err("missing `id`") };
            let Some(menu) = app.ui.canvas_tool_menu.as_ref() else { return err("no canvas context menu is open") };
            if !crate::canvas_tool_menu::available(app, menu, id) {
                return err("context action is unavailable");
            }
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &json!({}))
            {
                return err(error);
            }
            crate::canvas_tool_menu::choose(app, ctx, id);
            ok(json!({"command": id, "dialog": app.ui.dialogs.last().map(|d| d.id)}))
        }
        "engine.execute" | "ui.menu.invoke" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            // A control-channel menu click must respect the same modal gate as the native menu.
            // engine.execute is deliberately not subject to the UI's menu-click semantics.
            if req.method == "ui.menu.invoke" && !crate::menus::modal_allows(app, id) {
                return err("menu command is unavailable while a modal dialog is open");
            }
            let params = p.get("params").cloned().unwrap_or(json!({}));
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &params)
            {
                return err(error);
            }
            // `engine.execute` is programmatic: engine commands run directly with their default
            // params and never open a dialog (an agent would otherwise get a modal instead of a
            // result). `ui.menu.invoke` behaves like a menu click, so it may open the dialog.
            if req.method == "engine.execute" && photocraft_engine::commands::find(id).is_some() {
                return run_waiting(app, wait, |app| app.run(id, params));
            }
            run_waiting(app, wait, |app| crate::menus::invoke(app, ctx, id, params))
        }
        "engine.commands" => wrap(app.run("command.list", json!({}))),
        // Background jobs (#210): running ones with progress, then the last few that ended.
        "jobs.list" => wrap(app.session.execute("jobs.list", json!({})).map_err(|e| e.to_string())),
        "jobs.cancel" => {
            let all = p.get("job").is_none_or(Value::is_null);
            match p.get("job").and_then(Value::as_u64) {
                Some(j) if app.session.job(photocraft_engine::jobs::JobId(j)).is_some() => {
                    crate::jobs_ui::cancel(app, photocraft_engine::jobs::JobId(j));
                    ok(json!({"cancelled": 1}))
                }
                Some(j) => err(format!("no running job {j}")),
                None if all => {
                    let ids: Vec<_> = app.session.jobs().into_iter().map(|j| j.id).collect();
                    for j in &ids {
                        crate::jobs_ui::cancel(app, *j);
                    }
                    ok(json!({"cancelled": ids.len()}))
                }
                None => err("`job` must be a job id"),
            }
        }
        "ui.menu.list" => ok(serde_json::to_value(crate::menus::menu_items(app)).unwrap_or_default()),
        "ui.inspect" => ok(inspect(app, ctx)),
        "ui.set" => {
            if let Some(field) = p.as_object().and_then(|o| o.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str()))) {
                return err(format!("unknown field `{field}` (fields: {})", UI_SET_FIELDS.join(", ")));
            }
            let gradient_blend = if let Some(value) = p.get("gradientBlendMode") {
                let Some(name) = value.as_str() else { return err("gradientBlendMode must be a blend mode name") };
                let Some(mode) = photocraft_engine::commands::blend_from_str(name).filter(|m| photocraft_color::BlendMode::LAYER_MODES.contains(m)) else {
                    return err(format!("unknown gradient blend mode `{name}`"));
                };
                Some(mode)
            } else {
                None
            };
            // Everything is validated and materialized before the first field is applied, so a
            // rejected call applies none of its fields (#412's guarantee, down to the values
            // and nested keys).
            let applied = (|| -> std::result::Result<Value, String> {
                let tool = match s("tool") {
                    Some(t) => Tool::from_name(t).map(Some).ok_or_else(|| format!("unknown tool `{t}`"))?,
                    None => None,
                };
                let gradient_classic = bool_field(p, "gradientClassic")?;
                // The Eyedropper's options bar (#1649): Sample Size, Sample, Show Sampling Ring.
                let eyedropper_size = match p.get("eyedropperSampleSize") {
                    Some(v) => Some(
                        photocraft_engine::sample_cmds::size_param("ui.set", &json!({"size": v}))
                            .map_err(|_| format!("eyedropperSampleSize must be \"point\" or one of {:?}", photocraft_engine::sample_cmds::SAMPLE_SIZES))?,
                    ),
                    None => None,
                };
                let eyedropper_sample = match p.get("eyedropperSample") {
                    Some(v) => {
                        let ids: Vec<&str> = photocraft_engine::sample_cmds::SampleLayers::ALL.iter().map(|(_, n)| *n).collect();
                        let id = v.as_str().filter(|id| ids.contains(id)).ok_or_else(|| format!("eyedropperSample must be one of {}", ids.join(", ")))?;
                        Some(id.to_string())
                    }
                    None => None,
                };
                let eyedropper_ring = bool_field(p, "eyedropperRing")?;
                // The Crop tool's overlay menu (#1919).
                let crop_overlay = match p.get("cropOverlay") {
                    Some(v) => Some(v.as_str().and_then(crate::crop_overlay::CropOverlay::from_id).ok_or_else(|| {
                        let ids: Vec<&str> = crate::crop_overlay::CropOverlay::ALL.iter().map(|k| k.id()).collect();
                        format!("cropOverlay must be one of {}", ids.join(", "))
                    })?),
                    None => None,
                };
                let crop_overlay_show = match p.get("cropOverlayShow") {
                    Some(v) => Some(
                        v.as_str()
                            .and_then(crate::crop_overlay::OverlayShow::from_id)
                            .ok_or_else(|| "cropOverlayShow must be one of auto, always, never".to_string())?,
                    ),
                    None => None,
                };
                let crop_overlay_orientation = match uint_field(p, "cropOverlayOrientation")? {
                    Some(o) if o < 4 => Some(o as u8),
                    Some(_) => return Err("cropOverlayOrientation must be 0, 1, 2 or 3".into()),
                    None => None,
                };
                // The Crop tool's gear menu: Show Cropped Area and the shield (#1919).
                let crop_shield = merged_object(&app.ui.tool_options.crop_shield, p.get("cropShield"), "cropShield")?;
                if crop_shield.as_ref().is_some_and(|s| !(0.0..=100.0).contains(&s.opacity)) {
                    return Err("cropShield.opacity must be 0..100".into());
                }
                let panels = merged_object(&app.ui.panels, p.get("panels"), "panels")?;
                let mask_target = bool_field(p, "maskTarget")?;
                let vector_mask_target = bool_field(p, "vectorMaskTarget")?;
                let selection_mode = uint_field(p, "selectionMode")?;
                let dock_tabs = merged_object(&app.ui.dock_tabs, p.get("dockTabs"), "dockTabs")?;
                let dock = whole_object(&app.ui.dock, p.get("dock"), "dock")?;
                // Which chip the Color panel edits.
                let color_panel = whole_object(&app.ui.color_panel, p.get("colorPanel"), "colorPanel")?;
                let dock_width = num_field(p, "dockWidth")?;
                let zoom = num_field(p, "zoom")?;
                let rotation = num_field(p, "rotation")?;
                if rotation.is_some() && app.session.active_index().is_none() {
                    return Err("no document open".into());
                }
                let center = match p.get("center") {
                    Some(v) => {
                        let a = v.as_array().ok_or_else(|| "center must be [x, y]".to_string())?;
                        if a.len() != 2 {
                            return Err("center must be exactly [x, y]".into());
                        }
                        let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) else {
                            return Err("center must be [x, y] of two numbers".into());
                        };
                        Some([x as f32, y as f32])
                    }
                    None => None,
                };
                let fit = bool_field(p, "fit")? == Some(true);
                let theme = match s("theme") {
                    Some(name) => {
                        let kind = crate::theme::ThemeKind::from_name(name).ok_or_else(|| {
                            let names: Vec<_> = crate::theme::ThemeKind::ALL.iter().map(|k| k.id()).collect();
                            format!("unknown theme `{name}` ({})", names.join(", "))
                        })?;
                        Some(kind)
                    }
                    None => None,
                };
                let brush_section = uint_field(p, "brushSection")?;
                let brush_tab = uint_field(p, "brushTab")?;
                let brushes_view = match p.get("brushesView") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushesView: {e} (list, grid)"))?,
                    None => None,
                };
                let brush_picker_view = match p.get("brushPickerView") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushPickerView: {e} (list, grid)"))?,
                    None => None,
                };
                // `Some(None)` is an explicit null, which closes the picker (a missing field
                // leaves it alone).
                let brush_picker = match p.get("brushPicker") {
                    Some(v) => {
                        let at = serde_json::from_value::<Option<[f32; 2]>>(v.clone())
                            .map_err(|_| "brushPicker must be [x, y] in screen points, or null to close it".to_string())?;
                        if at.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
                            return Err("brushPicker must be [x, y] in screen points, or null to close it".into());
                        }
                        Some(at)
                    }
                    None => None,
                };
                // brushSize rides the journaled `tools.setBrush` command (#744). Numeric values
                // are validated here so the command cannot fail after other fields were
                // applied; non-numeric values keep the API's historical silent no-op.
                let brush_size: Option<f64> = match p.get("brushSize").and_then(Value::as_f64) {
                    Some(size) => {
                        if !size.is_finite() || size > f64::from(photocraft_paint::MAX_BRUSH_SIZE) {
                            return Err(format!("brushSize must be finite and at most {} px", photocraft_paint::MAX_BRUSH_SIZE));
                        }
                        Some(size)
                    }
                    None => None,
                };

                // Apply (nothing below can fail).
                if let Some(t) = tool {
                    app.ui.tool = t;
                    // Each tool keeps its own brush (#218), so switch it in before `brushSize`
                    // below sets the new tool's size.
                    crate::paint_mouse::sync_tool_brush(app);
                }
                let gradient_before = app.ui.tool_options.clone();
                if let Some(mode) = gradient_blend {
                    app.ui.tool_options.gradient_blend_mode = mode;
                }
                if let Some(classic) = gradient_classic {
                    app.ui.tool_options.gradient_classic = classic;
                }
                if gradient_blend.is_some() {
                    crate::gradient_ui::options_changed(app, &gradient_before);
                }
                if let Some(size) = eyedropper_size {
                    app.ui.tool_options.eyedropper_size = size;
                }
                if let Some(id) = eyedropper_sample {
                    app.ui.tool_options.eyedropper_sample = id;
                }
                if let Some(ring) = eyedropper_ring {
                    app.ui.tool_options.eyedropper_ring = ring;
                }
                if let Some(k) = crop_overlay {
                    app.ui.tool_options.crop_overlay = k;
                }
                if let Some(v) = crop_overlay_show {
                    app.ui.tool_options.crop_overlay_show = v;
                }
                if let Some(o) = crop_overlay_orientation {
                    app.ui.tool_options.crop_overlay_orientation = o;
                }
                if let Some(s) = crop_shield {
                    app.ui.tool_options.crop_shield = s;
                }
                if let Some(v) = panels {
                    app.ui.panels = v;
                }
                if let Some(m) = mask_target {
                    app.ui.mask_target = m;
                    app.ui.vector_mask_target &= !m;
                }
                if let Some(m) = vector_mask_target {
                    app.ui.vector_mask_target = m;
                    app.ui.mask_target &= !m;
                }
                // Selection tools' options-bar mode: 0 New, 1 Add, 2 Subtract, 3 Intersect.
                if let Some(m) = selection_mode {
                    app.ui.selection_mode = m.min(3) as u8;
                }
                if let Some(v) = dock_tabs {
                    app.ui.dock_tabs = v;
                }
                // Dock group order, heights and collapsed groups (see `dock::DockLayout`).
                if let Some(v) = dock {
                    app.ui.dock = v;
                }
                if let Some(v) = color_panel {
                    app.ui.color_panel = v;
                }
                // Right dock width in points (clamped to the dock's 250..=520 range), applied next frame.
                if let Some(w) = dock_width {
                    crate::panels::request_dock_width(ctx, w as f32);
                }
                if let Some(i) = app.session.active_index() {
                    if let Some(z) = zoom {
                        let size = app.session.documents().get(i).map_or([0, 0], |st| [st.doc.size.width, st.doc.size.height]);
                        app.ui.views[i].zoom = crate::zoom_levels::clamp(z as f32, size);
                        app.ui.views[i].fit_pending = false;
                        app.ui.views[i].fill_pending = false;
                    }
                    if let Some(c) = center {
                        app.ui.views[i].center = c;
                        app.ui.views[i].fit_pending = false;
                        app.ui.views[i].fill_pending = false;
                    }
                    if fit {
                        app.ui.views[i].fit_pending = true;
                        app.ui.views[i].fill_pending = false;
                    }
                    if let Some(r) = rotation {
                        app.ui.views[i].rotation = crate::rotate_view::wrap_deg(r as f32);
                    }
                }
                if let Some(k) = theme {
                    app.set_theme(ctx, k);
                }
                if let Some(i) = brush_section {
                    app.ui.brush_section = (i as usize).min(crate::brush_panel::SECTIONS.len() - 1);
                }
                if let Some(i) = brush_tab {
                    app.ui.brush_tab = (i as usize).min(1);
                }
                if let Some(v) = brushes_view {
                    app.ui.brushes_panel.view = v;
                }
                if let Some(v) = brush_picker_view {
                    app.ui.brush_picker_list.view = v;
                }
                if let Some(at) = brush_picker {
                    app.ui.brush_picker = at;
                }
                // Which chip the Color panel edits.
                if let Some(c) = color_panel {
                    app.ui.color_panel = c;
                }
                if let Some(size) = brush_size {
                    app.run("tools.setBrush", json!({"brush": {"size": size}})).map_err(|e| e.to_string())?;
                }
                Ok(Value::Null)
            })();
            wrap(applied)
        }
        "ui.dialog.open" => {
            let kind = match s("kind").unwrap_or("") {
                "newDocument" | "NewDocument" => DialogKind::NewDocument,
                "about" | "About" => DialogKind::About,
                "layerStyle" | "LayerStyle" => {
                    return match crate::layer_style::open(app, s("effect")) {
                        Some(id) => ok(json!({"dialog": id})),
                        None => err("no active layer"),
                    };
                }
                "colorPicker" | "ColorPicker" => {
                    let target = if s("target") == Some("background") { "background" } else { "foreground" };
                    return ok(json!({"dialog": crate::color_picker_ui::open(app, target)}));
                }
                "command" | "Command" => {
                    let Some(cmd) = s("command") else { return err("command dialogs need `command`") };
                    let label = photocraft_engine::commands::find(cmd).map(|c| c.label).unwrap_or(cmd);
                    return ok(json!({"dialog": crate::dialogs::open_command_dialog(app, cmd, label)}));
                }
                other => return err(format!("unknown dialog kind `{other}`")),
            };
            let mut fields = if kind == DialogKind::NewDocument { app.new_document_fields() } else { Default::default() };
            if let Some(f) = p.get("fields").and_then(Value::as_object) {
                fields.extend(f.clone());
            }
            ok(json!({"dialog": app.ui.open_dialog(kind, fields)}))
        }
        // The shell's own dialog (Window › Workspace, View › Pixel Aspect Ratio › Custom, Show
        // Extras Options, 32-bit Preview Options), listed by ui.inspect with the id "shell".
        "ui.dialog.set" if s("dialog") == Some(SHELL_DIALOG) => {
            let Some(field) = s("field") else { return err("need `dialog` and `field`") };
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            match app.ui.shell.dialog.as_mut() {
                Some((_, fields)) => {
                    fields.insert(field.to_string(), value);
                    ok(Value::Null)
                }
                None => err("no shell dialog is open"),
            }
        }
        "ui.dialog.confirm" if s("dialog") == Some(SHELL_DIALOG) => {
            let Some((kind, fields)) = app.ui.shell.dialog.clone() else { return err("no shell dialog is open") };
            if let Some((command, params)) = crate::workspace_ui::dialog_command(&kind, &fields)
                && let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(command, &params)
            {
                return err(error);
            }
            wrap(crate::workspace_ui::confirm(app, ctx))
        }
        "ui.dialog.apply" if s("dialog") == Some(SHELL_DIALOG) => err("`ui.dialog.apply` is for Preferences; use `ui.dialog.confirm`"),
        "ui.dialog.cancel" if s("dialog") == Some(SHELL_DIALOG) => match app.ui.shell.dialog.take() {
            Some(_) => ok(Value::Null),
            None => err("no such dialog"),
        },
        "ui.dialog.set" => {
            let (Some(id), Some(field)) = (u("dialog"), s("field")) else { return err("need `dialog` and `field`") };
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            match app.ui.dialog_mut(id) {
                Some(d) => {
                    d.fields.insert(field.to_string(), value);
                    ok(Value::Null)
                }
                None => err(format!("no dialog {id}")),
            }
        }
        "ui.dialog.confirm" | "ui.dialog.apply" => match u("dialog") {
            Some(id) => {
                let command = app.ui.dialogs.iter().find(|dialog| dialog.id == id).and_then(|dialog| {
                    if req.method == "ui.dialog.apply" {
                        dialog.fields.get("values").map(|values| ("prefs.set".to_string(), json!({"path": "", "value": values})))
                    } else {
                        dialog.fields.get("__command").and_then(Value::as_str).map(|command| (command.to_string(), Value::Object(dialog.fields.clone())))
                    }
                });
                if let Some((command, params)) = command
                    && let Some(authorize) = app.services.automation_command.as_ref()
                    && let Err(error) = authorize(&command, &params)
                {
                    return err(error);
                }
                let apply = req.method == "ui.dialog.apply";
                run_waiting(app, wait, |app| if apply { crate::prefs_ui::apply(app, id) } else { crate::dialogs::confirm(app, id) })
            }
            None => err("missing `dialog`"),
        },
        "ui.dialog.cancel" => match u("dialog").and_then(|id| app.ui.close_dialog(id)) {
            Some(_) => ok(Value::Null),
            None => err("no such dialog"),
        },
        "ui.window.open" => {
            if let Some(d) = u("document")
                && !app.session.set_active(d as usize)
            {
                return err(format!("no document {d}"));
            }
            wrap(crate::menus::invoke(app, ctx, "window.newWindowForDocument", json!({})))
        }
        "ui.window.close" => {
            let Some(id) = u("window") else { return err("missing `window`") };
            let before = app.ui.windows.len();
            app.ui.windows.retain(|w| w.id != id);
            if app.ui.windows.len() < before { ok(Value::Null) } else { err(format!("no window {id}")) }
        }
        "ui.pointer" => {
            let Some(events) = p.get("events").and_then(Value::as_array) else { return err("missing `events`") };
            // Modifier flags may be top-level or grouped under "modifiers".
            let m = p.get("modifiers").unwrap_or(p);
            let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
            let mods = egui::Modifiers {
                shift: flag("shift"),
                alt: flag("alt"),
                command: flag("command"),
                mac_cmd: cfg!(target_os = "macos") && flag("command"),
                ctrl: flag("ctrl"),
            };
            if let Some(t) = s("tool").and_then(Tool::from_name) {
                app.ui.tool = t;
            }
            // Space held: the Crop tool moves the frame being drawn, a marquee, lasso or shape
            // being drawn moves instead of growing (hold_keys.rs).
            let space = flag("space");
            crate::crop_ui::set_space(app, space);
            for e in events {
                let x = e.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                let y = e.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                let pr = e.get("pressure").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                let ev = match e.get("kind").and_then(Value::as_str).unwrap_or("move") {
                    "down" => ToolEvent::Down { x, y, pressure: pr },
                    "up" => ToolEvent::Up { x, y },
                    _ => ToolEvent::Move { x, y, pressure: pr },
                };
                // With the Color Picker on top the image is its eyedropper, as for the mouse.
                if crate::color_picker_ui::top(app).is_some() {
                    if !matches!(ev, ToolEvent::Up { .. }) {
                        crate::color_picker_ui::sample_at(app, x, y);
                    }
                    continue;
                }
                // Curves' modal eyedroppers use document coordinates here, matching the native
                // canvas path after its ViewXform conversion. Sample on press only.
                if !matches!(s("button"), Some("secondary" | "right")) && crate::adjust_dialog::picker_armed(app) {
                    if matches!(ev, ToolEvent::Down { .. }) {
                        crate::adjust_dialog::sample_at(app, x, y);
                    }
                    continue;
                }
                // The dialog owns all pointer events, including Up: no underlying brush stroke
                // or context menu may leak through while inspecting a Color Range preview.
                if app.ui.dialogs.last().is_some_and(|d| crate::color_range_ui::owns(&d.fields)) {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    let up = matches!(ev, ToolEvent::Up { .. });
                    let held = app.ui.dialogs.last().and_then(|d| d.fields.get("__pointerDown")).and_then(Value::as_bool).unwrap_or(false);
                    if let Some(d) = app.ui.dialogs.last_mut() {
                        d.fields.insert("__pointerDown".into(), json!(!up && (down || held)));
                    }
                    if !up && (down || held) && !space && !matches!(s("button"), Some("secondary" | "right" | "middle")) {
                        crate::color_range_ui::pick_top(app, [x, y], mods);
                    }
                    continue;
                }
                if matches!(s("button"), Some("secondary" | "right")) {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    // Right-click with the Move tool, or ⌘/Ctrl+right-click: list the layers there.
                    if crate::layer_pick_ui::is_gesture(app.ui.tool, mods) {
                        if down {
                            app.ui.canvas_tool_menu = None;
                            crate::layer_pick_ui::open(app, screen_point(app, x, y), x, y);
                        }
                        continue;
                    }
                    if crate::canvas_tool_menu::applies(app.ui.tool) {
                        if down {
                            crate::canvas_tool_menu::open(app, app.ui.tool, screen_point(app, x, y));
                        }
                        continue;
                    }
                    if !crate::paint_mouse::pointer_secondary(app, down, mods, screen_point(app, x, y)) {
                        continue;
                    }
                }
                // A simulated pen: tilt/rotation reach the stroke like a real stylus's (see `stylus`).
                let tilt = |k: &str| e.get(k).and_then(Value::as_f64).map(|v| v as f32);
                let pen = (tilt("tiltX"), tilt("tiltY"), tilt("rotation"));
                if pen != (None, None, None) {
                    let (tilt_x, tilt_y, rotation) = (pen.0.unwrap_or(0.0), pen.1.unwrap_or(0.0), pen.2.unwrap_or(0.0));
                    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: pr, tilt_x, tilt_y, rotation, eraser: false }));
                }
                if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
                    d.reposition = space;
                }
                tool_event(app, ev, mods);
            }
            app.stylus.feed.set(None);
            ok(json!({"status": app.ui.status}))
        }
        "ui.click" | "ui.move" => {
            // Screen coordinates in points (as reported by ui.inspect window size).
            let x = p.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let y = p.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let pos = egui::pos2(x, y);
            let button = match s("button").unwrap_or("left") {
                "right" | "secondary" => egui::PointerButton::Secondary,
                "middle" => egui::PointerButton::Middle,
                _ => egui::PointerButton::Primary,
            };
            let clicks = if req.method == "ui.click" { u("count").unwrap_or(1) } else { 0 };
            if clicks > MAX_CLICKS {
                return err(format!("`count` must be at most {MAX_CLICKS} (got {clicks})"));
            }
            app.synthetic.push(egui::Event::PointerMoved(pos));
            for _ in 0..clicks {
                app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() });
                app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: Default::default() });
            }
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.key" => {
            let Some(name) = s("key") else { return err("missing `key`") };
            let Some(key) = egui::Key::from_name(name) else { return err(format!("unknown key `{name}`")) };
            // Modifier flags may be top-level or grouped under "modifiers".
            let m = p.get("modifiers").unwrap_or(p);
            let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
            let modifiers = egui::Modifiers {
                command: flag("command"),
                mac_cmd: cfg!(target_os = "macos") && flag("command"),
                shift: flag("shift"),
                alt: flag("alt"),
                ctrl: flag("ctrl"),
            };
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.type" => {
            let Some(text) = s("text") else { return err("missing `text`") };
            app.synthetic.push(egui::Event::Text(text.to_string()));
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.resize" => {
            let (w, h) = (p.get("width").and_then(Value::as_f64).unwrap_or(1280.0), p.get("height").and_then(Value::as_f64).unwrap_or(800.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32)));
            ok(Value::Null)
        }
        "ui.gpu.simulateLoss" => {
            let error = p.get("error").and_then(Value::as_bool).unwrap_or(false);
            let fault = if error {
                photocraft_gpu::Fault::Error("simulated error (ui.gpu.simulateLoss)".into())
            } else {
                photocraft_gpu::Fault::Lost("simulated (ui.gpu.simulateLoss)".into())
            };
            let active = match app.gpu_health() {
                Some(h) => {
                    h.mark(fault);
                    true
                }
                None => false,
            };
            app.check_gpu(ctx);
            ctx.request_repaint();
            ok(json!({"wasActive": active, "gpuInfo": app.perf.gpu_info}))
        }
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.screenshot" => {
            // Occluded windows don't render on macOS, so the screenshot would never arrive: raise first.
            if p.get("focus").and_then(Value::as_bool).unwrap_or(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            // The capture itself is issued by `PhotocraftApp::issue_screenshots` once open/close
            // animations (modals, popups) have settled.
            let token = app.ui.alloc_id();
            ctx.request_repaint();
            Outcome::Screenshot { token, path: s("path").map(str::to_string) }
        }
        "app.open" => match s("path") {
            Some(path) => {
                let opened = match app.services.automation_read.as_mut() {
                    Some(read) => read(path),
                    None => Err("automation read authority is not configured".into()),
                };
                wrap(opened.and_then(|(name, bytes)| {
                    let name = app.open_name(&name);
                    let warnings = app.open_automation_bytes(&name, &bytes)?;
                    // Brushes/gradients go to the preset libraries: no document, no Open Recent entry.
                    if !crate::preset_files_ui::is_preset_file(&name) {
                        app.opened_from(path);
                    }
                    Ok(json!({"path": path, "name": name, "warnings": warnings}))
                }))
            }
            None => err("missing `path`"),
        },
        "app.save" => wrap(app.save_automation(s("path").map(str::to_string)).map(|(p, w)| json!({"path": p, "warnings": w}))),
        "app.quit" => {
            app.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Value::Null)
        }
        other => err(format!("unknown method `{other}`")),
    }
}

/// Snapshot of everything on screen, addressable by id.
pub fn inspect(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let screen = ctx.content_rect();
    let mut dialogs: Vec<Value> =
        app.ui.dialogs.iter().map(|d| json!({"id": d.id, "kind": d.kind, "title": crate::dialogs::title(d), "fields": d.fields})).collect();
    if let Some((kind, fields)) = &app.ui.shell.dialog {
        dialogs.push(json!({"id": SHELL_DIALOG, "kind": kind, "title": crate::workspace_ui::title(kind), "fields": fields}));
    }
    json!({
        "window": {"width": screen.width(), "height": screen.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "tool": app.ui.tool,
        "toolOptions": app.ui.tool_options,
        "magnetic": app.ui.magnetic,
        "textEdit": app.ui.text_edit,
        "typeTransform": app.ui.type_transform,
        "layerMenu": app.ui.layer_menu,
        "brushPicker": app.ui.brush_picker.map(|pos| json!({
            "pos": pos,
            "list": app.ui.brush_picker_list,
        })),
        "canvasToolMenu": app.ui.canvas_tool_menu.as_ref().map(|menu| {
            json!({
                "pos": menu.pos,
                "tool": menu.tool,
                "entries": crate::canvas_tool_menu::rows(menu).iter().map(|row| match row {
                    Some((label, id)) => json!({"label": label, "id": id, "enabled": crate::canvas_tool_menu::entry_enabled(app, menu, id)}),
                    None => json!({"separator": true}),
                }).collect::<Vec<_>>()
            })
        }),
        "panels": app.ui.panels,
        "view": app.ui.view,
        "views": app.ui.views,
        "dialogs": dialogs,
        "windows": app.ui.windows,
        "theme": app.ui.theme,
        "status": app.ui.status,
        "statusError": app.ui.status_error,
        "notices": app.ui.notices,
        "gpuFallbackNotice": app.ui.gpu_fallback_notice,
        "frame": app.frame,
        "session": photocraft_engine::inspect::session(&app.session),
        "document": app.session.active().map(photocraft_engine::inspect::document),
        "perf": {"fps": app.fps, "timings": app.perf},
        "brush": {"size": app.session.tools.brush.size, "hardness": app.session.tools.brush.hardness, "opacity": app.session.tools.brush.opacity},
        "distort": app.distort.describe(),
        "jobs": crate::jobs_ui::inspect(app),
        "cameraRaw": app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)),
    })
}

pub fn save_screenshot(app: &mut PhotocraftApp, image: &egui::ColorImage, path: Option<&str>) -> Value {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let png = match app.services.encode_png.as_ref() {
        Some(enc) => enc(w as u32, h as u32, &rgba),
        None => Err("no PNG encoder configured".into()),
    };
    let Some(path) = path else {
        return match png {
            Ok(bytes) => json!({
                "ok": true,
                "result": {
                    "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "mime": "image/png",
                    "width": w,
                    "height": h,
                }
            }),
            Err(error) => json!({"ok": false, "error": error}),
        };
    };
    let result = png.and_then(|bytes| match app.services.automation_write.as_mut() {
        Some(write) => write(path, &bytes),
        None => Err("automation write authority is not configured".into()),
    });
    match result {
        Ok(()) => json!({"ok": true, "result": {"path": path, "width": w, "height": h}}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, ctx, &req) {
            Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    #[test]
    fn shell_dialogs_are_listed_and_driven_through_ui_dialog() {
        // #1004: Window › Workspace and View dialogs live in `ui.shell.dialog`, and ui.inspect and
        // ui.dialog.* only knew `ui.dialogs`.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let shell = |app: &mut PhotocraftApp| {
            let v = call(app, &ctx, "ui.inspect", json!({}));
            v["result"]["dialogs"].as_array().unwrap().iter().find(|d| d["id"] == "shell").cloned()
        };
        // Listed, then cancelled.
        crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({})).unwrap();
        let d = shell(&mut app).expect("the open shell dialog is listed");
        assert_eq!((d["kind"].as_str(), d["title"].as_str()), (Some("newWorkspace"), Some("New Workspace")));
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "shell"}))["ok"], true);
        assert!(app.ui.shell.dialog.is_none() && shell(&mut app).is_none());
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "shell"}))["ok"], false, "nothing left to cancel");
        // A field set over control reaches OK.
        crate::menus::invoke(&mut app, &ctx, "view.show.showExtrasOptions", json!({})).unwrap();
        assert!(app.ui.view.show.notes);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": "shell", "field": "notes", "value": false}))["ok"], true);
        assert_eq!(shell(&mut app).unwrap()["fields"]["notes"], false);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "shell"}))["ok"], true);
        assert!(!app.ui.view.show.notes && app.ui.shell.dialog.is_none());
        // An OK that fails keeps the dialog open and says why.
        crate::menus::invoke(&mut app, &ctx, "view.show.showExtrasOptions", json!({})).unwrap();
        call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": "shell", "field": "bogus", "value": true}));
        let r = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "shell"}));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap_or("").contains("bogus"), "{r}");
        assert!(app.ui.shell.dialog.is_some());
        // Apply is Preferences-only; numeric ids still address ordinary dialogs only.
        assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": "shell"}))["ok"], false);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": 0}))["ok"], false);
        assert!(app.ui.shell.dialog.is_some());
    }

    #[test]
    fn opening_new_document_through_control_matches_clipboard_image_size() {
        // #2034: every way of opening New Document uses the clipboard-aware initial fields.
        let clipboard = std::sync::Arc::new(std::sync::Mutex::new(Some((100, 200, vec![255; 100 * 200 * 4]))));
        let image = clipboard.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services { clipboard_get_image: Some(Box::new(move || image.lock().ok()?.clone())), ..Default::default() },
        );
        let ctx = egui::Context::default();
        let opened = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"newDocument"}));
        assert_eq!(opened["ok"], true);
        let id = opened["result"]["dialog"].as_u64().expect("dialog id is returned");
        let fields = &app.ui.dialog_mut(id).expect("dialog was opened").fields;
        assert_eq!((fields["width"].as_u64(), fields["height"].as_u64()), (Some(100), Some(200)));
        assert_eq!(fields["__preset"], "Clipboard");
    }

    #[test]
    fn expired_queued_edit_does_not_run_and_a_live_retry_runs_once() {
        use std::time::{Duration, Instant};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let before = app.session.active().unwrap();
        let layers = before.doc.layers.len();
        let steps = before.history.past_len();
        let (tx, rx) = std::sync::mpsc::channel();
        app = app.with_control(rx);
        let (mut expired, expired_reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        expired.deadline = Some(Instant::now() - Duration::from_secs(1));
        tx.send(expired).unwrap();
        let (mut retry, retry_reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        retry.deadline = Some(Instant::now() + Duration::from_secs(60));
        tx.send(retry).unwrap();
        app.drain_control(&egui::Context::default());
        assert_eq!(expired_reply.try_recv().unwrap(), json!({"ok": false, "error": "timeout"}));
        assert_eq!(retry_reply.try_recv().unwrap()["ok"], true);
        let after = app.session.active().unwrap();
        assert_eq!(after.doc.layers.len(), layers + 1, "only the live retry adds a layer");
        assert_eq!(after.history.past_len(), steps + 1, "the expired edit adds no undo step");
    }

    #[test]
    fn deadline_free_requests_run_even_after_the_receiver_is_dropped() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let layers = app.session.active().unwrap().doc.layers.len();
        let (tx, rx) = std::sync::mpsc::channel();
        app = app.with_control(rx);
        let (req, reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        assert!(req.deadline.is_none());
        drop(reply);
        tx.send(req).unwrap();
        app.drain_control(&egui::Context::default());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    }

    #[test]
    fn right_pointer_opens_agent_visible_selection_menu() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "tool": "RectMarquee", "button": "secondary",
                "events": [{"kind": "down", "x": 4, "y": 4}, {"kind": "up", "x": 4, "y": 4}]
            }),
        );
        assert_eq!(result.get("ok"), Some(&Value::Bool(true)));
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
        assert!(entries.iter().any(|entry| entry.get("id") == Some(&json!("select.inverse")) && entry.get("enabled") == Some(&json!(true))));
        assert!(app.session.active().unwrap().doc.selection.is_some(), "right-click must not edit selection");
    }

    #[test]
    fn agent_can_inspect_and_choose_pen_make_selection_from_full_context_menu() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[2,2],[20,2],[20,20]]}]}})).unwrap();
        let expected_pos = screen_point(&app, 8.0, 8.0);
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "tool": "Pen", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]
            }),
        );
        assert_eq!(result["ok"], true);
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(app.ui.canvas_tool_menu.as_ref().unwrap().pos, expected_pos);
        let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
        assert_eq!(entries.iter().filter(|row| row.get("separator") == Some(&json!(true))).count(), 9);
        assert!(entries.iter().any(|row| row.get("id") == Some(&json!("path.toSelection")) && row.get("enabled") == Some(&json!(true))));
        assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"file.new"}))["ok"], false);
        let chosen = call(&mut app, &ctx, "ui.context.choose", json!({"id":"path.toSelection"}));
        assert_eq!(chosen["ok"], true);
        let dialog = chosen["result"]["dialog"].as_u64().unwrap();
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
        assert!(app.session.active().unwrap().doc.selection.is_some());
    }

    #[test]
    fn ui_inspect_reports_the_view_preferences() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let before = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(before.pointer("/result/view/show/selection_edges"), Some(&json!(true)));
        assert_eq!(before.pointer("/result/view/screen_mode"), Some(&json!("standard")));
        assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "view.show.selectionEdges"}))["ok"], true);
        let after = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(after.pointer("/result/view/show/selection_edges"), Some(&json!(false)));
        assert_eq!(after["result"]["view"], json!(app.ui.view));
    }

    #[test]
    fn engine_execute_runs_with_defaults_but_menu_invoke_opens_the_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let rev = app.session.active().unwrap().revision;
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "filter.blur.gaussianBlur"}));
        assert!(!r.to_string().contains("\"dialog\""), "engine.execute opened a dialog: {r}");
        assert!(app.session.active().unwrap().revision > rev, "engine.execute didn't run the filter");
        let rev = app.session.active().unwrap().revision;
        let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"}));
        assert!(r.to_string().contains("dialog"), "ui.menu.invoke should open the dialog: {r}");
        assert_eq!(app.session.active().unwrap().revision, rev, "opening a dialog must not edit the document");
    }

    #[test]
    fn menu_invoke_cannot_edit_document_behind_an_open_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();

        let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageSize"}));
        assert_eq!(opened["ok"], true, "{opened}");
        let dialog = opened["result"]["dialog"].as_u64().unwrap();
        let revision = app.session.active().unwrap().revision;
        let before = call(&mut app, &ctx, "ui.inspect", json!({}));
        let fields = before["result"]["dialogs"][0]["fields"].clone();

        for params in [json!({"id": "image.imageRotation.90cw"}), json!({"id": "image.imageRotation.90cw", "params": {}})] {
            let blocked = call(&mut app, &ctx, "ui.menu.invoke", params);
            assert_eq!(blocked["ok"], false, "{blocked}");
            assert_eq!(app.session.active().unwrap().revision, revision);
            let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
            assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
            assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
        }

        // View navigation stays permitted by the same policy as native menus and shortcuts.
        assert!(crate::menus::modal_allows(&app, "view.zoomIn"));
        assert!(!crate::menus::modal_allows(&app, "image.imageRotation.90cw"));

        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
        let rotated = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageRotation.90cw"}));
        assert_eq!(rotated["ok"], true, "{rotated}");
        assert!(app.session.active().unwrap().revision > revision);
    }

    #[test]
    fn ui_set_rejects_unknown_fields_before_changing_anything() {
        use crate::theme::ThemeKind;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.set_theme(&ctx, ThemeKind::StudioLight);
        // `dark` was advertised to MCP clients but never read; a typo looked like success too.
        for params in [json!({"dark": true}), json!({"thme": "classic"}), json!({"tool": "move", "dark": true})] {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
            assert!(r["error"].as_str().unwrap().contains("unknown field"), "{r}");
        }
        assert_eq!(app.ui.theme, ThemeKind::StudioLight);
        assert_eq!(app.ui.tool, Tool::Brush, "a rejected call applies none of its fields");
        // Every field the method reads gets past the check (a bad value is its own error).
        for field in UI_SET_FIELDS {
            let r = call(&mut app, &ctx, "ui.set", json!({ field: null }));
            assert!(!r.to_string().contains("unknown field"), "{field}: {r}");
        }
        assert_eq!(call(&mut app, &ctx, "ui.set", Value::Null)["ok"], true);
    }

    #[test]
    fn ui_set_gradient_blend_mode_validates_and_updates_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let good = call(&mut app, &ctx, "ui.set", json!({"tool": "gradient", "gradientBlendMode": "Difference", "gradientClassic": true}));
        assert_eq!(good["ok"], true, "{good}");
        assert_eq!(app.ui.tool_options.gradient_blend_mode, photocraft_color::BlendMode::Difference);
        assert!(app.ui.tool_options.gradient_classic);
        let bad = call(&mut app, &ctx, "ui.set", json!({"gradientBlendMode": "nonsense", "gradientClassic": false}));
        assert_eq!(bad["ok"], false, "{bad}");
        assert!(app.ui.tool_options.gradient_classic, "invalid mode must not change options");
    }

    #[test]
    fn ui_set_drives_the_eyedropper_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let good = call(
            &mut app,
            &ctx,
            "ui.set",
            json!({"tool": "eyedropper", "eyedropperSampleSize": 11, "eyedropperSample": "currentAndBelow", "eyedropperRing": false}),
        );
        assert_eq!(good["ok"], true, "{good}");
        let o = &app.ui.tool_options;
        assert_eq!((o.eyedropper_size, o.eyedropper_sample.as_str(), o.eyedropper_ring), (11, "currentAndBelow", false));
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"eyedropperSampleSize": "point"}))["ok"], true);
        assert_eq!(app.ui.tool_options.eyedropper_size, 1);
        for bad in [
            json!({"eyedropperSampleSize": 4}),
            json!({"eyedropperSampleSize": "big"}),
            json!({"eyedropperSample": "below"}),
            json!({"eyedropperSample": 2}),
            json!({"eyedropperRing": "yes"}),
        ] {
            let r = call(&mut app, &ctx, "ui.set", bad.clone());
            assert_eq!(r["ok"], false, "{bad}: {r}");
        }
        // A rejected call applies none of its fields.
        let r = call(&mut app, &ctx, "ui.set", json!({"eyedropperSampleSize": 101, "eyedropperSample": "nope"}));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(app.ui.tool_options.eyedropper_size, 1);
    }

    /// #1919: the Crop tool's overlay menu over the control channel, reported by `ui.inspect`.
    #[test]
    fn ui_set_drives_the_crop_overlay_options() {
        use crate::crop_overlay::{CropOverlay, OverlayShow};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let good =
            call(&mut app, &ctx, "ui.set", json!({"tool": "crop", "cropOverlay": "goldenSpiral", "cropOverlayShow": "always", "cropOverlayOrientation": 2}));
        assert_eq!(good["ok"], true, "{good}");
        let o = &app.ui.tool_options;
        assert_eq!((o.crop_overlay, o.crop_overlay_show, o.crop_overlay_orientation), (CropOverlay::GoldenSpiral, OverlayShow::Always, 2));
        let seen = call(&mut app, &ctx, "ui.inspect", json!({}));
        let t = &seen["result"]["toolOptions"];
        assert_eq!(
            (t["crop_overlay"].as_str(), t["crop_overlay_show"].as_str(), t["crop_overlay_orientation"].as_u64()),
            (Some("goldenSpiral"), Some("always"), Some(2))
        );
        for bad in [
            json!({"cropOverlay": "spiral"}),
            json!({"cropOverlay": 1}),
            json!({"cropOverlayShow": "sometimes"}),
            json!({"cropOverlayOrientation": 4}),
            json!({"cropOverlayOrientation": -1}),
            json!({"cropOverlayOrientation": "1"}),
            json!({"cropOverlay": "grid", "cropOverlayShow": "nope"}),
        ] {
            let r = call(&mut app, &ctx, "ui.set", bad.clone());
            assert_eq!(r["ok"], false, "{bad}: {r}");
        }
        assert_eq!(app.ui.tool_options.crop_overlay, CropOverlay::GoldenSpiral, "a rejected call applies none of its fields");
    }

    /// #1919: the Crop tool's gear menu (Show Cropped Area, crop shield) over the control channel.
    #[test]
    fn ui_set_drives_the_crop_shield_options() {
        use crate::crop_shield::{CropShield, ShieldColor};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let good = call(&mut app, &ctx, "ui.set", json!({"cropShield": {"color": "custom", "custom_color": [255, 0, 0], "opacity": 40}}));
        assert_eq!(good["ok"], true, "{good}");
        let want = CropShield { color: ShieldColor::Custom, custom_color: [255, 0, 0], opacity: 40.0, ..Default::default() };
        assert_eq!(app.ui.tool_options.crop_shield, want);
        // A patch: the other fields stay.
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropShield": {"show_cropped_area": false}}))["ok"], true);
        assert_eq!(app.ui.tool_options.crop_shield, CropShield { show_cropped_area: false, ..want.clone() });
        let t = &call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["toolOptions"]["crop_shield"];
        assert_eq!((t["color"].as_str(), t["opacity"].as_f64(), t["show_cropped_area"].as_bool()), (Some("custom"), Some(40.0), Some(false)));
        for bad in [
            json!({"cropShield": true}),
            json!({"cropShield": {"opacity": 101}}),
            json!({"cropShield": {"opacity": -1}}),
            json!({"cropShield": {"opacity": "50"}}),
            json!({"cropShield": {"color": "red"}}),
            json!({"cropShield": {"custom_color": [256, 0, 0]}}),
            json!({"cropShield": {"shield": true}}),
            json!({"cropShield": {"enabled": false}, "cropOverlay": "nope"}),
        ] {
            let r = call(&mut app, &ctx, "ui.set", bad.clone());
            assert_eq!(r["ok"], false, "{bad}: {r}");
        }
        assert_eq!(app.ui.tool_options.crop_shield, CropShield { show_cropped_area: false, ..want }, "a rejected call applies none of its fields");
    }

    #[test]
    fn ui_set_opens_the_brush_preset_picker_and_sets_its_view() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(app.ui.brush_picker_list.view, crate::brush_panel::BrushesView::Grid, "tip thumbnails by default");
        let r = call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushPicker": [120, 80], "brushPickerView": "list"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.ui.brush_picker, Some([120.0, 80.0]));
        assert_eq!(app.ui.brush_picker_list.view, crate::brush_panel::BrushesView::List);
        for bad in [json!({"brushPicker": [1]}), json!({"brushPicker": "here"}), json!({"brushPickerView": "tiles"})] {
            assert_eq!(call(&mut app, &ctx, "ui.set", bad.clone())["ok"], false, "{bad}");
        }
        assert_eq!(app.ui.brush_picker, Some([120.0, 80.0]), "a bad value leaves the picker alone");
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
        assert_eq!(app.ui.brush_picker, None);
    }

    #[test]
    fn ui_inspect_tracks_brush_picker_open_and_closed() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 128, "height": 128})).unwrap();

        let closed = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(closed["result"]["brushPicker"].is_null());

        // A right-click on the canvas with Brush opens the same picker as the options bar.
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}))["ok"], true);
        let click = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "button": "right",
                "events": [{"kind": "down", "x": 64, "y": 64}, {"kind": "up", "x": 64, "y": 64}]
            }),
        );
        assert_eq!(click["ok"], true, "{click}");
        let open = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(open["result"]["brushPicker"]["pos"].is_array(), "{open}");
        assert_eq!(open["result"]["brushPicker"]["list"]["view"], "grid");

        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPickerView": "list"}))["ok"], true);
        let changed = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(changed["result"]["brushPicker"]["list"]["view"], "list");

        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
        let closed_again = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(closed_again["result"]["brushPicker"].is_null());
    }

    #[test]
    fn ui_set_applies_all_fields_or_none() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let before = app.ui.tool;
        let panels_before = serde_json::to_value(&app.ui.panels).unwrap_or_default();
        // A late invalid field must not leave the earlier valid one applied - including the
        // journaled brush command, which is validated up front for exactly this reason.
        for params in
            [json!({"tool": "move", "theme": "nope"}), json!({"tool": "move", "brushSize": 1e300}), json!({"tool": "move", "colorPanel": {"nope": true}})]
        {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
            assert_eq!(app.ui.tool, before, "{params}: a rejected call applies none of its fields");
        }
        // Wrong types and unknown nested keys are errors, not silent no-ops with ok:true.
        for params in [
            json!({"panels": "x"}),
            json!({"panels": {"nope": true}}),
            json!({"dockTabs": {"nope": 1}}),
            json!({"dock": {"nope": 1}}),
            json!({"center": [1]}),
            json!({"center": [1, 2, 3]}),
            json!({"center": "here"}),
            json!({"colorPanel": "x"}),
            json!({"selectionMode": "add"}),
        ] {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
        }
        assert_eq!(serde_json::to_value(&app.ui.panels).unwrap_or_default(), panels_before, "nothing was applied");
        // Non-numeric brushSize keeps the API's historical silent no-op (see the dispatch test).
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushSize": "large"}))["ok"], true);
    }

    #[test]
    fn ui_set_color_panel_picks_the_edited_chip() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": true}}))["ok"], true);
        assert!(app.ui.color_panel.background);
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": "yes"}}))["ok"], false);
        assert!(app.ui.color_panel.background, "a bad value changes nothing");
    }

    #[test]
    fn ui_set_unknown_theme_error_names_every_theme_and_each_name_works() {
        use crate::theme::ThemeKind;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.set", json!({"theme": "nope"}));
        assert_eq!(r["ok"], false);
        let error = r["error"].as_str().unwrap();
        for kind in ThemeKind::ALL {
            assert!(error.contains(kind.id()), "{error} lacks {}", kind.id());
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": kind.id()}))["ok"], true);
            assert_eq!(app.ui.theme, kind);
            assert_eq!(ThemeKind::from_name(kind.id()), Some(kind));
        }
    }

    #[test]
    fn ui_window_open_rejects_an_unknown_document_index() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        // An out-of-range index must error instead of silently switching the active document
        // and opening the window for whatever is active now.
        let before = app.session.active_index();
        let r = call(&mut app, &ctx, "ui.window.open", json!({"document": 99}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().unwrap().contains("no document 99"), "{r}");
        assert_eq!(app.session.active_index(), before, "the active document is untouched");
        // A valid index still opens the window.
        assert_eq!(call(&mut app, &ctx, "ui.window.open", json!({"document": 0}))["ok"], true);
    }

    #[test]
    fn ui_set_brush_size_dispatches_a_journaled_brush_command() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();

        let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": 42.5}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        let (id, params) = app.session.journal.last().cloned().expect("brush change is journaled");
        assert_eq!(id, "tools.setBrush");
        assert_eq!(params, json!({"brush": {"size": 42.5}}));

        // The control API has historically ignored non-numeric optional values.
        for params in [json!({}), json!({"brushSize": null}), json!({"brushSize": "large"}), json!({"brushSize": true})] {
            let journal_len = app.session.journal.len();
            let r = call(&mut app, &ctx, "ui.set", params);
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(app.session.tools.brush.size, 42.5);
            assert_eq!(app.session.journal.len(), journal_len);
        }

        // Each tool keeps its own brush (#218), so `tool` + `brushSize` in one call sets the new
        // tool's size rather than the one it was carrying.
        call(&mut app, &ctx, "ui.set", json!({"tool": "eraser", "brushSize": 12.0}));
        assert_eq!(app.session.tools.brush.size, 12.0, "the Eraser's own size");
        call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}));
        assert_eq!(app.session.tools.brush.size, 42.5, "the Brush gets its own back");
        call(&mut app, &ctx, "ui.set", json!({"tool": "eraser"}));
        assert_eq!(app.session.tools.brush.size, 12.0);
        call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushSize": 42.5}));

        // Values that cannot be represented by BrushSettings must report the command error and
        // leave both the brush and journal unchanged instead of mutating tool state directly.
        let journal_len = app.session.journal.len();
        let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": f64::MAX}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().is_some(), "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        assert_eq!(app.session.journal.len(), journal_len);
    }

    #[test]
    fn preferences_apply_is_available_over_control_without_closing() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "edit.preferences.interface"}));
        let id = r["result"]["dialog"].as_u64().unwrap();
        let mut values = app.ui.dialog_mut(id).unwrap().fields["values"].clone();
        values["interface"]["uiScale"] = json!("200");
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "values", "value": values}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], true);
        assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
        assert!(app.ui.dialog_mut(id).is_some());
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
        assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
        for params in [json!({}), json!({"dialog": "invalid"}), json!({"dialog": u64::MAX})] {
            assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", params)["ok"], false);
        }
    }

    #[test]
    fn preferences_apply_respects_the_automation_command_policy() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id == "prefs.set" { Err("preference changes denied".into()) } else { Ok(()) })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let before = app.session.prefs().to_json();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["uiScale"] = json!("200");
        let r = call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"], "preference changes denied");
        assert_eq!(app.session.prefs().to_json(), before);
        assert!(app.ui.dialog_mut(id).is_some());
    }

    #[test]
    fn synthetic_shortcuts_use_the_automation_command_policy() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id.starts_with("file.") { Err("filesystem command denied".into()) } else { Ok(()) })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        app.automation_input = true;
        let error = crate::menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err();
        assert_eq!(error, "filesystem command denied");
    }

    #[test]
    fn automation_open_and_save_use_the_roots_and_reply_with_warnings() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        use std::cell::RefCell;
        use std::rc::Rc;

        // Without granted roots both fail closed, even with an exporter and an ambient writer.
        let services = crate::Services {
            export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
            write: Some(Box::new(|_p: &str, _b: &[u8]| Err("ambient writer used".into()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
        assert!(r.to_string().contains("automation read authority is not configured"), "{r}");
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
        assert!(r.to_string().contains("automation write authority is not configured"), "{r}");

        let written: Rc<RefCell<Vec<String>>> = Rc::default();
        let w = written.clone();
        let services = crate::Services {
            import: Some(Box::new(|name: &str, _b: &[u8]| {
                Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), vec!["Adjustment layer flattened".to_string()]))
            })),
            export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), vec!["Layers were flattened".to_string()])))),
            automation_read: Some(Box::new(|path: &str| Ok((path.rsplit('/').next().unwrap_or(path).to_string(), b"x".to_vec())))),
            automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
                w.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
        assert_eq!(r["result"]["warnings"], json!(["Adjustment layer flattened"]), "{r}");
        assert_eq!(r["result"]["name"], "warn.psd");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("in/warn.psd"));
        assert_eq!(app.ui.recent_files, vec!["in/warn.psd".to_string()]);
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
        assert_eq!(r["result"], json!({"path": "out.png", "warnings": ["Layers were flattened"]}), "{r}");
        assert_eq!(*written.borrow(), vec!["out.png".to_string()]);
        // Without `path`, only a layered file is written back, like File › Save (#416).
        call(&mut app, &ctx, "app.open", json!({"path": "in/flat.jpg"}));
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
        assert_eq!(written.borrow().len(), 1, "nothing written over the JPEG");
        call(&mut app, &ctx, "app.open", json!({"path": "in/layered.psd"}));
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert_eq!(r["result"]["path"], "in/layered.psd", "{r}");
        assert_eq!(written.borrow().last().map(String::as_str), Some("in/layered.psd"));
        // A template opens untitled, so a save without `path` never writes over it.
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/card.psdt"}));
        assert_eq!(r["result"]["name"], "Untitled-1", "{r}");
        assert_eq!(app.session.active().unwrap().path, None);
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
    }

    fn deny_ambient_file(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
        if id.starts_with("file.") && id != "file.new" {
            Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
        } else {
            Ok(())
        }
    }

    fn deny_smart_object_paths(id: &str, params: &Value) -> photocraft_engine::Result<()> {
        // The production policy lives in the automation crate. This gate exercises the control
        // session's state-derived path check without adding that dependency to the UI crate.
        if matches!(id, "layer.smartObjects.editContents" | "layer.smartObjects.convertToLayers" | "layer.smartObjects.saveContents")
            && params.get("path").and_then(Value::as_str).is_some()
        {
            Err(photocraft_engine::EngineError::Other("ambient smart-object path denied".into()))
        } else {
            Ok(())
        }
    }

    fn smart_control_app() -> (PhotocraftApp, egui::Context, photocraft_doc::LayerId) {
        let services = crate::Services {
            automation_authorize: Some(deny_smart_object_paths),
            automation_command: Some(Box::new(|id, params| deny_smart_object_paths(id, params).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 8, "height": 6, "background": "transparent"})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let id = app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        (app, egui::Context::default(), photocraft_doc::LayerId(id))
    }

    #[test]
    fn control_edits_nested_embedded_smart_objects_and_converts_an_explicit_target() {
        use photocraft_doc::{LayerContent, SmartSource};
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::Arc;

        let (mut app, ctx, smart) = smart_control_app();
        // Embed this one-smart-layer document inside the outer object: two Edit Contents
        // requests must reach the raster layer, as in a nested product-photo template.
        let nested = photocraft_engine::smart_cmds::encode_source(&app.session.active().unwrap().doc).unwrap();
        app.session
            .edit("Nested fixture", |doc, _| {
                let LayerContent::Smart(sm) = &mut doc.layer_mut(smart).unwrap().content else { panic!("smart fixture") };
                sm.source = SmartSource::Embedded { file_name: "product.pcraft".into(), bytes: Arc::new(nested) };
                Ok(())
            })
            .unwrap();
        let other = app.run("layer.new.layer", json!({"name": "Unrelated"})).unwrap()["layer"].as_u64().unwrap();
        assert_ne!(app.session.active().unwrap().active_layer, Some(smart));
        for expected_document in [1, 2] {
            let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.editContents", "params": {"layer": smart.0}}));
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(r["result"]["document"], expected_document, "{r}");
            assert!(app.session.active().unwrap().path.is_none(), "contents open in memory");
            assert!(app.session.authorize.is_none(), "the request gate must not remain installed");
        }
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "edit.fill", "params": {"color": "#0000ff"}}));
        assert_eq!(r["ok"], true, "{r}");
        for expected_documents in [2, 1] {
            let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.saveContents"}));
            assert_eq!(r["ok"], true, "{r}");
            let r = call(&mut app, &ctx, "engine.execute", json!({"command": "file.close"}));
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(app.session.documents().len(), expected_documents);
        }
        assert_eq!(app.session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(other)));
        assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px[0], [0.0, 0.0, 1.0, 1.0]);
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.convertToLayers", "params": {"layer": smart.0}}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(other)).is_some(), "the unrelated active layer remains");
        assert!(app.session.authorize.is_none());

        let written = Rc::new(RefCell::new(Vec::new()));
        let output = written.clone();
        app.services.export =
            Some(Box::new(|doc, _, _| photocraft_engine::smart_cmds::encode_source(doc).map(|bytes| (bytes, Vec::new())).map_err(|e| e.to_string())));
        app.services.automation_write = Some(Box::new(move |path, bytes| {
            output.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        app.services.write = Some(Box::new(|_, _| Err("ambient writer used".into())));
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out/template.pcraft"}));
        assert_eq!(r["ok"], true, "{r}");
        let written = written.borrow();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].0, "out/template.pcraft");
        let saved = photocraft_engine::smart_cmds::decode_source(&written[0].0, &written[0].1).unwrap();
        assert_eq!(photocraft_compose::flatten(&saved).px[0], [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn control_refuses_linked_smart_objects_in_commands_and_actions_but_local_editing_still_works() {
        use photocraft_doc::{LayerContent, SmartSource};
        use std::sync::Arc;

        let (mut app, ctx, smart) = smart_control_app();
        let path = std::env::temp_dir().join(format!("pc-control-smart-linked-{}.pcraft", std::process::id()));
        let st = app.session.active().unwrap();
        let LayerContent::Smart(sm) = &st.doc.layer(smart).unwrap().content else { panic!("smart fixture") };
        let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("embedded fixture") };
        std::fs::write(&path, bytes.as_slice()).unwrap();
        app.session
            .edit("Linked fixture", |doc, _| {
                let LayerContent::Smart(sm) = &mut doc.layer_mut(smart).unwrap().content else { panic!("smart fixture") };
                sm.source = SmartSource::Linked { path: path.to_string_lossy().into_owned() };
                Ok(())
            })
            .unwrap();
        app.run("layer.new.layer", json!({"name": "Unrelated"})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let active = app.session.active().unwrap().active_layer;
        for command in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers"] {
            let r = call(&mut app, &ctx, "engine.execute", json!({"command": command, "params": {"layer": smart.0}}));
            assert_eq!(r["ok"], false, "{r}");
            assert!(r["error"].as_str().unwrap().contains("ambient smart-object path denied"), "{r}");
            app.session.actions.list =
                vec![photocraft_engine::actions_cmds::Action { name: "Linked contents".into(), steps: vec![(command.into(), json!({"layer": smart.0}))] }];
            let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Linked contents"}}));
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(r["result"]["ran"], 0, "{r}");
            assert_eq!(r["result"]["failed"]["id"], command, "{r}");
            assert_eq!(app.session.documents().len(), 1);
            assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before), "a refused operation must not edit the document");
            assert_eq!(app.session.active().unwrap().active_layer, active);
            assert!(app.session.authorize.is_none(), "a refused request must restore the local session");
        }
        // A synthetic thumbnail action uses the same temporary authorization as control calls.
        app.automation_input = true;
        let error = crate::menus::invoke(&mut app, &ctx, "layer.smartObjects.editContents", json!({"layer": smart.0})).unwrap_err();
        assert!(error.contains("ambient smart-object path denied"), "{error}");
        assert!(app.session.authorize.is_none());
        app.automation_input = false;
        app.run("layer.smartObjects.editContents", json!({"layer": smart.0})).unwrap();
        assert_eq!(app.session.documents().len(), 2, "a human can still open the existing linked file");
        assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px[0], [1.0, 0.0, 0.0, 1.0]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn actions_play_checks_each_step_over_control() {
        let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
            name: "Open".into(),
            steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        });
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Open"}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["ran"], 0, "{r}");
        assert_eq!(r["result"]["failed"]["id"], "file.open", "{r}");
        assert!(r["result"]["failed"]["error"].as_str().unwrap_or("").contains("disabled"), "{r}");
        assert!(app.session.documents().is_empty());
        assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the request");
    }

    #[test]
    fn actions_play_checks_each_step_for_synthetic_input() {
        let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
            name: "Open".into(),
            steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        });
        app.automation_input = true;
        let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 0, "{r}");
        assert_eq!(r["failed"]["id"], "file.open", "{r}");
        assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the command");

        // A local play (no automation input) still runs recorded steps.
        app.automation_input = false;
        app.session.actions.list[0].steps.remove(0);
        app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 1, "{r}");
    }

    #[test]
    fn huge_click_count_is_rejected() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        // #982: a huge count used to queue a press and a release per click before replying.
        for count in [json!(MAX_CLICKS + 1), json!(1_000_000_000u64), json!(u64::MAX)] {
            let r = call(&mut app, &ctx, "ui.click", json!({"x": 10, "y": 10, "count": count}));
            assert_eq!(r["ok"], false, "{r}");
            assert!(r["error"].as_str().unwrap().contains("`count` must be at most 256"), "{r}");
            assert!(app.synthetic.is_empty(), "a rejected click queues nothing");
        }
        // Single, double and the largest allowed clicks still queue a move plus a press and a release each.
        for (count, events) in [(None, 3), (Some(2), 5), (Some(MAX_CLICKS), 2 * MAX_CLICKS as usize + 1)] {
            let params = match count {
                Some(n) => json!({"x": 10, "y": 10, "count": n}),
                None => json!({"x": 10, "y": 10}),
            };
            let (req, _rx) = ControlRequest::new("ui.click", params);
            assert!(matches!(handle(&mut app, &ctx, &req), Outcome::AfterInput));
            assert_eq!(app.synthetic.len(), events);
            app.synthetic.clear();
        }
    }
}
