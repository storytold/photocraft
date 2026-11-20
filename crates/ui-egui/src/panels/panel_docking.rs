//! PhotoCraft's workspace on the framework dock: the `DockArea` pass over the panel registry, the one path that
//! changes the workspace ([`apply`]), the panel queries the menus and tests read, the control protocol's
//! `window.panel.*` ids, and the saved layout's load, snapshot and one-time migration of pre-workspace saves.
//!
//! `app.ui.docking` is the live [`Workspace`]; the registry (`panels::registry`) names every panel it may hold.
//! The lock is `app.workspaces.set.locked`.

use craft::behavior::dock::{Action, DockEdge, DockPlacement, Layout, Node, PanelPlace, PanelState, Permissions, SplitSize, Target};
use craft::behavior::{DockSide, EditEvent, EditEvents, IconId};
use craft::ui::contract::{Bind, BodyCx, Lazy, SizeLimits};
use craft::ui::docking::{
    DockCommands, DockConfig, DockEvent, DockPanels, DockScene, DockSizes, DockTo, DragUnit, FloatSpace, GroupAt, HidePanels, PanelSpec, actions, move_actions,
    request_actions,
};
use craft::ui::menu::PanelRow;
use craft::ui::patterns::dock::{DockArea, DockAreaOutput};
use craft::ui::primitives::IconSpec;
use craft::ui::rails::ToolRail;
use serde_json::{Map, Value, json};

use super::registry::{self, CANVAS, CANVAS_MIN, LEGACY_GROUPS, TOOLBAR};
use crate::PhotocraftApp;

/// The saved arrangement: layout, hidden panels with their return places, and the floated panels' docked places.
pub(crate) type Workspace = craft::behavior::dock::Workspace<String>;

/// Where `window.panel.float` puts a panel when the request gives no place (`docs/control-protocol.md:773`).
const FLOAT_AT: [f32; 2] = [400.0, 140.0];

/// The size `window.panel.float` gives a panel that has no float size of its own (`docs/control-protocol.md:774`).
const FLOAT_SIZE: [f32; 2] = [300.0, 400.0];

/// What the workspace allows `panel`, as the dock and every command apply it.
fn permissions(panel: &str) -> Permissions {
    registry::permissions(panel)
}

/// Whether `panel` is in the workspace, docked or floating.
pub(crate) fn is_shown(app: &PhotocraftApp, id: &str) -> bool {
    app.ui.docking.layout.contains(&id.to_owned())
}

/// Whether `panel`'s Window menu row is checked: it is in the layout, or, in a column collapsed to icons, its flyout is
/// open.
pub(crate) fn is_checked(app: &PhotocraftApp, id: &str) -> bool {
    let panel = id.to_owned();
    if app.ui.docking.layout.opens_in_flyout(&panel) { app.ui.flyout.as_ref() == Some(&panel) } else { is_shown(app, id) }
}

/// Whether `panel` shows: the active tab of its group in an expanded column, neither the group minimized nor its column
/// collapsed, or the panel open in a collapsed column's flyout.
pub(crate) fn is_front(app: &PhotocraftApp, id: &str) -> bool {
    let panel = id.to_owned();
    let layout = &app.ui.docking.layout;
    layout.panel_state(&panel) == PanelState::Front || (layout.opens_in_flyout(&panel) && app.ui.flyout.as_ref() == Some(&panel))
}

/// Shows or hides panel `id` (an id or a label). `Some(true)` opens a hidden panel at its place, and brings a shown
/// one to the front, expanding its minimized group; `Some(false)` closes it; `None` shows a panel that is hidden,
/// minimized or behind another tab and closes one that is in front. A panel in a column collapsed to icons opens and
/// closes its flyout instead, and stays in the layout. Returns whether the panel shows.
pub(crate) fn show_panel(app: &mut PhotocraftApp, id: &str, show: Option<bool>) -> Result<bool, String> {
    let panel = registry::normalize(id).ok_or_else(|| format!("`{id}` is not a registered panel"))?;
    let owned = panel.to_owned();
    let show = show.unwrap_or_else(|| !is_front(app, panel));
    if app.ui.docking.layout.opens_in_flyout(&owned) {
        app.ui.flyout = show.then_some(owned);
        return Ok(show);
    }
    if !show && !is_shown(app, panel) {
        return Ok(false);
    }
    let action = if show { Action::open(owned.clone(), None) } else { Action::close(owned.clone()) };
    apply(app, vec![action])?;
    // A panel that opened into a column of icons shows in its flyout.
    if show && app.ui.docking.layout.opens_in_flyout(&owned) {
        app.ui.flyout = Some(owned);
    }
    Ok(show)
}

/// Applies `actions` to the workspace in order, all or none, under the lock: every change of the workspace from the
/// menus, the rail, the control protocol, the dock pass and the migration goes through here. A closed Timeline stops
/// playing, and the layout is remembered. Returns the panels the actions closed.
///
/// While the dock pass runs on the app's context, the workspace is on screen: actions that pass the checks go to the
/// pass, which hands them back to [`show`] to apply here when it ends, in the same frame.
pub(crate) fn apply(app: &mut PhotocraftApp, actions: Vec<Action<String>>) -> Result<Vec<String>, String> {
    apply_during(app, actions, false)
}

/// [`apply`], for the actions of a dock pass whose divider drag or window move is still `dragging`: the steps of a
/// drag change the workspace, and the layout is remembered once, when the drag commits ([`show`]).
fn apply_during(app: &mut PhotocraftApp, actions: Vec<Action<String>>, dragging: bool) -> Result<Vec<String>, String> {
    let (next, closed) = planned(app, &actions)?;
    if request_actions(app.ctx(), actions).is_ok() {
        return Ok(closed);
    }
    app.ui.docking = next;
    stop_closed_timeline(app, &closed);
    if !dragging {
        crate::dock::persist(app);
    }
    Ok(closed)
}

/// Whether a workspace gesture of the pass (a divider drag, a window move) is still running after it: its last
/// event is its begin or a step, not its commit or cancel. The area's own one-step edits belong to no gesture.
fn dragging(edit: &EditEvents<Workspace>) -> bool {
    edit.iter().rfind(|event| event.id().is_some()).is_some_and(|event| matches!(event, EditEvent::Begin { .. } | EditEvent::Update { .. }))
}

