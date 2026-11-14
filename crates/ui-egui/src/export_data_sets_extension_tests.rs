//! #13754: Data Sets as Files writes layered PSDs, so its dialog offers only the case of the
//! extension (`.psd` by default, or `.PSD`) and no format.
use crate::PhotocraftApp;
use egui::vec2;
use egui_kittest::Harness;
use serde_json::{Value, json};

fn field(h: &Harness<'_, PhotocraftApp>, key: &str) -> Option<Value> {
    h.state().ui.dialogs.last().and_then(|d| d.fields.get(key).cloned())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_dialog_has_a_file_extension_defaulting_to_psd_and_no_format() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    h.state_mut().run("image.variables.dataSets", json!({"dataSets": [{"name": "Set 1", "values": []}]})).unwrap();
    h.state_mut().sync_views();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.export.dataSetsAsFiles", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(field(&h, "extension"), Some(json!(".psd")), "the extension defaults to .psd");
    assert_eq!(field(&h, "format"), None, "there is no format choice");
    assert!(field(&h, "dir").is_some() && field(&h, "naming").is_some());
}
