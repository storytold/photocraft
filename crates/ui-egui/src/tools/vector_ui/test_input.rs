//! The headless app the vector tool tests share. Tool input goes through `crate::canvas::testing`, which runs the
//! dispatcher's tool; this file has no driver of its own.

use egui::vec2;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const WIDTH: f32 = 1280.0;

/// A harness on a 200 × 100 document with `tool` selected.
pub(super) fn harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = Harness::builder().with_size(vec2(WIDTH, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(6);
    h.state_mut().ui.tool = tool;
    h.run_steps(4);
    h
}
