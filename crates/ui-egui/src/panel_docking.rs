//! Application-owned panel IDs and commands around the shared docking layout.

use crate::dock::Group;
use craft_ui::docking::{Action, Floating, Layout, Node, Placement, Zone};
use craft_ui::layout::{SplitAxis, SplitSize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const PANELS: &[(&str, &str, Group)] = &[
    ("color", "Color", Group::Color),
    ("swatches", "Swatches", Group::Color),
    ("gradients", "Gradients", Group::Color),
    ("patterns", "Patterns", Group::Color),
    ("properties", "Properties", Group::Properties),
    ("adjustments", "Adjustments", Group::Properties),
    ("character", "Character", Group::Character),
    ("paragraph", "Paragraph", Group::Character),
    ("navigator", "Navigator", Group::Navigator),
    ("histogram", "Histogram", Group::Navigator),
    ("info", "Info", Group::Navigator),
    ("history", "History", Group::History),
    ("actions", "Actions", Group::History),
    ("layerComps", "Layer Comps", Group::History),
    ("layers", "Layers", Group::Layers),
    ("channels", "Channels", Group::Layers),
    ("paths", "Paths", Group::Layers),
    ("brushes", "Brushes", Group::Brushes),
];
#[derive(Clone, Default)]
struct LegacyDrag {
    generation: u64,
    layout: Option<Layout<String>>,
    tabs: crate::state::DockTabs,
    hidden: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
}

fn cancel_legacy_drag(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let key = egui::Id::new("photocraft-panel-docking").with("legacy-origin");
    let previous = ctx.data_mut(|data| data.remove_temp::<LegacyDrag>(key));
    if let Some(previous) = previous {
        craft_ui::docking::cancel_drag::<String>(ctx, egui::Id::new("photocraft-panel-docking"));
        if previous.generation != app.ui.docking_generation {
            return false;
        }
        app.ui.dock_tabs = previous.tabs;
        app.ui.docking = previous.layout;
        app.ui.docking_hidden = previous.hidden;
        true
    } else {
        false
    }
}

pub(crate) fn normalize(raw: &str) -> Option<&'static str> {
    PANELS.iter().find(|(id, label, _)| id.eq_ignore_ascii_case(raw.trim()) || label.eq_ignore_ascii_case(raw.trim())).map(|p| p.0)
}

pub(crate) fn valid(layout: &Layout<String>) -> bool {
    layout.validate().is_ok() && layout.panels().iter().all(|id| normalize(id) == Some(id.as_str()))
}

fn legacy(app: &PhotocraftApp) -> Layout<String> {
    legacy_layout(app, false)
}

fn legacy_layout(app: &PhotocraftApp, all: bool) -> Layout<String> {
    let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
    let mut layout = Layout::default();
    for group in app.ui.dock.order().into_iter().rev().filter(|g| all || g.shown(&app.ui.panels)) {
        let labels = group.tabs(pro);
        let visible = if all { group.tabs(pro).iter().copied().enumerate().collect() } else { app.ui.dock.visible_tabs(group, pro) };
        let panels: Vec<String> = visible.iter().filter_map(|(_, label)| normalize(label).map(str::to_owned)).collect();
        if panels.is_empty() {
            continue;
        }
        let selected = match group {
            Group::Color => app.ui.dock_tabs.color,
            Group::Properties => app.ui.dock_tabs.properties,
            Group::Character => app.ui.dock_tabs.character,
            Group::Navigator => app.ui.dock_tabs.navigator,
            Group::History => app.ui.dock_tabs.history,
            Group::Layers => app.ui.dock_tabs.layers,
            Group::Brushes => app.ui.dock_tabs.brushes,
        };
        let active = labels.get(selected).and_then(|label| normalize(label)).and_then(|id| panels.iter().position(|p| p == id)).unwrap_or(0);
        if group == Group::Properties && !pro {
            layout.floating.push(Floating { panels, active, rect: [120.0, 160.0, 320.0, 440.0] });
            continue;
        }
        let node = Node::Tabs { panels, active };
        layout.root = Some(match layout.root.take() {
            Some(second) => Node::Split {
                axis: SplitAxis::Vertical,
                size: SplitSize::FixedFirst(app.ui.dock.height(group)),
                first: Box::new(node),
                second: Box::new(second),
            },
            None => node,
        });
    }
    layout
}

// Native collapse flags remain app-owned. An intact native tab group keeps its collapsed
// presentation when the workspace first becomes customizable; mixed custom groups are expanded.
fn collapsed_members(app: &PhotocraftApp, layout: &Layout<String>) -> std::collections::HashSet<String> {
    let mut collapsed: std::collections::HashSet<String> = app.ui.docking_collapsed.iter().cloned().collect();
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Split { first, second, .. } => pending.extend([first.as_ref(), second.as_ref()]),
            Node::Tabs { panels, .. } => {
                let group = panels.first().and_then(|id| PANELS.iter().find(|p| p.0 == id)).map(|p| p.2);
                if let Some(group) = group.filter(|group| app.ui.dock.is_collapsed(*group)) {
                    let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
                    let native: Vec<_> = app.ui.dock.visible_tabs(group, pro).iter().filter_map(|(_, name)| normalize(name)).collect();
                    if panels.len() == native.len() && panels.iter().all(|panel| native.contains(&panel.as_str())) {
                        collapsed.extend(panels.iter().cloned());
                    }
                }
            }
            _ => {}
        }
    }
    collapsed
}

fn first_root(layout: &Layout<String>, except: &str) -> Option<String> {
    let root = Layout { root: layout.root.clone(), floating: Vec::new() };
    root.panels().into_iter().find(|p| p.as_str() != except).cloned()
}

fn ensure(layout: &mut Layout<String>, panel: &str) -> Result<(), String> {
    let anchor = first_root(layout, panel);
    layout.apply(Action::Open { panel: panel.into(), anchor }).map_err(|e| e.to_string())
}

fn number(params: &Value, key: &str, default: f32) -> Result<f32, String> {
    match params.get(key) {
        None => Ok(default),
        Some(v) => v.as_f64().filter(|v| v.is_finite() && v.abs() <= 1_000_000.0).map(|v| v as f32).ok_or_else(|| format!("{key} must be a finite coordinate")),
    }
}

fn placement(params: &Value) -> Result<Placement<String>, String> {
    if let Some(before) = params.get("before") {
        let id = before.as_str().and_then(normalize).ok_or("before must name a panel")?;
        return Ok(Placement::Tab { before: Some(id.into()) });
    }
    let zone = match params.get("zone") {
        None => "tab",
        Some(value) => value.as_str().ok_or("zone must be a string")?,
    };
    Ok(match zone {
        "tab" => Placement::Tab { before: None },
        "center" => Placement::Split(Zone::Center),
        "left" => Placement::Split(Zone::Left),
        "right" => Placement::Split(Zone::Right),
        "top" => Placement::Split(Zone::Top),
        "bottom" => Placement::Split(Zone::Bottom),
        _ => return Err("zone must be tab, center, left, right, top or bottom".into()),
    })
}

pub(crate) fn command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.group" {
        return Some(group_command(app, params));
    }
    if id == "window.panel.layout" {
        return Some(
            params
                .get("action")
                .ok_or_else(|| "action is required".to_string())
                .and_then(|value| serde_json::from_value::<Action<String>>(value.clone()).map_err(|error| error.to_string()))
                .and_then(|action| apply_action(app, action)),
        );
    }
    if id == "window.panel.detach" || id == "window.panel.attach" {
        return Some(window_command(app, id == "window.panel.detach", params));
    }
    if id == "window.panel.railMove" {
        return Some(rail_move(app, params));
    }
    if id == "window.panel.toRail" || id == "window.panel.fromRail" {
        return Some(rail_command(app, id == "window.panel.toRail", params));
    }
    if let Some(result) = rail_window_item(app, id, params) {
        return Some(result);
    }
    if let Some(result) = open_floating(app, id, params) {
        return Some(result);
    }
    if let Some(panel) = id.strip_prefix("window.panel.").or_else(|| id.strip_prefix("window.toggle.")).and_then(normalize) {
        if app.ui.docking.is_none() && panel != "brushes" {
            return None;
        }
        let showing = app.ui.docking.as_ref().is_some_and(|layout| layout.contains(&panel.to_string()));
        let show = match params.get("show").or_else(|| params.get("on")) {
            None => !showing || app.ui.docking.as_ref().is_some_and(|layout| !panel_active(layout, panel) || collapsed_members(app, layout).contains(panel)),
            Some(value) => match value.as_bool() {
                Some(show) => show,
                None => return Some(Err("show must be a boolean".into())),
            },
        };
        return Some(change(app, if show { "activate" } else { "close" }, &json!({"panel":panel})));
    }
    let operation = match id {
        "window.panel.move" => "move",
        "window.panel.float" => "float",
        "window.panel.dock" => "dock",
        "window.panel.activate" => "activate",
        "window.panel.close" => "close",
        "window.panel" if app.ui.docking.is_some() => "activate",
        _ => return None,
    };
    Some(change(app, operation, params))
}

/// Move to Separate Window / Return to PhotoCraft Window: a panel's group as its own OS window,
/// free to leave the app window (for another monitor), or back to floating inside it. A docked
/// panel floats first. Optional `x`, `y`, `width`, `height` place the window in screen points.
fn window_command(app: &mut PhotocraftApp, detach: bool, params: &Value) -> Result<Value, String> {
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
    let keys = ["x", "y", "width", "height"];
    let rect = match keys.iter().filter(|k| params.get(**k).is_some()).count() {
        0 => None,
        4 => {
            let [x, y, width, height] = keys.map(|k| number(params, k, 0.0));
            let rect = [x?, y?, width?, height?];
            if rect[2] < 1.0 || rect[3] < 1.0 {
                return Err("width and height must be positive".into());
            }
            Some(rect)
        }
        _ => return Err("give all of x, y, width and height, or none".into()),
    };
    if rect.is_some_and(|r| r[2] > MAX_WINDOW_SIZE || r[3] > MAX_WINDOW_SIZE) {
        return Err(format!("width and height must be at most {MAX_WINDOW_SIZE}"));
    }
    if detach {
        let floating = app.ui.docking.as_ref().is_some_and(|layout| layout.floating.iter().any(|g| g.panels.iter().any(|p| p == panel)));
        if !floating {
            change(app, "float", &json!({"panel": panel}))?;
        }
    }
    let members = app.ui.docking.as_ref().map(|layout| group_members(layout, panel)).filter(|m| !m.is_empty()).unwrap_or_else(|| vec![panel.to_owned()]);
    // One entry per window, under the panel it was opened by.
    app.ui.docking_windows.retain(|id, _| id != panel && !members.contains(id));
    if detach {
        app.ui.docking_windows.insert(panel.to_owned(), rect);
    }
    Ok(json!({"panel": panel, "window": detach}))
}

