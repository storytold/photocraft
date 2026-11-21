use super::METHODS;
use super::inspect::inspect;
use super::params::{
    BUTTONS, POINTER_MODIFIERS, UI_SET_FIELDS, bool_field, err, f32_field, f32_range, input_choice, input_modifiers, input_number, input_object, merged_object,
    modifiers_from, num_field, ok, uint_field,
};
use super::{ControlRequest, PhotocraftApp, Reply, Tool, Value, json, run_waiting};
use craft::behavior::dock::Action;
use craft::behavior::geometry::CropGuide;
use craft::behavior::geometry::crop::normalized_angle;
use craft::behavior::kurbo::{Point, Size};
use craft::behavior::{CanvasView, PointerButton, PointerSample};
use craft::keymap::{CommandRef, Key, Mods};
use craft::ui::canvas::{ToolHost, ToolInput};
use craft::ui::runtime::windows::{WindowAction, WindowHost};
use photocraft_engine::jobs::{CancelReport, JobId};

/// A domain handler: `Some` when it owns the request's method, `None` to let the next one try.
type Handler = fn(&mut PhotocraftApp, &egui::Context, &ControlRequest) -> Option<Reply>;

/// A validated `ui.set` field, applied after every field has parsed.
type Write = Box<dyn FnOnce(&mut PhotocraftApp)>;

pub(super) fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Reply {
    const HANDLERS: [Handler; 5] = [engine_methods, shell_methods, dialog_methods, input_methods, app_methods];
    HANDLERS.iter().find_map(|handler| handler(app, ctx, req)).unwrap_or_else(|| err(photocraft_engine::command_hints::unknown_method(&req.method, METHODS)))
}

fn str_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

fn u64_param(p: &Value, key: &str) -> Option<u64> {
    p.get(key).and_then(Value::as_u64)
}

pub(super) fn wait_param(p: &Value) -> bool {
    p.get("wait").and_then(Value::as_bool).unwrap_or(true)
}

/// `v` read as the type `witness` has, or the error `error` says.
fn parse_like<T: serde::de::DeserializeOwned>(_witness: &T, v: &Value, error: impl FnOnce() -> String) -> std::result::Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|_| error())
}

// ---- engine commands, menus and jobs ----

fn engine_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    Some(match req.method.as_str() {
        "ui.context.choose" => context_choose(app, ctx, &req.params),
        "engine.execute" | "ui.menu.invoke" => execute_command(app, ctx, req),
        "engine.commands" => Reply::from_result(app.run("command.list", json!({}))),
        // Background jobs (#210): running ones with progress, then the last few that ended.
        "jobs.list" => Reply::from_result(app.session.execute("jobs.list", json!({})).map_err(|e| e.to_string())),
        "jobs.cancel" => jobs_cancel(app, &req.params),
        "ui.menu.list" => {
            // Report the same enablement the native menu shows, so a listed item is invokable.
            let rows = crate::menus::model(app, ctx).rows.clone();
            ok(menu_list(app, &rows))
        }
        "ui.inspect" => ok(inspect(app, ctx)),
        _ => return None,
    })
}

/// The menu rows as `ui.menu.list` reports them: one flat entry per row, its menu path in `path`.
// The reply is flat (no `children` key): `Row` carries `path`. `color` is the Edit › Menus colour name of a tinted row.
fn menu_list(app: &PhotocraftApp, rows: &[craft::menu::Row]) -> Value {
    let colors = &app.session.prefs().menus.colors;
    Value::Array(
        rows.iter()
            .map(|row| {
                let color = row.tint.and_then(|_| colors.get(&row.command.id));
                let mut item = json!({
                    "id": row.command.id,
                    "label": row.label,
                    "path": row.path,
                    "shortcut": row.shortcut.as_ref().map(|chord| chord.to_string()),
                    "enabled": row.enabled && crate::menus::allows(app, &row.command),
                    "checked": row.kind.checked(),
                });
                if let Some(color) = color {
                    item["color"] = json!(color);
                }
                item
            })
            .collect(),
    )
}

/// The id of the top hosted dialog as `ui.inspect` lists it (a decimal string).
fn top_dialog(app: &PhotocraftApp) -> Option<String> {
    app.framework.dialogs.stack().top().map(|id| id.value().to_string())
}

fn context_choose(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Reply {
    use crate::canvas_tool_menu::MenuKind;
    let Some(id) = str_param(p, "id") else { return err("missing `id`") };
    let Some(menu) = app.ui.canvas_tool_menu.as_ref() else { return err("no canvas context menu is open") };
    if matches!(menu.kind, MenuKind::BrushMode { .. }) {
        return mode_menu_choose(app, id);
    }
    if !crate::canvas_tool_menu::available(app, menu, id) {
        return err("context action is unavailable");
    }
    // A pasteboard row changes a preference: the policy sees the `prefs.set` it runs.
    let (command, params) = crate::canvas_tool_menu::engine_call(app, menu, id);
    if let Some(authorize) = app.services.automation_command.as_ref()
        && let Err(error) = authorize(&command, &params)
    {
        return err(error);
    }
    crate::canvas_tool_menu::choose(app, ctx, id);
    ok(json!({"command": id, "dialog": top_dialog(app)}))
}

/// A row of the painting tools' Shift+right-click menu, by its English label (`brushModeMenu.choices`).
fn mode_menu_choose(app: &mut PhotocraftApp, label: &str) -> Reply {
    let item = match crate::brush_mode_menu::find(app, label) {
        Ok(item) => item,
        Err(e) => return err(e),
    };
    if let Some((command, params)) = crate::brush_mode_menu::engine_call(app, app.ui.tool, item)
        && let Some(authorize) = app.services.automation_command.as_ref()
        && let Err(error) = authorize(command, &params)
    {
        return err(error);
    }
    crate::brush_mode_menu::choose(app, item);
    ok(json!({"choice": label, "brushPicker": app.ui.brush_picker.is_some()}))
}

/// `engine.execute` and `ui.menu.invoke`: run command `command` (or `id`) with `params`, as the menu would or, for `engine.execute`, directly.
fn execute_command(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Reply {
    let p = &req.params;
    let Some(id) = str_param(p, "command").or(str_param(p, "id")) else { return err("missing `command`") };
    if req.method == "engine.execute" && METHODS.contains(&id) && photocraft_engine::commands::find(id).is_none() {
        return err(photocraft_engine::command_hints::protocol_method_as_command(id));
    }
    let params = p.get("params").cloned().unwrap_or(json!({}));
    let wait = wait_param(p);
    // A control-channel menu click must respect the same modal gate as the native menu.
    // The documented `ui` requests of Camera Raw and the full-window workspaces operate
    // the already-open one rather than click a menu. Only that continuation may pass its
    // own modal gate: one `ui` object, for the workspace that is the top of the hosted
    // stack. Both command entry points respect document ownership.
    let continuation = crate::distort_ui::modal_workspace(app) == Some(id)
        && params.as_object().is_some_and(|fields| fields.len() == 1 && fields.get("ui").is_some_and(Value::is_object))
        && app.framework.dialogs.open_dialogs().last().is_some_and(|dialog| dialog.key.as_deref() == Some(id))
        && !crate::crop_ui::blocks(app, id);
    if !continuation && !crate::menus::allows(app, &CommandRef::new(id)) {
        return err("command is unavailable while a modal dialog is open");
    }
    if let Some(authorize) = app.services.automation_command.as_ref()
        && let Err(error) = authorize(id, &params)
    {
        return err(error);
    }
    // `engine.execute` is programmatic: engine commands run directly with their default
    // params and never open a dialog (an agent would otherwise get a modal instead of a
    // result). `ui.menu.invoke` behaves like a menu click, so it may open the dialog.
    if req.method == "engine.execute" && photocraft_engine::commands::find(id).is_some() {
        return run_waiting(app, wait, |app| {
            // These bypass `menus::invoke`: a Type box drag in progress applies its box first.
            crate::type_box_preview::end_drag(app);
            let previous = std::mem::replace(&mut app.programmatic_playback, true);
            let result = app.run(id, params);
            app.programmatic_playback = previous;
            result
        });
    }
    run_waiting(app, wait, |app| crate::menus::invoke(app, ctx, id, params))
}

fn jobs_cancel(app: &mut PhotocraftApp, p: &Value) -> Reply {
    let all = p.get("job").is_none_or(Value::is_null);
    match p.get("job").and_then(Value::as_u64) {
        // Same reply as the engine's `jobs.cancel`: `{cancelled: n}`, plus `notCancelled:
        // [{job, reason}]` for the jobs left running (a save already writing its file, a print).
        Some(j) if app.session.job(JobId(j)).is_some() => ok(cancel_each(app, vec![JobId(j)]).to_json()),
        Some(j) => err(format!("no running job {j}")),
        None if all => {
            let ids: Vec<_> = app.session.jobs().into_iter().map(|j| j.id).collect();
            ok(cancel_each(app, ids).to_json())
        }
        None => err("`job` must be a job id"),
    }
}

fn cancel_each(app: &mut PhotocraftApp, ids: Vec<JobId>) -> CancelReport {
    let mut report = CancelReport::default();
    for id in ids {
        match crate::jobs_ui::try_cancel(app, id) {
            Ok(()) => report.cancelled.push(id),
            Err(why) => report.refused.push((id, why)),
        }
    }
    report
}

// ---- the shell: view state, windows, GPU, focus, screenshots, notices ----

fn shell_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    let p = &req.params;
    Some(match req.method.as_str() {
        "ui.set" => ui_set(app, ctx, p),
        "ui.window.open" => {
            if let Some(d) = u64_param(p, "document")
                && !app.session.set_active(d as usize)
            {
                return Some(err(format!("no document {d}")));
            }
            Reply::from_result(crate::menus::invoke(app, ctx, "window.newWindowForDocument", json!({})))
        }
        "ui.window.close" => {
            let Some(id) = u64_param(p, "window") else { return Some(err("missing `window`")) };
            let before = app.ui.windows.len();
            app.ui.windows.retain(|w| w.id != id);
            if app.ui.windows.len() < before { ok(Value::Null) } else { err(format!("no window {id}")) }
        }
        "ui.resize" => {
            let (w, h) = (p.get("width").and_then(Value::as_f64).unwrap_or(1280.0), p.get("height").and_then(Value::as_f64).unwrap_or(800.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32)));
            ok(Value::Null)
        }
        "ui.gpu.simulateLoss" => simulate_gpu_loss(app, ctx, p),
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            // The window host sends `Minimized(false)` first and asks for attention where the platform cannot focus.
            WindowHost::of(ctx).act(egui::ViewportId::ROOT, WindowAction::Focus);
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.screenshot" => screenshot(app, ctx, p),
        "ui.notice.post" => notice_post(app, p),
        _ => return None,
    })
}

