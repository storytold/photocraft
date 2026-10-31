use super::dialogs::open_preferences;
use super::fields::{choice_label, color_of, preference_matches};
use super::{AppearanceMode, Color32, DarkTheme, LightTheme, Map, PhotocraftApp, RichText, SavedTheme, Sense, ThemeKind, Tokens, Value, json, prefs, vec2};

/// The GPU canvas colours from Preferences › Transparency & Gamut.
pub fn canvas_style(app: &PhotocraftApp) -> crate::gpu_canvas::CanvasStyle {
    let t = &app.session.prefs().transparency_and_gamut;
    let [l, d] = t.colors();
    let f = |c: [u8; 3]| c.map(|v| v as f32 / 255.0);
    let g = prefs::parse_hex(&t.gamut_warning_color).unwrap_or([128; 3]);
    crate::gpu_canvas::CanvasStyle {
        checker_square: t.square().unwrap_or(0.0),
        checker_light: f(l),
        checker_dark: f(d),
        gamut_color: f(g),
        gamut_opacity: t.gamut_warning_opacity as f32 / 100.0,
    }
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
        CanvasColor::Custom => color_of(&i.canvas_custom_color),
    })
}

pub(super) fn selected_theme(interface: &prefs::Interface, system: Option<egui::Theme>) -> ThemeKind {
    let light = match interface.appearance_mode {
        AppearanceMode::Light => true,
        AppearanceMode::Dark => false,
        AppearanceMode::Auto => system == Some(egui::Theme::Light),
    };
    if light {
        match interface.light_theme {
            LightTheme::StudioLight => ThemeKind::StudioLight,
            LightTheme::ProLightGray => ThemeKind::ProLightGray,
            LightTheme::ProLight => ThemeKind::ProLight,
            LightTheme::Classic => ThemeKind::Classic,
            LightTheme::Adwaita => ThemeKind::Adwaita,
            LightTheme::BreezeLight => ThemeKind::BreezeLight,
            LightTheme::SoftLight => ThemeKind::SoftLight,
        }
    } else {
        match interface.dark_theme {
            DarkTheme::Pro => ThemeKind::Pro,
            DarkTheme::ProMedium => ThemeKind::ProMedium,
            DarkTheme::Studio => ThemeKind::Studio,
            DarkTheme::SolarizedDark => ThemeKind::SolarizedDark,
            DarkTheme::AdwaitaDark => ThemeKind::AdwaitaDark,
            DarkTheme::BreezeDark => ThemeKind::BreezeDark,
            DarkTheme::SoftDark => ThemeKind::SoftDark,
        }
    }
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
    let selected = selected_theme(&app.session.prefs().interface, system_theme(app, ctx));
    if app.ui.theme != selected {
        app.apply_theme(ctx, selected);
    }
}

/// Photoshop's four interface brightnesses, darkest first.
const BRIGHTNESSES: [ThemeKind; 4] = [ThemeKind::Pro, ThemeKind::ProMedium, ThemeKind::ProLightGray, ThemeKind::ProLight];

/// Darken Interface (⇧⌘F1, `lighter` false) and Lighten Interface (⇧⌘F2): the next of
/// Photoshop's four brightnesses from the theme on screen, stopping at either end, as in
/// Photoshop. Another theme steps from the nearest brightness: Medium Gray for a dark one, Light
/// Gray for a light one. The choice is saved like picking it in Preferences › Interface.
pub(crate) fn step_brightness(app: &mut PhotocraftApp, ctx: &egui::Context, lighter: bool) -> Result<(), String> {
    let current = app.ui.theme;
    let at = BRIGHTNESSES.iter().position(|k| *k == current).unwrap_or(if crate::theme::Tokens::for_kind(current).dark() { 1 } else { 2 });
    let next = if lighter { (at + 1).min(BRIGHTNESSES.len() - 1) } else { at.saturating_sub(1) };
    let Some(&kind) = BRIGHTNESSES.get(next) else { return Ok(()) };
    if kind == current {
        return Ok(());
    }
    app.run("prefs.set", json!({"path": "interface.theme", "value": kind.id()}))?;
    sync_appearance(app, ctx);
    Ok(())
}

pub(super) const LIGHT_THEME_OPTIONS: &[(&str, ThemeKind)] = &[
    ("proLight", ThemeKind::ProLight),
    ("proLightGray", ThemeKind::ProLightGray),
    ("studioLight", ThemeKind::StudioLight),
    ("classic", ThemeKind::Classic),
    ("adwaita", ThemeKind::Adwaita),
    ("breezeLight", ThemeKind::BreezeLight),
    ("softLight", ThemeKind::SoftLight),
];

