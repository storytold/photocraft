use super::{PhotocraftApp, Sense, Tokens, icons, theme, vec2, widgets};

// ----------------------------------------------------------------------------- title bar

/// The app's top bar: brand mark, menus, the document title and the workspace controls. With
/// [`PhotocraftApp::custom_titlebar`] (Windows and Linux) it is also the window's title bar, as in
/// Photoshop on Windows: the caption buttons take its ends (`titlebar`). With the system
/// title bar (Windows and Linux, `system_title_bar`) the OS draws its own icon and title, so the
/// in-app bar shows menus and workspace controls only: no brand mark, no centred title.
pub fn title_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let custom = app.custom_titlebar;
    let system = system_title_bar(app);
    // The caption buttons' room at each end (`titlebar`); a side with buttons has no margin.
    let (caption_left, caption_right) = crate::titlebar::reserved(app, ui.ctx());
    // macOS: the strip fits around the traffic lights where AppKit measured them (#2554).
    let mac_bar = app.mac_titlebar.filter(|_| cfg!(target_os = "macos") && app.integrated_titlebar).map(|b| b.in_egui_points(ui.ctx().zoom_factor()));
    // `egui::Margin` is in `i8`: the rest of a wider start (a small UI Scale) goes before the content.
    let mac_inset = mac_bar.map_or(0.0, |b| (b.content_left - f32::from(i8::MAX)).max(0.0));
    let left = if let Some(bar) = mac_bar {
        bar.content_left.min(f32::from(i8::MAX)) as i8
    } else if cfg!(target_os = "macos") && app.integrated_titlebar {
        78
    } else if caption_left > 0.0 {
        0
    } else {
        10
    };
    let right = if caption_right > 0.0 { 0 } else { 10 };
    // A large UI Scale shrinks the measured lights below what the strip's controls need (9.3 pt at
    // 300% on a 1x display): the strip keeps its floor and its row stays on the lights, moved down
    // only as far as a control row needs.
    let mac_height = mac_bar.map(|b| b.height.max(MacTitleBar::MIN_HEIGHT));
    let mac_row = mac_bar.map(|b| b.height.max(ui.spacing().interact_size.y));
    let bar = egui::Panel::top("title_bar")
        .exact_size(mac_height.unwrap_or_else(|| crate::titlebar::bar_height(app, ui.ctx(), if t.pro { 32.0 } else { 38.0 })))
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left, right, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect().with_min_x(ui.max_rect().left() + mac_inset);
            // The menus, controls and title are laid out in `row`: on macOS, centred on the lights.
            let row = mac_row.map_or(full, |h| full.with_max_y(full.top() + h));
            let mut mac_row_ui = mac_row.map(|_| ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect().with_max_y(row.bottom())).layout(*ui.layout())));
            // Only the free gap between the menus and the far-end controls drags the window: a
            // press on a menu title must open the menu, never move the window. The gap is last
            // frame's, as the menus are laid out after this.
            let span_id = ui.scope_id().with("titledrag-span");
            let (gap_l, gap_r) = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(span_id)).unwrap_or((full.right(), full.right()));
            let gap = egui::Rect::from_x_y_ranges(gap_l.max(full.left())..=gap_r.min(full.right()).max(gap_l), full.y_range());
            ui.ctx().data_mut(|d| d.insert_temp(crate::titlebar::modal_drag_area_id(), gap));
            let drag = ui.interact(gap, ui.scope_id().with("titledrag"), Sense::click_and_drag());
            if drag.drag_started_by(egui::PointerButton::Primary) {
                crate::titlebar::start_os_move(ui.ctx());
            }
            if drag.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            // Preferences › Interface › Show File Name In Title: with it off the bar keeps the
            // app name and the unsaved dot, since the tabs already name the file (#5037).
            let show_name = app.session.prefs().interface.show_file_name_in_title;
            let title = app
                .session
                .active()
                .map(|d| format!("{}{}", if show_name { d.doc.name.as_str() } else { "PhotoCraft" }, if d.is_dirty() { "  •" } else { "" }))
                .unwrap_or_else(|| "PhotoCraft".into());
            // The menus and the far-end controls are laid out first; the title is centred in
            // whatever room is left between them (`free`, left to right: menus then controls, or
            // controls then menus in a right-to-left UI), shortened or dropped rather than drawn
            // over them.
            let rtl = crate::direction::is_rtl();
            let mut free = (full.left(), full.right());
            crate::direction::horizontal_centered(mac_row_ui.as_mut().unwrap_or(&mut *ui), |ui| {
                // The menus start beside the caption buttons at their end, if it has any.
                if !rtl && mac_inset > 0.0 {
                    ui.add_space(mac_inset);
                }
                let lead = if rtl { caption_right } else { caption_left };
                if lead > 0.0 {
                    ui.add_space(lead + 4.0);
                }
                if !system {
                    let side = if t.pro { 18.0 } else { 20.0 };
                    let (mark, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                    crate::brand::paint_mark(ui, mark);
                    ui.add_space(6.0);
                }
                // With the macOS menu bar the menus are at the top of the screen instead. Either way
                // this is the menus' inner edge, facing the title.
                let menus_end = if app.services.native_menu.is_some() {
                    if rtl { ui.cursor().right() } else { ui.cursor().left() }
                } else {
                    crate::menus::menu_bar(app, ui)
                };
                // The menu bar takes the whole row, so the far-end group gets its own rect: from
                // the menus to the bar's end, or to the caption buttons.
                let group = if rtl {
                    let left_edge = if caption_left > 0.0 { full.left() + caption_left + 4.0 } else { full.left() };
                    egui::Rect::from_min_max(egui::pos2(left_edge, row.top()), egui::pos2((menus_end - TITLE_GAP).max(left_edge), row.bottom()))
                } else {
                    let right_edge = if caption_right > 0.0 { full.right() - caption_right - 4.0 } else { full.right() };
                    egui::Rect::from_min_max(egui::pos2((menus_end + TITLE_GAP).min(right_edge), row.top()), egui::pos2(right_edge, row.bottom()))
                };
                let mut group_ui = ui.new_child(egui::UiBuilder::new().max_rect(group).layout(crate::direction::trailing()));
                let controls_end = {
                    let ui = &mut group_ui;
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // The icons keep a margin above and below them in a strip fitted to the lights.
                    let icon = mac_row.map_or(28.0, |h| (h - 6.0).min(28.0));
                    let mut ws = app.ui.workspace.clone();
                    let options = crate::workspace_ui::switcher_options(app);
                    let opts: Vec<(String, &str)> = options.iter().map(|(k, l)| (k.clone(), l.as_str())).collect();
                    // A narrow bar drops what is also in a menu, the theme toggle first
                    // (Preferences), then search (Edit › Search), and narrows the switcher
                    // (Window › Workspace) before anything runs over.
                    let room = ui.available_width();
                    let icons_shown: u8 = if room >= WORKSPACE_MIN + 2.0 * ICON_SLOT {
                        2
                    } else if room >= WORKSPACE_MIN + ICON_SLOT {
                        1
                    } else {
                        0
                    };
                    let ws_width = (room - f32::from(icons_shown) * ICON_SLOT - 6.0).clamp(WORKSPACE_MIN, 130.0);
                    if room >= WORKSPACE_MIN && widgets::dropdown(ui, "workspace", tl!("Workspace"), &mut ws, &opts, ws_width) {
                        app.ui.workspace = ws;
                        crate::menus::apply_workspace(app);
                    }
                    if icons_shown >= 1
                        && icons::button(ui, "search", icon, app.ui.palette_open, &crate::shortcuts::tip_label(app, "Search commands", "edit.search")).clicked()
                    {
                        app.ui.palette_open = !app.ui.palette_open;
                    }
                    let (theme_icon, appearance_label) = match app.session.prefs().interface.appearance_mode {
                        photocraft_engine::prefs::AppearanceMode::Auto => ("monitor", tl!("Auto")),
                        photocraft_engine::prefs::AppearanceMode::Dark => ("moon", tl!("Dark")),
                        photocraft_engine::prefs::AppearanceMode::Light => ("sun", tl!("Light")),
                    };
                    let appearance_tip = format!("{}: {}", tl!("Appearance Mode"), appearance_label);
                    if icons_shown >= 2 {
                        let button = icons::button(ui, theme_icon, icon, false, &appearance_tip);
                        if button.clicked() {
                            crate::prefs_ui::cycle_appearance(app, ui.ctx());
                        }
                        button.context_menu(|ui| crate::prefs_ui::appearance_context_menu(app, ui));
                    }
                    if rtl { ui.min_rect().right() } else { ui.min_rect().left() }
                };
                free = if rtl { (controls_end, menus_end) } else { (menus_end, controls_end) };
            });
            ui.ctx().data_mut(|d| d.insert_temp(span_id, free));
            if system {
                return;
            }
            let font = theme::medium(13.0);
            let galley = ui.painter().layout_job(title_job(&title, font.clone(), t.text_dim));
            if let Some(x) = title_x(full.center().x, free.0, free.1, galley.size().x) {
                ui.painter().galley(egui::pos2(x, row.center().y - galley.size().y / 2.0), galley, t.text_dim);
            } else if free.1 - free.0 > 80.0 {
                // Too narrow for the whole name: show the start of it, elided, in the free gap
                // next to the menus.
                let avail = free.1 - free.0 - 2.0 * TITLE_GAP;
                let mut job = title_job(&title, font, t.text_dim);
                job.wrap = egui::text::TextWrapping::truncate_at_width(avail);
                let g = ui.painter().layout_job(job);
                let x = if rtl { free.1 - TITLE_GAP - g.size().x } else { free.0 + TITLE_GAP };
                ui.painter().galley(egui::pos2(x, row.center().y - g.size().y / 2.0), g, t.text_dim);
            }
        });
    // A click anywhere on the bar keeps a zoom being typed (zoom_field.rs).
    crate::zoom_field::set_title_bar_rect(ui.ctx(), bar.response.rect);
    if custom {
        crate::titlebar::caption_buttons(app, ui, bar.response.rect);
    }
}