/// Dock to Icon Bar / Remove from Icon Bar. Docking takes the panel (`group: true`: every tab of
/// its group) out of the dock, a floating group or its own window and makes it an icon on the
/// bar beside the dock; its old spot is remembered. Removing takes `to`: `float`, `dock` (back
/// where it was) or `none` (closed, as Window › <panel> leaves it).
fn rail_command(app: &mut PhotocraftApp, to_rail: bool, params: &Value) -> Result<Value, String> {
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
    if to_rail {
        let group = match params.get("group") {
            None => false,
            Some(value) => value.as_bool().ok_or("group must be a boolean")?,
        };
        let layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
        let panels = if group { group_members(&layout, panel) } else { Vec::new() };
        let panels = if panels.is_empty() { vec![panel.to_owned()] } else { panels };
        for id in &panels {
            let showing = app.ui.docking.clone().unwrap_or_else(|| legacy(app)).contains(id);
            if showing {
                change(app, "close", &json!({"panel": id}))?;
            }
            app.ui.docking_windows.remove(id);
            if !app.ui.rail_panels.contains(id) {
                app.ui.rail_panels.push(id.clone());
            }
        }
        app.ui.rail_open = None;
        return Ok(json!({"rail": app.ui.rail_panels}));
    }
    // Off the bar, a panel becomes a floating window unless asked to dock or just close.
    let to = match params.get("to") {
        None => "float",
        Some(value) => value.as_str().filter(|to| ["float", "dock", "none"].contains(to)).ok_or("to must be float, dock or none")?,
    };
    let Some(index) = app.ui.rail_panels.iter().position(|id| id == panel) else { return Err("panel is not on the icon bar".into()) };
    let previous_open = app.ui.rail_open.clone();
    app.ui.rail_panels.remove(index);
    if app.ui.rail_open.as_deref() == Some(panel) {
        app.ui.rail_open = None;
    }
    let result = match to {
        "float" => {
            // Optional placement, e.g. where its icon was dropped.
            let mut float = json!({"panel": panel});
            for key in ["x", "y", "width", "height"] {
                if let Some(value) = params.get(key) {
                    float[key] = value.clone();
                }
            }
            change(app, "float", &float)
        }
        "dock" => change(app, "dock", &json!({"panel": panel})),
        _ => Ok(Value::Null),
    };
    if let Err(error) = result {
        // Nothing moved: it stays on the bar.
        app.ui.rail_panels.insert(index.min(app.ui.rail_panels.len()), panel.to_owned());
        app.ui.rail_open = previous_open;
        return Err(error);
    }
    Ok(json!({"rail": app.ui.rail_panels}))
}

/// The icon bar's built-in group buttons, in their default order.
pub(crate) const RAIL_GROUPS: [Group; 6] = [Group::Properties, Group::Navigator, Group::Color, Group::Layers, Group::History, Group::Brushes];

/// The built-in group buttons in the order the user arranged them.
pub(crate) fn rail_group_order(app: &PhotocraftApp) -> Vec<Group> {
    let mut order: Vec<Group> = Vec::new();
    for group in app.ui.rail_groups.iter().copied().chain(RAIL_GROUPS) {
        if RAIL_GROUPS.contains(&group) && !order.contains(&group) {
            order.push(group);
        }
    }
    order
}

/// Move Icon: put a bar icon at `index` in its section, either a built-in `group` button or a
/// docked `panel`.
fn rail_move(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let index = params.get("index").and_then(Value::as_u64).ok_or("index must be a non-negative integer")?;
    let index = usize::try_from(index).map_err(|_| "index is out of range".to_string())?;
    if let Some(group) = params.get("group") {
        let group = group.as_str().and_then(Group::from_key).filter(|g| RAIL_GROUPS.contains(g)).ok_or("group must name an icon bar group")?;
        let mut order = rail_group_order(app);
        order.retain(|g| *g != group);
        if index > order.len() {
            return Err("index is out of range".into());
        }
        order.insert(index, group);
        app.ui.rail_groups = order.clone();
        return Ok(json!({"groups": order.iter().map(|g| g.key()).collect::<Vec<_>>()}));
    }
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("give a group or a panel on the icon bar")?;
    let from = app.ui.rail_panels.iter().position(|p| p == panel).ok_or("panel is not on the icon bar")?;
    if index >= app.ui.rail_panels.len() {
        return Err("index is out of range".into());
    }
    let moved = app.ui.rail_panels.remove(from);
    app.ui.rail_panels.insert(index, moved);
    Ok(json!({"rail": app.ui.rail_panels}))
}

/// An icon being dragged within the bar.
pub(crate) enum RailItem {
    Group(Group),
    Panel(String),
}

/// An icon dragged within the bar moves among the others in its section: a line shows where it
/// will land, and letting go puts it there. `groups` and `panels` are the icons as drawn.
pub(crate) fn rail_reorder(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    item: &RailItem,
    released: bool,
    groups: &[(Group, egui::Rect)],
    panels: &[(String, egui::Rect)],
    rail: egui::Rect,
) {
    let Some(pointer) = ctx.pointer_interact_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos())) else { return };
    if !rail.contains(pointer) || app.session.prefs().workspace_locked {
        return;
    }
    // The other icons of its section, top to bottom, and where the dragged one sits now.
    let (others, current): (Vec<egui::Rect>, Option<usize>) = match item {
        RailItem::Group(group) => (groups.iter().filter(|(g, _)| g != group).map(|(_, r)| *r).collect(), groups.iter().position(|(g, _)| g == group)),
        RailItem::Panel(panel) => (panels.iter().filter(|(p, _)| p != panel).map(|(_, r)| *r).collect(), panels.iter().position(|(p, _)| p == panel)),
    };
    let index = others.iter().filter(|r| r.center().y < pointer.y).count();
    if Some(index) == current {
        return;
    }
    if !released {
        let y = others.get(index).map_or_else(|| others.last().map_or(rail.top(), |r| r.bottom() + 2.0), |r| r.top() - 2.0);
        let t = crate::theme::Tokens::get(ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, rail_drag_id().with("reorder")));
        painter.line_segment([egui::pos2(rail.left() + 4.0, y), egui::pos2(rail.right() - 4.0, y)], egui::Stroke::new(2.0, t.accent));
        return;
    }
    let params = match item {
        RailItem::Panel(panel) => json!({"panel": panel, "index": index}),
        RailItem::Group(group) => {
            // Hidden buttons (groups living only on the bar) keep their place: insert before the
            // drawn button it was dropped above, in the full order.
            let drawn: Vec<Group> = groups.iter().map(|(g, _)| *g).filter(|g| g != group).collect();
            let mut order = rail_group_order(app);
            order.retain(|g| g != group);
            let at = drawn.get(index).and_then(|before| order.iter().position(|g| g == before)).unwrap_or(order.len());
            json!({"group": group.key(), "index": at})
        }
    };
    if let Err(error) = app.run("window.panel.railMove", params) {
        app.ui.status = error;
    }
}

/// The size a panel undocked from the icon bar floats at.
const UNDOCKED_SIZE: egui::Vec2 = egui::vec2(300.0, 400.0);

/// An icon-bar panel's icon being dragged: an outline of the window it will become follows the
/// pointer, and released away from the bar the panel floats there.
pub(crate) fn rail_icon_drag(app: &mut PhotocraftApp, ctx: &egui::Context, panel: &str, label: &str, rail: egui::Rect, released: bool) {
    let Some(pointer) = ctx.pointer_interact_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos())) else { return };
    let away = !rail.contains(pointer);
    let window = egui::Rect::from_min_size(pointer - egui::vec2(24.0, 12.0), UNDOCKED_SIZE);
    if !released {
        if away {
            let t = crate::theme::Tokens::get(ctx);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, rail_drag_id().with("undock")));
            painter.rect(window, t.radius_sm, t.accent.gamma_multiply(0.12), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            painter.text(window.min + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, tl!(label), egui::FontId::proportional(13.0), t.text);
        }
        return;
    }
    if away {
        undock_from_rail(app, panel, window);
    }
}

fn undock_from_rail(app: &mut PhotocraftApp, panel: &str, window: egui::Rect) {
    let params = json!({"panel": panel, "to": "float", "x": window.left(), "y": window.top(), "width": window.width(), "height": window.height()});
    if let Err(error) = app.run("window.panel.fromRail", params) {
        app.ui.status = error;
    }
}

/// Window › <panel> for a panel on the icon bar opens (or closes) its flyout, as clicking its
/// icon does, rather than pulling it off the bar.
fn rail_window_item(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let panel = id.strip_prefix("window.panel.").or_else(|| id.strip_prefix("window.toggle.")).and_then(normalize)?;
    if !app.ui.rail_panels.iter().any(|p| p == panel) {
        return None;
    }
    let show = match params.get("show").or_else(|| params.get("on")) {
        None => app.ui.rail_open.as_deref() != Some(panel),
        Some(value) => match value.as_bool() {
            Some(show) => show,
            None => return Some(Err("show must be a boolean".into())),
        },
    };
    app.ui.rail_open = if show { Some(panel.to_owned()) } else { app.ui.rail_open.take().filter(|open| open != panel) };
    Some(Ok(json!({"panel": panel, "flyout": show})))
}

/// A group that lives only on the icon bar: its panels have their own icons there, so the
/// rail's built-in button for the group steps aside rather than show it twice.
pub(crate) fn rail_replaces(app: &PhotocraftApp, group: Group) -> bool {
    app.ui.docking.as_ref().is_some_and(|layout| {
        !PANELS.iter().any(|p| p.2 == group && layout.contains(&p.0.to_owned()))
            && PANELS.iter().any(|p| p.2 == group && app.ui.rail_panels.iter().any(|r| r == p.0))
    })
}

