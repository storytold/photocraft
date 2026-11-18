//! #7942 / #7531: path fields of command dialogs get a Browse… button (Artboards to Files'
//! folder, Install Plug-in…'s .wasm file) that fills the field from the file service.
use crate::PhotocraftApp;
use craft::host::{PickKind, Picked, ScriptedFiles};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};
use std::sync::Arc;

fn harness(files: Arc<ScriptedFiles>) -> Harness<'static, PhotocraftApp> {
    // The framework builds its host's file service from the services the app is made with.
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { files: Some(files), ..Default::default() });
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

fn field(h: &Harness<'_, PhotocraftApp>, dialog: DialogId, key: &str) -> Option<Value> {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog)?;
    serde_json::to_value(form.fields()).ok()?.get(key).cloned()
}

fn open(h: &mut Harness<'_, PhotocraftApp>, command: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    // PLAN: assumed `app.framework.dialogs: DialogHost` and `OpenDialog { id, .. }`
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    dialog
}

/// A folder row asks for a folder and a file row for a `.wasm` file; each answer fills its own row.
#[test]
fn browse_asks_the_file_service_and_fills_the_field() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["/tmp/exports".into()]), Picked::Paths(vec!["/tmp/blur.wasm".into()])]));
    let mut h = harness(files.clone());
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    h.state_mut().sync_views();

    let folder = open(&mut h, "file.export.artboardsToFiles");
    assert!(field(&h, folder, "dir").is_some(), "the dialog has a Dir row");
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(field(&h, folder, "dir"), Some(json!("/tmp/exports")));
    let ctx = h.ctx.clone();
    crate::dialogs::cancel(h.state_mut(), &ctx, folder).unwrap();

    let plugin = open(&mut h, "plugin.install");
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(field(&h, plugin, "path"), Some(json!("/tmp/blur.wasm")));

    let asked = files.requests();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].kind, PickKind::Folder);
    assert_eq!(asked[1].kind, PickKind::Open);
    assert_eq!(asked[1].filters.iter().flat_map(|f| f.extensions.clone()).collect::<Vec<_>>(), ["wasm"]);
}
