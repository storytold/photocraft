//! #13849: a script file run through File › Scripts › Browse… while an action records becomes one
//! step naming the file; playing it runs the file again, once.

use serde_json::{Value, json};

use crate::Session;

/// A script file holding `body` in a new temporary folder.
fn script_file(body: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tidy up.txt");
    std::fs::write(&path, body).unwrap();
    (dir, path.to_string_lossy().into_owned())
}

/// Select all, fill, deselect and add a layer: four commands of their own.
const FOUR_STEPS: &str = "select.all {}\nedit.fill {\"color\": \"#3366cc\"}\nselect.deselect {}\nlayer.new.layer {}\n";

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 12, "height": 8, "background": "white"})).unwrap();
    s
}

fn layers(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

fn pixel(s: &Session) -> [f32; 4] {
    photocraft_compose::render(&s.active().unwrap().doc, photocraft_geom::Rect::new(3, 3, 4, 4)).px[0]
}

fn steps(s: &mut Session, action: &str) -> Value {
    s.execute("actions.get", json!({"action": action})).unwrap()["steps"].clone()
}

#[test]
fn recording_a_script_file_adds_one_step_naming_it() {
    let (_dir, path) = script_file(FOUR_STEPS);
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Tidy"})).unwrap();
    let run = s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    assert_eq!(run["ok"], true, "{run}");
    assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 1);
    assert_eq!(steps(&mut s, "Tidy"), json!([["file.scripts.browse", {"path": path}]]), "not the script's own commands");
    assert_eq!(crate::actions_cmds::browse_file_step("file.scripts.browse", &json!({"path": path})), Some(path.as_str()));
    assert_eq!(layers(&s), 2, "the script ran");
}

