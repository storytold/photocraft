//! A tool flyout marks the shown tool with a check and draws no mark before the other tools, as the reference
//! application's flyouts do; an empty ring before every other tool read as a list of radio buttons (#17102).

use super::*;
use egui::accesskit::{Role, Toggled};
use egui::{PointerButton, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

/// The whole app with a small document open and the Rectangle tool picked.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1100.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app.ui.tool = crate::state::Tool::Rectangle;
        app
    });
    h.run_steps(4);
    h
}

/// Right-clicks the shape tools' slot, which opens its flyout.
fn open_shape_flyout(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    // The slot's button, not its corner mark (a node of a few points) nor the options bar's tool button at the top.
    let slot = h.query_all_by_label("Rectangle Tool").rfind(|node| node.rect().width() >= 16.0);
    let at = slot.expect("the shape tools' slot is drawn").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(4);
}

#[test]
fn the_shape_flyout_checks_the_shown_tool_and_marks_no_other() {
    let mut h = harness();
    open_shape_flyout(&mut h);
    let rows: Vec<(String, Option<Toggled>, Rect)> = h
        .query_all_by_role(Role::MenuItemCheckBox)
        .map(|row| (row.accesskit_node().label().unwrap_or_default(), row.accesskit_node().toggled(), row.rect()))
        .collect();
    let checked: Vec<&str> = rows.iter().filter(|(_, on, _)| *on == Some(Toggled::True)).map(|(name, _, _)| name.as_str()).collect();
    assert_eq!(checked, ["Rectangle Tool"], "only the shown tool is checked: {rows:?}");
    assert!(rows.iter().any(|(name, _, _)| name == "Ellipse Tool"), "the flyout lists the other shape tools: {rows:?}");
    // What each row paints in its mark column, the room before its tool icon (the row's image): the shown tool's
    // check (a stroked path), nothing at all for the others. The accessible state alone would not catch a row that
    // paints a mark it does not report.
    let shapes = &h.output().shapes;
    for (name, _, row) in &rows {
        let inside: Vec<&egui::Shape> =
            shapes.iter().map(|clipped| &clipped.shape).filter(|shape| row.contains(shape.visual_bounding_rect().center())).collect();
        let icon = inside.iter().find(|shape| matches!(shape, egui::Shape::Mesh(_))).map(|shape| shape.visual_bounding_rect());
        let icon = icon.unwrap_or_else(|| panic!("the {name} row paints its tool icon"));
        let marks: Vec<&&egui::Shape> =
            inside.iter().filter(|shape| !matches!(shape, egui::Shape::Text(_)) && shape.visual_bounding_rect().max.x <= icon.min.x).collect();
        if name == "Rectangle Tool" {
            assert!(matches!(marks[..], [egui::Shape::Path(_)]), "the shown tool's row paints one check before its icon: {marks:?}");
        } else {
            assert!(marks.is_empty(), "the {name} row paints nothing before its icon: {marks:?}");
        }
    }
    // No empty ring (a stroked circle without fill) inside the flyout's rows.
    let flyout = rows.iter().fold(Rect::NOTHING, |all, (_, _, rect)| all.union(*rect));
    let rings = h
        .output()
        .shapes
        .iter()
        .filter(|clipped| match &clipped.shape {
            egui::Shape::Circle(circle) => circle.stroke.width > 0.0 && circle.fill == egui::Color32::TRANSPARENT && flyout.contains(circle.center),
            _ => false,
        })
        .count();
    assert_eq!(rings, 0, "no row of the flyout draws an empty ring");
}
