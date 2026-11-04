//! The Home screen's Recent files drawn as cards or as a list, and what clicking them does.
//!
//! Drawing only asks [`crate::recent_thumbs::RecentView`] for what it needs: a card off screen is
//! not drawn and not asked for, and nothing here touches the disk.

use egui::{Align2, Color32, Rect, Sense, Stroke, Vec2, pos2, vec2};
use photocraft_engine::prefs::HomeRecentSort;

use super::arrange::Row;
use crate::PhotocraftApp;
use crate::recent_thumbs::{FileStat, Preview, size_label, when_label};
use crate::theme::{Tokens, medium, semibold};

/// Narrowest a card is laid out.
const CARD_MIN_WIDTH: f32 = 190.0;
/// Height of a card's picture area.
const CARD_SURFACE_HEIGHT: f32 = 170.0;
/// Height of the name and time under it.
const CARD_TEXT_HEIGHT: f32 = 48.0;
/// Space between cards, across and down.
const CARD_GAP: Vec2 = vec2(22.0, 26.0);
/// The most room a card's picture takes inside its area.
const CARD_PICTURE: Vec2 = vec2(164.0, 136.0);
/// Space the picture leaves at either side of a card narrower than [`CARD_PICTURE`] allows.
const CARD_PICTURE_MARGIN: f32 = 26.0;

const LIST_ROW_HEIGHT: f32 = 50.0;
const LIST_HEADER_HEIGHT: f32 = 30.0;
const LIST_PAD_LEFT: f32 = 10.0;
/// Right padding of a list row: room for its × button.
const LIST_PAD_RIGHT: f32 = 40.0;
const LIST_GAP: f32 = 14.0;
const LIST_SIZE_WIDTH: f32 = 80.0;
const LIST_KIND_WIDTH: f32 = 70.0;
const LIST_WHEN_WIDTH_COMPACT: f32 = 92.0;
const LIST_THUMB: Vec2 = vec2(44.0, 36.0);
const LIST_PICTURE: Vec2 = vec2(40.0, 32.0);
/// A list narrower than this shows only the name and the time.
pub(super) const LIST_COMPACT_BELOW: f32 = 520.0;

const REMOVE_BUTTON: f32 = 24.0;

/// What the frame's clicks asked for, done once the lists are drawn.
#[derive(Default)]
pub(super) struct Actions {
    open: Option<String>,
    remove: Option<String>,
    reveal: Option<String>,
    clear: bool,
    /// A list header was clicked: sort by it, or flip the order when it already is.
    header: Option<HomeRecentSort>,
}

impl Actions {
    pub(super) fn apply(self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        if self.clear {
            app.clear_recent();
        } else if let Some(path) = &self.remove {
            app.remove_recent(path);
        }
        if let Some(path) = &self.reveal
            && let Err(e) = crate::menus::invoke(app, ctx, "file.revealInFinder", serde_json::json!({"path": path}))
        {
            app.ui.status = e;
        }
        if let Some(kind) = self.header {
            app.session.prefs.edit(|p| {
                let i = &mut p.interface;
                if i.home_recent_sort == kind {
                    i.home_recent_reverse = !i.home_recent_reverse;
                } else {
                    i.home_recent_sort = kind;
                    i.home_recent_reverse = false;
                }
            });
        }
        if let Some(path) = self.open {
            open(app, &path);
        }
    }
}

/// Open a recent file; a file known to be gone asks what to do instead (never merely fails).
fn open(app: &mut PhotocraftApp, path: &str) {
    if app.recent_view.status(path) == Some(FileStat::Missing) {
        crate::recent_missing_prompt::open(app, path);
    } else if let Err(e) = app.open_path(path) {
        app.open_failed(&crate::file_open::display_name(path), &e);
    }
}

/// The picture scaled to fit `max`, keeping its shape.
fn fit(size: [f32; 2], max: Vec2) -> Vec2 {
    let [w, h] = size;
    if w < 1.0 || h < 1.0 {
        return Vec2::ZERO;
    }
    let k = (max.x / w).min(max.y / h).max(0.0);
    vec2((w * k).round().max(1.0), (h * k).round().max(1.0))
}