/// The icon-bar entries: panel id, icon and label.
pub(crate) fn rail_entries(app: &PhotocraftApp) -> Vec<(String, &'static str, &'static str)> {
    app.ui.rail_panels.iter().filter_map(|id| PANELS.iter().find(|p| p.0 == id).map(|p| (id.clone(), panel_icon(p.0), p.1))).collect()
}

fn panel_icon(panel: &str) -> &'static str {
    match panel {
        "color" => "palette",
        "swatches" => "swatch-book",
        "gradients" => "gradient",
        "patterns" => "grid-2x2",
        "properties" => "sliders-horizontal",
        "adjustments" => "adjustment-layer",
        "character" => "type",
        "paragraph" => "align-left",
        "navigator" => "navigation",
        "histogram" => "gauge",
        "info" => "info",
        "history" => "clock",
        "actions" => "play",
        "layerComps" => "copy",
        "layers" => "layers",
        "channels" => "contrast",
        "paths" => "pen-tool",
        "brushes" => "brush",
        _ => "panel-right",
    }
}

fn rail_rect_id() -> egui::Id {
    egui::Id::new("photocraft-panel-docking").with("rail-rect")
}

/// Where the icon bar was drawn, so a panel dragged there can dock to it.
pub(crate) fn note_rail_rect(ctx: &egui::Context, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(rail_rect_id(), rect));
}

fn rail_drag_id() -> egui::Id {
    egui::Id::new("photocraft-panel-docking").with("rail-drag")
}

/// A press on a panel tab or a floating group's grip may end on the icon bar: remember what
/// was picked up (`true` for a whole group) and where. The bar is a blocked area for the shared
/// layout, so a drop there is ours: the panel docks to the bar.
fn rail_drop(app: &mut PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>) {
    let area = egui::Id::new("photocraft-panel-docking");
    let key = rail_drag_id();
    let (pressed, released, down, pos) =
        ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down(), i.pointer.interact_pos()));
    if pressed && let Some(origin) = ctx.input(|i| i.pointer.press_origin()) {
        let tab = layout.panels().into_iter().find(|panel| ctx.read_response(area.with(("tab", *panel))).is_some_and(|r| r.rect.contains(origin)));
        let grip = || {
            layout
                .floating
                .iter()
                .filter_map(|group| group.panels.first())
                .find(|first| ctx.read_response(area.with(("move-group", Some(*first)))).is_some_and(|r| r.rect.contains(origin)))
        };
        let picked = tab.map(|panel| (panel.clone(), false)).or_else(|| grip().map(|panel| (panel.clone(), true)));
        ctx.data_mut(|d| match picked {
            Some((panel, group)) => {
                d.insert_temp(key, (panel, group, origin));
            }
            None => {
                d.remove::<(String, bool, egui::Pos2)>(key);
            }
        });
    }
    let Some((panel, group, origin)) = ctx.data(|d| d.get_temp::<(String, bool, egui::Pos2)>(key)) else { return };
    let (Some(rail), Some(pos)) = (ctx.data(|d| d.get_temp::<egui::Rect>(rail_rect_id())), pos) else { return };
    let over = origin.distance(pos) > 4.0 && rail.contains(pos);
    if down && over {
        let t = crate::theme::Tokens::get(ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, key));
        painter.rect(rail.shrink(1.0), t.radius_sm, t.accent.gamma_multiply(0.15), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    if released || !down {
        ctx.data_mut(|d| d.remove::<(String, bool, egui::Pos2)>(key));
    }
    if released && over {
        // A drag that began in the native dock must not be rolled back as cancelled.
        ctx.data_mut(|d| d.remove::<LegacyDrag>(area.with("legacy-origin")));
        if let Err(error) = app.run("window.panel.toRail", json!({"panel": panel, "group": group})) {
            app.ui.status = error;
        }
    }
}

/// The open icon-bar panel, beside the bar: a header with its menu, then the panel. A click
/// elsewhere (not in a menu it opened) tucks it away again.
pub(crate) fn rail_flyout(app: &mut PhotocraftApp, ctx: &egui::Context, rail: egui::Rect, button: Option<egui::Rect>) {
    let Some(panel) = app.ui.rail_open.clone().filter(|p| app.ui.rail_panels.contains(p)) else {
        app.ui.rail_open = None;
        return;
    };
    let t = crate::theme::Tokens::get(ctx);
    let size = egui::vec2(300.0, (rail.height() - 16.0).clamp(160.0, 460.0));
    let top = button.map_or(rail.top(), |b| b.top()).min(rail.bottom() - size.y).max(rail.top());
    let left = if crate::direction::is_rtl() { rail.right() + 4.0 } else { rail.left() - size.x - 4.0 };
    let label = PANELS.iter().find(|p| p.0 == panel).map_or(panel.as_str(), |p| p.1);
    let locked = app.session.prefs().workspace_locked;
    let mut request: Option<&'static str> = None;
    let mut grip = None;
    let shown =
        egui::Area::new(egui::Id::new("photocraft-rail-flyout")).order(egui::Order::Middle).fixed_pos(egui::pos2(left, top)).constrain(true).show(ctx, |ui| {
            egui::Frame::NONE.fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(t.radius_sm).shadow(ui.visuals().popup_shadow).show(
                ui,
                |ui| {
                    ui.set_width(size.x);
                    ui.set_height(size.y);
                    // The title bar tears the panel off the bar into a floating window. Its buttons,
                    // drawn after, keep their clicks.
                    let bar = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(size.x, 28.0));
                    grip = Some(ui.interact(bar, egui::Id::new("photocraft-rail-flyout-grip"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab));
                    egui::Frame::NONE.fill(t.tab_strip).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            // Not selectable text, so a press on the name drags the title bar.
                            ui.add(egui::Label::new(egui::RichText::new(tl!(label)).color(t.text)).selectable(false).sense(egui::Sense::hover()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                                    request = Some("close");
                                }
                                let menu = crate::icons::button(ui, "ellipsis", 20.0, false, tl!("Panel menu"));
                                egui::Popup::menu(&menu).show(|ui| {
                                    ui.set_min_width(200.0);
                                    panel_menu(app, ui, &panel);
                                    for (text, wanted) in [(tl!("Float Panel"), "float"), (tl!("Dock Panel"), "dock")] {
                                        if ui.add_enabled(!locked, egui::Button::new(text)).clicked() {
                                            request = Some(wanted);
                                            ui.close();
                                        }
                                    }
                                });
                            });
                        });
                    });
                    egui::Frame::NONE.inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        body(app, ui, &panel);
                    });
                },
            );
        });
    if let Some(grip) = grip.filter(|_| !locked)
        && (grip.dragged() || grip.drag_stopped())
        && let Some(pointer) = ctx.pointer_interact_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos()))
    {
        let grab = ctx.input(|i| i.pointer.press_origin()).map_or(egui::vec2(24.0, 12.0), |origin| origin - shown.response.rect.min);
        let window = egui::Rect::from_min_size(pointer - grab, UNDOCKED_SIZE);
        if grip.drag_stopped() {
            // Let go over the flyout itself or the bar, it stays on the bar.
            if !rail.contains(pointer) && !shown.response.rect.contains(pointer) {
                undock_from_rail(app, &panel, window);
                return;
            }
        } else {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, rail_drag_id().with("tear-off")));
            painter.rect(window, t.radius_sm, t.accent.gamma_multiply(0.12), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
    }
    let clicked_away = ctx.input(|i| i.pointer.any_pressed())
        && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !shown.response.rect.contains(p) && !rail.contains(p))
        && !egui::Popup::is_any_open(ctx);
    match request {
        Some("close") => app.ui.rail_open = None,
        Some(to) => {
            if let Err(error) = app.run("window.panel.fromRail", json!({"panel": panel, "to": to})) {
                app.ui.status = error;
            }
        }
        None if clicked_away => app.ui.rail_open = None,
        None => {}
    }
}

/// The largest side a separate panel window may have, in points: a window the GPU surface can
/// always back, whatever a command or a saved layout asks for.
const MAX_WINDOW_SIZE: f32 = 8192.0;

/// A window side the OS and GPU surface accept: at least `min`, at most [`MAX_WINDOW_SIZE`], and
/// `min` for a non-finite size (`clamp` would pass a NaN through).
fn fit_side(side: f32, min: f32) -> f32 {
    if side.is_finite() { side.clamp(min, MAX_WINDOW_SIZE) } else { min }
}

/// Separate windows read from a saved layout, which may be stale or hand-edited: entries for
/// unknown panels are dropped, and a geometry that isn't a usable window (non-finite, far off,
/// or too small or large to show) is forgotten, so the window opens over the app window instead.
pub(crate) fn sanitize_windows(windows: std::collections::BTreeMap<String, Option<[f32; 4]>>) -> std::collections::BTreeMap<String, Option<[f32; 4]>> {
    let usable = |[x, y, w, h]: [f32; 4]| {
        [x, y, w, h].iter().all(|v| v.is_finite())
            && x.abs() <= 1_000_000.0
            && y.abs() <= 1_000_000.0
            && (1.0..=MAX_WINDOW_SIZE).contains(&w)
            && (1.0..=MAX_WINDOW_SIZE).contains(&h)
    };
    windows.into_iter().filter(|(id, _)| normalize(id) == Some(id.as_str())).map(|(id, rect)| (id, rect.filter(|r| usable(*r)))).take(PANELS.len()).collect()
}

/// Floating groups shown as OS windows, by the panel their entry is under. An entry whose panel
/// has docked is dropped; one whose panel is closed waits for it to reopen.
fn separate_windows(app: &mut PhotocraftApp, layout: &Layout<String>) -> Vec<String> {
    let mut shown: Vec<String> = Vec::new();
    let mut groups: Vec<Vec<String>> = Vec::new();
    app.ui.docking_windows.retain(|id, _| {
        if !layout.contains(id) {
            return true;
        }
        let Some(group) = layout.floating.iter().find(|g| g.panels.contains(id)) else { return false };
        if groups.contains(&group.panels) {
            return false;
        }
        groups.push(group.panels.clone());
        shown.push(id.clone());
        true
    });
    shown
}

/// What a separate panel window asked for this frame, run after it is drawn.
enum WindowRequest {
    Activate(String),
    Attach,
    Dock,
    Rail,
    Close,
}