fn simulate_gpu_loss(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Reply {
    if p.get("destroy").and_then(Value::as_bool).unwrap_or(false) {
        let destroyed = app.destroy_window_device();
        ctx.request_repaint();
        return ok(json!({"destroyed": destroyed}));
    }
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

fn screenshot(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Reply {
    let parsed = input_object(p, None, &["path", "focus", "window"]).and_then(|p| {
        Ok((
            input_choice(&p, "path", "path", &[])?.map(str::to_string),
            bool_field(&p, "focus")?,
            input_choice(&p, "window", "window", &[])?.map(str::to_string),
        ))
    });
    let (path, focus, window) = match parsed {
        Ok(parsed) => parsed,
        Err(e) => return err(e),
    };
    let host = WindowHost::of(ctx);
    let viewport = match window.as_deref() {
        None | Some("main") => egui::ViewportId::ROOT,
        Some(id) => match host.viewport_of(egui::Id::unique(id)) {
            Some(viewport) => viewport,
            None => return err(format!("no open window `{id}`")),
        },
    };
    // Occluded windows don't render on macOS, so the screenshot would never arrive: raise first.
    if focus.unwrap_or(true) {
        host.act(viewport, WindowAction::Focus);
    }
    // The capture itself is admitted by `PhotocraftApp::drain_control` (budget and
    // geometry) and issued from a later `App::ui` pass once open/close animations
    // (modals, popups) have settled (craft-automation's screenshot queue).
    let token = app.ui.alloc_id();
    ctx.request_repaint();
    Reply::screenshot(token, path, viewport)
}

/// `ui.notice.post {title, lines?, tone?}`: shows a notice in the named tone and replies its id.
fn notice_post(app: &mut PhotocraftApp, p: &Value) -> Reply {
    const TONES: [&str; 6] = ["neutral", "accent", "info", "good", "warn", "bad"];
    let parsed = input_object(p, None, &["title", "lines", "tone"]).and_then(|p| {
        let title = input_choice(&p, "title", "title", &[])?.ok_or_else(|| "missing `title`".to_string())?.to_string();
        let lines = match p.get("lines") {
            None => Vec::new(),
            Some(Value::Array(lines)) => lines
                .iter()
                .map(|line| line.as_str().map(str::to_string).ok_or_else(|| "lines must be an array of strings".to_string()))
                .collect::<std::result::Result<Vec<_>, _>>()?,
            Some(_) => return Err("lines must be an array of strings".to_string()),
        };
        let name = input_choice(&p, "tone", "tone", &TONES)?.unwrap_or("neutral");
        let tone = craft::theme::variant::Tone::from_name(name).ok_or_else(|| format!("unknown tone `{name}`"))?;
        Ok((title, lines, tone))
    });
    let (title, lines, tone) = match parsed {
        Ok(parsed) => parsed,
        Err(e) => return err(e),
    };
    let id = crate::notices::post(app, title, lines, tone, None);
    ok(json!({"id": id}))
}

// ---- ui.set: every field is validated and materialized into a `UiSet` before the first one is applied ----

fn ui_set(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Reply {
    if let Some(field) = p.as_object().and_then(|o| o.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str()))) {
        return err(format!("unknown field `{field}` (fields: {})", UI_SET_FIELDS.join(", ")));
    }
    // Everything is validated and materialized before the first field is applied, so a
    // rejected call applies none of its fields (#412's guarantee, down to the values
    // and nested keys).
    Reply::from_result(UiSet::parse(app, ctx, p).and_then(|set| {
        if let Some((id, _)) = &set.theme {
            app.check_theme(id)?;
        }
        set.apply(app, ctx)
    }))
}

struct UiSet {
    tools: ToolSet,
    crop: CropSet,
    panels: PanelSet,
    view: ViewSet,
    /// The suite theme to show and whether it is the dark one.
    theme: Option<(craft::theme::ThemeId, bool)>,
    brush: BrushSet,
    /// The workspace edits of `panels`, `dockTabs`, `dock` and `dockWidth`, run as one atomic list.
    workspace: Vec<Action<String>>,
}

impl UiSet {
    fn parse(app: &PhotocraftApp, ctx: &egui::Context, p: &Value) -> std::result::Result<Self, String> {
        let tools = ToolSet::parse(app, p)?;
        let crop = CropSet::parse(app, p)?;
        let panels = PanelSet::parse(app, p)?;
        let view = ViewSet::parse(app, p)?;
        let theme = theme_param(app, p)?;
        let brush = BrushSet::parse(app, p)?;
        // A refused `dock.order` or `dock.heights` refuses the whole request.
        let mut workspace = crate::panel_docking::legacy_actions(app, panels.panels_legacy.as_ref(), panels.dock_tabs.as_ref(), p.get("dock"))?;
        if let Some(total) = panels.dock_width {
            workspace.extend(crate::panel_docking::width_actions(app, ctx, total as f32));
        }
        // Tab 0 is Brush Settings, tab 1 the Brushes panel.
        if let Some(tab) = uint_field(p, "brushTab")? {
            workspace.push(Action::open(if tab == 0 { "brushSettings" } else { "brushes" }.to_owned(), None));
        }
        // The whole list is checked on a clone of the workspace, so `apply` cannot refuse it.
        crate::panel_docking::check(app, &workspace)?;
        Ok(Self { tools, crop, panels, view, theme, brush, workspace })
    }

    /// Apply (nothing but the frame selection and the final `brushSize` command can fail; the workspace list was checked).
    fn apply(mut self, app: &mut PhotocraftApp, ctx: &egui::Context) -> std::result::Result<Value, String> {
        // All fields are validated and the workspace list is checked. Select the requested frame
        // thumbnail before applying anything else, so a denied selection leaves every field intact.
        self.tools.select_frame_target(app)?;
        if !self.workspace.is_empty() {
            crate::panel_docking::apply(app, std::mem::take(&mut self.workspace))?;
        }
        self.tools.apply(app);
        self.crop.apply(app);
        self.panels.apply_options(app);
        let note = self.theme.and_then(|(id, dark)| app.pick_theme(ctx, &id, dark));
        self.panels.apply_color(app);
        self.view.apply(app);
        self.brush.apply(app)?;
        Ok(note.map_or(Value::Null, |note| json!({"note": note})))
    }
}

/// `theme`: a suite theme id, shown in the mode on screen, or a name PhotoCraft took before theme
/// ids (`proMedium`, `Pro (Medium Gray)`, `gnome`, ...), shown in its own mode.
fn theme_param(app: &PhotocraftApp, p: &Value) -> std::result::Result<Option<(craft::theme::ThemeId, bool)>, String> {
    let Some(name) = str_param(p, "theme") else { return Ok(None) };
    let (id, dark) = photocraft_engine::prefs::theme_ids::parse(name).ok_or_else(|| format!("unknown theme `{name}`"))?;
    Ok(Some((id, dark.unwrap_or(app.ui.theme.is_dark()))))
}

// -- tools: stroke, tool, gradient, eyedropper, Content-Aware Move, tile grid, layer filter, marquee --

struct ToolSet {
    gradient_blend: Option<photocraft_color::BlendMode>,
    shape_stroke: Option<Option<photocraft_doc::ShapeStroke>>,
    frame_shape: Option<String>,
    frame_sides: Option<u32>,
    frame_target: Option<crate::frame_tool::Target>,
    tool: Option<Tool>,
    gradient_classic: Option<bool>,
    stroke_trace: Option<&'static str>,
    eyedropper_size: Option<craft::color_ui::ui::SampleSize>,
    eyedropper_sample: Option<craft::color_ui::ui::SampleSource>,
    eyedropper_ring: Option<bool>,
    cam_transform_on_drop: Option<bool>,
    move_show_transform: Option<bool>,
    tile_grid: Option<bool>,
    layer_filter: Option<craft::layers::ui::filter::LayerFilter>,
    pen_mode: Option<craft::draw::ui::shape::VectorMode>,
    pen_path_op: Option<String>,
    marquee_style: Option<String>,
    marquee_size: Option<[f32; 2]>,
}

impl ToolSet {
    /// Validate the complete tool patch before applying any field; omitted settings are retained.
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let (marquee_style, marquee_size) = marquee_params(app, p)?;
        Ok(Self {
            gradient_blend: gradient_blend_param(p)?,
            shape_stroke: shape_stroke_param(app, p)?,
            frame_shape: frame_shape_param(p)?,
            frame_sides: frame_sides_param(p)?,
            frame_target: frame_target_param(p)?,
            tool: tool_param(p)?,
            gradient_classic: bool_field(p, "gradientClassic")?,
            stroke_trace: stroke_trace_param(p)?,
            // The Eyedropper's options bar (#1649): Sample Size, Sample, Show Sampling Ring.
            eyedropper_size: eyedropper_size_param(p)?,
            eyedropper_sample: eyedropper_sample_param(p)?,
            eyedropper_ring: bool_field(p, "eyedropperRing")?,
            // Content-Aware Move's Transform On Drop checkbox.
            cam_transform_on_drop: bool_field(p, "camTransformOnDrop")?,
            move_show_transform: bool_field(p, "moveShowTransform")?,
            // Developer overlay of the active layer's storage tiles (tile_overlay.rs).
            tile_grid: bool_field(p, "tileGrid")?,
            layer_filter: p.get("layerFilter").map(|v| crate::layer_filter_ui::patched(&app.ui.layer_filter, v)).transpose()?,
            pen_mode: pen_mode_param(p)?,
            pen_path_op: pen_path_op_param(p)?,
            marquee_style,
            marquee_size,
        })
    }

    fn select_frame_target(&self, app: &mut PhotocraftApp) -> std::result::Result<(), String> {
        let Some(target) = self.frame_target else { return Ok(()) };
        if let Some(id) = crate::frame_tool::selected(app).map(|l| l.id) {
            crate::frame_tool::select_result(app, id, target)?;
        } else {
            app.ui.frame_tool.target = target;
            app.ui.frame_tool.selected = None;
        }
        Ok(())
    }

    /// Apply a validated patch, selecting the tool before its per-tool settings.
    fn apply(self, app: &mut PhotocraftApp) {
        if let Some(shape) = &self.frame_shape {
            app.ui.frame_tool.ellipse = shape == "ellipse";
            app.ui.frame_tool.shape.clone_from(shape);
        }
        if let Some(sides) = self.frame_sides {
            app.ui.frame_tool.sides = sides;
        }
        if let Some(stroke) = &self.shape_stroke {
            if let Some(stroke) = stroke {
                app.ui.tool_options.stroke_width = stroke.width;
                app.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions::from(stroke);
            } else {
                app.ui.tool_options.stroke_width = 0.0;
            }
        }
        if let Some(t) = self.tool {
            crate::tool_memory::select(app, t);
            // Each tool keeps its own brush (#218), so switch it in before `brushSize`
            // sets the new tool's size.
            crate::paint_mouse::sync_tool_brush(app);
        }
        self.apply_gradient_and_trace(app);
        let eyedropper = &mut app.ui.tool_options.eyedropper;
        if let Some(size) = self.eyedropper_size {
            eyedropper.size = size;
        }
        if let Some(source) = self.eyedropper_sample {
            eyedropper.source = source;
        }
        if let Some(ring) = self.eyedropper_ring {
            eyedropper.show_ring = ring;
        }
        if let Some(on) = self.cam_transform_on_drop {
            app.ui.tool_options.cam_transform_on_drop = on;
        }
        if let Some(on) = self.move_show_transform {
            app.ui.tool_options.move_show_transform = on;
        }
        if let Some(on) = self.tile_grid {
            app.ui.view.tile_grid = on;
        }
        if let Some(f) = self.layer_filter {
            crate::layer_filter_ui::apply(app, f);
        }
        if let Some(m) = self.pen_mode {
            app.ui.tool_options.vector_mode = m;
        }
        if let Some(o) = self.pen_path_op {
            app.ui.tool_options.pen_path_op = o;
        }
        let o = &mut app.ui.tool_options;
        if let Some(style) = self.marquee_style {
            o.marquee_style = style;
        }
        if let Some([w, h]) = self.marquee_size {
            (o.marquee_width, o.marquee_height) = (w, h);
        }
    }

    fn apply_gradient_and_trace(&self, app: &mut PhotocraftApp) {
        let gradient_before = app.ui.tool_options.clone();
        if let Some(mode) = self.gradient_blend {
            app.ui.tool_options.gradient_blend_mode = mode;
        }
        if let Some(v) = &self.stroke_trace {
            app.stroke_trace.set(v);
        }
        if let Some(classic) = self.gradient_classic {
            app.ui.tool_options.gradient_classic = classic;
        }
        if self.gradient_blend.is_some() {
            crate::gradient_ui::options_changed(app, &gradient_before);
        }
    }
}

fn frame_shape_param(p: &Value) -> std::result::Result<Option<String>, String> {
    p.get("frameShape")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_str()
                .filter(|s| ["rectangle", "ellipse", "triangle", "polygon", "custom"].contains(s))
                .map(str::to_string)
                .ok_or_else(|| "frameShape must be rectangle, ellipse, triangle, polygon or custom".to_string())
        })
        .transpose()
}

