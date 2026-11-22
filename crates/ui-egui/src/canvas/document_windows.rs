//! The document windows of Window › New Window for Document and Float in Window: one framework
//! `DocumentWindow` for each floating group of `app.ui.document_windows`, with the document's canvas
//! as its body.

use craft::behavior::canvas::zoom::wrapped_canvas;
use craft::behavior::kurbo::Size;
use craft::host::target::WindowState;
use craft::ui::canvas::ToolHost;
use craft::ui::contract::{ShowContainer as _, SizeLimits};
use craft::ui::runtime::windows::WindowHost;
use craft::ui::shell::{DocumentWindow, Title, TitleBar, Wrapped};
use craft::window::policy::Frame;
use egui::{Rect, Vec2, vec2};
use photocraft_doc::DocId;

use super::canvas_view;
use super::document_display::CanvasHost;
use crate::PhotocraftApp;
use crate::state::View;

/// The smallest a document window gets: the Zoom Resizes Windows wrap never shrinks it below this.
const MIN_SIZE: Vec2 = vec2(240.0, 160.0);

/// Whether a zoom edit in a window asks the window to wrap the image (Preferences › General › Zoom
/// Resizes Windows): the preference is on, the zoom differs between `before` and `after` (compared
/// bitwise, so a NaN never asks again and again), and neither view is waiting for a Fit or Fill
/// Screen, which size the image to the window and not the window to the image.
fn wants_wrap(preference: bool, before: &View, after: &View) -> bool {
    let sizing = |v: &View| v.asks(craft::ui::canvas::ViewCommands::FitOnScreen) || v.asks(craft::ui::canvas::ViewCommands::FillScreen);
    preference && after.zoom.to_bits() != before.zoom.to_bits() && !sizing(before) && !sizing(after)
}

/// Closes the floating window `id`: its own view goes, and its documents return to the docked tabs.
fn close_window(app: &mut PhotocraftApp, id: u64) {
    app.ui.window_views.remove(&id);
    app.ui.document_windows.close_window(id);
}

/// Puts `app.ui.document_windows` in step with the open documents: a document no group holds joins the docked
/// group (the first one, or a new one), and a closed document leaves every group, the windows it empties going with
/// it. The framework keeps groups of document ids and has no add of its own, so a new document floats in a
/// throwaway window and closing that window docks it.
pub(crate) fn sync_documents(app: &mut PhotocraftApp, ids: &[DocId]) {
    let windows = &mut app.ui.document_windows;
    for group in windows.docked.iter_mut().chain(windows.floating.iter_mut().map(|(group, _)| group)) {
        group.documents.retain(|id| ids.contains(id));
        if group.shown.is_some_and(|shown| !group.documents.contains(&shown)) {
            group.shown = group.documents.first().copied();
        }
    }
    windows.docked.retain(|group| !group.documents.is_empty());
    let emptied: Vec<u64> = windows.floating.iter().filter(|(group, _)| group.documents.is_empty()).map(|(group, _)| group.id).collect();
    for id in emptied {
        close_window(app, id);
    }
    for id in ids {
        app.ui.document_windows.add_document(id);
    }
}

