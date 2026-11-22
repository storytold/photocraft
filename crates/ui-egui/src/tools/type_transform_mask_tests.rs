//! #12082: ⌘/Ctrl over a Type Mask session shows the transform box and a Ctrl+drag inside it moves
//! the mask text. The session's text lives on a hidden working layer (`type_tool/pointer.rs` sets
//! `visible: false` when the mask starts), and `frame` refused invisible layers, so the box never
//! appeared. An invisible ORDINARY type layer keeps the refusal (Done-when 3), and without the
//! modifier the mask keeps today's behaviour: no frame, and the box's move reach is not armed.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_doc::{LayerContent, LayerId, TextLayer};
use photocraft_geom::Affine;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp;
use crate::canvas::testing::{ToolEvent, to_screen, tool_event};
use crate::state::Tool;

fn new_app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    app.sync_views();
    // The drag's endpoints assert where they land, which snapping would move.
    app.ui.extras.snap = false;
    app.ui.tool_options.type_size = 48.0;
    app.ui.tool = tool;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

/// The canvas harness, as `type_tool/canvas_tests.rs` builds it.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().allow_missing_glyphs().with_context(app.ctx().clone()).with_size(vec2(1200.0, 800.0)).with_pixels_per_point(1.0).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The ⌘ key as the harness holds it on Windows/macOS (mirrors `canvas_tests::type_command`).
fn type_command() -> Modifiers {
    Modifiers { command: true, ctrl: true, ..Modifiers::NONE }
}

fn hold(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.run_steps(1);
}

/// Sets the main view's zoom and centre through the `ui.set` view command, as `canvas_tests` does.
fn set_view(h: &mut Harness<'static, Box<PhotocraftApp>>, zoom: f32, center: [f32; 2]) {
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"zoom": zoom, "center": center}));
    let app = h.state_mut();
    assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Reply::Done(_)), "ui.set sets the view");
    h.run_steps(2);
}

fn transform_press(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, down: bool, mods: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: mods });
    h.run_steps(1);
}

fn transform_drag(h: &mut Harness<'static, Box<PhotocraftApp>>, a: Pos2, b: Pos2, mods: Modifiers) {
    hold(h, mods);
    h.hover_at(a);
    h.run_steps(1);
    transform_press(h, a, true, mods);
    for k in 1..=6 {
        h.hover_at(a + (b - a) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    transform_press(h, b, false, mods);
    hold(h, Modifiers::NONE);
}

fn text(app: &PhotocraftApp, id: LayerId) -> TextLayer {
    let LayerContent::Text(t) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a text layer") };
    t.clone()
}

fn selection_bounds(app: &PhotocraftApp) -> Option<photocraft_geom::Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds())
}

fn assert_affine(a: Affine, b: Affine) {
    for (x, y) in a.m.into_iter().zip(b.m) {
        // Translations around 100 doc px at a fitted ppp cancel to ~1e-4 in f32 rounding.
        assert!((x - y).abs() < 1e-3, "{a:?} != {b:?}");
    }
}

/// Done-when 1: the frame (and with it the 8-handle box the overlay draws from it) exists over a
/// Horizontal and a Vertical Type Mask session while ⌘/Ctrl is held, and not without it.
/// Done-when 1: the frame (and with it the 8-handle box the overlay draws from it) exists over a
/// Horizontal and a Vertical Type Mask session while ⌘/Ctrl is held, and not without it.
#[test]
fn holding_ctrl_shows_the_transform_frame_over_a_type_mask() {
    for tool in [Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let mut app = new_app(tool);
        click(&mut app, 300.0, 250.0, Modifiers::NONE);
        assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask && ed.created), "{tool:?}: a mask session");
        let mut h = harness(app);
        assert!(crate::type_transform::frame(h.state_mut()).is_none(), "{tool:?}: no modifier, no frame");
        hold(&mut h, type_command());
        assert!(crate::type_transform::frame(h.state_mut()).is_some(), "{tool:?}: Ctrl over a Type Mask shows the frame");
        hold(&mut h, Modifiers::NONE);
        assert!(crate::type_transform::frame(h.state_mut()).is_none(), "{tool:?}: letting go hides it again");
    }
}