/// One floating group as its own OS window: its tabs, the panel menu, and the active panel.
/// Immediate, like the screen colour picker: a deferred viewport stopped the main window
/// redrawing on Windows.
fn show_window(app: &mut PhotocraftApp, ctx: &egui::Context, key: &str, reveal: Option<&String>) {
    let Some(group) = app.ui.docking.as_ref().and_then(|layout| layout.floating.iter().find(|g| g.panels.iter().any(|p| p == key))).cloned() else {
        return;
    };
    let active = group.panels.get(group.active).cloned().unwrap_or_else(|| key.to_owned());
    let label = |id: &str| PANELS.iter().find(|p| p.0 == id).map(|p| tl!(p.1).to_string()).unwrap_or_else(|| id.to_owned());
    // Created where it was left, or over its spot in the app window; after that the OS window
    // owns its geometry, so the builder keeps its first placement and doesn't fight a move.
    let placed = egui::Id::new("photocraft-panel-window").with(key);
    let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).unwrap_or_default();
    let stored = app.ui.docking_windows.get(key).copied().flatten();
    let [x, y, width, height] = ctx.data_mut(|d| {
        *d.get_temp_mut_or_insert_with(placed, || stored.unwrap_or([origin.x + group.rect[0], origin.y + group.rect[1], group.rect[2], group.rect[3]]))
    });
    let title = group.panels.iter().map(|p| label(p)).collect::<Vec<_>>().join(" · ");
    let builder = egui::ViewportBuilder::default()
        .with_title(format!("{title} — PhotoCraft"))
        .with_position(if x.is_finite() && y.is_finite() { [x, y] } else { [origin.x, origin.y] })
        .with_inner_size([fit_side(width, 220.0), fit_side(height, 120.0)])
        .with_min_inner_size([220.0, 120.0]);
    let viewport = egui::ViewportId::from_hash_of(("photocraft-panel-window", key));
    if reveal.is_some_and(|p| group.panels.contains(p)) {
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
    }
    let locked = app.session.prefs().workspace_locked;
    let mut request = None;
    let mut geometry = None;
    ctx.show_viewport_immediate(viewport, builder, |ui, _| {
        let t = crate::theme::Tokens::get(ui.ctx());
        let (close, outer, inner) = ui.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
        if close {
            request = Some(WindowRequest::Close);
        }
        if let (Some(outer), Some(inner)) = (outer, inner) {
            geometry = Some([outer.left(), outer.top(), inner.width(), inner.height()]);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.card)).show(ui, |ui| {
            egui::Frame::NONE.fill(t.tab_strip).inner_margin(egui::Margin::symmetric(4, 2)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    for panel in &group.panels {
                        if ui.selectable_label(*panel == active, label(panel)).clicked() && *panel != active {
                            request = Some(WindowRequest::Activate(panel.clone()));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let menu = crate::icons::button(ui, "ellipsis", 22.0, false, tl!("Panel menu"));
                        egui::Popup::menu(&menu).show(|ui| {
                            ui.set_min_width(200.0);
                            panel_menu(app, ui, &active);
                            for (label, wanted) in [
                                (tl!("Return to PhotoCraft Window"), WindowRequest::Attach),
                                (tl!("Dock Panel Group"), WindowRequest::Dock),
                                (tl!("Dock to Icon Bar"), WindowRequest::Rail),
                                (tl!("Close Tab Group"), WindowRequest::Close),
                            ] {
                                if ui.add_enabled(!locked, egui::Button::new(label)).clicked() {
                                    request = Some(wanted);
                                    ui.close();
                                }
                            }
                        });
                    });
                });
            });
            egui::Frame::NONE.inner_margin(egui::Margin::same(8)).show(ui, |ui| body(app, ui, &active));
        });
    });
    if let Some(rect) = geometry.filter(|r| r.iter().all(|v| v.is_finite()))
        && app.ui.docking_windows.get(key).is_some_and(|stored| *stored != Some(rect))
        && !ctx.input(|i| i.pointer.any_down())
    {
        app.ui.docking_windows.insert(key.to_owned(), Some(rect));
    }
    let result = match request {
        None => return,
        Some(WindowRequest::Activate(panel)) => {
            if let Err(error) = app.run("window.panel.activate", json!({"panel": panel})) {
                app.ui.status = error;
            }
            return;
        }
        Some(WindowRequest::Attach) => app.run("window.panel.attach", json!({"panel": key})),
        Some(WindowRequest::Close) => app.run("window.panel.group", json!({"panel": key, "operation": "close"})),
        Some(WindowRequest::Rail) => app.run("window.panel.toRail", json!({"panel": key, "group": true})),
        Some(WindowRequest::Dock) => {
            group.panels.iter().try_for_each(|panel| app.run("window.panel.dock", json!({"panel": panel})).map(|_| ())).map(|_| Value::Null)
        }
    };
    // Gone from the screen: the next creation places it from the stored geometry again.
    ctx.data_mut(|d| d.remove::<[f32; 4]>(placed));
    if let Err(error) = result {
        app.ui.status = error;
    }
}

/// A collapsed group is only its tab strip and can't grow, so a divider whose growing side is
/// collapsed has nowhere to give space: the group above fills the column and the drag does
/// nothing. As in Photoshop, dragging a divider towards a collapsed group expands that group
/// (the one touching the divider), and the drag then resizes as usual.
fn expand_toward_drag(app: &mut PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>, collapsed: &std::collections::HashSet<String>) {
    fn edge_group(node: &Node<String>, last: bool, depth: usize) -> Option<&Vec<String>> {
        match node {
            _ if depth > craft_ui::docking::MAX_DEPTH => None,
            Node::Split { first, second, .. } => edge_group(if last { second } else { first }, last, depth + 1),
            Node::Tabs { panels, .. } => Some(panels),
            Node::Stack { .. } => None,
        }
    }
    let area = egui::Id::new("photocraft-panel-docking");
    let mut pending: Vec<(&Node<String>, Vec<bool>)> = layout.root.iter().map(|root| (root, Vec::new())).collect();
    while let Some((node, path)) = pending.pop() {
        let Node::Split { axis, first, second, .. } = node else { continue };
        if path.len() < craft_ui::docking::MAX_DEPTH {
            pending.push((first, [path.as_slice(), &[false]].concat()));
            pending.push((second, [path.as_slice(), &[true]].concat()));
        }
        if *axis != SplitAxis::Vertical {
            continue;
        }
        let Some(response) = ctx.read_response(area.with(("split", &path))) else { continue };
        let delta = response.drag_delta().y;
        if !response.dragged() || delta == 0.0 {
            continue;
        }
        // Up grows the side below the divider, down the side above it.
        let growing = if delta < 0.0 { edge_group(second, false, 0) } else { edge_group(first, true, 0) };
        if let Some(panel) = growing.filter(|panels| panels.iter().all(|p| collapsed.contains(p))).and_then(|panels| panels.first()) {
            if let Err(error) = app.run("window.panel.group", json!({"panel": panel, "operation": "expand"})) {
                app.ui.status = error;
            }
            return;
        }
    }
}

/// What the dock grows by when a panel docks beside a group as a new column.
const COLUMN_WIDTH: f32 = 260.0;

/// Side-by-side columns in the dock: a left/right split adds its sides' columns, a top/bottom
/// split is as wide as its wider side.
fn column_count(node: Option<&Node<String>>) -> usize {
    // Bounded by the layout's validated depth; a deeper tree just counts as one column.
    fn count(node: &Node<String>, depth: usize) -> usize {
        match node {
            _ if depth > craft_ui::docking::MAX_DEPTH => 1,
            Node::Split { axis: SplitAxis::Horizontal, first, second, .. } => count(first, depth + 1) + count(second, depth + 1),
            Node::Split { first, second, .. } => count(first, depth + 1).max(count(second, depth + 1)),
            Node::Tabs { .. } | Node::Stack { .. } => 1,
        }
    }
    node.map_or(0, |node| count(node, 0))
}

/// The dock's width range in a window `window_width` wide (#7896), widened so every column can keep
/// a usable width.
fn dock_width_range(columns: usize, window_width: f32) -> std::ops::RangeInclusive<f32> {
    let range = crate::panels::dock_width_range(window_width);
    *range.start()..=range.end().max(columns as f32 * (COLUMN_WIDTH + 70.0))
}

/// Where the first panel opened from the Window menu floats; later ones cascade from here.
const FLOAT_ORIGIN: [f32; 4] = [400.0, 140.0, 300.0, 400.0];
const FLOAT_CASCADE: f32 = 28.0;
/// The gap between floating windows opened side by side.
const FLOAT_GAP: f32 = 12.0;

/// Where a newly opened floating panel goes: beside the floating windows already open, within
/// `bounds` (the canvas area), so it never covers one of them. Only when there is no room does it
/// cascade, and then onto a corner no other window starts at, so its title bar stays visible.
fn open_slot(layout: &Layout<String>, bounds: egui::Rect) -> [f32; 4] {
    let [x, y, w, h] = FLOAT_ORIGIN;
    let taken: Vec<egui::Rect> =
        layout.floating.iter().map(|g| egui::Rect::from_min_size(egui::pos2(g.rect[0], g.rect[1]), egui::vec2(g.rect[2], g.rect[3]))).collect();
    let fits = |r: egui::Rect| bounds.contains_rect(r) && !taken.iter().any(|t| t.intersects(r));
    // To the right of the origin first, then to its left.
    let candidates = (0..8).map(|k| x + k as f32 * (w + FLOAT_GAP)).chain((1..8).map(|k| x - k as f32 * (w + FLOAT_GAP)));
    for left in candidates {
        let r = egui::Rect::from_min_size(egui::pos2(left, y), egui::vec2(w, h));
        if fits(r) {
            return [left, y, w, h];
        }
    }
    let step = (0..8).map(|k| k as f32 * FLOAT_CASCADE).find(|step| !taken.iter().any(|t| t.min == egui::pos2(x + step, y + step))).unwrap_or(0.0);
    [x + step, y + step, w, h]
}

