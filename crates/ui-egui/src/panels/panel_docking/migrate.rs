//! The native dock state as a craft-ui workspace: the one conversion.
//!
//! `crate::dock`'s state is the workspace while nothing has customized it, and `ui.set`,
//! the menus and the saved workspaces all carry its shape. [`default_layout::layout`] reads
//! that state as the craft-ui layout it is drawn as; [`materialize`] commits it as a
//! [`Workspace`] once the shell is ready to draw from the workspace alone. Everything
//! downstream, the dispatcher included, then reads and writes the workspace; the native
//! state stays app-owned input the conversion must keep accepting, so a theme change or a
//! still-settling canvas re-derives it until the first real change fixes the workspace.

use craft_ui::docking::{Layout, Location, Workspace};

use super::{PANELS, Returns, commit};
use crate::PhotocraftApp;

pub(crate) use super::default_layout::layout as legacy_layout;

/// The layout drawn while the workspace has not been materialized yet.
pub(crate) fn legacy(app: &PhotocraftApp) -> Layout<String> {
    legacy_layout(app, false)
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
