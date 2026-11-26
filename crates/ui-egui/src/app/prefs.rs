use super::{PhotocraftApp, theme};

impl PhotocraftApp {
    /// Shows `theme` and saves it as the theme of its mode, which becomes the appearance mode.
    pub fn set_theme(&mut self, ctx: &egui::Context, theme: theme::AppTheme) {
        if let Err(e) = self.save_theme(&theme) {
            self.ui.status = e;
        } else {
            self.apply_theme(ctx, theme);
        }
    }

    /// Saves `theme` as the theme of its mode's slot, and its mode as the appearance mode.
    pub(crate) fn save_theme(&mut self, theme: &theme::AppTheme) -> Result<(), String> {
        self.save_theme_choice(theme.id(), theme.is_dark())
    }

    /// Saves theme `id` as the theme of the dark or light slot, and that mode as the appearance
    /// mode: where every pick goes. The suite follows the preferences (`Framework::sync_theme`),
    /// and loads a library theme picked this way.
    fn save_theme_choice(&mut self, id: &craft::theme::ThemeId, dark: bool) -> Result<(), String> {
        let slot = if dark { "interface.darkTheme" } else { "interface.lightTheme" };
        let mode = if dark { "dark" } else { "light" };
        self.run("prefs.set", serde_json::json!({"values": {slot: id.as_str(), "interface.appearanceMode": mode}})).map(drop)
    }

    /// Refuses theme `id` for `ui.set` and Window › Theme when the suite doesn't know it.
    pub(crate) fn check_theme(&self, id: &craft::theme::ThemeId) -> Result<(), String> {
        match self.framework.theme_availability(id) {
            Err(why @ super::framework::ThemeUnavailable::Unknown) => Err(why.message(id)),
            _ => Ok(()),
        }
    }

    /// Picks theme `id` for the dark or light slot (one [`check_theme`](Self::check_theme)
    /// passed): shown now when the suite has it, else saved like an Appearance settings pick, and
    /// the suite's read of it shows it. Returns the note for a theme still loading.
    pub(crate) fn pick_theme(&mut self, ctx: &egui::Context, id: &craft::theme::ThemeId, dark: bool) -> Option<String> {
        let Err(why) = self.framework.theme_availability(id) else {
            let theme = self.framework.theme_for(id, dark, ctx);
            self.set_theme(ctx, theme);
            return None;
        };
        if let Err(e) = self.save_theme_choice(id, dark) {
            self.ui.status = e;
        }
        // The suite takes the pick now, so the slot reads as loading from this frame on and the
        // current look stays until the suite's read shows the theme.
        if self.framework.sync_theme(&mut self.session) {
            crate::prefs_ui::show_selected_theme(self, ctx);
        }
        Some(why.message(id))
    }

    pub(crate) fn apply_theme(&mut self, ctx: &egui::Context, theme: theme::AppTheme) {
        self.ui.theme = theme;
        theme::apply(ctx, &self.ui.theme);
    }

    /// Follows the theme selection the suite's folder holds, shared with the other Craft apps, in
    /// place of the in-memory selection the app starts with. The desktop app calls it at startup.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_suite(&mut self, ctx: &egui::Context) {
        self.framework.open_suite(ctx);
    }
}

impl PhotocraftApp {
    /// Install the app's icons, fonts, image loaders and the theme. Call from the app creator
    /// (and again for a replacement Context) before the first `run_ui` so the very first frame
    /// renders; otherwise `logic` does it and the first frame is skipped.
    pub fn setup_context(ctx: &egui::Context, theme: theme::AppTheme) {
        Self::setup(ctx, theme, None, None);
    }

    /// [`setup_context`](Self::setup_context) for a window whose frame the platform decided
    /// (`decision`): the runtime installs the frame's look with the fonts, and the dialogs that
    /// open their own window keep their size and place in the settings folder `settings`.
    pub fn setup_window(ctx: &egui::Context, theme: theme::AppTheme, decision: craft::window::policy::Decision, settings: Option<&std::path::Path>) {
        Self::setup(ctx, theme, Some(decision), settings);
    }

    /// The one setup both entry points run.
    fn setup(ctx: &egui::Context, theme: theme::AppTheme, frame: Option<craft::window::policy::Decision>, settings: Option<&std::path::Path>) {
        crate::icons::register();
        theme::install_runtime(ctx, frame, settings);
        egui_extras::install_image_loaders(ctx);
        theme::apply(ctx, &theme);
        // egui's own ⌘+ / ⌘- / ⌘0 scale the whole interface; PhotoCraft zooms the canvas instead
        // (shortcuts.rs), like Photoshop.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
    }
}
