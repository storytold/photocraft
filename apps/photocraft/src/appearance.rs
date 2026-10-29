//! Connection to the desktop settings portal, shared with the caption-button layout reader.

#[cfg(target_os = "linux")]
pub(crate) fn portal_proxy() -> Option<zbus::blocking::Proxy<'static>> {
    let conn = zbus::blocking::connection::Builder::session().ok()?.method_timeout(std::time::Duration::from_secs(2)).build().ok()?;
    zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings").ok()
}

/// Platforms where craft-platform watches the desktop settings portal.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
pub(crate) use portal::{PortalTheme, start_watcher};

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod portal {
    use craft_platform::appearance::{Theme, Watcher};

    /// Put the settings portal's answer into one viewport's input; no answer clears winit's value.
    fn apply_portal(observed: Option<Theme>, raw: &mut egui::RawInput) {
        raw.system_theme = observed.map(|theme| match theme {
            Theme::Dark => egui::Theme::Dark,
            Theme::Light => egui::Theme::Light,
        });
    }

    /// The appearance native decorations follow: the fixed variant when both style slots hold the
    /// same theme, the portal's answer otherwise.
    fn decoration_theme(ctx: &egui::Context, observed: Option<Theme>) -> Option<Theme> {
        match photocraft_ui_egui::theme::installed(ctx) {
            Some((light, dark)) if light == dark => Some(if light.is_dark() { Theme::Dark } else { Theme::Light }),
            _ => observed,
        }
    }

    /// On Linux and the BSDs winit cannot read the desktop appearance reliably (Wayland compositors
    /// send none), so the portal's answer replaces winit's in the input of every viewport. This
    /// runs as a viewport hook because `App::raw_input_hook` skips immediate viewports.
    pub(crate) struct PortalTheme(std::sync::Mutex<Watcher>);

    impl PortalTheme {
        pub(crate) fn new(watcher: Watcher) -> Self {
            Self(std::sync::Mutex::new(watcher))
        }

        /// Waits for the watcher's first answer, at most `remaining` of the startup budget, so the
        /// first frame already uses the desktop's appearance.
        pub(crate) fn wait_first(&self, remaining: std::time::Duration) {
            self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).wait_first(remaining);
        }
    }

    impl eframe::ViewportHooks for PortalTheme {
        fn raw_input(&self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
            let observed = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).current();
            apply_portal(observed, raw_input);
            craft_platform::appearance::sync_native_window_theme(ctx, decoration_theme(ctx, observed));
        }
    }

    /// Starts the portal watcher; `ctx` is set once the window's context exists, and the watcher
    /// wakes it on a change.
    pub(crate) fn start_watcher(ctx: std::sync::Arc<std::sync::OnceLock<egui::Context>>) -> Option<Watcher> {
        Watcher::start(move || {
            if let Some(ctx) = ctx.get() {
                ctx.request_repaint();
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn portal_answer_replaces_winit_value() {
            let mut raw = egui::RawInput { system_theme: Some(egui::Theme::Dark), ..Default::default() };
            apply_portal(None, &mut raw);
            assert_eq!(raw.system_theme, None);
            apply_portal(Some(Theme::Light), &mut raw);
            assert_eq!(raw.system_theme, Some(egui::Theme::Light));
            apply_portal(Some(Theme::Dark), &mut raw);
            assert_eq!(raw.system_theme, Some(egui::Theme::Dark));
        }
    }
}
