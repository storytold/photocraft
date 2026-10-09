use super::*;
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
fn document_pixel_at_the_coordinate_limits() {
    let mut s = session_with_doc();
    // The last representable column used to panic (its 1x1 rect saturated to empty).
    assert_eq!(px(&mut s, i32::MAX, 0), vec![0.0; 4]);
    assert_eq!(px(&mut s, i32::MIN, i32::MAX), vec![0.0; 4]);
    // Values beyond i32 used to wrap around onto the canvas (2^32 read column 0).
    let e = s.execute("document.pixel", json!({"x": 1i64 << 32, "y": 0})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
}

#[test]
fn file_new_at_the_size_limit_does_not_fill_every_tile() {
    // A 300000² white background used to fill ~360 GB of tiles before returning (#705).
    for background in ["white", "black", "backgroundColor", "#336699"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300_000, "height": 300_000, "background": background})).unwrap();
        assert_eq!(px(&mut s, 299_999, 299_999)[3], 1.0, "{background}");
        assert_eq!(px(&mut s, 0, 0)[3], 1.0, "{background}");
    }
}

#[test]
fn command_ids_are_unique_and_documented() {
    let mut seen = std::collections::HashSet::new();
    for c in command_specs() {
        assert!(seen.insert(c.id), "duplicate command id {}", c.id);
        assert!(!c.label.is_empty());
        assert!(c.id.contains('.'), "{} should be namespaced", c.id);
    }
    assert!(command_specs().len() > 60, "{}", command_specs().len());
}

#[test]
fn unknown_and_disabled_commands_error() {
    let mut s = Session::new();
    assert!(matches!(s.execute("nope.nothing", json!({})), Err(EngineError::UnknownCommand(_))));
    assert!(matches!(s.execute("layer.new.layer", json!({})), Err(EngineError::Disabled(..))));
    assert!(!s.is_enabled("edit.undo"));
}

#[test]
fn file_new_takes_whole_floats_and_survives_odd_sizes() {
    // #254: the New dialog sent `512.0`; it must not fall back to 1920 x 1080.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 512.0, "height": 511.6})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (512, 512));
    s.execute("file.new", json!({"width": -5.0, "height": "x"})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (1, 1080));
}

#[test]
fn file_new_variants() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 10, "height": 5, "background": "transparent"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].name, "Layer 1");
    s.execute("file.new", json!({"width": 10, "height": 5, "mode": "cmyk", "depth": 16})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.mode, photocraft_color::ColorMode::Cmyk);
    assert_eq!(d.depth, photocraft_color::SampleType::U16);
    s.execute("file.new", json!({"background": "#ff0000", "width": 4, "height": 4})).unwrap();
    assert_eq!(px(&mut s, 1, 1), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.documents().len(), 3);
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(s.documents().len(), 2);
}

#[test]
fn layer_lifecycle_with_undo() {
    let mut s = session_with_doc();
    let r = s.execute("layer.new.layer", json!({})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    assert_eq!(s.active().unwrap().active_layer, Some(LayerId(id)));
    s.execute("layer.setProps", json!({"name": "Ink", "opacity": 0.5, "blend": "multiply"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let l = doc.layer(LayerId(id)).unwrap();
    assert_eq!((l.name.as_str(), l.opacity, l.blend), ("Ink", 0.5, photocraft_color::BlendMode::Multiply));
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), 3);
    s.execute("layer.delete", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), 2);
    for _ in 0..4 {
        s.execute("edit.undo", json!({})).unwrap();
    }
    assert_eq!(s.active().unwrap().doc.layer_count(), 1);
    assert!(!s.is_enabled("edit.undo"));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), 2);
}

