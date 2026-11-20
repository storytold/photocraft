//! #13981: the batch Lens Correction dialog shows its controls in the reference application's four
//! sections with the reference captions and the Edge entries in the reference order.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, ParamKind};
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;

pub(super) type H = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The open dialogs drawn over `app`, as the window shows them.
pub(super) fn harness(app: Box<PhotocraftApp>) -> H {
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 1000.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::run(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Opens the dialog from the File menu over `app`.
pub(super) fn open_in(mut app: Box<PhotocraftApp>) -> (H, DialogId) {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, super::COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    (harness(app), id)
}

/// The dialog open over an app without a document (the command needs none).
pub(super) fn opened() -> (H, DialogId) {
    open_in(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()))
}

pub(super) fn field(h: &H, id: DialogId, key: &str) -> Value {
    h.state()
        .framework
        .dialogs
        .get_by_id::<HostedParamDialog>(id)
        .and_then(|d| d.fields().get(key).and_then(|v| serde_json::to_value(v).ok()))
        .unwrap_or(Value::Null)
}

pub(super) fn set(h: &mut H, id: DialogId, key: &str, value: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(3);
}

/// The params OK sends.
pub(super) fn params(h: &H, id: DialogId) -> Value {
    let fields = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
    crate::filter_dialog::command_params(super::COMMAND, &fields)
}

/// A fresh folder holding one source, `a.png`.
pub(super) fn source_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut image = photocraft_engine::Session::new();
    image.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    let png = photocraft_io::export(&image.active().unwrap().doc, "a.png", &Default::default()).unwrap().bytes;
    std::fs::write(dir.path().join("a.png"), png).unwrap();
    dir
}

/// The options of the list row `key`, as their labels.
fn labels(key: &str) -> Vec<String> {
    let rows = super::rows(".".into());
    let row = rows.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("a {key} row"));
    let ParamKind::Choice { options, .. } = &row.kind else { panic!("{key} is a list") };
    options.iter().map(|(_, label)| label.as_str().to_owned()).collect()
}

#[test]
fn the_four_sections_and_their_rows_follow_the_reference_order() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (h, _) = opened();
        let top = |label: &str| h.query_all_by_label(label).map(|n| n.rect().top()).reduce(f32::min).unwrap_or_else(|| panic!("no {label}"));
        let order = [
            "Source Files",
            "Input",
            "Destination Folder",
            "File Type",
            "Output",
            "Lens Correction Profile",
            "Profile",
            "Correction Options",
            "Geometric Distortion",
            "Chromatic Aberration",
            "Vignette",
            "Auto Scale Image",
            "Edge",
        ];
        for pair in order.windows(2) {
            assert!(top(pair[0]) < top(pair[1]), "order: {pair:?}");
        }
        for generated in ["Auto scale", "Correct c a", "Correct distortion", "Correct vignette"] {
            assert!(h.query_by_label(generated).is_none(), "the generated label {generated} is gone");
        }
    });
}

#[test]
fn edge_lists_the_reference_entries_in_order() {
    assert_eq!(labels("edge"), ["Edge Extension", "Transparency", "Black Color", "White Color"]);
    assert_eq!(labels("profile"), ["Auto", "Generic", "None"]);
}

/// OK sends the params the form sent before the layout: the same keys.
#[test]
fn ok_runs_the_command_with_the_same_params() {
    let (mut h, id) = opened();
    assert_eq!(
        params(&h, id),
        json!({"input": ".", "output": "./corrected", "format": "psd", "profile": "auto", "correctDistortion": true, "correctVignette": false, "correctCA": false, "autoScale": true, "edge": "transparency"})
    );
    let sources = source_folder();
    let results = tempfile::tempdir().unwrap();
    set(&mut h, id, "input", json!(sources.path().to_string_lossy()));
    set(&mut h, id, "output", json!(results.path().to_string_lossy()));
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
    h.run_steps(4);
    assert!(h.state().framework.dialogs.is_empty(), "OK closes the dialog");
    assert!(std::fs::read_dir(results.path()).unwrap().next().is_some(), "OK runs the batch into the destination folder");
}
