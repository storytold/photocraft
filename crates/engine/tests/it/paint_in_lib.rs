//! Tests that run paint commands, moved from the engine-core `lib` tests: 21 test(s).

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::Value;

use serde_json::json;

fn session_with_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s
}

fn px(s: &mut Session, x: i32, y: i32) -> Vec<f32> {
    serde_json::from_value(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap()).unwrap()
}

#[test]
fn an_edit_after_a_new_fill_layer_on_a_selection_reaches_the_whole_layer() {
    // #8801: the selection that masked a new fill or adjustment layer is gone, so the next
    // pixel edit on another layer is not limited to it.
    for (id, params) in [("layer.newFillLayer.solidColor", json!({"color": "#000000"})), ("layer.newAdjustmentLayer.invert", json!({}))] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#3366cc"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        s.execute(id, params).unwrap();
        s.execute("layer.setProps", json!({"visible": false})).unwrap();
        s.execute("layer.selectBottom", json!({})).unwrap();
        s.execute("layer.duplicate", json!({})).unwrap();
        s.execute("image.adjustments.desaturate", json!({})).unwrap();
        for x in [5, 35] {
            let p = px(&mut s, x, 10);
            assert!((p[0] - p[1]).abs() < 1e-3 && (p[1] - p[2]).abs() < 1e-3, "{id}: ({x}, 10) is not grey: {p:?}");
        }
    }
}

#[test]
fn a_new_adjustment_mask_can_be_painted_immediately() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        assert_eq!(px(&mut s, 20, 20), vec![0.0, 0.0, 0.0, 1.0]);
        s.execute("paint.stroke", json!({"points": [[20, 20]], "size": 10, "hardness": 1.0, "color": "#000000", "target": "mask"})).unwrap();
        assert_eq!(px(&mut s, 20, 20), vec![1.0, 1.0, 1.0, 1.0]);
        assert_eq!(px(&mut s, 2, 2), vec![0.0, 0.0, 0.0, 1.0]);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(px(&mut s, 20, 20), vec![0.0, 0.0, 0.0, 1.0]);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(px(&mut s, 20, 20), vec![1.0, 1.0, 1.0, 1.0]);
    }
}

#[test]
fn threshold_and_hue_params() {
    let mut s = session_with_doc();
    s.execute("edit.fill", json!({"color": "#404040"})).unwrap();
    s.execute("layer.newAdjustmentLayer.threshold", json!({"level": 128})).unwrap();
    assert_eq!(px(&mut s, 0, 0), vec![0.0, 0.0, 0.0, 1.0]);
    s.execute("layer.setAdjustment", json!({"level": 10})).unwrap();
    assert_eq!(px(&mut s, 0, 0), vec![1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn paint_stroke_and_selection() {
    let mut s = session_with_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    let r = s.execute("paint.stroke", json!({"points": [[4, 20], [60, 20]], "size": 6, "color": "#0000ff"})).unwrap();
    assert!(r["damage"][2].as_i64().unwrap() > 0);
    assert_eq!(px(&mut s, 10, 20), vec![0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&mut s, 50, 20), vec![1.0, 1.0, 1.0, 1.0], "outside selection stays white");
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[4, 30], [60, 30]], "size": 6, "erase": false})).unwrap();
    assert_eq!(px(&mut s, 50, 30), vec![0.0, 0.0, 0.0, 1.0], "foreground is black");
}

