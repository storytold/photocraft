use super::{PhotocraftApp, Value, json};
use crate::canvas_tool_menu::{CanvasToolMenu, MenuKind};
use craft::ui::contract::Entry;

/// One row of a menu as `ui.inspect` lists it: a separator, a heading, or an item with its id.
fn entry_json(entry: &Entry<String>) -> Value {
    if entry.is_separator() {
        json!({"separator": true})
    } else if entry.is_heading() {
        json!({"heading": entry.label().as_str()})
    } else {
        json!({"label": entry.label().as_str(), "id": entry.value(), "enabled": entry.is_enabled(), "checked": entry.is_checked()})
    }
}

/// The agent link for `ui.inspect`: its status (`port`, `launchFlags`, `error`), the suite's
/// `AI-MCP` flag (`mcpFlag`) and the ArtCraft Launcher's switch (`launcher`).
fn agent_link_json(app: &PhotocraftApp) -> Value {
    let status = app.services.agent_link.as_ref().map(|link| link.status().clone()).unwrap_or_default();
    let switch = app.services.agent_switch.as_ref();
    let mut v = json!(status);
    if let Some(m) = v.as_object_mut() {
        m.insert("mcpFlag".into(), json!(switch.is_some_and(|s| s.mcp_flag())));
        m.insert("launcher".into(), json!(switch.is_some_and(|s| s.launcher())));
    }
    v
}

/// The open canvas context menu's rows for `ui.state`: the spelling suggestions first (type edit
/// menu over a misspelled word), then the menu's own rows, in the order the menu shows them.
fn context_entries(app: &PhotocraftApp, menu: &CanvasToolMenu) -> Vec<Value> {
    crate::canvas_tool_menu::entries(app, menu).iter().map(entry_json).collect()
}

/// The painting tools' Shift+right-click menu: the current mode, the labels `ui.context.choose` takes and the rows.
fn brush_mode_menu(app: &PhotocraftApp, menu: &CanvasToolMenu) -> Value {
    use crate::brush_mode_menu as m;
    let tool = app.ui.tool;
    let plain = matches!(menu.kind, MenuKind::BrushMode { plain: true });
    let current = m::checked(app, tool, plain);
    json!({
        "pos": menu.pos,
        "tool": tool,
        "mode": current.and_then(m::Item::label),
        "choices": m::labels(tool),
        "items": m::entries(app, tool, plain).iter().map(entry_json).collect::<Vec<_>>(),
    })
}

/// The last pass's rect of each canvas and hosted dialog, as `[x0, y0, x1, y1]` in screen points. An id
/// the framework reports no rect for (absent, behind the modal, no area) is left out.
fn rects(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    use craft::ui::runtime::access::bounds;
    let rect = |id: egui::Id| bounds(ctx, id).map(|r| json!([r.min.x, r.min.y, r.max.x, r.max.y]));
    let mut out = serde_json::Map::new();
    for idx in 0..app.session.documents().len() {
        if let Some(r) = rect(crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id()) {
            out.insert(format!("canvas.{idx}"), r);
        }
    }
    for dialog in app.framework.dialogs.open_dialogs() {
        if let Some(r) = rect(dialog.id) {
            out.insert(dialog_key(app, dialog.id), r);
        }
    }
    Value::Object(out)
}

/// The key `rects` stores hosted dialog `id` under: the id `dialogs` lists it by, so the unsaved-changes prompt is
/// [`crate::discard_ui::CONTROL_ID`] there too (#15544), any other dialog its host id.
pub(super) fn dialog_key(app: &PhotocraftApp, id: egui::Id) -> String {
    if crate::discard_ui::host_id(app) == Some(id) { crate::discard_ui::CONTROL_ID.to_string() } else { id.value().to_string() }
}

/// The notices posted since the last pass and the toast cards on screen in every window.
fn notices(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let mut all: Vec<Value> = app.ui.notices.iter().map(|notice| serde_json::to_value(notice).unwrap_or_default()).collect();
    all.extend(craft::ui::runtime::toast::snapshot(ctx).into_iter().map(|(window, card)| {
        json!({
            "window": window.as_str(),
            "id": card.id.value(),
            "title": card.toast.title.as_ref().map(|title| title.as_str()),
            "lines": card.toast.lines.iter().map(|line| line.as_str()).collect::<Vec<_>>(),
            "tone": card.toast.tone.name(),
        })
    }));
    Value::Array(all)
}

