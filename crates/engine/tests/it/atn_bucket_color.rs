//! A Paint Bucket step's explicit colour survives an .atn round trip (#7199).
use photocraft_engine::actions_cmds::{Action, ActionList, ActionStep, ActionsExt, ActionsFile};
use photocraft_engine::{Session, atn_cmds};
use serde_json::{Value, json};

fn fill(actions: Vec<Action>, run_action: bool, params: &Value) -> Vec<f32> {
    let mut s = Session::new();
    // The default foreground is black, so only the step's own colour can make the fill red.
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ffffff"})).unwrap();
    if run_action {
        s.actions_mut().list = ActionList::from_file(ActionsFile::new(actions, Vec::new()));
        s.execute("actions.play", json!({"action": 0})).unwrap();
    } else {
        s.execute("paint.bucket", params.clone()).unwrap();
    }
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(2, 2)
}

#[test]
fn bucket_explicit_color_replays_after_atn_round_trip() {
    let red = json!({"x": 2, "y": 2, "antiAlias": false, "color": "#ff0000"});
    let action = Action::new("Bucket", vec![ActionStep::new("paint.bucket", red.clone())]);
    let (bytes, skipped) = atn_cmds::export("Set", &[&action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let loaded = atn_cmds::import(&bytes).unwrap();
    assert!(loaded.unsupported.is_empty());
    let replayed = fill(loaded.actions, true, &red);
    assert_eq!(replayed, fill(Vec::new(), false, &red));
    assert_eq!(replayed, vec![1.0, 0.0, 0.0, 1.0]);
}
