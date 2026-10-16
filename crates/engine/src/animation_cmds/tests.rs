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
    // Saved, so a step that wrongly dirtied the document would show.
    let steps = {
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        st.history.past_len()
    };
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    assert_eq!(current(&s), 1, "next stops at the last frame");
    s.execute("animation.previousFrame", json!({})).unwrap();
    s.execute("animation.previousFrame", json!({})).unwrap();
    assert_eq!(current(&s), 0);
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), steps, "navigation adds no history step");
    assert_eq!(st.saved_revision, st.revision, "a clean document stays clean");
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
        // A refused command leaves the animation, the layers and the history exactly as they were.
        let snapshot = |s: &Session| {
            let st = s.active().unwrap();
            (st.doc.frame_animation.clone(), shown(s), st.revision, st.history.can_undo())
        };
        let before = snapshot(&s);
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
        assert_eq!(snapshot(&s), before, "{cmd} {p} changed the document");
    }
}

#[test]
fn a_stored_hidden_list_in_any_order_still_hides_the_right_layers() {
    let mut s = session(&["A", "B", "C"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let ids: Vec<_> = doc(&s).layers.iter().map(|l| l.id).collect();
    // A hand-edited file: frame 1 hides Background, B and C, written in descending order with a repeat.
    let a = s.active_mut().unwrap();
    let mut d = (*a.doc).clone();
    d.frame_animation.as_mut().unwrap().frame_mut(1).unwrap().hidden = vec![ids[3], ids[2], ids[0], ids[2]];
    d.frame_animation.as_mut().unwrap().clamp();
    a.doc = std::sync::Arc::new(d);
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert_eq!(shown(&s), [("Background".into(), false), ("A".into(), true), ("B".into(), false), ("C".into(), false)]);
    assert_eq!(doc(&s).frame_animation.as_ref().unwrap().frames[1].hidden, [ids[0], ids[2], ids[3]], "sorted and without repeats");
}

#[test]
fn frames_from_layers_keep_the_eye_of_layers_inside_a_group() {
    use photocraft_doc::{Group, Layer, LayerContent};
    let mut s = session(&["A"]);
    // A group holding one visible and one hidden child, above the others.
    let mut hidden_child = Layer::new("hidden child", LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None }));
    hidden_child.visible = false;
    let shown_child = Layer::new("shown child", LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None }));
    let (hid, shown_id) = (hidden_child.id, shown_child.id);
    let group = Layer::new("G", LayerContent::Group(Group { children: vec![shown_child, hidden_child], expanded: true, artboard: None }));
    let st = s.active_mut().unwrap();
    let mut d = (*st.doc).clone();
    d.layers.push(group);
    st.doc = std::sync::Arc::new(d);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let last = doc(&s).layers.len() - 1;
    s.execute("animation.selectFrame", json!({"frame": last})).unwrap();
    let g = doc(&s).layers.last().unwrap();
    let kids = g.children().unwrap();
    assert!(g.visible);
    assert_eq!(kids.iter().map(|k| (k.id, k.visible)).collect::<Vec<_>>(), [(shown_id, true), (hid, false)], "the hidden child stays hidden");
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": last})).unwrap();
    assert!(!doc(&s).layers.last().unwrap().children().unwrap()[1].visible);
}

#[test]
fn navigation_replies_are_compact_and_not_journaled() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let r = s.execute("animation.nextFrame", json!({})).unwrap();
    assert_eq!(r, json!({"animation": {"current": 1, "frameCount": 3}}));
    for id in ["animation.selectFrame", "animation.nextFrame", "animation.previousFrame"] {
        assert!(!crate::commands::find(id).unwrap().journal, "{id} is navigation, not a recorded step");
    }
}

#[test]
fn the_delay_limit_in_the_error_is_exact() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    let e = s.execute("animation.setDelay", json!({"seconds": 1000})).unwrap_err().to_string();
    assert!(e.contains("655.35"), "{e}");
}

#[test]
fn too_many_layers_for_frames_are_refused() {
    use photocraft_doc::{Group, Layer, LayerContent};
    let mut s = session(&[]);
    let st = s.active_mut().unwrap();
    let mut d = (*st.doc).clone();
    for i in 0..1001 {
        d.layers.push(Layer::new(format!("L{i}"), LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None })));
    }
    st.doc = std::sync::Arc::new(d);
    assert!(s.execute("animation.makeFramesFromLayers", json!({})).is_err());
    assert!(doc(&s).frame_animation.is_none());
}

