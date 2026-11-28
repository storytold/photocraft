//! The remembered workspace: what `prefs.panelLayout` and a saved workspace hold, the chrome flags (`panels`) and the
//! dock's workspace (`docking`), and loading one back.
//!
//! The layout is saved when it changes: `panel_docking::apply`, the one path of every change (the dock pass's
//! included), calls [`persist`] while Remember Workspace Changes is on. The arrangement itself is `app.ui.docking`
//! (`panel_docking`); a save from before the workspace migrates once when it loads.

use serde_json::Value;

use crate::PhotocraftApp;
use crate::panel_docking::{self, Workspace};
use crate::state::Panels;

pub(crate) const RECOVERY_TITLE: &str = "The saved panel layout could not be restored.";
pub(crate) const RECOVERY_GUIDANCE: &str = "It has been preserved. Editing panels or resetting the workspace will replace it.";

/// The saved data belongs to the fallback until a committed workspace edit replaces it.
pub(crate) struct Recovery {
    saved: Value,
    fallback: Workspace,
}

/// What a saved layout holds: the chrome flags and the workspace.
pub fn snapshot(app: &PhotocraftApp) -> Value {
    if let Some(recovery) = &app.panel_recovery {
        return recovery.saved.clone();
    }
    serde_json::json!({"panels": app.ui.panels, "docking": app.ui.docking})
}

/// Remembers the layout in the preferences while Remember Workspace Changes is on, when it differs from the one
/// remembered.
pub fn persist(app: &mut PhotocraftApp) {
    if app.panel_recovery.as_ref().is_some_and(|recovery| recovery.fallback != *app.ui.docking) {
        finish_recovery(app);
    }
    if !app.session.prefs().workspace.remember_workspace_changes {
        return;
    }
    let now = snapshot(app);
    if app.session.prefs().panel_layout != now {
        app.session.prefs.remember(|prefs| prefs.panel_layout = now);
    }
}

/// Restores the remembered layout at launch. Unreadable parts keep their defaults.
pub fn restore(app: &mut PhotocraftApp) {
    if !app.session.prefs().workspace.remember_workspace_changes {
        return;
    }
    let saved = app.session.prefs().panel_layout.clone();
    // Brush Settings is a transient window: a remembered workspace does not open it on a fresh launch, especially
    // with no document. A workspace switch keeps it as the workspace saved it.
    let (mut workspace, unreadable) = panel_docking::loaded(&saved);
    panel_docking::close_transient_on_launch(app, &mut workspace);
    restore_layout(app, &saved, workspace, unreadable);
}

/// Applies a saved layout (a workspace or `panelLayout`): its workspace, and its chrome flags when they read.
pub fn apply(app: &mut PhotocraftApp, v: &Value) {
    let (workspace, unreadable) = panel_docking::loaded(v);
    restore_layout(app, v, workspace, unreadable);
}

fn restore_layout(app: &mut PhotocraftApp, v: &Value, workspace: Workspace, unreadable: bool) {
    finish_recovery(app);
    std::sync::Arc::make_mut(&mut app.ui.docking).replace(workspace);
    if unreadable {
        app.panel_recovery = Some(Recovery { saved: v.clone(), fallback: (*app.ui.docking).clone() });
        crate::notices::post_panel_recovery(app);
    }
    apply_panels(app, v);
}

/// A readable restore, a successful workspace switch or reset, or an actual panel edit replaces the fallback.
pub(crate) fn finish_recovery(app: &mut PhotocraftApp) {
    if app.panel_recovery.take().is_some() {
        crate::notices::remove_titled(app, RECOVERY_TITLE);
    }
}

/// Applies the chrome flags of a saved layout, when they read.
fn apply_panels(app: &mut PhotocraftApp, v: &Value) {
    if let Some(saved) = v.get("panels")
        && let Ok(mut panels) = serde_json::from_value::<Panels>(saved.clone())
    {
        // The embedding host owns this flag, including for layouts that serialize it.
        panels.menu_bar = app.ui.panels.menu_bar;
        app.ui.panels = panels;
    }
}

#[cfg(test)]
mod tests;
