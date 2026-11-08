use photocraft_engine::Session;
use serde_json::json;

/// Make Shape preserves source geometry, defaults to legacy, and records conversion atomically.
#[test]
fn pixel_align_make_shape_supports_work_saved_and_inline_paths() {
    for source in ["work", "saved", "inline"] {
        for enabled in [None, Some(false), Some(true)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            let path = json!({"subpaths":[{"closed":true,"knots":[[10,10],[50,10],[50,50]]}]});
            s.execute("path.set", json!({"name":"work","path":path})).unwrap();
            let expected = s.active().unwrap().doc.work_path.clone().unwrap();
            let mut params = json!({"fill":null,"stroke":{"width":1}});
            match source {
                "work" => params["name"] = json!("work"),
                "saved" => {
                    s.execute("path.set", json!({"name":"Saved","path":path})).unwrap();
                    params["name"] = json!("Saved");
                }
                _ => params["path"] = path,
            }
            if let Some(value) = enabled {
                params["pixelAlignStroke"] = json!(value);
            }
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let info = s.execute("path.toShape", params).unwrap();
            assert_eq!(info["pixelAlignStroke"], enabled.unwrap_or(false), "source={source}");
            let st = s.active().unwrap();
            let photocraft_doc::LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape") };
            assert_eq!(shape.path, expected);
            assert_eq!(st.doc.paths, before.paths);
            assert_eq!(st.doc.work_path.is_none(), source == "work");
            assert_eq!(st.history.past_len(), history + 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.work_path, before.work_path);
            assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
            s.execute("edit.redo", json!({})).unwrap();
            let redone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(redone["pixelAlignStroke"], enabled.unwrap_or(false));
            assert_eq!(redone["path"], info["path"]);
        }
    }
}

/// Invalid alignment values cannot consume a Work Path or alter document/history.
#[test]
fn pixel_align_make_shape_rejects_invalid_flag_atomically() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":100})).unwrap();
    s.execute("path.set", json!({"path":{"subpaths":[{"knots":[[10,10],[40,10],[40,40]]}]}})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for invalid in [json!(null), json!("true"), json!(1), json!([]), json!({})] {
        assert!(s.execute("path.toShape", json!({"name":"work","pixelAlignStroke":invalid})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

/// Complete Stroke copies a snapshot of alignment and style, and pastes both in one undo step.
#[test]
fn complete_stroke_clipboard_preserves_pixel_alignment_and_geometry() {
    for enabled in [false, true] {
        for new_document in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            let source = s
                .execute(
                    "shape.create",
                    json!({"kind":"rect","rect":[10,10,20,20],
                "pixelAlignStroke":enabled,"stroke":{"width":1,"color":"#ff0000"}}),
                )
                .unwrap();
            let history = s.active().unwrap().history.past_len();
            s.execute("path.style.copyStroke", json!({})).unwrap();
            assert_eq!(s.active().unwrap().history.past_len(), history, "copy must not add history");
            // Editing the source after copying must not change the clipboard snapshot.
            s.execute("shape.edit", json!({"pixelAlignStroke":!enabled,"stroke":null})).unwrap();
            assert!(s.execute("path.style.copyStroke", json!({})).is_err());
            if new_document {
                s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            }
            let before = s
                .execute(
                    "shape.create",
                    json!({"kind":"ellipse","rect":[40,40,30,30],
                "pixelAlignStroke":!enabled,"fill":"#00ff00","stroke":{"width":3,"color":"#0000ff"}}),
                )
                .unwrap();
            let history = s.active().unwrap().history.past_len();
            let after = s.execute("path.style.pasteStroke", json!({})).unwrap();
            assert_eq!(after["pixelAlignStroke"], enabled, "new_document={new_document}");
            assert_eq!(after["stroke"], source["stroke"]);
            assert_eq!(after["path"], before["path"]);
            assert_eq!(after["fill"], before["fill"]);
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            s.execute("edit.undo", json!({})).unwrap();
            let undone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(undone["pixelAlignStroke"], !enabled);
            assert_eq!(undone["stroke"], before["stroke"]);
            assert_eq!(undone["path"], before["path"]);
            s.execute("edit.redo", json!({})).unwrap();
            let redone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(redone["pixelAlignStroke"], enabled);
            assert_eq!(redone["stroke"], source["stroke"]);
            assert_eq!(redone["path"], before["path"]);
        }
    }
}

/// Appending honors an explicit flag, preserves an omitted flag, and undoes both atomically.
#[test]
fn pixel_align_append_honors_explicit_values_and_preserves_omission() {
    for initial in [false, true] {
        for explicit in [None, Some(false), Some(true)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100})).unwrap();
            let before = s.execute("shape.create", json!({"kind":"rect","rect":[10,10,20,20],"pixelAlignStroke":initial})).unwrap();
            let mut params = json!({"kind":"rect","rect":[50,50,20,20],"addTo":before["layer"]});
            if let Some(value) = explicit {
                params["pixelAlignStroke"] = json!(value);
            }
            let after = s.execute("shape.create", params).unwrap();
            assert_eq!(after["pixelAlignStroke"], explicit.unwrap_or(initial));
            assert_ne!(after["path"], before["path"]);
            s.execute("edit.undo", json!({})).unwrap();
            let undone = s.execute("shape.info", json!({})).unwrap();
            assert_eq!(undone["pixelAlignStroke"], initial);
            assert_eq!(undone["path"], before["path"]);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.execute("shape.info", json!({})).unwrap()["pixelAlignStroke"], explicit.unwrap_or(initial));
        }
    }
}

