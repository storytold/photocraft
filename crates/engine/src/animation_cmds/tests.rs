use super::*;
use photocraft_doc::LayerId;
use serde_json::json;

/// A 16×12 document with `names` as layers, bottom to top (the new-document Background is first).
fn session(names: &[&str]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    for n in names {
        s.execute("layer.new.layer", json!({"name": n})).unwrap();
    }
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn shown(s: &Session) -> Vec<(String, bool)> {
    doc(s).layers.iter().map(|l| (l.name.clone(), l.visible)).collect()
}

fn delays(s: &Session) -> Vec<u32> {
    doc(s).frame_animation.as_ref().unwrap().frames.iter().map(|f| f.delay_ms).collect()
}

fn current(s: &Session) -> usize {
    doc(s).frame_animation.as_ref().unwrap().current
}

fn id_of(s: &Session, name: &str) -> LayerId {
    doc(s).layers.iter().find(|l| l.name == name).unwrap().id
}

#[test]
fn make_frames_from_layers_shows_one_layer_per_frame() {
    let mut s = session(&["A", "B"]);
    let r = s.execute("animation.makeFramesFromLayers", json!({"seconds": 0.2})).unwrap();
    assert_eq!(r["animation"]["frames"].as_array().unwrap().len(), 3);
    assert_eq!(delays(&s), [200, 200, 200]);
    assert_eq!(shown(&s), [("Background".into(), true), ("A".into(), false), ("B".into(), false)]);
    s.execute("animation.selectFrame", json!({"frame": 2})).unwrap();
    assert_eq!(shown(&s), [("Background".into(), false), ("A".into(), false), ("B".into(), true)]);
    assert_eq!(current(&s), 2);
}

#[test]
fn visibility_edits_change_only_the_selected_frame() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    assert_eq!(current(&s), 1);
    let a = id_of(&s, "A");
    s.execute("layer.hideLayers", json!({"layer": a.0})).unwrap();
    let anim = doc(&s).frame_animation.as_ref().unwrap();
    assert!(anim.frames[0].hidden.is_empty());
    assert_eq!(anim.frames[1].hidden, [a]);
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    assert!(doc(&s).layer(a).unwrap().visible);
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert!(!doc(&s).layer(a).unwrap().visible);
}

#[test]
fn a_new_layer_shows_in_every_frame() {
    let mut s = session(&[]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Late"})).unwrap();
    for f in [0, 1] {
        s.execute("animation.selectFrame", json!({"frame": f})).unwrap();
        assert!(shown(&s).iter().any(|(n, v)| n == "Late" && *v), "frame {f}");
    }
}

#[test]
fn delays_apply_to_the_selected_frame_a_list_or_all() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    s.execute("animation.setDelay", json!({"seconds": 0.5})).unwrap();
    assert_eq!(delays(&s), [500, 0, 0]);
    s.execute("animation.setDelay", json!({"delayMs": 40, "frames": [1, 2]})).unwrap();
    assert_eq!(delays(&s), [500, 40, 40]);
    s.execute("animation.setDelay", json!({"seconds": 0.1, "frames": "all"})).unwrap();
    assert_eq!(delays(&s), [100, 100, 100]);
}

#[test]
fn frame_changes_undo_and_redo() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    s.execute("animation.setDelay", json!({"seconds": 1})).unwrap();
    assert_eq!(delays(&s), [0, 1000]);
    assert!(s.undo());
    assert_eq!(delays(&s), [0, 0]);
    assert!(s.undo());
    assert_eq!(delays(&s), [0]);
    assert!(s.redo());
    assert!(s.redo());
    assert_eq!(delays(&s), [0, 1000]);
}

#[test]
fn selecting_a_frame_is_not_a_history_step_and_keeps_a_clean_document_clean() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let steps = s.active().unwrap().history.can_undo();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    assert_eq!(current(&s), 1, "next stops at the last frame");
    s.execute("animation.previousFrame", json!({})).unwrap();
    s.execute("animation.previousFrame", json!({})).unwrap();
    assert_eq!(current(&s), 0);
    assert_eq!(s.active().unwrap().history.can_undo(), steps);
}

#[test]
fn loop_counts() {
    let mut s = session(&[]);
    s.execute("animation.create", json!({})).unwrap();
    for (v, plays) in [(json!("once"), 1), (json!(5), 5), (json!("forever"), 0), (json!(1), 1), (json!(0), 0)] {
        let r = s.execute("animation.setLoop", json!({"loops": v})).unwrap();
        assert_eq!(doc(&s).frame_animation.as_ref().unwrap().loops.plays(), plays, "{r}");
    }
    assert!(s.execute("animation.setLoop", json!({"loops": "sometimes"})).is_err());
    assert!(s.execute("animation.setLoop", json!({})).is_err());
}

#[test]
fn move_reverse_and_delete_keep_the_selection_on_its_frame() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    for (i, ms) in [(0, 10), (1, 20), (2, 30)] {
        s.execute("animation.setDelay", json!({"delayMs": ms, "frames": [i]})).unwrap();
    }
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    s.execute("animation.moveFrame", json!({"from": 0, "to": 2})).unwrap();
    assert_eq!(delays(&s), [20, 30, 10]);
    assert_eq!(current(&s), 2, "the selected frame moved with it");
    s.execute("animation.reverseFrames", json!({})).unwrap();
    assert_eq!(delays(&s), [10, 30, 20]);
    assert_eq!(current(&s), 0);
    s.execute("animation.deleteFrame", json!({"frame": 1})).unwrap();
    assert_eq!(delays(&s), [10, 20]);
    assert_eq!(current(&s), 0);
    s.execute("animation.deleteFrame", json!({})).unwrap();
    assert_eq!(delays(&s), [20]);
    assert!(s.execute("animation.deleteFrame", json!({})).is_err(), "the last frame stays");
}

#[test]
fn deleting_the_animation_keeps_the_layers_as_they_show() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    let before = shown(&s);
    s.execute("animation.delete", json!({})).unwrap();
    assert!(doc(&s).frame_animation.is_none());
    assert_eq!(shown(&s), before);
    assert!(s.execute("animation.newFrame", json!({})).is_err());
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = session(&["A"]);
    assert!(s.execute("animation.selectFrame", json!({"frame": 0})).is_err(), "no animation yet");
    s.execute("animation.create", json!({})).unwrap();
    assert!(s.execute("animation.create", json!({})).is_err(), "only one");
    for (cmd, p) in [
        ("animation.selectFrame", json!({"frame": 9})),
        ("animation.selectFrame", json!({})),
        ("animation.selectFrame", json!({"frame": -1})),
        ("animation.setDelay", json!({})),
        ("animation.setDelay", json!({"seconds": -1})),
        ("animation.setDelay", json!({"seconds": 1e12})),
        ("animation.setDelay", json!({"seconds": "soon"})),
        ("animation.setDelay", json!({"seconds": 1, "frames": [7]})),
        ("animation.setDelay", json!({"seconds": 1, "frames": 3})),
        ("animation.moveFrame", json!({"from": 0, "to": 5})),
        ("animation.deleteFrame", json!({"frame": 4})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
}
