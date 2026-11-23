//! #13709: the Create Droplet dialog's Play group has Override Action "Open" Commands, Include All
//! Subfolders, Suppress File Open Options Dialogs and Suppress Color Profile Warnings, in that order
//! under the Set and Action choices and all off, and OK saves what they show in the droplet.

use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use craft::ui::dialog::DialogId;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The Play checkboxes, top to bottom.
const PLAY: [&str; 4] =
    ["Override Action \"Open\" Commands", "Include All Subfolders", "Suppress File Open Options Dialogs", "Suppress Color Profile Warnings"];

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app with an action "Invert" in set "Tools" selected, and the Create Droplet dialog open on it.
fn open_dialog() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut action = Action::new("Invert", vec![ActionStep::new("image.adjustments.invert", json!({}))]);
    action.set = Some("Tools".into());
    app.session.actions_mut().list.push(action).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    let id = crate::dialogs::file_forms::open_create_droplet(&mut app).unwrap();
    let mut h = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    (h, id)
}

/// Whether the checkbox `label` shows a tick.
fn ticked(h: &H, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

/// Points the droplet at `path`, then presses OK and reads back the saved Play options.
fn save(h: &mut H, id: DialogId, path: &str) -> [Value; 4] {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "path", json!(path)).unwrap();
    h.run_steps(2);
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let options = &saved["options"];
    ["overrideOpen", "includeSubfolders", "suppressOpenOptions", "suppressWarnings"].map(|key| options[key].clone())
}

#[test]
fn the_play_checkboxes_sit_under_set_and_action_in_order_and_start_off() {
    let (h, _) = open_dialog();
    let top = |label: &str| h.get_by_label(label).rect().top();
    let action = top("Action");
    assert!(top("Set") < action, "Set comes before Action");
    let rows: Vec<f32> = PLAY.iter().map(|label| h.get_by_role_and_label(Role::CheckBox, label).rect().top()).collect();
    assert!(action < rows[0], "the Play checkboxes are under the Action choice: {action} vs {rows:?}");
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "in the reference order: {rows:?}");
    assert!(rows[3] < top("Format"), "before the destination settings: {rows:?}");
    for label in PLAY {
        assert!(!ticked(&h, label), "{label} starts off");
    }
}

#[test]
fn ticking_the_play_checkboxes_saves_them_in_the_droplet() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, id) = open_dialog();
    // Tick all but the first, then (in a second dialog) the first alone, so each click lands on its own checkbox.
    for label in &PLAY[1..] {
        h.get_by_role_and_label(Role::CheckBox, label).click();
        h.run_steps(2);
    }
    assert!(!ticked(&h, PLAY[0]) && PLAY[1..].iter().all(|label| ticked(&h, label)));
    let path = dir.path().join("Invert.pcdroplet").to_string_lossy().into_owned();
    assert_eq!(save(&mut h, id, &path), [json!(false), json!(true), json!(true), json!(true)]);

    let (mut h, id) = open_dialog();
    // The framework remembers a form's values between openings: clear the three ticked last time.
    for label in &PLAY[1..] {
        if ticked(&h, label) {
            h.get_by_role_and_label(Role::CheckBox, label).click();
            h.run_steps(2);
        }
    }
    h.get_by_role_and_label(Role::CheckBox, PLAY[0]).click();
    h.run_steps(2);
    let path = dir.path().join("Open.pcdroplet").to_string_lossy().into_owned();
    assert_eq!(save(&mut h, id, &path), [json!(true), json!(false), json!(false), json!(false)]);
}
