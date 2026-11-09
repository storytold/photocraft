//! A separate process keeps the alternate Skin Smooth manifest out of the shared unit-test registry.
use photocraft_ui_egui::{PhotocraftApp, Services, menus};
use serde_json::{Value, json};

#[test]
fn skin_only_is_independent_of_presets_without_the_specialized_ui() {
    const ID: &str = "local.skinsmooth.skin-smooth";
    let mut bytes = include_bytes!("../../plugins/tests/fixtures/skin-smooth.wasm").to_vec();
    let start = bytes.windows(8).position(|w| w == b"{\n  \"id\"").unwrap();
    let mut stream = serde_json::Deserializer::from_slice(&bytes[start..]).into_iter::<Value>();
    let mut manifest = stream.next().unwrap().unwrap();
    let length = stream.byte_offset();
    manifest["accelerator"] = json!("");
    let encoded = serde_json::to_vec(&manifest).unwrap();
    assert!(encoded.len() <= length);
    bytes[start..start + length].fill(b' ');
    bytes[start..start + encoded.len()].copy_from_slice(&encoded);
    photocraft_plugins::registry::install_bytes(&bytes).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    app.run("plugin.preset.save", json!({"id":ID,"name":"Alternate","params":{"skinOnly":false,"fine":13}})).unwrap();
    app.session.prefs.edit(|p| p.dialogs.insert(format!("plugin.choices.{ID}"), json!({"__pluginPreset":"saved:Alternate","skinOnly":true})));
    menus::invoke(&mut app, &egui::Context::default(), "plugin.filter.local.skinsmooth.skin-smooth", json!({})).unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["fine"], 13.0);
    assert_eq!(fields["skinOnly"], true);
}