/// Window › <panel> for a dock panel that isn't showing opens it as a floating window, ready to
/// drag and resize (Window › Workspace › Open Panels Floating). A panel closed while floating
/// returns where it was. A panel already showing keeps the usual activate/close toggle, and
/// a locked workspace or the setting turned off opens it in the dock as before.
fn open_floating(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let prefs = app.session.prefs();
    if prefs.window_panels_docked || prefs.workspace_locked {
        return None;
    }
    let panel = id.strip_prefix("window.panel.").or_else(|| id.strip_prefix("window.toggle.")).and_then(normalize)?;
    // An explicit hide (or a malformed value, which the dock path reports) isn't an open.
    if params.get("show").or_else(|| params.get("on")).is_some_and(|v| v.as_bool() != Some(true)) {
        return None;
    }
    let layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
    if !valid(&layout) || layout.contains(&panel.to_owned()) {
        return None;
    }
    let remembered = app.ui.docking.as_ref().and(app.ui.docking_hidden.get(panel)).and_then(|at| at.floating);
    let [x, y, width, height] = remembered.unwrap_or_else(|| open_slot(&layout, app.last_canvas_rect));
    let result = change(app, "float", &json!({"panel": panel, "x": x, "y": y, "width": width, "height": height}));
    if result.is_ok() {
        reveal_group(app, panel);
    }
    Some(result)
}

pub(crate) fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let layout = app.ui.docking.as_ref()?;
    let panel = id.strip_prefix("window.panel.").or_else(|| id.strip_prefix("window.toggle.")).and_then(normalize)?;
    Some(layout.contains(&panel.to_string()) || app.ui.rail_panels.iter().any(|p| p == panel))
}

// UI output and automation share this atomic dispatcher. The application owns the registry
// and saved return locations; craft-ui owns tree validation and structural changes.
fn apply_action(app: &mut PhotocraftApp, action: Action<String>) -> Result<Value, String> {
    if app.session.prefs().workspace_locked && !matches!(action, Action::Activate { .. }) {
        return Err("workspace is locked".into());
    }
    let registered = |id: &String| normalize(id) == Some(id.as_str());
    let known = match &action {
        Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } => {
            registered(panel)
                && registered(anchor)
                && match placement {
                    Placement::Tab { before: Some(id) } => registered(id),
                    _ => true,
                }
        }
        Action::Open { panel, anchor } => registered(panel) && anchor.as_ref().is_none_or(registered),
        Action::Float { panel, .. }
        | Action::Close { panel }
        | Action::Activate { panel }
        | Action::MoveFloating { panel, .. }
        | Action::SetStackOpen { panel, .. }
        | Action::ResizeStack { panel, .. } => registered(panel),
        Action::ResizeSplit { path, .. } => path.len() <= craft_ui::docking::MAX_DEPTH,
    };
    if !known {
        return Err("docking action contains an unknown panel or an excessive split path".into());
    }
    if app.ui.docking.as_ref().is_some_and(|layout| !valid(layout)) {
        return Err("saved panel layout is invalid; reset the workspace first".into());
    }
    let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
    let returning = match &action {
        Action::Close { panel } => layout.location(panel).ok().map(|location| (panel.clone(), location)),
        Action::Float { panel, .. } => layout.location(panel).ok().filter(|location| location.floating.is_none()).map(|location| (panel.clone(), location)),
        _ => None,
    };
    let reveal = match &action {
        Action::Activate { panel } => Some(panel.clone()),
        _ => None,
    };
    let activate = match &action {
        Action::Activate { panel } => PANELS.iter().find(|p| p.0 == panel).map(|p| p.2),
        _ => None,
    };
    layout.apply(action).map_err(|error| error.to_string())?;
    if let Some(group) = activate {
        app.ui.dock.set_collapsed(group, false);
    }
    if let Some((panel, location)) = returning {
        app.ui.docking_hidden.insert(panel, location);
    }
    if app.ui.docking.is_none() {
        app.ui.docking_hidden.extend(migration_hidden(app, &layout));
    }
    app.ui.docking = Some(layout);
    app.ui.panels.dock = true;
    sync_visibility(app);
    if let Some(panel) = reveal {
        app.ui.docking_reveal = Some(panel.clone());
        reveal_group(app, &panel);
    }
    Ok(json!({"layout": app.ui.docking}))
}

fn change(app: &mut PhotocraftApp, operation: &str, params: &Value) -> Result<Value, String> {
    if app.session.prefs().workspace_locked && operation != "activate" {
        return Err("workspace is locked".into());
    }
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
    if app.ui.docking.as_ref().is_some_and(|layout| !valid(layout)) {
        return Err("saved panel layout is invalid; reset the workspace first".into());
    }
    let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
    let migrated = if app.ui.docking.is_none() { migration_hidden(app, &layout) } else { app.ui.docking_hidden.clone() };
    let mut hidden: std::collections::BTreeMap<_, _> =
        migrated.iter().filter(|(id, _)| normalize(id).is_some()).take(64).map(|(id, at)| (id.clone(), at.clone())).collect();
    let mut restored = false;
    if operation == "dock"
        && params.get("anchor").or_else(|| params.get("onto")).is_none()
        && let Some(location) = hidden.get(panel).filter(|location| {
            location.floating.is_none()
                && location.anchor.as_ref().is_none_or(|anchor| Layout { root: layout.root.clone(), floating: Vec::new() }.contains(anchor))
        })
    {
        let mut candidate = layout.clone();
        if candidate.contains(&panel.to_string()) {
            candidate.apply(Action::Close { panel: panel.into() }).map_err(|e| e.to_string())?;
        }
        if candidate.restore(panel.into(), location).is_ok() {
            layout = candidate;
            restored = true;
            hidden.remove(panel);
        }
    }
    match operation {
        "dock" if restored => {}
        "float" => {
            let rect = [number(params, "x", 400.0)?, number(params, "y", 140.0)?, number(params, "width", 300.0)?, number(params, "height", 400.0)?];
            ensure(&mut layout, panel)?;
            if let Ok(location) = layout.location(&panel.to_string())
                && location.floating.is_none()
            {
                hidden.insert(panel.into(), location);
            }
            layout.apply(Action::Float { panel: panel.into(), rect }).map_err(|e| e.to_string())?;
        }
        "move" | "dock" => {
            if operation == "move" && params.get("anchor").or_else(|| params.get("onto")).is_none() {
                return Err("move requires a destination panel".into());
            }
            let anchor = match params.get("anchor").or_else(|| params.get("onto")) {
                Some(v) => Some(v.as_str().and_then(normalize).ok_or("anchor must name a registered panel")?.to_string()),
                None => first_root(&layout, panel),
            };
            let placement = placement(params)?;
            if let Some(anchor) = anchor {
                ensure(&mut layout, panel)?;
                layout.apply(Action::Move { panel: panel.into(), anchor, placement }).map_err(|e| e.to_string())?;
            } else if operation == "dock" {
                if layout.contains(&panel.to_string()) {
                    layout.apply(Action::Close { panel: panel.into() }).map_err(|e| e.to_string())?;
                }
                layout.apply(Action::Open { panel: panel.into(), anchor: None }).map_err(|e| e.to_string())?;
            } else {
                return Err("move requires an existing destination panel".into());
            }
        }
        "close" => {
            hidden.insert(panel.into(), layout.location(&panel.to_string()).map_err(|e| e.to_string())?);
            layout.apply(Action::Close { panel: panel.into() }).map_err(|e| e.to_string())?;
        }
        _ => {
            if !layout.contains(&panel.to_string()) && hidden.contains_key(panel) {
                // A missing neighbor is allowed: the normal visible dock becomes the fallback.
                restore_hidden(&mut layout, &mut hidden, panel);
            }
            ensure(&mut layout, panel)?;
            layout.apply(Action::Activate { panel: panel.into() }).map_err(|e| e.to_string())?;
            hidden.remove(panel);
        }
    }
    app.ui.docking = Some(layout);
    app.ui.docking_hidden = hidden;
    app.ui.panels.dock = true;
    sync_visibility(app);
    if operation == "activate"
        && let Some((_, _, group)) = PANELS.iter().find(|p| p.0 == panel)
    {
        app.ui.dock.set_collapsed(*group, false);
        app.ui.docking_reveal = Some(panel.to_owned());
        reveal_group(app, panel);
    }
    Ok(json!({"panel": panel, "layout": app.ui.docking}))
}

pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let area = egui::Id::new("photocraft-panel-docking");
    let stale_origin =
        ui.ctx().data_mut(|data| data.get_temp::<LegacyDrag>(area.with("legacy-origin")).is_some_and(|origin| origin.generation != app.ui.docking_generation));
    let changed_workspace = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<u64>(area.with("generation"));
        data.insert_temp(area.with("generation"), app.ui.docking_generation);
        previous.is_some_and(|generation| generation != app.ui.docking_generation)
    });
    if changed_workspace || stale_origin {
        ui.ctx().data_mut(|data| data.insert_temp(area.with("legacy-suppressed"), true));
        craft_ui::docking::cancel_drag::<String>(ui.ctx(), area);
        ui.ctx().data_mut(|data| data.remove::<LegacyDrag>(area.with("legacy-origin")));
    }

    let cancelled =
        ui.input(|input| !input.focused || input.key_pressed(egui::Key::Escape) || (!input.pointer.primary_down() && !input.pointer.any_released()));
    if cancelled && cancel_legacy_drag(app, ui.ctx()) {
        ui.ctx().data_mut(|data| data.insert_temp(area.with("legacy-suppressed"), true));
        if app.ui.docking.is_none() {
            return false;
        }
    }
    let Some(layout) = app.ui.docking.as_ref() else { return false };
    if !valid(layout) {
        app.ui.docking = None;
        let message = "Invalid panel layout; restored the default dock";
        app.ui.status = tl!(message).into();
        return false;
    }
    let layout = layout.clone();
    let reveal = app.ui.docking_reveal.take();
    // Groups in their own OS windows are drawn there, not inside the app window. Where windows
    // can't be made (the browser), they stay floating in the page.
    let windows = if ui.ctx().embed_viewports() { Vec::new() } else { separate_windows(app, &layout) };
    let mut drawn = layout.clone();
    drawn.floating.retain(|group| !windows.iter().any(|key| group.panels.contains(key)));
    let t = crate::theme::Tokens::get(ui.ctx());
    let locked = app.session.prefs().workspace_locked;
    let collapsed = collapsed_members(app, &layout);
    let mut requested_width = ui.ctx().data_mut(|data| data.remove_temp::<f32>(crate::panels::dock_width_id()));
    // A panel docked beside a group adds a column: the dock grows by one instead of halving its
    // groups, and shrinks back when a column leaves (whatever moved it: drag, menu or command).
    let columns = column_count(layout.root.as_ref());
    let remembered = area.with("columns");
    if let Some((before, width)) = ui.ctx().data_mut(|data| data.get_temp::<(usize, f32)>(remembered))
        && requested_width.is_none()
        && !changed_workspace
        && before > 0
        && columns > 0
        && before != columns
    {
        let grown = width + (columns as f32 - before as f32) * COLUMN_WIDTH;
        let range = dock_width_range(columns, ui.ctx().content_rect().width());
        requested_width = Some(grown.clamp(*range.start(), *range.end()));
    }
    let mut commands = Vec::new();
    let mut targets = Vec::new();
    let rail = ui.ctx().data(|d| d.get_temp::<egui::Rect>(rail_rect_id())).filter(|_| app.ui.panels.rail && !locked);
    let mut draw = |ui: &mut egui::Ui| {
        let mut style = craft_ui::docking::DockStyle::from_ui(ui);
        style.tab_height = if t.pro { 28.0 } else { 40.0 };
        style.min_pane = 0.0;
        style.background = t.card;
        // Studio themes leave the strip transparent over the dock's card; a floating group has no
        // card behind it, so the canvas showed through its titles (#4509).
        style.tab_background = if t.tab_strip.a() == 0 { t.card } else { t.tab_strip };
        style.active_background = t.card;
        style.text = t.text;
        style.inactive_text = t.text_dim;
        style.border = egui::Stroke::new(1.0, t.card_border);
        style.accent = t.accent;
        style.float_label = tl!("Float Panel").to_string();
        style.close_label = tl!("Close Panel").to_string();
        style.move_label = tl!("Group with").to_string();
        style.panels_label = tl!("Panels").to_string();
        style.resize_label = tl!("Resize panels").to_string();
        style.resize_window_label = tl!("Resize panel window").to_string();
        let mut area = craft_ui::docking::DockArea::new(egui::Id::new("photocraft-panel-docking"));
        // A panel dropped on the icon bar docks to it (`rail_drop`), not into the layout.
        if let Some(rail) = rail {
            area = area.blocked_rects(&[rail]);
        }
        if let Some(panel) = &reveal {
            area = area.raise_panel(panel.clone());
        }
        area.show_customized(
            ui,
            &drawn,
            &style,
            |id| PANELS.iter().find(|p| p.0 == id).map(|p| tl!(p.1).to_string()).unwrap_or_else(|| id.clone()),
            |_| craft_ui::docking::Permissions { movable: !locked, floatable: !locked, closable: !locked, accepts_tabs: !locked },
            |id| {
                if collapsed.contains(id) {
                    craft_ui::docking::PanelLimits { min: egui::vec2(220.0, 0.0), max: egui::vec2(1_000_000.0, 0.0) }
                } else {
                    // Layers owns its row scroll and footer; leave room for its fixed controls
                    // plus a scrolling row area instead of clipping the footer after a split.
                    let height = if id == "layers" { 220.0 } else { 80.0 };
                    craft_ui::docking::PanelLimits { min: egui::vec2(220.0, height), ..Default::default() }
                }
            },
            &mut PanelContent { app, collapsed: &collapsed, commands: &mut commands, targets: &mut targets },
        )
    };
    let output = if layout.root.is_some() {
        let mut panel = egui::Panel::right("shared-panel-dock")
            .default_size(if t.pro { 290.0 } else { 300.0 })
            .size_range(dock_width_range(columns, ui.ctx().content_rect().width()))
            .resizable(!locked)
            .frame(egui::Frame::NONE.fill(t.card));
        if let Some(width) = requested_width {
            panel = panel.exact_size(width);
        }
        let shown = panel.show(ui, &mut draw);
        let width = shown.response.rect.width();
        ui.ctx().data_mut(|data| data.insert_temp(remembered, (columns, width)));
        shown.inner
    } else {
        draw(ui)
    };
    if let Some(error) = output.error {
        app.ui.status = error.to_string();
    }
    // Every frame, before any early return: a viewport skipped for a frame closes.
    for key in &windows {
        show_window(app, ui.ctx(), key, reveal.as_ref());
    }
    if !locked {
        expand_toward_drag(app, ui.ctx(), &drawn, &collapsed);
        if rail.is_some() {
            rail_drop(app, ui.ctx(), &drawn);
        }
    }
    if ui.input(|input| input.pointer.any_released()) {
        if !output.actions.iter().any(|action| matches!(action, Action::Move { .. } | Action::Float { .. })) && cancel_legacy_drag(app, ui.ctx()) {
            return app.ui.docking.is_some();
        }
        ui.ctx().data_mut(|data| data.remove::<LegacyDrag>(egui::Id::new("photocraft-panel-docking").with("legacy-origin")));
    }
    if let Some(panel) = output.group_drag.as_ref()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        let members = group_members(&layout, panel);
        let eligible: Vec<_> = targets.iter().filter(|target| !members.contains(&target.anchor)).cloned().collect();
        if let Some(proposal) = craft_ui::docking::resolve_drop(panel, pointer, &eligible) {
            ui.painter().rect_stroke(proposal.preview, 0.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
    }
    if let Some((panel, pointer)) = output.group_drop {
        let members = group_members(&layout, &panel);
        let eligible: Vec<_> = targets.iter().filter(|target| !members.contains(&target.anchor)).cloned().collect();
        if let Some(proposal) = craft_ui::docking::resolve_drop(&panel, pointer, &eligible) {
            commands.push(("window.panel.group", json!({"panel":panel,"operation":"move","anchor":proposal.anchor,"placement":proposal.placement})));
        }
    }
    for action in output.actions {
        if let Err(error) = app.run("window.panel.layout", json!({"action": action})) {
            app.ui.status = error;
        }
    }
    for (id, params) in commands {
        if let Err(error) = app.run(id, params) {
            app.ui.status = error;
        }
    }
    crate::dock::persist(app, ui.ctx());
    true
}

pub(crate) fn legacy_tab(app: &mut PhotocraftApp, ui: &mut egui::Ui, panel: &str, response: &egui::Response) {
    let suppressed = egui::Id::new("photocraft-panel-docking").with("legacy-suppressed");
    if ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary)) {
        ui.ctx().data_mut(|data| data.remove::<bool>(suppressed));
    }
    if ui.ctx().data(|data| data.get_temp::<bool>(suppressed)).unwrap_or(false) || app.session.prefs().workspace_locked {
        return;
    }
    let Some(panel) = normalize(panel) else { return };
    // Native tab reordering and vertical whole-group moves stay in the dock.
    // Crossing the dock column horizontally detaches through the shared layout.
    let strip = egui::Rect::from_min_max(egui::pos2(ui.max_rect().left(), response.rect.top()), egui::pos2(ui.max_rect().right(), response.rect.bottom()));
    if response.dragged()
        && ui.input(|input| input.focused && input.pointer.primary_down() && !input.key_pressed(egui::Key::Escape))
        && ui.ctx().pointer_interact_pos().is_some_and(|position| position.x < strip.left() - 24.0 || position.x > strip.right() + 24.0)
        && let Some(id) = normalize(panel)
    {
        let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
        let bounds = ui.max_rect();
        let source = egui::Rect::from_min_size(
            egui::pos2(bounds.left(), response.rect.top()),
            egui::vec2(bounds.width().clamp(240.0, 600.0), bounds.height().clamp(200.0, 600.0)),
        );
        if ensure(&mut layout, id).is_ok() && craft_ui::docking::begin_drag(ui.ctx(), egui::Id::new("photocraft-panel-docking"), id.to_string(), source) {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("photocraft-panel-docking").with("legacy-origin"),
                    LegacyDrag {
                        generation: app.ui.docking_generation,
                        layout: app.ui.docking.clone(),
                        tabs: app.ui.dock_tabs,
                        hidden: app.ui.docking_hidden.clone(),
                    },
                )
            });
            if app.ui.docking.is_none() {
                app.ui.docking_hidden = migration_hidden(app, &layout);
            }
            app.ui.docking = Some(layout);
            // This drag may end on the icon bar (`rail_drop`).
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(response.rect.center());
            ui.ctx().data_mut(|d| d.insert_temp(rail_drag_id(), (id.to_string(), false, origin)));
        }
    }
}

fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: &str) {
    let Some((_, label, group)) = PANELS.iter().find(|p| p.0 == id) else { return };
    let pro = crate::theme::Tokens::get(ui.ctx()).pro;
    let Some(tab) = group.tabs(pro).iter().position(|name| name == label) else { return };
    *group.tab_mut(&mut app.ui.dock_tabs) = tab;
    if group.scrolls_itself(tab) {
        crate::panels::dock_body(app, ui, *group, tab);
    } else {
        crate::panels::offer_color_field_fill(ui, pro, *group, tab);
        egui::ScrollArea::vertical()
            .id_salt(("shared-panel-body", id))
            .auto_shrink([false, false])
            .show(ui, |ui| crate::panels::dock_body(app, ui, *group, tab));
    }
    let selected = *group.tab_mut(&mut app.ui.dock_tabs);
    if selected != tab
        && let Some(label) = group.tabs(pro).get(selected)
    {
        let _ = app.run("window.panel.activate", json!({"panel":label}));
    }
}

#[cfg(test)]
#[path = "panel_docking_tests.rs"]
mod tests;

/// Return positions for panels already hidden in a legacy workspace.
fn migration_hidden(app: &PhotocraftApp, visible: &Layout<String>) -> std::collections::BTreeMap<String, craft_ui::docking::Location<String>> {
    let full = legacy_layout(app, true);
    PANELS
        .iter()
        .filter(|(id, _, _)| !visible.contains(&id.to_string()))
        .filter_map(|(id, _, _)| full.location(&id.to_string()).ok().map(|at| ((*id).to_owned(), at)))
        .collect()
}

fn group_members(layout: &Layout<String>, panel: &str) -> Vec<String> {
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Split { first, second, .. } => pending.extend([second.as_ref(), first.as_ref()]),
            Node::Tabs { panels, .. } if panels.iter().any(|id| id == panel) => return panels.clone(),
            Node::Stack { entries } if entries.iter().any(|entry| entry.panel == panel) => return vec![panel.to_owned()],
            _ => {}
        }
    }
    layout.floating.iter().find(|group| group.panels.iter().any(|id| id == panel)).map(|group| group.panels.clone()).unwrap_or_default()
}

fn reveal_group(app: &mut PhotocraftApp, panel: &str) {
    if let Some(layout) = &app.ui.docking {
        let members = group_members(layout, panel);
        app.ui.docking_collapsed.retain(|id| !members.contains(id));
        for id in members {
            if let Some((_, _, group)) = PANELS.iter().find(|p| p.0 == id) {
                app.ui.dock.set_collapsed(*group, false);
            }
        }
    }
}