/// Whether [`apply`] would take `actions`: the same refusals, on a copy of the workspace. Changes nothing.
pub(crate) fn check(app: &PhotocraftApp, actions: &[Action<String>]) -> Result<(), String> {
    planned(app, actions).map(|_| ())
}

/// The workspace `actions` make of the live one, and the panels they close; the refusals of [`apply`] and [`check`].
fn planned(app: &PhotocraftApp, actions: &[Action<String>]) -> Result<(Workspace, Vec<String>), String> {
    let mut next = app.ui.docking.clone();
    let closed = next.apply_all(actions.iter().cloned(), app.workspaces.set.locked, |panel: &String| permissions(panel)).map_err(|error| error.to_string())?;
    // The model leaves the panel registry to the application: an action may not bring in an unknown panel.
    if let Some(unknown) = next.layout.panels().into_iter().find(|panel| !registry::registered(panel)) {
        return Err(format!("`{unknown}` is not a registered panel"));
    }
    Ok((next, closed))
}

/// The Timeline plays only while it is in the workspace.
fn stop_closed_timeline(app: &mut PhotocraftApp, closed: &[String]) {
    if closed.iter().any(|panel| panel == "timeline") {
        super::timeline_ui::stop(app);
    }
}

/// What the dock reads of `panel`: its name, icon, least size, group heights, float size and, for the toolbar, the grid
/// its column is whole steps of and its float in the main window.
fn spec(app: &PhotocraftApp, panel: &str) -> PanelSpec {
    if panel == CANVAS {
        let least = SizeLimits::at_least(egui::vec2(CANVAS_MIN[0], CANVAS_MIN[1]));
        return PanelSpec::new(tl!("Documents").to_string()).canvas().limits(least).allow_native(false);
    }
    let Some(entry) = registry::entry(panel) else { return PanelSpec::new(panel.to_owned()) };
    let mut spec =
        PanelSpec::new((entry.label)()).icon(IconSpec::svg(IconId::new(entry.icon))).limits(SizeLimits::at_least(egui::vec2(entry.min[0], entry.min[1])));
    if let Some(heights) = entry.heights {
        spec = spec.heights(heights);
    }
    if let Some([width, height]) = entry.float_size {
        spec = spec.float_size(egui::vec2(width, height));
    }
    if panel == TOOLBAR {
        // The toolbar floats in the main window, as production's: its tooltips, tool flyouts and Edit Toolbar drawer
        // open beside the one-button-wide rail, which its own OS window would clip.
        spec = spec.grid(ToolRail::<crate::state::Tool>::dock_grid(app.ctx(), super::toolbar::cells(app))).allow_native(false);
    }
    spec
}

/// The registry's panels as the dock asks for them.
struct Panels<'a> {
    app: &'a mut PhotocraftApp,
}

impl DockPanels<String> for Panels<'_> {
    fn spec(&self, panel: &String) -> PanelSpec {
        spec(self.app, panel)
    }

    fn permissions(&self, panel: &String) -> Permissions {
        permissions(panel)
    }

    fn body(&mut self, panel: &String, ui: &mut egui::Ui, _: &mut BodyCx<'_>) {
        body(self.app, ui, panel);
    }

    /// Each group's header and tab strip and each column's bar settle as the panel bodies do.
    fn spec_version(&self) -> Option<u64> {
        Some(super::region_versions::dock_chrome(self.app))
    }

    fn content(&self, panel: &String) -> Option<u64> {
        match panel.as_str() {
            // The canvas shows state that changes without a command: the view, redraw progress, animation and job overlays.
            CANVAS => None,
            TOOLBAR => Some(super::region_versions::toolbar(self.app)),
            _ => Some(super::region_versions::panel(self.app, panel)),
        }
    }

    /// The canvas group is the document area: it draws its own tabs and documents in the group's rect. The toolbar's
    /// group is its rail, which takes the group's full rect and measures its height at a width.
    fn group_view(&mut self, panels: &[String], _: usize, ui: &mut egui::Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if panels.iter().any(|panel| panel == TOOLBAR) {
            return Some(match at {
                GroupAt::Measure(width) => super::toolbar::measure(self.app, ui, width),
                GroupAt::Show(rect) => {
                    // The rail settles as a panel body does: while nothing it shows changed and no input reaches it,
                    // the frame repaints its last run instead of running every tool button again (#15493).
                    let version = super::region_versions::toolbar_drawn(self.app, ui);
                    let app = &mut *self.app;
                    Lazy::new("toolbar", 0.0).settles(true).content(version).show(ui, |ui| super::toolbar::toolbar(app, ui));
                    rect.height()
                }
            });
        }
        if !panels.iter().any(|panel| panel == CANVAS) {
            return None;
        }
        match at {
            // Protected, the canvas never sits in a flyout, which is what measures.
            GroupAt::Measure(_) => Some(CANVAS_MIN[1]),
            GroupAt::Show(rect) => {
                crate::canvas::document_area(self.app, ui);
                Some(rect.height())
            }
        }
    }

    fn panel_rows(&mut self, panel: &String) -> Vec<PanelRow> {
        registry::entry(panel).map(|entry| (entry.rows)(self.app)).unwrap_or_default()
    }
}

/// Draws panel `id`'s body in `ui`, the group's body room. Each body fills that room and scrolls itself.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: &str) {
    let Some(entry) = registry::entry(id) else { return };
    (entry.body)(app, ui);
}