/// Snapshot of everything on screen, addressable by id.
pub fn inspect(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let screen = ctx.content_rect();
    // The framework host's dialogs in their one stack order (top last).
    let mut dialogs = craft::automation::dialogs::inspect(&app.framework.dialogs);
    crate::layer_style::describe_params(app, &mut dialogs);
    crate::discard_ui::describe(app, &mut dialogs);
    // Keyboard Shortcuts and Menus draws beneath the host's dialogs, outside the host (#15544).
    if let (Some(shortcuts), Some(list)) = (crate::prefs_ui::shortcuts::inspect(app), dialogs.as_array_mut()) {
        list.insert(0, shortcuts);
    }
    // Last frame's task bar rect, unless the bar can't show now (hidden by `ui.set` before the
    // next draw).
    let task_bar_rect = app.ui.task_bar_shown.filter(|_| crate::crop_task_bar::eligible(app)).or_else(|| crate::transform_task_bar::shown_rect(app, app.ctx()));
    // The one open canvas context menu, told apart by its kind.
    let menu = app.ui.canvas_tool_menu.as_ref();
    let kind = menu.map(|menu| &menu.kind);
    // The old `panels` and `dockTabs` words, derived from the workspace for readers that still use them.
    let legacy = crate::panel_docking::legacy_view(app);
    let picker = &app.ui.brush_picker_state;
    json!({
        "window": {"width": screen.width(), "height": screen.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "scratch": photocraft_raster::spill::stats(),
        "memory": photocraft_raster::memory::stats(),
        "tool": app.ui.tool,
        "toolOptions": app.ui.tool_options,
        "artboardTool": app.ui.artboard_tool,
        "perspectiveCrop": crate::perspective_crop_ui::quad(app),
        "strokeOptionsOpen": app.ui.shape_stroke_open,
        "magnetic": crate::magnetic_lasso_ui::inspect(app),
        "textEdit": app.ui.text_edit,
        // The paragraph box's overflow flag (set by the overlay each frame): agents
        // learn the overflow without a screen; never persisted (serde skip). Gated on the
        // session: the flag is a per-frame snapshot, and an `ui.inspect` running between the
        // frame that ended the edit and the one that resets the flag would otherwise report
        // overflow beside `"textEdit": null`.
        "typeBoxOverflow": app.ui.text_edit.is_some() && app.ui.type_box_overflow,
        "typeTransform": crate::type_transform::inspect(app),
        "camTransform": app.ui.cam_transform,
        "layerMenu": menu.zip(kind).and_then(|(menu, kind)| match kind {
            MenuKind::Layers { layers } => Some(json!({
                "pos": menu.pos,
                "layers": layers.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
            })),
            _ => None,
        }),
        "layerPicker": app.ui.layer_picker,
        "brushModeMenu": menu.filter(|menu| matches!(menu.kind, MenuKind::BrushMode { .. })).map(|menu| brush_mode_menu(app, menu)),
        "brushPicker": app.ui.brush_picker.map(|pos| json!({
            "pos": pos,
            "list": {
                "view": picker.browser.view,
                "filter": picker.browser.filter,
                // The card's parts.
                "showName": picker.parts.name,
                "showStroke": picker.parts.stroke,
                "showTip": picker.parts.tip,
                // The footer slider: the cards' width scale.
                "scale": picker.browser.zoom,
            },
        })),
        "canvasToolMenu": menu.filter(|menu| !matches!(menu.kind, MenuKind::Layers { .. } | MenuKind::BrushMode { .. })).map(|menu| {
            json!({
                "pos": menu.pos,
                "tool": menu.tool,
                "pasteboard": matches!(menu.kind, MenuKind::Pasteboard),
                "guide": matches!(menu.kind, MenuKind::Guide),
                "crop": matches!(menu.kind, MenuKind::Crop { .. }),
                "entries": context_entries(app, menu)
            })
        }),
        "panels": legacy.get("panels").cloned().unwrap_or_else(|| json!(app.ui.panels)),
        // The contextual task bar (#10890): Window › Contextual Task Bar, whether it shows now and
        // where (screen points), and its place (pinned, dragged-to corner).
        "taskBar": json!({
            "enabled": app.ui.panels.context_task_bar,
            "visible": task_bar_rect.is_some(),
            "rect": task_bar_rect,
            "pinned": app.ui.panels.task_bar.pinned,
            "at": app.ui.panels.task_bar.at,
            "offset": app.ui.panels.task_bar.offset,
        }),
        "dockTabs": legacy.get("dockTabs").cloned().unwrap_or(Value::Null),
        "docking": app.ui.docking,
        "statusInfo": app.ui.chrome.status_info,
        "view": app.ui.view,
        "views": app.ui.views,
        "dialogs": dialogs,
        "rects": rects(app, ctx),
        "windows": app.ui.windows,
        "theme": app.ui.theme.id().as_str(),
        "themeDark": app.ui.theme.is_dark(),
        "status": app.ui.status,
        "statusError": app.ui.status_error,
        "notices": notices(app, ctx),
        "updatesOpen": app.ui.updates_open,
        "updates": {
            "supported": cfg!(not(target_arch = "wasm32")),
            "currentVersion": photocraft_engine::build_info::VERSION,
            "status": crate::updates_ui::status_message(app),
        },
        "gpuFallbackNotice": app.ui.gpu_fallback_notice,
        "frame": app.frame,
        "session": photocraft_engine::inspect::session(&app.session),
        "document": app.session.active().map(photocraft_engine::inspect::document),
        "perf": {
            "fps": app.fps,
            "timings": app.perf,
            "strokeTrace": app.stroke_trace.last,
            "renderPath": crate::render_path::inspect(app),
            "slowFrames": app.slow_frames,
        },
        "brush": {"size": app.session.tools.brush.size, "hardness": app.session.tools.brush.hardness, "opacity": app.session.tools.brush.opacity},
        "distort": app.distort.describe(),
        "jobs": crate::jobs_ui::inspect(app),
        "agentLink": agent_link_json(app),
        "histogram": crate::histogram_panel::inspect(app),
        "cameraRaw": crate::camera_raw_ui::describe(app),
    })
}