fn frame_sides_param(p: &Value) -> std::result::Result<Option<u32>, String> {
    match uint_field(p, "frameSides")? {
        Some(n) if !(3..=100).contains(&n) => Err("frameSides must be 3..=100".into()),
        n => Ok(n.map(|n| n as u32)),
    }
}

fn frame_target_param(p: &Value) -> std::result::Result<Option<crate::frame_tool::Target>, String> {
    p.get("frameTarget")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value::<crate::frame_tool::Target>(v.clone()).map_err(|_| "frameTarget must be frame, content or both".to_string()))
        .transpose()
}

/// The Pen tools' options bar (#1397): Path or Shape mode.
fn pen_mode_param(p: &Value) -> std::result::Result<Option<craft::draw::ui::shape::VectorMode>, String> {
    use craft::draw::ui::shape::VectorMode;
    let Some(v) = p.get("penMode") else { return Ok(None) };
    match v.as_str() {
        Some("path") => Ok(Some(VectorMode::Path)),
        Some("shape") => Ok(Some(VectorMode::Shape)),
        _ => Err("penMode must be \"path\" or \"shape\"".to_string()),
    }
}

/// The Pen tools' Path Operation.
fn pen_path_op_param(p: &Value) -> std::result::Result<Option<String>, String> {
    let Some(v) = p.get("penPathOp") else { return Ok(None) };
    v.as_str()
        .filter(|o| crate::vector_ui::PATH_OPS.iter().any(|(id, ..)| id == o))
        .map(|o| Some(o.to_string()))
        .ok_or_else(|| "penPathOp must be one of newLayer, combine, subtract, intersect, exclude".to_string())
}