#[test]
fn bad_blend_mode_is_a_param_error() {
    let mut s = session_with_doc();
    let e = s.execute("layer.setProps", json!({"blend": "sparkle"})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
}

#[test]
fn adjustment_layers_change_composite() {
    let mut s = session_with_doc();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    assert_eq!(px(&mut s, 3, 3), vec![0.0, 0.0, 0.0, 1.0]);
    let doc = s.execute("document.inspect", json!({})).unwrap();
    assert_eq!(doc["layers"][0]["kind"], "Adjustment");
    assert_eq!(doc["layers"][0]["name"], "Invert 1");
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    assert_eq!(px(&mut s, 3, 3), vec![1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn new_adjustment_and_fill_layers_are_masked_by_the_selection() {
    // #1250: as in Photoshop, a new adjustment or fill layer made while a selection is active
    // gets the selection as its layer mask, so it acts on the selected area only.
    for depth in [8, 16, 32] {
        // (command, params, whether it turns the white canvas black inside the selection)
        for (id, params, blackens) in [
            ("layer.newAdjustmentLayer.invert", json!({}), true),
            ("layer.newAdjustmentLayer.hueSaturation", json!({"hue": 40, "saturation": 20}), false),
            ("layer.newFillLayer.solidColor", json!({"color": "#000000"}), true),
            ("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#000000"}), true),
            ("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"}), false),
        ] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
            s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
            let layer = LayerId(s.execute(id, params).unwrap()["layer"].as_u64().unwrap());
            let d = s.active().unwrap();
            let mask = d.doc.layer(layer).unwrap().mask.as_ref().unwrap_or_else(|| panic!("{id} at {depth}-bit: no layer mask"));
            assert_eq!(mask.value(10, 10), 1.0, "{id} at {depth}-bit: the mask hides the selected area");
            assert_eq!(mask.value(2, 2), 0.0, "{id} at {depth}-bit: the mask reveals outside the selection");
            assert!(mask.linked && mask.enabled, "{id}");
            // The selection stays, as after Layer › Layer Mask › Reveal Selection.
            assert!(d.doc.selection.is_some(), "{id} dropped the selection");
            // The white canvas is untouched outside the selection.
            assert_eq!(px(&mut s, 2, 2), vec![1.0, 1.0, 1.0, 1.0], "{id} at {depth}-bit changed pixels outside the selection");
            if blackens {
                assert_eq!(px(&mut s, 10, 10)[..3], [0.0, 0.0, 0.0][..], "{id} at {depth}-bit: no effect inside the selection");
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert!(s.active().unwrap().doc.layer(layer).is_none(), "{id}: undo keeps the layer");
        }
    }
}

#[test]
fn new_adjustment_and_fill_layers_have_no_mask_without_a_selection() {
    let mut s = session_with_doc();
    for (id, params) in [
        ("layer.newAdjustmentLayer.invert", json!({})),
        ("layer.newFillLayer.solidColor", json!({"color": "#000000"})),
        ("layer.newFillLayer.gradient", json!({})),
        ("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"})),
    ] {
        let layer = LayerId(s.execute(id, params).unwrap()["layer"].as_u64().unwrap());
        assert!(s.active().unwrap().doc.layer(layer).unwrap().mask.is_none(), "{id} got a mask with no selection");
    }
}

#[test]
fn new_fill_and_adjustment_layers_take_the_active_path_as_their_vector_mask() {
    // #1419: as in Photoshop, the path selected in the Paths panel becomes the vector mask of a
    // new fill or adjustment layer (a Solid Color fill becomes a shape) and wins over a selection.
    let square = json!({"subpaths": [{"closed": true, "knots": [[8, 8], [24, 8], [24, 24], [8, 24]]}]});
    for depth in [8, 16, 32] {
        for (id, params, blackens) in [
            ("layer.newFillLayer.solidColor", json!({"color": "#000000"}), true),
            ("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#000000"}), true),
            ("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"}), false),
            ("layer.newAdjustmentLayer.invert", json!({}), true),
        ] {
            for (path_name, setup) in [("work", "work"), ("Saved", "Saved")] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
                s.execute("path.set", json!({"name": setup, "path": square})).unwrap();
                // A selection elsewhere: the path wins, as in Photoshop.
                s.execute("select.rect", json!({"x": 40, "y": 30, "width": 8, "height": 8})).unwrap();
                let mut p = params.clone();
                p["path"] = json!(path_name);
                let layer = LayerId(s.execute(id, p).unwrap()["layer"].as_u64().unwrap());
                let d = s.active().unwrap();
                let l = d.doc.layer(layer).unwrap();
                let vm = l.vector_mask.as_ref().unwrap_or_else(|| panic!("{id} ({path_name}) at {depth}-bit: no vector mask"));
                let want = if path_name == "work" { d.doc.work_path.clone().unwrap() } else { d.doc.paths[0].path.clone() };
                assert_eq!(vm.path, want, "{id}: the vector mask is the path");
                assert!(l.mask.is_none(), "{id}: the path wins over the selection");
                for (x, y) in [(2, 2), (42, 32), (30, 30)] {
                    assert_eq!(px(&mut s, x, y), vec![1.0, 1.0, 1.0, 1.0], "{id} ({path_name}) at {depth}-bit changed pixels outside the path at ({x}, {y})");
                }
                if blackens {
                    assert_eq!(px(&mut s, 16, 16)[..3], [0.0, 0.0, 0.0][..], "{id} ({path_name}) at {depth}-bit: no effect inside the path");
                }
                s.execute("edit.undo", json!({})).unwrap();
                assert!(s.active().unwrap().doc.layer(layer).is_none(), "{id}: undo keeps the layer");
            }
        }
    }
}

