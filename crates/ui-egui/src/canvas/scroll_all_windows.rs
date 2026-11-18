//! Hand tool › Scroll All Windows.
//!
//! With the option on, or with Shift held while dragging with the Hand, a drag in one document
//! view scrolls every other view that is shown: the main window's document, or the documents in its
//! Window › Arrange tiles, and the document windows of Float in Window and New Window for Document.
//! After each step of the drag, each of those views moves by the same distance in document pixels
//! as the dragged view, as the reference application does: a view at 100 % moves twice the screen
//! distance of one at 50 %. Each view keeps its own zoom and rotation. Documents in hidden tabs
//! keep their views.
//!
//! The tool host reports the pan after its Overscroll clamp, with the press's choice
//! (`ToolHostResponse::pan`); this module only moves the other views.

use craft::ui::canvas::ViewPan;

use crate::PhotocraftApp;
use crate::state::View;

/// The view a Hand drag moves itself: a document's view in the main window, or a document window
/// (the id of its floating group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Main(usize),
    Window(u64),
}

/// Whether the main window shows the Home screen or an open-file card instead of a document
/// canvas, as `tabs::documents` draws it.
fn main_canvas_hidden(app: &mut PhotocraftApp) -> bool {
    let n = app.session.documents().len();
    let opening = !app.jobs.opens.is_empty();
    (!opening && app.ui.chrome.shows_home(n, app.session.prefs().general.auto_show_home_screen)) || app.jobs.focus.is_some()
}

