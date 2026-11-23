use super::{AppearanceMode, Color32, PhotocraftApp, json, prefs};
use crate::theme::AppTheme;
use craft::menu::{ThemeChoice, ThemeMenu, ThemeRow};
use craft::theme::{Mode, ThemeId};
use craft::ui::runtime::env::system_appearance;

/// The GPU canvas's gamut warning colours from Preferences › Transparency & Gamut.
pub fn canvas_style(app: &PhotocraftApp) -> crate::gpu_canvas::CanvasStyle {
    let t = &app.session.prefs().transparency_and_gamut;
    let f = |c: [u8; 3]| c.map(|v| v as f32 / 255.0);
    let g = prefs::parse_hex(&t.gamut_warning_color).unwrap_or([128; 3]);
    crate::gpu_canvas::CanvasStyle { gamut_color: f(g), gamut_opacity: t.gamut_warning_opacity as f32 / 100.0 }
}

/// Pasteboard colour from Preferences › Interface (`None` = the theme's default canvas).
pub fn pasteboard_color(app: &PhotocraftApp) -> Option<Color32> {
    use prefs::CanvasColor;
    let i = &app.session.prefs().interface;
    Some(match i.canvas_color {
        CanvasColor::Default => return None,
        CanvasColor::Black => Color32::BLACK,
        CanvasColor::DarkGray => Color32::from_gray(40),
        CanvasColor::MediumGray => Color32::from_gray(83),
        CanvasColor::LightGray => Color32::from_gray(184),
        CanvasColor::Custom => prefs::parse_hex(&i.canvas_custom_color).map_or(Color32::GRAY, |c| Color32::from_rgb(c[0], c[1], c[2])),
    })
}

/// The suite theme and mode the preferences show: the slot of the appearance mode, which `Auto`
/// takes from the system.
pub(super) fn selected_theme(interface: &prefs::Interface, system: Option<egui::Theme>) -> (ThemeId, bool) {
    let (slot, dark) = selected_slot(interface, system);
    (ThemeId::parse(slot).unwrap_or_else(|_| ThemeId::suite_default()), dark)
}

/// The theme id the mode shows, as the preferences hold it, and whether that mode is dark.
fn selected_slot(interface: &prefs::Interface, system: Option<egui::Theme>) -> (&str, bool) {
    let light = match interface.appearance_mode {
        AppearanceMode::Light => true,
        AppearanceMode::Dark => false,
        AppearanceMode::Auto => system == Some(egui::Theme::Light),
    };
    (if light { &interface.light_theme } else { &interface.dark_theme }, !light)
}

pub(crate) fn cycle_appearance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let next = match app.session.prefs().interface.appearance_mode {
        AppearanceMode::Auto => AppearanceMode::Light,
        AppearanceMode::Light => AppearanceMode::Dark,
        AppearanceMode::Dark => AppearanceMode::Auto,
    };
    if let Err(e) = app.run("prefs.set", json!({"path": "interface.appearanceMode", "value": next.name()})) {
        app.ui.status = e;
        return;
    }
    sync_appearance(app, ctx);
}

/// Darken Interface (⇧⌘F1, `lighter` false) and Lighten Interface (⇧⌘F2): the next of
/// Photoshop's four brightnesses from the theme on screen, stopping at either end, as in
/// Photoshop. Another theme steps from the nearest brightness: Medium Gray for a dark one, Light
/// Gray for a light one. The choice is saved like picking it in Preferences › Interface.
pub(crate) fn step_brightness(app: &mut PhotocraftApp, ctx: &egui::Context, lighter: bool) -> Result<(), String> {
    let brightnesses = AppTheme::brightnesses();
    let at = brightnesses.iter().position(|k| *k == app.ui.theme).unwrap_or(if app.ui.theme.is_dark() { 1 } else { 2 });
    let next = if lighter { (at + 1).min(brightnesses.len() - 1) } else { at.saturating_sub(1) };
    let Some(theme) = brightnesses.get(next) else { return Ok(()) };
    if *theme == app.ui.theme {
        return Ok(());
    }
    app.save_theme(theme)?;
    sync_appearance(app, ctx);
    Ok(())
}