/// Draws the workspace, the canvas included, in the room `ui` has left. With `panels_hidden` (Shift+Tab: the panels
/// hide and the tools stay) only the toolbar stays.
///
/// The area shows the live workspace and writes none of it. Every change of the pass reaches [`apply`] once, after
/// the pass: the area's own (tab picks, drops, menu rows, divider drags, window moves), then what the bodies asked
/// for while it ran (`show_panel` and the other [`apply`] callers). Each pass applies only its own input: the area
/// reads the pass's pointer and the files dropped on it, and a second pass in the same frame gets neither again, so
/// nothing applies twice. A divider drag or window move changes the workspace at each step and remembers the layout
/// once, when it commits: the area's `edit` says whether its gesture is still running.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, panels_hidden: bool) {
    let ctx = ui.ctx().clone();
    let workspace = app.ui.docking.clone();
    let locked = app.workspaces.set.locked;
    let auto_collapse = app.session.prefs().workspace.auto_collapse_icon_panels;
    // A flyout stays open only while its panel is in a column collapsed to icons.
    if app.ui.flyout.as_ref().is_some_and(|panel| !app.ui.docking.layout.opens_in_flyout(panel)) {
        app.ui.flyout = None;
    }
    let flyout = app.ui.flyout.clone();
    let keeps_toolbar = |panel: &String| panel == TOOLBAR;
    let config = DockConfig::default().group_order_items(true).native_windows(cfg!(not(target_arch = "wasm32")));
    let output: DockAreaOutput<String> = {
        let mut panels = Panels { app: &mut *app };
        let mut area = DockArea::new(tl!("Panels").to_string(), Bind::given(workspace), &mut panels, config);
        if locked {
            area = area.locked();
        }
        if auto_collapse {
            area = area.auto_collapse();
        }
        if panels_hidden {
            area = area.hide(HidePanels::Except(&keeps_toolbar));
        }
        area.flyout(Bind::given(flyout)).show(ui)
    };
    #[cfg(test)]
    probe::record_windows(&ctx, &app.ui.docking.layout, &output.windows);
    let DockAreaOutput { actions, requested, events, error, edit, window_files, .. } = output;
    if let Some(error) = error {
        app.ui.status = format!("{}: {}", tl!("Invalid panel layout"), error.error);
    }
    let dragging = dragging(&edit);
    let changes: Vec<Action<String>> = actions.into_iter().chain(requested).collect();
    let applied = !changes.is_empty()
        && match apply_during(app, changes, dragging) {
            Ok(_) => true,
            Err(error) => {
                app.ui.status = error;
                false
            }
        };
    // A drag that ends with no change of its own in this pass still saves the steps it held.
    if !applied && !dragging && edit.ended().is_some() {
        crate::dock::persist(app);
    }
    for event in events {
        match event {
            DockEvent::ToggleAutoCollapse => {
                app.session.prefs.edit(|prefs| prefs.workspace.auto_collapse_icon_panels = !prefs.workspace.auto_collapse_icon_panels);
            }
            DockEvent::FlyoutChanged { panel } => app.ui.flyout = panel,
            _ => {}
        }
    }
    // Files dropped on a floating panel's own window open as documents, and the main window comes forward.
    if !window_files.is_empty() {
        app.open_dropped(&ctx, window_files, crate::file_open::DropTo::Window);
    }
}

/// The actions that give the end dock the total width `total`, its columns scaled within their bounds. Empty when
/// there is no end dock, the width is not a positive number or the columns already have it.
pub(crate) fn width_actions(app: &PhotocraftApp, ctx: &egui::Context, total: f32) -> Vec<Action<String>> {
    if !total.is_finite() || total <= 0.0 {
        return Vec::new();
    }
    DockSizes::of(ctx).resize_side(&app.ui.docking.layout, DockEdge::End, &|panel: &String| spec(app, panel), total)
}

// ---------------------------------------------------------------------------------------------------------------
// The control protocol's panel ids (`docs/control-protocol.md:770-800`).
// ---------------------------------------------------------------------------------------------------------------

/// Runs a `window.panel.*`, `window.toggle.<panel>`, `type.panels.*` or `ui.dock` command; `None` for an id that is
/// not one of them. `window.panel.modifierKeys` is the Window menu's (its params latch the modifier keys), and the
/// `window.workspace.*` ids run where the menu runs them.
pub(crate) fn command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let result = match id {
        "window.panel.modifierKeys" => return None,
        "ui.dock" => ui_dock(app, params),
        "window.panel" | "window.panel.activate" => fields(params).and_then(|fields| {
            let panel = required_panel(&fields)?;
            toggled(app, &panel, Some(true))
        }),
        "window.panel.group" => group_command(app, params),
        "window.panel.layout" => match params.get("action") {
            Some(action) => {
                serde_json::from_value::<Action<String>>(action.clone()).map_err(|error| error.to_string()).and_then(|action| run(app, vec![action]))
            }
            None => Err("action is required".into()),
        },
        "window.panel.move" => move_command(app, params),
        "window.panel.float" => float_command(app, params),
        "window.panel.dock" => dock_command(app, params),
        "window.panel.close" => one_panel(app, params, Action::close),
        "window.panel.closeGroup" => one_panel(app, params, Action::close_group),
        "window.panel.closeOthers" => one_panel(app, params, Action::close_others),
        "window.panel.minimize" => minimize_command(app, params),
        "window.panel.iconic" => iconic_command(app, params),
        "window.panel.window" => window_command(app, params),
        // Opening the Character or Paragraph panel from the type tool never hides it (`view_cmds/dispatch.rs:303`).
        "type.panels.character" => toggled(app, "character", Some(true)),
        "type.panels.paragraph" => toggled(app, "paragraph", Some(true)),
        _ => {
            let panel = toggle_panel(id)?;
            show_param(params).and_then(|show| toggled(app, panel, show))
        }
    };
    Some(result)
}

/// The panel a toggle id names: `window.panel.<panel>`, `window.toggle.<panel>`, or `type.panels.<panel>` for the type
/// panels that have no command of their own.
pub(crate) fn toggle_panel(id: &str) -> Option<&'static str> {
    let name = id
        .strip_prefix("window.panel.")
        .or_else(|| id.strip_prefix("window.toggle."))
        .or_else(|| id.strip_prefix("type.panels.").filter(|name| matches!(*name, "characterStyles" | "paragraphStyles" | "glyphs")))?;
    registry::normalize(name)
}

/// The `show` (or `on`) param of a panel toggle: `None` toggles.
fn show_param(params: &Value) -> Result<Option<bool>, String> {
    match params.get("show").or_else(|| params.get("on")) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| "show must be a boolean".into()),
    }
}

/// [`show_panel`] with the protocol's answer.
fn toggled(app: &mut PhotocraftApp, panel: &str, show: Option<bool>) -> Result<Value, String> {
    let shown = show_panel(app, panel, show)?;
    Ok(json!({"panel": panel, "shown": shown, "layout": app.ui.docking.layout}))
}