pub(crate) fn rail_on(app: &PhotocraftApp, group: Group) -> bool {
    app.ui.docking.as_ref().is_some_and(|layout| {
        let collapsed = collapsed_members(app, layout);
        PANELS.iter().filter(|p| p.2 == group).any(|p| layout.contains(&p.0.to_owned()) && !collapsed.contains(p.0))
    })
}

pub(crate) fn rail_click(app: &mut PhotocraftApp, group: Group) {
    let Some(layout) = app.ui.docking.as_ref() else { return };
    // A group that lives only on the icon bar opens there, rather than docking a second copy.
    if !PANELS.iter().any(|p| p.2 == group && layout.contains(&p.0.to_owned()))
        && let Some(panel) = PANELS.iter().find(|p| p.2 == group && app.ui.rail_panels.iter().any(|r| r == p.0))
    {
        app.ui.rail_open = if app.ui.rail_open.as_deref() == Some(panel.0) { None } else { Some(panel.0.to_owned()) };
        return;
    }
    let floating = PANELS.iter().filter(|p| p.2 == group).any(|p| layout.floating.iter().any(|g| g.panels.iter().any(|id| id == p.0)));
    let existing = PANELS.iter().find(|p| p.2 == group && layout.contains(&p.0.to_owned())).map(|p| p.0);
    let panel = existing.or_else(|| {
        group
            .tabs(matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium))
            .get(*group.tab_mut(&mut app.ui.dock_tabs))
            .and_then(|label| normalize(label))
    });
    if let Some(panel) = panel {
        let result = if existing.is_some() && rail_on(app, group) {
            if floating {
                group_command(app, &json!({"panel": panel, "operation":"close"}))
            } else {
                group_command(app, &json!({"panel": panel, "operation":"collapse"}))
            }
        } else {
            change(app, "activate", &json!({"panel":panel}))
        };
        if let Err(error) = result {
            app.ui.status = error;
        }
    }
}

fn root_groups(layout: &Layout<String>) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Split { first, second, .. } => pending.extend([second.as_ref(), first.as_ref()]),
            Node::Tabs { panels, .. } => out.push(panels.clone()),
            Node::Stack { entries } => out.extend(entries.iter().map(|e| vec![e.panel.clone()])),
        }
    }
    out
}

fn group_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?;
    let operation = params.get("operation").and_then(Value::as_str).ok_or("operation is required")?;
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let mut layout = app.ui.docking.clone().unwrap_or_else(|| legacy(app));
    if !valid(&layout) {
        return Err("invalid panel layout".into());
    }
    let members = group_members(&layout, panel);
    if members.is_empty() {
        return Err("panel is hidden".into());
    }
    let mut hidden = if app.ui.docking.is_none() { migration_hidden(app, &layout) } else { app.ui.docking_hidden.clone() };
    let mut collapsed = collapsed_members(app, &layout);
    match operation {
        "close" => {
            for id in &members {
                hidden.insert(id.clone(), layout.location(id).map_err(|e| e.to_string())?);
                layout.apply(Action::Close { panel: id.clone() }).map_err(|e| e.to_string())?;
            }
            collapsed.retain(|id| !members.contains(id));
        }
        "collapse" => {
            collapsed.extend(members.iter().cloned());
        }
        "expand" => {
            collapsed.retain(|id| !members.contains(id));
        }
        "move" => {
            let anchor = params.get("anchor").and_then(Value::as_str).and_then(normalize).ok_or("anchor must name a registered panel")?;
            let placement = params
                .get("placement")
                .map(|v| serde_json::from_value::<Placement<String>>(v.clone()).map_err(|e| e.to_string()))
                .unwrap_or_else(|| placement(params))?;
            if members.iter().any(|id| id == anchor) {
                return Err("group cannot be moved into itself".into());
            }
            let first = members.first().ok_or("group is empty")?;
            layout.apply(Action::Move { panel: first.clone(), anchor: anchor.to_owned(), placement }).map_err(|e| e.to_string())?;
            let destination = group_members(&layout, first);
            let before = destination.iter().position(|id| id == first).and_then(|index| destination.get(index + 1)).cloned();
            for id in members.iter().skip(1) {
                layout
                    .apply(Action::Move { panel: id.clone(), anchor: first.clone(), placement: Placement::Tab { before: before.clone() } })
                    .map_err(|e| e.to_string())?;
            }
            layout.apply(Action::Activate { panel: panel.to_owned() }).map_err(|e| e.to_string())?;
        }
        "up" | "down" => {
            let groups = root_groups(&layout);
            let index = groups.iter().position(|g| g.contains(&panel.to_owned())).ok_or("floating groups cannot move within the dock")?;
            let neighbor = if operation == "up" { index.checked_sub(1) } else { index.checked_add(1).filter(|i| *i < groups.len()) };
            let anchor = neighbor.and_then(|i| groups.get(i)).and_then(|g| g.first()).ok_or("group is already at the edge")?;
            let first = members.first().ok_or("group is empty")?;
            layout
                .apply(Action::Move {
                    panel: first.clone(),
                    anchor: anchor.clone(),
                    placement: Placement::Split(if operation == "up" { Zone::Top } else { Zone::Bottom }),
                })
                .map_err(|e| e.to_string())?;
            for id in members.iter().skip(1) {
                layout
                    .apply(Action::Move { panel: id.clone(), anchor: first.clone(), placement: Placement::Tab { before: None } })
                    .map_err(|e| e.to_string())?;
            }
            layout.apply(Action::Activate { panel: panel.to_owned() }).map_err(|e| e.to_string())?;
        }
        _ => return Err("operation must be close, collapse, expand, move, up or down".into()),
    }
    app.ui.docking = Some(layout);
    app.ui.docking_hidden = hidden;
    app.ui.docking_collapsed = collapsed.into_iter().collect();
    sync_visibility(app);
    // Native collapse flags must not re-collapse a custom group after explicit expansion.
    if operation == "expand" || operation == "move" {
        reveal_group(app, panel);
    }
    Ok(json!({"layout":app.ui.docking}))
}

pub(crate) struct LegacyUpdate {
    layout: Layout<String>,
    hidden: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
    clear_collapse: Vec<Group>,
}

/// Validate and stage the legacy controls before ui.set commits any fields.
pub(crate) fn prepare_legacy_update(
    app: &PhotocraftApp,
    request: &Value,
    panels: Option<&crate::state::Panels>,
    tabs: Option<&crate::state::DockTabs>,
    dock: Option<&crate::dock::DockLayout>,
    pro: bool,
) -> Result<Option<LegacyUpdate>, String> {
    let Some(current) = &app.ui.docking else { return Ok(None) };
    if panels.is_none() && tabs.is_none() && dock.is_none() {
        return Ok(None);
    }
    if !valid(current) {
        return Err("invalid panel layout; reset the workspace first".into());
    }
    // A legacy flat-column replacement cannot represent arbitrary custom splits. Reject it
    // explicitly instead of accepting a command whose requested order/height is ignored.
    if let Some(dock) = dock
        && (dock.order != app.ui.dock.order || dock.heights != app.ui.dock.heights)
    {
        return Err("dock order/heights cannot replace a customized layout; use window.panel.layout".into());
    }
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let clear_collapse: Vec<_> =
        dock.map(|d| Group::ALL.into_iter().filter(|g| d.is_collapsed(*g) != app.ui.dock.is_collapsed(*g)).collect()).unwrap_or_default();
    if !clear_collapse.is_empty()
        && root_groups(current).iter().any(|ids| {
            let groups: std::collections::HashSet<_> = ids.iter().filter_map(|id| PANELS.iter().find(|p| p.0 == id).map(|p| p.2)).collect();
            groups.len() > 1
        })
    {
        return Err("legacy collapse settings cannot replace mixed custom groups; use window.panel.group".into());
    }
    let mut layout = current.clone();
    let mut hidden = app.ui.docking_hidden.clone();
    for group in Group::ALL {
        let visibility = request.get("panels").and_then(|p| p.get(group.key())).and_then(Value::as_bool);
        let selected = request.get("dockTabs").and_then(|p| p.get(group.key())).is_some();
        let group_panels: Vec<_> = PANELS.iter().filter(|p| p.2 == group).collect();
        for &&(id, label, _) in &group_panels {
            let explicitly_hidden = dock.is_some_and(|d| d.hidden_tabs.get(&group).is_some_and(|names| names.iter().any(|name| name == label)));
            let explicitly_shown = dock.is_some() && !explicitly_hidden;
            if visibility == Some(false) || explicitly_hidden {
                if layout.contains(&id.to_owned()) {
                    hidden.insert(id.to_owned(), layout.location(&id.to_owned()).map_err(|e| e.to_string())?);
                    layout.apply(Action::Close { panel: id.to_owned() }).map_err(|e| e.to_string())?;
                }
            } else if (visibility == Some(true) || explicitly_shown) && !layout.contains(&id.to_owned()) {
                if hidden.contains_key(id) {
                    restore_hidden(&mut layout, &mut hidden, id);
                }
                ensure(&mut layout, id)?;
            }
        }
        if selected {
            let tabs = tabs.ok_or("dockTabs is missing")?;
            let index = match group {
                Group::Color => tabs.color,
                Group::Properties => tabs.properties,
                Group::Character => tabs.character,
                Group::Navigator => tabs.navigator,
                Group::History => tabs.history,
                Group::Layers => tabs.layers,
                Group::Brushes => tabs.brushes,
            };
            let id = group.tabs(pro).get(index).and_then(|label| normalize(label)).ok_or("dockTabs index is outside the panel group")?;
            if visibility == Some(false) {
                return Err("cannot select a tab in a hidden group".into());
            }
            if !layout.contains(&id.to_owned()) {
                if hidden.contains_key(id) {
                    restore_hidden(&mut layout, &mut hidden, id);
                }
                ensure(&mut layout, id)?;
            }
            layout.apply(Action::Activate { panel: id.to_owned() }).map_err(|e| e.to_string())?;
        }
    }
    Ok(Some(LegacyUpdate { layout, hidden, clear_collapse }))
}

