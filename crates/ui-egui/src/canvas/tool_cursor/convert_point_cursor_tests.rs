//! #12746: Convert Point's cursor says what a press would do. Over an anchor or a shown handle end
//! of the targeted path it is the convert-point caret; over a segment, over empty canvas, with no
//! path targeted and over a locked shape layer's anchor it is the black arrow (the Path Selection
//! arrow of #13292). ⌥ keeps the caret over an anchor and turns the arrow elsewhere into the arrow
//! with "+", ⌘/Ctrl is the black arrow everywhere, ⇧ changes nothing, and Preferences › Cursors ›
//! Other Cursors = Precise keeps the crosshair. The tests drive the real canvas (`document_area`)
//! and compare the OS cursor bitmap it hands the platform with the arrow's, the Convert Point
//! caret's and the crosshair's.

use egui::{Color32, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::{PenVariant, Shape, rasterize};
use crate::PhotocraftApp;
use crate::anchor_tools::delete_anchor_targeted_path_tests::{select_layer, select_saved_path, setup};
use crate::state::Tool;

/// Path 1 is the square (50,450)-(150,550); the shape layer is the rectangle (450,100)-(650,250).
const SQUARE_CORNER: [f32; 2] = [50.0, 450.0];
const SQUARE_TOP: [f32; 2] = [100.0, 450.0];
const SQUARE_MIDDLE: [f32; 2] = [100.0, 500.0];
const SHAPE_CORNER: [f32; 2] = [450.0, 100.0];
const SHAPE_TOP: [f32; 2] = [550.0, 100.0];
/// The out-handle end of the curved path's first anchor: empty canvas, 50 pt off every segment.
const HANDLE_END: [f32; 2] = [50.0, 400.0];

/// The canvas at 100 % over `app`, the view centred on `view` (a document point).
fn harness(app: PhotocraftApp, view: [f32; 2]) -> Harness<'static, PhotocraftApp> {
    let mut app = app;
    app.sync_views();
    app.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = view;
    v.fit_pending = false;
    h.run_steps(2);
    h
}

/// The screen point the canvas at 100 % shows the document point `at` at.
fn screen(h: &Harness<'static, PhotocraftApp>, at: [f32; 2], view: [f32; 2]) -> Pos2 {
    h.state().last_canvas_rect.center() + vec2(at[0] - view[0], at[1] - view[1])
}

/// Hover the document point `at` with `mods` held; the screen point it lands on.
fn hover(h: &mut Harness<'static, PhotocraftApp>, at: [f32; 2], view: [f32; 2], mods: Modifiers) -> Pos2 {
    h.event(egui::Event::ModifiersChanged(mods));
    let p = screen(h, at, view);
    h.hover_at(p);
    h.run_steps(3);
    p
}

/// Press at `from` with `mods` held and drag to `to`, leaving the button down: the tool's drag is
/// then in progress.
fn drag_from(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], to: [f32; 2], view: [f32; 2], mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    let p = screen(h, from, view);
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: mods });
    h.run_steps(1);
    h.hover_at(screen(h, to, view));
    h.run_steps(3);
}

/// Press at `from` with `mods` held and release: a click, leaving no drag in progress.
fn click_at(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], view: [f32; 2], mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    let p = screen(h, from, view);
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: mods });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: mods });
    h.run_steps(2);
}

#[derive(Debug, PartialEq)]
enum Shown {
    Caret,
    Arrow,
    Crosshair,
    Other,
}

/// Which cursor the last frame asked the platform for.
fn shown(h: &Harness<'static, PhotocraftApp>) -> Shown {
    let out = &h.output().platform_output;
    let scale = super::native_scale(&h.ctx);
    let same =
        |a: &egui::CustomCursorImage, b: Option<egui::CustomCursorImage>| b.is_some_and(|b| a.size == b.size && a.hotspot == b.hotspot && a.rgba == b.rgba);
    match &out.cursor_image {
        Some(i) if same(i, rasterize(&Shape::Arrow, scale)) => Shown::Arrow,
        Some(i) if same(i, PenVariant::from_tool(Tool::ConvertPoint).and_then(|v| super::pen::rasterize(v, scale))) => Shown::Caret,
        Some(i) if same(i, rasterize(&super::STANDARD_CROSSHAIR, scale)) => Shown::Crosshair,
        _ => Shown::Other,
    }
}

