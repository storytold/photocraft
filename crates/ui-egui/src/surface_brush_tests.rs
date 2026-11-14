//! The Brush's Surface options: the popover's toggles reach `tools.setBrush`, and the brush cursor
//! takes the dab's shape from the stored perspective planes (Follow Perspective) or stays plain.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 240, "height": 160})).unwrap();
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app
}

fn open_popover(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.get_by_label("Surface options").click();
    h.run_steps(3);
    h
}

fn checkbox<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::CheckBox).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("{label}"))
}

#[test]
fn the_popover_toggles_reach_the_brush_settings() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut h = open_popover(app(tool));
        assert!(!h.state().session.tools.brush.surface.color);
        checkbox(&h, "Surface Color").click();
        h.run_steps(3);
        assert!(h.state().session.tools.brush.surface.color, "{tool:?}: Surface Color");
        checkbox(&h, "Follow Perspective").click();
        h.run_steps(3);
        assert!(h.state().session.tools.brush.surface.perspective, "{tool:?}: Follow Perspective");
    }
}

#[test]
fn strength_detail_and_sample_are_greyed_until_surface_color_is_on() {
    let h = open_popover(app(Tool::Brush));
    assert!(h.query_all_by_role(Role::Slider).all(|n| n.accesskit_node().is_disabled()));
}

#[test]
fn the_cursor_follows_a_plane_and_stays_round_off_it() {
    let mut app = app(Tool::Brush);
    app.run("tools.setBrush", json!({"brush": {"surface": {"perspective": true}}})).unwrap();
    // Off every plane (none stored yet): the plain outline.
    assert_eq!(crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [120.0, 100.0]), None);
    app.run("perspective.setPlanes", json!({"planes": [[[70, 60], [170, 60], [220, 150], [20, 150]]]})).unwrap();
    let on = crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [120.0, 100.0]).expect("on the floor plane");
    assert!(on.0 > 0.0 && on.2 > 0.0 && on.2 <= 1.0, "{on:?}");
    assert_eq!(crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [5.0, 5.0]), None, "off the plane");
    // Follow Perspective off, or another tool: always plain.
    assert_eq!(crate::brush_cursor::surface_shape(&mut app, Tool::Eraser, [120.0, 100.0]), None);
    app.run("tools.setBrush", json!({"brush": {"surface": {"perspective": false}}})).unwrap();
    assert_eq!(crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [120.0, 100.0]), None);
}

#[test]
fn ui_inspect_reports_the_cursor_shape() {
    use crate::control::{ControlRequest, Outcome, handle};
    let ctx = egui::Context::default();
    let mut app = app(Tool::Brush);
    let mut inspect = |app: &mut PhotocraftApp| {
        let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
        let Outcome::Done(v) = handle(app, &ctx, &req) else { panic!("ui.inspect replies at once") };
        v
    };
    assert!(inspect(&mut app)["brushCursor"].is_null());
    crate::brush_cursor::remember_shape(&ctx, Some((12.0, 0.5, 0.4)));
    let v = inspect(&mut app);
    assert_eq!(v["brushCursor"]["projScale"], json!(0.4f32));
    assert_eq!(v["brushCursor"]["radius"], json!(12.0f32));
}