/// Draw the thumbnail of `row` centred in `area`, no larger than `max`: dimmed when the file is
/// missing; a file icon, the kind and "No preview" when there is no thumbnail.
fn picture(app: &mut PhotocraftApp, ui: &egui::Ui, area: Rect, max: Vec2, row: &Row, missing: bool, compact: bool) {
    let t = Tokens::get(ui.ctx());
    match app.recent_view.preview(&row.path) {
        Preview::Image { id, size, .. } => {
            let r = Rect::from_center_size(area.center(), fit(size, max));
            if !missing && !compact {
                ui.painter().rect_filled(r.translate(vec2(0.0, 2.0)).expand(1.0), 0.0, t.shadow);
            }
            // A missing file's thumbnail is gray and about a third as strong.
            let tint = if missing { Color32::from_white_alpha(89) } else { Color32::WHITE };
            ui.painter().image(id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        Preview::Loading => {}
        Preview::None if compact => crate::icons::paint(ui, area, "file", 16.0, t.text_dim),
        Preview::None => {
            let c = area.center();
            crate::icons::paint(ui, Rect::from_center_size(pos2(c.x, c.y - 20.0), Vec2::splat(34.0)), "file", 34.0, t.text_dim);
            ui.painter().text(pos2(c.x, c.y + 8.0), Align2::CENTER_CENTER, &row.kind, semibold(12.0), t.text_dim);
            ui.painter().text(pos2(c.x, c.y + 24.0), Align2::CENTER_CENTER, tl!("No preview"), egui::FontId::proportional(11.0), t.text_faint);
        }
    }
}

/// The × that takes a file off the list; says whether it was clicked.
fn remove_button(ui: &mut egui::Ui, id: egui::Id, at: Rect, name: &str) -> bool {
    let x = ui.interact(at, id.with("remove"), Sense::click());
    let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", name)]);
    x.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let color = crate::icons::button_chrome(ui, at, false, x.hovered());
    crate::icons::paint(ui, at, "x", 12.0, color);
    x.on_hover_text(tl!("Remove from Recent")).clicked()
}

/// The right-click menu of a recent file.
fn menu(ui: &mut egui::Ui, path: &str, missing: bool, actions: &mut Actions) {
    if ui.button(tl!("Open")).clicked() {
        actions.open = Some(path.to_string());
        ui.close();
    }
    // A file that is gone cannot be shown.
    if ui.add_enabled(!missing, egui::Button::new(tl!(crate::menus::reveal_label()))).clicked() {
        actions.reveal = Some(path.to_string());
        ui.close();
    }
    ui.separator();
    if ui.button(tl!("Remove from Recent")).clicked() {
        actions.remove = Some(path.to_string());
        ui.close();
    }
    if ui.button(tl!("Clear Recent Files")).clicked() {
        actions.clear = true;
        ui.close();
    }
}

// ------------------------------------------------------------------ grid

/// The recent files as cards, `order` being the rows to show, in `width` points.
pub(super) fn grid(app: &mut PhotocraftApp, ui: &mut egui::Ui, width: f32, rows: &[Row], order: &[usize], now: u64, actions: &mut Actions) {
    let columns = (((width + CARD_GAP.x) / (CARD_MIN_WIDTH + CARD_GAP.x)).floor() as usize).max(1);
    let card_w = ((width - CARD_GAP.x * (columns - 1) as f32) / columns as f32).max(1.0);
    let card_h = CARD_SURFACE_HEIGHT + CARD_TEXT_HEIGHT;
    ui.add_space(18.0);
    for line in order.chunks(columns) {
        let (strip, _) = ui.allocate_exact_size(vec2(width, card_h), Sense::hover());
        for (i, &index) in line.iter().enumerate() {
            let Some(row) = rows.get(index) else { continue };
            let rect = Rect::from_min_size(pos2(strip.left() + i as f32 * (card_w + CARD_GAP.x), strip.top()), vec2(card_w, card_h));
            if ui.is_rect_visible(rect) {
                card(app, ui, rect, row, now, actions);
            }
        }
        ui.add_space(CARD_GAP.y);
    }
}

fn card(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, row: &Row, now: u64, actions: &mut Actions) {
    let t = Tokens::get(ui.ctx());
    let id = ui.id().with(("recent-card", row.path.as_str()));
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name));
    // The × sits over the card, so the card counts as hovered while the pointer is on it.
    let hovered = ui.rect_contains_pointer(rect);
    app.recent_view.want(&row.path, row.opened.unwrap_or(0));
    let missing = app.recent_view.status(&row.path) == Some(FileStat::Missing);

    let surface = Rect::from_min_size(rect.min, vec2(rect.width(), CARD_SURFACE_HEIGHT));
    ui.painter().rect_filled(surface, t.radius_sm, if hovered { t.hover } else { t.card });
    let max = vec2(CARD_PICTURE.x.min((surface.width() - CARD_PICTURE_MARGIN).max(8.0)), CARD_PICTURE.y);
    picture(app, ui, surface, max, row, missing, false);

    let name = crate::tab_strip::elided(ui, &row.name, medium(13.0), t.text, rect.width());
    let name_at = pos2(rect.left(), surface.bottom() + 9.0);
    let line_h = name.size().y;
    ui.painter().galley(name_at, name, t.text);
    let meta_at = pos2(rect.left(), name_at.y + line_h + 2.0);
    if missing {
        ui.painter().text(meta_at, Align2::LEFT_TOP, tl!("File not found"), egui::FontId::proportional(12.0), t.warning);
        missing_badge(ui, surface.min + vec2(8.0, 8.0));
    } else if let Some(opened) = row.opened {
        ui.painter().text(meta_at, Align2::LEFT_TOP, when_label(now, opened), egui::FontId::proportional(12.0), t.text_dim);
    }

    let mut removed = false;
    if hovered {
        let close = Rect::from_center_size(pos2(surface.right() - 18.0, surface.top() + 18.0), Vec2::splat(REMOVE_BUTTON));
        removed = remove_button(ui, id, close, &row.name);
        if removed {
            actions.remove = Some(row.path.clone());
        }
    }
    let resp = resp.on_hover_text(&row.path).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() && !removed {
        actions.open = Some(row.path.clone());
    }
    resp.context_menu(|ui| menu(ui, &row.path, missing, actions));
}

