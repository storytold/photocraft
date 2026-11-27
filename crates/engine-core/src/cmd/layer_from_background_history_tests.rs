//! Layer › New › Layer from Background adds one History state named "Make Layer", as in the
//! reference application, however it runs (#16760): the menu entry, the double-click on the
//! Background row and its lock icon all run `layer.new.layerFromBackground` with no params, and a
//! recorded action step runs it with the params `atn_cmds` translates a Photoshop `set` into.

use serde_json::json;

use crate::Session;
use crate::actions_cmds::{Action, ActionStep, ActionsExt};

const CMD: &str = "layer.new.layerFromBackground";

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

fn has_background(s: &Session) -> bool {
    s.active().unwrap().doc.background().is_some()
}

fn document_with_background() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": "white"})).unwrap();
    assert!(has_background(&s), "the fixture starts with a Background");
    s
}

#[test]
fn every_way_in_adds_one_make_layer_state_that_undo_and_redo_step_over() {
    // `{}`: the menu entry, the double-click and the lock icon; the other is what a recorded `set`
    // on the Background plays as (`atn_cmds::steps_make::background_from`).
    for p in [json!({}), json!({"ifPresent": true, "select": false})] {
        let mut s = document_with_background();
        let before = history(&s);
        s.execute(CMD, p.clone()).unwrap();
        let after = history(&s);
        assert_eq!(after.len(), before.len() + 1, "{p}: one state: {after:?}");
        assert_eq!(after.last().map(String::as_str), Some("Make Layer"), "{p}: {after:?}");
        assert!(!has_background(&s), "{p}: the Background became a layer");

        s.execute("edit.undo", json!({})).unwrap();
        assert!(has_background(&s), "{p}: one undo brings the Background back");
        s.execute("edit.redo", json!({})).unwrap();
        assert!(!has_background(&s), "{p}: redo converts it again");
        assert_eq!(history(&s), after, "{p}: redo keeps the state's name");
    }
}

#[test]
fn a_played_action_step_keeps_the_name_when_each_step_is_its_own_state() {
    let mut s = document_with_background();
    s.execute("actions.setHistory", json!({"singleState": false})).unwrap();
    s.actions_mut().list.push(Action::new("Unlock", vec![ActionStep::new(CMD, json!({}))])).unwrap();
    let r = s.execute("actions.play", json!({"action": "Unlock"})).unwrap();
    assert!(r.get("failed").is_none(), "the step plays: {r}");
    assert!(!has_background(&s), "the step converted the Background");
    assert_eq!(history(&s).last().map(String::as_str), Some("Make Layer"), "{:?}", history(&s));
}
