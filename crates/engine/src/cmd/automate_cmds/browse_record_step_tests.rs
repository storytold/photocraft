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
