//! Hand tool › Scroll All Windows.
//!
//! With the option on, or with Shift held while dragging with the Hand, a drag in one document
//! view scrolls every other view that is shown: the main window's document, or the documents in its
//! Window › Arrange tiles, and the document windows of Float in Window and New Window for Document.
//! After each step of the drag, each of those views puts its center at the same relative location
//! as the dragged view's center: the same fraction of its own document's width and height. Each
//! view keeps its own zoom and rotation. Documents in hidden tabs keep their views.

use crate::PhotocraftApp;
use crate::state::View;

/// The view a Hand drag moves itself: a document's view in the main window, or a document window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Main(usize),
    Window(u64),
}

/// The view that the canvas of document `idx` being drawn shows: the document window
/// `canvas::extra_windows` is drawing, or else the document's view in the main window.
pub fn source(app: &PhotocraftApp, idx: usize) -> Source {
    app.drawing_window.map_or(Source::Main(idx), Source::Window)
}

/// Whether a Hand drag scrolls every shown view: Scroll All Windows is on, or Shift is held.
pub fn active(app: &PhotocraftApp, shift: bool) -> bool {
    app.ui.tool_options.hand_scroll_all || shift
}

/// Put every shown view except `source` at the relative location of `center`, a point in document
/// `doc`: each view's center becomes the same fraction of its own document's width and height.
pub fn follow(app: &mut PhotocraftApp, source: Source, doc: usize, center: [f32; 2]) {
    let sizes: Vec<[f32; 2]> = app.session.documents().iter().map(|d| [d.doc.size.width as f32, d.doc.size.height as f32]).collect();
    let Some(&[w, h]) = sizes.get(doc) else {
        return;
    };
    let rel = [center[0] / w, center[1] / h];
    if !(w > 0.0 && h > 0.0 && rel[0].is_finite() && rel[1].is_finite()) {
        return;
    }
    let place = |v: &mut View, d: usize| {
        let Some(&[w, h]) = sizes.get(d) else {
            return;
        };
        let c = [rel[0] * w, rel[1] * h];
        if v.center != c {
            v.zoom_animating = false;
            v.zoom_anchor = None;
            v.fit_pending = false;
            v.fill_pending = false;
            v.center = c;
        }
    };
    // The main window shows the arranged tiles, or else the active document alone.
    let shown: Vec<usize> = match crate::canvas::arranged_cells(app, app.last_canvas_rect) {
        Some(cells) => cells.into_iter().map(|(d, _)| d).collect(),
        None => app.session.active_index().into_iter().collect(),
    };
    for d in shown {
        if source != Source::Main(d)
            && let Some(v) = app.ui.views.get_mut(d)
        {
            place(v, d);
        }
    }
    for w in app.ui.windows.iter_mut().filter(|w| w.open && source != Source::Window(w.id)) {
        place(&mut w.view, w.document);
    }
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, vec2};
    use serde_json::json;

    use super::*;
    use crate::state::{DocWindow, Tool};

    fn view(zoom: f32, center: [f32; 2], rotation: f32) -> View {
        View { zoom, center, rotation, fit_pending: false, fill_pending: false, ..View::default() }
    }

    /// Three documents of different sizes, the first two tiled side by side and the third in a
    /// hidden tab.
    fn app_with_tiles() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for (w, h) in [(400, 300), (800, 600), (200, 100)] {
            app.run("file.new", json!({"width": w, "height": h})).unwrap();
        }
        app.sync_views();
        app.session.set_active(0);
        app.ui.view.arrange = "twoUpVertical".into();
        app.arrange_start = 0;
        app.ui.views[0] = view(1.0, [200.0, 150.0], 0.0);
        app.ui.views[1] = view(2.0, [400.0, 300.0], 0.0);
        app.ui.views[2] = view(1.0, [100.0, 50.0], 0.0);
        app
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn a_drag_puts_the_other_shown_views_at_the_same_relative_location() {
        let mut app = app_with_tiles();
        app.ui.windows.push(DocWindow { id: 7, document: 2, view: view(0.5, [100.0, 50.0], 90.0), open: true, position: None });
        // The dragged 400 × 300 view's center moved to a quarter of the width and a fifth of the height.
        follow(&mut app, Source::Main(0), 0, [100.0, 60.0]);
        assert_eq!(app.ui.views[0].center, [200.0, 150.0], "the dragged view's canvas stores its own view");
        // The 800 × 600 tile at 200 % centers on the same fractions of its own document.
        assert!(near(app.ui.views[1].center, [200.0, 120.0]), "{:?}", app.ui.views[1].center);
        assert_eq!(app.ui.views[1].zoom, 2.0, "each view keeps its zoom");
        // The document window shows the 200 × 100 document at 50 %, turned 90°.
        let w = &app.ui.windows[0].view;
        assert!(near(w.center, [50.0, 20.0]), "{:?}", w.center);
        assert_eq!((w.zoom, w.rotation), (0.5, 90.0), "and its rotation");
        // The third document's main view is in a tab hidden behind the tiles: it stays.
        assert_eq!(app.ui.views[2].center, [100.0, 50.0]);
    }

    #[test]
    fn a_drag_in_a_document_window_scrolls_the_main_window_and_skips_itself() {
        let mut app = app_with_tiles();
        app.ui.view.arrange = String::new();
        app.ui.windows.push(DocWindow { id: 3, document: 1, view: view(1.0, [400.0, 300.0], 0.0), open: true, position: None });
        app.ui.windows.push(DocWindow { id: 4, document: 2, view: view(1.0, [100.0, 50.0], 0.0), open: false, position: None });
        // The window's 800 × 600 view moved its center to three quarters of the width and a quarter of the height.
        follow(&mut app, Source::Window(3), 1, [600.0, 150.0]);
        assert_eq!(app.ui.windows[0].view.center, [400.0, 300.0], "the dragged window's canvas stores its own view");
        // Tabs: the main window shows the active 400 × 300 document only.
        assert!(near(app.ui.views[0].center, [300.0, 75.0]), "{:?}", app.ui.views[0].center);
        assert_eq!(app.ui.views[1].center, [400.0, 300.0], "a hidden tab stays");
        assert_eq!(app.ui.windows[1].view.center, [100.0, 50.0], "a closed window stays");
    }

    #[test]
    fn the_source_is_the_document_window_being_drawn() {
        let mut app = app_with_tiles();
        assert_eq!(source(&app, 1), Source::Main(1));
        app.drawing_window = Some(9);
        assert_eq!(source(&app, 1), Source::Window(9));
    }

    /// A Hand drag with `option` and the Shift key as given, on the main canvas of the 2000 × 1500
    /// document 0 or (`in_window`) on the canvas of a window showing the 1000 × 750 document 1, as
    /// `canvas::extra_windows` draws it. Both views start at their document's center at 100 %.
    /// Returns the dragged view's center and the stored centers of document 0's main view and of
    /// the window's view.
    fn drag_on_canvas(option: bool, shift: bool, in_window: bool) -> ([f32; 2], [f32; 2], [f32; 2]) {
        drag_on_canvas_with(option, shift, false, in_window)
    }

    /// As `drag_on_canvas`, with Shift held in Window › Modifier Keys when `sticky_shift` is set.
    fn drag_on_canvas_with(option: bool, shift: bool, sticky_shift: bool, in_window: bool) -> ([f32; 2], [f32; 2], [f32; 2]) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("file.new", json!({"width": 1000, "height": 750})).unwrap();
        app.run("prefs.set", json!({"path": "tools.enableFlickPanning", "value": false})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.tool = Tool::Hand;
        app.ui.tool_options.hand_scroll_all = option;
        app.ui.shell.sticky_shift = sticky_shift;
        app.ui.views[0] = view(1.0, [1000.0, 750.0], 0.0);
        app.ui.windows.push(DocWindow { id: 5, document: 1, view: view(1.0, [500.0, 375.0], 0.0), open: true, position: None });
        let mut v = if in_window { view(1.0, [500.0, 375.0], 0.0) } else { view(1.0, [1000.0, 750.0], 0.0) };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let modifiers = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                if in_window {
                    app.drawing_window = Some(5);
                    v = crate::canvas::canvas_view(app, ui, 1, rect, v.clone(), false);
                    app.drawing_window = None;
                } else {
                    v = crate::canvas::canvas_view(app, ui, 0, rect, v.clone(), true);
                }
            });
            out.textures_delta.clear();
        };
        let start = Pos2::new(200.0, 150.0);
        let press = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        frame(&mut app, vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let end = start + vec2(-60.0, 30.0);
        for i in 1..=6 {
            frame(&mut app, vec![egui::Event::PointerMoved(start + (end - start) * (i as f32 / 6.0))]);
        }
        frame(&mut app, vec![press(end, false)]);
        (v.center, app.ui.views[0].center, app.ui.windows[0].view.center)
    }

    #[test]
    fn the_option_or_shift_makes_a_hand_drag_scroll_the_document_windows_too() {
        let (dragged, _, window) = drag_on_canvas(false, false, false);
        assert!(near(dragged, [1060.0, 720.0]), "the dragged view follows the pointer: {dragged:?}");
        assert_eq!(window, [500.0, 375.0], "off: the window stays");
        // 1060 / 2000 and 720 / 1500 of the 1000 × 750 document.
        let moved = [530.0, 360.0];
        let (_, _, window) = drag_on_canvas(true, false, false);
        assert!(near(window, moved), "on: it scrolls to the same relative location: {window:?}");
        let (_, _, window) = drag_on_canvas(false, true, false);
        assert!(near(window, moved), "Shift turns it on for the drag: {window:?}");
    }

    #[test]
    fn shift_held_in_the_modifier_keys_panel_makes_a_hand_drag_scroll_the_document_windows_too() {
        let (dragged, _, window) = drag_on_canvas_with(false, false, true, false);
        assert!(near(dragged, [1060.0, 720.0]), "the dragged view follows the pointer: {dragged:?}");
        assert!(near(window, [530.0, 360.0]), "sticky Shift turns it on for the drag: {window:?}");
    }

    #[test]
    fn a_hand_drag_in_a_document_window_scrolls_the_main_window_once() {
        let (dragged, main, window) = drag_on_canvas(false, false, true);
        assert!(near(dragged, [560.0, 345.0]), "the dragged window follows the pointer: {dragged:?}");
        assert_eq!((main, window), ([1000.0, 750.0], [500.0, 375.0]), "off: nothing else moves");
        // The window's own canvas moved its view (`dragged`), and `follow` leaves its stored view
        // alone: `extra_windows` stores the canvas's view, so the window moves once.
        let (dragged, main, window) = drag_on_canvas(true, false, true);
        assert!(near(dragged, [560.0, 345.0]), "{dragged:?}");
        // 560 / 1000 and 345 / 750 of the 2000 × 1500 document.
        assert!(near(main, [1120.0, 690.0]), "on: the main window scrolls to the same relative location: {main:?}");
        assert_eq!(window, [500.0, 375.0]);
    }

    #[test]
    fn with_overscroll_off_the_other_windows_follow_the_clamped_center() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("file.new", json!({"width": 1000, "height": 750})).unwrap();
        app.run("prefs.set", json!({"values": {"tools.enableFlickPanning": false, "tools.overscroll": false}})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.tool = Tool::Hand;
        app.ui.tool_options.hand_scroll_all = true;
        app.ui.windows.push(DocWindow { id: 5, document: 1, view: view(4.0, [500.0, 375.0], 0.0), open: true, position: None });
        // A 400 × 300 canvas at 100 % keeps the center 200 px or more inside the left edge.
        let mut v = view(1.0, [220.0, 750.0], 0.0);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                v = crate::canvas::canvas_view(app, ui, 0, rect, v.clone(), true);
            });
            out.textures_delta.clear();
        };
        let start = Pos2::new(200.0, 150.0);
        let press =
            |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let end = start + vec2(60.0, 0.0);
        for i in 1..=6 {
            frame(&mut app, vec![egui::Event::PointerMoved(start + (end - start) * (i as f32 / 6.0))]);
        }
        frame(&mut app, vec![press(end, false)]);
        frame(&mut app, vec![]);
        assert!(near(v.center, [200.0, 750.0]), "the dragged view stops at its limit: {:?}", v.center);
        // 200 / 2000 of the 1000 × 750 document. The window at 400 % has room to go further left.
        let window = app.ui.windows[0].view.center;
        assert!(near(window, [100.0, 375.0]), "{window:?}");
    }

    #[test]
    fn a_flick_panning_glide_moves_the_other_windows_too() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("file.new", json!({"width": 1000, "height": 750})).unwrap();
        app.run("prefs.set", json!({"path": "tools.enableFlickPanning", "value": true})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.tool = Tool::Hand;
        app.ui.windows.push(DocWindow { id: 5, document: 1, view: view(1.0, [500.0, 375.0], 0.0), open: true, position: None });
        let mut v = view(1.0, [1000.0, 750.0], 0.0);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> [f32; 2] {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                v = crate::canvas::canvas_view(app, ui, 0, rect, v.clone(), true);
            });
            out.textures_delta.clear();
            v.center
        };
        let start = Pos2::new(200.0, 150.0);
        let press =
            |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        // Shift turns Scroll All Windows on for the drag, and is let go with the button.
        frame(&mut app, vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let mut end = start;
        for i in 1..=8 {
            end = start + vec2(-20.0 * i as f32, 0.0);
            frame(&mut app, vec![egui::Event::PointerMoved(end)]);
        }
        let released = frame(&mut app, vec![press(end, false), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        // The window's document is half the size, so on each frame of the glide its center is at
        // half the dragged view's center.
        let mut glided = released;
        for _ in 0..5 {
            glided = frame(&mut app, vec![]);
            let half = [glided[0] / 2.0, glided[1] / 2.0];
            assert!(near(app.ui.windows[0].view.center, half), "{:?} {half:?}", app.ui.windows[0].view.center);
        }
        assert!(glided[0] > released[0], "the dragged view glides on: {released:?} {glided:?}");
    }

    #[test]
    fn the_hand_tool_options_bar_shows_scroll_all_windows() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::Hand;
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::options_bar(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.run_steps(4);
        assert!(!h.state().ui.tool_options.hand_scroll_all, "off by default");
        h.get_by_label("Scroll All Windows").click();
        h.run_steps(2);
        assert!(h.state().ui.tool_options.hand_scroll_all, "the checkbox turns it on");
        h.get_by_label("Scroll All Windows").click();
        h.run_steps(2);
        assert!(!h.state().ui.tool_options.hand_scroll_all, "and off again");
    }
}
