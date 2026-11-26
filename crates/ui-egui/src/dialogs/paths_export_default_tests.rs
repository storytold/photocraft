//! #13777: File › Export › Paths to Illustrator opens its Paths dropdown on the row that is
//! selected in the Paths panel, as the reference application does: a saved path or the Work
//! Path when that row is selected, All Paths when no row is (or the row is gone), and the
//! choice is not remembered between openings. The shape layer's path is not an entry yet
//! (#13774), so that row keeps today's All Paths.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, ParamKind};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "file.export.pathsToIllustrator";

/// A document with a Work Path and two saved paths, "Curve A" and "Curve B".
fn app_with_paths() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let path = |x: f64| json!({"subpaths": [{"closed": true, "knots": [[x, 10.0], [x + 40.0, 10.0], [x + 40.0, 50.0]]}]});
    for (name, x) in [("Curve A", 10.0), ("Curve B", 60.0), ("work", 110.0)] {
        app.run("path.set", json!({"name": name, "path": path(x)})).unwrap();
    }
    app
}

/// Opens the dialog as the menu item does and returns its id.
fn open_dialog(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    let r = super::file_ui::invoke(app, &ctx, COMMAND, &json!({})).unwrap().unwrap();
    serde_json::from_value(r["dialog"].clone()).expect("the menu opened the dialog")
}

/// The entry the open dialog `id` shows in its Paths dropdown.
fn preselected(app: &PhotocraftApp, id: DialogId) -> Value {
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the hosted form");
    dialog.fields().text("paths").map_or(Value::Null, |p| json!(p))
}

/// Opens the dialog, returns its preselected entry and cancels it.
fn open(app: &mut PhotocraftApp) -> Value {
    let id = open_dialog(app);
    let paths = preselected(app, id);
    let ctx = app.ctx().clone();
    assert!(app.framework.dialogs.cancel_by_id(&ctx, id));
    paths
}

/// The Paths choice's option ids, in order.
fn choices(app: &PhotocraftApp) -> Value {
    let rows = super::file_forms::paths_to_file_rows(app);
    let Some(ParamKind::Choice { options, .. }) = rows.iter().find(|p| p.key == "paths").map(|p| &p.kind) else { panic!("a Paths choice") };
    json!(options.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>())
}

#[test]
fn a_selected_saved_path_is_preselected() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("Curve B".into());
    assert_eq!(open(&mut app), "Curve B");
}

#[test]
fn the_selected_work_path_is_preselected() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("work".into());
    assert_eq!(open(&mut app), "work");
}

#[test]
fn no_selected_row_preselects_all_paths() {
    let mut app = app_with_paths();
    app.ui.selected_path = None;
    assert_eq!(open(&mut app), "all");
}

#[test]
fn a_row_the_document_no_longer_has_preselects_all_paths() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("Deleted path".into());
    assert_eq!(open(&mut app), "all");
    app.run("path.delete", json!({"name": "work"})).unwrap();
    app.ui.selected_path = Some("work".into());
    assert_eq!(open(&mut app), "all", "a selected Work Path the document no longer has");
}

/// The layer path entry does not exist yet (#13774): that row keeps today's default.
#[test]
fn the_shape_layer_path_row_keeps_all_paths() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("layer".into());
    assert_eq!(open(&mut app), "all");
}

#[test]
fn the_choice_is_not_remembered_between_openings() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("Curve A".into());
    let id = open_dialog(&mut app);
    assert_eq!(preselected(&app, id), "Curve A");
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "paths", json!("work")).unwrap();
    assert!(app.framework.dialogs.cancel_by_id(&ctx, id));
    app.ui.selected_path = Some("Curve B".into());
    assert_eq!(open(&mut app), "Curve B");
}

/// The dropdown's entries and the command's parameter values stay as they are.
#[test]
fn the_entries_keep_their_parameter_values() {
    let mut app = app_with_paths();
    open(&mut app);
    let choices = choices(&app);
    // Document Bounds comes first (#13771).
    assert_eq!(choices, json!(["documentBounds", "all", "work", "Curve A", "Curve B"]));
}
