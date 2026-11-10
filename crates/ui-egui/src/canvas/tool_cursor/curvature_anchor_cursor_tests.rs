//! #12759: the Curvature Pen over an anchor of the path being drawn or the active path
//! shows Photoshop's arrow with a small dotted circle beside it — not the pen with the
//! four-arrow move badge. The badge stays off the anchor, except the first anchor while
//! drawing, which this item leaves alone (#12750 owns its close cursor from 2 anchors
//! on) and keeps today's move badge; the segment keeps its `+`, and a press drag keeps
//! the move badge it started with.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::tool_feedback::{Badge, badge};

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A Pen-drawn work path: a closed square of corners (40, 40)–(160, 160).
fn with_corner_square(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [[40, 40], [160, 40], [160, 160], [40, 160]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    drag(app, at, at, mods);
}

#[test]
fn an_anchor_hover_shows_no_badge_while_a_segment_keeps_its_add() {
    let mut app = app(Tool::CurvaturePen);
    with_corner_square(&mut app);
    for (at, want) in [([160.0, 40.0], None), ([100.0, 40.0], Some(Badge::Add)), ([100.0, 100.0], None)] {
        app.hover_doc = Some(at);
        assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), want, "at {at:?}");
    }
    // ⌘/Ctrl is Direct Selection: no Curvature Pen badge.
    app.hover_doc = Some([160.0, 40.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::COMMAND), None);
}

#[test]
fn drawn_anchors_hover_shows_no_badge() {
    let mut app = app(Tool::CurvaturePen);
    click(&mut app, [20.0, 180.0], Modifiers::NONE);
    click(&mut app, [60.0, 190.0], Modifiers::NONE);
    // The second anchor: a press would drag it, yet no badge — the arrow shows that.
    app.hover_doc = Some([60.0, 190.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), None);
    // The active path's segments stay untouched while drawing.
    app.hover_doc = Some([100.0, 40.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), None);
}

#[test]
fn the_first_anchor_while_drawing_keeps_todays_move_badge() {
    // #12759 leaves the first anchor while drawing alone (#12750 owns its close cursor
    // from 2 anchors on): at 2 knots there is no close cursor yet, so it keeps today's
    // four arrows instead of the arrow.
    let mut app = app(Tool::CurvaturePen);
    click(&mut app, [20.0, 180.0], Modifiers::NONE);
    click(&mut app, [60.0, 190.0], Modifiers::NONE);
    app.hover_doc = Some([20.0, 180.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), Some(Badge::Move));
}

#[test]
fn dragging_a_drawn_anchor_keeps_the_move_badge() {
    let mut app = app(Tool::CurvaturePen);
    click(&mut app, [20.0, 180.0], Modifiers::NONE);
    click(&mut app, [60.0, 190.0], Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 190.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 200.0, pressure: 1.0 }, Modifiers::NONE);
    // The knot follows the pointer, so the hover still hits it — and the drag keeps the
    // move badge it started with, as a press drags the anchor.
    app.hover_doc = Some([70.0, 200.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), Some(Badge::Move));
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 200.0 }, Modifiers::NONE);
}

/// The real canvas: hovering an anchor shows the arrow, while a segment and empty canvas
/// keep the pen, and Caps Lock keeps the crosshair. In this harness the pen bitmap cannot be
/// uploaded, so the pen shows as the `CursorLifecycle` visible fallback (a crosshair) instead
/// of the pen bitmap a desktop backend shows; the arrow is still an OS arrow.
#[test]
fn hovering_an_anchor_shows_the_arrow_on_the_canvas() {
    use egui::CursorIcon;
    let mut app = app(Tool::CurvaturePen);
    with_corner_square(&mut app);
    app.ui.extras.rulers = false;
    let mut h = canvas_harness(app);
    // The segment and the empty canvas keep the pen: here that is the crosshair fallback
    // (see the doc comment), not the pen bitmap of a desktop backend.
    for (at, want) in [([160.0, 40.0], CursorIcon::Default), ([100.0, 40.0], CursorIcon::Crosshair), ([100.0, 100.0], CursorIcon::Crosshair)] {
        let p = screen(h.state(), at[0], at[1]);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        assert_eq!(h.output().platform_output.cursor_icon, want, "at {at:?}");
    }
    // Caps Lock forces the precise crosshair, whatever the hover (#12759 keeps it).
    h.state_mut().caps_lock = true;
    let p = screen(h.state(), 160.0, 40.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Crosshair, "caps lock over an anchor");
}

/// #12759 leaves the first anchor while drawing alone (#12750 owns its close cursor from
/// 2 anchors on): at 2 knots it keeps today's pen — the crosshair fallback here — while
/// the second anchor gets the arrow.
#[test]
fn drawing_first_anchor_keeps_the_pen_cursor() {
    use egui::CursorIcon;
    let mut app = app(Tool::CurvaturePen);
    click(&mut app, [20.0, 180.0], Modifiers::NONE);
    click(&mut app, [60.0, 190.0], Modifiers::NONE);
    app.ui.extras.rulers = false;
    let mut h = canvas_harness(app);
    for (at, want) in [([20.0, 180.0], CursorIcon::Crosshair), ([60.0, 190.0], CursorIcon::Default)] {
        let p = screen(h.state(), at[0], at[1]);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        assert_eq!(h.output().platform_output.cursor_icon, want, "at {at:?}");
    }
}

fn canvas_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
    use egui::vec2;
    use egui_kittest::Harness;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 100.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(app: &PhotocraftApp, x: f64, y: f64) -> egui::Pos2 {
    let v = &app.ui.views[0];
    let xf = crate::canvas::ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: false,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    xf.to_screen(x as f32, y as f32)
}