#[test]
fn selecting_a_frame_waits_for_a_background_job_like_an_edit_does() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let job = match s.start("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap() {
        crate::jobs::Started::Job(id) => id,
        crate::jobs::Started::Done(_) => return, // ran inline (no threads): nothing to wait for
    };
    let e = s.execute("animation.selectFrame", json!({"frame": 1})).unwrap_err().to_string();
    assert!(e.contains("still running"), "{e}");
    assert_eq!(current(&s), 0);
    let t = std::time::Instant::now();
    while !s.poll_jobs().iter().any(|e| e.id == job) {
        assert!(t.elapsed() < std::time::Duration::from_secs(60), "the job did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert_eq!(current(&s), 1);
}

#[test]
fn a_group_hidden_child_stays_hidden_in_the_frames_of_the_other_groups() {
    use photocraft_doc::{Group, Layer, LayerContent};
    let mut s = session(&[]);
    let group = |name: &str| {
        let mut hidden = Layer::new(format!("{name} hidden"), LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None }));
        hidden.visible = false;
        let id = hidden.id;
        (Layer::new(name, LayerContent::Group(Group { children: vec![hidden], expanded: true, artboard: None })), id)
    };
    let ((g1, _), (g2, h2)) = (group("G1"), group("G2"));
    let (g1_id, g2_id) = (g1.id, g2.id);
    let st = s.active_mut().unwrap();
    let mut d = (*st.doc).clone();
    d.layers.push(g1);
    d.layers.push(g2);
    st.doc = std::sync::Arc::new(d);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    // The frame for G1 (G2 is hidden in it): deleting the animation keeps the layers as that frame shows them.
    let frame_of_g1 = doc(&s).layers.iter().position(|l| l.id == g1_id).unwrap();
    s.execute("animation.selectFrame", json!({"frame": frame_of_g1})).unwrap();
    assert!(doc(&s).frame_animation.as_ref().unwrap().frames[frame_of_g1].hidden.contains(&h2), "G2's hidden child is listed in every frame");
    s.execute("animation.delete", json!({})).unwrap();
    let g2 = doc(&s).layers.iter().find(|l| l.id == g2_id).unwrap();
    assert!(!g2.visible, "G2 is hidden in that frame");
    assert!(!g2.children().unwrap()[0].visible, "and its hidden child is still hidden, ready for when it is shown");
}

#[test]
fn stepping_errors_name_the_command_that_was_run() {
    let mut s = session(&["A"]);
    // No animation: the error names the command asked for, not always Next Frame.
    for (cmd, delta) in [("animation.nextFrame", 1), ("animation.previousFrame", -1)] {
        let e = super::step(&mut s, cmd, delta, false).unwrap_err().to_string();
        assert!(e.contains(cmd), "{e}");
    }
}

#[test]
fn move_frame_needs_both_ends_and_a_refusal_leaves_no_history_step() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    for p in [json!({}), json!({"from": 2}), json!({"to": 0}), json!({"from": null, "to": 1}), json!({"from": 0, "to": null})] {
        assert!(s.execute("animation.moveFrame", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps, "nothing was recorded");
    s.execute("animation.moveFrame", json!({"from": 0, "to": 2})).unwrap();
}

#[test]
fn a_recorded_action_returns_to_the_frame_it_edited() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    let a = id_of(&s, "A");
    s.execute("actions.record", json!({"name": "Hide on 2"})).unwrap();
    // A click, then a step back and forth, land on frame 1: one recorded step.
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    s.execute("layer.hideLayers", json!({"layer": a.0})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps: Vec<String> = s.journal.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(steps.iter().filter(|id| *id == "animation.selectFrame").count(), 1, "{steps:?}");

    // Undo the hide, show it again in frame 1, and replay from frame 0.
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    s.execute("actions.play", json!({"action": "Hide on 2"})).unwrap();
    assert_eq!(current(&s), 1, "the action selects the frame it recorded on");
    let hidden_in = |s: &Session, f: usize| doc(s).frame_animation.as_ref().unwrap().frames[f].hidden.contains(&a);
    assert!(hidden_in(&s, 1), "the layer is hidden in frame 2");
    assert!(!hidden_in(&s, 0), "and frame 1 is untouched");

    // Selecting a frame outside recording still adds no journal entry or history step.
    let (journal, past) = (s.journal.len(), s.active().unwrap().history.past_len());
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    assert_eq!((s.journal.len(), s.active().unwrap().history.past_len()), (journal, past));
}

fn hidden_in(s: &Session, frame: usize) -> Vec<LayerId> {
    doc(s).frame_animation.as_ref().unwrap().frames[frame].hidden.clone()
}

#[test]
fn undo_and_redo_give_back_the_frame_that_was_edited() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let (a, b) = (id_of(&s, "A"), id_of(&s, "B"));
    let start: Vec<_> = (0..3).map(|f| hidden_in(&s, f)).collect();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    s.execute("layer.showLayers", json!({"layer": b.0})).unwrap();
    let after_first: Vec<_> = (0..3).map(|f| hidden_in(&s, f)).collect();
    s.execute("animation.selectFrame", json!({"frame": 2})).unwrap();
    s.execute("layer.showLayers", json!({"layer": a.0})).unwrap();
    let after_second: Vec<_> = (0..3).map(|f| hidden_in(&s, f)).collect();
    assert_ne!(start[1], after_first[1]);
    assert_eq!((&after_first[0], &after_first[2]), (&start[0], &start[2]), "only the selected frame changes");
    assert_eq!((&after_second[0], &after_second[1]), (&after_first[0], &after_first[1]));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((0..3).map(|f| hidden_in(&s, f)).collect::<Vec<_>>(), after_first);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((0..3).map(|f| hidden_in(&s, f)).collect::<Vec<_>>(), start);
    s.execute("edit.redo", json!({})).unwrap();
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!((0..3).map(|f| hidden_in(&s, f)).collect::<Vec<_>>(), after_second);
}