#[test]
fn new_fill_layer_with_a_missing_path_fails_without_a_layer() {
    let mut s = session_with_doc();
    let before = s.active().unwrap().doc.layers.len();
    for (id, params) in [
        ("layer.newFillLayer.solidColor", json!({"path": "work"})),
        ("layer.newFillLayer.solidColor", json!({"path": "Nope"})),
        ("layer.newFillLayer.gradient", json!({"path": "Nope"})),
        ("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard", "path": "Nope"})),
        ("layer.newAdjustmentLayer.invert", json!({"path": "Nope"})),
        ("layer.newFillLayer.solidColor", json!({"path": 42})),
        ("layer.newFillLayer.solidColor", json!({"path": {"subpaths": "x"}})),
    ] {
        assert!(s.execute(id, params.clone()).is_err(), "{id} {params}: no error");
        assert_eq!(s.active().unwrap().doc.layers.len(), before, "{id} {params}: added a layer");
    }
}

#[test]
fn every_adjustment_command_runs() {
    let mut s = session_with_doc();
    let ids: Vec<&str> =
        command_specs().iter().map(|c| c.id).filter(|id| id.starts_with("layer.newAdjustmentLayer.") || id.starts_with("image.adjustments.")).collect();
    assert!(ids.len() >= 28);
    for id in ids {
        // re-select the background for destructive ones
        let bg = s.active().unwrap().doc.layers[0].id;
        s.select_layer(bg).unwrap();
        s.execute(id, json!({})).unwrap_or_else(|e| panic!("{id}: {e}"));
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
fn marquee_dragged_past_the_canvas_stops_at_its_edge() {
    let mut s = session_with_doc(); // 64 × 48
    let bounds = |s: &mut Session| s.execute("document.inspect", json!({})).unwrap()["selectionBounds"].clone();
    s.execute("select.rect", json!({"x": -20, "y": 10, "width": 200, "height": 100})).unwrap();
    assert_eq!(bounds(&mut s), json!([0, 10, 64, 38]));
    s.execute("select.rect", json!({"x": -30, "y": -30, "width": 200, "height": 200, "ellipse": true})).unwrap();
    assert_eq!(bounds(&mut s), json!([0, 0, 64, 48]));
    // Entirely off the canvas: nothing is selected.
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap();
    assert_eq!(bounds(&mut s), Value::Null);
}

#[test]
fn marquee_steps_are_named_after_their_tool() {
    // #513: the elliptical marquee shares `select.rect` but records its own step name.
    let mut s = session_with_doc();
    let last = |s: &Session| s.active().unwrap().history.undo_label().map(str::to_string);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 10, "ellipse": true})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Elliptical Marquee"));
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    assert_eq!(last(&s).as_deref(), Some("Rectangular Marquee"));
}

#[test]
fn undo_and_redo_restore_the_targeted_layers() {
    // #495: each history state brings back the layers it targeted when it was created.
    let mut s = session_with_doc();
    let target = |s: &Session| {
        let st = s.active().unwrap();
        (st.active_layer.unwrap(), st.selected_layers())
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
fn selection_modes() {
    let mut s = session_with_doc();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 0, "width": 10, "height": 10, "mode": "add"})).unwrap();
    let b = s.execute("document.inspect", json!({})).unwrap()["selectionBounds"].clone();
    assert_eq!(b, json!([0, 0, 30, 10]));
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10, "mode": "subtract"})).unwrap();
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["selectionBounds"], json!([20, 0, 10, 10]));
    s.execute("select.inverse", json!({})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["selectionBounds"], json!([0, 0, 64, 48]));
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20, "ellipse": true})).unwrap();
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
fn clipping_masks_apply_to_every_selected_layer() {
    // #1248: with several layers selected, Create Clipping Mask clips all but the lowest (the
    // base) and Release releases them all, each as one history step.
    let mut s = session_with_doc();
    let ids: Vec<u64> = ["A", "B", "C"].iter().map(|n| s.execute("layer.new.layer", json!({"name": n})).unwrap()["layer"].as_u64().unwrap()).collect();
    s.execute("layer.select", json!({"layer": ids[0]})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id, "mode": "add"})).unwrap();
    }
    let clipped = |s: &Session| ids.iter().map(|id| s.active().unwrap().doc.layer(LayerId(*id)).unwrap().clipped).collect::<Vec<_>>();
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("layer.createClippingMask", json!({})).unwrap();
    assert_eq!(r["layers"], json!([ids[1], ids[2]]));
    assert_eq!(clipped(&s), [false, true, true], "the lowest selected layer is the base");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    s.execute("layer.releaseClippingMask", json!({})).unwrap();
    assert_eq!(clipped(&s), [false, false, false]);
    s.undo();
    assert_eq!(clipped(&s), [false, true, true], "release is one step");
    // A `layer` param still acts on that layer alone.
    s.execute("layer.releaseClippingMask", json!({"layer": ids[2]})).unwrap();
    assert_eq!(clipped(&s), [false, true, false]);
    assert!(s.execute("layer.createClippingMask", json!({"layer": 999_999})).is_err());
}