/// The marquee's Style (`normal`, `fixedRatio`, `fixedSize`) and its Width and Height, which must fit the range the
/// style (the new one, else the current one) gives them.
fn marquee_params(app: &PhotocraftApp, p: &Value) -> std::result::Result<(Option<String>, Option<[f32; 2]>), String> {
    let style = match p.get("marqueeStyle") {
        None => None,
        Some(v) => match v.as_str() {
            Some(s @ ("normal" | "fixedRatio" | "fixedSize")) => Some(s.to_string()),
            _ => return Err("marqueeStyle must be normal, fixedRatio or fixedSize".into()),
        },
    };
    let size = match p.get("marqueeSize") {
        None => None,
        Some(v) => {
            let numbers = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]));
            let Some([w, h]) = numbers.filter(|n| n.iter().all(f32_range)) else {
                return Err("marqueeSize must be [width, height] of two finite numbers".into());
            };
            let range = crate::tool_memory::style_range(style.as_deref().unwrap_or(&app.ui.tool_options.marquee_style));
            if ![w, h].iter().all(|n| range.contains(&(*n as f32))) {
                return Err(format!("marqueeSize must be within {} and {} for this style", range.start(), range.end()));
            }
            Some([w as f32, h as f32])
        }
    };
    Ok((style, size))
}

fn gradient_blend_param(p: &Value) -> std::result::Result<Option<photocraft_color::BlendMode>, String> {
    let Some(value) = p.get("gradientBlendMode") else { return Ok(None) };
    let Some(name) = value.as_str() else { return Err("gradientBlendMode must be a blend mode name".into()) };
    photocraft_engine::commands::parse_blend(name)
        .filter(|m| photocraft_color::BlendMode::LAYER_MODES.contains(m))
        .map(Some)
        .ok_or_else(|| format!("unknown gradient blend mode `{name}`"))
}

fn shape_stroke_param(app: &PhotocraftApp, p: &Value) -> std::result::Result<Option<Option<photocraft_doc::ShapeStroke>>, String> {
    let Some(v) = p.get("shapeStroke") else { return Ok(None) };
    if let Some(object) = v.as_object() {
        let keys = ["width", "opacity", "align", "cap", "join", "miterLimit", "dashes", "dashOffset"];
        if let Some(key) = object.keys().find(|key| !keys.contains(&key.as_str())) {
            return Err(format!("unknown shapeStroke field `{key}`"));
        }
    }
    let o = &app.ui.tool_options;
    let base = o.shape_stroke.stroke(o.stroke_width, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
    Ok(Some(photocraft_engine::vector_cmds::parse_stroke(v, Some(base), photocraft_doc::Color::BLACK)?))
}

fn tool_param(p: &Value) -> std::result::Result<Option<Tool>, String> {
    match str_param(p, "tool") {
        Some(t) => Tool::from_name(t).map(Some).ok_or_else(|| format!("unknown tool `{t}`")),
        None => Ok(None),
    }
}

/// The stroke latency trace (`stroke_trace`), at runtime: on (stderr) or off. A CSV
/// file is only for the user's own launch (`--stroke-trace <path>`): a path over the
/// control channel would write outside the automation write root.
fn stroke_trace_param(p: &Value) -> std::result::Result<Option<&'static str>, String> {
    match p.get("strokeTrace") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(if *b { "stderr" } else { "off" })),
        Some(Value::String(v)) => match v.trim() {
            "stderr" | "on" | "true" => Ok(Some("stderr")),
            "off" | "false" => Ok(Some("off")),
            _ => Err("strokeTrace must be true, false, \"stderr\" or \"off\"; a CSV file is set at launch (--stroke-trace <path>)".into()),
        },
        Some(_) => Err("strokeTrace must be true, false, \"stderr\" or \"off\"".into()),
    }
}

fn eyedropper_size_param(p: &Value) -> std::result::Result<Option<craft::color_ui::ui::SampleSize>, String> {
    use craft::color_ui::ui::SampleSize;
    let Some(v) = p.get("eyedropperSampleSize") else { return Ok(None) };
    let refused = || format!("eyedropperSampleSize must be \"point\" or one of {:?}", photocraft_engine::sample_cmds::SAMPLE_SIZES);
    let pixels = photocraft_engine::sample_cmds::size_param("ui.set", &json!({"size": v})).map_err(|_| refused())?;
    SampleSize::from_pixels(pixels).map(Some).ok_or_else(refused)
}

fn eyedropper_sample_param(p: &Value) -> std::result::Result<Option<craft::color_ui::ui::SampleSource>, String> {
    use craft::color_ui::ui::SampleSource;
    let Some(v) = p.get("eyedropperSample") else { return Ok(None) };
    let ids: Vec<&str> = photocraft_engine::sample_cmds::SampleLayers::ALL.iter().map(|(_, n)| *n).collect();
    let refused = || format!("eyedropperSample must be one of {}", ids.join(", "));
    let id = v.as_str().filter(|id| ids.contains(id)).ok_or_else(refused)?;
    SampleSource::from_id(id).map(Some).ok_or_else(refused)
}

// -- the Crop tool's overlay, Fill and gear menu --

struct CropSet {
    writes: Vec<Write>,
}

