//! #13754: Data Sets as Files writes layered PSDs, so its dialog offers only the case of the
//! extension (`.psd` by default, or `.PSD`) and no format.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::Harness;
use serde_json::{Value, json};

fn field(h: &Harness<'_, Box<PhotocraftApp>>, dialog: DialogId, key: &str) -> Option<Value> {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog)?;
    serde_json::to_value(form.fields()).ok()?.get(key).cloned()
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_dialog_has_a_file_extension_defaulting_to_psd_and_no_format() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    h.state_mut().run("image.variables.dataSets", json!({"dataSets": [{"name": "Set 1", "values": []}]})).unwrap();
    h.state_mut().sync_views();
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, "file.export.dataSetsAsFiles", json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    assert_eq!(field(&h, dialog, "extension"), Some(json!(".psd")), "the extension defaults to .psd");
    assert_eq!(field(&h, dialog, "format"), None, "there is no format choice");
    assert!(field(&h, dialog, "dir").is_some() && field(&h, dialog, "naming").is_some());
}