#[test]
fn arrange_and_group() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.arrange.bringToFront", json!({})).unwrap();
    let names: Vec<String> = s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Background", "B", "A"]);
    assert!(s.execute("layer.arrange.bringForward", json!({})).is_err());
    s.execute("layer.groupLayers", json!({})).unwrap();
    let doc = s.execute("document.inspect", json!({})).unwrap();
    assert_eq!(doc["layers"][0]["kind"], "Group");
    assert_eq!(doc["layers"][0]["children"][0]["name"], "A");
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
fn image_rotation_rotates_every_layer_not_just_the_active_one() {
    // Issue #1: Image Rotation must rotate the whole canvas, not only the selected layer.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "background": "white"})).unwrap();
    // A second (lower) layer with a red dot at the top-left; keep a third layer active.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("dot", |doc, a| {
        doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(2, 2, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let dotted = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap(); // a different, empty active layer
    assert_ne!(s.active().unwrap().active_layer.unwrap(), dotted, "active layer is not the dotted one");

    s.execute("image.imageRotation.180", json!({})).unwrap();

    // The non-active layer's content rotated too: (2,2)..(4,4) in 20x10 → (16,6)..(18,8).
    let st = s.active().unwrap();
    let surf = st.doc.layer(dotted).unwrap().surface().unwrap();
    assert_eq!(surf.rgba(17, 6), [1.0, 0.0, 0.0, 1.0], "dot moved to the opposite corner");
    assert_eq!(surf.rgba(3, 3)[3], 0.0, "original spot is now empty");
    // The Background layer rotated as well (still fully opaque white everywhere).
    assert_eq!(st.doc.layers[0].surface().unwrap().rgba(1, 1), [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn fill_layers_and_colors() {
    let mut s = session_with_doc();
    s.execute("tools.setColors", json!({"foreground": "#112233"})).unwrap();
    s.execute("tools.swapColors", json!({})).unwrap();
    assert_eq!(s.tools.foreground, [1.0, 1.0, 1.0, 1.0]);
    s.execute("tools.defaultColors", json!({})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#ff00ff"})).unwrap();
    assert_eq!(px(&mut s, 1, 1), vec![1.0, 0.0, 1.0, 1.0]);
    s.execute("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#ffffff", "angle": 0})).unwrap();
    let l = px(&mut s, 0, 10)[0];
    let r = px(&mut s, 63, 10)[0];
    assert!(l < r);
}

#[test]
fn fill_layer_pixel_rewrites_refresh_the_effect_maps() {
    // A pixel-only rewrite of a fill layer's cache (what Image › Transform and friends do)
    // leaves the Fill spec alone; the CPU effect maps must follow the cache surface, or the
    // composite keeps the pre-rewrite drop shadow. Warm render vs. purged render must agree.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#ffffff", "angle": 0})).unwrap();
    s.edit("shadow", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let render = |s: &Session| {
        let d = &s.active().unwrap().doc;
        photocraft_compose::render(d, d.bounds()).px
    };
    // Give the layer the cached-pixels state a PSD import produces (the cache is what Image
    // › Transform & friends rewrite in place, without touching the Fill spec).
    s.edit("cache", |doc, active| {
        let fmt = doc.pixel_format();
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Fill(f) = &l.content else { return Err(EngineError::Other("not a fill layer".into())) };
        let fill = f.clone();
        let mut surface = photocraft_raster::Surface::new(fmt);
        surface.fill_rect(photocraft_geom::Rect::new(8, 8, 30, 40), &[0.0, 0.0, 1.0, 1.0]);
        l.fill_cache = Some(photocraft_doc::FillCache { fill, surface });
        Ok(())
    })
    .unwrap();
    let warm = render(&s); // populate the effect-map cache
    let warm2 = render(&s);
    assert_eq!(warm, warm2, "a warm cache does not change the composite");
    // Rewrite the cached pixels in place, without touching the Fill spec.
    s.edit("shift", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let fc = l.fill_cache.as_mut().unwrap();
        let moved = photocraft_algo::resample::translate_surface(&fc.surface, 12, 0);
        fc.surface = moved;
        Ok(())
    })
    .unwrap();
    let after = render(&s);
    assert_ne!(after, warm, "the rewritten cache changes the composite");
    photocraft_compose::purge_effect_cache();
    let fresh = render(&s);
    assert_eq!(after, fresh, "the effect maps track the cache surface, not the Fill spec alone");
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
        photocraft_compose::render(d, d.bounds()).px
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

#[test]
fn journal_records_mutations_only() {
    let mut s = session_with_doc();
    s.execute("document.inspect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "x"})).unwrap();
    let ids: Vec<&str> = s.journal.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["file.new", "layer.new.layer"]);
    // replaying the journal reproduces the document
    let mut replay = Session::new();
    for (id, p) in s.journal.clone() {
        replay.execute(&id, p).unwrap();
    }
    assert_eq!(replay.active().unwrap().doc.layer_count(), s.active().unwrap().doc.layer_count());
}

#[test]
fn command_list_reports_enablement() {
    let mut s = Session::new();
    let list = s.execute("command.list", json!({})).unwrap();
    let find = |id: &str| list.as_array().unwrap().iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!(find("file.new")["enabled"], true);
    assert_eq!(find("layer.new.layer")["enabled"], false);
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
    assert_eq!(s.active().unwrap().last_damage, None);
    s.execute("paint.stroke", json!({"points": [[10, 10], [20, 10]], "size": 4})).unwrap();
    let d = s.active().unwrap().last_damage.unwrap();
    assert!(d.contains(15, 10) && d.width() < 30);
    // Undo reports the same bounded area back, not a whole-canvas recomposite.
    s.execute("edit.undo", json!({})).unwrap();
    let d = s.active().unwrap().last_damage.unwrap();
    assert!(d.contains(15, 10) && d.width() < 30);
}

#[test]
fn integer_params_accept_json_floats() {
    // UIs send coordinates as floats (e.g. 12.0); every integer parameter must accept them.
    let mut s = session_with_doc();
    s.execute("select.rect", json!({"x": 1.0, "y": 2.0, "width": 10.0, "height": 5.4})).unwrap();
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["selectionBounds"], json!([1, 2, 10, 5]));
    let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 3.0, "y": 3.0})).unwrap()).unwrap();
    assert_eq!(px, vec![1.0, 1.0, 1.0, 1.0]);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.translate", json!({"dx": 2.0, "dy": -1.0})).unwrap();
}

