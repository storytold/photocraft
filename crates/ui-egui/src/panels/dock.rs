use super::history::history;
use super::info::info_panel;
use super::layers::{channels, layers};
use super::navigator::navigator;
use super::properties::{adjustments_grid, properties_body};
use super::{PhotocraftApp, Stroke, Tokens, icons, pos2};

// ----------------------------------------------------------------------------- dock

/// The panels and the canvas in the space left by the menus, bars and toolbar: craft-ui's
/// docking layout, with the icon rail at the window's end edge in the Studio themes.
pub fn dock_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if app.ui.panels.rail && !t.pro {
        rail(app, ui, &t);
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::panel_docking::show(app, ui));
    crate::dock::persist(app, ui.ctx());
    crate::color_panel_ui::persist(app);
}

/// The dock and its icon rail as one right panel, for the layout with the toolbox between the
/// canvas and the dock. The canvas is drawn separately afterwards. Keeping the rail inside this
/// panel means the toolbox cannot cover it when it is one column wide.
pub fn dock_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (rail_w, _) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
    let rail_shown = app.ui.panels.rail;
    let rw = if rail_shown { rail_w } else { 0.0 };
    let content = ui.available_rect_before_wrap();
    let range = crate::panels::dock_width_range(content.width());
    let (min_dock, max_dock) = (*range.start(), *range.end());
    let stored = crate::panel_docking::dock_width(app).clamp(min_dock, max_dock);
    let drag_id = egui::Id::unique("dock-edge-drag");
    let mut drag = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(drag_id));
    if drag.is_some() && !ui.ctx().input(|i| i.pointer.any_down()) {
        drag = None;
        ui.ctx().data_mut(|d| d.remove::<(f32, f32)>(drag_id));
    }
    let panel_w = if let Some((start_x, start_w)) = drag {
        let pointer = ui.ctx().input(|i| i.pointer.latest_pos()).map_or(start_x, |pos| pos.x);
        let dx = if crate::direction::is_rtl() { pointer - start_x } else { start_x - pointer };
        (start_w + dx).clamp(min_dock + rw, max_dock + rw)
    } else {
        stored + rw
    };
    if drag.is_some() {
        crate::panel_docking::set_end_dock_total(app, (panel_w - rw).clamp(min_dock, max_dock));
    }
    crate::direction::end_panel("dock-column")
        .resizable(false)
        .show_separator_line(false)
        .exact_size(panel_w)
        .frame(egui::Frame::NONE.fill(t.card))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let height = ui.available_height();
            let mut dock_rect = ui.max_rect();
            if rail_shown {
                // The rail sits between the dock (at the window edge) and the toolbox.
                let rail_rect = if crate::direction::is_rtl() {
                    egui::Rect::from_min_size(egui::pos2(dock_rect.right() - rw, dock_rect.top()), egui::vec2(rw, height))
                } else {
                    egui::Rect::from_min_size(dock_rect.min, egui::vec2(rw, height))
                };
                ui.allocate_rect(rail_rect, egui::Sense::hover());
                let mut rail_ui = ui.new_child(egui::UiBuilder::new().max_rect(rail_rect));
                rail_body(app, &mut rail_ui, &t);
                if crate::direction::is_rtl() {
                    dock_rect.max.x = rail_rect.left();
                } else {
                    dock_rect.min.x = rail_rect.right();
                }
            }
            let mut dock_ui = ui.new_child(egui::UiBuilder::new().max_rect(dock_rect));
            crate::panel_docking::show_dock(app, &mut dock_ui);
            // The panel's inner edge (between the toolbox and the rail) is the resize handle.
            // The docking layout's own column-edge handle at the dock's content edge is covered
            // below, so it cannot draw the second, overlaid bar.
            let panel_rect = ui.max_rect();
            let panel_edge = if crate::direction::is_rtl() { panel_rect.right() } else { panel_rect.left() };
            // Block the docking layout's column-edge handle at the dock's content edge. It is
            // registered before the resize handle so the handle wins where they overlap when the
            // rail is hidden and both edges coincide.
            let dock_edge = if crate::direction::is_rtl() { dock_rect.right() } else { dock_rect.left() };
            let cover = egui::Rect::from_min_max(egui::pos2(dock_edge - 2.0, dock_rect.top()), egui::pos2(dock_edge + 2.0, dock_rect.bottom()));
            let _ = ui.interact(cover, egui::Id::unique("dock-content-edge"), egui::Sense::click_and_drag());
            let handle =
                egui::Rect::from_min_max(egui::pos2(panel_edge - 4.0, panel_rect.top()), egui::pos2(panel_edge + 4.0, panel_rect.bottom()));
            let response = ui.interact(handle, egui::Id::unique("dock-edge-handle"), egui::Sense::drag());
            // The bar is always visible, on the foreground layer so no panel, rail or dock
            // content can hide it; it brightens under the pointer.
            let active = response.hovered() || response.dragged();
            if active {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            let line = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("dock-edge-line"));
            // The theme's separator colour at rest, the accent while the edge is hovered or
            // dragged: a dark divider that lights up, not a permanently lit bar.
            let color = if active { t.accent } else { t.separator };
            ui.ctx().layer_painter(line).vline(panel_edge, panel_rect.y_range(), Stroke::new(1.0, color));
            if response.drag_started() {
                let start_x = response.interact_pointer_pos().map_or(panel_edge, |pos| pos.x);
                ui.ctx().data_mut(|d| d.insert_temp(drag_id, (start_x, panel_w)));
            }
            if response.dragged() {
                ui.ctx().request_repaint();
            }
            if response.drag_stopped() {
                ui.ctx().data_mut(|d| d.remove::<(f32, f32)>(drag_id));
            }
        });
    crate::dock::persist(app, ui.ctx());
    crate::color_panel_ui::persist(app);
}