/// Whether the window shows the system's title bar instead of the app drawing its own: Windows
/// and Linux with Preferences › Interface › System Title Bar. macOS always has system decorations,
/// but keeps the in-app brand mark and centred title, so it never counts as system mode here; nor
/// does the web build (the browser tab has no document title), nor an embedder or the snapshot
/// example that never turned the preference on.
pub fn system_title_bar(app: &PhotocraftApp) -> bool {
    !app.custom_titlebar && app.session.prefs().interface.system_title_bar && !cfg!(any(target_os = "macos", target_arch = "wasm32"))
}

/// The OS window title: the active document's name suffixed with the app name (a `*` prefix
/// marks unsaved changes, like the `•` in the app-drawn title), or just the app name with no
/// document open. With Preferences › Interface › Show File Name In Title off, the file name
/// drops out and only the app name (and the `*`) remains (#5037).
pub fn window_title(app: &PhotocraftApp) -> String {
    if !app.session.prefs().interface.show_file_name_in_title {
        return format!("{}PhotoCraft", if app.session.active().is_some_and(|d| d.is_dirty()) { "*" } else { "" });
    }
    app.session.active().map(|d| format!("{}{} \u{2014} PhotoCraft", if d.is_dirty() { "*" } else { "" }, d.doc.name)).unwrap_or_else(|| "PhotoCraft".into())
}

