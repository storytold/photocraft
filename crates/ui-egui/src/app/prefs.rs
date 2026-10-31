use super::{PhotocraftApp, dock, panel_docking, single_line_drag, text_field_menu, theme, tool_cursor, wheel_nav};

impl PhotocraftApp {
    /// Records a picked type family for the font menu's Recent group, capped by
    /// Preferences ▸ Type ▸ Recent Fonts (0 keeps no list), most recent first, de-duplicated.
    pub fn note_recent_font(&mut self, family: &str) {
        let cap = self.session.prefs().type_.recent_fonts.min(50) as usize;
        let r = &mut self.ui.recent_fonts;
        r.retain(|f| f != family);
        if cap == 0 {
            r.clear();
            return;
        }
        r.insert(0, family.to_string());
        r.truncate(cap);
    }

    /// Preferences ▸ Type ▸ Recent Fonts: a smaller cap (or 0) applies to the existing list at
    /// once, not only after the next pick.
    pub fn sync_recent_fonts(&mut self) {
        self.ui.recent_fonts.truncate(self.session.prefs().type_.recent_fonts.min(50) as usize);
    }
}

impl PhotocraftApp {
    pub fn set_theme(&mut self, ctx: &egui::Context, kind: theme::ThemeKind) {
        if let Err(e) = self.run("prefs.set", serde_json::json!({"path": "interface.theme", "value": kind.id()})) {
            self.ui.status = e;
        } else {
            self.apply_theme(ctx, kind);
        }
    }

    pub(crate) fn apply_theme(&mut self, ctx: &egui::Context, kind: theme::ThemeKind) {
        // Color and Swatches trade indices between Pro and Studio layouts.
        if let Some(selected) = dock::Group::Color.tabs(theme::Tokens::for_kind(self.ui.theme).pro).get(self.ui.dock_tabs.color)
            && let Some(tab) = dock::Group::Color.tabs(theme::Tokens::for_kind(kind).pro).iter().position(|name| name == selected)
        {
            self.ui.dock_tabs.color = tab;
        }
        self.ui.theme = kind;
        panel_docking::sync_visibility(self);
        theme::apply(ctx, kind);
        self.checker = None;
    }
}

impl PhotocraftApp {
    /// Prepare this app's context in the app creator, before the first egui pass.
    ///
    /// Font definitions take effect at the next begin-pass. Installing them here lets the first
    /// UI pass draw the editor instead of presenting a blank font-setup frame. Saved UI scale,
    /// language and font size are applied before registration, avoiding startup resizing and
    /// another font rebuild. The host must supply native DPI before preparation, as eframe does.
    /// Call again before the first pass of a replacement context. Hosts that omit this retain
    /// the deferred setup in `logic_frame`.
    pub fn prepare_first_frame(&mut self, ctx: &egui::Context) {
        let zoom = crate::prefs_ui::display_zoom(self.session.prefs().interface.ui_scale, ctx.native_pixels_per_point());
        // Initialize before begin-pass rather than queue a runtime zoom change: queued zoom
        // rescales the previous content rect, which does not yet describe this new window.
        ctx.options_mut(|options| options.zoom_factor = zoom);
        theme::set_ui_font_size(ctx, self.session.prefs().interface.ui_font_size);
        crate::i18n::sync_context(ctx, &self.session.prefs().interface.language);
        Self::setup_context(ctx, self.ui.theme);
        self.styled = true;
        self.fonts_ready = false;
    }

    /// Install fonts, image loaders, theme and Type reset pass markers on a context.
    ///
    /// This only registers context resources. App creators should use [`Self::prepare_first_frame`]
    /// to also mark the app as prepared; otherwise `logic_frame` repeats setup and skips its
    /// first editor frame. Registration during a pass takes effect on the following pass.
    pub fn setup_context(ctx: &egui::Context, kind: theme::ThemeKind) {
        Self::install_type_reset_hooks(ctx);
        ctx.add_plugin(tool_cursor::CursorLifecycle);
        ctx.add_plugin(single_line_drag::SingleLineDrag::default());
        theme::install_fonts(ctx);
        egui_extras::install_image_loaders(ctx);
        theme::apply(ctx, kind);
        // Right-click in any text field: its Cut / Copy / Paste menu (text_field_menu.rs).
        ctx.add_plugin(text_field_menu::TextFieldMenu);
        // egui's own ⌘+ / ⌘- / ⌘0 scale the whole interface; PhotoCraft zooms the canvas instead
        // (shortcuts.rs), like Photoshop.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        // ⌘/Ctrl + wheel scrolls sideways, as in Photoshop; only a pinch zooms (wheel_nav.rs).
        wheel_nav::configure(ctx);
    }
}