/// The "+" a cursor badge is: `tool_feedback::draw_badge`'s two white strokes of 1.2 points crossing
/// 30 points right of and 11 below the pointer.
fn drawn_plus(h: &Harness<'static, PhotocraftApp>, at: Pos2) -> bool {
    let c = at + vec2(30.0, 11.0);
    let glyph = |a: Pos2, b: Pos2| {
        h.output().shapes.iter().any(|s| {
            matches!(&s.shape, egui::Shape::LineSegment { points, stroke }
                if stroke.color == Color32::WHITE && stroke.width == 1.2
                    && (points[0] - a).length() < 0.01 && (points[1] - b).length() < 0.01)
        })
    };
    glyph(c - vec2(3.5, 0.0), c + vec2(3.5, 0.0)) && glyph(c - vec2(0.0, 3.5), c + vec2(0.0, 3.5))
}

/// The square's Path 1 targeted, the pointer at `at` on it, `mods` held.
fn on_path_one(at: [f32; 2], mods: Modifiers) -> Shown {
    let (mut app, _) = setup(Tool::ConvertPoint);
    select_saved_path(&mut app);
    let mut h = harness(app, SQUARE_MIDDLE);
    hover(&mut h, at, SQUARE_MIDDLE, mods);
    shown(&h)
}

