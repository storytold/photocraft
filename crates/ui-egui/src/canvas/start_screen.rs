//! The Home screen: shown with no document open (Preferences › General › Auto Show the Home
//! Screen) or over the open documents by the Home button. A rail with New file, Open and the
//! community links beside a scrolling page: a welcome and the Recent files as a grid of cards or a
//! list (see [`recent`]). Narrower than [`STACK_BELOW`] the rail stacks on top, for tiling window
//! managers.

use egui::{Align, Layout, Rect, Sense, Stroke, UiBuilder, Vec2, pos2, vec2};
use photocraft_engine::prefs::{HomeRecentSort, HomeRecentView};

use super::PhotocraftApp;
use super::tabs::{open_on_double_click, paint_dots};
use crate::theme::{Tokens, medium, semibold};

mod arrange;
mod recent;
#[cfg(test)]
mod tests;

use arrange::{Row, arrange};

/// Width of the rail.
const RAIL_WIDTH: f32 = 190.0;

/// Below this width the rail goes on top of the page instead of beside it.
const STACK_BELOW: f32 = 640.0;

/// Height of the header with the wordmark.
const HEADER_HEIGHT: f32 = 54.0;

/// The widest the page's content grows.
const CONTENT_MAX_WIDTH: f32 = 1160.0;

/// Width of the Filter field.
const FILTER_WIDTH: f32 = 230.0;

/// Height of a row of the rail's Home entry.
const NAV_ROW_HEIGHT: f32 = 34.0;

/// Below this width the Sort and Filter controls take a row each.
const TOOLBAR_SINGLE_ROW: f32 = 520.0;

pub(super) fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    paint_dots(ui, area, app.session.prefs().interface.show_canvas_dots);
    open_on_double_click(app, ui, area);
    let narrow = area.width() < STACK_BELOW;
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        let header = Rect::from_min_size(area.min, vec2(area.width(), HEADER_HEIGHT.min(area.height())));
        header_bar(app, ui, header);
        ui.painter().hline(area.x_range(), header.bottom(), Stroke::new(1.0, t.separator));
        let body = Rect::from_min_max(pos2(area.left(), header.bottom()), area.max);
        let page = if narrow {
            // The rail's buttons in a row on top; the links follow the page.
            let mut rail = ui.new_child(UiBuilder::new().max_rect(body.shrink2(vec2(16.0, 12.0))));
            rail.horizontal_wrapped(|ui| rail_buttons(app, ui, 0.0));
            let used = rail.min_rect().height() + 24.0;
            Rect::from_min_max(pos2(body.left(), (body.top() + used).min(body.bottom())), body.max)
        } else {
            rail(app, ui, Rect::from_min_size(body.min, vec2(RAIL_WIDTH.min(body.width()), body.height())));
            Rect::from_min_max(pos2((body.left() + RAIL_WIDTH).min(body.right()), body.top()), body.max)
        };
        page_area(app, ui, page, narrow);
    });
}

