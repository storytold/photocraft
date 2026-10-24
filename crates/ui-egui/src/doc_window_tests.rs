//! Tools in an extra document window (#6022): "Window ▸ Arrange ▸ New Window for Document" and a
//! tab dragged off the strip show the document in an OS window of its own, and drawing or making a
//! selection there edits the document like it does in the main window.

use egui::{PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DocWindow, Tool, View};

const WINDOW: u64 = 7;

/// A 400 × 300 document with a transparent layer and a window for it at 100 %. The main canvas is
/// at 50 %, so an input mapped through the main window's view lands somewhere else entirely.
fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1.0}})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    let main = &mut app.ui.views[0];
    (main.zoom, main.center, main.fit_pending) = (0.5, [100.0, 75.0], false);
    let view = View { zoom: 1.0, center: [200.0, 150.0], fit_pending: false, doc_size: [400, 300], ..View::default() };
    app.ui.windows.push(DocWindow { id: WINDOW, document: 0, view, open: true, position: None });
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::canvas::extra_windows(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(8);
    h
}

/// Where document point `(x, y)` is on screen in the window (100 %, centred on (200, 150) inside
/// the rulers when they are shown).
fn in_window(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let rect = crate::canvas::window_canvas_rect(&h.ctx, WINDOW).expect("the window's canvas was laid out");
    let rect = crate::rulers::content_rect(h.state(), rect);
    assert!(rect.width() > 120.0 && rect.height() > 90.0, "room to draw in: {rect:?}");
    rect.center() + vec2(x - 200.0, y - 150.0)
}

/// Press or release the left button at `p`.
fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
}

/// Press at `from`, drag through a few points to `to`, release.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(h, from, true);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    button(h, to, false);
    h.run_steps(2);
}

/// The document points of the one `paint.stroke` the journal holds.
fn stroke_points(h: &Harness<'static, PhotocraftApp>) -> Vec<(f64, f64)> {
    let strokes: Vec<_> = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").collect();
    assert_eq!(strokes.len(), 1, "one stroke");
    strokes[0].1["points"].as_array().unwrap().iter().map(|pt| (pt[0].as_f64().unwrap(), pt[1].as_f64().unwrap())).collect()
}

/// [`drag`] between two document points.
fn drag_doc(h: &mut Harness<'static, PhotocraftApp>, from: (f32, f32), to: (f32, f32)) {
    let (a, b) = (in_window(h, from.0, from.1), in_window(h, to.0, to.1));
    drag(h, a, b);
}

