//! #13971: the Layers to Files dialog is titled "Export Layers To Files", labels its fields
//! Destination, File Name Prefix, Visible Layers Only and File Type, confirms with Run (Cancel
//! stays) and shows the reference's help text at the bottom. Its command and params, and the
//! other export dialogs, are unchanged.
use crate::PhotocraftApp;
use egui::accesskit::Role;
use egui::{Key, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const EXPORT: &str = "file.export.layersToFiles";
const HELP: &str = "Please specify the format and location for saving each layer as a file.";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 16, "height": 16, "name": "l2f-test"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp A"})).unwrap();
    app.sync_views();
    h.run_steps(2);
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, id: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    dialog
}

fn texts(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.get_all_by(|n| n.label().is_some() || n.value().is_some()).flat_map(|n| [n.accesskit_node().label(), n.accesskit_node().value()]).flatten().collect()
}

#[test]
fn the_title_labels_and_help_text_are_the_references() {
    let mut h = harness();
    let dialog = open(&mut h, EXPORT);
    let d = h.state().ui.dialogs.iter().find(|d| d.id == dialog).unwrap();
    assert_eq!(crate::dialogs::title(d), "Export Layers To Files");
    let texts = texts(&h);
    for want in ["Export Layers To Files", "File Type", HELP] {
        assert!(texts.iter().any(|t| t == want), "{want} missing from {texts:?}");
    }
    h.get_by_role_and_label(Role::TextInput, "Destination");
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "File Name Prefix").accesskit_node().value().as_deref(), Some("l2f-test"));
    h.get_by_role_and_label(Role::CheckBox, "Visible Layers Only");
    for old in ["Dir", "Prefix", "Visible only", "Format"] {
        assert!(!texts.iter().any(|t| t == old), "{old} is still shown");
    }
}

#[test]
fn run_confirms_and_cancel_stays() {
    let mut h = harness();
    let dialog = open(&mut h, EXPORT);
    h.get_by_role_and_label(Role::Button, "Run");
    h.get_by_role_and_label(Role::Button, "Cancel");
    assert!(h.query_by_role_and_label(Role::Button, "OK").is_none());
    // Run is the default button: Enter runs it (the unsaved document's empty Destination asks
    // for one, #13969).
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["message"], "Please specify destination.");
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == dialog));
}

#[test]
fn the_command_and_params_are_unchanged() {
    let mut h = harness();
    let dialog = open(&mut h, EXPORT);
    let d = h.state_mut().ui.dialog_mut(dialog).unwrap();
    assert_eq!(d.fields["__command"], EXPORT);
    let params = crate::dialogs::filter_dialog::params_of(&d.fields);
    let mut keys: Vec<&String> = params.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["dir", "format", "prefix", "quality", "trim", "visibleOnly"]);
}

#[test]
fn the_other_export_dialogs_are_unchanged() {
    let mut h = harness();
    for (id, title) in [("file.export.layerCompsToFiles", "Layer Comps to Files"), ("file.export.artboardsToFiles", "Artboards to Files")] {
        let dialog = open(&mut h, id);
        let d = h.state().ui.dialogs.iter().find(|d| d.id == dialog).unwrap();
        assert_eq!(crate::dialogs::title(d), title);
        assert!(!texts(&h).iter().any(|t| t == HELP), "{id}");
        h.get_by_role_and_label(Role::Button, "OK");
        crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
        h.run_steps(3);
    }
}