pub(super) const DARK_THEME_OPTIONS: &[(&str, ThemeKind)] = &[
    ("proMedium", ThemeKind::ProMedium),
    ("pro", ThemeKind::Pro),
    ("studio", ThemeKind::Studio),
    ("solarizedDark", ThemeKind::SolarizedDark),
    ("adwaitaDark", ThemeKind::AdwaitaDark),
    ("breezeDark", ThemeKind::BreezeDark),
    ("softDark", ThemeKind::SoftDark),
];

pub(super) fn favorite_theme_groups(favorites: &[SavedTheme]) -> (Vec<ThemeKind>, Vec<ThemeKind>) {
    let group = |options: &[(&str, ThemeKind)]| {
        options.iter().filter_map(|(id, kind)| SavedTheme::parse(id).filter(|theme| favorites.contains(theme)).map(|_| *kind)).collect()
    };
    (group(LIGHT_THEME_OPTIONS), group(DARK_THEME_OPTIONS))
}

/// Right-click menu on the top-right appearance button. A plain click still cycles modes.
pub(crate) fn appearance_context_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.button(tl!("Appearance Settings…")).clicked() {
        open_preferences(app, "interface");
        ui.close();
    }
    let (light, dark) = favorite_theme_groups(&app.session.prefs().interface.favorite_themes);
    let has_light = !light.is_empty();
    if !light.is_empty() || !dark.is_empty() {
        ui.separator();
    }
    for kind in light {
        if ui.selectable_label(app.ui.theme == kind, tl!(kind.label())).clicked() {
            app.set_theme(ui.ctx(), kind);
            ui.close();
        }
    }
    if has_light && !dark.is_empty() {
        ui.separator();
    }
    for kind in dark {
        if ui.selectable_label(app.ui.theme == kind, tl!(kind.label())).clicked() {
            app.set_theme(ui.ctx(), kind);
            ui.close();
        }
    }
}

pub(super) fn system_theme(app: &PhotocraftApp, ctx: &egui::Context) -> Option<egui::Theme> {
    app.services.system_theme.as_ref().and_then(|read| read(ctx)).or_else(|| ctx.system_theme())
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
    let native = ctx.native_pixels_per_point();
    let native_scale = native.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1.0);
    let scale = display_scale(app.session.prefs().interface.ui_scale, native);
    ctx.set_zoom_factor(scale / native_scale);
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

/// Interface › Enable Animations: off, egui's fades and slides take no time and scroll areas jump
/// straight to their target (#3273). Re-checked every frame because a theme change rebuilds the
/// style; both the dark and light styles are set, so a system appearance switch keeps it.
pub(super) fn sync_animations(app: &PhotocraftApp, ctx: &egui::Context) {
    let on = app.session.prefs().interface.enable_animations;
    let time = if on { egui::Style::default().animation_time } else { 0.0 };
    if ctx.global_style().animation_time != time {
        let scroll = if on { egui::style::ScrollAnimation::default() } else { egui::style::ScrollAnimation::none() };
        ctx.all_styles_mut(|s| {
            s.animation_time = time;
            s.scroll_animation = scroll;
        });
    }
}

/// Resolve the saved appearance mode without changing it or either theme slot.
pub(crate) fn sync_appearance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // A restored or embedding-supplied pin must not suppress native appearance events.
    ctx.set_theme(egui::ThemePreference::System);
    // The native service also covers Wayland sessions where winit has no system theme.
    let selected = selected_theme(&app.session.prefs().interface, system_theme(app, ctx));
    if app.ui.theme != selected {
        app.apply_theme(ctx, selected);
    }
}