#[test]
fn undo_and_redo_restore_the_targeted_layers() {
    // #495: each history state brings back the layers it targeted when it was created.
    let mut s = session_with_doc();
    let target = |s: &Session| {
        let st = s.active().unwrap();
        (st.active_layer.unwrap(), st.selected_layers().to_vec())
    };
    let select = |s: &mut Session, id: LayerId, mode: &str| {
        let steps = s.active().unwrap().history.past_len();
        s.execute("layer.select", json!({"layer": id.0, "mode": mode})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps, "selecting is not a step");
    };
    let bg = target(&s).0;
    s.execute("layer.new.layer", json!({})).unwrap();
    let new = target(&s).0;
    assert!(s.undo());
    assert_eq!(target(&s), (bg, vec![bg]), "the new document's target");
    assert!(s.redo());
    assert_eq!(target(&s), (new, vec![new]), "redo targets the new layer again");
    // So Fill paints the new layer, not the background.
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.layer(new).unwrap().surface().unwrap().content_bounds().is_empty());
    let mut v = [0.0f32; 4];
    doc.layer(bg).unwrap().surface().unwrap().read_pixel(5, 5, &mut v);
    assert_eq!(v, [1.0; 4], "the background is untouched");
    // Selecting another layer doesn't change what a state targets: undo returns to New Layer's
    // target and redo to Fill's.
    select(&mut s, bg, "replace");
    assert!(s.undo());
    assert_eq!(target(&s), (new, vec![new]));
    select(&mut s, bg, "replace");
    assert!(s.redo());
    assert_eq!(target(&s), (new, vec![new]), "redo targets the filled layer");
    // Undoing a delete targets the restored layer.
    s.execute("layer.delete", json!({})).unwrap();
    assert_eq!(target(&s).0, bg);
    assert!(s.undo());
    assert_eq!(target(&s), (new, vec![new]));
    // A multi-layer target comes back too: a step taken with both layers selected...
    select(&mut s, bg, "add");
    s.execute("select.all", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(s.undo());
    assert_eq!(target(&s), (bg, vec![bg, new]));
    // ...and the copies a multi-layer duplicate selects after its edit.
    s.execute("layer.duplicate", json!({})).unwrap();
    let copies = target(&s);
    assert_eq!(copies.1.len(), 2);
    select(&mut s, bg, "replace");
    assert!(s.undo());
    assert!(s.redo());
    assert_eq!(target(&s), copies);
}