/// Move every shown view except `source` by the distance of `pan`, in document pixels: each view's
/// center moves by the same number of its own document's pixels. Does nothing unless the press chose
/// Scroll All Windows.
pub fn follow(app: &mut PhotocraftApp, source: Source, pan: ViewPan) {
    if !pan.scroll_all {
        return;
    }
    let delta = [pan.delta.x as f32, pan.delta.y as f32];
    if !(delta[0].is_finite() && delta[1].is_finite()) || delta == [0.0, 0.0] {
        return;
    }
    let place = |v: &mut View| {
        let c = [v.center[0] + delta[0], v.center[1] + delta[1]];
        if c[0].is_finite() && c[1].is_finite() {
            v.zoom_animating = false;
            v.zoom_anchor = None;
            v.clear_pending();
            v.center = c;
        }
    };
    if !main_canvas_hidden(app) {
        // The main window shows the arranged tiles, or else a docked document alone.
        let shown: Vec<usize> = match crate::canvas::arranged_cells(app, app.last_canvas_rect) {
            Some(cells) => cells.into_iter().map(|(d, _)| d).collect(),
            None => super::tabs::main_canvas_document(app).into_iter().collect(),
        };
        for d in shown {
            if source != Source::Main(d)
                && let Some(v) = app.ui.views.get_mut(d)
            {
                place(v);
            }
        }
    }
    // A New Window for Document draws its own view. Any other floating window draws its document's
    // own view (#11276), moved once however many windows share it, and not when the dragged window
    // is that view's own.
    let floating: Vec<(u64, Option<usize>)> = app
        .ui
        .document_windows
        .floating
        .iter()
        .map(|(g, _)| (g.id, g.shown.or_else(|| g.documents.first().copied()).and_then(|d| super::tabs::document_index(app, d))))
        .collect();
    let mut moved = Vec::new();
    if let Source::Window(id) = source
        && !app.ui.window_views.contains_key(&id)
        && let Some((_, Some(doc))) = floating.iter().find(|(w, _)| *w == id)
    {
        moved.push(*doc);
    }
    for (id, doc) in floating {
        if source == Source::Window(id) {
            continue;
        }
        if let Some(v) = app.ui.window_views.get_mut(&id) {
            place(v);
        } else if let Some(doc) = doc.filter(|d| !moved.contains(d)) {
            moved.push(doc);
            if let Some(v) = app.ui.views.get_mut(doc) {
                place(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use craft::behavior::dock::{Arrangement, DocumentGroup};
    use egui::{Pos2, Rect, vec2};
    use serde_json::json;

    use super::*;

    fn view(zoom: f32, center: [f32; 2], rotation: f32) -> View {
        View { zoom, center, rotation, pending: Vec::new(), ..View::default() }
    }

    /// Where a view is stored: a document's view, or a New Window's own.
    #[derive(Clone, Copy)]
    enum Spot {
        Doc(usize),
        Window(u64),
    }

    /// How the three documents (400 × 300, 800 × 600, 200 × 100) are shown.
    #[derive(Clone, Copy)]
    enum Layout {
        /// Documents 0 and 1 tiled side by side; 2 in a hidden tab with a New Window (id 7, own view).
        Tiles,
        /// One tab group showing document 0; New Windows 3 (document 1) and 4 (document 2), own views.
        Tabs,
        /// Document 0 floating alone (window 5, no own view); document 1 docked and shown.
        Floated,
        /// As `Floated`, with a New Window 6 of document 0 beside window 5.
        FloatedTwice,
    }

    /// The app with the views at known centers and the windows of `layout`.
    fn app_with(layout: Layout) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for (w, h) in [(400, 300), (800, 600), (200, 100)] {
            app.run("file.new", json!({"width": w, "height": h})).unwrap();
        }
        app.sync_views();
        app.session.set_active(0);
        app.ui.views[0] = view(1.0, [200.0, 150.0], 0.0);
        app.ui.views[1] = view(2.0, [400.0, 300.0], 0.0);
        app.ui.views[2] = view(1.0, [100.0, 50.0], 0.0);
        let id = |i: usize| app.session.documents()[i].doc.id;
        let (d0, d1, d2) = (id(0), id(1), id(2));
        let group = |id: u64, documents: Vec<photocraft_doc::DocId>, shown| {
            let mut g = DocumentGroup::new(id, documents);
            g.shown = Some(shown);
            g
        };
        let area = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 200.0));
        let windows = &mut app.ui.document_windows;
        match layout {
            Layout::Tiles => {
                windows.docked = vec![group(0, vec![d0], d0), group(1, vec![d1, d2], d1)];
                windows.arrangement = Some(Arrangement::TwoUpVertical);
                windows.floating = vec![(group(7, vec![d2], d2), area)];
                app.ui.window_views.insert(7, view(0.5, [100.0, 50.0], 90.0));
            }
            Layout::Tabs => {
                windows.docked = vec![group(0, vec![d0, d1, d2], d0)];
                windows.floating = vec![(group(3, vec![d1], d1), area), (group(4, vec![d2], d2), area)];
                app.ui.window_views.insert(3, view(1.0, [400.0, 300.0], 0.0));
                app.ui.window_views.insert(4, view(1.0, [100.0, 50.0], 0.0));
            }
            Layout::Floated | Layout::FloatedTwice => {
                windows.docked = vec![group(0, vec![d1, d2], d1)];
                windows.floating = vec![(group(5, vec![d0], d0), area)];
                if matches!(layout, Layout::FloatedTwice) {
                    windows.floating.push((group(6, vec![d0], d0), area));
                    app.ui.window_views.insert(6, view(1.0, [200.0, 150.0], 0.0));
                }
            }
        }
        app.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        app
    }

    fn center(app: &PhotocraftApp, at: Spot) -> [f32; 2] {
        match at {
            Spot::Doc(d) => app.ui.views[d].center,
            Spot::Window(id) => app.ui.window_views[&id].center,
        }
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    /// How the main canvas is hidden in a row.
    #[derive(Clone, Copy, PartialEq)]
    enum Hidden {
        No,
        Home,
        Card,
    }

    #[test]
    fn a_view_pan_moves_every_other_shown_view() {
        use Spot::{Doc, Window};
        // (name, layout, hidden, source, scroll_all, delta, expected centers; a view not listed stays)
        type Row<'a> = (&'a str, Layout, Hidden, Source, bool, [f64; 2], Vec<(Spot, [f32; 2])>);
        let rows: Vec<Row<'_>> = vec![
            (
                "a tile's pan moves the other tile and the window by the same document pixels, not the hidden tab",
                Layout::Tiles,
                Hidden::No,
                Source::Main(0),
                true,
                [-50.0, 20.0],
                vec![(Doc(0), [200.0, 150.0]), (Doc(1), [350.0, 320.0]), (Window(7), [50.0, 70.0]), (Doc(2), [100.0, 50.0])],
            ),
            (
                "a window's pan moves the main view and skips the window and the hidden tabs",
                Layout::Tabs,
                Hidden::No,
                Source::Window(3),
                true,
                [100.0, -75.0],
                vec![(Window(3), [400.0, 300.0]), (Doc(0), [300.0, 75.0]), (Doc(1), [400.0, 300.0]), (Window(4), [200.0, -25.0])],
            ),
            (
                "a floating document's pan moves the docked document shown in the main window",
                Layout::Floated,
                Hidden::No,
                Source::Window(5),
                true,
                [-100.0, -90.0],
                vec![(Doc(1), [300.0, 210.0]), (Doc(0), [200.0, 150.0])],
            ),
            (
                "the main view's pan moves a floating window's document view",
                Layout::Floated,
                Hidden::No,
                Source::Main(1),
                true,
                [-100.0, -90.0],
                vec![(Doc(0), [100.0, 60.0]), (Doc(1), [400.0, 300.0])],
            ),
            (
                "a document view shared by two windows moves once",
                Layout::FloatedTwice,
                Hidden::No,
                Source::Window(6),
                true,
                [10.0, 5.0],
                vec![(Doc(0), [210.0, 155.0]), (Window(6), [200.0, 150.0]), (Doc(1), [410.0, 305.0])],
            ),
            (
                "Home hides the main view; another window follows",
                Layout::Tabs,
                Hidden::Home,
                Source::Window(3),
                true,
                [-30.0, -10.0],
                vec![(Doc(0), [200.0, 150.0]), (Window(4), [70.0, 40.0])],
            ),
            (
                "an open card hides the main view; another window follows",
                Layout::Tabs,
                Hidden::Card,
                Source::Window(3),
                true,
                [-30.0, -10.0],
                vec![(Doc(0), [200.0, 150.0]), (Window(4), [70.0, 40.0])],
            ),
            (
                "a pan without the Scroll All Windows choice moves nothing",
                Layout::Tiles,
                Hidden::No,
                Source::Main(0),
                false,
                [-50.0, 20.0],
                vec![(Doc(1), [400.0, 300.0]), (Window(7), [100.0, 50.0])],
            ),
        ];
        for (name, layout, hidden, source, scroll_all, delta, expected) in rows {
            let mut app = app_with(layout);
            match hidden {
                Hidden::No => {}
                Hidden::Home => app.ui.chrome.home = Some(app.session.documents().len()),
                Hidden::Card => {
                    let job = photocraft_engine::jobs::JobId(9);
                    app.jobs.opens.push(crate::jobs_ui::OpenTab { job, name: "opening.psd".into(), path: None, slot: None, order: None });
                    app.jobs.focus = Some(job);
                }
            }
            follow(&mut app, source, ViewPan::new(craft::behavior::kurbo::Vec2::new(delta[0], delta[1]), scroll_all));
            for (at, want) in expected {
                let got = center(&app, at);
                let label = match at {
                    Doc(d) => format!("document {d}"),
                    Window(w) => format!("window {w}"),
                };
                assert!(near(got, want), "{name}: {label} is at {got:?}, wanted {want:?}");
            }
            assert_eq!(app.ui.views[1].zoom, 2.0, "{name}: each view keeps its zoom");
        }
    }
}