/// The Appearance menu's rows: every theme in both modes, the current choice and the favourites.
pub fn theme_menu(app: &PhotocraftApp) -> ThemeMenu {
    let interface = &app.session.prefs().interface;
    let mut rows = Vec::new();
    for (row, dark) in [(AppTheme::light_row(), false), (AppTheme::dark_row(), true)] {
        for theme in &row {
            rows.push(ThemeRow::new(theme.id().clone(), dark, app.framework.theme_name(theme.id(), dark)));
        }
    }
    let current = match interface.appearance_mode {
        AppearanceMode::Auto => ThemeChoice::System,
        _ => {
            let (id, dark) = selected_theme(interface, None);
            ThemeChoice::theme(id, dark)
        }
    };
    let favorites: Vec<ThemeId> = interface.favorite_themes.iter().filter_map(|f| ThemeId::parse(f).ok()).collect();
    ThemeMenu::new(rows, current).favorites(&favorites)
}

/// The OS's light or dark mode, `None` where it gives no answer.
pub(super) fn system_theme(_app: &PhotocraftApp, ctx: &egui::Context) -> Option<egui::Theme> {
    system_appearance(ctx).appearance.mode.map(|mode| match mode {
        Mode::Dark => egui::Theme::Dark,
        _ => egui::Theme::Light,
    })
}

/// Resolve an absolute pixels-per-point preference without multiplying the OS DPI twice.
/// Monitor dimensions are physical pixels, independent of window size and UI zoom.
pub(super) fn display_scale(pref: prefs::UiScale, native: Option<f32>) -> f32 {
    let native = native.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1.0);
    match pref {
        // The system's scale, fractional ones included (125%, 150%, 175%: #1072), and 100% on a
        // large 4K display run unscaled (#1278); 200% there is a fixed choice.
        prefs::UiScale::Auto => native,
        fixed => fixed.name().parse::<f32>().map_or(1.0, |pct| pct / 100.0),
    }
}

pub(super) fn sync_display_scale(app: &PhotocraftApp, ctx: &egui::Context) {
    ctx.set_zoom_factor(interface_zoom(app.session.prefs().interface.ui_scale, ctx));
}

/// The zoom factor over the native pixels per point that gives `pref`'s scale. The craft runtime
/// sets the same one before each frame (`app/framework.rs`).
pub(crate) fn interface_zoom(pref: prefs::UiScale, ctx: &egui::Context) -> f32 {
    let native = ctx.native_pixels_per_point();
    let native_scale = native.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1.0);
    display_scale(pref, native) / native_scale
}

/// Interface › Show Tooltips and Tools › Show Tooltips (either one off hides them): egui never
/// shows a tooltip whose delay is infinite. Re-checked every frame because a theme change
/// rebuilds the style; that's one style read, and a write only when it differs.
pub(super) fn sync_tooltips(app: &PhotocraftApp, ctx: &egui::Context) {
    let p = app.session.prefs();
    let delay = if p.interface.show_tooltips && p.tools.show_tooltips { crate::theme::TOOLTIP_DELAY } else { f32::INFINITY };
    if ctx.global_style().interaction.tooltip_delay != delay {
        ctx.global_style_mut(|s| s.interaction.tooltip_delay = delay);
    }
}

/// Shows the theme the saved appearance mode and slots select, without changing them, when it
/// isn't the one on screen. Runs every frame, so it only compares the slot's id and the mode; a
/// slot the suite can't resolve, shown as the mode's default, resolves again only when
/// `Framework::sync_theme` reports a change.
pub(crate) fn sync_appearance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // A restored or embedding-supplied pin must not suppress native appearance events.
    ctx.set_theme(egui::ThemePreference::System);
    // The native service also covers Wayland sessions where winit has no system theme.
    let (slot, dark) = selected_slot(&app.session.prefs().interface, system_theme(app, ctx));
    let shown = &app.ui.theme;
    if dark == shown.is_dark() && (slot == shown.id().as_str() || app.framework.shows_fallback_for(slot, shown)) {
        return;
    }
    show_selected_theme(app, ctx);
}

/// Resolves the theme the saved appearance mode and slots select and shows it when it differs
/// from the one on screen: also after the suite moved (`Framework::sync_theme`), when a library
/// theme's definition may have changed under the same id.
pub(crate) fn show_selected_theme(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let (id, dark) = selected_theme(&app.session.prefs().interface, system_theme(app, ctx));
    // A library theme the suite is loading: the current look stays until the suite's read shows it.
    if app.framework.theme_availability(&id) == Err(crate::app::framework::ThemeUnavailable::Loading) {
        return;
    }
    let selected = app.framework.theme_for(&id, dark, ctx);
    if app.ui.theme != selected {
        app.apply_theme(ctx, selected);
    }
}