/// Done-when 2: a Ctrl+drag inside the box moves the mask text; committing then leaves the
/// selection at the same size, offset by the drag (reference: (+100, -40) at 100 %).
#[test]
fn ctrl_drag_inside_the_box_moves_the_mask_text_and_the_committed_selection_takes_the_offset() {
    // Baseline: the same mask committed without any drag.
    let mut base = new_app(Tool::HorizontalTypeMask);
    click(&mut base, 100.0, 300.0, Modifiers::NONE);
    crate::type_tool::commit_with(&mut base, Modifiers::NONE);
    let b0 = selection_bounds(&base).expect("the mask commits as a selection");

    let mut app = new_app(Tool::HorizontalTypeMask);
    click(&mut app, 100.0, 300.0, Modifiers::NONE);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let mut h = harness(app);
    // Zoom 100 %: the reference drag (+100, -40) screen px is then the same in document units.
    set_view(&mut h, 1.0, [400.0, 200.0]);
    let zoom = f64::from(h.state().point_zoom());
    hold(&mut h, type_command());
    let f = crate::type_transform::frame(h.state_mut()).expect("the frame with Ctrl held");
    let before = text(h.state(), id);
    let a = to_screen(h.state(), [f.state.pivot.x, f.state.pivot.y]);
    // The reference drag: (+100, -40) screen pixels at 100 %.
    let b = a + vec2(100.0, -40.0);
    transform_drag(&mut h, a, b, type_command());
    let after = text(h.state(), id);
    let (dx, dy) = (100.0 / zoom, -40.0 / zoom);
    assert_affine(after.transform, Affine::translate(dx, dy).mul(&before.transform));
    assert_eq!(after.text, before.text, "the text itself is untouched");
    assert!(h.state().ui.text_edit.as_ref().is_some_and(|ed| ed.mask && ed.layer == id.0), "still the mask session");
    assert!(h.state().ui.type_transform.is_none(), "the capture ended with the release");

    crate::type_tool::commit_with(h.state_mut(), Modifiers::NONE);
    assert!(h.state().ui.text_edit.is_none(), "the commit ends the session");
    let b1 = selection_bounds(h.state()).expect("the committed outline");
    for (got, want) in [
        (f64::from(b1.x0), f64::from(b0.x0) + dx),
        (f64::from(b1.y0), f64::from(b0.y0) + dy),
        (f64::from(b1.x1), f64::from(b0.x1) + dx),
        (f64::from(b1.y1), f64::from(b0.y1) + dy),
    ] {
        assert!((got - want).abs() < 1.0, "moved selection edge {got} != {want} (+{dx}, {dy})");
    }
    assert!((f64::from(b1.x1 - b1.x0) - f64::from(b0.x1 - b0.x0)).abs() < 1.0, "the size is kept");
    assert!((f64::from(b1.y1 - b1.y0) - f64::from(b0.y1 - b0.y0)).abs() < 1.0, "the size is kept");
}

/// Done-when 3 (guard, passes before and after): the invisible-layer refusal stays for a NORMAL
/// layer - a hidden ordinary type layer being edited gets no frame even with ⌘/Ctrl held (pattern 14).
#[test]
fn an_invisible_ordinary_type_layer_keeps_todays_refusal() {
    let mut app = new_app(Tool::Type);
    let id = LayerId(app.run("type.create", json!({"text": "AB", "size": 48, "x": 300, "y": 250})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    // The session over that layer: not a mask, `mask: false`.
    app.ui.text_edit = Some(crate::state::TextEdit { layer: id.0, document: None, caret: 1, anchor: 0, session: "s".into(), created: false, mask: false });
    let mut h = harness(app);
    hold(&mut h, type_command());
    assert!(crate::type_transform::frame(h.state_mut()).is_none(), "an invisible ordinary layer: no frame");
    assert!(h.state().ui.type_transform.is_none(), "and a hidden layer cannot be moved");
}

/// Guard: without the modifier the mask session keeps today's behaviour - no frame at all, so the
/// box's move reach is not armed: a drag beside the mask text never takes the box path, it is the
/// tool's own (the editor commits the session unchanged, and the selection is the plain outline).
/// This is the discriminating guard: with a wrongly armed reach the committed outline would move.
#[test]
fn without_ctrl_a_press_beside_the_mask_text_does_not_move_it() {
    // Baseline: the same mask committed without any drag.
    let mut base = new_app(Tool::HorizontalTypeMask);
    click(&mut base, 100.0, 300.0, Modifiers::NONE);
    crate::type_tool::commit_with(&mut base, Modifiers::NONE);
    let base_sel = base.session.active().unwrap().doc.selection.clone();
    assert!(base_sel.is_some(), "the baseline commits");

    let mut app = new_app(Tool::HorizontalTypeMask);
    click(&mut app, 100.0, 300.0, Modifiers::NONE);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let mut h = harness(app);
    let zoom = f64::from(h.state().point_zoom());
    assert!(crate::type_transform::frame(h.state_mut()).is_none(), "no modifier, no frame: no move reach either");
    // Where the move reach would have reached: 20 screen points past the text's right edge.
    let shown = text(h.state(), id).cache.expect("the laid-out glyphs").content_bounds();
    let a = to_screen(h.state(), [f64::from(shown.x1) + 20.0 / zoom, f64::from(shown.y0 + shown.y1) / 2.0]);
    transform_drag(&mut h, a, a + vec2(60.0, 30.0), Modifiers::NONE);
    assert!(h.state().ui.type_transform.is_none(), "the press is the tool's, never the box's");
    let sel = h.state().session.active().unwrap().doc.selection.clone();
    assert!(
        sel == base_sel,
        "the mask commits unchanged: {:?} vs the plain outline {:?}",
        sel.as_ref().map(|s| s.content_bounds()),
        base_sel.as_ref().map(|s| s.content_bounds())
    );
}
