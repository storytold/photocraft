//! #13762: File › Export › Data Sets as Files has a Data Set dropdown at the top: All Data Sets, a
//! separator, then every data set by name in document order, opening on All Data Sets. One set
//! chosen, OK writes only that set's file; All Data Sets passes no filter.
//!
//! The document is a 120 × 80 "promo-card" with one visibility variable and the data sets
//! `Spring`, `Summer` and `Fall`.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const DATA_SETS: &str = "file.export.dataSetsAsFiles";
const ALL: &str = "All Data Sets";
const SETS: [&str; 3] = ["Spring", "Summer", "Fall"];

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80, "background": "white", "name": "promo-card"})).unwrap();
    let layer = app.session.active().unwrap().doc.layers[0].id;
    app.run("image.variables.define", json!({"defs": [{"name": "show", "layer": layer.0, "type": "visibility"}]})).unwrap();
    let sets: Vec<Value> =
        SETS.iter().enumerate().map(|(i, name)| json!({"name": name, "values": [{"variable": "show", "kind": "visibility", "value": i != 1}]})).collect();
    app.run("image.variables.dataSets", json!({"dataSets": sets})).unwrap();
    app.sync_views();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Opens Data Sets as Files from the menu, writing into `dir`.
fn open(h: &mut H, dir: &std::path::Path) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, DATA_SETS, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "dir", json!(dir.to_string_lossy())).unwrap();
    h.run_steps(2);
    dialog
}

/// What OK sends to the command.
fn params(h: &H, dialog: DialogId) -> Value {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog).unwrap();
    crate::filter_dialog::command_params(DATA_SETS, form.fields())
}

/// The Data Set dropdown: the one showing All Data Sets or a set's name.
fn dropdown<'a>(h: &'a H) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().is_some_and(|v| v == ALL || SETS.contains(&v.as_str()))).expect("the Data Set dropdown")
}

/// Opens the dropdown with a click; returns its options' labels, in order.
fn open_list(h: &mut H) -> Vec<String> {
    let at = dropdown(h).rect().center();
    crate::layers_panel_nodes::click(h, at, Modifiers::NONE);
    h.query_all_by_role(Role::ListBoxOption).filter_map(|n| n.accesskit_node().label().map(|l| l.to_string())).collect()
}

/// Picks `entry` in the dropdown, as a hand does.
fn pick(h: &mut H, entry: &str) {
    open_list(h);
    h.get_by_role_and_label(Role::ListBoxOption, entry).click();
    h.run_steps(2);
}

/// OK: the files written into `dir`, sorted.
fn run(h: &mut H, dialog: DialogId, dir: &std::path::Path) -> Vec<String> {
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    let mut files: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    files
}

fn no_filter(params: &Value) -> bool {
    params.get("dataSets").is_none_or(Value::is_null)
}

#[test]
fn the_dropdown_lists_all_data_sets_then_each_set_in_document_order() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let dialog = open(&mut h, dir.path());
    assert_eq!(dropdown(&h).value().as_deref(), Some(ALL), "it opens on All Data Sets");
    assert!(no_filter(&params(&h, dialog)), "All Data Sets passes no filter");
    assert_eq!(open_list(&mut h), [ALL, "Spring", "Summer", "Fall"]);
}

#[test]
fn one_set_chosen_writes_only_that_sets_file() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let dialog = open(&mut h, dir.path());
    pick(&mut h, "Summer");
    assert_eq!(dropdown(&h).value().as_deref(), Some("Summer"));
    assert_eq!(params(&h, dialog)["dataSets"], json!(["Summer"]));
    assert_eq!(run(&mut h, dialog, dir.path()), ["Summer.png"]);
}

#[test]
fn all_data_sets_again_writes_every_set() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let dialog = open(&mut h, dir.path());
    pick(&mut h, "Fall");
    pick(&mut h, ALL);
    assert!(no_filter(&params(&h, dialog)));
    assert_eq!(run(&mut h, dialog, dir.path()), ["Fall.png", "Spring.png", "Summer.png"]);
}

#[test]
fn agents_still_choose_sets_with_the_command() {
    let mut h = harness();
    let dir = tempfile::tempdir().unwrap();
    let r = h.state_mut().run(DATA_SETS, json!({"dir": dir.path().to_string_lossy(), "dataSets": ["Spring"], "format": "png"})).unwrap();
    assert_eq!(r["count"], 1);
}
