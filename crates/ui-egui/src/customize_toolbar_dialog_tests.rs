//! #14616: Edit › Toolbar… opens the Customize Toolbar dialog: the help text, a Toolbar list and an
//! Extra Tools list of tool groups from the stored layout, Done (the default, Enter), Cancel,
//! Restore Defaults and Clear Tools. Restore Defaults and Clear Tools change only the dialog until
//! Done; Cancel leaves the toolbar as it was. No history state; it opens with no document; its
//! state is the `toolbar` and `extras` fields over the control channel.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{HELP, KIND, clear_tools, restore_defaults};
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome, handle};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn open(app: &mut PhotocraftApp) -> u64 {
    let r = crate::menus::invoke(app, &egui::Context::default(), "edit.toolbar", json!({})).expect("Edit › Toolbar… opens a dialog");
    r["dialog"].as_u64().expect("a dialog id")
}

fn field(app: &PhotocraftApp, id: u64, key: &str) -> Vec<Vec<String>> {
    let d = app.ui.dialogs.iter().find(|d| d.id == id).expect("the dialog is open");
    serde_json::from_value(d.fields[key].clone()).unwrap()
}

fn stored(app: &PhotocraftApp) -> photocraft_engine::prefs::ToolbarLayout {
    app.session.prefs().toolbar.layout()
}

fn defaults() -> photocraft_engine::prefs::ToolbarLayout {
    photocraft_engine::prefs::ToolbarCustomization::default().layout()
}

#[test]
fn edit_toolbar_opens_customize_toolbar_on_the_stored_layout_with_no_document() {
    let mut app = app();
    assert!(app.session.active().is_none(), "no document open");
    let id = open(&mut app);
    let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
    assert_eq!(d.fields["__prefsui"], KIND);
    assert_eq!(d.fields["__label"], "Customize Toolbar");
    assert_eq!(field(&app, id, "toolbar"), defaults().toolbar);
    assert_eq!(field(&app, id, "toolbar")[0], ["Move", "Artboard"], "groups in toolbar order");
    assert!(field(&app, id, "extras").is_empty(), "Extra Tools is empty by default");
    // A customised layout shows as stored.
    crate::dialogs::cancel(&mut app, id).unwrap();
    app.run("edit.toolbar", json!({"extras": [["Zoom"]]})).unwrap();
    let id = open(&mut app);
    assert_eq!(field(&app, id, "extras"), [["Zoom"]]);
    assert!(!field(&app, id, "toolbar").iter().flatten().any(|t| t == "Zoom"));
}

#[test]
fn clear_tools_moves_every_group_to_extra_tools_and_done_allows_an_empty_toolbar() {
    let mut app = app();
    let id = open(&mut app);
    let before = field(&app, id, "toolbar");
    clear_tools(&mut app.ui.dialog_mut(id).unwrap().fields);
    assert!(field(&app, id, "toolbar").is_empty());
    assert_eq!(field(&app, id, "extras"), before, "every group, in order");
    crate::dialogs::confirm(&mut app, id).unwrap();
    let layout = stored(&app);
    assert!(layout.toolbar.is_empty(), "Done with an empty Toolbar list is allowed");
    assert_eq!(layout.extras, before);
}

#[test]
fn restore_defaults_and_cancel_only_change_the_dialog() {
    let mut app = app();
    app.run("edit.toolbar", json!({"extras": [["Zoom"], ["Hand", "RotateView"]]})).unwrap();
    let custom = stored(&app);
    // Restore Defaults, then Cancel: the toolbar stays as it was.
    let id = open(&mut app);
    restore_defaults(&mut app.ui.dialog_mut(id).unwrap().fields);
    assert_eq!((field(&app, id, "toolbar"), field(&app, id, "extras")), (defaults().toolbar, defaults().extras));
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(stored(&app), custom);
    // Clear Tools, then Cancel: the same.
    let id = open(&mut app);
    clear_tools(&mut app.ui.dialog_mut(id).unwrap().fields);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(stored(&app), custom);
    // Restore Defaults, then Done: the defaults.
    let id = open(&mut app);
    restore_defaults(&mut app.ui.dialog_mut(id).unwrap().fields);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(stored(&app), defaults());
}

#[test]
fn no_history_state_is_added() {
    let mut app = app();
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    let steps = app.session.active().unwrap().history.entries().len();
    let id = open(&mut app);
    clear_tools(&mut app.ui.dialog_mut(id).unwrap().fields);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
}

#[test]
fn the_dialog_is_read_and_set_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app();
    let id = open(&mut app);
    let (inspect, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Outcome::Done(inspect) = handle(&mut app, &ctx, &inspect) else { panic!("ui.inspect replies at once") };
    let listed = inspect["result"]["dialogs"].as_array().unwrap().iter().find(|d| d["id"] == id).cloned().expect("listed");
    assert_eq!(listed["fields"]["toolbar"][0], json!(["Move", "Artboard"]));
    let toolbar = json!([["Brush", "Pencil"], ["Move"]]);
    let extras = json!([["Zoom"]]);
    for (k, v) in [("toolbar", &toolbar), ("extras", &extras)] {
        let (set, _rx) = ControlRequest::new("ui.dialog.set", json!({"dialog": id, "field": k, "value": v}));
        assert!(matches!(handle(&mut app, &ctx, &set), Outcome::Done(r) if r["ok"] == true), "{k}");
    }
    let (confirm, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": id}));
    assert!(matches!(handle(&mut app, &ctx, &confirm), Outcome::Done(r) if r["ok"] == true));
    let layout = stored(&app);
    assert_eq!(layout.toolbar[..2], [vec!["Brush".to_string(), "Pencil".to_string()], vec!["Move".to_string()]]);
    assert!(layout.extras.contains(&vec!["Zoom".to_string()]));
}

#[test]
fn the_dialog_shows_its_lists_and_buttons_and_its_buttons_work() {
    let mut app = app();
    let id = open(&mut app);
    let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    if !h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        return;
    }
    for label in [HELP, "Toolbar", "Extra Tools", "Done", "Cancel", "Restore Defaults", "Clear Tools", "Move", "Rectangular Marquee"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    h.get_by_label("Clear Tools").click();
    h.run_steps(2);
    assert!(field(h.state(), id, "toolbar").is_empty(), "Clear Tools empties the Toolbar list");
    h.get_by_label("Restore Defaults").click();
    h.run_steps(2);
    assert_eq!(field(h.state(), id, "toolbar"), defaults().toolbar);
    // Enter is Done.
    h.get_by_label("Clear Tools").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id), "Enter closed it");
    assert!(stored(h.state()).toolbar.is_empty(), "and committed it");
}