/// Runs `actions` through [`apply`] with the protocol's answer.
fn run(app: &mut PhotocraftApp, actions: Vec<Action<String>>) -> Result<Value, String> {
    let closed = apply(app, actions)?;
    Ok(json!({"layout": app.ui.docking.layout, "closed": closed}))
}

/// The params as an object, panels named by id or label and `onto` standing for `anchor`. The canvas may be an
/// anchor or a target, since docks and frames open beside it.
fn fields(params: &Value) -> Result<Map<String, Value>, String> {
    let mut fields = match params {
        Value::Null => Map::new(),
        Value::Object(fields) => fields.clone(),
        _ => return Err("params must be an object".into()),
    };
    if let Some(onto) = fields.remove("onto") {
        fields.entry("anchor").or_insert(onto);
    }
    for key in ["panel", "anchor", "before", "target"] {
        match fields.get(key) {
            None | Some(Value::Null) => {}
            Some(value) => {
                // Docks and frames open beside the canvas, so it may be an anchor or a target.
                let canvas = key != "panel" && value.as_str().is_some_and(|raw| raw.trim().eq_ignore_ascii_case(CANVAS));
                let panel = if canvas { Some(CANVAS) } else { value.as_str().and_then(registry::normalize) };
                let panel = panel.ok_or_else(|| format!("{key} must name a registered panel"))?;
                fields.insert(key.into(), json!(panel));
            }
        }
    }
    Ok(fields)
}

/// The panel a normalized param names.
fn named(fields: &Map<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn required_panel(fields: &Map<String, Value>) -> Result<String, String> {
    named(fields, "panel").ok_or_else(|| "panel must name a registered panel".into())
}

fn flag(fields: &Map<String, Value>, key: &str) -> bool {
    fields.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// What a move or float takes along: the panel, its group (`group: true`) or its dock column (`column: true`).
fn unit(fields: &Map<String, Value>, panel: String) -> DragUnit<String> {
    if flag(fields, "column") {
        DragUnit::Column(panel)
    } else if flag(fields, "group") {
        DragUnit::Group(panel)
    } else {
        DragUnit::Panel(panel)
    }
}

/// Where a `zone` param puts a unit beside `anchor`: `tab` (the default) joins its tabs before `before`; `center`
/// joins them at the end; `left`, `right`, `top`/`above` and `bottom`/`below` go beside its group.
fn destination(fields: &Map<String, Value>, anchor: String) -> Result<DockTo<String>, String> {
    let zone = match fields.get("zone") {
        None | Some(Value::Null) => "tab",
        Some(zone) => zone.as_str().ok_or("zone must be tab, center, left, right, top, bottom, above or below")?,
    };
    let rtl = crate::direction::is_rtl();
    let side = match zone {
        "tab" => return Ok(DockTo::tabs(anchor, named(fields, "before"))),
        "center" => DockSide::Center,
        "left" if rtl => DockSide::End,
        "left" => DockSide::Start,
        "right" if rtl => DockSide::Start,
        "right" => DockSide::End,
        "top" | "above" => DockSide::Top,
        "bottom" | "below" => DockSide::Bottom,
        _ => return Err("zone must be tab, center, left, right, top, bottom, above or below".into()),
    };
    Ok(DockTo::beside(anchor, side))
}

/// Runs `f` with the dock scene of the live workspace, for the framework's panel commands and moves.
fn with_scene<R>(app: &PhotocraftApp, f: impl FnOnce(&DockScene<'_, String>) -> R) -> R {
    // The moves and Dock Panel built here never read the space's bounds: `window.panel.float` gives its own rect.
    let sizes = DockSizes::of(app.ctx());
    let space = FloatSpace::new(egui::Rect::NOTHING, &sizes);
    let policy = |panel: &String| permissions(panel);
    let workspace = &app.ui.docking;
    let scene = DockScene::new(&workspace.layout, &space, app.workspaces.set.locked, &policy).floated(&workspace.floated);
    f(&scene)
}

/// `window.panel.move {panel, anchor, zone?, before?, group?, placement?}`.
fn move_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let fields = fields(params)?;
    let panel = required_panel(&fields)?;
    let anchor = named(&fields, "anchor").ok_or("anchor must name a registered panel")?;
    let actions = match fields.get("placement") {
        Some(placement) => {
            let placement: DockPlacement<String> = serde_json::from_value(placement.clone()).map_err(|error| error.to_string())?;
            vec![if flag(&fields, "group") { Action::move_group(panel, Target::at(anchor, placement)) } else { Action::move_beside(panel, anchor, placement) }]
        }
        None => {
            let to = destination(&fields, anchor)?;
            let unit = unit(&fields, panel);
            with_scene(app, |scene| move_actions(scene, &unit, to)).map_err(|error| error.to_string())?
        }
    };
    run(app, actions)
}

/// The window rect of a float: the `x`, `y`, `width` and `height` params, or (400, 140) and the panel's float size,
/// 300 × 400 without one.
fn float_rect(fields: &Map<String, Value>, panel: &str) -> Result<egui::Rect, String> {
    let size = registry::entry(panel).and_then(|entry| entry.float_size).unwrap_or(FLOAT_SIZE);
    let number = |key: &str, default: f32| match fields.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value.as_f64().map(|v| v as f32).ok_or_else(|| format!("{key} must be a number")),
    };
    let (x, y) = (number("x", FLOAT_AT[0])?, number("y", FLOAT_AT[1])?);
    let (width, height) = (number("width", size[0])?, number("height", size[1])?);
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width <= 0.0 || height <= 0.0 {
        return Err("float geometry must be finite, with a positive width and height".into());
    }
    Ok(egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height)))
}

/// `window.panel.float {panel, x?, y?, width?, height?, group?, column?}`.
fn float_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let fields = fields(params)?;
    let panel = required_panel(&fields)?;
    let to = Target::float(float_rect(&fields, &panel)?);
    let action = if flag(&fields, "column") {
        Action::move_column(panel, to)
    } else if flag(&fields, "group") {
        Action::move_group(panel, to)
    } else {
        Action::move_panel(panel, to)
    };
    run(app, vec![action])
}