#[test]
fn fill_clear_and_masks() {
    let mut s = session_with_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    assert_eq!(px(&mut s, 5, 5), vec![0.0, 1.0, 0.0, 1.0]);
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    assert_eq!(px(&mut s, 5, 5), vec![1.0, 1.0, 1.0, 1.0]);
    s.execute("layer.layerMask.delete", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!(px(&mut s, 5, 5), vec![1.0, 1.0, 1.0, 1.0]);
    assert_eq!(px(&mut s, 20, 20), vec![0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn clipping_and_merge_and_flatten() {
    let mut s = session_with_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.createClippingMask", json!({})).unwrap();
    assert_eq!(px(&mut s, 5, 5), vec![0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&mut s, 30, 5), vec![1.0, 1.0, 1.0, 1.0]);
    s.execute("layer.mergeDown", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), 2);
    assert_eq!(px(&mut s, 5, 5), vec![0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&mut s, 30, 5), vec![1.0, 1.0, 1.0, 1.0]);
    s.execute("layer.flattenImage", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), 1);
    assert_eq!(px(&mut s, 5, 5), vec![0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn canvas_flips_and_rotation() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 10, "height": 4})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap();
    assert_eq!(px(&mut s, 9, 0), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&mut s, 0, 0), vec![1.0, 1.0, 1.0, 1.0]);
    s.execute("image.imageRotation.flipCanvasVertical", json!({})).unwrap();
    assert_eq!(px(&mut s, 9, 3), vec![1.0, 0.0, 0.0, 1.0]);
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (4, 10));
    assert_eq!(px(&mut s, 0, 9), vec![1.0, 0.0, 0.0, 1.0]);
    s.execute("image.imageRotation.90ccw", json!({})).unwrap();
    s.execute("image.imageRotation.180", json!({})).unwrap();
    assert_eq!(px(&mut s, 0, 0), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn group_effect_maps_follow_child_visibility_and_opacity() {
    // A styled group's maps come from its children's composite: hiding a child or changing its
    // opacity must refresh the group's shadow. Warm render vs. purged render must agree.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "background": "transparent"})).unwrap();
    let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("paint.pencil", json!({"points": [[20, 24]], "size": 12, "color": "#ff0000"})).unwrap();
    let b = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("paint.pencil", json!({"points": [[44, 24]], "size": 12, "color": "#0000ff"})).unwrap();
    let g = s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    for l in [a, b] {
        s.execute("layer.moveTo", json!({"layer": l, "target": g, "position": "into"})).unwrap();
    }
    s.edit("shadow", |doc, _| {
        doc.layer_mut(LayerId(g)).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let render = |s: &Session| {
        let d = &s.active().unwrap().doc;
        photocraft_compose::render(d, d.bounds()).into_px()
    };
    let mut prev = render(&s); // populate the effect-map cache
    type Edit = (&'static str, fn(&mut photocraft_doc::Layer));
    let edits: [Edit; 2] = [("hide", |l| l.visible = false), ("opacity", |l| l.opacity = 0.3)];
    for ((name, edit), child) in edits.into_iter().zip([a, b]) {
        let child = LayerId(child);
        s.edit(name, |doc, _| {
            edit(doc.layer_mut(child).unwrap());
            Ok(())
        })
        .unwrap();
        let warm = render(&s);
        assert_ne!(warm, prev, "{name}: the composite changes");
        photocraft_compose::purge_effect_cache();
        assert_eq!(warm, render(&s), "{name}: the group's maps follow its children");
        prev = warm;
    }
}

/// With no document open, a document command says so instead of only `enabled: false` (#3664).
#[test]
fn command_list_tells_no_document_from_other_preconditions() {
    let mut s = Session::new();
    let list = s.execute("command.list", json!({})).unwrap();
    let find = |list: &Value, id: &str| list.as_array().unwrap().iter().find(|c| c["id"] == id).unwrap().clone();
    for id in ["paint.stroke", "layer.new.layer", "filter.blur.gaussianBlur"] {
        let c = find(&list, id);
        assert_eq!((c["enabled"].clone(), c["needsDocument"].clone()), (json!(false), json!(true)), "{id}: {c}");
        assert!(c["disabledReason"].as_str().is_some_and(|r| r.starts_with("no document")), "{id}: {c}");
    }
    let new = find(&list, "file.new");
    assert_eq!((new["enabled"].clone(), new["needsDocument"].clone(), new["disabledReason"].clone()), (json!(true), json!(false), Value::Null));
    // A precondition that isn't the missing document keeps its own reason.
    let paste = find(&list, "file.newFromClipboard");
    assert_eq!((paste["enabled"].clone(), paste["needsDocument"].clone()), (json!(false), json!(false)));
    assert_eq!(paste["disabledReason"], "the clipboard is empty");
    // With a document, the same commands run; a missing selection is not a missing document.
    let mut s = session_with_doc();
    let list = s.execute("command.list", json!({})).unwrap();
    assert_eq!(find(&list, "paint.stroke")["needsDocument"], false);
    assert_eq!(find(&list, "paint.stroke")["disabledReason"], Value::Null);
    let deselect = find(&list, "select.deselect");
    assert_eq!((deselect["enabled"].clone(), deselect["needsDocument"].clone()), (json!(false), json!(false)));
    assert_eq!(deselect["disabledReason"], "no selection");
}

#[test]
fn translate_moves_pixels_and_respects_locks() {
    let mut s = session_with_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.translate", json!({"dx": 10, "dy": 5})).unwrap();
    assert_eq!(px(&mut s, 11, 6), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&mut s, 1, 1), vec![1.0, 1.0, 1.0, 1.0]);
    // Background is position-locked
    let bg = s.active().unwrap().doc.layers[0].id;
    s.select_layer(bg).unwrap();
    assert!(s.execute("layer.translate", json!({"dx": 1, "dy": 0})).is_err());
}

#[test]
fn damage_is_reported_for_strokes_only() {
    let mut s = session_with_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    // A new empty layer changes no pixels (#1771).
    assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
    s.execute("paint.stroke", json!({"points": [[10, 10], [20, 10]], "size": 4})).unwrap();
    let d = s.active().unwrap().last_damage.unwrap();
    assert!(d.contains(15, 10) && d.width() < 30);
    // Undo reports the same bounded area back, not a whole-canvas recomposite.
    s.execute("edit.undo", json!({})).unwrap();
    let d = s.active().unwrap().last_damage.unwrap();
    assert!(d.contains(15, 10) && d.width() < 30);
}

#[test]
fn edits_that_change_no_pixels_report_empty_damage() {
    // A new selection must not recomposite the canvas (#1773: seconds on large documents).
    let mut s = session_with_doc();
    let points = json!([[2, 2], [40, 3], [30, 30], [5, 25]]);
    s.execute("select.lasso", json!({"points": points, "mode": "replace"})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
    s.execute("select.inverse", json!({})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
    s.execute("select.deselect", json!({})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
    // A new empty layer draws nothing either (#1771); filling it does recomposite.
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert_ne!(s.active().unwrap().last_damage, Some(photocraft_geom::Rect::EMPTY));
}

#[test]
fn same_pixels_ignores_the_selection_but_not_layers() {
    let mut s = session_with_doc();
    let a = s.active().unwrap().doc.clone();
    s.execute("select.all", json!({})).unwrap();
    let b = s.active().unwrap().doc.clone();
    assert!(photocraft_engine::layer_multi_cmds::same_pixels(&a, &b));
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert!(!photocraft_engine::layer_multi_cmds::same_pixels(&b, &s.active().unwrap().doc));
    // A pathologically deep group nest is treated as changed (bounded walk), never a crash.
    let mut deep = (*b).clone();
    let mut l = photocraft_doc::Layer::group("g", Vec::new());
    for _ in 0..300 {
        l = photocraft_doc::Layer::group("g", vec![l]);
    }
    deep.layers.push(l);
    assert!(!photocraft_engine::layer_multi_cmds::same_pixels(&deep, &deep.clone()));
}

/// ⌥-dragging a Layers panel row: `copy` leaves the layer where it is and drops a duplicate at the
/// target, as one "Duplicate Layer" step, with the copy active.
#[test]
fn move_to_copy_places_a_duplicate_in_one_step() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap();
    let g = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    let names = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    let steps = |s: &Session| s.active().unwrap().history.past_len();
    let before = steps(&s);
    // bottom->top: Background, A, B, G ; copy A above B
    let r = s.execute("layer.moveTo", json!({"layer": a, "target": b, "position": "above", "copy": true})).unwrap();
    let copy = r["layer"].as_u64().expect("the copy's id");
    assert_ne!(copy, a);
    assert_eq!(names(&s), ["Background", "A", "B", "A copy", "G"]);
    assert_eq!(steps(&s), before + 1, "one history step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Duplicate Layer"));
    assert_eq!(s.active().unwrap().active_layer, Some(photocraft_doc::LayerId(copy)), "the copy is active");
    let doc = &s.active().unwrap().doc;
    let (orig, dup) = (doc.layer(photocraft_doc::LayerId(a)).unwrap(), doc.layer(photocraft_doc::LayerId(copy)).unwrap());
    assert_eq!(orig.content, dup.content, "the pixels are copied");
    // Below and into a group.
    s.execute("layer.moveTo", json!({"layer": b, "target": a, "position": "below", "copy": true})).unwrap();
    assert_eq!(names(&s), ["Background", "B copy", "A", "B", "A copy", "G"]);
    s.execute("layer.moveTo", json!({"layer": a, "target": g, "position": "into", "copy": true})).unwrap();
    // `document.inspect` lists the top of the stack first; "A copy" is taken, so this is "A copy 2".
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["layers"][0]["children"][0]["name"], "A copy 2");
    // A group can be copied beside itself, but not into itself.
    s.execute("layer.moveTo", json!({"layer": g, "target": g, "position": "above", "copy": true})).unwrap();
    assert_eq!(names(&s).last().map(String::as_str), Some("G copy"));
    let past = steps(&s);
    assert!(s.execute("layer.moveTo", json!({"layer": g, "target": g, "position": "into", "copy": true})).is_err());
    assert_eq!(steps(&s), past, "a refused copy records nothing");
    // A copy of the Background is an ordinary, unlocked layer.
    let bg = s.active().unwrap().doc.layers[0].id.0;
    s.execute("layer.moveTo", json!({"layer": bg, "target": b, "position": "above", "copy": true})).unwrap();
    let doc = &s.active().unwrap().doc;
    let bg_copy = doc.layers.iter().find(|l| l.name == "Background copy").expect("a Background copy");
    assert!(!bg_copy.locks.transparency);
    assert!(doc.layers[0].locks.transparency, "the Background keeps its lock");
    // Undo removes each copy and nothing else.
    for _ in 0..5 {
        s.execute("edit.undo", json!({})).unwrap();
    }
    assert_eq!(names(&s), ["Background", "A", "B", "G"]);
}

/// #2259: the commands behind the arrow-key nudges take the `coalesce` key, so a run of them is
/// one step an agent can drive too, and one Undo takes the whole run back.
#[test]
fn translate_and_transform_selection_coalesce_into_one_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 60, "height": 60})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let steps = |s: &Session| s.active().unwrap().history.past_len();
    let left = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds().x0
    };
    let sel = |s: &Session| s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds().x0;

    let base = steps(&s);
    for _ in 0..5 {
        s.execute("layer.translate", json!({"dx": 1, "dy": 0, "coalesce": "nudge:1"})).unwrap();
    }
    assert_eq!((steps(&s), left(&s)), (base + 1, 15), "five nudges, one step");
    s.execute("layer.translate", json!({"dx": 1, "dy": 0, "coalesce": "nudge:2"})).unwrap();
    s.execute("layer.translate", json!({"dx": 1, "dy": 0})).unwrap();
    s.execute("layer.translate", json!({"dx": 1, "dy": 0, "coalesce": "nudge:2"})).unwrap();
    assert_eq!(steps(&s), base + 4, "another key, an uncoalesced move and a repeated key each start a step");
    for _ in 0..3 {
        assert!(s.undo());
    }
    assert_eq!(left(&s), 15);
    assert!(s.undo());
    assert_eq!(left(&s), 10, "Undo takes the run of five back at once");

    let base = steps(&s);
    for _ in 0..4 {
        s.execute("select.transformSelection", json!({"dx": 1, "dy": 0, "coalesce": "nudge:3"})).unwrap();
    }
    assert_eq!((steps(&s), sel(&s)), (base + 1, 14));
    assert!(s.undo());
    assert_eq!(sel(&s), 10);
}