impl CropSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let crop = &app.ui.tool_options.crop;
        let mut writes: Vec<Write> = Vec::new();
        // The Crop tool's overlay menu (#1919).
        if let Some(v) = p.get("cropOverlay") {
            let guide = parse_like(&crop.guide, v, || {
                let ids: Vec<String> = CropGuide::ALL.iter().filter_map(|g| serde_json::to_value(g).ok()?.as_str().map(str::to_string)).collect();
                format!("cropOverlay must be one of {}", ids.join(", "))
            })?;
            writes.push(Box::new(move |app| app.ui.tool_options.crop.guide = guide));
        }
        if let Some(v) = p.get("cropOverlayShow") {
            let show = parse_like(&crop.guide_show, v, || "cropOverlayShow must be one of auto, always, never".to_string())?;
            writes.push(Box::new(move |app| app.ui.tool_options.crop.guide_show = show));
        }
        match uint_field(p, "cropOverlayOrientation")? {
            Some(o) if o < 4 => writes.push(Box::new(move |app| app.ui.tool_options.crop.guide_orientation = o as u8)),
            Some(_) => return Err("cropOverlayOrientation must be 0, 1, 2 or 3".into()),
            None => {}
        }
        // The Crop tool's Fill (#3823): "background" (Background (default), or Transparent
        // (default) with Delete Cropped Pixels off) or "contentAware".
        if let Some(v) = p.get("cropFill") {
            if !matches!(v.as_str(), Some("background" | "contentAware")) {
                return Err("cropFill must be \"background\" or \"contentAware\"".into());
            }
            let fill = parse_like(&crop.fill, v, || "cropFill must be \"background\" or \"contentAware\"".to_string())?;
            writes.push(Box::new(move |app| app.ui.tool_options.crop.fill = fill));
        }
        // The Crop tool's gear menu: Show Cropped Area and the shield (#1919).
        if let Some(shield) = merged_object(&crop.shield, p.get("cropShield"), "cropShield")? {
            if !(0.0..=100.0).contains(&shield.opacity) {
                return Err("cropShield.opacity must be 0..100".into());
            }
            writes.push(Box::new(move |app| app.ui.tool_options.crop.shield = shield));
        }
        Ok(Self { writes })
    }

    fn apply(self, app: &mut PhotocraftApp) {
        for write in self.writes {
            write(app);
        }
    }
}

// -- panels: Histogram, masks targets, docking, Color panel, chrome --

/// The `panels` object split in two: the keys `Panels` has, as a patch with its `task_bar` merged into the bar's
/// current place (so `{"at": [x, y]}` keeps a pin and `{"pinned": true}` keeps a dragged position, #10890), and the
/// group words of the old panel list (`layers`, `history`, ...), as an object for `panel_docking::legacy_actions`.
/// A key in neither is refused with its name.
fn panels_patch(app: &PhotocraftApp, patch: Option<&Value>) -> std::result::Result<(Option<Value>, Option<Value>), String> {
    let Some(patch) = patch else { return Ok((None, None)) };
    let Some(object) = patch.as_object() else { return Err("panels must be an object".into()) };
    let own: Vec<String> =
        serde_json::to_value(&app.ui.panels).map_err(|e| e.to_string())?.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    let groups = crate::panel_docking::legacy_view(app);
    let group_known = |key: &str| groups.get("panels").is_some_and(|panels| panels.get(key).is_some());
    let mut own_patch = serde_json::Map::new();
    let mut legacy = serde_json::Map::new();
    for (key, value) in object {
        if own.contains(key) {
            own_patch.insert(key.clone(), value.clone());
        } else if group_known(key) {
            legacy.insert(key.clone(), value.clone());
        } else {
            return Err(format!("unknown panels field `{key}`"));
        }
    }
    if let Some(bar) = own_patch.get("task_bar") {
        let merged = merged_object(&app.ui.panels.task_bar, Some(bar), "panels.task_bar")?;
        own_patch.insert("task_bar".into(), serde_json::to_value(merged).map_err(|e| e.to_string())?);
    }
    let own_patch = (!own_patch.is_empty()).then_some(Value::Object(own_patch));
    let legacy = (!legacy.is_empty()).then_some(Value::Object(legacy));
    Ok((own_patch, legacy))
}

struct PanelSet {
    histogram_refresh: Option<bool>,
    panels: Option<crate::state::Panels>,
    /// The group words of `panels` (`layers`, `history`, ...), for `legacy_actions`.
    panels_legacy: Option<Value>,
    mask_target: Option<bool>,
    updates_open: Option<bool>,
    vector_mask_target: Option<bool>,
    filter_mask_target: Option<bool>,
    selection_mode: Option<u64>,
    /// `dockTabs` merged onto the workspace's legacy view.
    dock_tabs: Option<Value>,
    color_panel: Option<crate::state::ColorPanelState>,
    dock_width: Option<f64>,
    /// The Histogram settings and range, the Actions panel and preset browser settings, the toolbar columns, the recent
    /// view and the status bar item, written in the order given.
    writes: Vec<Write>,
}

impl PanelSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let mut writes: Vec<Write> = Vec::new();
        // The Histogram panel: its options, the dragged tonal range ([from, to] levels or
        // null) and Uncached Refresh.
        if let Some(settings) = merged_object(&app.ui.histogram.settings, p.get("histogramPanel"), "histogramPanel")? {
            writes.push(Box::new(move |app| {
                app.ui.histogram.settings = settings;
                crate::histogram_panel::persist_options(app);
            }));
        }
        if let Some(range) = histogram_range_param(p)? {
            writes.push(Box::new(move |app| app.ui.histogram.range = range));
        }
        if let Some(settings) = merged_object(&app.ui.actions.panel.settings, p.get("actionsPanel"), "actionsPanel")? {
            writes.push(Box::new(move |app| app.ui.actions.panel.settings = settings));
        }
        writes.extend(preset_browsers_param(app, p)?);
        // The toolbar's width in tool columns: its dock column's steps, from one to the cells of its grid.
        if let Some(columns) = uint_field(p, "toolbarColumns")? {
            let most = crate::panels::toolbar::cells(app);
            let steps = u16::try_from(columns).ok().filter(|steps| (1..=most).contains(&usize::from(*steps)));
            let steps = steps.ok_or_else(|| format!("toolbarColumns must be 1 to {most}"))?;
            writes.push(Box::new(move |app| {
                let action = Action::set_column_steps(crate::panels::registry::TOOLBAR.to_owned(), steps);
                if let Err(error) = crate::panel_docking::apply(app, vec![action]) {
                    app.ui.status = error;
                }
            }));
        }
        if let Some(v) = p.get("recentView") {
            let view = parse_like(&app.ui.chrome.home_recent_view, v, || "recentView must be grid or list".to_string())?;
            writes.push(Box::new(move |app| app.ui.chrome.home_recent_view = view));
        }
        if let Some(v) = p.get("statusInfo") {
            let key = v
                .as_str()
                .filter(|key| crate::chrome_ui::status_item_of(key).is_some())
                .ok_or_else(|| "statusInfo is not a status bar item".to_string())?
                .to_string();
            writes.push(Box::new(move |app| app.ui.chrome.status_info = key));
        }
        let dock_tabs = if p.get("dockTabs").is_some() {
            // Every group is a known field, hidden ones too: the request may open the group and pick its tab at once.
            let mut current = crate::panel_docking::legacy_view(app).get("dockTabs").and_then(Value::as_object).cloned().unwrap_or_default();
            for (group, _) in crate::panels::registry::LEGACY_GROUPS {
                current.entry((*group).to_owned()).or_insert(Value::Null);
            }
            let mut merged = merged_object(&Value::Object(current), p.get("dockTabs"), "dockTabs")?;
            if let Some(Value::Object(tabs)) = merged.as_mut() {
                tabs.retain(|_, tab| !tab.is_null());
            }
            merged
        } else {
            None
        };
        let (panels_patch, panels_legacy) = panels_patch(app, p.get("panels"))?;
        Ok(Self {
            histogram_refresh: bool_field(p, "histogramRefresh")?,
            panels: merged_object(&app.ui.panels, panels_patch.as_ref(), "panels")?,
            panels_legacy,
            mask_target: bool_field(p, "maskTarget")?,
            updates_open: bool_field(p, "updatesOpen")?,
            vector_mask_target: bool_field(p, "vectorMaskTarget")?,
            filter_mask_target: filter_mask_target_param(app, p)?,
            selection_mode: uint_field(p, "selectionMode")?,
            dock_tabs,
            // Which chip the Color panel edits.
            color_panel: merged_object(&app.ui.color_panel, p.get("colorPanel"), "colorPanel")?,
            dock_width: f32_field(p, "dockWidth")?,
            writes,
        })
    }

    /// Everything that precedes the theme.
    fn apply_options(&mut self, app: &mut PhotocraftApp) {
        if let Some(v) = self.panels.take() {
            app.ui.panels = v;
        }
        for write in std::mem::take(&mut self.writes) {
            write(app);
        }
        if self.histogram_refresh == Some(true) {
            crate::histogram_panel::uncached_refresh(app);
        }
        self.apply_mask_targets(app);
        // Selection tools' options-bar mode: 0 New, 1 Add, 2 Subtract, 3 Intersect.
        if let Some(m) = self.selection_mode {
            app.ui.selection_mode = m.min(3) as u8;
        }
    }

    fn apply_mask_targets(&self, app: &mut PhotocraftApp) {
        if let Some(m) = self.mask_target {
            app.end_filter_mask_view();
            app.ui.filter_mask_target = None;
            app.ui.mask_target = m;
            app.ui.properties_layer = None;
            app.ui.vector_mask_target &= !m;
            if m {
                app.target_mask_over_colour_rows();
            }
            app.sync_mask_targets();
        }
        if let Some(open) = self.updates_open {
            app.ui.updates_open = open;
        }
        if let Some(m) = self.vector_mask_target {
            app.end_filter_mask_view();
            app.ui.filter_mask_target = None;
            app.ui.vector_mask_target = m;
            app.ui.mask_target &= !m;
            app.sync_mask_targets();
        }
        if let Some(m) = self.filter_mask_target {
            let layer = if m { app.session.active().and_then(|st| st.active_layer).map(|id| id.0) } else { None };
            app.select_filter_mask_target(layer);
        }
    }

    /// Applies the Color panel state.
    fn apply_color(&mut self, app: &mut PhotocraftApp) {
        if let Some(v) = self.color_panel.take() {
            app.ui.color_panel = v;
        }
    }
}