/// `window.panel.dock {panel, anchor?, zone?, before?, group?, side?}`: without an anchor, a floating panel goes back
/// where it floated from when that place is still docked (Dock Panel), or into the dock on `side`; with one, beside
/// or into the anchor's group.
fn dock_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let fields = fields(params)?;
    let panel = required_panel(&fields)?;
    let actions = match named(&fields, "anchor") {
        Some(anchor) => {
            let to = destination(&fields, anchor)?;
            let unit = unit(&fields, panel);
            with_scene(app, |scene| move_actions(scene, &unit, to))
        }
        None => {
            let side = match fields.get("side").and_then(Value::as_str) {
                None => None,
                Some("start") => Some(DockEdge::Start),
                Some("end") => Some(DockEdge::End),
                Some(_) => return Err("side must be start or end".into()),
            };
            match side {
                Some(edge) => {
                    let unit = unit(&fields, panel);
                    with_scene(app, |scene| move_actions(scene, &unit, DockTo::Dock(edge)))
                }
                None if flag(&fields, "group") => with_scene(app, |scene| move_actions(scene, &DragUnit::Group(panel), DockTo::Docked)),
                None => with_scene(app, |scene| actions(scene, DockCommands::DockPanel, &panel)),
            }
        }
    }
    .map_err(|error| error.to_string())?;
    run(app, actions)
}

/// A command on one panel: `{panel}`.
fn one_panel(app: &mut PhotocraftApp, params: &Value, action: fn(String) -> Action<String>) -> Result<Value, String> {
    let panel = required_panel(&fields(params)?)?;
    run(app, vec![action(panel)])
}

/// Whether the group holding `panel` is minimized.
fn minimized(layout: &Layout<String>, panel: &String) -> bool {
    layout.place(panel).is_some_and(|place| layout.group(&place).is_some_and(|group| group.minimized))
}

/// `window.panel.minimize {panel, minimized?}`: minimizes the panel's group to its tab strip or restores it; without
/// `minimized`, toggles.
fn minimize_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let fields = fields(params)?;
    let panel = required_panel(&fields)?;
    let on = match fields.get("minimized") {
        None | Some(Value::Null) => !minimized(&app.ui.docking.layout, &panel),
        Some(value) => value.as_bool().ok_or("minimized must be a boolean")?,
    };
    run(app, vec![Action::set_minimized(panel, on)])
}

/// `window.panel.iconic {panel, iconic?}`: collapses the column holding the panel, docked or floating, to icons, or
/// expands it; without `iconic`, toggles.
fn iconic_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let fields = fields(params)?;
    let panel = required_panel(&fields)?;
    let layout = &app.ui.docking.layout;
    let current = layout.place(&panel).and_then(|place| layout.column(&place)).map(|column| column.iconic).ok_or("panel is hidden")?;
    let on = match fields.get("iconic") {
        None | Some(Value::Null) => !current,
        Some(value) => value.as_bool().ok_or("iconic must be a boolean")?,
    };
    run(app, vec![Action::set_iconic(panel, on)])
}

/// `window.panel.window {panel, native?}`: floats a docked panel's group, whose window the dock gives an OS window of
/// its own on the desktop; `native: false` keeps a floating window in the main window.
fn window_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if cfg!(target_arch = "wasm32") {
        return Err("floating panels stay in the page on the web".into());
    }
    let fields = fields(params)?;
    let panel = required_panel(&fields)?;
    let native = fields.get("native").and_then(Value::as_bool).unwrap_or(true);
    let floating = matches!(app.ui.docking.layout.place(&panel), Some(PanelPlace::Floating { .. }));
    let actions = match (native, floating) {
        (true, true) => Vec::new(),
        (true, false) => vec![Action::move_group(panel.clone(), Target::float(float_rect(&fields, &panel)?))],
        (false, _) => vec![Action::set_native(panel, None)],
    };
    run(app, actions)
}

/// `ui.dock {action}` or `ui.dock {operation, …}` with the operations `open`, `close`, `activate`, `float`,
/// `moveFloating`, `move`, `setStackOpen`, `resizeStack` and `resizeSplit`.
fn ui_dock(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let fields = fields(params)?;
    if let Some(action) = fields.get("action") {
        let action = serde_json::from_value::<Action<String>>(action.clone()).map_err(|error| error.to_string())?;
        return run(app, vec![action]);
    }
    let operation = fields.get("operation").and_then(Value::as_str).ok_or("operation is required")?;
    let panel = || required_panel(&fields);
    let rect = || -> Result<egui::Rect, String> {
        match fields.get("rect").and_then(Value::as_array) {
            Some(rect) => {
                let at = |i: usize| rect.get(i).and_then(Value::as_f64).map(|v| v as f32).ok_or("rect must be [x, y, width, height]");
                let rect = egui::Rect::from_min_size(egui::pos2(at(0)?, at(1)?), egui::vec2(at(2)?, at(3)?));
                if !rect.is_finite() || !rect.is_positive() {
                    return Err("float geometry must be finite, with a positive width and height".into());
                }
                Ok(rect)
            }
            None => float_rect(&fields, &panel()?),
        }
    };
    let action = match operation {
        "open" => return toggled(app, &panel()?, Some(true)),
        "close" => Action::close(panel()?),
        "activate" => Action::activate(panel()?),
        "float" => Action::move_panel(panel()?, Target::float(rect()?)),
        "moveFloating" => Action::move_floating(panel()?, rect()?),
        "move" => {
            let anchor = named(&fields, "anchor").ok_or("anchor must name a registered panel")?;
            let placement = fields.get("placement").cloned().unwrap_or_else(|| json!({"Tab": {"before": null}}));
            let placement: DockPlacement<String> = serde_json::from_value(placement).map_err(|error| error.to_string())?;
            Action::move_beside(panel()?, anchor, placement)
        }
        "setStackOpen" => Action::set_stack_open(panel()?, fields.get("open").and_then(Value::as_bool).ok_or("open must be a boolean")?),
        "resizeStack" => Action::resize_stack(panel()?, fields.get("height").and_then(Value::as_f64).map(|height| height as f32)),
        "resizeSplit" => {
            let path: Vec<bool> = serde_json::from_value(fields.get("path").cloned().unwrap_or(json!([]))).map_err(|error| error.to_string())?;
            let size: SplitSize = serde_json::from_value(fields.get("size").cloned().ok_or("size is required")?).map_err(|error| error.to_string())?;
            Action::resize_split(path, size)
        }
        _ => return Err("operation must be open, close, activate, float, moveFloating, move, setStackOpen, resizeStack or resizeSplit".into()),
    };
    run(app, vec![action])
}