/// "Missing" with a warning triangle, over the picture's corner.
fn missing_badge(ui: &egui::Ui, at: egui::Pos2) {
    let t = Tokens::get(ui.ctx());
    let text = ui.painter().layout_no_wrap(tl!("Missing").into(), medium(11.0), t.warning);
    let rect = Rect::from_min_size(at, vec2(text.size().x + 5.0 + 13.0 + 5.0 + 7.0, text.size().y + 4.0));
    ui.painter().rect_filled(rect, t.radius_sm, t.dock);
    crate::icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 11.5, rect.center().y), Vec2::splat(13.0)), "triangle-alert", 13.0, t.warning);
    ui.painter().galley(pos2(rect.left() + 5.0 + 13.0 + 5.0, rect.center().y - text.size().y / 2.0), text, t.warning);
}

// ------------------------------------------------------------------ list

/// Where a list's columns start and how wide they are, in points from the row's left edge.
struct Columns {
    name: (f32, f32),
    when: (f32, f32),
    size: (f32, f32),
    kind: (f32, f32),
    folder: (f32, f32),
}

fn columns(width: f32, compact: bool) -> Columns {
    let none = (0.0, 0.0);
    if compact {
        let name_w = (width - LIST_PAD_LEFT - LIST_PAD_RIGHT - LIST_GAP - LIST_WHEN_WIDTH_COMPACT).max(40.0);
        let when_x = LIST_PAD_LEFT + name_w + LIST_GAP;
        return Columns { name: (LIST_PAD_LEFT, name_w), when: (when_x, LIST_WHEN_WIDTH_COMPACT), size: none, kind: none, folder: none };
    }
    // Name 3fr, Last Opened 1.1fr, Size, Kind, Folder 1.8fr, as one grid.
    let flex = (width - LIST_PAD_LEFT - LIST_PAD_RIGHT - LIST_SIZE_WIDTH - LIST_KIND_WIDTH - 4.0 * LIST_GAP).max(100.0);
    let (name_w, when_w, folder_w) = (flex * 3.0 / 5.9, flex * 1.1 / 5.9, flex * 1.8 / 5.9);
    let name_x = LIST_PAD_LEFT;
    let when_x = name_x + name_w + LIST_GAP;
    let size_x = when_x + when_w + LIST_GAP;
    let kind_x = size_x + LIST_SIZE_WIDTH + LIST_GAP;
    let folder_x = kind_x + LIST_KIND_WIDTH + LIST_GAP;
    Columns { name: (name_x, name_w), when: (when_x, when_w), size: (size_x, LIST_SIZE_WIDTH), kind: (kind_x, LIST_KIND_WIDTH), folder: (folder_x, folder_w) }
}

