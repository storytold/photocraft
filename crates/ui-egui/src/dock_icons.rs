//! Collapse to Icons (#4059): the dock column turns into a strip of panel icons and back, like
//! Photoshop's dock columns.
//!
//! The bar on top of the column holds the double arrow: it collapses the column to icons
//! (Collapse to Icons) and expands it again (Expand Panels); double-clicking the bar or its
//! context menu, and every panel menu, do the same. On the strip each visible panel is an icon,
//! groups set apart by a line. Clicking an icon opens its group as a flyout beside the strip on
//! that panel; clicking the icon again closes it, and with Preferences ▸ Workspace ▸
//! Auto-Collapse Iconic Panels on, so does a click anywhere outside the flyout.
//!
//! The state is [`crate::dock::DockLayout::iconic`] (`ui.set {"dock": {"iconic": true}}`), saved
//! with workspaces and remembered with the rest of the layout. Which flyout is open
//! ([`crate::dock::DockLayout::flyout`]) is transient; Window › <panel> and the icon rail open it.

use egui::{Rect, Sense, Stroke, pos2, vec2};

use crate::PhotocraftApp;
use crate::dock::Group;
use crate::theme::Tokens;

/// Height of the bar above the dock column.
pub fn bar_height(pro: bool) -> f32 {
    if pro { 14.0 } else { 18.0 }
}

/// What the double arrow and the menus do next.
pub fn toggle_label(iconic: bool) -> &'static str {
    if iconic { tl!("Expand Panels") } else { tl!("Collapse to Icons") }
}

/// Collapse the column to icons, or expand it again; either way no flyout stays open.
pub fn set_iconic(app: &mut PhotocraftApp, on: bool) {
    app.ui.dock.iconic = on;
    app.ui.dock.flyout = None;
}

/// The icon a panel shows on the strip (and its tooltip is the panel's name).
pub fn tab_icon(name: &str) -> &'static str {
    match name {
        "Color" => "palette",
        "Swatches" => "swatch-book",
        "Gradients" => "gradient",
        "Patterns" => "grid-2x2",
        "Properties" => "sliders-horizontal",
        "Adjustments" => "contrast",
        "Character" => "type",
        "Paragraph" => "align-left",
        "Navigator" => "navigation",
        "Histogram" => "gauge",
        "Info" => "info",
        "History" => "clock",
        "Actions" => "play",
        "Layer Comps" => "copy",
        "Layers" => "layers",
        "Channels" => "blend",
        "Paths" => "pen-tool",
        "Brushes" => "brush",
        _ => "panels-top-left",
    }
}

fn flyout_id() -> egui::Id {
    egui::Id::new("dock-icons-flyout")
}

fn strip_id() -> egui::Id {
    egui::Id::new("dock-icons-strip")
}

/// `(group, tab, rect)` of the icons drawn last frame, for tests and automation.
pub fn last_icons(ctx: &egui::Context) -> Vec<(Group, usize, Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(Group, usize, Rect)>>(strip_id().with("icons"))).unwrap_or_default()
}

/// The open flyout's rect last frame.
pub fn last_flyout(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Option<Rect>>(flyout_id())).flatten()
}

/// The bar on top of the dock column with the Collapse to Icons / Expand Panels double arrow.
pub fn column_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let h = bar_height(t.pro);
    let iconic = app.ui.dock.iconic;
    let (rect, bar) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    ui.painter().rect_filled(rect, 0.0, t.chrome);
    // The arrows point where the column goes: towards the window edge to collapse, back to expand.
    let rtl = crate::direction::is_rtl();
    let icon = if iconic == rtl { "chevrons-right" } else { "chevrons-left" };
    let at = if rtl { rect.left_top() } else { pos2(rect.right() - h, rect.top()) };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(at, vec2(h, h))));
    let label = toggle_label(iconic);
    let button = crate::icons::button_with_icon_size(&mut child, icon, h, h - 2.0, false, label);
    let mut toggle = button.clicked() || bar.double_clicked();
    bar.context_menu(|ui| {
        crate::widgets::style_spectrum_popup_menu(ui);
        if ui.button(label).clicked() {
            toggle = true;
            ui.close();
        }
    });
    if toggle {
        set_iconic(app, !iconic);
    }
}

/// With Auto-Collapse Iconic Panels on, a press outside the flyout and the strip closes the
/// flyout. A press while a menu is open only closes the menu (it may be the flyout's own).
fn pressed_outside(ctx: &egui::Context) -> bool {
    let Some(p) = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten()) else {
        return false;
    };
    let strip = ctx.data(|d| d.get_temp::<Rect>(strip_id()));
    !egui::Popup::is_any_open(ctx) && !last_flyout(ctx).is_some_and(|r| r.contains(p)) && !strip.is_some_and(|r| r.contains(p))
}

