//! The Color panel's options kept in the preferences (#9592); the panel itself is
//! [`crate::panels::color`].

use crate::PhotocraftApp;

/// Remember the Color panel's options in the preferences when they change, so they survive a
/// restart (#9592). Cheap: a small JSON compare per frame.
pub fn persist(app: &mut PhotocraftApp) {
    let stored = app.session.prefs().color_panel.clone();
    // Changed outside the panel since the last sync (`prefs.set`, a reset): the panel takes it.
    if app.ui.color_panel_synced.as_ref().is_some_and(|synced| *synced != stored) {
        restore(app);
        app.ui.color_panel_synced = Some(stored);
        return;
    }
    let Ok(now) = serde_json::to_value(app.ui.color_panel) else { return };
    if stored != now {
        app.session.prefs.edit(|p| p.color_panel = now.clone());
    }
    app.ui.color_panel_synced = Some(now);
}

/// Restore the remembered Color panel options (at launch, or after a preference change made
/// outside the panel); none stored (a reset) or unreadable ones are the defaults.
pub fn restore(app: &mut PhotocraftApp) {
    app.ui.color_panel = serde_json::from_value(app.session.prefs().color_panel.clone()).unwrap_or_default();
}
