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
    let mut h = Harness::builder().with_size(vec2(1800.0, 1000.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
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
        checkbox(&h, "Follow Depth").click();
        h.run_steps(3);
        assert!(h.state().session.tools.brush.surface.depth, "{tool:?}: Follow Depth");
        // Size by Depth starts on; Relief is a slider.
        assert!(h.state().session.tools.brush.surface.size_by_depth);
        checkbox(&h, "Size by Depth").click();
        h.run_steps(3);
        assert!(!h.state().session.tools.brush.surface.size_by_depth, "{tool:?}: Size by Depth");
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
    assert!(crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [120.0, 100.0]).is_none());
    app.run("perspective.setPlanes", json!({"planes": [[[70, 60], [170, 60], [220, 150], [20, 150]]]})).unwrap();
    let on = crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [120.0, 100.0]).expect("on the floor plane");
    assert!(on.radius > 0.0 && on.proj_scale > 0.0 && on.proj_scale <= 1.0, "{on:?}");
    assert!(crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [5.0, 5.0]).is_none(), "off the plane");
    // Follow Perspective off, or another tool: always plain.
    assert!(crate::brush_cursor::surface_shape(&mut app, Tool::Eraser, [120.0, 100.0]).is_none());
    app.run("tools.setBrush", json!({"brush": {"surface": {"perspective": false}}})).unwrap();
    assert!(crate::brush_cursor::surface_shape(&mut app, Tool::Brush, [120.0, 100.0]).is_none());
}

#[test]
fn ui_inspect_reports_the_cursor_shape() {
    use crate::control::{ControlRequest, Outcome, handle};
    let ctx = egui::Context::default();
    let mut app = app(Tool::Brush);
    let inspect = |app: &mut PhotocraftApp| {
        let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
        let Outcome::Done(v) = handle(app, &ctx, &req) else { panic!("ui.inspect replies at once") };
        v
    };
    let before = inspect(&mut app);
    assert_eq!(before["ok"], true, "{before}");
    assert!(before["result"]["brushCursor"].is_null());
    crate::brush_cursor::remember_shape(
        &ctx,
        Some(photocraft_engine::brush_cmds::CursorShape { radius: 12.0, proj_angle: 0.5, proj_scale: 0.4, offset: [1.0, 2.0], outline: None }),
    );
    let v = inspect(&mut app);
    assert_eq!(v["result"]["brushCursor"]["projScale"], json!(0.4f32));
    assert_eq!(v["result"]["brushCursor"]["radius"], json!(12.0f32));
    assert_eq!(v["result"]["brushCursor"]["offset"], json!([1.0f32, 2.0f32]));
    assert!(v["result"]["brushCursor"]["outline"].is_null());
}

#[test]
fn relief_and_size_by_depth_are_greyed_until_follow_depth_is_on() {
    let h = open_popover(app(Tool::Brush));
    assert!(checkbox(&h, "Size by Depth").accesskit_node().is_disabled());
    assert!(h.query_all_by_role(Role::Slider).all(|n| n.accesskit_node().is_disabled()));
}

#[test]
fn the_depth_options_reach_tools_set_brush() {
    let mut app = app(Tool::Brush);
    app.run("tools.setBrush", json!({"brush": {"surface": {"depth": true, "relief": 0.8, "sizeByDepth": false}}})).unwrap();
    let s = &app.session.tools.brush.surface;
    assert!(s.depth && !s.size_by_depth && (s.relief - 0.8).abs() < 1e-6);
}

/// A host over an empty temporary AI store: nothing is set up, and no test reads a person's settings.
#[cfg(not(target_arch = "wasm32"))]
fn empty_host(dir: &std::path::Path) -> photocraft_engine::depth_cmds::AiHost {
    use std::sync::Arc;
    let store = craft_ai_native::shared::SharedStore::new(dir.join("connections.json"));
    let shared = store.clone();
    photocraft_engine::depth_cmds::AiHost::new(store, move || {
        let app = craft_ai_core::AppId::new("photocraft").map_err(|e| e.to_string())?;
        Ok(craft_ai_client::AiClient::new(app, shared.clone(), Arc::new(craft_ai_native::secrets::MemoryStore::new())))
    })
}

/// Draw frames of the Estimate Depth window until `ui.inspect` reports `phase` (workers answer in between).
#[cfg(not(target_arch = "wasm32"))]
fn wait_for_phase(app: &mut PhotocraftApp, phase: &str) {
    let ctx = egui::Context::default();
    for _ in 0..400 {
        ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::depth_ui::show(app, ui.ctx())).textures_delta.clear();
        if crate::dialogs::depth_ui::inspect(app)["phase"] == phase {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the window never reached `{phase}`: {}", crate::dialogs::depth_ui::inspect(app));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn estimate_depth_with_an_unset_tool_opens_the_missing_model_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(Tool::Brush);
    app.session.depth.ai.set_host(empty_host(dir.path()));
    let reason = app.session.depth.ai.status_reason_now().expect("nothing is set up");
    assert_eq!(crate::dialogs::depth_ui::unavailable_reason(&app), Some(reason));
    crate::dialogs::depth_ui::start(&mut app);
    assert_eq!(crate::dialogs::depth_ui::inspect(&app)["phase"], "loading");
    wait_for_phase(&mut app, "missing");
    // Nothing was estimated, and closing leaves no window.
    assert!(app.session.active().unwrap().doc.channels.is_empty());
    crate::dialogs::depth_ui::cancel(&mut app);
    assert!(crate::dialogs::depth_ui::inspect(&app).is_null());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn estimate_depth_without_a_document_says_so() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    crate::dialogs::depth_ui::start(&mut app);
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert!(crate::dialogs::depth_ui::inspect(&app).is_null());
}
