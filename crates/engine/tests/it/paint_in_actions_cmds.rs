//! Tests that run paint commands, moved from the engine-core `actions_cmds` tests: 5 test(s).

use photocraft_engine::Session;
use photocraft_engine::actions_cmds::*;

use photocraft_engine::actions_cmds::ActionsExt;

use serde_json::json;

fn pixel(s: &Session, layer: usize, x: i32, y: i32) -> Vec<f32> {
    s.active().unwrap().doc.layers[layer].surface().unwrap().pixel(x, y)
}

/// CopyCat's shape: Edit Contents, change the contents, Save. The Save step has no file to
/// write: it updates the smart object, headless too.
#[test]
fn save_in_edit_contents_updates_the_smart_object() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    let steps = vec![ActionStep::new("edit.fill", json!({"contents": "color", "color": "#0000ff"})), ActionStep::new("file.save", json!({}))];
    s.actions_mut().list.push(Action::new("Edit and save", steps)).unwrap();
    let played = s.execute("actions.play", json!({"action": "Edit and save"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    s.set_active(0);
    let doc = &s.active().unwrap().doc;
    let flat = photocraft_compose::flatten_to_surface(doc, doc.pixel_format(), None);
    let p = flat.pixel(2, 2);
    assert!(p[0] < 0.01 && p[2] > 0.99, "the smart object shows the saved contents: {p:?}");
    // With no Edit Contents document, Save stays the shell's (it needs the document's file).
    s.actions_mut().list.push(Action::new("Save", vec![ActionStep::new("file.save", json!({}))])).unwrap();
    assert!(s.execute("actions.play", json!({"action": "Save"})).unwrap().get("failed").is_some());
}

fn record_red(s: &mut Session, depth: u32) {
    s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
    s.execute("actions.record", json!({"name": "Red"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let _ = s.execute("document.pixel", json!({"x": 1, "y": 1}));
    let _ = s.execute("actions.list", json!({}));
    s.execute("actions.stop", json!({})).unwrap();
}

#[test]
fn record_keeps_only_replayable_steps() {
    let mut s = Session::new();
    record_red(&mut s, 8);
    let got = s.execute("actions.get", json!({"action": "Red"})).unwrap();
    let ids: Vec<&str> = got["steps"].as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
    assert_eq!(ids, ["layer.new.layer", "select.rect", "edit.fill"]);
    assert!(s.journal.iter().all(|(id, _)| !id.starts_with("actions.")));
    let listed = s.execute("actions.list", json!({})).unwrap();
    assert_eq!(listed["actions"][0]["steps"], 3);
    assert!(listed["recording"].is_null());
}

#[test]
/// Playback on a different document must match native edits at every depth, independent of history grouping.
fn play_on_another_document_matches_pixels_at_each_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        // This control checks the optional, per-step history behavior.
        s.execute("actions.setHistory", json!({"singleState": false})).unwrap();
        record_red(&mut s, depth);
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
        let before = s.active().unwrap().history.past_len();
        let r = s.execute("actions.play", json!({"action": 0})).unwrap();
        assert!(r.get("failed").is_none(), "depth {depth}: {r}");
        assert_eq!(r["ran"], 3, "depth {depth}");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "depth {depth}");
        assert_eq!(s.active().unwrap().history.past_len(), before + 3, "depth {depth}: one history step per played step");
        let px = pixel(&s, 1, 5, 5);
        assert!(px.iter().zip([1.0, 0.0, 0.0, 1.0]).all(|(a, b)| (a - b).abs() < 1.0e-5), "depth {depth}: {px:?}");
        assert_eq!(s.recording.playing, 0);
    }
}

#[test]
fn recorded_layer_drag_replays_on_another_document() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        s.execute("actions.record", json!({"name": "Retouch"})).unwrap();
        s.execute("layer.stampVisible", json!({})).unwrap();
        let layer1 = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let layer2 = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": layer1.0, "mode": "replace"})).unwrap();
        s.execute("filter.noise.median", json!({"radius": 6})).unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let copy = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.moveTo", json!({"layer": copy.0, "target": layer2.0, "position": "above"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let ActionStep { command: id, params: step } = s.actions().list.actions()[0].steps.last().unwrap();
        assert_eq!((id.as_str(), step), ("layer.moveTo", &json!({"target": "Layer 1 copy", "position": "above"})));
        let names = |s: &Session| s.active().unwrap().doc.layers.iter().rev().map(|l| l.name.clone()).collect::<Vec<_>>();
        let recorded = names(&s);
        let saved = s.actions().list.to_file().to_json().unwrap();
        let mut replay = Session::new();
        replay.actions_mut().list = ActionList::from_file(ActionsFile::from_json(&saved).unwrap());
        replay.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
        assert!(result.get("failed").is_none(), "{depth}: {result}");
        assert_eq!(names(&replay), recorded, "{depth}: the same stack");
    }
    // Several layers of the drop target's name: the recorded occurrence picks the right one.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
    let upper = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Mover"})).unwrap();
    let mover = s.active().unwrap().active_layer.unwrap();
    s.execute("actions.record", json!({"name": "Drop"})).unwrap();
    s.execute("layer.moveTo", json!({"layer": mover.0, "target": upper.0, "position": "below"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions().list.actions()[0].steps[0].params, json!({"target": "Twin", "targetOccurrence": 1, "position": "below"}));
    // The moved layer shares the target's name: the occurrence counts the other "Twin"s only,
    // so it means the same layer before the move (on replay) as after it (when recorded).
    let build = |s: &mut Session| {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        for _ in 0..3 {
            s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        }
        s.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>()
    };
    let mut s = Session::new();
    let ids = build(&mut s);
    let (lowest, top) = (ids[1], ids[3]);
    s.execute("actions.record", json!({"name": "Twins"})).unwrap();
    s.execute("layer.moveTo", json!({"layer": top.0, "target": lowest.0, "position": "below"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions().list.actions()[0].steps[0].params, json!({"target": "Twin", "targetOccurrence": 0, "position": "below"}));
    let order = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    let want: Vec<_> = order(&s).iter().map(|id| ids.iter().position(|x| x == id).unwrap()).collect();
    let saved = s.actions().list.to_file().to_json().unwrap();
    let mut replay = Session::new();
    replay.actions_mut().list = ActionList::from_file(ActionsFile::from_json(&saved).unwrap());
    let ids2 = build(&mut replay);
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let got: Vec<_> = order(&replay).iter().map(|id| ids2.iter().position(|x| x == id).unwrap()).collect();
    assert_eq!(got, want, "the same layer became the lowest");
    // A moved group holding a same-named layer: its contents don't count either, or moving the
    // group past another "Twin" would shift the occurrence between recording and replay.
    let build_group = |s: &mut Session| {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        let top = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": group.0, "mode": "replace"})).unwrap();
        (group, top)
    };
    let mut g = Session::new();
    let (group, top) = build_group(&mut g);
    g.execute("actions.record", json!({"name": "Group"})).unwrap();
    g.execute("layer.moveTo", json!({"layer": group.0, "target": top.0, "position": "above"})).unwrap();
    g.execute("actions.stop", json!({})).unwrap();
    let recorded = g.actions().list.actions()[0].steps[0].params.clone();
    let saved = g.actions().list.to_file().to_json().unwrap();
    let mut replay = Session::new();
    replay.actions_mut().list = ActionList::from_file(ActionsFile::from_json(&saved).unwrap());
    let (group2, _) = build_group(&mut replay);
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{recorded}: {result}");
    let top_layer = |s: &Session| s.active().unwrap().doc.layers.last().map(|l| l.id);
    assert_eq!(top_layer(&replay), Some(group2), "{recorded}: the group went to the top");
    // Several moved layers named like the target are left out too, so the name is unambiguous.
    let mut m = Session::new();
    m.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let a = photocraft_doc::LayerId(m.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
    let b = photocraft_doc::LayerId(m.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
    let target = photocraft_doc::LayerId(m.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
    m.execute("layer.moveTo", json!({"layers": [a.0, b.0], "target": "Twin", "position": "above"})).unwrap();
    assert_eq!(m.active().unwrap().doc.layers.last().map(|l| l.id), Some(b), "both moved above the third Twin");
    assert_eq!(m.active().unwrap().doc.layers.iter().rev().nth(2).map(|l| l.id), Some(target));
    // An ⌥-drag copy of the selected layer beside itself replays by name too.
    let mut c = Session::new();
    c.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let src = photocraft_doc::LayerId(c.execute("layer.new.layer", json!({"name": "Solo"})).unwrap()["layer"].as_u64().unwrap());
    c.execute("actions.record", json!({"name": "Copy"})).unwrap();
    c.execute("layer.moveTo", json!({"layer": src.0, "target": src.0, "position": "above", "copy": true})).unwrap();
    c.execute("actions.stop", json!({})).unwrap();
    assert_eq!(c.actions().list.actions()[0].steps[0].params, json!({"target": "Solo", "position": "above", "copy": true}));
    let saved = c.actions().list.to_file().to_json().unwrap();
    let mut replay = Session::new();
    replay.actions_mut().list = ActionList::from_file(ActionsFile::from_json(&saved).unwrap());
    replay.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    replay.execute("layer.new.layer", json!({"name": "Solo"})).unwrap();
    let before = replay.active().unwrap().doc.layers.len();
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(replay.active().unwrap().doc.layers.len(), before + 1, "one copy");
    // Copying a group holding a layer named like the target: the new copy's contents (made
    // before the step is recorded) don't count, so the occurrence matches the replay.
    let build_copy = |s: &mut Session| {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().active_layer.unwrap();
        let a = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
        let b = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.select", json!({"layer": group.0, "mode": "replace"})).unwrap();
        (group, a, b)
    };
    let mut gc = Session::new();
    let (group, a, _) = build_copy(&mut gc);
    gc.execute("actions.record", json!({"name": "Copy group"})).unwrap();
    // Below `a`: the copy's "Twin" then comes before `a` in document order.
    gc.execute("layer.moveTo", json!({"layer": group.0, "target": a.0, "position": "below", "copy": true})).unwrap();
    gc.execute("actions.stop", json!({})).unwrap();
    let saved = gc.actions().list.to_file().to_json().unwrap();
    let mut replay = Session::new();
    replay.actions_mut().list = ActionList::from_file(ActionsFile::from_json(&saved).unwrap());
    let (_, a2, _) = build_copy(&mut replay);
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let layers = &replay.active().unwrap().doc.layers;
    let at = layers.iter().position(|l| l.id == a2).unwrap();
    let below = at.checked_sub(1).and_then(|i| layers.get(i)).map(|l| l.name.as_str());
    assert_eq!(below, Some("Group 1 copy"), "the copy landed below the same Twin");
    // A drag of several layers keeps its ids (not covered by name).
    s.execute("actions.record", json!({"name": "Several"})).unwrap();
    s.execute("layer.moveTo", json!({"layers": [ids[2].0, ids[3].0], "target": ids[1].0, "position": "above"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions().list.actions()[1].steps[0].params["target"], json!(ids[1].0));
    // Bad targets are errors, not panics.
    for bad in [
        json!({"target": "Nobody"}),
        json!({"target": "Twin"}),
        json!({"target": "Twin", "targetOccurrence": 9}),
        json!({"target": "Twin", "targetOccurrence": -1}),
        json!({"target": true}),
    ] {
        assert!(s.execute("layer.moveTo", bad.clone()).is_err(), "{bad}");
    }
}

/// AUTO-219-4: a recording made on one document replays on a second one — a crop, a filter,
/// a selection fill and styled text all act on the document the replay runs in.
#[test]
fn a_recorded_edit_run_replays_on_a_second_document() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    s.execute("actions.record", json!({"name": "Studio"})).unwrap();
    s.execute("image.crop", json!({"x": 8, "y": 8, "width": 48, "height": 48})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 12, "height": 12})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("type.create", json!({"x": 10, "y": 36, "text": "Hi", "size": 12, "color": "#000000"})).unwrap();
    s.execute("type.setStyle", json!({"fauxBold": true})).unwrap();
    let stopped = s.execute("actions.stop", json!({})).unwrap();
    let steps = stopped["steps"].as_u64().unwrap();
    assert!(steps >= 7, "the recording keeps every family: {stopped}");

    // The second document: a different size, background and name. The replay must act here.
    s.execute("file.new", json!({"width": 100, "height": 80, "background": "black", "name": "Second"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "Studio"})).unwrap();
    assert!(played.get("failed").is_none(), "replay: {played}");
    assert_eq!(played["ran"].as_u64(), Some(steps), "every step ran: {played}");
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (48, 48), "the crop replayed onto the second document");
    assert_eq!(pixel(&s, 0, 16, 16), vec![1.0, 0.0, 0.0, 1.0], "the selection fill replayed");
    assert_eq!(pixel(&s, 0, 2, 2), vec![0.0, 0.0, 0.0, 1.0], "outside the selection stays black");
    let last = d.layers.last().unwrap();
    match &last.content {
        photocraft_doc::LayerContent::Text(t) => {
            assert_eq!(t.text, "Hi", "the styled text replayed");
            assert!(t.char_runs().iter().any(|r| r.style.faux_bold), "the bold run replayed");
        }
        other => panic!("expected the recorded type layer, got {other:?}"),
    }
    // The first document is untouched.
    let first = &s.documents()[0].doc;
    assert_eq!((first.size.width, first.size.height), (48, 48), "the recording document keeps its own crop");
}