/// Keep the OS window title (and the taskbar / Alt-Tab entry) on the active file. Sends
/// `ViewportCommand::Title` only when the title changed since the last frame.
pub fn sync_window_title(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let want = window_title(app);
    if app.last_window_title != want {
        app.last_window_title = want.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(want));
    }
}

/// The in-app title line. It reads left to right as a whole, so a right-to-left document name keeps
/// "— PhotoCraft" after it, at the right; egui orders the name's own words.
pub(crate) fn title_job(title: &str, font: egui::FontId, color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), font, color);
    job.direction = egui::epaint::text::TextDirection::LeftToRight;
    job
}

/// Minimum space kept between the window title and the menus or controls beside it.
const TITLE_GAP: f32 = 16.0;

/// The macOS title strip around the traffic lights: its height and where its content starts after
/// them. AppKit draws the lights itself and their size and place change between macOS versions
/// (larger and lower from macOS 26), so the shell measures the real buttons (`traffic_lights.rs`)
/// rather than assuming one layout (#2554, as WordCraft's #1559).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacTitleBar {
    pub height: f32,
    pub content_left: f32,
}

impl MacTitleBar {
    /// The strip's lowest height (WordCraft's): its controls keep a margin above and below them.
    /// In AppKit points when measured, and in egui points too, where a large UI Scale converts
    /// the lights below it.
    pub const MIN_HEIGHT: f32 = 28.0;

