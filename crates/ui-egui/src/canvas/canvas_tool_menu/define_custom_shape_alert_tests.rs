//! #14291: OK in Define Custom Shape with an empty or spaces-only name shows the alert "You must specify a name for
//! the shape." titled "Shape Name"; the dialog stays open and nothing is stored. A name with spaces around it is
//! stored as typed. Both openers (the Pen's canvas menu and the Edit menu) share the dialog key, so both are checked.

use craft::ui::dialog::{HostedAlert, StackDialog};
use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::super::tests::{choose_open, dialog_open};
use crate::PhotocraftApp;
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let knots = json!([[100, 100], [300, 100], [200, 250]]);
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": knots}]}})).unwrap();
    app.ui.tool = Tool::Pen;
    app
}

fn draw(ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>) {
    crate::dialogs::run(app, ui);
}

fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_context(ctx).build_ui_state(draw, app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Sets the dialog's `key` row to `text`.
fn set(h: &mut Harness<'static, Box<PhotocraftApp>>, key: &str, text: &str) {
    let app = h.state_mut();
    let id = app.framework.dialogs.stack().ids().find(|id| app.framework.dialogs.get_by_id::<HostedParamDialog>(*id).is_some()).unwrap();
    app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id).unwrap().set_field(key, FieldValue::Text(text.into()));
    h.run_steps(2);
}

/// Types `name` into the dialog and presses OK.
fn ok_with(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str) {
    set(h, "name", name);
    h.get_by_label("OK").click();
    h.run_steps(6);
}

fn alert_message(app: &PhotocraftApp) -> Option<String> {
    let fields = app.framework.dialogs.get::<HostedAlert>()?.json_fields()?;
    fields.get("message").and_then(Value::as_str).map(str::to_owned)
}

fn stored(app: &PhotocraftApp) -> Vec<String> {
    photocraft_engine::edit_menu_cmds::custom_shapes(&app.session).iter().map(|c| c.name.clone()).collect()
}

fn refuses_then_stores(mut h: Harness<'static, Box<PhotocraftApp>>) {
    for typed in ["", "   "] {
        ok_with(&mut h, typed);
        assert!(h.query_all_by_label("Shape Name").next().is_some(), "{typed:?}: the alert is titled Shape Name");
        let app = h.state();
        assert_eq!(alert_message(app).as_deref(), Some("You must specify a name for the shape."), "{typed:?}");
        assert!(dialog_open(app), "{typed:?}: the name dialog stays open");
        assert!(stored(app).is_empty(), "{typed:?}: nothing is stored");
        let dialogs = &mut h.state_mut().framework.dialogs;
        let alert = dialogs.get::<HostedAlert>().map(StackDialog::id).unwrap();
        dialogs.close_by_id(alert);
        h.run_steps(3);
    }
    ok_with(&mut h, "  RectA  ");
    let app = h.state();
    assert!(alert_message(app).is_none());
    assert!(!dialog_open(app), "a name closes the dialog");
    assert_eq!(stored(app), ["  RectA  "]);
}

#[test]
fn the_pen_menu_dialog_refuses_an_empty_name_with_the_alert() {
    let mut app = app();
    choose_open(&mut app, "edit.defineCustomShape");
    assert!(dialog_open(&app));
    refuses_then_stores(harness(app));
}

#[test]
fn the_edit_menu_dialog_refuses_an_empty_name_with_the_alert() {
    let mut app = app();
    crate::filter_dialog::open(&mut app, "edit.defineCustomShape").unwrap();
    assert!(dialog_open(&app));
    let mut h = harness(app);
    // The Path row names the Work Path: without it this dialog fails with "no such path" on main (#14284).
    set(&mut h, "path", "work");
    refuses_then_stores(h);
}