#[test]
fn a_copy_of_the_document_keeps_its_own_frames() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({"delayMs": 100})).unwrap();
    let before = (**s.active().map(|st| &st.doc).unwrap()).clone();
    let a = id_of(&s, "A");
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    s.execute("layer.hideLayers", json!({"layer": a.0})).unwrap();
    s.execute("animation.setDelay", json!({"delayMs": 5, "frames": "all"})).unwrap();
    s.execute("animation.deleteFrame", json!({"frame": 0})).unwrap();
    let kept = before.frame_animation.as_ref().unwrap();
    assert_eq!(kept.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [100, 100]);
    assert_eq!(kept.current, 0);
    assert_eq!(kept.frames.len(), 2);
    assert!(!kept.frames[1].hidden.contains(&a), "the copy still shows A in frame 2");
}

#[test]
fn a_duplicated_frame_and_its_original_are_edited_apart() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    let a = id_of(&s, "A");
    s.execute("layer.hideLayers", json!({"layer": a.0})).unwrap();
    s.execute("animation.setDelay", json!({"delayMs": 90})).unwrap();
    assert_eq!(delays(&s), [0, 90]);
    assert!(hidden_in(&s, 0).is_empty() && hidden_in(&s, 1) == [a]);
}

#[test]
fn reordering_frames_moves_each_frames_look_with_it_and_edits_after_stay_on_their_frame() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({"delayMs": 10})).unwrap();
    s.execute("animation.setDelay", json!({"delayMs": 30, "frames": [2]})).unwrap();
    let before: Vec<_> = (0..3).map(|f| (delays(&s)[f], hidden_in(&s, f))).collect();
    s.execute("animation.reverseFrames", json!({})).unwrap();
    let reversed: Vec<_> = (0..3).map(|f| (delays(&s)[f], hidden_in(&s, f))).collect();
    assert_eq!(reversed, before.iter().rev().cloned().collect::<Vec<_>>());
    s.execute("animation.moveFrame", json!({"from": 0, "to": 2})).unwrap();
    let moved: Vec<_> = (0..3).map(|f| (delays(&s)[f], hidden_in(&s, f))).collect();
    assert_eq!(moved, [reversed[1].clone(), reversed[2].clone(), reversed[0].clone()]);
    // An edit to the selected frame afterwards touches that frame only.
    let sel = current(&s);
    s.execute("animation.setDelay", json!({"delayMs": 77})).unwrap();
    for f in (0..3).filter(|f| *f != sel) {
        assert_eq!((delays(&s)[f], hidden_in(&s, f)), moved[f]);
    }
    assert_eq!(delays(&s)[sel], 77);
}

#[test]
fn pasted_frames_go_after_the_selection_and_undo() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({"delayMs": 100})).unwrap();
    let b = id_of(&s, "B");
    s.execute("animation.pasteFrames", json!({"frames": [{"delayMs": 250, "hidden": [b.0]}, {"delayMs": 300}], "after": 0})).unwrap();
    assert_eq!(delays(&s), [100, 250, 300, 100, 100]);
    assert_eq!(current(&s), 2, "the last pasted frame is selected");
    assert!(doc(&s).layers.iter().all(|l| l.visible), "the selected pasted frame hides nothing");
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert!(!doc(&s).layers.iter().find(|l| l.id == b).unwrap().visible);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(delays(&s), [100, 100, 100]);
}

