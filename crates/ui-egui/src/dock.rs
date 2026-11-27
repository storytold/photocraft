//! The remembered workspace: what `prefs.panelLayout` and a saved workspace hold, the chrome flags (`panels`) and the
//! dock's workspace (`docking`), and loading one back.
//!
//! The layout is saved when it changes: `panel_docking::apply`, the one path of every change (the dock pass's
//! included), calls [`persist`] while Remember Workspace Changes is on. The arrangement itself is `app.ui.docking`
//! (`panel_docking`); a save from before the workspace migrates once when it loads.

use serde_json::Value;

use crate::PhotocraftApp;
use crate::state::Panels;

/// What a saved layout holds: the chrome flags and the workspace.
pub fn snapshot(app: &PhotocraftApp) -> Value {
    crate::panel_docking::snapshot(app)
}

/// Remembers the layout in the preferences while Remember Workspace Changes is on, when it differs from the one
/// remembered.
pub fn persist(app: &mut PhotocraftApp) {
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
    crate::panel_docking::load_at_launch(app, &saved);
    apply_panels(app, &saved);
}

/// Applies a saved layout (a workspace or `panelLayout`): its workspace, and its chrome flags when they read.
pub fn apply(app: &mut PhotocraftApp, v: &Value) {
    crate::panel_docking::load(app, v);
    apply_panels(app, v);
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
