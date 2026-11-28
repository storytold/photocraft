//! A separate process keeps the alternate Skin Smooth manifest out of the shared unit-test registry.
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{PhotocraftApp, Services, control, menus};
use serde_json::{Value, json};

fn fields(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    control::inspect(app, ctx)["dialogs"].as_array().unwrap().last().unwrap()["fields"].clone()
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, dialog: &Value, fields: Value) {
    for (field, value) in fields.as_object().unwrap() {
        let (req, _) = control::ControlRequest::new("ui.dialog.set", json!({"dialog":dialog,"field":field,"value":value}));
        let control::Reply::Done(reply) = control::handle(app, ctx, &req) else { panic!("missing reply for {field}") };
        assert!(reply["error"].is_null(), "{field}: {reply}");
    }
}

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
    let ctx = egui::Context::default();
    let dialog = menus::invoke(&mut app, &ctx, "plugin.filter.local.skinsmooth.skin-smooth", json!({})).unwrap()["dialog"].clone();
    let initial = fields(&app, &ctx);
    assert_eq!(initial["fine"], 13.0);
    assert_eq!(initial["skinOnly"], true);
    let positive = json!([[0.7, 0.5, 0.4]]);
    let negative = json!([[0.2, 0.3, 0.5]]);
    set(&mut app, &ctx, &dialog, json!({"positiveColors":positive,"negativeColors":negative}));
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 1000.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
        .build_ui_state(|ui, app| photocraft_ui_egui::dialogs::run(app, ui), app);
    h.run_steps(6);
    assert_eq!(fields(h.state(), &h.ctx)["__presetDirty"], false, "samples are not preset edits");
    let ctx = h.ctx.clone();
    set(h.state_mut(), &ctx, &dialog, json!({"fine":12}));
    h.run_steps(3);
    h.get_by_label("Reset preset").click();
    h.run_steps(3);
    assert_eq!(fields(h.state(), &h.ctx)["fine"], 13.0);
    for name in ["Natural", "Alternate"] {
        h.query_all_by_label("Preset").last().unwrap().click();
        h.run_steps(3);
        h.get_by_label(name).click();
        h.run_steps(3);
        let fields = &fields(h.state(), &h.ctx);
        assert_eq!(fields["positiveColors"], positive);
        assert_eq!(fields["negativeColors"], negative);
        assert_eq!(fields["skinOnly"], true);
        assert_eq!(fields["__presetDirty"], false);
    }
}
