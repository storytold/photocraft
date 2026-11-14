//! #13681: OK in File › Automate › Conditional Mode Change… with nothing to convert shows
//! Photoshop's alert, and after its OK the dialog is still there with the same values; the
//! document and its history are untouched. Run without the dialog, the command stays silent.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::ColorMode;
use serde_json::json;

use super::COMMAND;
use super::tests::{LEFT, RIGHT, checked, click, field, mode, opened, pick_target, steps};
use crate::PhotocraftApp;
use crate::state::DialogKind;

const NO_SOURCE: &str = "At least one Source Mode must be selected.";
const NO_MATCH: &str = "Mode of the document does not match any selected Source Modes.";
const SAME_MODE: &str = "Mode of the document is the same as the Target Mode.  No conversion performed.";

fn alerts(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields["message"].as_str().map(str::to_string)).collect()
}

/// The alert's own OK: the dialog's under it is greyed while the alert is up.
fn ok_the_alert(h: &mut Harness<'_, PhotocraftApp>) {
    let ok = h.query_all_by_role_and_label(Role::Button, "OK").find(|n| !n.accesskit_node().is_disabled());
    ok.unwrap_or_else(|| panic!("the alert's OK")).click();
    h.run_steps(3);
}

/// Each case set up through the real controls of an RGB document's dialog, with its alert. Every
/// case starts from All or None and picks its target, whatever the dialog opens with. The last
/// one was not measured: unticked and the same as the target, and the order makes it "does not
/// match".
#[test]
fn ok_with_nothing_to_convert_shows_the_alert_and_keeps_the_dialog_with_its_values() {
    let cases: [(&str, &[&str], &str, &str); 5] = [
        ("nothing ticked", &["None"], "Grayscale", NO_SOURCE),
        ("nothing ticked, target RGB Color", &["None"], "RGB Color", NO_SOURCE),
        ("RGB Color unticked", &["All", "RGB Color"], "Grayscale", NO_MATCH),
        ("every source, target RGB Color", &["All"], "RGB Color", SAME_MODE),
        ("RGB Color unticked, target RGB Color", &["All", "RGB Color"], "RGB Color", NO_MATCH),
    ];
    for (case, clicks, target, message) in cases {
        let (mut h, id) = opened();
        for label in clicks {
            let role = if ["All", "None"].contains(label) { Role::Button } else { Role::CheckBox };
            click(&mut h, role, label);
        }
        pick_target(&mut h, target);
        let values = (field(&h, id, "from"), field(&h, id, "to"));
        let boxes: Vec<bool> = LEFT.into_iter().chain(RIGHT).map(|l| checked(&h, l)).collect();
        let (revision, before) = (h.state().session.active().unwrap().revision, steps(&h));

        click(&mut h, Role::Button, "OK");
        assert_eq!(alerts(&h), [message], "{case}");
        assert!(h.query_by_label(message).is_some(), "{case}: the alert shows its text");
        ok_the_alert(&mut h);

        let open: Vec<u64> = h.state().ui.dialogs.iter().map(|d| d.id).collect();
        assert_eq!(open, [id], "{case}: the alert is gone, the dialog is still open");
        assert_eq!((field(&h, id, "from"), field(&h, id, "to")), values, "{case}: the same values");
        assert_eq!(LEFT.into_iter().chain(RIGHT).map(|l| checked(&h, l)).collect::<Vec<_>>(), boxes, "{case}: the same boxes");
        let st = h.state().session.active().unwrap();
        assert_eq!((mode(&h), st.revision, steps(&h)), (ColorMode::Rgb, revision, before), "{case}: the document and its history are unchanged");
    }
}

/// Confirming the dialog again while its alert is up, as automation can, is refused again
/// without a second alert.
#[test]
fn a_second_confirm_does_not_stack_another_alert() {
    let (mut h, id) = opened();
    click(&mut h, Role::Button, "All");
    pick_target(&mut h, "RGB Color");
    for _ in 0..2 {
        assert_eq!(crate::dialogs::confirm(h.state_mut(), id), Err(SAME_MODE.to_string()));
        h.run_steps(3);
    }
    assert_eq!(alerts(&h), [SAME_MODE]);
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "the dialog is still open");
}

/// After the alert the dialog still works: ticking a source that matches converts on OK.
#[test]
fn after_the_alert_the_dialog_converts_once_a_source_matches() {
    let (mut h, id) = opened();
    click(&mut h, Role::Button, "None");
    pick_target(&mut h, "Lab Color");
    click(&mut h, Role::Button, "OK");
    assert_eq!(alerts(&h), [NO_SOURCE]);
    ok_the_alert(&mut h);
    click(&mut h, Role::CheckBox, "RGB Color");
    assert_eq!(field(&h, id, "from"), json!(["rgb"]));
    click(&mut h, Role::Button, "OK");
    assert!(h.state().ui.dialogs.is_empty(), "a run that converts closes the dialog");
    assert_eq!(mode(&h), ColorMode::Lab);
}

/// With parameters (the path of the control channel, MCP and the CLI) the command runs without the
/// dialog: no alert, `changed: false` and the reason. An action step runs the engine command
/// directly (`Session::execute`); the engine test covers that path.
#[test]
fn run_without_the_dialog_stays_silent_and_gives_the_reason() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    for (from, to, reason) in [(json!([]), "rgb", "noSource"), (json!(["lab"]), "rgb", "noMatch"), (json!("any"), "rgb", "sameMode")] {
        let r = crate::menus::invoke(&mut app, &egui::Context::default(), COMMAND, json!({"from": from, "to": to})).unwrap();
        assert_eq!(r, json!({"changed": false, "mode": "rgb", "reason": reason}), "{from} to {to}");
        assert!(app.ui.dialogs.is_empty(), "{from} to {to}: no dialog and no alert");
    }
}
