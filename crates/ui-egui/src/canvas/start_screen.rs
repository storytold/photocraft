use super::tabs::{open_on_double_click, paint_dots};
use super::{PhotocraftApp, Rect, Sense};

/// The Home screen's card is this wide when the canvas has room, and as wide as the canvas when it doesn't.
const HOME_CARD_WIDTH: f32 = 460.0;

/// Least width of each of the Home screen's New document and Open buttons.
const HOME_BUTTON_WIDTH: f32 = 190.0;

/// Space between the Home screen's two buttons.
const HOME_BUTTON_GAP: f32 = 12.0;

pub(super) fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    paint_dots(ui, area, app.session.prefs().interface.show_canvas_dots);
    open_on_double_click(app, ui, area);
    // File › Open Recent, newest first (on the web there are no paths to reopen).
    let show_recent = app.session.prefs().interface.show_home_recent_files;
    let recent: Vec<String> =
        if cfg!(target_arch = "wasm32") || !show_recent { Vec::new() } else { app.ui.recent_files.iter().take(HOME_RECENT).cloned().collect() };
    let recent_h = if recent.is_empty() { 0.0 } else { 34.0 + recent.len() as f32 * HOME_RECENT_ROW };
    // A tiling window manager can give the canvas less room than the card wants: shrink the card
    // to fit, and stack the buttons when they don't fit side by side.
    let new_label = crate::shortcuts::command_label(app, "New document…", "file.new");
    let open_label = crate::shortcuts::command_label(app, "Open…", "file.open");
    let card_width = HOME_CARD_WIDTH.min(area.width());
    let button_width = HOME_BUTTON_WIDTH.max(crate::widgets::button_width(ui, &[&new_label, &open_label]));
    let side_by_side = 2.0 * button_width + HOME_BUTTON_GAP <= card_width;
    // Stacked buttons take one more row.
    let stacked_h = if side_by_side { 0.0 } else { 44.0 };
    let card = Rect::from_center_size(area.center(), egui::vec2(card_width, 330.0 + recent_h + stacked_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        ui.vertical_centered(|ui| {
            crate::direction::full_row(ui, |ui| {
                let title = ui.painter().layout_no_wrap(tl!("PhotoCraft").into(), crate::theme::semibold(38.0), t.text);
                let by = ui.painter().layout_no_wrap(tl!("open source").into(), egui::FontId::proportional(13.0), t.text_faint);
                let total = title.size().x + by.size().x + 10.0;
                ui.add_space(((card.width() - total) / 2.0).max(0.0));
                let (r, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ui.painter().galley(r.min, title, t.text);
                let _ = ui.allocate_exact_size(egui::vec2(by.size().x + 10.0, r.height()), Sense::hover());
                // The note sits past the title's trailing side: 10 px plus the row's item gap.
                let by_x = crate::direction::after_x(r, 10.0 + ui.spacing().item_spacing.x, by.size().x);
                ui.painter().galley(egui::pos2(by_x, r.bottom() - by.size().y - 8.0), by, t.text_faint);
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!("Create a new document or open an existing file.")).color(t.text_dim).size(14.0));
            ui.add_space(22.0);
            let (mut new_clicked, mut open_clicked) = (false, false);
            if side_by_side {
                crate::direction::full_row(ui, |ui| {
                    ui.add_space(((card.width() - 2.0 * button_width - HOME_BUTTON_GAP) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = HOME_BUTTON_GAP;
                    new_clicked = crate::widgets::primary_button(ui, &new_label, button_width).clicked();
                    open_clicked = crate::widgets::secondary_button(ui, &open_label, button_width).clicked();
                });
            } else {
                new_clicked = crate::widgets::primary_button(ui, &new_label, button_width).clicked();
                ui.add_space(HOME_BUTTON_GAP);
                open_clicked = crate::widgets::secondary_button(ui, &open_label, button_width).clicked();
            }
            if new_clicked {
                let fields = app.new_document_fields();
                app.ui.open_dialog(crate::state::DialogKind::NewDocument, fields);
            }
            if open_clicked {
                let _ = app.open_dialog_file();
            }
            ui.add_space(22.0);
            crate::direction::full_row(ui, |ui| {
                let msg = start_screen_drop_hint(app);
                let g = ui.painter().layout_no_wrap(msg.to_string(), egui::FontId::proportional(12.5), t.text_faint);
                ui.add_space(((card.width() - g.size().x - 24.0) / 2.0).max(0.0));
                let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                crate::icons::paint(ui, r, "image", 15.0, t.text_faint);
                ui.label(egui::RichText::new(msg).color(t.text_faint));
            });
            if !recent.is_empty() {
                ui.add_space(22.0);
                home_recent(app, ui, &recent);
            }
            ui.add_space(26.0);
            crate::links::discord_button(app, ui, 190.0);
            ui.add_space(10.0);
            crate::links::link_row(app, ui);
        });
    });
}

/// Wayland has no native file drops (winit 0.30, #386), so the hint offers File › Open and paste.
pub(super) fn start_screen_drop_hint(app: &PhotocraftApp) -> std::borrow::Cow<'static, str> {
    if app.services.is_wayland {
        crate::i18n::fmt(tl!("Use File › Open, or paste a copied image with {paste}."), &[("paste", &crate::notices::paste_hint(app))]).into()
    } else {
        tl!("Drop an image or PSD anywhere to open it.").into()
    }
}

/// Recent files listed on the Home screen.
const HOME_RECENT: usize = 6;

/// Height of one Home-screen recent-file row.
const HOME_RECENT_ROW: f32 = 30.0;

/// The parts of a recent-file row that mirror with the UI direction.
struct RecentRow {
    /// The × that takes the file off the list, at the reading end.
    close: Rect,
    /// The bound the folder text is anchored to: its right edge left-to-right, its left edge
    /// right-to-left.
    end: f32,
    /// The file icon, at the reading start.
    icon: Rect,
    /// The left edge of the file name.
    name_x: f32,
    /// How much room the folder text gets.
    folder_max: f32,
}

/// Where a recent-file row's parts sit: the file icon at the reading start, the × at the reading
/// end, and the file name and folder between them.
fn recent_row(row: Rect, name_w: f32, hovered: bool) -> RecentRow {
    let close = Rect::from_center_size(egui::pos2(crate::direction::end_x(row, 16.0), row.center().y), egui::vec2(22.0, 22.0));
    let end = if hovered { crate::direction::start_x(close, -4.0) } else { crate::direction::end_x(row, 10.0) };
    let icon = Rect::from_center_size(egui::pos2(crate::direction::start_x(row, 16.0), row.center().y), egui::vec2(16.0, 16.0));
    let name_x = crate::direction::start_x_w(row, 32.0, name_w);
    let name_rect = Rect::from_min_size(egui::pos2(name_x, row.top()), egui::vec2(name_w, row.height()));
    let folder_max = (crate::direction::step() * (end - crate::direction::end_x(name_rect, 0.0)) - 14.0).max(0.0);
    RecentRow { close, end, icon, name_x, folder_max }
}

/// The Home screen's "Recent" list: file name, its folder at the reading end, click to open. The
/// hovered row shows a × that takes it off the list, and right-click offers Remove from Recent
/// and Clear Recent Files (#2691).
fn home_recent(app: &mut PhotocraftApp, ui: &mut egui::Ui, recent: &[String]) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let width = 380.0;
    let rtl = crate::direction::is_rtl();
    crate::direction::full_row(ui, |ui| {
        // Line the heading up with the file icons.
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0) + 8.0);
        ui.label(egui::RichText::new(tl!("Recent")).font(crate::theme::semibold(12.5)).color(t.text_dim));
    });
    ui.add_space(4.0);
    let mut open = None;
    let mut remove = None;
    let mut clear = false;
    for path in recent {
        let name = crate::file_open::display_name(path);
        let folder = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let (row, resp) = ui.allocate_exact_size(egui::vec2(width, HOME_RECENT_ROW), Sense::click());
        // The × sits on top of the row, so the row counts as hovered while the pointer is on it.
        let hovered = ui.rect_contains_pointer(row);
        if hovered {
            ui.painter().rect_filled(row, t.radius_sm, t.hover);
        }
        let name_g = crate::tab_strip::elided(ui, &name, egui::FontId::proportional(12.5), t.text, 170.0);
        let name_w = name_g.size().x;
        let parts = recent_row(row, name_w, hovered);
        crate::icons::paint(ui, parts.icon, "file", 14.0, t.text_dim);
        ui.painter().galley(egui::pos2(parts.name_x, row.center().y - name_g.size().y / 2.0), name_g, t.text);
        if parts.folder_max > 20.0 && !folder.is_empty() {
            let fg = crate::tab_strip::elided(ui, &folder, egui::FontId::proportional(11.5), t.text_faint, parts.folder_max);
            let fx = if rtl { parts.end } else { parts.end - fg.size().x };
            ui.painter().galley(egui::pos2(fx, row.center().y - fg.size().y / 2.0), fg, t.text_faint);
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        let resp = resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand);
        if hovered {
            let x = ui.interact(parts.close, resp.id.with("remove"), Sense::click());
            let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", &name)]);
            x.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            let color = crate::icons::button_chrome(ui, parts.close, false, x.hovered());
            crate::icons::paint(ui, parts.close, "x", 12.0, color);
            if x.on_hover_text(tl!("Remove from Recent")).clicked() {
                remove = Some(path.clone());
            }
        }
        if resp.clicked() && remove.is_none() {
            open = Some(path.clone());
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("Remove from Recent")).clicked() {
                remove = Some(path.clone());
                ui.close();
            }
            if ui.button(tl!("Clear Recent Files")).clicked() {
                clear = true;
                ui.close();
            }
        });
    }
    if clear {
        app.clear_recent();
    } else if let Some(path) = remove {
        app.remove_recent(&path);
    }
    if let Some(path) = open
        && let Err(e) = app.open_path(&path)
    {
        app.open_failed(&crate::file_open::display_name(&path), &e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A right-to-left recent row is the left-to-right row mirrored about the row's centre.
    #[test]
    fn recent_row_mirrors_in_rtl() {
        let row = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(380.0, 30.0));
        let mirror = |x: f32| row.left() + row.right() - x;
        for hovered in [false, true] {
            let (ltr, rtl) = {
                let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
                let ltr = recent_row(row, 120.0, hovered);
                let rtl = {
                    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
                    recent_row(row, 120.0, hovered)
                };
                (ltr, rtl)
            };
            assert!((mirror(ltr.close.center().x) - rtl.close.center().x).abs() < 0.01);
            assert!((mirror(ltr.icon.center().x) - rtl.icon.center().x).abs() < 0.01);
            assert!((mirror(ltr.end) - rtl.end).abs() < 0.01);
            assert!((ltr.name_x + rtl.name_x + 120.0 - row.width()).abs() < 0.01);
            assert!((ltr.folder_max - rtl.folder_max).abs() < 0.01);
        }
    }

    /// The "open source" note is mirrored with the title in a right-to-left UI.
    #[test]
    fn note_mirrors_in_rtl() {
        let row_w = 460.0;
        let title_w = 180.0;
        let note_w = 60.0;
        let spacing = 8.0;
        let lead = (row_w - (title_w + note_w + 10.0)) / 2.0;
        // The note sits past the title's trailing side: 10 px plus the row's item gap.
        let off = 10.0 + spacing;
        let (ltr_x, rtl_x) = {
            let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
            let ltr_title = Rect::from_min_size(egui::pos2(lead, 0.0), egui::vec2(title_w, 20.0));
            let ltr_x = crate::direction::after_x(ltr_title, off, note_w);
            let rtl_x = {
                let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
                let rtl_title = Rect::from_min_size(egui::pos2(row_w - lead - title_w, 0.0), egui::vec2(title_w, 20.0));
                crate::direction::after_x(rtl_title, off, note_w)
            };
            (ltr_x, rtl_x)
        };
        assert!((ltr_x + rtl_x + note_w - row_w).abs() < 0.01);
    }
}
