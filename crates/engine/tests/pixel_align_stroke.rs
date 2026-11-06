use photocraft_engine::Session;
use serde_json::json;

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