/// The active document's selection bounds (empty without one).
fn selection(h: &Harness<'static, PhotocraftApp>) -> Rect {
    h.state().session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

/// The active layer's alpha at document pixel `(x, y)` of the active document.
fn alpha_at_doc(h: &Harness<'static, PhotocraftApp>, x: i32, y: i32) -> f32 {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

#[test]
fn a_marquee_dragged_in_a_document_window_selects_by_that_windows_view() {
    let mut h = harness(Tool::RectMarquee);
    h.state_mut().ui.extras.snap = false;
    drag_doc(&mut h, (150.0, 110.0), (250.0, 190.0));
    assert_eq!(selection(&h), Rect::new(150, 110, 250, 190));
}

#[test]
fn a_brush_stroke_in_a_document_window_paints_the_document() {
    let mut h = harness(Tool::Brush);
    assert_eq!(alpha_at_doc(&h, 200, 150), 0.0);
    drag_doc(&mut h, (120.0, 150.0), (280.0, 150.0));
    let strokes = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").count();
    assert_eq!(strokes, 1, "one stroke, committed on the release");
    // Every point is the window's own mapping: the main canvas, at another zoom, would put the
    // same pointer somewhere else in the document, and it doesn't carry this gesture.
    let stroke = h.state().session.journal.iter().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
    for pt in stroke["points"].as_array().unwrap() {
        let (x, y) = (pt[0].as_f64().unwrap(), pt[1].as_f64().unwrap());
        assert!((119.0..=281.0).contains(&x) && (y - 150.0).abs() < 1.0, "a point from the window's drag: {pt}");
    }
    assert!(alpha_at_doc(&h, 200, 150) > 0.9, "the middle of the stroke is painted");
    assert_eq!(alpha_at_doc(&h, 200, 40), 0.0, "and nothing beside it");
}

#[test]
fn a_brush_stroke_in_a_window_with_rulers_lands_under_the_pointer() {
    // The window shows the rulers too (#6017), so its image sits inside them while its tools run,
    // as it does in every other frame.
    let mut h = harness(Tool::Brush);
    h.state_mut().ui.extras.rulers = true;
    h.run_steps(4);
    drag_doc(&mut h, (120.0, 150.0), (280.0, 150.0));
    for (x, y) in stroke_points(&h) {
        assert!((119.0..=281.0).contains(&x) && (y - 150.0).abs() < 1.0, "a point under the pointer: ({x}, {y})");
    }
    assert!(alpha_at_doc(&h, 200, 150) > 0.9);
    assert!(h.state().ui.extras.rulers, "and View › Rulers stays on");
}

#[test]
fn working_in_a_window_leaves_the_main_canvas_view_alone() {
    let mut h = harness(Tool::Brush);
    let main = h.state().ui.views[0].clone();
    let ppp = h.state().ppp;
    let rect = h.state().last_canvas_rect;
    drag_doc(&mut h, (120.0, 150.0), (280.0, 150.0));
    let app = h.state();
    assert_eq!(app.ui.views[0], main, "the main window keeps its own zoom and position");
    assert_eq!(app.ppp, ppp);
    assert_eq!(app.last_canvas_rect, rect);
    assert_eq!(app.ui.windows[0].view.zoom, 1.0, "and the window its own");
    assert_eq!(app.ui.windows[0].view.center, [200.0, 150.0]);
}

#[test]
fn a_press_in_the_window_of_another_document_brings_it_forward_and_draws_there() {
    let mut h = harness(Tool::Brush);
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().session.active_index(), Some(1), "the other document is the active one");
    drag_doc(&mut h, (120.0, 150.0), (280.0, 150.0));
    assert_eq!(h.state().session.active_index(), Some(0), "pressing in the window brings its document forward");
    assert!(alpha_at_doc(&h, 200, 150) > 0.9, "and the stroke is drawn on it, not on the document that was active");
    let other = &h.state().session.documents()[1];
    let layer = other.doc.layer(other.active_layer.unwrap()).unwrap();
    assert_eq!(layer.surface().unwrap().rgba(200, 150)[3], 0.0, "the other document is untouched");
}

#[test]
fn a_right_click_in_a_window_opens_no_pop_up_over_the_main_window() {
    let mut h = harness(Tool::Brush);
    let p = in_window(&h, 200.0, 150.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(h.state().ui.brush_picker.is_none(), "the Brush Preset picker opens in the main window, at the main window's coordinates");
    assert!(h.state().ui.canvas_tool_menu.is_none());
}

#[test]
fn the_type_tool_does_not_start_editing_from_a_window() {
    // Typing goes to the main window's keyboard focus, so a text edit started from the window
    // would be one nothing can type into.
    let mut h = harness(Tool::Type);
    drag_doc(&mut h, (100.0, 100.0), (101.0, 101.0));
    assert!(h.state().ui.text_edit.is_none());
}

#[test]
fn a_window_is_inert_while_a_dialog_is_open_in_the_main_window() {
    let mut h = harness(Tool::Brush);
    h.state_mut().ui.open_dialog(crate::state::DialogKind::About, Default::default());
    drag_doc(&mut h, (120.0, 150.0), (280.0, 150.0));
    assert!(h.state().session.journal.iter().all(|(id, _)| id != "paint.stroke"));
    assert_eq!(alpha_at_doc(&h, 200, 150), 0.0);
}

#[test]
fn a_press_in_another_documents_window_under_a_dialog_keeps_the_active_document() {
    // The dialog belongs to the document it was opened on.
    let mut h = harness(Tool::Brush);
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().session.active_index(), Some(1));
    h.state_mut().ui.open_dialog(crate::state::DialogKind::About, Default::default());
    drag_doc(&mut h, (120.0, 150.0), (280.0, 150.0));
    assert_eq!(h.state().session.active_index(), Some(1), "the press doesn't bring the window's document forward");
    assert!(h.state().session.journal.iter().all(|(id, _)| id != "paint.stroke"));
}

#[test]
fn a_drag_whose_window_closes_ends_where_it_was() {
    // Nothing else carries the gesture on: the main canvas, at another zoom, would map the pointer
    // somewhere else in the document.
    let mut h = harness(Tool::Brush);
    let (from, to) = (in_window(&h, 120.0, 150.0), in_window(&h, 200.0, 150.0));
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(&mut h, from, true);
    for i in 1..=4 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    assert!(h.state().drag.is_some(), "the stroke is in flight");
    h.state_mut().ui.windows.clear();
    // The pointer, still down, moves over the main canvas, which takes a pass to see the window
    // stopped.
    h.event(egui::Event::PointerMoved(egui::pos2(300.0, 300.0)));
    h.run_steps(3);
    assert!(h.state().drag.is_none(), "the abandoned stroke ended");
    for (x, y) in stroke_points(&h) {
        assert!((119.0..=201.0).contains(&x) && (y - 150.0).abs() < 1.0, "only the window's points: ({x}, {y})");
    }
    button(&mut h, egui::pos2(300.0, 300.0), false);
    let strokes = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").count();
    assert_eq!(strokes, 1, "the release adds no second stroke");
}

/// A second 400 × 300 document with a transparent layer, behind the window's (0) or in front.
fn second_document(h: &mut Harness<'static, PhotocraftApp>, active: usize) {
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().session.set_active(active);
    h.run_steps(4);
}

/// Has document `doc`'s active layer any paint along the window drags' row (y = 150)?
fn painted(h: &Harness<'static, PhotocraftApp>, doc: usize) -> bool {
    let st = &h.state().session.documents()[doc];
    let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (100..300).any(|x| surface.rgba(x, 150)[3] > 0.0)
}

#[test]
fn a_drag_whose_document_goes_back_is_dropped_not_finished_in_the_other() {
    // Ending it where it was would commit its points to the document that came forward.
    let mut h = harness(Tool::Brush);
    second_document(&mut h, 0);
    let (from, to) = (in_window(&h, 120.0, 150.0), in_window(&h, 200.0, 150.0));
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(&mut h, from, true);
    for i in 1..=4 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    assert!(h.state().drag.is_some(), "the stroke is in flight");
    // Window › the other document (or ⌃Tab) while the button is still down.
    h.state_mut().session.set_active(1);
    h.event(egui::Event::PointerMoved(egui::pos2(300.0, 300.0)));
    h.run_steps(3);
    assert!(h.state().drag.is_none(), "the abandoned stroke is gone");
    button(&mut h, egui::pos2(300.0, 300.0), false);
    assert!(h.state().session.journal.iter().all(|(id, _)| id != "paint.stroke"), "nothing was committed");
    assert!(!painted(&h, 1), "the document that came forward is untouched");
    assert!(!painted(&h, 0), "and so is the one the stroke started on");
}

#[test]
fn a_press_in_another_documents_window_mid_drag_leaves_the_drag_its_document() {
    // A second button pressed over the window while drawing in the main one.
    let mut h = harness(Tool::Brush);
    second_document(&mut h, 1);
    let main = &mut h.state_mut().ui.views[1];
    (main.zoom, main.center, main.fit_pending) = (3.0, [200.0, 150.0], false);
    h.run_steps(2);
    let window = crate::canvas::window_canvas_rect(&h.ctx, WINDOW).unwrap();
    let canvas = h.state().last_canvas_rect;
    let (from, to) = (canvas.max - vec2(200.0, 80.0), canvas.max - vec2(100.0, 80.0));
    assert!(!window.expand(40.0).contains(from), "a press on the main canvas, clear of the window: {from:?}, {window:?}");
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(&mut h, from, true);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
    assert!(h.state().drag.is_some(), "the stroke is in flight");
    let over = in_window(&h, 200.0, 150.0);
    h.event(egui::Event::PointerButton { pos: over, button: PointerButton::Middle, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().session.active_index(), Some(1), "the window's document stays behind");
    assert!(h.state().drag.is_some(), "and the stroke carries on");
    h.event(egui::Event::PointerButton { pos: over, button: PointerButton::Middle, pressed: false, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
    button(&mut h, to, false);
    assert!(!painted(&h, 0), "the window's document is untouched");
}

#[test]
fn every_tool_survives_a_drag_in_a_document_window() {
    // A press and drag over the image: an error or a no-op for a tool that needs more than that,
    // never a panic.
    let mut h = harness(Tool::Brush);
    for tool in Tool::ALL {
        h.state_mut().ui.tool = tool;
        h.run_steps(1);
        drag_doc(&mut h, (70.0, 60.0), (190.0, 160.0));
        h.state_mut().ui.dialogs.clear();
    }
}

#[test]
fn a_drag_held_still_in_a_window_carries_on() {
    // egui runs no pass while the button is held still, so the first one after a pause can come
    // seconds after the last: the window still owns its drag then.
    let mut h = harness(Tool::Brush);
    let (from, mid, to) = (in_window(&h, 120.0, 150.0), in_window(&h, 160.0, 150.0), in_window(&h, 200.0, 150.0));
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(&mut h, from, true);
    h.event(egui::Event::PointerMoved(mid));
    h.run_steps(1);
    assert!(h.state().drag.is_some(), "the stroke is in flight");
    // Five seconds in one step, then the next move.
    let t = h.ctx.input(|i| i.time);
    h.input_mut().time = Some(t + 5.0);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
    h.input_mut().time = None;
    assert!(h.state().drag.is_some(), "the stroke is still in flight after the pause");
    button(&mut h, to, false);
    let points = stroke_points(&h);
    assert!(points.iter().any(|&(x, _)| x > 199.0), "the stroke goes on to the move after the pause: {points:?}");
}

#[test]
fn a_drag_belongs_to_its_canvas_until_that_canvas_misses_a_pass() {
    use crate::canvas::{drag_is_abandoned, drag_is_foreign};
    let (a, b) = (egui::Id::new("a"), egui::Id::new("b"));
    assert!(!drag_is_foreign(None, a, 10), "a drag no canvas started (the control channel's) is anyone's");
    assert!(!drag_is_foreign(Some((a, 10)), a, 10), "its own canvas carries it on");
    assert!(drag_is_foreign(Some((a, 10)), b, 10), "another canvas leaves it alone");
    assert!(drag_is_foreign(Some((a, 10)), b, 11), "also when drawn before the owner in the next pass");
    assert!(!drag_is_abandoned(Some((a, 10)), b, 11), "which hasn't stopped");
    assert!(!drag_is_foreign(Some((a, 10)), b, 12), "but not one whose canvas missed a pass");
    assert!(drag_is_abandoned(Some((a, 10)), b, 12), "which is abandoned");
    assert!(!drag_is_abandoned(Some((a, 10)), a, 12), "a canvas never abandons its own drag");
    assert!(!drag_is_abandoned(None, a, 100), "nor is a drag no canvas started");
    assert!(drag_is_foreign(Some((a, u64::MAX)), b, u64::MAX), "the pass count can't overflow");
}