/// The floating document windows, each in its own OS window (or on the fallback layer where there is
/// none). `ui` is the frame's root `Ui`: `DocumentWindow` is a container shown in a parent.
pub fn extra_windows(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let windows: Vec<(u64, Rect)> = app.ui.document_windows.floating.iter().map(|(g, rect)| (g.id, *rect)).collect();
    let mut closed = Vec::new();
    let mut docked = Vec::new();
    let dock_target = app.dock_drop_rect;
    for (id, rect) in windows {
        let shown = app.ui.document_windows.floating.iter().find(|(g, _)| g.id == id).and_then(|(g, _)| g.shown.or_else(|| g.documents.first().copied()));
        let Some(doc) = shown.and_then(|d| crate::canvas::tabs::document_index(app, d)) else { continue };
        let title = format!("{} — window {}", crate::canvas::tabs::titled_name(app, &app.session.documents()[doc]), id);
        // A New Window for Document keeps its own view beside the document's; any other floating window
        // draws the document's own view, which the Navigator, the status bar's zoom and the View commands
        // edit (#11276). Either is read here, inside the loop: a Scroll All Windows pan in a window drawn
        // earlier in it may have moved it since the list was taken.
        let own = app.ui.window_views.contains_key(&id);
        let Some(start) = (if own { app.ui.window_views.get(&id) } else { app.ui.views.get(doc) }).cloned() else { continue };
        let preference = app.session.prefs().general.zoom_resizes_windows;
        let size = app.session.documents()[doc].doc.size;
        let doc_size = Size::new(f64::from(size.width), f64::from(size.height));
        let focused = app.session.active_index() == Some(doc);
        let bar_title = {
            let text = crate::canvas::tabs::tab_title_at(app, doc, start.zoom * 100.0);
            if app.session.documents()[doc].is_dirty() { Title::unsaved(text) } else { Title::new(text) }
        };
        let mut view = start.clone();
        let mut dock = false;
        let mut bar_rect = None;
        let inner = DocumentWindow::new(title).first_placed(rect).overlay(|o| o.size_limits(SizeLimits::at_least(MIN_SIZE))).show(ui, |ui, cx| {
            if let Some(bar) = window_title_bar(ui, bar_title.clone()) {
                if bar.close {
                    cx.request_close();
                }
                dock |= dropped_on_main(ui, id, bar.rect, dock_target);
                bar_rect = Some(bar.rect);
            }
            // The window's status bar along its bottom, before the canvas takes the rest; a zoom typed into it applies
            // to the view the canvas draws this pass.
            let host = CanvasHost { idx: doc, window: Some(id) };
            let shown = ToolHost::shown_space(ui.ctx(), host.canvas_id()).map(|space| space.view);
            let edited = egui::Panel::bottom(egui::IdSalt::new(("floating document status", id)))
                .resizable(false)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| crate::panels::document_window_status(app, ui, doc, shown))
                .inner;
            if let Some(edited) = edited {
                // The window's own scale, which its canvas builds its view with: it may sit on another display.
                let ppp = ui.ctx().pixels_per_point();
                view.set_from(&edited, ppp);
            }
            // A second run of the body in one pass continues from the first run's view; the wrap rule
            // compares with the view the window started the pass with, so both runs file the same ask.
            let (after, canvas) = document_window_canvas(app, ui, id, doc, view.clone(), focused);
            // The view the window's canvas showed this pass, kept by the tool host that ran in it.
            if wants_wrap(preference, &start, &after)
                && let Some(drawn) = ToolHost::shown_space(ui.ctx(), host.canvas_id()).map(|space| space.view)
            {
                DocumentWindow::wrap_in(cx, canvas, wrapped_canvas(&drawn, doc_size, canvas));
            }
            view = after;
        });
        let output = inner.output;
        dock |= os_move_dropped(app, ui.ctx(), id, output.response.id, bar_rect, dock_target);
        if dock {
            docked.push((id, doc));
            continue;
        }
        follow_os_window(app, ui.ctx(), id, output.response.id);
        if output.close_requested.is_some() {
            if own {
                // A New Window for Document is a second view: closing it leaves the document open.
                closed.push(id);
            } else {
                // Closing a floating document closes the document, as in the reference application, through the
                // guarded close the tab's × takes: a document with unsaved changes asks first, and the window stays
                // until it is answered (#16415). The window goes with the document.
                let doc_id = app.session.documents()[doc].doc.id;
                super::tabs::close_document(app, ui.ctx(), doc_id);
            }
        }
        if output.focus_gained && app.session.active_index() != Some(doc) {
            app.session.set_active(doc);
        }
        // A window that took the wanted size shows the whole image: centre it. One the work area capped
        // keeps the zoomed spot.
        if output.wrapped == Some(Wrapped::Fits) {
            view.center = [size.width as f32 / 2.0, size.height as f32 / 2.0];
        }
        if own {
            if let Some(v) = app.ui.window_views.get_mut(&id) {
                *v = view;
            }
        } else if let Some(v) = app.ui.views.get_mut(doc) {
            *v = view;
        }
    }
    for id in closed {
        close_window(app, id);
    }
    // A window whose title bar was dropped on the main window's tabs goes back into them, its document in front.
    for (id, doc) in docked {
        close_window(app, id);
        app.session.set_active(doc);
        // The OS window closes in the next pass, the first that no longer shows it; nothing else may ask for one.
        ui.ctx().request_repaint();
    }
}

