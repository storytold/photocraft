//! The main window's document area: the canvas panel with the tab strip and the documents, the Home screen, and the
//! panes of a Window › Arrange n-up or Tile arrangement.

use std::cell::RefCell;

use craft::behavior::PressPhase;
use craft::behavior::dock::Arrangement;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::canvas::{SplitAt, SplitViews};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Elide, Text};
use craft::ui::docking::Arranged;
use craft::ui::shell::CanvasPanel;
use craft::ui::style::StyleCx;
use craft::ui::tabs::{Tab, TabPartEvent};
use egui::Rect;

use super::document_display::CanvasHost;
use super::start_screen::start_screen;
use super::tabs::{self, document_index, main_canvas_document};
use super::textures::retain_gpu_documents;
use super::{PhotocraftApp, canvas_view};

/// The canvas panel with the tab strip and the document, or the Home screen; then drops layers dragged onto another
/// document (`layer_transfer`).
pub fn document_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // Files the OS drops on the main window outside the tab strip: the Home screen has no other target (#16588).
    let dropped = super::window_drop::take(ui, ui.max_rect());
    if crate::jobs_ui::batch_preview(app, ui) {
        // The preview replaces the tabs: a floating window dropped meanwhile docks nowhere (#16415).
        app.dock_drop_rect = None;
        // It replaces the canvas too: a file dropped where the canvas was opens, not into the hidden document.
        app.drop_canvas_rect = None;
    } else {
        crate::distort_ui::drop_foreign(app, ui.ctx());
        crate::layer_transfer::prepare(app, ui.ctx());
        documents(app, ui);
        crate::layer_transfer::finish(app, ui.ctx());
        // The Shift+Alt+right-click HUD Color Picker (#6098), while its gesture is held.
        crate::color_hud::show(app, ui);
    }
    if let Some(files) = dropped {
        super::window_drop::open(app, ui.ctx(), files);
    }
}

/// Where each pane of Window › Arrange's n-up or Tile arrangement is in `rect`, with the document it shows: pane `i` is
/// docked group `i` of `app.ui.document_windows`, showing that group's shown document, in the rect `SplitViews`
/// places it in with the dividers at `app.ui.split_at`. `None` with no arrangement or fewer than two docked groups:
/// the single canvas fills the area.
pub(super) fn arranged_panes(app: &PhotocraftApp, rect: Rect) -> Option<Vec<(usize, Rect)>> {
    let windows = &app.ui.document_windows;
    let layout = windows.arrangement?;
    let count = windows.docked.len();
    if count < 2 {
        return None;
    }
    let panes = app.ui.split_at.placed(layout, count, rect, crate::direction::is_rtl());
    Some(
        windows
            .docked
            .iter()
            .zip(panes)
            .filter_map(|(group, pane)| {
                let shown = group.shown.or_else(|| group.documents.first().copied())?;
                Some((document_index(app, shown)?, pane))
            })
            .collect(),
    )
}

/// The arranged canvases' display and drop areas, inside any title and status bars.
pub(crate) fn arranged_cells(app: &PhotocraftApp, rect: Rect) -> Option<Vec<(usize, Rect)>> {
    let mut panes = arranged_panes(app, rect)?;
    if app.ui.document_windows.arrangement == Some(Arrangement::Tile) {
        let heights = tile_bar_heights(app);
        for (_, pane) in &mut panes {
            *pane = TilePane::new(*pane, heights).canvas;
        }
    }
    Some(panes)
}

fn tile_bar_heights(app: &PhotocraftApp) -> (f32, f32) {
    StyleCx::with_root(app.ctx(), |cx| {
        let size = &cx.tokens.size.bar;
        let title = if app.session.prefs().workspace.large_tabs { size.title } else { size.tab };
        (title, size.status)
    })
}

/// Tile chrome takes its own rows so it is outside the tools and drop targets of the canvas.
struct TilePane {
    title: Rect,
    canvas: Rect,
    status: Rect,
}

impl TilePane {
    fn new(pane: Rect, (title, status): (f32, f32)) -> Self {
        // With an exceptionally short pane, preserve a canvas before adding either full-height bar.
        let top = pane.top() + if pane.height() > title { title } else { 0.0 };
        let bottom = pane.bottom() - if pane.bottom() - top > status { status } else { 0.0 };
        Self {
            title: Rect::from_x_y_ranges(pane.x_range(), pane.top()..=top),
            canvas: Rect::from_x_y_ranges(pane.x_range(), top..=bottom),
            status: Rect::from_x_y_ranges(pane.x_range(), bottom..=pane.bottom()),
        }
    }
}