/// The wordmark, the Home button, and the command search.
fn header_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, header: Rect) {
    let t = Tokens::get(ui.ctx());
    let mut bar = ui.new_child(UiBuilder::new().max_rect(header.shrink2(vec2(20.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
    bar.label(egui::RichText::new(tl!("PhotoCraft")).font(semibold(18.0)).color(t.text));
    // The options bar is hidden on Home, so this is the way back to the open documents.
    let documents = app.session.documents().len();
    if (documents > 0 || !app.session.prefs().general.auto_show_home_screen) {
        crate::chrome_ui::home_button(app, &mut bar);
    }
    bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if crate::icons::button(ui, "search", 30.0, false, tl!("Search commands")).clicked() {
            let ctx = ui.ctx().clone();
            let _ = crate::menus::invoke(app, &ctx, "edit.search", serde_json::json!({}));
        }
    });
}

/// The New file and Open buttons, `width` wide (0: as wide as their labels).
fn rail_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui, width: f32) {
    let new_tip = crate::shortcuts::tip_label(app, "New document…", "file.new");
    let open_tip = crate::shortcuts::tip_label(app, "Open…", "file.open");
    if crate::widgets::primary_button(ui, tl!("New file"), width).on_hover_text(new_tip).clicked() {
        let fields = app.new_document_fields();
        app.ui.open_dialog(crate::state::DialogKind::NewDocument, fields);
    }
    if crate::widgets::secondary_button(ui, tl!("Open"), width).on_hover_text(open_tip).clicked() {
        let _ = app.open_dialog_file();
    }
}

/// The rail beside the page: the buttons, the Home entry, and at the bottom the drop hint and links.
fn rail(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let inner = rect.shrink2(vec2(16.0, 22.0));
    let width = inner.width().max(0.0);
    let mut top = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    top.spacing_mut().item_spacing.y = 8.0;
    rail_buttons(app, &mut top, width);
    top.add_space(4.0);
    let (line, _) = top.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    top.painter().rect_filled(line, 0.0, t.separator);
    top.add_space(4.0);
    let (row, resp) = top.allocate_exact_size(vec2(width, NAV_ROW_HEIGHT), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, true, tl!("Home")));
    top.painter().rect_filled(row, t.radius_sm, t.pressed);
    crate::icons::paint(&top, Rect::from_center_size(pos2(row.left() + 20.0, row.center().y), vec2(16.0, 16.0)), "house", 16.0, t.text);
    top.painter().text(pos2(row.left() + 38.0, row.center().y), egui::Align2::LEFT_CENTER, tl!("Home"), medium(13.0), t.text);

    // Added from the bottom up: the links, the Discord button, the hint.
    let mut bottom = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::bottom_up(Align::Min)));
    crate::links::link_list(app, &mut bottom);
    bottom.add_space(10.0);
    crate::links::discord_button(app, &mut bottom, width);
    bottom.add_space(14.0);
    drop_hint(app, &mut bottom);
}

/// "Drop an image or PSD anywhere to open it." (or what Wayland offers instead), wrapped.
fn drop_hint(app: &PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let msg = start_screen_drop_hint(app);
    ui.add(egui::Label::new(egui::RichText::new(msg).size(12.0).color(t.text_faint)).wrap());
}

/// The scrolling page: the welcome and the recent files. In a narrow window the drop hint and
/// links, which the rail would carry, come last.
fn page_area(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, narrow: bool) {
    let pad = if narrow { vec2(16.0, 18.0) } else { vec2(36.0, 26.0) };
    let mut page = ui.new_child(UiBuilder::new().max_rect(rect));
    egui::ScrollArea::vertical().id_salt("home-page").auto_shrink([false, false]).show(&mut page, |ui| {
        let available = ui.available_width();
        let width = (available - 2.0 * pad.x).clamp(0.0, CONTENT_MAX_WIDTH);
        let left = ((available - width) / 2.0).max(0.0);
        ui.add_space(pad.y);
        ui.horizontal_top(|ui| {
            ui.add_space(left);
            ui.vertical(|ui| {
                ui.set_width(width);
                welcome(ui);
                if !cfg!(target_arch = "wasm32") {
                    recent_section(app, ui, width);
                }
                if narrow {
                    ui.add_space(28.0);
                    drop_hint(app, ui);
                    ui.add_space(12.0);
                    crate::links::discord_button(app, ui, 0.0);
                    ui.add_space(10.0);
                    crate::links::link_list(app, ui);
                }
                ui.add_space(40.0);
            });
        });
    });
}

fn welcome(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(tl!("Welcome to PhotoCraft")).size(30.0).color(t.text));
    });
    ui.add_space(26.0);
}

/// Why the list is not shown, or that it is empty: a title, a line under it and perhaps a button.
fn notice(ui: &mut egui::Ui, icon: Option<&str>, title: &str, body: &str, button: Option<&str>) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut clicked = false;
    ui.add_space(48.0);
    // A centred column of its own, placed by hand: narrowing the parent's centred layout moved
    // everything below it.
    let available = ui.available_width();
    let column = available.min(420.0);
    let top_left = ui.cursor().min;
    let rect = Rect::from_min_size(pos2(top_left.x + (available - column) / 2.0, top_left.y), vec2(column, 10_000.0));
    let mut inner = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center)));
    if let Some(icon) = icon {
        let (r, _) = inner.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        crate::icons::paint(&inner, r, icon, 40.0, t.text_dim);
        inner.add_space(8.0);
    }
    inner.label(egui::RichText::new(title).font(medium(16.0)).color(t.text));
    inner.add_space(6.0);
    inner.add(egui::Label::new(egui::RichText::new(body).color(t.text_dim)).wrap());
    if let Some(button) = button {
        inner.add_space(14.0);
        clicked = crate::widgets::secondary_button(&mut inner, button, 0.0).clicked();
    }
    let used = inner.min_rect().height();
    ui.allocate_space(vec2(available, used));
    clicked
}