/// The panels of the group, tab node or accordion entry holding `panel`, in tab order; empty when it is hidden.
fn group_members(layout: &Layout<String>, panel: &String) -> Vec<String> {
    let Some(place) = layout.place(panel) else { return Vec::new() };
    if let Some(group) = layout.group(&place) {
        return group.panels.clone();
    }
    match layout.node_of(&place) {
        Some(Node::Tabs { panels, .. }) => panels.clone(),
        _ => vec![panel.clone()],
    }
}

/// `window.panel.group {panel, operation}`: commands for a panel's whole group.
fn group_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let panel = params.get("panel").and_then(Value::as_str).and_then(registry::normalize).ok_or("panel must name a registered panel")?.to_owned();
    let operation = params.get("operation").and_then(Value::as_str).ok_or("operation is required")?;
    if app.workspaces.set.locked {
        return Err("workspace is locked".into());
    }
    let layout = &app.ui.docking.layout;
    let members = group_members(layout, &panel);
    if members.is_empty() {
        return Err("panel is hidden".into());
    }
    let actions = match operation {
        "close" => vec![Action::close_group(panel.clone())],
        "collapse" => vec![Action::set_minimized(panel.clone(), true)],
        "expand" => vec![Action::set_minimized(panel.clone(), false)],
        "move" => {
            let fields = fields(params)?;
            let anchor = named(&fields, "anchor").ok_or("anchor must name a registered panel")?;
            if members.contains(&anchor) {
                return Err("group cannot be moved into itself".into());
            }
            let to = destination(&fields, anchor)?;
            let unit = DragUnit::Group(panel.clone());
            let mut moves = with_scene(app, |scene| move_actions(scene, &unit, to)).map_err(|error| error.to_string())?;
            moves.push(Action::activate(panel.clone()));
            moves
        }
        "up" | "down" => {
            let up = operation == "up";
            if !layout.can_step_group(&panel, up, |panel: &String| permissions(panel)) {
                return Err("group is already at the edge".into());
            }
            vec![Action::step_group(panel.clone(), up), Action::activate(panel.clone())]
        }
        _ => return Err("operation must be close, collapse, expand, move, up or down".into()),
    };
    apply(app, actions)?;
    Ok(json!({"layout": app.ui.docking.layout}))
}

// ---------------------------------------------------------------------------------------------------------------
// The saved layout.
// ---------------------------------------------------------------------------------------------------------------

/// A saved `docking` value from a newer version than this one reads.
fn newer(docking: &Value) -> bool {
    docking.get("version").and_then(Value::as_u64).is_some_and(|version| version > u64::from(Workspace::VERSION))
}

/// Loads the workspace of a saved layout (`prefs.panelLayout` or a saved workspace) into `app.ui.docking`.
///
/// A readable `docking` workspace is fitted to the registry. One from a newer version, or one that does not read,
/// shows the default; [`snapshot`] writes a newer one back unchanged until the arrangement changes. A save from
/// before the workspace (`panels`, `dockTabs`, `dock`, `timelineOpen`) migrates once onto the default.
pub(crate) fn load(app: &mut PhotocraftApp, saved: &Value) {
    app.ui.docking.replace(loaded(saved));
}

/// [`load`] at launch: Brush Settings is a transient window, so a remembered workspace that holds it loads with it
/// closed (its place kept for when it opens). A workspace switch loads it as saved.
pub(crate) fn load_at_launch(app: &mut PhotocraftApp, saved: &Value) {
    let mut workspace = loaded(saved);
    let transient = "brushSettings".to_owned();
    if workspace.layout.contains(&transient)
        && let Err(error) = workspace.apply(Action::close(transient), false, |panel: &String| permissions(panel))
    {
        app.ui.status = error.to_string();
    }
    app.ui.docking.replace(workspace);
}

/// The workspace of a saved layout, fitted to the registry. A save that predates the toolbar panel brings the toolbar's
/// old chrome flags over once ([`migrate_toolbar`]).
fn loaded(saved: &Value) -> Workspace {
    let default = registry::default_workspace();
    let toolbar = TOOLBAR.to_owned();
    let mut predates_toolbar = false;
    let mut workspace = match saved.get("docking") {
        Some(docking) if docking.is_object() => match serde_json::from_value::<Workspace>(docking.clone()) {
            Ok(workspace) if workspace.validate().is_ok() => {
                predates_toolbar = !workspace.layout.contains(&toolbar) && !workspace.hidden.iter().any(|(panel, _)| *panel == toolbar);
                workspace
            }
            _ => default.clone(),
        },
        _ => {
            predates_toolbar = true;
            migrated(saved, &default)
        }
    };
    workspace.reconcile(|panel: &String| registry::registered(panel), |panel: &String| permissions(panel), &default);
    if predates_toolbar {
        migrate_toolbar(&mut workspace, saved.get("panels"));
    }
    workspace
}

/// The first panel of the end dock's outermost column other than `skip`: the anchor a column moves before to take the
/// window's end edge.
fn end_edge_anchor(layout: &Layout<String>, skip: &String) -> Option<String> {
    let end = layout.dock_path(DockEdge::End)?;
    layout
        .panels()
        .into_iter()
        .filter(|panel| *panel != skip)
        .find(|panel| matches!(layout.place(panel), Some(PanelPlace::Dock { path, column: 0, .. }) if path == end))
        .cloned()
}

