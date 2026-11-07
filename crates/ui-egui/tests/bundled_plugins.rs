//! `Services::bundled_plugins_only` holds below the menus (App Store guideline 2.5.2): a command
//! run any other way (a script's or an action's steps, a control client) can't install a plug-in
//! either. Its own test binary: the engine's policy is process-wide.

use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

#[test]
fn bundled_plugins_only_refuses_installs_that_bypass_the_menus() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");
    let mut app = PhotocraftApp::new(Session::new(), Services { bundled_plugins_only: true, ..Default::default() });
    let e = app.run("plugin.install", json!({"path": path})).unwrap_err();
    assert!(e.contains(photocraft_ui_egui::plugin_ui::BUNDLED_ONLY), "{e}");
    let listed = app.run("plugin.list", json!({})).unwrap();
    assert!(listed["plugins"].as_array().unwrap().is_empty(), "{listed}");
}