/// `presetBrowsers {<panel>: {view, zoom}}`: the preset panels' browser settings, each merged into the panel's own.
fn preset_browsers_param(app: &PhotocraftApp, p: &Value) -> std::result::Result<Vec<Write>, String> {
    let Some(v) = p.get("presetBrowsers") else { return Ok(Vec::new()) };
    let Some(panels) = v.as_object() else { return Err("presetBrowsers must be an object".into()) };
    let mut writes: Vec<Write> = Vec::new();
    for (panel, patch) in panels {
        let Some(current) = app.ui.presets_ui.browsers.get(panel.as_str()) else { return Err(format!("unknown preset panel `{panel}`")) };
        let field = format!("presetBrowsers.{panel}");
        if let Some(key) = patch.as_object().and_then(|o| o.keys().find(|k| !["view", "zoom"].contains(&k.as_str()))) {
            return Err(format!("unknown {field} field `{key}` (fields: view, zoom)"));
        }
        if patch.get("zoom").is_some_and(|z| !z.as_f64().is_some_and(f64::is_finite)) {
            return Err(format!("{field}.zoom must be a finite number"));
        }
        let Some(merged) = merged_object(current, Some(patch), &field)? else { continue };
        let panel = panel.clone();
        writes.push(Box::new(move |app| {
            if let Some(slot) = app.ui.presets_ui.browsers.get_mut(panel.as_str()) {
                *slot = merged;
            }
        }));
    }
    Ok(writes)
}

fn histogram_range_param(p: &Value) -> std::result::Result<Option<Option<craft::imaging::ui::histogram::LevelRange>>, String> {
    let Some(v) = p.get("histogramRange") else { return Ok(None) };
    serde_json::from_value::<Option<[u8; 2]>>(v.clone())
        .map(|range| Some(range.map(|[from, to]| craft::imaging::ui::histogram::LevelRange::new(f32::from(from), f32::from(to)))))
        .map_err(|_| "histogramRange must be [from, to] levels 0..255, or null".to_string())
}

/// Target the active smart object's filter mask (FILE-215-3): it needs smart filters.
fn filter_mask_target_param(app: &PhotocraftApp, p: &Value) -> std::result::Result<Option<bool>, String> {
    let target = bool_field(p, "filterMaskTarget")?;
    if target == Some(true) {
        check_filter_mask_target(app)?;
    }
    Ok(target)
}

fn check_filter_mask_target(app: &PhotocraftApp) -> std::result::Result<(), String> {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let st = app.session.active().ok_or("filterMaskTarget needs an open document")?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("filterMaskTarget needs an active layer")?;
    if !matches!(&l.content, photocraft_doc::LayerContent::Smart(sm) if !sm.smart_filters.is_empty()) {
        return Err("filterMaskTarget needs an active smart object with smart filters".into());
    }
    let composite = matches!(st.channel_view.target, ChannelTarget::Composite | ChannelTarget::LayerMask | ChannelTarget::VectorMask);
    if !composite || st.doc.quick_mask.is_some() {
        return Err("filterMaskTarget can't be set while a single channel or Quick Mask is targeted".into());
    }
    Ok(())
}

// -- the document view: zoom, centre, rotation, fit --

struct ViewSet {
    zoom: Option<f64>,
    rotation: Option<f64>,
    center: Option<[f32; 2]>,
    fit: bool,
}

impl ViewSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let zoom = f32_field(p, "zoom")?;
        let rotation = f32_field(p, "rotation")?;
        if rotation.is_some() && app.session.active_index().is_none() {
            return Err("no document open".into());
        }
        let center = center_param(p)?;
        let fit = bool_field(p, "fit")? == Some(true);
        Ok(Self { zoom, rotation, center, fit })
    }

    /// Writes the view the canvas binds (`app.ui.views[i]`), which the canvas shows on its next pass.
    fn apply(&self, app: &mut PhotocraftApp) {
        let Some(i) = app.session.active_index() else { return };
        if self.zoom.is_some() || self.center.is_some() || self.rotation.is_some() || self.fit {
            app.ui.views[i].zoom_animating = false;
            app.ui.views[i].zoom_anchor = None;
        }
        if let Some(z) = self.zoom {
            let size = app.session.documents().get(i).map_or(Size::ZERO, |st| Size::new(f64::from(st.doc.size.width), f64::from(st.doc.size.height)));
            app.ui.views[i].zoom = craft::behavior::canvas::ZoomLadder::clamp(z, size) as f32;
            app.ui.views[i].clear_pending();
        }
        if let Some(c) = self.center {
            app.ui.views[i].center = c;
            app.ui.views[i].clear_pending();
        }
        if self.fit {
            app.ui.views[i].clear_pending();
            app.ui.views[i].place(craft::ui::canvas::ViewCommands::FitOnScreen);
        }
        if let Some(r) = self.rotation {
            app.ui.views[i].rotation = normalized_angle(r) as f32;
        }
    }
}

fn center_param(p: &Value) -> std::result::Result<Option<[f32; 2]>, String> {
    let Some(v) = p.get("center") else { return Ok(None) };
    let a = v.as_array().ok_or_else(|| "center must be [x, y]".to_string())?;
    if a.len() != 2 {
        return Err("center must be exactly [x, y]".into());
    }
    let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) else {
        return Err("center must be [x, y] of two numbers".into());
    };
    if !f32_range(&x) || !f32_range(&y) {
        return Err("center must be finite numbers within f32 range".into());
    }
    Ok(Some([x as f32, y as f32]))
}

// -- brushes: sections, picker, size --

struct BrushSet {
    section: Option<u64>,
    brushes_view: Option<craft::ui::patterns::presets::PresetView>,
    picker_name: Option<bool>,
    picker_stroke: Option<bool>,
    picker_tip: Option<bool>,
    picker_scale: Option<f32>,
    picker: Option<Option<[f32; 2]>>,
    size: Option<f64>,
}