/// Whether an OS move of the floating window `window` (the container `container`) by its title bar (`bar`, in the
/// window's points) ended this pass with the pointer on the bar over `target`, the main window's tab strip in its
/// points. Where the platform answers the window's hit test (Windows, the app-drawn frame), the OS runs the move and
/// the app sees neither the press nor the release: the window moving from one pass to the next starts it, the
/// physical primary button going up (`Services::primary_down`) ends it, and the OS pointer (`Services::cursor_pos`)
/// says where (#16415).
fn os_move_dropped(app: &mut PhotocraftApp, ctx: &egui::Context, window: u64, container: egui::Id, bar: Option<Rect>, target: Option<Rect>) -> bool {
    let host = WindowHost::of(ctx);
    let Some(facts) = host.viewport_of(container).and_then(|viewport| host.facts(viewport)) else { return false };
    let (last_key, moving_key) = (egui::Id::unique(("floating document place", window)), egui::Id::unique(("floating document moving", window)));
    let previous = ctx.data(|d| d.get_temp::<Rect>(last_key));
    ctx.data_mut(|d| d.insert_temp(last_key, facts.inner));
    // A move keeps the size; a resize or a wrap to the zoom is no move.
    let moved = previous.is_some_and(|before| before.min != facts.inner.min && (before.size() - facts.inner.size()).length() < 0.5);
    let Some(primary_down) = app.services.primary_down.as_ref() else { return false };
    let down = primary_down();
    // The OS may run the whole move between two of the app's passes (Windows draws no frame of the app while it
    // moves a window), so a move first seen with the button up ends at once.
    let moving = ctx.data(|d| d.get_temp::<bool>(moving_key).unwrap_or(false)) || moved;
    if down || !moving {
        ctx.data_mut(|d| d.insert_temp(moving_key, moving));
        // Nothing wakes the app when the OS move ends: look again soon while it lasts.
        if moving {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        return false;
    }
    ctx.data_mut(|d| d.remove_temp::<bool>(moving_key));
    let (Some(bar), Some(target), Some(main)) = (bar, target, host.facts(egui::ViewportId::ROOT)) else { return false };
    let Some(pointer) = app.services.cursor_pos.as_mut().and_then(|cursor| cursor(ctx)) else { return false };
    // The pointer is in the main window's points; the release on the bar is where the move held the window.
    let on_bar = bar.translate(facts.inner.min.to_vec2()).expand(2.0).contains(main.inner.min + pointer.to_vec2());
    on_bar && target.contains(pointer)
}

/// Whether this pass ends a move of the floating window `window` by its title bar (`bar`, in the window's points) with
/// the pointer over `target`, the main window's tab strip in its points (`PhotocraftApp::dock_drop_rect`): the press on
/// the bar notes where the window was, and the release, which the runtime adds once the OS move ends, docks when the
/// window moved and the pointer is over the strip (#16415). A click on the bar, which moves nothing, docks nothing.
fn dropped_on_main(ui: &egui::Ui, window: u64, bar: Rect, target: Option<Rect>) -> bool {
    let ctx = ui.ctx();
    let host = WindowHost::of(ctx);
    let Some(facts) = host.facts(ctx.viewport_id()) else { return false };
    let key = egui::Id::unique(("floating document press", window));
    let (pressed, released, pos) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.interact_pos()));
    if pressed && pos.is_some_and(|p| bar.contains(p)) {
        ctx.data_mut(|d| d.insert_temp(key, facts.inner.min));
        return false;
    }
    if !released {
        return false;
    }
    let Some(start) = ctx.data_mut(|d| d.remove_temp::<egui::Pos2>(key)) else { return false };
    let (Some(target), Some(main), Some(pos)) = (target, host.facts(egui::ViewportId::ROOT), pos) else { return false };
    if start == facts.inner.min {
        return false;
    }
    target.translate(main.inner.min.to_vec2()).contains(facts.inner.min + pos.to_vec2())
}

