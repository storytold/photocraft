//! The document windows of Window › New Window for Document and Float in Window: one framework
//! `DocumentWindow` for each floating group of `app.ui.document_windows`, with the document's canvas
//! as its body.

use craft::behavior::canvas::zoom::wrapped_canvas;
use craft::behavior::kurbo::Size;
use craft::ui::canvas::ToolHost;
use craft::ui::contract::{ShowContainer as _, SizeLimits};
use craft::ui::shell::{DocumentWindow, Wrapped};
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
        let mut view = start.clone();
        let inner = DocumentWindow::new(title).first_placed(rect).overlay(|o| o.size_limits(SizeLimits::at_least(MIN_SIZE))).show(ui, |ui, cx| {
            // A second run of the body in one pass continues from the first run's view; the wrap rule
            // compares with the view the window started the pass with, so both runs file the same ask.
            let (after, canvas) = document_window_canvas(app, ui, id, doc, view.clone(), focused);
            // The view the window's canvas showed this pass, kept by the tool host that ran in it.
            let host = CanvasHost { idx: doc, window: Some(id) };
            if wants_wrap(preference, &start, &after)
                && let Some(drawn) = ToolHost::shown_space(ui.ctx(), host.canvas_id()).map(|space| space.view)
            {
                DocumentWindow::wrap_in(cx, canvas, wrapped_canvas(&drawn, doc_size, canvas));
            }
            view = after;
        });
        let output = inner.output;
        if output.close_requested.is_some() {
            closed.push(id);
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
