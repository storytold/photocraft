//! #13709: the Create Droplet dialog's Play group has Override Action "Open" Commands, Include All
//! Subfolders, Suppress File Open Options Dialogs and Suppress Color Profile Warnings, in that order
//! under the Set and Action dropdowns and all off, and OK saves what they show in the droplet.

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The Play checkboxes, top to bottom.
const PLAY: [&str; 4] = ["Override Action \"Open\" Commands", "Include All Subfolders", "Suppress File Open Options Dialogs", "Suppress Color Profile Warnings"];

/// The app with an action "Invert" in set "Tools" selected, and the Create Droplet dialog open on
/// it. Returns the dialog's id.
fn open_dialog() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let mut action = photocraft_engine::actions_cmds::Action::new("Invert", vec![("image.adjustments.invert".into(), json!({}))]);
    action.set = Some("Tools".into());
    h.state_mut().session.actions.list = vec![action];
    h.state_mut().ui.actions.selected = Some(0);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "file.automate.createDroplet", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

/// Whether the checkbox `label` shows a tick.
fn ticked(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn the_play_checkboxes_sit_under_set_and_action_in_order_and_start_off() {
    let (h, _) = open_dialog();
    let top = |role: Role, label: &str| h.get_by_role_and_label(role, label).rect().top();
    let action = top(Role::ComboBox, "Action");
    assert!(top(Role::ComboBox, "Set") < action, "Set comes before Action");
    let rows: Vec<f32> = PLAY.iter().map(|label| top(Role::CheckBox, label)).collect();
    assert!(action < rows[0], "the Play checkboxes are under the Action dropdown: {action} vs {rows:?}");
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "in the reference order: {rows:?}");
    assert!(rows[3] < top(Role::ComboBox, "Format"), "before the destination settings: {rows:?}");
    for label in PLAY {
        assert!(!ticked(&h, label), "{label} starts off");
    }
}

#[test]
fn ticking_the_play_checkboxes_saves_them_in_the_droplet() {
    let (mut h, id) = open_dialog();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Invert.pcdroplet").to_string_lossy().into_owned();
    let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    fields.insert("path".into(), json!(path));
    fields.insert("shim".into(), json!(false));
    // Tick all but the first, then the first, so each click lands on its own checkbox.
    for label in [PLAY[1], PLAY[2], PLAY[3]] {
        h.get_by_role_and_label(Role::CheckBox, label).click();
        h.run_steps(2);
    }
    assert!(!ticked(&h, PLAY[0]) && PLAY[1..].iter().all(|label| ticked(&h, label)));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let options = &saved["options"];
    assert_eq!(
        [&options["overrideOpen"], &options["includeSubfolders"], &options["suppressOpenOptions"], &options["suppressWarnings"]],
        [&json!(false), &json!(true), &json!(true), &json!(true)],
        "{saved}"
    );

    let (mut h, id) = open_dialog();
    let path = dir.path().join("Open.pcdroplet").to_string_lossy().into_owned();
    let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    fields.insert("path".into(), json!(path));
    fields.insert("shim".into(), json!(false));
    h.get_by_role_and_label(Role::CheckBox, PLAY[0]).click();
    h.run_steps(2);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let options = &saved["options"];
    assert_eq!(
        [&options["overrideOpen"], &options["includeSubfolders"], &options["suppressOpenOptions"], &options["suppressWarnings"]],
        [&json!(true), &json!(false), &json!(false), &json!(false)],
        "{saved}"
    );
}