/// Tell the GPU canvas which documents are on screen (the arranged panes or the main canvas's, plus the shown
/// document of every floating window): they split the display texture budget, and the textures of the others are
/// freed first. Only a change of that set (or a document's size) re-splits it, so switching between shown documents
/// or windows moves no texture.
pub(crate) fn publish_shown(app: &PhotocraftApp) {
    let Some(gpu) = &app.gpu else { return };
    let docs = app.session.documents();
    let mut shown: Vec<usize> = match arranged_panes(app, app.last_canvas_rect) {
        Some(cells) => cells.into_iter().map(|(d, _)| d).collect(),
        None => main_canvas_document(app).into_iter().collect(),
    };
    shown.extend(app.ui.document_windows.floating.iter().filter_map(|(group, _)| group.shown).filter_map(|id| document_index(app, id)));
    let mut shown: Vec<(u64, u64)> = shown.into_iter().filter_map(|d| docs.get(d)).map(|st| (st.doc.id.0, gpu.canvas_bytes(&st.doc))).collect();
    shown.sort_unstable();
    shown.dedup();
    gpu.show_documents(&shown);
}

/// What the document area does after Window › Arrange changed the document windows (top-w15 calls it after
/// `arrange`). A new n-up or Tile arrangement starts from even panes and every document fits its new pane, as
/// production's layouts do; Tile is a pane per open document, floating windows docked first (the framework's
/// `arrange` consolidates before it splits). The other window commands and the Match commands leave the panes and
/// views alone.
pub(crate) fn apply_arranged(app: &mut PhotocraftApp, arranged: Arranged) {
    if arranged == Arranged::Windows && app.ui.document_windows.arrangement.is_some() {
        app.ui.split_at = SplitAt::default();
        app.ui.views.iter_mut().for_each(|view| view.place(craft::ui::canvas::ViewCommands::FitOnScreen));
    }
}

/// The height of the band at the top of the document area that takes a floating window's drop where no tab strip
/// shows: a tab's height.
const DOCK_BAND: f32 = 32.0;

fn documents(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // The web build's drop path reads where the canvas is (`file_open::target_at`); set again when one shows.
    app.drop_canvas_rect = None;
    // A floating window's title bar docks on the tab strip (`tabs::strip` sets its rect); where no strip shows, on a
    // band of a tab's height at the top of the area, where the strip would be.
    let area = ui.max_rect();
    app.dock_drop_rect = Some(Rect::from_min_size(area.min, egui::vec2(area.width(), DOCK_BAND.min(area.height()))));
    retain_gpu_documents(app);
    publish_shown(app);
    crate::cas_transform::tick(app, ui.ctx());
    if app.home_screen_up() {
        // The Home screen hides the tabs: a floating window dropped on it docks nowhere (#16415).
        app.dock_drop_rect = None;
        start_screen(app, ui);
        return;
    }
    // Files opening in the background (#210) have tabs before they have documents.
    let opening = !app.jobs.opens.is_empty();
    // The tabs region and the document run one after the other inside `show`, each with the app to itself.
    let out = {
        let shared = RefCell::new(&mut *app);
        let panel = CanvasPanel::new().tabs(|ui, cx| tabs::strip(&mut shared.borrow_mut(), ui, cx)).opening(opening);
        panel.show(ui, |ui, cx| document(&mut shared.borrow_mut(), ui, cx.rect))
    };
    // Auto Show the Home Screen off: double-clicking the empty workspace shows File › Open, as in Photoshop (#5367).
    if out.empty_double && app.session.documents().is_empty() && app.jobs.opens.is_empty() {
        let _ = app.open_dialog_file();
    }
}