/// The recent files as a list under clickable column headers.
#[allow(clippy::too_many_arguments)]
pub(super) fn list(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    width: f32,
    rows: &[Row],
    order: &[usize],
    sort: HomeRecentSort,
    now: u64,
    actions: &mut Actions,
) {
    let t = Tokens::get(ui.ctx());
    let compact = width < LIST_COMPACT_BELOW;
    let cols = columns(width, compact);
    ui.add_space(18.0);
    let (head, _) = ui.allocate_exact_size(vec2(width, LIST_HEADER_HEIGHT), Sense::hover());
    let headers: [(&str, Option<HomeRecentSort>, (f32, f32)); 5] = [
        ("Name", Some(HomeRecentSort::Name), cols.name),
        ("Last Opened", Some(HomeRecentSort::Recent), cols.when),
        ("Size", Some(HomeRecentSort::Size), cols.size),
        ("Kind", Some(HomeRecentSort::Kind), cols.kind),
        ("Folder", None, cols.folder),
    ];
    for (label, kind, (x, w)) in headers {
        if w <= 0.0 {
            continue;
        }
        let cell = Rect::from_min_size(pos2(head.left() + x, head.top()), vec2(w, head.height()));
        let resp = ui.interact(cell, ui.id().with(("recent-header", label)), if kind.is_some() { Sense::click() } else { Sense::hover() });
        let shown = tl!(label);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, kind.is_some(), shown));
        let active = kind == Some(sort);
        let color = if active || resp.hovered() { t.text } else { t.text_dim };
        ui.painter().text(cell.left_center(), Align2::LEFT_CENTER, shown, medium(12.0), color);
        if let Some(kind) = kind
            && resp.clicked()
        {
            actions.header = Some(kind);
        }
    }
    ui.painter().hline(head.x_range(), head.bottom(), Stroke::new(1.0, t.separator));

    for &index in order {
        let Some(row) = rows.get(index) else { continue };
        let (rect, _) = ui.allocate_exact_size(vec2(width, LIST_ROW_HEIGHT), Sense::hover());
        if ui.is_rect_visible(rect) {
            list_row(app, ui, rect, row, &cols, now, actions);
        }
    }
}

fn list_row(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, row: &Row, cols: &Columns, now: u64, actions: &mut Actions) {
    let t = Tokens::get(ui.ctx());
    let id = ui.id().with(("recent-row", row.path.as_str()));
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name));
    let hovered = ui.rect_contains_pointer(rect);
    app.recent_view.want(&row.path, row.opened.unwrap_or(0));
    let stat = app.recent_view.status(&row.path);
    let missing = stat == Some(FileStat::Missing);
    if hovered {
        ui.painter().rect_filled(rect, 0.0, t.hover);
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.separator));

    let thumb = Rect::from_min_size(pos2(rect.left() + cols.name.0, rect.center().y - LIST_THUMB.y / 2.0), LIST_THUMB);
    ui.painter().rect_filled(thumb, t.radius_sm, t.card);
    picture(app, ui, thumb, LIST_PICTURE, row, missing, true);

    let text_y = rect.center().y;
    let cell = |ui: &egui::Ui, (x, w): (f32, f32), text: &str, font: egui::FontId, color: Color32| {
        if w > 0.0 {
            let g = crate::tab_strip::elided(ui, text, font, color, w);
            ui.painter().galley(pos2(rect.left() + x, text_y - g.size().y / 2.0), g, color);
        }
    };
    let name_x = thumb.right() + 12.0 - rect.left();
    cell(ui, (name_x, cols.name.0 + cols.name.1 - name_x), &row.name, medium(13.0), t.text);
    let small = egui::FontId::proportional(12.5);
    if missing {
        cell(ui, cols.when, tl!("File not found"), small.clone(), t.warning);
    } else if let Some(opened) = row.opened {
        cell(ui, cols.when, &when_label(now, opened), small.clone(), t.text_dim);
    }
    if let Some(FileStat::Present { size, .. }) = stat {
        cell(ui, cols.size, &size_label(size), small.clone(), t.text_dim);
    }
    cell(ui, cols.kind, &row.kind, small.clone(), t.text_dim);
    cell(ui, cols.folder, &row.folder, small, t.text_dim);

    let mut removed = false;
    if hovered {
        let close = Rect::from_center_size(pos2(rect.right() - 8.0 - REMOVE_BUTTON / 2.0, rect.center().y), Vec2::splat(REMOVE_BUTTON));
        removed = remove_button(ui, id, close, &row.name);
        if removed {
            actions.remove = Some(row.path.clone());
        }
    }
    let resp = resp.on_hover_text(&row.path).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() && !removed {
        actions.open = Some(row.path.clone());
    }
    resp.context_menu(|ui| menu(ui, &row.path, missing, actions));
}