/// Open Preferences on `section` unless it is already open.
fn open_preferences(app: &mut PhotocraftApp, section: &str) {
    if !app.ui.dialogs.iter().any(|d| crate::prefs_ui::is_preferences(&d.fields)) {
        crate::prefs_ui::open_preferences(app, section);
    }
}

/// The "Recent" heading, its controls and the files (or why there are none to show).
fn recent_section(app: &mut PhotocraftApp, ui: &mut egui::Ui, width: f32) {
    app.sync_recent();
    let (show, stored) = {
        let p = app.session.prefs();
        (p.interface.show_home_recent_files, p.file_handling.recent_files.len())
    };
    if !show {
        let title = tl!("Recent files are hidden");
        let body = tl!("Your list is kept. Turn it back on in Preferences › Interface.");
        if notice(ui, None, title, body, Some(tl!("Open Preferences"))) {
            open_preferences(app, "interface");
        }
        return;
    }
    if app.recent_cap() == 0 && stored > 0 {
        let title = tl!("The recent file list is set to 0 files");
        let body = tl!("Raise Recent File List Contains in Preferences › File Handling to list them again.");
        if notice(ui, None, title, body, Some(tl!("Open Preferences"))) {
            open_preferences(app, "fileHandling");
        }
        return;
    }
    let listed = app.ui.recent_files.clone();
    if listed.is_empty() {
        notice(ui, Some("folder"), tl!("Your recent files will appear here"), tl!("Open or create a file to get started."), None);
        return;
    }

    let t = Tokens::get(ui.ctx());
    let (mut view, mut sort, mut reverse) = {
        let i = &app.session.prefs().interface;
        (i.home_recent_view, i.home_recent_sort, i.home_recent_reverse)
    };
    let before = (view, sort, reverse);
    let now = crate::recent_thumbs::unix_now();
    let ctx = ui.ctx().clone();
    app.recent_view.begin(&ctx, &listed);

    // Recent, with the view toggles at the right.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Recent")).font(semibold(15.0)).color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if view_toggle(ui, ViewIcon::Grid, tl!("Grid view"), view == HomeRecentView::Grid) {
                view = HomeRecentView::Grid;
            }
            if view_toggle(ui, ViewIcon::List, tl!("List view"), view == HomeRecentView::List) {
                view = HomeRecentView::List;
            }
        });
    });
    ui.add_space(6.0);

    // Sort and its direction at the left, Filter at the right.
    let mut filter = app.ui.home_recent_filter.clone();
    let single_row = width >= TOOLBAR_SINGLE_ROW;
    let mut toolbar = |ui: &mut egui::Ui| {
        ui.label(egui::RichText::new(tl!("Sort")).color(t.text_dim));
        let options = [(HomeRecentSort::Recent, "Recent"), (HomeRecentSort::Name, "Name"), (HomeRecentSort::Size, "Size"), (HomeRecentSort::Kind, "Kind")];
        crate::widgets::dropdown(ui, "home-recent-sort", tl!("Sort"), &mut sort, &options, 104.0);
        let (bar, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
        ui.painter().rect_filled(bar, 0.0, t.field_border);
        let arrow = if reverse { "chevron-up" } else { "chevron-down" };
        if crate::icons::button(ui, arrow, 30.0, false, tl!("Reverse sort order")).clicked() {
            reverse = !reverse;
        }
    };
    let filter_field = |ui: &mut egui::Ui, filter: &mut String, width: f32, rtl: bool| {
        // In a right-to-left layout the first widget lands rightmost, so the field goes first.
        let label = |ui: &mut egui::Ui| ui.label(egui::RichText::new(tl!("Filter")).color(t.text_dim));
        if !rtl {
            label(ui);
        }
        let field = crate::widgets::text_field(ui, egui::Id::unique("home-recent-filter"), filter, tl!("Filter Recent Files"), width, false);
        let _ = field.accessible_name(tl!("Filter Recent Files"));
        if rtl {
            label(ui);
        }
    };
    if single_row {
        ui.horizontal(|ui| {
            toolbar(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| filter_field(ui, &mut filter, FILTER_WIDTH, true));
        });
    } else {
        ui.horizontal(&mut toolbar);
        ui.add_space(4.0);
        ui.horizontal(|ui| filter_field(ui, &mut filter, (width - 60.0).clamp(80.0, FILTER_WIDTH), false));
    }
    if filter != app.ui.home_recent_filter {
        app.ui.home_recent_filter = filter.clone();
    }
    if (view, sort, reverse) != before {
        app.session.prefs.edit(|p| {
            p.interface.home_recent_view = view;
            p.interface.home_recent_sort = sort;
            p.interface.home_recent_reverse = reverse;
        });
    }

    // Every file's recorded time is known; a size sort, or a file without a time, needs a look at
    // the files off screen too.
    let opened = app.session.prefs().file_handling.recent_opened.clone();
    if sort == HomeRecentSort::Size || listed.iter().any(|p| !opened.contains_key(p)) {
        for path in &listed {
            app.recent_view.want_status(path);
        }
    }
    let rows: Vec<Row> = listed
        .iter()
        .map(|path| {
            let (size, modified) = match app.recent_view.status(path) {
                Some(crate::recent_thumbs::FileStat::Present { size, modified }) => (Some(size), modified),
                _ => (None, None),
            };
            Row {
                path: path.clone(),
                name: crate::file_open::display_name(path),
                folder: std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
                kind: crate::recent_thumbs::kind_label(path),
                opened: opened.get(path).copied().or(modified),
                size,
            }
        })
        .collect();
    let order = arrange(&rows, sort, reverse, &filter);

    let mut actions = recent::Actions::default();
    if order.is_empty() {
        let title = crate::i18n::fmt(tl!("No recent files match “{filter}”"), &[("filter", filter.trim())]);
        notice(ui, None, &title, tl!("Try a different part of the file name."), None);
    } else if view == HomeRecentView::List {
        recent::list(app, ui, width, &rows, &order, sort, now, &mut actions);
    } else {
        recent::grid(app, ui, width, &rows, &order, now, &mut actions);
    }
    actions.apply(app, &ctx);
    app.recent_view.finish(&ctx, app.services.thumbnail_cache_dir.as_deref());
}