#[test]
fn playing_the_step_runs_the_file_again_once() {
    let (_dir, path) = script_file(FOUR_STEPS);
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Tidy"})).unwrap();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    // A direct run on a fresh document, to compare with.
    let mut direct = new_doc();
    let before = direct.active().unwrap().history.past_len();
    direct.execute("file.scripts.browse", json!({"path": path})).unwrap();
    let direct_states = direct.active().unwrap().history.past_len() - before;
    // The recorded step on another fresh document.
    s.execute("file.new", json!({"width": 12, "height": 8, "background": "white"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "Tidy"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(played["ran"], 1);
    assert_eq!(layers(&s), layers(&direct), "one new layer, not two");
    assert_eq!(pixel(&s), pixel(&direct));
    assert_eq!(direct_states, 4, "a direct run: one History state per command, as today");
}

#[test]
fn a_missing_file_fails_the_step_with_its_path() {
    let (dir, path) = script_file(FOUR_STEPS);
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Tidy"})).unwrap();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    drop(dir);
    s.execute("file.new", json!({"width": 12, "height": 8})).unwrap();
    let played = s.execute("actions.play", json!({"action": "Tidy"}));
    let message = match played {
        Ok(v) => v["failed"]["error"].as_str().map(str::to_owned).unwrap_or_else(|| panic!("the step fails: {v}")),
        Err(e) => e.to_string(),
    };
    assert!(message.contains(&path), "{message}");
    assert_eq!(layers(&s), 1, "nothing ran");
}

#[test]
fn a_script_file_that_plays_an_action_is_still_one_step() {
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Inner"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let (_dir, path) = script_file("actions.play {\"action\": \"Inner\"}\nselect.all {}\n");
    s.execute("actions.record", json!({"name": "Outer"})).unwrap();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(steps(&mut s, "Outer"), json!([["file.scripts.browse", {"path": path}]]));
    let before = layers(&s);
    s.execute("actions.play", json!({"action": "Outer"})).unwrap();
    assert_eq!(layers(&s), before + 1, "the inner action plays once");
}

#[test]
fn outside_recording_and_for_script_text_nothing_changes() {
    let (_dir, path) = script_file(FOUR_STEPS);
    // Not recording: the commands journal as today, then Browse.
    let mut s = new_doc();
    let start = s.journal.len();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    let ids: Vec<&str> = s.journal[start..].iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["select.all", "edit.fill", "select.deselect", "layer.new.layer", "file.scripts.browse"]);
    // Script text while recording keeps recording its commands (#3057).
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Text"})).unwrap();
    s.execute("file.scripts.browse", json!({"script": "select.all {}\nlayer.new.layer {}\n"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let recorded: Vec<String> = steps(&mut s, "Text").as_array().unwrap().iter().map(|st| st[0].as_str().unwrap().to_owned()).collect();
    assert_eq!(recorded, ["select.all", "layer.new.layer"]);
}

/// CodeRabbit on #15118: a script file that moves the action being recorded keeps the recording on that
/// action: `[Keep, Target]` recording into `Target`, the script moves it first, and the Browse step is
/// recorded in `Target`, not in `Keep`.
#[test]
fn a_script_that_moves_the_recorded_action_keeps_recording_into_it() {
    let (_dir, path) = script_file("actions.move {\"action\": \"Target\", \"to\": 0}\n");
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Keep"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("actions.record", json!({"name": "Target"})).unwrap();
    let run = s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    assert_eq!(run["ok"], true, "{run}");
    assert_eq!(s.actions.list[0].name, "Target", "the script moved it first");
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(steps(&mut s, "Target"), json!([["file.scripts.browse", {"path": path}]]), "recorded into the moved action");
    assert_eq!(steps(&mut s, "Keep"), json!([]), "and not into the one that took its place");
}

/// CodeRabbit on #15118: Repeat Transform repeats the last transform even when it ran in a script file during
/// recording (whose journal entries are cut out of the recorded action).
#[test]
fn repeat_transform_repeats_a_transform_run_by_a_script_file_while_recording() {
    let (_dir, path) = script_file("edit.transform {\"matrix\": [1, 0, 0, 1, 2, 0], \"interpolation\": \"nearest\"}\n");
    let mut s = new_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("actions.record", json!({"name": "Scale"})).unwrap();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert!(s.journal.iter().all(|(id, _)| id != "edit.transform"), "the entry is not in the journal");
    assert!(s.last_transform.is_some(), "but the last transform is kept");
    let again = s.execute("edit.transform.again", json!({}));
    assert!(again.is_ok(), "Repeat Transform has the script's transform to repeat: {again:?}");
}

/// CodeRabbit on #15118: a command in the script file followed by `actions.play` of another action must not
/// reach the recorded action: the play flushes the recording, which used to commit the file's earlier
/// command, so the action held it and the Browse step, and playback ran it twice. What the user recorded
/// before the file stays.
#[test]
fn a_command_then_actions_play_in_the_file_stays_one_browse_step() {
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Inner"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let (_dir, path) = script_file("layer.new.layer {}\nactions.play {\"action\": \"Inner\"}\n");
    s.execute("actions.record", json!({"name": "Outer"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let start = layers(&s);
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(steps(&mut s, "Outer"), json!([["select.all", {}], ["file.scripts.browse", {"path": path}]]), "the user's step, then the file as one step");
    assert_eq!(layers(&s), start + 2, "the file ran when it was recorded: its layer and Inner's");
    // Playing the action runs the file once more: two more layers, not four.
    s.execute("actions.play", json!({"action": "Outer"})).unwrap();
    assert_eq!(layers(&s), start + 4, "one run of the file, not two");
}

/// CodeRabbit on #15118: `actions.move` of a recorded step in the file, while the action holds no step but the
/// file: the file's own command is no step to move. The file is still one Browse step.
#[test]
fn a_command_then_a_step_move_in_the_file_stays_one_browse_step() {
    let mut s = new_doc();
    let (_dir, path) = script_file("layer.new.layer {}\nactions.move {\"action\": \"Target\", \"step\": 0, \"to\": 0}\n");
    s.execute("actions.record", json!({"name": "Target"})).unwrap();
    let run = s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    assert_eq!(run["results"][0].get("error"), None, "{run}");
    assert!(run["results"][1].get("error").is_some(), "no recorded step to move: {run}");
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(steps(&mut s, "Target"), json!([["file.scripts.browse", {"path": path}]]), "the file's command was not committed");
}

/// CodeRabbit on #15118: a script file that runs a command and then moves the recorded action keeps the
/// recording on that action, and the user's step before the file stays in it.
#[test]
fn a_command_then_a_list_move_in_the_file_records_into_the_moved_action() {
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Keep"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let (_dir, path) = script_file("layer.new.layer {}\nactions.move {\"action\": \"Target\", \"to\": 0}\n");
    s.execute("actions.record", json!({"name": "Target"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].name, "Target");
    assert_eq!(steps(&mut s, "Target"), json!([["select.all", {}], ["file.scripts.browse", {"path": path}]]));
    assert_eq!(steps(&mut s, "Keep"), json!([]));
}

/// CodeRabbit on #15118: Repeat Last Filter repeats a filter a script file applied during recording, though
/// the file's journal entries are cut from the recorded action.
#[test]
fn repeat_last_filter_repeats_a_filter_run_by_a_script_file_while_recording() {
    let (_dir, path) = script_file("filter.other.offset {\"horizontal\": 3}\n");
    let mut s = new_doc();
    s.execute("actions.record", json!({"name": "Shift"})).unwrap();
    s.execute("file.scripts.browse", json!({"path": path})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert!(s.journal.iter().all(|(id, _)| id != "filter.other.offset"), "the entry is not in the journal");
    let last = crate::filters::last_filter(&s);
    assert_eq!(last, Some(("filter.other.offset".to_owned(), json!({"horizontal": 3}))), "the filter the script ran");
    assert!(s.execute("filter.lastFilter", json!({})).is_ok());
}