impl BrushSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let section = uint_field(p, "brushSection")?;
        let brushes_view = brushes_view_param(p)?;
        // The three parts of the picker's cards. At least one stays on (a rejected call
        // leaves them all alone).
        let picker_name = bool_field(p, "brushPickerName")?;
        let picker_stroke = bool_field(p, "brushPickerStroke")?;
        let picker_tip = bool_field(p, "brushPickerTip")?;
        check_picker_parts(app, [picker_name, picker_stroke, picker_tip])?;
        Ok(Self {
            section,
            brushes_view,
            picker_name,
            picker_stroke,
            picker_tip,
            picker_scale: picker_scale_param(p)?,
            picker: picker_param(p)?,
            size: brush_size_param(p)?,
        })
    }

    fn apply(&self, app: &mut PhotocraftApp) -> std::result::Result<(), String> {
        if let Some(i) = self.section {
            app.ui.brush_section = (i as usize).min(crate::brush_panel::SECTIONS.len() - 1);
        }
        if let Some(v) = self.brushes_view {
            app.ui.brushes_panel.view = v;
        }
        self.apply_picker(app);
        if let Some(size) = self.size {
            app.run("tools.setBrush", json!({"brush": {"size": size}})).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn apply_picker(&self, app: &mut PhotocraftApp) {
        let state = &mut app.ui.brush_picker_state;
        if let Some(v) = self.picker_name {
            state.parts.name = v;
        }
        if let Some(v) = self.picker_stroke {
            state.parts.stroke = v;
        }
        if let Some(v) = self.picker_tip {
            state.parts.tip = v;
        }
        if let Some(v) = self.picker_scale {
            state.browser.zoom = f64::from(v);
        }
        match self.picker {
            Some(Some([x, y])) => crate::brush_picker::open(app, [x, y]),
            Some(None) => crate::brush_picker::close(app),
            None => {}
        }
    }
}

fn brushes_view_param(p: &Value) -> std::result::Result<Option<craft::ui::patterns::presets::PresetView>, String> {
    let Some(v) = p.get("brushesView") else { return Ok(None) };
    serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushesView: {e} (list, grid)"))
}

fn check_picker_parts(app: &PhotocraftApp, parts: [Option<bool>; 3]) -> std::result::Result<(), String> {
    if parts.iter().all(Option::is_none) {
        return Ok(());
    }
    let shown = &app.ui.brush_picker_state.parts;
    let on = [parts[0].unwrap_or(shown.name), parts[1].unwrap_or(shown.stroke), parts[2].unwrap_or(shown.tip)];
    if on.iter().any(|b| *b) { Ok(()) } else { Err("brushPickerName, brushPickerStroke and brushPickerTip: at least one must stay on".into()) }
}

/// The picker footer slider: the preset cards' size scale (1 standard; 0.30 and
/// below the tips drop their size numbers).
fn picker_scale_param(p: &Value) -> std::result::Result<Option<f32>, String> {
    match num_field(p, "brushPickerScale")? {
        Some(s) if !(0.15..=2.0).contains(&s) => Err("brushPickerScale must be between 0.15 and 2".into()),
        Some(s) => Ok(Some(s as f32)),
        None => Ok(None),
    }
}

/// `Some(None)` is an explicit null, which closes the picker (a missing field leaves it alone).
fn picker_param(p: &Value) -> std::result::Result<Option<Option<[f32; 2]>>, String> {
    let Some(v) = p.get("brushPicker") else { return Ok(None) };
    let at =
        serde_json::from_value::<Option<[f32; 2]>>(v.clone()).map_err(|_| "brushPicker must be [x, y] in screen points, or null to close it".to_string())?;
    if at.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
        return Err("brushPicker must be [x, y] in screen points, or null to close it".into());
    }
    Ok(Some(at))
}

/// brushSize rides the journaled `tools.setBrush` command (#744). Numeric values
/// are validated here so the command cannot fail after other fields were
/// applied; non-numeric values keep the API's historical silent no-op.
fn brush_size_param(p: &Value) -> std::result::Result<Option<f64>, String> {
    let Some(size) = p.get("brushSize").and_then(Value::as_f64) else { return Ok(None) };
    if !size.is_finite() || size > f64::from(photocraft_paint::MAX_BRUSH_SIZE) {
        return Err(format!("brushSize must be finite and at most {} px", photocraft_paint::MAX_BRUSH_SIZE));
    }
    Ok(Some(size))
}

// ---- dialogs ----

/// Every `ui.dialog.*` method: `control::dialogs::method` resolves the hosted dialog by its id or `"top"`. The
/// request is automation input, so a Save on a hosted prompt is judged by the automation policy.
fn dialog_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    if !req.method.starts_with("ui.dialog.") {
        return None;
    }
    let previous = std::mem::replace(&mut app.automation_input, true);
    let reply = super::dialogs::method(app, ctx, req);
    app.automation_input = previous;
    reply
}

// ---- synthetic input: pointer events, clicks, moves, wheel, keys, text, Caps Lock ----

/// `ui.pointer` queues its events for the canvas's tool host; the window methods go to craft-automation's
/// synthetic input, which the window host injects pass by pass. Their document points map through the main
/// canvas's view.
fn input_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    if req.method == "ui.pointer" {
        return Some(pointer(app, ctx, &req.params));
    }
    let view = crate::canvas::document_display::active_view(app);
    let map = move |_page: usize, at: egui::Pos2| view.as_ref().map(|view| view.to_screen(Point::new(f64::from(at.x), f64::from(at.y))));
    let host = WindowHost::of(ctx);
    let queued = craft::automation::synthetic::input_method(&req.method, &req.params, &host, egui::ViewportId::ROOT, &map)?;
    Some(match queued {
        Ok(()) => {
            app.input_queued_for = Some(app.input_target());
            Reply::AfterInput
        }
        Err(e) => err(e),
    })
}

/// What one `ui.pointer` event does.
#[derive(Clone, Copy, PartialEq)]
enum PointerKind {
    Down,
    Move,
    Up,
}

/// One validated `ui.pointer` event: document point, pressure, and the simulated pen's tilt and rotation in degrees.
struct PointerStep {
    kind: PointerKind,
    x: f64,
    y: f64,
    pressure: f32,
    tilt: [f32; 2],
    rotation: f32,
}

struct ParsedPointer {
    tool: Option<Tool>,
    button: String,
    modifiers: Value,
    events: Vec<PointerStep>,
}