/// The toolbar of a save that predates the toolbar panel, from the chrome flags it saved in `panels`: `toolbar: false`
/// closes it, `toolbar_double: true` makes it two tool columns wide, and a `toolbar_side` that resolves to the right
/// of the window moves its column to the end dock's outer edge. The flags are read as plain values: no type of them is
/// left. A flag that does not read changes nothing.
fn migrate_toolbar(workspace: &mut Workspace, panels: Option<&Value>) {
    let Some(panels) = panels.and_then(Value::as_object) else { return };
    let toolbar = TOOLBAR.to_owned();
    let mut actions = Vec::new();
    if panels.get("toolbar").and_then(Value::as_bool) == Some(false) {
        actions.push(Action::close(toolbar.clone()));
    } else {
        if !workspace.layout.contains(&toolbar) {
            actions.push(Action::open(toolbar.clone(), None));
        }
        if panels.get("toolbar_double").and_then(Value::as_bool) == Some(true) {
            actions.push(Action::set_column_steps(toolbar.clone(), 2));
        }
        let rtl = crate::direction::is_rtl();
        let right = match panels.get("toolbar_side").and_then(Value::as_str) {
            Some("right") => true,
            Some("start") => rtl,
            Some("end") => !rtl,
            _ => false,
        };
        if right {
            let to = match end_edge_anchor(&workspace.layout, &toolbar) {
                Some(anchor) => Target::at(anchor, DockPlacement::column(false)),
                None => Target::new_dock(DockEdge::End),
            };
            actions.push(Action::move_column(toolbar.clone(), to));
        }
    }
    for action in actions {
        // A step the workspace refuses leaves the toolbar where the default put it.
        let _ = workspace.apply(action, false, |panel: &String| permissions(panel));
    }
}

/// A save from before the workspace on the default workspace: its group flags, selected tabs, collapsed groups and
/// closed tabs, and an open Timeline. The saved group order and heights do not carry over; anything that does not
/// apply leaves the default.
fn migrated(saved: &Value, default: &Workspace) -> Workspace {
    let mut dock = saved.get("dock").cloned();
    // Every group keeps the one tab order and its built-in height.
    if let Some(dock) = dock.as_mut().and_then(Value::as_object_mut) {
        dock.remove("order");
        dock.remove("heights");
    }
    // An old save lists a tab pick for every group, hidden ones too: a hidden group's pick has nothing to select.
    let panels = saved.get("panels");
    let mut tabs = saved.get("dockTabs").cloned();
    if let Some(picks) = tabs.as_mut().and_then(Value::as_object_mut) {
        picks.retain(|key, _| panels.and_then(|panels| panels.get(key)).and_then(Value::as_bool) != Some(false));
    }
    let Ok(mut actions) = legacy_on(&default.layout, panels, tabs.as_ref(), dock.as_ref()) else { return default.clone() };
    if saved.get("timelineOpen").and_then(Value::as_bool) == Some(true) {
        actions.push(Action::open("timeline".to_owned(), None));
    }
    let mut workspace = default.clone();
    match workspace.apply_all(actions, false, |panel: &String| permissions(panel)) {
        Ok(_) => workspace,
        Err(_) => default.clone(),
    }
}

/// What a saved layout holds: the chrome flags and the workspace. A newer saved workspace that shows as the default
/// is written back as it was saved, so this version never overwrites it until the user changes the arrangement.
pub(crate) fn snapshot(app: &PhotocraftApp) -> Value {
    let kept = app.session.prefs().panel_layout.get("docking").filter(|docking| newer(docking)).cloned();
    let docking = match kept {
        Some(kept) if app.ui.docking == registry::default_workspace() => kept,
        _ => json!(app.ui.docking),
    };
    json!({"panels": app.ui.panels, "docking": docking})
}

// ---------------------------------------------------------------------------------------------------------------
// The old panel and dock words of saves and of `ui.set` (`panels.<group>`, `dockTabs.<group>`, `dock`).
// ---------------------------------------------------------------------------------------------------------------

/// The first panel of legacy group `key` that the layout holds.
fn shown_member(layout: &Layout<String>, panels: &[&str]) -> Option<String> {
    panels.iter().map(|panel| (*panel).to_owned()).find(|panel| layout.contains(panel))
}

/// Whether `panel` is the selected tab of the group or tab node holding it (an accordion entry always is).
fn selected(layout: &Layout<String>, panel: &String) -> bool {
    let Some(place) = layout.place(panel) else { return false };
    if let Some(group) = layout.group(&place) {
        return group.active_panel() == Some(panel);
    }
    match layout.node_of(&place) {
        Some(Node::Tabs { panels, active, .. }) => panels.get(*active) == Some(panel),
        _ => true,
    }
}

/// The workspace actions of the old panel and dock words, each a JSON object or absent, on the live workspace:
/// `panels.<group>` shows a hidden group's panels or closes a shown group's; `dockTabs.<group>: i` selects the i-th
/// panel of the group's tab order; `dock.hidden_tabs` closes the named tabs; `dock.collapsed` (every collapsed group)
/// minimizes those groups and expands the others. `dock.order` and `dock.heights` are refused: groups are arranged
/// with `window.panel.layout`.
pub(crate) fn legacy_actions(
    app: &PhotocraftApp,
    panels: Option<&Value>,
    dock_tabs: Option<&Value>,
    dock: Option<&Value>,
) -> Result<Vec<Action<String>>, String> {
    // The old dock words: a request names only these (order and heights are refused below).
    const DOCK_KEYS: [&str; 7] = ["order", "heights", "collapsed", "hidden_tabs", "hiddenTabs", "tab_order", "tabOrder"];
    if let Some(dock) = dock.filter(|dock| !dock.is_null()) {
        let Some(map) = dock.as_object() else { return Err("`dock` must be an object".into()) };
        if let Some(unknown) = map.keys().find(|key| !DOCK_KEYS.contains(&key.as_str())) {
            return Err(format!("unknown `dock` field `{unknown}`"));
        }
    }
    if dock.is_some_and(|dock| dock.get("order").is_some() || dock.get("heights").is_some()) {
        return Err("dock order/heights cannot replace a customized layout; use window.panel.layout".into());
    }
    legacy_on(&app.ui.docking.layout, panels, dock_tabs, dock)
}