/// Draw the column collapsed to icons: the `shown` groups (the layout decides their order) as
/// icons, and the open group's flyout drawn by [`crate::dock::show`] with `body`.
pub fn iconic_column(app: &mut PhotocraftApp, ui: &mut egui::Ui, shown: &[Group], body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui, Group, usize)) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    if shown.is_empty() {
        app.ui.dock.flyout = None;
        return;
    }
    let mut open = app.ui.dock.flyout.filter(|g| shown.contains(g) && !app.ui.dock.is_collapsed(*g));
    if open.is_some() && app.session.prefs().workspace.auto_collapse_icon_panels && pressed_outside(&ctx) {
        open = None;
    }
    let (width, size) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
    let order: Vec<Group> = app.ui.dock.order().into_iter().filter(|g| shown.contains(g)).collect();
    let mut icons: Vec<(Group, usize, Rect)> = Vec::new();
    let mut anchor: Option<Rect> = None;
    let strip = crate::direction::end_panel("dock-icons")
        .resizable(false)
        .exact_size(width)
        .frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::same(2)))
        .show(ui, |ui| {
            column_bar(app, ui);
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (n, g) in order.iter().copied().enumerate() {
                if n > 0 {
                    ui.add_space(2.0);
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().line_segment([line.left_center(), line.right_center()], Stroke::new(1.0, t.separator));
                    ui.add_space(2.0);
                }
                let selected = *g.tab_mut(&mut app.ui.dock_tabs);
                for (tab, name) in app.ui.dock.visible_tabs(g, t.pro) {
                    let on = open == Some(g) && selected == tab;
                    let resp = ui.vertical_centered(|ui| crate::icons::rail_button(ui, tab_icon(name), size, on, name)).inner;
                    if on {
                        anchor = Some(resp.rect);
                    }
                    if resp.clicked() {
                        if on {
                            open = None;
                            anchor = None;
                        } else {
                            // A panel opened from its icon always shows expanded.
                            *g.tab_mut(&mut app.ui.dock_tabs) = tab;
                            app.ui.dock.set_collapsed(g, false);
                            open = Some(g);
                            anchor = Some(resp.rect);
                        }
                    }
                    icons.push((g, tab, resp.rect));
                }
            }
            ui.max_rect()
        })
        .inner;
    // The bar's double arrow may just have expanded the column.
    if !app.ui.dock.iconic {
        open = None;
    }
    // The selected panel may have no icon (closed by Close Tab): anchor to the group's first one.
    if anchor.is_none() {
        anchor = open.and_then(|o| icons.iter().find(|(g, _, _)| *g == o)).map(|(_, _, r)| *r);
    }
    app.ui.dock.flyout = open;
    ctx.data_mut(|d| {
        d.insert_temp(strip_id(), strip.expand(2.0));
        d.insert_temp(strip_id().with("icons"), icons);
    });
    let flyout = match (open, anchor) {
        (Some(g), Some(anchor)) => Some(flyout(app, &ctx, g, anchor, strip, body)),
        _ => None,
    };
    ctx.data_mut(|d| d.insert_temp(flyout_id(), flyout));
    // The flyout's own menus can collapse its group, close it or expand the column.
    if let Some(g) = open
        && (!app.ui.dock.iconic || app.ui.dock.is_collapsed(g) || !g.shown(&app.ui.panels))
    {
        app.ui.dock.flyout = None;
    }
}

/// Group `g` beside the strip, top-aligned with its icon and kept on screen.
fn flyout(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    g: Group,
    anchor: Rect,
    strip: Rect,
    mut body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui, Group, usize),
) -> Rect {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let width = if t.pro { 290.0 } else { 300.0 };
    let height = app.ui.dock.height(g).max(g.preferred_fill()).min((screen.height() - 8.0).max(0.0));
    let x = if crate::direction::is_rtl() { strip.right() + 1.0 } else { strip.left() - 1.0 - width };
    let y = anchor.top().min(screen.bottom() - 4.0 - height).max(screen.top());
    let margin = if t.pro { 2.0 } else { 8.0 };
    egui::Area::new(flyout_id())
        .order(egui::Order::Middle)
        .fixed_pos(pos2(x, y))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, t.dock);
            ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
            let mut inner = ui.new_child(egui::UiBuilder::new().id_salt("dock-icons-flyout").max_rect(rect.shrink(margin)));
            crate::dock::show(app, &mut inner, &[g], &mut body);
            rect
        })
        .inner
}

#[cfg(test)]
#[path = "dock_icons_tests.rs"]
mod tests;
