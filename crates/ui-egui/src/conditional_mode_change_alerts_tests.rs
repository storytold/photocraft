//! #13681: OK in File › Automate › Conditional Mode Change… with nothing to convert shows
//! Photoshop's alert, and after its OK the dialog is still there with the same values; the
//! document and its history are untouched. Run without the dialog, the command stays silent.

use craft::ui::dialog::{DialogId, HostedAlert, StackDialog};
use egui::accesskit::Role;
use photocraft_color::ColorMode;
use serde_json::{Value, json};

use super::COMMAND;
use super::tests::{H, LEFT, RIGHT, checked, click, field, is_open, mode, opened, set, steps, try_ok};
use crate::PhotocraftApp;

const NO_SOURCE: &str = "At least one Source Mode must be selected.";
const NO_MATCH: &str = "Mode of the document does not match any selected Source Modes.";
const SAME_MODE: &str = "Mode of the document is the same as the Target Mode.  No conversion performed.";

/// The open alerts: their ids and messages.
fn alerts(h: &H) -> Vec<(DialogId, String)> {
    let dialogs = &h.state().framework.dialogs;
    dialogs
        .open_dialogs()
        .into_iter()
        .filter_map(|d| {
            let alert = dialogs.get_by_id::<HostedAlert>(d.id)?;
            let message = alert.json_fields()?.get("message").and_then(Value::as_str)?.to_owned();
            Some((d.id, message))
        })
        .collect()
}

fn messages(h: &H) -> Vec<String> {
    alerts(h).into_iter().map(|(_, m)| m).collect()
}

/// The alert's own OK.
fn ok_the_alert(h: &mut H) {
    let [(id, _)] = alerts(h).try_into().unwrap_or_else(|a: Vec<_>| panic!("one alert: {a:?}"));
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("the alert's OK");
    h.run_steps(3);
}

/// Each case set up through the real controls of an RGB document's dialog, with its alert. Every
/// case starts from All or None and picks its target, whatever the dialog opens with. The last
/// one was not measured: unticked and the same as the target, and the order makes it "does not
/// match".
#[test]
fn ok_with_nothing_to_convert_shows_the_alert_and_keeps_the_dialog_with_its_values() {
    let cases: [(&str, &[&str], &str, &str); 5] = [
        ("nothing ticked", &["None"], "grayscale", NO_SOURCE),
        ("nothing ticked, target RGB Color", &["None"], "rgb", NO_SOURCE),
        ("RGB Color unticked", &["All", "RGB Color"], "grayscale", NO_MATCH),
        ("every source, target RGB Color", &["All"], "rgb", SAME_MODE),
        ("RGB Color unticked, target RGB Color", &["All", "RGB Color"], "rgb", NO_MATCH),
    ];
    for (case, clicks, target, message) in cases {
        let (mut h, id) = opened();
        for label in clicks {
            let role = if ["All", "None"].contains(label) { Role::Button } else { Role::CheckBox };
            click(&mut h, role, label);
        }
        set(&mut h, id, "to", json!(target));
        let values = (field(&h, id, "from"), field(&h, id, "to"));
        let boxes: Vec<bool> = LEFT.into_iter().chain(RIGHT).map(|l| checked(&h, l)).collect();
        let (revision, before) = (h.state().session.active().unwrap().revision, steps(&h));

        assert!(try_ok(&mut h, id).is_err(), "{case}: OK is refused");
        assert_eq!(messages(&h), [message], "{case}");
        ok_the_alert(&mut h);

        assert!(alerts(&h).is_empty(), "{case}: the alert is gone");
        assert!(is_open(&h, id), "{case}: the dialog is still open");
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
    set(&mut h, id, "to", json!("rgb"));
    for _ in 0..2 {
        assert!(try_ok(&mut h, id).is_err());
    }
    assert_eq!(messages(&h), [SAME_MODE]);
    assert!(is_open(&h, id), "the dialog is still open");
}

/// After the alert the dialog still works: ticking a source that matches converts on OK.
#[test]
fn after_the_alert_the_dialog_converts_once_a_source_matches() {
    let (mut h, id) = opened();
    click(&mut h, Role::Button, "None");
    set(&mut h, id, "to", json!("lab"));
    assert!(try_ok(&mut h, id).is_err());
    assert_eq!(messages(&h), [NO_SOURCE]);
    ok_the_alert(&mut h);
    click(&mut h, Role::CheckBox, "RGB Color");
    assert_eq!(field(&h, id, "from"), json!(["rgb"]));
    try_ok(&mut h, id).expect("a run that converts is accepted");
    assert!(!is_open(&h, id), "a run that converts closes the dialog");
    assert_eq!(mode(&h), ColorMode::Lab);
}

/// With parameters (the path of the control channel, MCP and the CLI) the command runs without the
/// dialog: no alert, `changed: false` and the reason. An action step runs the engine command
/// directly (`Session::execute`); the engine test covers that path.
#[test]
fn run_without_the_dialog_stays_silent_and_gives_the_reason() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    let ctx = app.ctx().clone();
    for (from, to, reason) in [(json!([]), "rgb", "noSource"), (json!(["lab"]), "rgb", "noMatch"), (json!("any"), "rgb", "sameMode")] {
        let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"from": from, "to": to})).unwrap();
        assert_eq!(r, json!({"changed": false, "mode": "rgb", "reason": reason}), "{from} to {to}");
        assert!(app.framework.dialogs.is_empty(), "{from} to {to}: no dialog and no alert");
    }
}