#[test]
fn move_to_refuses_to_nest_past_the_group_depth_cap() {
    let mut s = session_with_doc();
    let deep = s.execute("layer.new.layer", json!({"name": "Deep"})).unwrap()["layer"].as_u64().unwrap();
    for _ in 0..photocraft_doc::MAX_GROUP_DEPTH - 1 {
        s.execute("layer.groupLayers", json!({"layer": deep})).unwrap();
    }
    // `Deep` sits inside MAX - 1 groups. Moving a two-level group (G2 > G1 > Leaf) beside it
    // would put `Leaf` at MAX + 1.
    let path = s.active().unwrap().doc.path_of(photocraft_doc::LayerId(deep)).unwrap();
    assert_eq!(path.len() - 1, photocraft_doc::MAX_GROUP_DEPTH - 1);
    let leaf = s.execute("layer.new.layer", json!({"name": "Leaf"})).unwrap()["layer"].as_u64().unwrap();
    let g1 = s.execute("layer.groupLayers", json!({"layer": leaf})).unwrap()["layer"].as_u64().unwrap();
    let g = s.execute("layer.groupLayers", json!({"layer": g1})).unwrap()["layer"].as_u64().unwrap();
    let past = s.active().unwrap().history.past_len();
    let err = s.execute("layer.moveTo", json!({"layer": g, "target": deep, "position": "above"})).unwrap_err();
    assert!(err.to_string().contains("deeper than"), "{err}");
    assert!(s.active().unwrap().doc.max_group_depth() <= photocraft_doc::MAX_GROUP_DEPTH);
    assert_eq!(s.active().unwrap().history.past_len(), past, "no history step recorded");
    // A plain layer beside the deepest one still fits.
    s.execute("layer.moveTo", json!({"layer": leaf, "target": deep, "position": "above"})).unwrap();
    assert_eq!(s.active().unwrap().doc.max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH - 1);
}