/// One theme as a Color Theme swatch: its panel colour under a chrome strip with an accent dot,
/// outlined when chosen. The name is the tooltip and the accessible label. A star badge on its
/// corner adds the theme to (or removes it from) the favourites listed by the appearance
/// button's menu; it shows while the swatch is hovered, and stays for a favourite.
fn theme_swatch(ui: &mut egui::Ui, kind: ThemeKind, id: &str, selected: &mut String, favorites: &mut Vec<SavedTheme>) {
    const BADGE: f32 = 7.0;
    let t = Tokens::get(ui.ctx());
    let p = Tokens::for_kind(kind);
    let label = tl!(&choice_label(id)).to_string();
    let on = selected.as_str() == id;
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
    let painter = ui.painter();
    let inner = rect.shrink(3.0);
    painter.rect_filled(inner, 2.0, p.dock);
    painter.rect_filled(egui::Rect::from_min_size(inner.min, vec2(inner.width(), 5.0)), 0.0, p.chrome);
    painter.circle_filled(inner.right_bottom() - vec2(6.0, 6.0), 2.5, p.accent);
    let stroke = if on {
        egui::Stroke::new(2.0, t.accent)
    } else if resp.hovered() {
        egui::Stroke::new(1.0, t.text_dim)
    } else {
        egui::Stroke::new(1.0, t.card_border)
    };
    painter.rect_stroke(rect.shrink(1.0), 3.0, stroke, egui::StrokeKind::Inside);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, &label));
    if resp.on_hover_text(&label).clicked() {
        *selected = id.to_string();
    }
    let Some(theme) = SavedTheme::parse(id) else { return };
    let favorite = favorites.contains(&theme);
    let center = rect.right_top() + vec2(-3.0, 3.0);
    let badge = egui::Rect::from_center_size(center, vec2(BADGE * 2.0, BADGE * 2.0));
    if !(favorite || ui.rect_contains_pointer(rect.union(badge))) {
        return;
    }
    ui.painter().circle(center, BADGE, t.card, egui::Stroke::new(1.0, t.card_border));
    let (icon, tint) = if favorite { ("star-filled", t.accent) } else { ("star", t.text_dim) };
    let action = if favorite { tl!("Remove from favorites") } else { tl!("Add to favorites") };
    // Interact and paint without allocating: a widget placed at `badge` would grow the row by the
    // 4 px the badge overhangs the swatch, re-centring the swatches after this one whenever the
    // pointer arrives, and oscillating when that moves the swatch out from under it.
    let response = ui.interact(badge, ui.scope_id().with((id, "favorite")), Sense::click()).on_hover_text(action);
    crate::icons::paint(ui, badge, icon, 10.0, tint);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{action}: {label}")));
    if response.clicked() {
        if favorite {
            favorites.retain(|saved| *saved != theme);
        } else {
            favorites.push(theme);
        }
    }
}

/// A grid row of theme swatches, marked Active when the appearance mode shows this family.
fn theme_row(ui: &mut egui::Ui, title: &str, active: bool, selected: &mut String, favorites: &mut Vec<SavedTheme>, options: &[(&str, ThemeKind)]) {
    let t = Tokens::get(ui.ctx());
    // Grid columns always advance left to right, so emit them in reading order.
    crate::direction::grid_cells(
        ui,
        |ui| {
            ui.label(RichText::new(tl!(title)).color(t.text_dim));
        },
        |ui| {
            ui.horizontal(|ui| {
                // Room for the favourite star badges on the swatch corners.
                ui.spacing_mut().item_spacing.x = 6.0;
                let rtl = crate::direction::is_rtl();
                // A Grid cell's row stays physical, so under RTL the strip is emitted in
                // reverse: the Active marker goes first (the reading end) and the first option
                // ends up the rightmost one. The swatch paint itself never mirrors.
                if rtl && active {
                    ui.label(RichText::new(tl!("Active")).color(t.accent_text).small());
                    ui.add_space(6.0);
                }
                for i in 0..options.len() {
                    let (id, kind) = options[if rtl { options.len() - 1 - i } else { i }];
                    theme_swatch(ui, kind, id, selected, favorites);
                }
                if !rtl && active {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tl!("Active")).color(t.accent_text).small());
                }
            });
        },
    );
    ui.end_row();
}

/// Which of the Appearance rows drawn by `appearance_rows` a search shows.
#[derive(Clone, Copy)]
pub(super) struct AppearanceRows {
    mode: bool,
    light: bool,
    dark: bool,
}

impl AppearanceRows {
    /// `query` is lowercased; empty shows every row.
    pub(super) fn matching(query: &str) -> Self {
        let shown = |key: &str| query.is_empty() || preference_matches("interface", key, query);
        Self { mode: shown("appearanceMode"), light: shown("lightTheme"), dark: shown("darkTheme") }
    }

    pub(super) fn any(self) -> bool {
        self.mode || self.light || self.dark
    }
}