#[test]
fn painting_can_target_the_layer_mask() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("mask", |doc, _| {
        doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();
    // Adjustment layers are paintable through their mask only.
    assert!(s.execute("paint.stroke", json!({"points": [[20, 20]], "size": 10, "hardness": 1.0, "color": "#000000"})).is_err());
    s.execute("paint.stroke", json!({"points": [[20, 20]], "size": 10, "hardness": 1.0, "color": "#000000", "target": "mask"})).unwrap();
    let m = |s: &Session, x, y| s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.pixel(x, y)[0];
    assert!(m(&s, 20, 20) < 0.01, "painted black");
    assert!(m(&s, 2, 2) > 0.99, "rest still revealed");
    // Eraser on a mask paints the background colour (white).
    s.execute("paint.stroke", json!({"points": [[20, 20]], "size": 10, "hardness": 1.0, "erase": true, "target": "mask"})).unwrap();
    assert!(m(&s, 20, 20) > 0.99);
    // Gradient into the mask.
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [40, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    assert!(m(&s, 2, 20) < 0.1 && m(&s, 38, 20) > 0.9);
}

#[test]
fn layer_locks_are_set_and_enforced() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    assert!(s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4})).is_err(), "pixel lock blocks painting");
    s.execute("layer.setProps", json!({"locks": {"pixels": false, "position": true}})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4})).unwrap();
    assert!(s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 3, 0]})).is_err(), "position lock blocks transforms");
    assert!(s.execute("layer.setProps", json!({"locks": {"bogus": true}})).is_err());
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(l.locks.position && !l.locks.pixels);
}