#[test]
fn artboard_from_layers_refuses_to_nest_past_the_group_depth_cap() {
    let mut s = session_with_doc();
    let leaf = s.execute("layer.new.layer", json!({"name": "Leaf"})).unwrap()["layer"].as_u64().unwrap();
    let mut top = leaf;
    for _ in 0..photocraft_doc::MAX_GROUP_DEPTH {
        top = s.execute("layer.groupLayers", json!({"layer": top})).unwrap()["layer"].as_u64().unwrap();
    }
    assert_eq!(s.active().unwrap().doc.max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH);
    s.execute("layer.select", json!({"layer": top})).unwrap();
    let past = s.active().unwrap().history.past_len();
    let err = s.execute("layer.new.artboardFromLayers", json!({})).unwrap_err();
    assert!(err.to_string().contains("deeper than"), "{err}");
    assert_eq!(s.active().unwrap().doc.max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH);
    assert_eq!(s.active().unwrap().history.past_len(), past, "no history step recorded");
}

#[test]
fn move_to_reorders_and_nests() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap();
    let g = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    let names = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    // bottom->top: Background, A, B, G ; move A above B
    s.execute("layer.moveTo", json!({"layer": a, "target": b, "position": "above"})).unwrap();
    assert_eq!(names(&s), ["Background", "B", "A", "G"]);
    s.execute("layer.moveTo", json!({"layer": a, "target": g, "position": "into"})).unwrap();
    assert_eq!(names(&s), ["Background", "B", "G"]);
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["layers"][0]["children"][0]["name"], "A");
    // groups can't go into themselves
    assert!(s.execute("layer.moveTo", json!({"layer": g, "target": a, "position": "into"})).is_err());
    // one undo step
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(names(&s), ["Background", "B", "A", "G"]);
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