/// The document pane: an opening file's progress, the arranged panes, or the single canvas. With no docked document
/// it shows nothing over the surround: a floating document belongs to its own window, and drawing it here too would
/// show the same page twice.
fn document(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect) {
    // A tab click switches documents before any canvas is drawn this pass.
    publish_shown(app);
    let n = app.session.documents().len();
    if let Some(job) = app.jobs.focus.or_else(|| (n == 0).then(|| app.jobs.opens.last().map(|o| o.job)).flatten()) {
        crate::jobs_ui::open_card(app, ui, job);
        return;
    }
    let Some(active) = app.session.active_index() else { return };
    let Some(idx) = main_canvas_document(app) else { return };
    // The panel's document area (`CanvasPanelOutput::document`).
    app.last_canvas_rect = rect;
    app.last_canvas_layer = ui.layer_id();
    app.drop_canvas_rect = Some(rect);
    if arranged(app, ui, rect, active) {
        return;
    }
    let view = app.ui.views[idx].clone();
    let primary = app.session.active_index() == Some(idx);
    // While the active document floats, a press on the main canvas activates the document it shows: the canvas
    // output reports the press, the app activates (as `SplitViews` reports `activated`).
    let (_, output) = canvas_view(app, ui, CanvasHost::main(idx), rect, view, primary);
    if idx != active && matches!(output.press.phase, PressPhase::Down { .. }) {
        app.session.set_active(idx);
    }
}

/// Window › Arrange's n-up or Tile arrangement: a canvas per pane in `SplitViews`. The active document is the active pane;
/// a press on another pane activates its document, and a divider drag moves the panes. Returns false when no
/// arrangement shows.
fn arranged(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, active: usize) -> bool {
    let Some(layout) = app.ui.document_windows.arrangement else { return false };
    // The active document shows in its group's pane, whichever tab chose it.
    let active_id = app.session.documents()[active].doc.id;
    for group in &mut app.ui.document_windows.docked {
        if group.documents.contains(&active_id) {
            group.shown = Some(active_id);
        }
    }
    let Some(panes) = arranged_panes(app, rect) else { return false };
    let heights = (layout == Arrangement::Tile).then(|| tile_bar_heights(app));
    let count = app.ui.document_windows.docked.len();
    let active_pane = app.ui.document_windows.docked.iter().position(|group| group.shown == Some(active_id)).unwrap_or(0);
    let shown: Vec<Option<usize>> = app.ui.document_windows.docked.iter().map(|group| group.shown.and_then(|id| document_index(app, id))).collect();
    // The panes are laid out from the dividers as this pass starts; an edit of them lands after the pass.
    let mut at = app.ui.split_at.clone();
    let out = SplitViews::new(tl!("Documents"), layout, Bind::bound(&mut at)).count(count).active(active_pane).show(ui, |ui, _| {
        for &(d, pane) in &panes {
            let view = app.ui.views[d].clone();
            let tile = heights.map(|heights| TilePane::new(pane, heights));
            let canvas = tile.as_ref().map_or(pane, |tile| tile.canvas);
            let (v, _) = canvas_view(app, ui, CanvasHost::main(d), canvas, view, d == active);
            if let Some(tile) = &tile
                && tile.status.height() > 0.0
            {
                let id = app.session.documents()[d].doc.id;
                let zoom = format!("{}%", super::fmt_zoom(v.zoom * 100.0));
                Bar::new("", BarKind::Status).id_salt(("tile-status", id)).show_at(ui, tile.status, |ui, _| {
                    Text::new(zoom.clone()).elide(Elide::End).show(ui);
                });
            }
            if d != active {
                app.ui.views[d] = v;
            }
        }
    });
    app.ui.split_at = at;
    let mut selected = out.output.activated.and_then(|pane| shown.get(pane).copied().flatten());
    let mut closing = Vec::new();
    if let Some(heights) = heights {
        // SplitViews places a hit surface above each inactive canvas to handle its first press.
        // Title controls are siblings above those surfaces so Close works even on an inactive tile.
        for &(d, pane) in &panes {
            let title = TilePane::new(pane, heights).title;
            if title.height() <= 0.0 {
                continue;
            }
            let doc = &app.session.documents()[d];
            let label = tabs::tab_title_at(app, d, app.ui.views[d].zoom * 100.0);
            let mut tab = Tab::new(label).id_salt(("tile-title", doc.doc.id)).selected(d == active);
            if app.session.prefs().workspace.large_tabs {
                tab = tab.large();
            }
            if doc.is_dirty() {
                tab = tab.modified();
            }
            let out = tab.show_at(ui, title);
            if matches!(out.event, Some(TabPartEvent::Close)) {
                closing.push(doc.doc.id);
            } else if out.response.clicked() {
                selected = Some(d);
            }
        }
    }
    if let Some(d) = selected {
        app.session.set_active(d);
    }
    // A clean document can leave the session immediately, so close only after the pane indices were used.
    for id in closing {
        tabs::close_document(app, ui.ctx(), id);
    }
    true
}
