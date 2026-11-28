//! Polygonal Lasso through the real canvas: a Cmd-drag inside a selection floats it, a held temporary tool
//! keeps the outline, and every other tool or document drops it without recording a step.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::PolygonLasso;
    // 60 fps, so a double-click's two clicks fall inside egui's 0.3 s window.
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::testing::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    // 100 %: one document pixel per point.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

/// Where the canvas shows document point `(x, y)`.
fn screen(h: &Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) -> Pos2 {
    crate::canvas::testing::to_screen(h.state(), [f64::from(x), f64::from(y)])
}

fn button(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, down: bool, m: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: m });
    h.run_steps(1);
}

/// A click at document point `(x, y)`, the pointer still between press and release.
fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, m);
    button(h, p, false, m);
}

fn polygon_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::PolygonLasso;
    app
}

fn ev(app: &mut PhotocraftApp, kind: &str, x: f64, y: f64, m: Modifiers) {
    use crate::canvas::{ToolEvent, tool_event};
    let e = match kind {
        "down" => ToolEvent::Down { x, y, pressure: 1.0 },
        "up" => ToolEvent::Up { x, y },
        _ => ToolEvent::Move { x, y, pressure: 1.0 },
    };
    tool_event(app, e, m);
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn bounds(app: &PhotocraftApp) -> Option<Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds())
}

/// The tool or document changed: the next pass of the canvas.
fn switched(app: &mut PhotocraftApp) {
    crate::canvas::testing::frame(app);
}

/// ⌘-drag inside a selection with the Polygonal Lasso cuts and floats the pixels, as with the
/// marquee; a plain press there still places a point.
#[test]
fn cmd_drag_inside_the_selection_floats_it_with_the_polygonal_lasso() {
    let mut app = polygon_app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("paint", |doc, _| {
            doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    ev(&mut app, "down", 20.0, 20.0, Modifiers::COMMAND);
    ev(&mut app, "move", 40.0, 20.0, Modifiers::COMMAND);
    ev(&mut app, "up", 40.0, 20.0, Modifiers::COMMAND);
    let f = photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset);
    assert_eq!(f, Some((20, 0)));
    assert!(!app.canvas_tools.selection.outline_open(), "no polygon point was placed");
    app.run("select.drop", json!({})).unwrap();
    // A plain press inside the selection starts a polygon.
    ev(&mut app, "down", 35.0, 20.0, Modifiers::NONE);
    ev(&mut app, "up", 35.0, 20.0, Modifiers::NONE);
    assert!(app.canvas_tools.selection.outline_open());
}

/// An open outline hides the selection it would replace. Every other tool, and another document, drops it with no
/// history step and the old ants come back; the selection is untouched, and coming back starts a new outline.
#[test]
fn polygon_outline_lifecycle() {
    let tools = [
        Tool::QuickSelection,
        Tool::ObjectSelection,
        Tool::MagicWand,
        Tool::RectMarquee,
        Tool::EllipseMarquee,
        Tool::Lasso,
        Tool::MagneticLasso,
        Tool::Move,
        Tool::Brush,
    ];
    for tool in tools {
        let mut app = polygon_app();
        app.run("select.rect", json!({"x": 200, "y": 150, "width": 100, "height": 100})).unwrap();
        let before = steps(&app);
        for (x, y) in [(50.0, 50.0), (150.0, 50.0)] {
            ev(&mut app, "down", x, y, Modifiers::NONE);
            ev(&mut app, "up", x, y, Modifiers::NONE);
        }
        assert!(app.canvas_tools.selection.outline_open(), "{tool:?}: two vertices are open");
        assert!(app.canvas_tools.selection.replaces_selection(), "{tool:?}: the old outline is hidden while drawing");
        assert_eq!(steps(&app), before, "{tool:?}: drawing records nothing");
        app.ui.tool = tool;
        switched(&mut app);
        assert!(!app.canvas_tools.selection.outline_open(), "{tool:?}: the outline is gone");
        assert!(!app.canvas_tools.selection.replaces_selection(), "{tool:?}: the old outline is shown again");
        assert_eq!(bounds(&app), Some(Rect::new(200, 150, 300, 250)), "{tool:?}");
        assert_eq!(steps(&app), before, "{tool:?}: nothing was recorded");
        app.ui.tool = Tool::PolygonLasso;
        switched(&mut app);
        assert!(!app.canvas_tools.selection.outline_open(), "{tool:?}: coming back starts a new outline");
    }

    // Another document becoming active drops it too, while the tool stays.
    let mut app = polygon_app();
    for (x, y) in [(50.0, 50.0), (150.0, 50.0)] {
        ev(&mut app, "down", x, y, Modifiers::NONE);
        ev(&mut app, "up", x, y, Modifiers::NONE);
    }
    assert!(app.canvas_tools.selection.outline_open());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    switched(&mut app);
    assert!(!app.canvas_tools.selection.outline_open(), "the outline belongs to the document it was drawn on");
    assert!(!app.canvas_tools.selection.replaces_selection());
    assert_eq!(app.ui.tool, Tool::PolygonLasso, "the tool stays");
}

/// Two vertices placed with the Polygonal Lasso, well apart in time so they aren't a double-click.
fn two_vertices(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    click(h, 50.0, 50.0, Modifiers::NONE);
    h.run_steps(30);
    click(h, 150.0, 50.0, Modifiers::NONE);
    h.run_steps(30);
    assert!(h.state().canvas_tools.selection.outline_open());
}

/// A held temporary tool (Space is the Hand, ⌘ the Move tool) doesn't change the selected tool, so
/// the polygon being drawn stays.
#[test]
fn a_held_temporary_tool_keeps_the_polygon() {
    let mut h = harness();
    two_vertices(&mut h);
    h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
    assert!(h.state().canvas_tools.selection.outline_open(), "Space held");
    h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    h.event(egui::Event::ModifiersChanged(Modifiers::COMMAND));
    h.run_steps(3);
    assert_eq!(h.state().ui.tool, Tool::PolygonLasso);
    assert!(h.state().canvas_tools.selection.outline_open(), "⌘ held");
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
    assert!(h.state().canvas_tools.selection.outline_open(), "released");
}