/// Appearance Mode and the light and dark Color Theme swatches, as rows of the Appearance grid.
pub(super) fn appearance_rows(ui: &mut egui::Ui, obj: &mut Map<String, Value>, system: Option<egui::Theme>, rows: AppearanceRows) {
    let t = Tokens::get(ui.ctx());
    let mut mode = obj.get("appearanceMode").and_then(Value::as_str).unwrap_or("auto").to_string();
    if rows.mode {
        let choices: [(String, &str); 3] = [("auto".into(), tl!("Sync with system")), ("dark".into(), tl!("Dark")), ("light".into(), tl!("Light"))];
        let pairs: Vec<(String, &str)> = choices.iter().map(|(id, label)| (id.clone(), *label)).collect();
        // Grid columns always advance left to right, so emit them in reading order.
        crate::direction::grid_cells(
            ui,
            |ui| {
                ui.label(RichText::new(tl!("Appearance Mode")).color(t.text_dim));
            },
            |ui| {
                crate::widgets::dropdown(ui, "appearance-mode", tl!("Appearance Mode"), &mut mode, &pairs, 220.0);
            },
        );
        ui.end_row();
    }
    let light_active = mode == "light" || (mode == "auto" && system == Some(egui::Theme::Light));
    obj.insert("appearanceMode".into(), json!(mode));
    let mut light = obj.get("lightTheme").and_then(Value::as_str).unwrap_or("studioLight").to_string();
    let mut dark = obj.get("darkTheme").and_then(Value::as_str).unwrap_or("proMedium").to_string();
    let mut favorites: Vec<SavedTheme> = obj.get("favoriteThemes").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    if rows.light {
        theme_row(ui, "Light Theme", light_active, &mut light, &mut favorites, LIGHT_THEME_OPTIONS);
    }
    if rows.dark {
        theme_row(ui, "Dark Theme", !light_active, &mut dark, &mut favorites, DARK_THEME_OPTIONS);
    }
    obj.insert("lightTheme".into(), json!(light));
    obj.insert("darkTheme".into(), json!(dark));
    obj.insert("favoriteThemes".into(), json!(favorites));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use crate::text::direction::body_layout;
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    /// The Appearance grid as the preferences page draws it: two columns, the rows of
    /// `appearance_rows`, under the body's own direction.
    fn appearance_harness() -> Harness<'static, Map<String, Value>> {
        let fields = Map::new();
        // The app sets the direction flag with the style before its first frame; the widgets
        // mirror on it, and a Grid's first sizing pass would otherwise cache LTR column widths.
        Harness::builder().with_size(vec2(520.0, 480.0)).with_context_setup(|ctx| crate::theme::set_direction(ctx, crate::direction::is_rtl())).build_ui_state(
            |ui: &mut egui::Ui, f: &mut Map<String, Value>| {
                ui.with_layout(body_layout(), |ui| {
                    super::super::fields::grid_scope(ui, |ui| {
                        egui::Grid::new("appearance-grid").num_columns(2).show(ui, |ui| {
                            appearance_rows(ui, f, Some(egui::Theme::Dark), AppearanceRows::matching(""));
                        });
                    });
                });
            },
            fields,
        )
    }

    /// The rect of one light-theme swatch, by its theme name.
    fn swatch(h: &Harness<'_, Map<String, Value>>, index: usize) -> egui::Rect {
        let name = swatch_name(LIGHT_THEME_OPTIONS[index].0);
        h.query_all_by_role_and_label(Role::RadioButton, &name).next().unwrap_or_else(|| panic!("the {name} swatch")).rect()
    }

    /// The name a swatch shows, translated like the page does.
    fn swatch_name(id: &str) -> String {
        crate::i18n::t(&choice_label(id)).to_string()
    }

    /// The Appearance grid in a right-to-left UI: the label reads first (at the right), the
    /// editor follows toward the left, and the first light-theme swatch is the rightmost one.
    /// The favourite badge stays at its physical corner: it is painted, not laid out.
    #[test]
    fn appearance_rows_read_rtl() {
        let _rtl = direction_scope(Dir::Rtl);
        let mut h = appearance_harness();
        h.run_steps(3);
        let label = h.query_all_by_role_and_label(Role::Label, "Appearance Mode").next().expect("the Appearance Mode label").rect();
        let editor = h.query_all_by_role_and_label(Role::ComboBox, "Appearance Mode").next().expect("the Appearance Mode dropdown").rect();
        assert!(label.left() > editor.right(), "the label reads first: {label:?} {editor:?}");

        let first = swatch(&h, 0);
        let last = swatch(&h, LIGHT_THEME_OPTIONS.len() - 1);
        assert!(first.left() > last.left(), "the first swatch is the rightmost one: {first:?} {last:?}");

        h.hover_at(first.center());
        h.run_steps(3);
        let name = swatch_name(LIGHT_THEME_OPTIONS[0].0);
        let badge = h.query_by_label(&format!("Add to favorites: {name}")).expect("the badge shows while the swatch is hovered").rect();
        let corner = first.right_top() + vec2(-3.0, 3.0);
        assert!((badge.center() - corner).length() < 1.0, "the badge keeps its physical corner: {badge:?} {first:?}");
    }

    /// The left-to-right twin: the label keeps its place at the left and the first swatch stays
    /// the leftmost one.
    #[test]
    fn appearance_rows_read_ltr() {
        let _ltr = direction_scope(Dir::Ltr);
        let mut h = appearance_harness();
        h.run_steps(3);
        let label = h.query_all_by_role_and_label(Role::Label, "Appearance Mode").next().expect("the Appearance Mode label").rect();
        let editor = h.query_all_by_role_and_label(Role::ComboBox, "Appearance Mode").next().expect("the Appearance Mode dropdown").rect();
        assert!(label.right() < editor.left(), "the label keeps its place: {label:?} {editor:?}");

        let first = swatch(&h, 0);
        let last = swatch(&h, LIGHT_THEME_OPTIONS.len() - 1);
        assert!(first.left() < last.left(), "the first swatch keeps its place: {first:?} {last:?}");
    }
}