/// Invalid flag types leave history intact, and undo/redo changes only the rendering option.
#[test]
fn pixel_align_shape_commands_preserve_path_and_undo_and_validate_bool() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":100,"background":"transparent"})).unwrap();
    s.execute("shape.create", json!({"kind":"rect","rect":[10,10,60,60],"stroke":{"width":1}})).unwrap();
    let legacy = s.execute("shape.info", json!({})).unwrap();
    assert_eq!(legacy["pixelAlignStroke"], false);
    let path = legacy["path"].clone();
    let on = s.execute("shape.edit", json!({"pixelAlignStroke":true})).unwrap();
    assert_eq!(on["pixelAlignStroke"], true);
    assert_eq!(on["path"], path);
    assert!(s.execute("shape.edit", json!({"pixelAlignStroke":"true"})).is_err());
    assert_eq!(s.execute("shape.info", json!({})).unwrap()["pixelAlignStroke"], true);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("shape.info", json!({})).unwrap()["pixelAlignStroke"], false);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.execute("shape.info", json!({})).unwrap()["pixelAlignStroke"], true);
}

/// Placement forwards either explicit alignment value through the custom-shape preset command.
#[test]
fn custom_shape_presets_forward_pixel_alignment_without_changing_preset_geometry() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":100})).unwrap();
    s.execute("shape.create", json!({"kind":"rect","rect":[10,10,60,60]})).unwrap();
    let preset = s.execute("shape.presets.new", json!({"name":"Pixel Alignment Regression"})).unwrap();
    let mut placed_path = None;
    for value in [true, false] {
        s.execute("shape.presets.place", json!({"preset":preset["name"],"rect":[10,10,60,60],"pixelAlignStroke":value,"stroke":{"width":1}})).unwrap();
        assert_eq!(s.execute("shape.info", json!({})).unwrap()["pixelAlignStroke"], value);
        let st = s.active().unwrap();
        let photocraft_doc::LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape") };
        if let Some(expected) = &placed_path {
            assert_eq!(&shape.path, expected, "alignment must not alter placed preset geometry");
        } else {
            placed_path = Some(shape.path.clone());
        }
    }
}