#[test]
fn move_to_copy_rejects_bad_params() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    let past = s.active().unwrap().history.past_len();
    for p in [
        json!({"layer": a, "copy": true}),
        json!({"layer": a, "target": "x", "copy": true}),
        json!({"layer": a, "target": 9_999_999, "copy": true}),
        json!({"layer": 9_999_999, "target": a, "copy": true}),
        json!({"layer": a, "target": a, "position": "into", "copy": true}),
    ] {
        assert!(s.execute("layer.moveTo", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), past, "no history step recorded");
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn move_selected_layers_into_existing_group_preserves_order_selection_and_one_undo() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap();
    let c = s.execute("layer.new.layer", json!({"name": "C"})).unwrap()["layer"].as_u64().unwrap();
    let g = s.execute("layer.new.group", json!({"name": "Existing Group"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.select", json!({"layer": c, "mode": "add"})).unwrap();

    let past = s.active().unwrap().history.past_len();
    // Pass reversed IDs deliberately. The document order (A below C) must win.
    let r = s.execute("layer.moveTo", json!({"layers": [c, a], "target": g, "position": "into"})).unwrap();
    assert_eq!(r["moved"], 2);
    let st = s.active().unwrap();
    let root: Vec<_> = st.doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(root, ["Background", "B", "Existing Group"]);
    let names: Vec<_> = st.doc.layer(LayerId(g)).unwrap().children().unwrap().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["A", "C"]);
    assert_eq!(st.history.past_len(), past + 1, "one drag = one undo step");
    assert_eq!(st.active_layer, Some(LayerId(c)), "preserve the active member");
    assert!(st.selected_layers().contains(&LayerId(a)));
    assert!(st.selected_layers().contains(&LayerId(c)));

    s.execute("edit.undo", json!({})).unwrap();
    let root: Vec<_> = s.active().unwrap().doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(root, ["Background", "A", "B", "C", "Existing Group"]);
    assert!(s.active().unwrap().doc.layer(LayerId(g)).unwrap().children().unwrap().is_empty());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(LayerId(g)).unwrap().children().unwrap().len(), 2);
    assert_eq!(b, s.active().unwrap().doc.layers[1].id.0);
}

#[test]
fn move_selected_layers_keeps_selected_group_children_and_supports_above_below() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap();
    let c = s.execute("layer.new.layer", json!({"name": "C"})).unwrap()["layer"].as_u64().unwrap();
    let group = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    // Made before B moves into G: a new group goes above the active layer, which would then be inside G.
    let target = s.execute("layer.new.group", json!({"name": "Target"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.moveTo", json!({"layer": b, "target": group, "position": "into"})).unwrap();

    let r = s.execute("layer.moveTo", json!({"layers": [b, group, a], "target": target, "position": "into"})).unwrap();
    assert_eq!(r["moved"], 2, "moving a group already carries its selected child");
    let target_children = s.active().unwrap().doc.layer(LayerId(target)).unwrap().children().unwrap();
    assert_eq!(target_children.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["A", "G"]);
    assert_eq!(target_children[1].children().unwrap()[0].id.0, b);
    assert!(s.active().unwrap().selected_layers().contains(&LayerId(b)), "nested selected child is still selected");

    s.execute("layer.moveTo", json!({"layers": [group, a], "target": c, "position": "above"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Background", "C", "A", "G", "Target"]);
    s.execute("layer.moveTo", json!({"layers": [group, a], "target": c, "position": "below"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Background", "A", "G", "C", "Target"]);
}

#[test]
fn move_selected_layers_invalid_targets_leave_document_and_history_unchanged() {
    let mut s = session_with_doc();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    let g = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.moveTo", json!({"layer": a, "target": g, "position": "into"})).unwrap();
    let outside = s.execute("layer.new.layer", json!({"name": "Outside"})).unwrap()["layer"].as_u64().unwrap();
    let root = s.active().unwrap().doc.layers.clone();
    let past = s.active().unwrap().history.past_len();
    for params in [
        json!({"layers": [g, a], "target": a, "position": "into"}),
        json!({"layers": [g], "target": g, "position": "above"}),
        json!({"layers": [g, outside], "target": g, "position": "into"}),
        json!({"layers": [g, outside], "target": a, "position": "below"}),
        json!({"layers": [a], "target": outside, "position": "into"}),
        json!({"layers": [a, a], "target": outside}),
        json!({"layers": [a, "bogus"], "target": outside}),
        json!({"layers": [], "target": outside}),
        json!({"layers": [u64::MAX], "target": outside}),
        json!({"layers": [a], "target": outside, "position": "sideways"}),
    ] {
        assert!(s.execute("layer.moveTo", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().history.past_len(), past, "{params}: no history step");
        assert_eq!(s.active().unwrap().doc.layers, root, "{params}: layers unchanged");
    }
}

#[test]
fn selecting_a_layer_does_not_dirty_the_document() {
    let mut s = session_with_doc();
    let bg = s.active().unwrap().doc.layers[0].id;
    assert!(!s.active().unwrap().is_dirty());
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    assert!(!s.active().unwrap().is_dirty());
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(s.active().unwrap().is_dirty());
}

#[test]
fn coalesced_edits_share_one_history_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let steps = |s: &Session| s.active().unwrap().history.entries().len();
    let base = steps(&s);
    for o in [10, 20, 30] {
        s.execute("layer.setProps", json!({"opacity": o as f64 / 100.0, "coalesce": "drag-1"})).unwrap();
    }
    assert_eq!(steps(&s), base + 1, "three coalesced edits = one step");
    // A different key starts a new step; an uncoalesced edit breaks the chain.
    s.execute("layer.setProps", json!({"opacity": 0.4, "coalesce": "drag-2"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.6, "coalesce": "drag-2"})).unwrap();
    assert_eq!(steps(&s), base + 4);
    // Undo returns to the state before the whole coalesced run.
    s.undo();
    s.undo();
    s.undo();
    s.undo();
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap().opacity;
    assert!((l - 1.0).abs() < 1e-6, "{l}");
}

#[test]
fn type_edit_rerenders_cache_to_new_text() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400})).unwrap();
    let r = s.execute("type.create", json!({"x": 50, "y": 200, "text": "Lorem Ipsum", "size": 48, "coalesce": "k"})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    let width = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().content_bounds().width()
    };
    let w0 = width(&s);
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 0, "end": 11, "text": "Photocraft"}, "coalesce": "k"})).unwrap();
    let w1 = width(&s);
    assert!(w0 > 150 && w1 > 150, "cache widths {w0} → {w1}");
}

#[test]
fn levels_and_curves_params_cover_output_and_channels() {
    use photocraft_doc::Adjustment;
    let a = crate::commands::adjustment_from_params("levels", &json!({"inBlack": 10, "outWhite": 200, "green": {"gamma": 1.5}}));
    let Adjustment::Levels { master, per_channel, .. } = a else { panic!() };
    assert!((master.in_black - 10.0 / 255.0).abs() < 1e-6 && (master.out_white - 200.0 / 255.0).abs() < 1e-6);
    assert_eq!(per_channel[1].gamma, 1.5);
    assert_eq!(per_channel[0].gamma, 1.0);
    let c = crate::commands::adjustment_from_params("curves", &json!({"points": [[0, 0], [128, 160], [255, 255]], "blue": [[0, 20], [255, 235]]}));
    let Adjustment::Curves { master, per_channel, .. } = c else { panic!() };
    assert_eq!(master.len(), 3);
    assert!((per_channel[2][0].output - 20.0 / 255.0).abs() < 1e-6);
    assert_eq!(per_channel[0].len(), 2);
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
fn moving_a_layer_moves_its_effects_reference_point() {
    let mut s = session_with_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("ref", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().effects.reference = Some((-201.0, 9.0));
        Ok(())
    })
    .unwrap();
    s.execute("layer.translate", json!({"dx": 10, "dy": 5})).unwrap();
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    assert_eq!(l.effects.reference, Some((-191.0, 14.0)));
}

#[test]
fn marquee_feather_and_anti_aliased_ellipse() {
    let sel_at = |s: &Session, x: i32, y: i32| {
        let mut v = [0.0f32];
        if let Some(m) = s.active().unwrap().doc.selection.as_ref() {
            m.read_pixel(x, y, &mut v);
        }
        v[0]
    };
    // Hard rectangle: fully in or out.
    let mut s = session_with_doc();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(sel_at(&s, 10, 20), 1.0);
    assert_eq!(sel_at(&s, 9, 20), 0.0);
    // Feather softens only the new shape's edge: partial coverage across the boundary.
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20, "feather": 3})).unwrap();
    let (inside, edge, outside) = (sel_at(&s, 20, 20), sel_at(&s, 10, 20), sel_at(&s, 8, 20));
    assert!(inside > 0.95 && edge > 0.2 && edge < 0.8 && outside > 0.0 && outside < edge, "{inside} {edge} {outside}");
    // Anti-aliased ellipse edges have partial coverage; aliased ones don't.
    let partial = |s: &Session| (0..48).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| (0.01..0.99).contains(&sel_at(s, x, y))).count();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 40, "height": 30, "ellipse": true})).unwrap();
    assert!(partial(&s) > 20);
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 40, "height": 30, "ellipse": true, "antiAlias": false})).unwrap();
    assert_eq!(partial(&s), 0);
}