pub(crate) fn commit_legacy_update(app: &mut PhotocraftApp, update: LegacyUpdate, request: &Value) {
    for group in update.clear_collapse {
        let ids: Vec<_> = PANELS.iter().filter(|p| p.2 == group).map(|p| p.0).collect();
        app.ui.docking_collapsed.retain(|id| !ids.contains(&id.as_str()));
    }
    app.ui.docking = Some(update.layout);
    app.ui.docking_hidden = update.hidden;
    sync_visibility(app);
    for group in Group::ALL {
        if request.get("dockTabs").and_then(|p| p.get(group.key())).is_some()
            || request.get("panels").and_then(|p| p.get(group.key())).and_then(Value::as_bool) == Some(true)
        {
            let ids: Vec<_> = PANELS.iter().filter(|p| p.2 == group).map(|p| p.0).collect();
            app.ui.docking_collapsed.retain(|id| !ids.contains(&id.as_str()));
            app.ui.dock.set_collapsed(group, false);
        }
    }
}

struct PanelContent<'a> {
    app: &'a mut PhotocraftApp,
    collapsed: &'a std::collections::HashSet<String>,
    commands: &'a mut Vec<(&'static str, Value)>,
    targets: &'a mut Vec<craft_ui::docking::DropTarget<String>>,
}

fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, panel: &str) {
    match panel {
        "layers" => {
            crate::layer_row_ui::panel_menu(app, ui);
            ui.separator();
        }
        "swatches" => {
            crate::swatches_ui::panel_menu(app, ui);
            ui.separator();
        }
        "color" => {
            crate::color_panel_ui::panel_menu(app, ui);
            ui.separator();
        }
        "history" => {
            crate::history_menu_ui::panel_menu(app, ui);
            ui.separator();
        }
        "brushSettings" | "brush_settings" => {
            crate::brush_panel::panel_menu(app, ui);
            ui.separator();
        }
        "actions" | "Actions" => {
            crate::actions::panel_menu(app, ui);
            ui.separator();
        }
        _ => {}
    }
}

impl craft_ui::docking::DockContent<String> for PanelContent<'_> {
    fn body(&mut self, ui: &mut egui::Ui, id: &String) {
        if let Some(target) = self.targets.iter_mut().find(|target| target.anchor == *id) {
            target.rect = target.rect.union(ui.max_rect());
            if let Some(strip) = target.tab_strip.as_mut() {
                strip.min.x = target.rect.left();
                strip.max.x = target.rect.right();
            }
            let members = self.app.ui.docking.as_ref().map(|layout| group_members(layout, id)).unwrap_or_default();
            target.tabs = members
                .into_iter()
                .filter_map(|panel| {
                    ui.ctx().read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &panel))).map(|response| (panel, response.rect))
                })
                .collect();
        }
        if !self.collapsed.contains(id) {
            egui::Frame::NONE.inner_margin(egui::Margin::same(8)).show(ui, |ui| body(self.app, ui, id));
        }
    }
    fn panel_menu(&mut self, ui: &mut egui::Ui, id: &String) {
        panel_menu(self.app, ui, id);
    }
    fn group_header_width(&self, _panels: &[String], _active: &String, _floating: bool) -> f32 {
        28.0
    }
    fn group_header(&mut self, ui: &mut egui::Ui, panels: &[String], active: &String, floating: bool) {
        if !floating {
            let rect = ui.max_rect();
            self.targets.push(craft_ui::docking::DropTarget {
                anchor: active.clone(),
                rect,
                tab_strip: Some(rect),
                tabs: Vec::new(),
                allow_splits: true,
                accepts_tabs: !self.app.session.prefs().workspace_locked,
            });
        }
        let collapsed = panels.iter().all(|id| self.collapsed.contains(id));
        let locked = self.app.session.prefs().workspace_locked;
        let resp = crate::icons::button(ui, "ellipsis", ui.available_height().min(28.0), false, tl!("Panel menu"));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(180.0);
            panel_menu(self.app, ui, active);
            if ui.add_enabled(!locked, egui::Button::new(if collapsed { tl!("Expand Panel Group") } else { tl!("Collapse Panel Group") })).clicked() {
                self.commands.push(("window.panel.group", json!({"panel":active,"operation":if collapsed {"expand"} else {"collapse"}})));
                ui.close();
            }
            if ui.add_enabled(!locked, egui::Button::new(if floating { tl!("Dock Panel") } else { tl!("Float Panel") })).clicked() {
                self.commands.push((if floating { "window.panel.dock" } else { "window.panel.float" }, json!({"panel":active})));
                ui.close();
            }
            // Its own OS window, free to leave PhotoCraft's (another monitor); not in a browser.
            if !ui.ctx().embed_viewports() && ui.add_enabled(!locked, egui::Button::new(tl!("Move to Separate Window"))).clicked() {
                self.commands.push(("window.panel.detach", json!({"panel": active})));
                ui.close();
            }
            if self.app.ui.panels.rail && ui.add_enabled(!locked, egui::Button::new(tl!("Dock to Icon Bar"))).clicked() {
                self.commands.push(("window.panel.toRail", json!({"panel": active, "group": true})));
                ui.close();
            }
            if !floating {
                let groups = self.app.ui.docking.as_ref().map(root_groups).unwrap_or_default();
                let index = groups.iter().position(|g| g.contains(active));
                for (operation, label, enabled) in
                    [("up", tl!("Move Group Up"), index.is_some_and(|i| i > 0)), ("down", tl!("Move Group Down"), index.is_some_and(|i| i + 1 < groups.len()))]
                {
                    if ui.add_enabled(!locked && enabled, egui::Button::new(label)).clicked() {
                        self.commands.push(("window.panel.group", json!({"panel":active,"operation":operation})));
                        ui.close();
                    }
                }
            }
            ui.separator();
            if ui.add_enabled(!locked, egui::Button::new(tl!("Close Tab Group"))).clicked() {
                self.commands.push(("window.panel.group", json!({"panel":active,"operation":"close"})));
                ui.close();
            }
        });
    }
}

pub(crate) fn sync_visibility(app: &mut PhotocraftApp) {
    // A panel brought back into the layout (Window menu, a workspace, a command) leaves the bar.
    if let Some(layout) = &app.ui.docking {
        app.ui.rail_panels.retain(|id| !layout.contains(id));
        if app.ui.rail_open.as_ref().is_some_and(|open| !app.ui.rail_panels.contains(open)) {
            app.ui.rail_open = None;
        }
    }
    if let Some(layout) = &app.ui.docking {
        if !valid(layout) {
            return;
        }
        let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
        for group in Group::ALL {
            *group.shown_mut(&mut app.ui.panels) = PANELS.iter().filter(|p| p.2 == group).any(|p| layout.contains(&p.0.to_owned()));
            for (index, label) in group.tabs(pro).iter().enumerate() {
                if let Some(id) = normalize(label) {
                    if layout.contains(&id.to_owned()) {
                        app.ui.dock.show_tab(group, index, pro);
                    } else {
                        app.ui.dock.hide_tab(group, index, pro);
                    }
                }
            }
            let previous = *group.tab_mut(&mut app.ui.dock_tabs);
            if !group.tabs(pro).get(previous).and_then(|name| normalize(name)).is_some_and(|id| panel_active(layout, id))
                && let Some(selected) = group.tabs(pro).iter().position(|name| normalize(name).is_some_and(|id| panel_active(layout, id)))
            {
                *group.tab_mut(&mut app.ui.dock_tabs) = selected;
            }
        }
    }
}

fn panel_active(layout: &Layout<String>, panel: &str) -> bool {
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Split { first, second, .. } => pending.extend([first.as_ref(), second.as_ref()]),
            Node::Tabs { panels, active } if panels.iter().any(|p| p == panel) => return panels.get(*active).is_some_and(|p| p == panel),
            Node::Stack { entries } if entries.iter().any(|e| e.panel == panel) => return true,
            _ => {}
        }
    }
    layout.floating.iter().any(|g| g.panels.get(g.active).is_some_and(|p| p == panel))
}

/// Extra legacy menu choices use the same commands as the customized renderer.
pub(crate) fn legacy_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, label: &str) {
    let Some(panel) = normalize(label) else { return };
    let locked = app.session.prefs().workspace_locked;
    ui.add_enabled_ui(!locked, |ui| {
        if ui.button(tl!("Float Panel")).clicked() {
            if let Err(error) = app.run("window.panel.float", json!({"panel":panel})) {
                app.ui.status = error;
            }
            ui.close();
        }
        if !ui.ctx().embed_viewports() && ui.button(tl!("Move to Separate Window")).clicked() {
            if let Err(error) = app.run("window.panel.detach", json!({"panel":panel})) {
                app.ui.status = error;
            }
            ui.close();
        }
        if app.ui.panels.rail && ui.button(tl!("Dock to Icon Bar")).clicked() {
            if let Err(error) = app.run("window.panel.toRail", json!({"panel":panel})) {
                app.ui.status = error;
            }
            ui.close();
        }
        ui.menu_button(tl!("Group with"), |ui| {
            for &(anchor, label, _) in PANELS {
                if anchor != panel && ui.button(tl!(label)).clicked() {
                    let result = app
                        .run("window.panel.activate", json!({"panel":anchor}))
                        .and_then(|_| app.run("window.panel.move", json!({"panel":panel,"anchor":anchor})));
                    if let Err(error) = result {
                        app.ui.status = error;
                    }
                    ui.close();
                }
            }
        });
    });
}

// A closed group forms a bounded chain of hidden tab neighbors. Restore just the requested
// panel at the first surviving return position, then let the remaining tabs rejoin it.
fn restore_hidden(layout: &mut Layout<String>, hidden: &mut std::collections::BTreeMap<String, craft_ui::docking::Location<String>>, panel: &str) {
    let Some(mut at) = hidden.get(panel).cloned() else { return };
    let mut terminal = panel.to_owned();
    let mut seen = vec![terminal.clone()];
    while let Some(anchor) = at.anchor.as_ref().filter(|id| !layout.contains(id)).cloned() {
        if seen.len() >= PANELS.len() || seen.contains(&anchor) {
            return;
        }
        let Some(next) = hidden.get(&anchor).cloned() else { return };
        seen.push(anchor.clone());
        terminal = anchor;
        at = next;
    }
    if layout.restore(panel.to_owned(), &at).is_ok()
        && terminal != panel
        && let Some(last) = hidden.get_mut(&terminal)
    {
        last.anchor = Some(panel.to_owned());
        last.placement = Placement::Tab { before: None };
        last.size = None;
        last.sibling.clear();
    }
}
