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

/// Where document point `(x, y)` is on screen in the window (100 %, centred on (200, 150)).
fn in_window(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let rect = crate::canvas::window_canvas_rect(&h.ctx, WINDOW).expect("the window's canvas was laid out");
    assert!(rect.width() > 120.0 && rect.height() > 90.0, "room to draw in: {rect:?}");
    rect.center() + vec2(x - 200.0, y - 150.0)
}

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

/// [`drag`] between two document points.
fn drag_doc(h: &mut Harness<'static, PhotocraftApp>, from: (f32, f32), to: (f32, f32)) {
    let (a, b) = (in_window(h, from.0, from.1), in_window(h, to.0, to.1));
    drag(h, a, b);
}

fn selection(h: &Harness<'static, PhotocraftApp>) -> Rect {
    h.state().session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

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
fn a_drag_belongs_to_its_canvas_until_that_canvas_stops_running() {
    use crate::canvas::{DRAG_OWNER_TTL, drag_is_foreign};
    let (a, b) = (egui::Id::new("a"), egui::Id::new("b"));
    assert!(!drag_is_foreign(None, a, 10.0), "a drag no canvas started (the control channel's) is anyone's");
    assert!(!drag_is_foreign(Some((a, 10.0)), a, 10.5), "its own canvas carries it on");
    assert!(drag_is_foreign(Some((a, 10.0)), b, 10.5), "another canvas leaves it alone");
    assert!(!drag_is_foreign(Some((a, 10.0)), b, 10.0 + DRAG_OWNER_TTL), "but not one whose canvas stopped running");
}
