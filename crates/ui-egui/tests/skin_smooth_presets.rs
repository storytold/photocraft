//! A separate process keeps the alternate Skin Smooth manifest out of the shared unit-test registry.
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{PhotocraftApp, Services, menus};
use serde_json::{Value, json};

#[test]
fn skin_only_and_samples_are_independent_of_presets_without_the_specialized_ui() {
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
    let positive = json!([[0.7, 0.5, 0.4]]);
    let negative = json!([[0.2, 0.3, 0.5]]);
    app.ui.dialogs[0].fields.insert("positiveColors".into(), positive.clone());
    app.ui.dialogs[0].fields.insert("negativeColors".into(), negative.clone());
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 1000.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
        .build_ui_state(|ui, app| photocraft_ui_egui::dialogs::show(app, ui.ctx()), app);
    h.run_steps(6);
    assert_eq!(h.state().ui.dialogs[0].fields["__presetDirty"], false, "samples are not preset edits");
    h.state_mut().ui.dialogs[0].fields.insert("fine".into(), json!(12));
    h.run_steps(3);
    h.get_by_label("Reset preset").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs[0].fields["fine"], 13.0);
    for name in ["Natural", "Alternate"] {
        h.query_all_by_label("Preset").last().unwrap().click();
        h.run_steps(3);
        h.get_by_label(name).click();
        h.run_steps(3);
        let fields = &h.state().ui.dialogs[0].fields;
        assert_eq!(fields["positiveColors"], positive);
        assert_eq!(fields["negativeColors"], negative);
        assert_eq!(fields["skinOnly"], true);
        assert_eq!(fields["__presetDirty"], false);
    }
}
