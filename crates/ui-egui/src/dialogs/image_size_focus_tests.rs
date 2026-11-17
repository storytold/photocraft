//! Image Size opened from the menu or its shortcut starts with no field focused, so typing doesn't
//! overwrite the width (#14800). Tab focuses Width first, Enter is OK and Escape cancels. Image
//! Size shown by an action (playback or Record Again) still opens on Width, as Canvas Size does
//! (#2329, `sizing::tests::canvas_size_opens_on_width_selected`).

use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

use super::show;
use crate::PhotocraftApp;

/// `command`'s dialog opened through the menu for a new 200 x 100 px document, after `prepare`
/// runs on its fields before the first frame.
fn open_with(command: &str, prepare: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap()["dialog"].as_u64().unwrap();
    prepare(&mut h.state_mut().ui.dialog_mut(id).unwrap().fields);
    h.run_steps(3);
    h
}

fn open_from_menu(command: &str) -> Harness<'static, PhotocraftApp> {
    open_with(command, |_| {})
}

fn width_focused(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.query_all_by_role(Role::SpinButton).next().expect("the Width field").is_focused()
}

fn any_field_focused(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.query_all_by_role(Role::SpinButton).any(|n| n.is_focused()) || h.query_all_by_role(Role::TextInput).any(|n| n.is_focused())
}

fn field(h: &Harness<'static, PhotocraftApp>, key: &str) -> f64 {
    let d = h.state().ui.dialogs.first().expect("the dialog is open");
    d.fields.get(key).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

#[test]
fn image_size_from_the_menu_opens_with_no_field_focused() {
    let mut h = open_from_menu("image.imageSize");
    assert!(!any_field_focused(&h), "no field has focus");
    h.event(egui::Event::Text("5".into()));
    h.run_steps(2);
    assert_eq!(field(&h, "width"), 200.0, "typing a digit doesn't change Width");
}

#[test]
fn tab_focuses_width_first_and_enter_is_ok() {
    let mut h = open_from_menu("image.imageSize");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert!(width_focused(&h), "Tab goes to Width first");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Escape cancels");
    assert_eq!(h.state().session.active().unwrap().doc.size.width, 200);

    let mut h = open_from_menu("image.imageSize");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Enter is OK with nothing focused");
    assert_eq!(h.state().session.active().unwrap().doc.size.width, 200, "unchanged values applied");
}

/// Shown by an action (the dialog carries its paused playback's token), Image Size opens on Width.
#[test]
fn image_size_shown_by_an_action_opens_on_width() {
    let h = open_with("image.imageSize", |f| {
        f.insert(crate::actions::RESUME.into(), json!(1));
    });
    assert!(width_focused(&h), "Width has focus");
}

/// Record Again on a recorded Image Size step, from the Actions panel menu, opens on Width.
#[test]
fn image_size_from_record_again_opens_on_width() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("actions.record", json!({"name": "Resize"})).unwrap();
    app.run("image.imageSize", json!({"width": 100, "height": 50})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    app.ui.actions.selected = Some(0);
    app.ui.actions.selected_step = Some(0);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app| {
            if app.ui.dialogs.is_empty() {
                crate::actions::panel_menu(app, ui);
            }
            show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    h.get_by_label("Record Again").click();
    h.run_steps(4);
    let d = h.state().ui.dialogs.first().expect("the Record Again editor is open");
    assert_eq!(d.fields.get("__command").and_then(Value::as_str), Some("image.imageSize"));
    assert!(crate::actions::record_again::owns(&d.fields), "opened by Record Again");
    assert!(width_focused(&h), "Width has focus");
    h.event(egui::Event::Text("50".into()));
    h.run_steps(2);
    assert_eq!(field(&h, "width"), 50.0, "typing replaced the selected width");
}
