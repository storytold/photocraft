//! Record Again edits one definition only after its concrete command succeeds.
use photocraft_engine::actions_cmds::ActionsExt;
use photocraft_engine::{
    EngineError, Session,
    actions_cmds::{Action, ActionStep},
    atn_cmds,
};
use serde_json::{Value, json};

/// A disabled, dialog-enabled resize followed by another step exposes flag and ordering loss.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let mut a = Action::new(
        "A",
        vec![ActionStep::new("image.imageSize", json!({"width": 4, "height": 3, "resample": "nearest"})), ActionStep::new("select.all", json!({}))],
    );
    a.disabled.insert(0);
    a.dialogs.insert(0);
    s.actions_mut().list.push(a).unwrap();
    s
}

/// Only the selected step runs, retains its flags, and exports its new settings to ATN.
#[test]
fn successful_rerecording_updates_one_step_and_roundtrips_new_settings() {
    for menu in [false, true] {
        let mut s = session();
        if menu {
            s.actions_mut().list.replace_step(0, 0, ActionStep::new(atn_cmds::MENU_ITEM, json!({"id": "image.imageSize"}))).unwrap();
        }
        let revision = s.actions().list.revision();
        s.execute("actions.recordAgain", json!({"action": "A", "step": 0, "revision": revision, "params": {"width": 6, "height": 5, "resample": "nearest"}}))
            .unwrap();
        assert_eq!(s.active().unwrap().doc.size, photocraft_engine::doc::Size::new(6, 5));
        assert!(s.active().unwrap().doc.selection.is_none(), "the following step did not play");
        let a = &s.actions().list.actions()[0];
        assert_eq!(a.steps.len(), 2);
        assert_eq!(a.steps[0].command, "image.imageSize");
        assert_eq!(a.steps[0].params["width"], 6);
        assert!(a.disabled.contains(&0));
        assert!(a.dialogs.contains(&0));
        assert_eq!(s.actions().list.revision(), revision + 1);
        assert_eq!(s.journal.last().unwrap().0, "image.imageSize");
        let bytes = atn_cmds::export("Set", &[a], &[], false).unwrap().0;
        let loaded = atn_cmds::import(&bytes).unwrap();
        assert_eq!(loaded.actions[0].steps[0].params["width"].as_f64(), Some(6.0));
        assert!(loaded.actions[0].disabled.contains(&0));
        assert!(loaded.actions[0].dialogs.contains(&0));
    }
}

/// Permit the wrapper while denying its concrete edit, proving nested authorization remains active.
fn deny_resize(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "image.imageSize" { Err(EngineError::Other("resize denied".into())) } else { Ok(()) }
}

/// Invalid settings, stale selections, denied execution and recording preserve the original step.
#[test]
fn failed_or_stale_rerecording_never_replaces_the_original_step() {
    let mut s = session();
    let original = s.actions().list.clone();
    let revision = s.active().unwrap().revision;
    for request in [
        json!({"action":"A","step":0}),
        json!({"action":"A","step":0,"params":[]}),
        json!({"action":"A","step":0,"params":{"percent":-1}}),
        json!({"action":"A","step":99,"params":{}}),
        json!({"action":"Missing","step":0,"params":{}}),
        json!({"action":"A","step":0,"revision":original.revision() + 1,"params":{}}),
    ] {
        assert!(s.execute("actions.recordAgain", request.clone()).is_err(), "{request}");
        assert_eq!(s.actions().list, original);
        assert_eq!(s.actions().list.revision(), original.revision());
        assert_eq!(s.active().unwrap().revision, revision);
    }
    s.authorize = Some(deny_resize);
    assert!(s.execute("actions.recordAgain", json!({"action":"A","step":0,"params":{"width":6,"height":5}})).is_err());
    assert_eq!(s.actions().list, original);
    s.authorize = None;
    s.execute("actions.record", json!({"action":"A"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    assert!(s.execute("actions.recordAgain", json!({"action":"A","step":0,"params":{}})).is_err());
    assert_eq!(s.actions().list, original, "recording was not flushed by a refused edit");
}
