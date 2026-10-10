//! The no-op exporter for targets without a D-Bus session (wasm web build, macOS, iOS):
//! the application menu already shows in-window there, so there is nothing to export.
//! The API is identical to the live exporter, so callers compile unchanged.

use crate::{Error, MenuEvent, MenuModel, MenuWake};

/// A handle whose event queue is always empty.
pub struct AppMenu {
    _reason: String,
}

impl AppMenu {
    /// Always fails: there is no global-menu host on this platform. `wake` is accepted so
    /// every target compiles to the same call (it is never called here).
    pub fn start(app_name: &str, _wake: MenuWake) -> Result<AppMenu, Error> {
        Err(Error::Platform(format!(
            "{app_name}: no global menu (AppMenu/dbusmenu) on this platform; \
             the in-window menu bar is already visible"
        )))
    }

    /// No-op.
    pub fn replace(&self, _model: &MenuModel) {}

    /// Never fires on this platform.
    pub fn try_event(&self) -> Option<MenuEvent> {
        None
    }

    /// The unique bus name the export is served on, once connected; always `None` here.
    pub fn bus_name(&self) -> Option<String> {
        None
    }

    /// Always false: no menu host exists on this platform, so the app keeps its in-window
    /// menu bar.
    pub fn hosted(&self) -> bool {
        false
    }

    /// No-op.
    pub fn shutdown(&self) {}
}