#[test]
fn pasting_drops_layers_the_document_lacks_and_rejects_bad_params() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.pasteFrames", json!({"frames": [{"delayMs": 50, "hidden": [999_999]}]})).unwrap();
    let a = doc(&s).frame_animation.as_ref().unwrap();
    assert!(a.frames[1].hidden.is_empty());
    for bad in [
        json!({}),
        json!({"frames": []}),
        json!({"frames": [{"hidden": "x"}]}),
        json!({"frames": [{"delayMs": -1}]}),
        json!({"frames": [{}], "after": 9}),
        json!({"frames": [5]}),
        json!({"frames": ["x"]}),
        json!({"frames": [[1]]}),
        json!({"frames": [{}, 7]}),
    ] {
        assert!(s.execute("animation.pasteFrames", bad).is_err());
    }
    assert_eq!(delays(&s).len(), 2);
}

fn four(s: &mut Session) {
    for (i, ms) in [10, 20, 30, 40].iter().enumerate() {
        s.execute("animation.setDelay", json!({"delayMs": ms, "frames": [i]})).unwrap();
    }
}

fn four_frames() -> Session {
    let mut s = session(&["A", "B", "C"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    four(&mut s);
    s
}

#[test]
fn several_frames_delete_and_duplicate_together() {
    let mut s = four_frames();
    let r = s.execute("animation.newFrame", json!({"frames": [1, 2]})).unwrap();
    assert_eq!(delays(&s), [10, 20, 30, 20, 30, 40]);
    assert_eq!(r["selection"], json!([3, 4]));
    assert_eq!(current(&s), 4);
    let r = s.execute("animation.deleteFrame", json!({"frames": [0, 3, 4]})).unwrap();
    assert_eq!(delays(&s), [20, 30, 40]);
    assert_eq!(r["selection"], json!([2]), "the selected frame was deleted: the one after it takes over");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(delays(&s).len(), 6);
    assert!(s.execute("animation.deleteFrame", json!({"frames": [0, 1, 2, 3, 4, 5]})).is_err(), "one frame stays");
}

#[test]
fn a_group_of_frames_moves_into_a_gap_in_order() {
    let mut s = four_frames();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    let r = s.execute("animation.moveFrames", json!({"frames": [0, 1], "slot": 4})).unwrap();
    assert_eq!(delays(&s), [30, 40, 10, 20]);
    assert_eq!(r["selection"], json!([2, 3]));
    assert_eq!(current(&s), 3, "the selected frame went with its group");
    let r = s.execute("animation.moveFrames", json!({"frames": [1, 3], "slot": 0})).unwrap();
    assert_eq!(delays(&s), [40, 20, 30, 10]);
    assert_eq!(r["selection"], json!([0, 1]));
    s.execute("animation.moveFrames", json!({"frames": [2], "slot": 2})).unwrap();
    assert_eq!(delays(&s), [40, 20, 30, 10], "dropping a frame into its own gap changes nothing");
    for bad in [json!({"frames": [0]}), json!({"frames": [0], "slot": 5}), json!({"frames": [9], "slot": 0}), json!({"frames": [], "slot": 0})] {
        assert!(s.execute("animation.moveFrames", bad).is_err());
    }
}

#[test]
fn duplicated_frames_keep_their_own_look_whichever_frame_was_shown() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let before: Vec<_> = doc(&s).frame_animation.as_ref().unwrap().frames.iter().map(|f| f.hidden.clone()).collect();
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    s.execute("animation.newFrame", json!({"frames": [1, 2]})).unwrap();
    let a = doc(&s).frame_animation.as_ref().unwrap();
    assert_eq!(
        a.frames.iter().map(|f| f.hidden.clone()).collect::<Vec<_>>(),
        [before[0].clone(), before[1].clone(), before[2].clone(), before[1].clone(), before[2].clone()]
    );
    // The copy of frame 2 is shown, and the layers show it.
    assert_eq!(a.current, 4);
    assert_eq!(shown(&s), [("Background".into(), false), ("A".into(), false), ("B".into(), true)]);
}

#[test]
fn an_empty_frame_list_for_set_delay_is_refused_and_leaves_no_history_step() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.execute("animation.setDelay", json!({"seconds": 1, "frames": []})).is_err());
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    // A list still works, in any order and with repeats, and "all" and the selected frame are unchanged.
    s.execute("animation.setDelay", json!({"delayMs": 40, "frames": [2, 0, 2]})).unwrap();
    assert_eq!(delays(&s), [40, 0, 40]);
    s.execute("animation.setDelay", json!({"delayMs": 70, "frames": "all"})).unwrap();
    assert_eq!(delays(&s), [70; 3]);
}

#[test]
fn every_command_that_leaves_frames_selected_says_so_in_its_description() {
    // Scripts and agents read the `params` string; the commands that reply with `selection` must name it.
    for id in ["animation.newFrame", "animation.deleteFrame", "animation.moveFrames", "animation.pasteFrames"] {
        let spec = crate::commands::find(id).unwrap();
        assert!(spec.params.contains("selection"), "{id}: {}", spec.params);
    }
}