/// The docking layout's protected canvas panel: the document area, with the icon rail on its end
/// side, beside the dock, in the Pro themes.
pub(crate) fn canvas_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let backdrop = if app.ui.view.hides_chrome() { egui::Color32::BLACK } else { crate::prefs_ui::pasteboard_color(app).unwrap_or(t.canvas) };
    if app.ui.panels.rail && t.pro {
        rail(app, ui, &t);
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(backdrop)).show(ui, |ui| crate::canvas::document_area(app, ui));
}

/// Narrow icon rail, unless an embedding app hides it: shows, expands or collapses panel groups.
fn rail(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let (rw, _) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
    crate::direction::end_panel("rail")
        .resizable(false)
        .exact_size(rw)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 8)))
        .show(ui, |ui| rail_body(app, ui, t));
}

/// The rail's buttons, without the panel that normally holds them.
fn rail_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let customized = app.ui.docking.is_some();
    let p = app.ui.panels.clone();
    let (_, rb) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
    // The separator is on the canvas side.
    let r = ui.max_rect();
    let x = if crate::direction::is_rtl() { r.right() + 6.0 } else { r.left() - 6.0 };
    ui.painter().line_segment([pos2(x, r.top() - 8.0), pos2(x, r.bottom() + 8.0)], Stroke::new(1.0, t.separator));
    ui.spacing_mut().item_spacing.y = 4.0;
    use crate::dock::Group;
    let entries: [(&str, &str, Group); 6] = [
        ("sliders-horizontal", tl!("Properties"), Group::Properties),
        ("navigation", tl!("Navigator"), Group::Navigator),
        ("palette", tl!("Color & Swatches"), Group::Color),
        ("layers", tl!("Layers"), Group::Layers),
        ("clock", tl!("History"), Group::History),
        ("brush", tl!("Brushes"), Group::Brushes),
    ];
    for (icon, name, g) in entries {
        // Studio floats Properties outside the dock.
        let docked = t.pro || g != Group::Properties;
        let on = if customized { crate::panel_docking::rail_on(app, g) } else { g.shown(&p) && !(docked && app.ui.dock.is_collapsed(g)) };
        if icons::rail_button(ui, icon, rb, on, name).clicked() {
            if customized {
                crate::panel_docking::rail_click(app, g);
            } else {
                crate::dock::rail_click(app, g, docked);
            }
        }
    }
}

/// The dock's narrowest width (points). It is the right dock, on the left in a right-to-left UI.
pub(crate) const DOCK_MIN_WIDTH: f32 = 250.0;

/// What the dock leaves the toolbar and the canvas when it is widest (points).
const DOCK_LEAVES: f32 = 400.0;

/// The dock's width range (points) in a window `window_width` wide: from [`DOCK_MIN_WIDTH`] to
/// everything but [`DOCK_LEAVES`], and never capped below the old 520, so two-column layouts can
/// be as wide as the window allows (#7896).
pub(crate) fn dock_width_range(window_width: f32) -> std::ops::RangeInclusive<f32> {
    let max = if window_width.is_finite() { (window_width - DOCK_LEAVES).max(520.0) } else { 520.0 };
    DOCK_MIN_WIDTH..=max
}

pub(crate) fn dock_width_id() -> egui::Id {
    egui::Id::unique("dock-width-request")
}

/// Set the right dock's width on the next frame (`ui.set {dockWidth}`); clamped to its range.
pub fn request_dock_width(ctx: &egui::Context, w: f32) {
    let range = dock_width_range(ctx.content_rect().width());
    let w = if w.is_finite() { w.clamp(*range.start(), *range.end()) } else { *range.start() };
    ctx.data_mut(|d| d.insert_temp(dock_width_id(), w));
}

/// One dock group's tab content; `panel_docking` bounds it and scrolls it when it's taller.
pub(crate) fn dock_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, group: crate::dock::Group, tab: usize) {
    use crate::dock::Group;
    let pro = Tokens::get(ui.ctx()).pro;
    match (group, tab) {
        (Group::Color, 2) => crate::preset_panels::gradients_panel(app, ui),
        (Group::Color, 3) => crate::preset_panels::patterns_panel(app, ui),
        // The Color tab is the same in every theme, in the mode picked from its panel menu
        // (#6000); only the tab order differs between themes.
        (Group::Color, tab) if group.tabs(pro).get(tab) == Some(&"Color") => crate::color_panel_ui::panel(app, ui),
        (Group::Color, _) => crate::swatches_ui::panel(app, ui),
        (Group::Properties, 0) => properties_body(app, ui),
        (Group::Properties, _) => adjustments_grid(app, ui),
        (Group::Character, tab) => crate::type_tool::character_panel(app, ui, tab == 1),
        (Group::Navigator, 0) => navigator(app, ui),
        (Group::Navigator, 1) => crate::histogram_panel::show(app, ui),
        (Group::Navigator, _) => info_panel(app, ui),
        (Group::History, 0) => history(app, ui),
        (Group::History, 1) => crate::actions::panel(app, ui),
        (Group::History, _) => crate::comps_ui::panel(app, ui),
        (Group::Layers, 0) => layers(app, ui),
        (Group::Layers, 1) => channels(app, ui),
        (Group::Layers, _) => crate::vector_ui::paths_panel(app, ui),
        (Group::Brushes, _) => crate::brushes_tab::show(app, ui),
    }
}