/// Validate every event, then queue them in order for the tool host of the canvas they drive (an open full-window
/// workspace's, else the active document's main canvas); the canvas takes them on
/// its next pass and the reply does not wait for that. The tool host families own every interception (dialog
/// pickers, Color Range, the preview locator, context menus, Move transform controls). The host takes the
/// whole queue on its next pass; a held Space is queued with the events (`hold`, then `let_go`), so it is down for
/// exactly the events between them.
fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Reply {
    // Every field and event is validated before the first event is queued, so a rejected call
    // changes nothing (#2449).
    let parsed = match parse_pointer(p) {
        Ok(parsed) => parsed,
        Err(e) => return err(e),
    };
    // An open full-window workspace (Liquify, Filter Gallery, Vanishing Point) takes the events on its own canvas, in
    // that canvas's view, as the editor that is open; else the active document's main canvas takes them.
    let canvas = app
        .framework
        .dialogs
        .canvas()
        .or_else(|| app.session.active_index().map(|idx| crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id()));
    let Some((canvas, view)) = canvas.and_then(|canvas| ToolHost::shown_space(ctx, canvas).map(|space| (canvas, space.view))) else {
        return err("no canvas to drive");
    };
    if let Some(t) = parsed.tool {
        crate::tool_memory::select(app, t);
        // The tool's own brush now, not at the next frame, so a paint command right after uses
        // it (the Pencil's 1 px tip, #12200; any tool's remembered brush).
        crate::paint_mouse::sync_tool_brush(app);
    }
    let button = match parsed.button.as_str() {
        "secondary" | "right" => PointerButton::Secondary,
        "middle" => PointerButton::Middle,
        _ => PointerButton::Primary,
    };
    // A document point far enough out lands on the screen at a point that is not finite: refused here, before the
    // first event is queued, as the framework's own requests refuse one.
    if let Some(i) = parsed.events.iter().position(|step| {
        let at = view.to_screen(Point::new(step.x, step.y));
        !(at.x.is_finite() && at.y.is_finite())
    }) {
        return err(format!("events[{i}] is too far off the canvas: the point must be finite"));
    }
    let mods = tool_mods(modifiers_from(&parsed.modifiers));
    // Space held: the Crop tool moves the frame being drawn, a marquee, lasso or shape being drawn moves
    // instead of growing. Like a modifier flag, it may be top-level or grouped under "modifiers"; it is a held key
    // for the length of the request, and the tool host's momentary tools read it from the held keys.
    let space = p.get("modifiers").unwrap_or(p).get("space").and_then(Value::as_bool).unwrap_or(false);
    if space {
        ToolHost::inject(ctx, canvas, ToolInput::hold(Key::Space));
    }
    for step in &parsed.events {
        ToolHost::inject(ctx, canvas, tool_input(&view, step, button, mods));
    }
    if space {
        ToolHost::inject(ctx, canvas, ToolInput::let_go(Key::Space));
    }
    ctx.request_repaint();
    ok(json!({"status": app.ui.status}))
}

/// The modifiers the tool host reads, from the request's egui modifiers (command and Control apart on the Mac only).
fn tool_mods(m: egui::Modifiers) -> Mods {
    Mods { cmd: m.command, ctrl: cfg!(target_os = "macos") && m.ctrl, alt: m.alt, shift: m.shift }
}

/// The tool host's input for one step: the document point on the canvas's screen, with the pen's reading.
fn tool_input(view: &CanvasView, step: &PointerStep, button: PointerButton, mods: Mods) -> ToolInput {
    let at = view.to_screen(Point::new(step.x, step.y));
    let mut sample = PointerSample::new(step.pressure);
    sample.tilt = step.tilt.map(f32::to_radians);
    sample.rotation = step.rotation.to_radians();
    match step.kind {
        PointerKind::Down => ToolInput::down(button, at).mods(mods).sample(sample),
        PointerKind::Move => ToolInput::moved(at).mods(mods).sample(sample),
        PointerKind::Up => ToolInput::up(at).mods(mods),
    }
}

fn parse_pointer(p: &Value) -> std::result::Result<ParsedPointer, String> {
    let fields = ["events", "tool", "modifiers", "button", "shift", "alt", "command", "ctrl", "space"];
    let p = input_object(p, None, &fields)?;
    let tool = match input_choice(&p, "tool", "tool", &[])? {
        Some(t) => Some(Tool::from_name(t).ok_or_else(|| format!("unknown tool `{t}`"))?),
        None => None,
    };
    let button = input_choice(&p, "button", "button", &BUTTONS)?.unwrap_or("left").to_string();
    // Modifier flags may be top-level or grouped under "modifiers".
    let modifiers = input_modifiers(&p, &POINTER_MODIFIERS)?;
    let Some(events) = p.get("events") else { return Err("missing `events`".into()) };
    let Some(events) = events.as_array() else { return Err("events must be an array".into()) };
    let events = events.iter().enumerate().map(|(i, e)| parse_pointer_event(i, e)).collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ParsedPointer { tool, button, modifiers, events })
}

fn parse_pointer_event(i: usize, e: &Value) -> std::result::Result<PointerStep, String> {
    let name = format!("events[{i}]");
    let e = input_object(e, Some(&name), &["kind", "x", "y", "pressure", "tiltX", "tiltY", "rotation"])?;
    let num = |k: &str, range| input_number(&e, k, &format!("{name}.{k}"), range);
    let (x, y) = (num("x", None)?.unwrap_or(0.0), num("y", None)?.unwrap_or(0.0));
    let pressure = num("pressure", Some((0.0, 1.0)))?.unwrap_or(1.0) as f32;
    let kind = match input_choice(&e, "kind", &format!("{name}.kind"), &["down", "move", "up"])?.unwrap_or("move") {
        "down" => PointerKind::Down,
        "up" => PointerKind::Up,
        _ => PointerKind::Move,
    };
    let degrees = |k: &str, range| num(k, Some(range)).map(|v| v.map_or(0.0, |v| v as f32));
    let tilt = [degrees("tiltX", (-90.0, 90.0))?, degrees("tiltY", (-90.0, 90.0))?];
    let rotation = degrees("rotation", (0.0, 360.0))?;
    Ok(PointerStep { kind, x, y, pressure, tilt, rotation })
}

// ---- the application: open, save, quit ----

/// The application-level control methods: open, save and quit.
fn app_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    let p = &req.params;
    Some(match req.method.as_str() {
        "app.open" => match str_param(p, "path") {
            Some(path) => match p.get("approveHighMemory") {
                None | Some(Value::Null) => app_open(app, path, false),
                Some(Value::Bool(approved)) => app_open(app, path, *approved),
                Some(_) => err("expected boolean for `approveHighMemory`"),
            },
            None => err("missing `path`"),
        },
        "app.save" => match crate::FormatChoices::from_params(p, "app.save") {
            Ok(choices) => {
                let path = str_param(p, "path").map(str::to_string);
                run_waiting(app, wait_param(p), move |app| {
                    // Saving bypasses `menus::invoke`: a Type box drag in progress applies its box first.
                    crate::type_box_preview::end_drag(app);
                    app.save_automation_with(path, choices)
                })
            }
            Err(e) => err(e),
        },
        "app.quit" => {
            app.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Value::Null)
        }
        _ => return None,
    })
}

/// The `app.open` control method: opens a path through the configured automation read
/// capability, with optional high-memory approval.
fn app_open(app: &mut PhotocraftApp, path: &str, approved: bool) -> Reply {
    if let Some(outcome) = open_pcraft_directory(app, path) {
        return outcome;
    }
    let opened = match app.services.automation_read.as_mut() {
        Some(read) => read(path),
        None => Err("automation read authority is not configured".into()),
    };
    Reply::from_result(opened.and_then(|crate::AutomationReadResult { name, bytes, save_path }| {
        let name = app.open_name(&name);
        let warnings = app.open_automation_bytes_with_approval(&name, &bytes, approved)?;
        // Brushes/gradients go to the preset libraries: no document, no Open Recent entry.
        if !crate::preset_files_ui::is_preset_file(&name) {
            app.opened_from(path);
            if let Some(state) = app.session.active_mut()
                && state.path.is_some()
            {
                state.path = save_path;
            }
        }
        Ok(json!({"path": path, "name": name, "warnings": warnings}))
    }))
}

/// Directory .pcraft (the native on-disk form) loads through a capability-scoped
/// source; the single-file (zip) form falls back to the bytes path, as do all
/// other formats. `None` means "not a directory .pcraft".
fn open_pcraft_directory(app: &mut PhotocraftApp, path: &str) -> Option<Reply> {
    if !path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
        return None;
    }
    let open = app.services.automation_open_pcraft.as_mut()?;
    match open(path) {
        Ok(Some((name, doc))) => {
            app.session.add_document(doc, None);
            // Recent files, but no save-back target: the capability writer
            // replaces files, never directories.
            app.push_recent(path);
            app.sync_views();
            app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", &name)]);
            app.ui.status_error = false;
            Some(ok(json!({"path": path, "name": name, "warnings": []})))
        }
        Ok(None) => None, // a regular file: the bytes path reads the zip form
        Err(e) => Some(err(e)),
    }
}