#[test]
fn file_new_resolution_and_background_color() {
    let mut s = Session::new();
    s.tools.background = [1.0, 0.0, 0.0, 1.0];
    s.execute("file.new", json!({"width": 8, "height": 8, "resolution": 300, "background": "backgroundColor"})).unwrap();
    assert_eq!(s.active().unwrap().doc.resolution_dpi, 300.0);
    let p = px(&mut s, 2, 2);
    assert!(p[0] > 0.99 && p[1] < 0.01, "{p:?}");
}

#[test]
fn duplicating_the_background_unlocks_the_copy() {
    let mut s = session_with_doc();
    s.execute("layer.duplicate", json!({})).unwrap();
    let st = s.active().unwrap();
    let copy = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(copy.name, "Background copy");
    assert_eq!(copy.locks, photocraft_doc::Locks::default());
    assert!(st.doc.layers[0].locks.transparency);
}

#[test]
fn advanced_blending_channels() {
    let mut s = session_with_doc();
    let r = s.execute("layer.new.layer", json!({})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"channels": [true, true, false]})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(LayerId(id)).unwrap().excluded_channels, 0b100);
    let ins = crate::inspect::layer(s.active().unwrap().doc.layer(LayerId(id)).unwrap());
    assert_eq!(ins["channels"], json!([true, true, false, true]));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(LayerId(id)).unwrap().excluded_channels, 0);
}

#[test]
fn move_document_reorders_tabs_and_keeps_the_active_one() {
    let mut s = Session::new();
    for name in ["a", "b", "c"] {
        s.execute("file.new", json!({"width": 4, "height": 4, "name": name})).unwrap();
    }
    let names = |s: &Session| s.documents().iter().map(|d| d.doc.name.clone()).collect::<Vec<_>>();
    let first = names(&s);
    s.set_active(0);
    assert_eq!(s.move_document(2, 0), Some(0));
    assert_eq!(names(&s), [first[2].clone(), first[0].clone(), first[1].clone()]);
    assert_eq!(s.active_index(), Some(1), "the active document follows its tab");
    // `to` past the end moves to the last tab; `from` out of range does nothing.
    assert_eq!(s.move_document(0, 99), Some(2));
    assert_eq!(names(&s), first);
    assert_eq!(s.move_document(3, 0), None);
    let mut v = vec![1, 2];
    assert_eq!(move_item(&mut v, 2, 0), None, "out of range: no panic, nothing moves");
    assert_eq!(v, [1, 2]);
    assert_eq!(names(&s), first);
    assert_eq!(s.active_index(), Some(0));
    // The command (for the UI, agents and scripts) moves the active document by default.
    assert_eq!(s.execute("document.move", json!({"to": 2})).unwrap(), json!({"document": 2}));
    assert_eq!(names(&s), [first[1].clone(), first[2].clone(), first[0].clone()]);
    assert_eq!(s.execute("document.move", json!({"document": 2, "to": 0})).unwrap(), json!({"document": 0}));
    assert_eq!(names(&s), first);
    for bad in [json!({}), json!({"to": -1}), json!({"to": "1"}), json!({"document": 3, "to": 0}), json!({"document": 1.5, "to": 0})] {
        assert!(s.execute("document.move", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(names(&s), first);
}