/// The picture on a view toggle.
#[derive(Clone, Copy)]
enum ViewIcon {
    Grid,
    List,
}

/// One of the list and grid toggles; says whether it was clicked.
fn view_toggle(ui: &mut egui::Ui, icon: ViewIcon, tip: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, tip));
    let color = crate::icons::button_chrome(ui, rect, selected, resp.hovered());
    match icon {
        ViewIcon::Grid => crate::icons::paint(ui, rect, "grid-2x2", 16.0, color),
        ViewIcon::List => {
            // Three lines, each with a bullet.
            let c = rect.center();
            for dy in [-5.0, 0.0, 5.0] {
                ui.painter().line_segment([pos2(c.x - 2.0, c.y + dy), pos2(c.x + 7.0, c.y + dy)], Stroke::new(1.5, color));
                ui.painter().circle_filled(pos2(c.x - 6.5, c.y + dy), 1.0, color);
            }
        }
    }
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Wayland has no native file drops (winit 0.30, #386), so the hint offers File › Open and paste.
pub(super) fn start_screen_drop_hint(app: &PhotocraftApp) -> std::borrow::Cow<'static, str> {
    if app.services.is_wayland {
        crate::i18n::fmt(tl!("Use File › Open, or paste a copied image with {paste}."), &[("paste", &crate::notices::paste_hint(app))]).into()
    } else {
        tl!("Drop an image or PSD anywhere to open it.").into()
    }
}