/// [`legacy_actions`] on `layout`, without the order and heights refusal.
fn legacy_on(layout: &Layout<String>, panels: Option<&Value>, dock_tabs: Option<&Value>, dock: Option<&Value>) -> Result<Vec<Action<String>>, String> {
    let mut actions = Vec::new();
    // Each group's panels as they will be after the request's opens and closes, for the collapse step.
    let mut shown: Vec<(&str, Option<String>)> = LEGACY_GROUPS.iter().map(|(key, members)| (*key, shown_member(layout, members))).collect();
    let flag = |key: &str| panels.and_then(|panels| panels.get(key)).and_then(Value::as_bool);
    for ((key, members), (_, first)) in LEGACY_GROUPS.iter().zip(shown.iter_mut()) {
        match flag(key) {
            // A shown group stays as it is; a hidden one shows all of its panels at their places, with its first tab in
            // front. Each open shows the panel it opens, so the first is shown again after the others.
            Some(true) if first.is_none() => {
                actions.extend(members.iter().map(|panel| Action::open((*panel).to_owned(), None)));
                actions.extend(members.first().map(|panel| Action::activate((*panel).to_owned())));
                *first = members.first().map(|panel| (*panel).to_owned());
            }
            Some(false) => {
                actions.extend(members.iter().map(|panel| (*panel).to_owned()).filter(|panel| layout.contains(panel)).map(Action::close));
                *first = None;
            }
            _ => {}
        }
    }
    let hidden_tabs = dock.and_then(|dock| dock.get("hidden_tabs").or_else(|| dock.get("hiddenTabs"))).and_then(Value::as_object);
    for (key, names) in hidden_tabs.into_iter().flatten() {
        let Some((_, members)) = LEGACY_GROUPS.iter().find(|(group, _)| *group == key.as_str()) else { continue };
        for name in names.as_array().into_iter().flatten().filter_map(Value::as_str) {
            // A tab of a group this request opens above is closed again.
            if let Some(panel) = registry::normalize(name).filter(|panel| members.contains(panel)).map(str::to_owned)
                && (layout.contains(&panel) || actions.contains(&Action::open(panel.clone(), None)))
                && !actions.contains(&Action::close(panel.clone()))
            {
                actions.push(Action::close(panel));
            }
        }
    }
    for (key, index) in dock_tabs.and_then(Value::as_object).into_iter().flatten() {
        let Some((_, members)) = LEGACY_GROUPS.iter().find(|(group, _)| *group == key.as_str()) else { continue };
        let index = index.as_u64().ok_or("dockTabs entries must be tab indices")?;
        let panel = usize::try_from(index).ok().and_then(|index| members.get(index)).ok_or("dockTabs index is outside the panel group")?;
        if flag(key) == Some(false) {
            return Err("cannot select a tab in a hidden group".into());
        }
        let panel = (*panel).to_owned();
        // A panel this request opens above is there for the pick that follows.
        let present = layout.contains(&panel) || actions.contains(&Action::open(panel.clone(), None));
        actions.push(if present { Action::activate(panel) } else { Action::open(panel, None) });
    }
    if let Some(collapsed) = dock.and_then(|dock| dock.get("collapsed")).and_then(Value::as_array) {
        let collapsed: Vec<&str> = collapsed.iter().filter_map(Value::as_str).collect();
        if let Some(unknown) = collapsed.iter().find(|name| !LEGACY_GROUPS.iter().any(|(key, _)| key == *name)) {
            return Err(format!("`dock.collapsed` names no panel group: `{unknown}`"));
        }
        for (key, first) in &shown {
            let Some(first) = first else { continue };
            let minimize = collapsed.contains(key);
            if minimize != minimized(layout, first) || !layout.contains(first) {
                actions.push(Action::set_minimized(first.clone(), minimize));
            }
        }
    }
    Ok(actions)
}

/// The old panel and dock words of the live workspace, for readers of `ui.inspect`: `panels.<group>` is whether any
/// of the group's panels shows, `dockTabs.<group>` the index of its selected panel in the group's tab order, for the
/// groups that show.
pub(crate) fn legacy_view(app: &PhotocraftApp) -> Value {
    let layout = &app.ui.docking.layout;
    let mut panels = Map::new();
    let mut tabs = Map::new();
    for (key, members) in LEGACY_GROUPS {
        let laid_out: Vec<usize> = (0..members.len()).filter(|i| layout.contains(&members[*i].to_owned())).collect();
        panels.insert((*key).into(), json!(!laid_out.is_empty()));
        if let Some(index) = laid_out.iter().copied().find(|i| selected(layout, &members[*i].to_owned())).or_else(|| laid_out.first().copied()) {
            tabs.insert((*key).into(), json!(index));
        }
    }
    json!({"panels": panels, "dockTabs": tabs})
}

/// Where tests find panels as the last dock pass drew them.
#[cfg(test)]
pub(crate) mod probe {
    use super::*;
    use egui::Rect;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    pub(crate) use super::body;

    /// The panels of the tab group holding `panel`, in tab order.
    pub(crate) fn group_of(app: &PhotocraftApp, panel: &str) -> Vec<String> {
        group_members(&app.ui.docking.layout, &panel.to_owned())
    }

    /// The tab group holding `panel` as the last pass drew it, header and body: the group's node, a tab panel named
    /// after its shown panel.
    pub(crate) fn group_rect(h: &Harness<'_, Box<PhotocraftApp>>, panel: &str) -> Option<Rect> {
        let titles: Vec<String> = group_of(h.state(), panel).iter().filter_map(|member| registry::entry(member)).map(|entry| (entry.label)()).collect();
        h.query_all_by_role(Role::TabPanel).find(|n| n.accesskit_node().label().is_some_and(|label| titles.contains(&label))).map(|n| n.rect())
    }

    /// The id of the OS windows the last pass showed floating windows in, by each panel they hold.
    fn windows_id() -> egui::Id {
        egui::Id::unique("photocraft dock windows")
    }

    type Windows = Vec<(String, egui::ViewportId)>;

    /// Notes the OS windows the dock reported, `windows` by a panel of each, for every panel of their floating
    /// windows in `layout`, the layout the pass showed.
    pub(super) fn record_windows(ctx: &egui::Context, layout: &Layout<String>, windows: &[(String, egui::ViewportId)]) {
        let by_panel: Windows = windows
            .iter()
            .flat_map(|(anchor, viewport)| {
                let window = layout.floating.iter().find(|window| window.contains(anchor));
                window.into_iter().flat_map(|window| window.panels().cloned()).map(move |panel| (panel, *viewport))
            })
            .collect();
        ctx.data_mut(|data| data.insert_temp(windows_id(), by_panel));
    }

    /// The OS window the floating window holding `panel` showed in on the last pass, as the dock reported it.
    pub(crate) fn native_viewport(ctx: &egui::Context, panel: &str) -> Option<egui::ViewportId> {
        let windows = ctx.data(|data| data.get_temp::<Windows>(windows_id())).unwrap_or_default();
        windows.into_iter().find(|(shown, _)| shown == panel).map(|(_, viewport)| viewport)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod native_tests;