#[test]
fn all_lock_blocks_painting_a_mask_but_pixels_lock_does_not() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("mask", |doc, _| {
        doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();

    s.execute("layer.setProps", json!({"locks": {"all": true}})).unwrap();
    let history_steps = s.active().unwrap().history.entries().len();
    let err = s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4, "hardness": 1.0, "color": "#000000", "target": "mask"})).unwrap_err();
    assert_eq!(err.to_string(), "Could not complete your request because the layer \"Layer 1\" is locked");
    let state = s.active().unwrap();
    let mask = &state.doc.layer(id).unwrap().mask.as_ref().unwrap().surface;
    assert!(mask.pixel(5, 5)[0] > 0.99, "rejected stroke leaves the mask reveal-all");
    assert_eq!(state.history.entries().len(), history_steps, "rejected stroke adds no Brush Tool history step");

    s.execute("layer.setProps", json!({"locks": {"all": false, "pixels": true}})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4, "hardness": 1.0, "color": "#000000", "target": "mask"})).unwrap();
    let mask_value = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.pixel(5, 5)[0];
    assert!(mask_value(&s) < 0.01, "pixels-only lock permits mask painting");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(mask_value(&s) > 0.99, "undo restores the mask");
}

#[test]
fn merge_down_on_a_group_merges_the_group() {
    // #3832: with a group selected, Merge Down is Photoshop's Merge Group (⌘E): the group's
    // contents become one layer in its place, named after it, and the layer below is untouched.
    let mut s = session_with_doc();
    let below = s.execute("layer.new.layer", json!({"name": "Below"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 0, "width": 10, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.select", json!({"layer": b})).unwrap();
    s.execute("layer.select", json!({"layer": a, "mode": "add"})).unwrap();
    let group = s.execute("layer.groupLayers", json!({"name": "Badge"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": group, "opacity": 0.5})).ok();
    let before = [px(&mut s, 5, 5), px(&mut s, 25, 5), px(&mut s, 50, 5)];
    let layers = s.active().unwrap().doc.layer_count();
    s.execute("layer.select", json!({"layer": group})).unwrap();
    s.execute("layer.mergeLayers", json!({})).unwrap();
    let st = s.active().unwrap();
    let merged = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    assert_eq!(merged.name, "Badge");
    assert!(matches!(merged.content, photocraft_doc::LayerContent::Raster(_)), "one pixel layer");
    assert!(st.doc.layer(photocraft_doc::LayerId(below)).is_some(), "the layer below stays");
    assert_eq!(st.doc.layer_count(), layers - 2, "group and its two layers became one");
    let after = [px(&mut s, 5, 5), px(&mut s, 25, 5), px(&mut s, 50, 5)];
    for (a, b) in before.iter().zip(&after) {
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0 / 255.0), "{before:?} -> {after:?}");
    }
    // Undo brings the group back.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(group)).is_some());
}

#[test]
fn sessions_share_builtin_brushes_until_edited() {
    let mut first = Session::new();
    let second = Session::new();
    assert!(std::sync::Arc::ptr_eq(&first.tools.presets, &second.tools.presets));
    let count = second.tools.presets.len();
    first.execute("brush.presets.save", serde_json::json!({"name": "Session-only test brush"})).unwrap();
    assert_eq!(second.tools.presets.len(), count);
    assert!(second.tools.presets.iter().all(|preset| preset.name != "Session-only test brush"));
    assert!(first.tools.presets.iter().any(|preset| preset.name == "Session-only test brush"));
    assert!(!std::sync::Arc::ptr_eq(&first.tools.presets, &second.tools.presets));
}