/// The square's Path 1 with its first knot curved (out-handle ends at `HANDLE_END`).
fn curved_path_one() -> (PhotocraftApp, u64) {
    let (mut app, _) = setup(Tool::ConvertPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [{"anchor": [50, 450], "in": [50, 450], "out": [50, 400]}, [150, 450], [150, 550], [50, 550]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    (app, 0)
}

#[test]
fn over_an_anchor_it_is_the_caret_and_over_a_segment_the_black_arrow() {
    assert_eq!(on_path_one(SQUARE_CORNER, Modifiers::NONE), Shown::Caret);
    assert_eq!(on_path_one(SQUARE_TOP, Modifiers::NONE), Shown::Arrow);
}

#[test]
fn over_empty_canvas_it_is_the_black_arrow() {
    assert_eq!(on_path_one(SQUARE_MIDDLE, Modifiers::NONE), Shown::Arrow);
}

#[test]
fn with_no_path_targeted_it_is_the_black_arrow_everywhere() {
    // The active layer is the pixel layer "Red", so the shape layer's anchors are not drawn and no
    // path is targeted, whatever lies under the pointer.
    for at in [SHAPE_CORNER, SHAPE_TOP, [300.0, 150.0]] {
        let (app, _) = setup(Tool::ConvertPoint);
        let mut h = harness(app, SHAPE_TOP);
        hover(&mut h, at, SHAPE_TOP, Modifiers::NONE);
        assert_eq!(shown(&h), Shown::Arrow, "{at:?}");
    }
}

#[test]
fn over_the_anchor_of_the_active_shape_it_is_the_caret_and_of_a_locked_one_the_black_arrow() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    select_layer(&mut app, shape);
    let mut h = harness(app, SHAPE_TOP);
    hover(&mut h, SHAPE_CORNER, SHAPE_TOP, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Caret, "its anchors are drawn, so a press converts");
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    hover(&mut h, SHAPE_CORNER, SHAPE_TOP, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Arrow);
}

#[test]
fn over_a_shown_handle_end_it_is_the_caret_and_off_it_the_black_arrow() {
    let (app, _) = curved_path_one();
    let mut h = harness(app, SQUARE_MIDDLE);
    // Nothing selected: the handle is not drawn, so the pointer sits on empty canvas.
    hover(&mut h, HANDLE_END, SQUARE_MIDDLE, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Arrow);
    // ⌘ is Direct Selection: its click selects the anchor and draws its handles, changing nothing.
    // Released, so the caret over the handle end comes from the shown handle itself, not from a
    // drag pinned to its press.
    click_at(&mut h, SQUARE_CORNER, SQUARE_MIDDLE, Modifiers::COMMAND);
    hover(&mut h, HANDLE_END, SQUARE_MIDDLE, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Caret);
    assert_eq!(h.state().ui.direct_selection.anchors, vec![[0, 0]], "the anchor is selected, its handles drawn");
}

#[test]
fn with_ctrl_held_it_is_the_black_arrow_everywhere() {
    for at in [SQUARE_CORNER, SQUARE_TOP, SQUARE_MIDDLE] {
        assert_eq!(on_path_one(at, Modifiers::COMMAND), Shown::Arrow, "{at:?}");
    }
}

#[test]
fn with_alt_held_over_an_anchor_it_is_the_caret_without_the_plus() {
    let (mut app, _) = setup(Tool::ConvertPoint);
    select_saved_path(&mut app);
    let mut h = harness(app, SQUARE_MIDDLE);
    let at = hover(&mut h, SQUARE_CORNER, SQUARE_MIDDLE, Modifiers::ALT);
    assert_eq!(shown(&h), Shown::Caret);
    assert!(!drawn_plus(&h, at));
}

#[test]
fn with_alt_held_over_a_segment_or_empty_canvas_it_is_the_arrow_with_the_plus() {
    for at in [SQUARE_TOP, SQUARE_MIDDLE] {
        let (mut app, _) = setup(Tool::ConvertPoint);
        select_saved_path(&mut app);
        let mut h = harness(app, SQUARE_MIDDLE);
        hover(&mut h, at, SQUARE_MIDDLE, Modifiers::ALT);
        assert_eq!(shown(&h), Shown::Arrow, "{at:?}");
        assert!(drawn_plus(&h, screen(&h, at, SQUARE_MIDDLE)), "{at:?}");
    }
}

#[test]
fn with_shift_held_over_an_anchor_it_is_the_caret() {
    assert_eq!(on_path_one(SQUARE_CORNER, Modifiers::SHIFT), Shown::Caret);
}

#[test]
fn a_drag_keeps_the_cursor_its_press_set() {
    // The anchor: the caret stays the caret while the pointer travels over the path's interior.
    let (mut app, _) = setup(Tool::ConvertPoint);
    select_saved_path(&mut app);
    let mut h = harness(app, SQUARE_MIDDLE);
    drag_from(&mut h, SQUARE_CORNER, SQUARE_MIDDLE, SQUARE_MIDDLE, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Caret, "the press was on an anchor");
    // A straight segment: the arrow it already showed stays the arrow through the drag.
    let (mut app, _) = setup(Tool::ConvertPoint);
    select_saved_path(&mut app);
    let mut h = harness(app, SQUARE_MIDDLE);
    hover(&mut h, SQUARE_TOP, SQUARE_MIDDLE, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Arrow, "before the press");
    drag_from(&mut h, SQUARE_TOP, SQUARE_MIDDLE, SQUARE_MIDDLE, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Arrow, "and during the drag");
}

#[test]
fn the_precise_other_cursors_crosshair_keeps_working() {
    for at in [SQUARE_CORNER, SQUARE_TOP] {
        let (mut app, _) = setup(Tool::ConvertPoint);
        select_saved_path(&mut app);
        app.run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
        let mut h = harness(app, SQUARE_MIDDLE);
        hover(&mut h, at, SQUARE_MIDDLE, Modifiers::NONE);
        assert_eq!(shown(&h), Shown::Crosshair, "{at:?}");
    }
}