/// Keeps the rect the floating window `window` is placed at (`DocumentWindow::first_placed`, the container `container`)
/// where its OS window is. The placed rect is the caller's: the framework puts the window back there on every pass, so
/// without this a move by the title bar or by the OS snapped back at the next pass (#16415). A minimized or maximized
/// window keeps the normal rect.
fn follow_os_window(app: &mut PhotocraftApp, ctx: &egui::Context, window: u64, container: egui::Id) {
    let host = WindowHost::of(ctx);
    let Some(facts) = host.viewport_of(container).and_then(|viewport| host.facts(viewport)) else { return };
    if facts.state != WindowState::Normal {
        return;
    }
    // The outer origin with the inner size, as the framework places the window: it takes the rect's size as the inner
    // size, so the outer size of a decorated window (the system's title bar) would grow it by its frame every pass.
    let placed = Rect::from_min_size(facts.outer.unwrap_or(facts.inner).min, facts.inner.size());
    if !placed.is_finite() || placed.width() <= 0.0 || placed.height() <= 0.0 {
        return;
    }
    if let Some((_, rect)) = app.ui.document_windows.floating.iter_mut().find(|(group, _)| group.id == window)
        && *rect != placed
    {
        *rect = placed;
    }
}

/// The title bar of a floating document window with the app-drawn frame (`Frame::Integrated`, the default on Windows
/// and Linux): the document's title, a move band and the caption buttons. `DocumentWindow` leaves that bar to its
/// body there (#16415); without it the window could not be moved, minimized, maximized or closed. With the system's
/// frame the OS draws the bar, and where OS windows are unavailable the fallback layer has its own title row, so
/// nothing is drawn then. Returns the bar's rect and whether its Close was clicked, or `None` with no bar.
fn window_title_bar(ui: &mut egui::Ui, title: Title) -> Option<ShownBar> {
    let host = WindowHost::of(ui.ctx());
    if !host.viewports_available() || matches!(host.frame(), Frame::Native) {
        return None;
    }
    let out = TitleBar::new().show_titled(ui, title, |_, _| {}, |_, _| {});
    Some(ShownBar { rect: out.response.rect, close: out.close_requested })
}

/// A floating document window's title bar as this pass showed it.
struct ShownBar {
    /// Its rect, in the window's points.
    rect: Rect,
    /// Its Close was clicked.
    close: bool,
}

/// The canvas of the document window `window` showing document `doc`, drawn in its own viewport.
/// Returns the view after this frame's input and the canvas rect.
///
/// While a framework modal is open in the main viewport, the framework drops presses, keys, wheel
/// and text in this viewport (`drop_under_modal`) and keeps moves and releases, so the canvas is as
/// inert as the main one's; no layer of its own is needed here.
pub(super) fn document_window_canvas(app: &mut PhotocraftApp, ui: &mut egui::Ui, window: u64, doc: usize, view: View, focused: bool) -> (View, Rect) {
    let mut out = (view.clone(), Rect::NOTHING);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(crate::theme::Tokens::get(ui.ctx()).canvas)).show(ui, |ui| {
        let rect = ui.available_rect_before_wrap();
        let host = CanvasHost { idx: doc, window: Some(window) };
        out = (canvas_view(app, ui, host, rect, view, focused).0, rect);
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(zoom: f32, fit: bool, fill: bool) -> View {
        let pending = [(fit, craft::ui::canvas::ViewCommands::FitOnScreen), (fill, craft::ui::canvas::ViewCommands::FillScreen)]
            .into_iter()
            .filter_map(|(asked, command)| asked.then_some(craft::ui::canvas::ViewRequest::Place(command)))
            .collect();
        View { zoom, pending, ..View::default() }
    }

    #[test]
    fn a_zoom_edit_wraps_the_window_and_a_fit_or_an_unchanged_zoom_does_not() {
        // (preference, before, after, asks)
        let rows = [
            (true, view(1.0, false, false), view(2.0, false, false), true),
            (true, view(1.0, false, false), view(1.0, false, false), false),
            (true, view(1.0, false, false), view(2.0, true, false), false),
            (true, view(1.0, false, false), view(2.0, false, true), false),
            (true, view(1.0, true, false), view(2.0, false, false), false),
            (false, view(1.0, false, false), view(2.0, false, false), false),
            (true, view(f32::NAN, false, false), view(f32::NAN, false, false), false),
        ];
        for (i, (preference, before, after, asks)) in rows.iter().enumerate() {
            assert_eq!(wants_wrap(*preference, before, after), *asks, "row {i}");
        }
    }
}
