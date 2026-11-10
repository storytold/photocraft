//! The native dock state as a craft-ui workspace: the one conversion.
//!
//! `crate::dock`'s state is the workspace while nothing has customized it, and `ui.set`,
//! the menus and the saved workspaces all carry its shape. [`legacy_layout`] reads that
//! state as the craft-ui layout it is drawn as; [`materialize`] commits it as a
//! [`Workspace`] once the shell is ready to draw from the workspace alone. Everything
//! downstream, the dispatcher included, then reads and writes the workspace; the native
//! state stays app-owned input the conversion must keep accepting.

use crate::dock::Group;
use craft_ui::docking::{Column, Floating, Layout, Location, Node, Side, Workspace};

use super::{PANELS, PROPERTIES_WINDOW, Returns, commit, default_width, normalize, with_canvas};
use crate::PhotocraftApp;
use craft_ui::docking::Group as CraftGroup;

/// The layout drawn while the workspace has not been materialized yet.
pub(crate) fn legacy(app: &PhotocraftApp) -> Layout<String> {
    legacy_layout(app, false)
}

/// The native dock's arrangement as a craft-ui layout: the canvas beside one dock column of the
/// shown groups, top to bottom, with their tab order, selected tab, user-set heights and
/// collapsed (minimized) groups. The Studio themes float Properties over the canvas's top right
/// corner. `all` includes the groups that are not shown, with every tab.
pub(crate) fn legacy_layout(app: &PhotocraftApp, all: bool) -> Layout<String> {
    let pro = app.ui.theme.is_pro();
    let mut groups = Vec::new();
    let mut floating = Vec::new();
    for group in app.ui.dock.order().into_iter().filter(|g| all || g.shown(&app.ui.panels)) {
        let labels = group.tabs(pro);
        let visible = if all { group.tabs(pro).iter().copied().enumerate().collect() } else { app.ui.dock.visible_tabs(group, pro) };
        let panels: Vec<String> = visible.iter().filter_map(|(_, label)| normalize(label).map(str::to_owned)).collect();
        if panels.is_empty() {
            continue;
        }
        let selected = *group.tab_mut(&mut app.ui.dock_tabs.clone());
        let active = labels.get(selected).and_then(|label| normalize(label)).and_then(|id| panels.iter().position(|p| p == id)).unwrap_or(0);
        if group == Group::Properties && !pro {
            floating.push(Floating::single(panels, active, studio_properties_rect(app)).with_id(PROPERTIES_WINDOW));
            continue;
        }
        let height = app.ui.dock.heights.contains_key(&group).then(|| app.ui.dock.height(group));
        // The native collapse flag is the group's in-place tab-strip minimize; the workspace
        // carries it as `minimized`, so the conversion needs no per-frame remap.
        let minimized = app.ui.dock.is_collapsed(group);
        groups.push(CraftGroup { minimized, height, ..CraftGroup::new(panels, active) });
    }
    let (columns, width) = if groups.is_empty() { (Vec::new(), 0.0) } else { (vec![Column::new(groups, default_width(pro))], default_width(pro)) };
    Layout { root: Some(with_canvas(Node::Dock { side: Side::End, columns }, width)), floating }
}

/// Where the Studio themes' Properties card floats: inside the canvas's top right corner.
fn studio_properties_rect(app: &PhotocraftApp) -> [f32; 4] {
    let (width, height) = (320.0, 440.0);
    let canvas = app.last_canvas_rect;
    if canvas.is_positive() && canvas.width() > width + 24.0 {
        [canvas.right() - width - 12.0, canvas.top() + 44.0, width, height]
    } else {
        [120.0, 160.0, width, height]
    }
}

/// The native dock as a workspace: its shown panels, and where each hidden one goes back.
pub(crate) fn legacy_workspace(app: &PhotocraftApp) -> Workspace<String> {
    let layout = legacy(app);
    let hidden = migration_hidden(app, &layout);
    Workspace { hidden, ..Workspace::new(layout) }
}

/// Return positions for panels already hidden in a legacy workspace.
fn migration_hidden(app: &PhotocraftApp, visible: &Layout<String>) -> Vec<(String, Location<String>)> {
    let full = legacy_layout(app, true);
    PANELS
        .iter()
        .filter(|(id, _, _)| !visible.contains(&id.to_string()))
        .filter_map(|(id, _, _)| full.location(&id.to_string()).ok().map(|at| ((*id).to_owned(), at)))
        .collect()
}

/// Ready for the workspace to take over: the converted layout is final once the Studio themes
/// have a canvas to anchor their Properties card on. Pro themes are ready straight away; a
/// workspace saved by a newer version stays on disk and the default view keeps drawing instead.
pub(crate) fn ready(app: &PhotocraftApp) -> bool {
    if app.ui.docking_kept.is_some() {
        return false;
    }
    // The Studio themes hold Properties as a floating card on the canvas; its position needs
    // the canvas panel's real rect, not the state default (800 × 600) that sits there until the
    // panel has drawn once. Pro themes are ready straight away.
    app.ui.theme.is_pro() || app.drop_canvas_rect.is_some()
}

/// Commit the native dock state as the workspace: the shell draws from it alone from then on.
pub(crate) fn materialize(app: &mut PhotocraftApp) {
    if !ready(app) {
        return;
    }
    if app.ui.docking.is_some() && app.ui.docking_pristine_theme.is_none() {
        // The workspace has been modified since the conversion. A customized layout keeps its
        // shape, like any layout a user arranged.
        return;
    }
    let theme = app.ui.theme;
    let workspace = legacy_workspace(app);
    let returns: Returns = workspace.hidden.iter().filter(|(_, at)| at.floating.is_none()).map(|(id, at)| (id.clone(), at.clone())).collect();
    // The conversion puts the dock up: a workspace exists, so the dock column must draw, the
    // way the native state always had. Later commits don't force it (a `ui.set` may hide it).
    app.ui.panels.dock = true;
    commit(app, workspace, returns);
    app.ui.docking_pristine_theme = Some(theme);
}