    /// The strip for traffic lights whose right edge is `lights_right` and whose centre is
    /// `lights_centre_y` below the top of the window, `light` tall: the lights centred in it, and
    /// content the same gap after them as they keep from the top and bottom. Never lower than 28 pt.
    /// All in AppKit window points.
    pub fn around_lights(lights_right: f32, lights_centre_y: f32, light: f32) -> Self {
        let height = (2.0 * lights_centre_y).max(Self::MIN_HEIGHT);
        let gap = (height - light) / 2.0;
        Self { height, content_left: (lights_right + gap).max(0.0) }
    }

    /// The strip in egui points under egui's `zoom_factor`: AppKit window points are egui points
    /// times `pixels_per_point / native_pixels_per_point`, which is the zoom. PhotoCraft's UI Scale
    /// sets the zoom (a fixed scale is absolute: 100% on a 2x display is a 0.5 zoom); at Auto it is 1.
    pub fn in_egui_points(self, zoom_factor: f32) -> Self {
        if !(zoom_factor.is_finite() && zoom_factor > 0.0) {
            return self;
        }
        Self { height: self.height / zoom_factor, content_left: self.content_left / zoom_factor }
    }
}

/// The workspace switcher's narrowest width, and one 28 pt title-bar icon with its spacing.
const WORKSPACE_MIN: f32 = 90.0;

const ICON_SLOT: f32 = 34.0;

/// Left edge for a title `width` wide: centred on `center` when it fits in the free gap from
/// `left` to `right` (the menus' and the controls' facing edges, whichever side each is on),
/// else slid into that gap; None when the gap is too small for the whole title.
pub(super) fn title_x(center: f32, left: f32, right: f32, width: f32) -> Option<f32> {
    let (lo, hi) = (left + TITLE_GAP, right - TITLE_GAP - width);
    if !(lo.is_finite() && hi.is_finite() && width.is_finite()) || hi < lo {
        return None;
    }
    if !center.is_finite() {
        return Some(lo);
    }
    Some((center - width / 2.0).clamp(lo, hi))
}

#[cfg(test)]
mod mac_title_bar_tests {
    use super::MacTitleBar;

    /// #2554: the strip is as tall as the measured lights need to sit centred in it, and its
    /// content starts the same gap after them; never lower than 28 pt.
    #[test]
    fn the_strip_fits_around_the_measured_lights() {
        // macOS 26+: 14 pt lights centred 16 pt down, the zoom button ending at 80 pt.
        let bar = MacTitleBar::around_lights(80.0, 16.0, 14.0);
        assert_eq!(bar.height, 32.0);
        assert_eq!(bar.content_left, 89.0, "the gap after the lights equals the gap above them");
        // The classic 12 pt lights centred 14 pt down: a 28 pt strip.
        assert_eq!(MacTitleBar::around_lights(70.0, 14.0, 12.0), MacTitleBar { height: 28.0, content_left: 78.0 });
        // Never lower than 28 pt.
        assert_eq!(MacTitleBar::around_lights(40.0, 10.0, 12.0).height, 28.0);
    }

    /// #2554: the measurement is in AppKit points, egui points only at UI Scale Auto. A fixed UI
    /// Scale zooms egui by `pixels_per_point / native_pixels_per_point`, so the strip converts.
    #[test]
    fn the_strip_converts_from_appkit_points_by_the_ui_zoom() {
        let bar = MacTitleBar::around_lights(70.0, 14.0, 12.0);
        // UI Scale Auto: unchanged.
        assert_eq!(bar.in_egui_points(1.0), bar);
        // A 2x display at UI Scale 100% (pixels_per_point 1): twice as many egui points, past
        // what `egui::Margin` holds.
        assert_eq!(bar.in_egui_points(1.0 / 2.0), MacTitleBar { height: 56.0, content_left: 156.0 });
        // ... and at 250% (pixels_per_point 2.5).
        assert_eq!(bar.in_egui_points(2.5 / 2.0), MacTitleBar { height: 22.4, content_left: 62.4 });
        // A zoom egui never has leaves it as measured.
        assert_eq!(bar.in_egui_points(0.0), bar);
    }
}
