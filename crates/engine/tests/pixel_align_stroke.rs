use photocraft_engine::Session;
use serde_json::json;

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
    for value in [true, false] {
        s.execute("shape.presets.place", json!({"preset":preset["name"],"rect":[10,10,60,60],"pixelAlignStroke":value,"stroke":{"width":1}})).unwrap();
        assert_eq!(s.execute("shape.info", json!({})).unwrap()["pixelAlignStroke"], value);
    }
}
