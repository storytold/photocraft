//! #11762: count points keep their document coordinates through Image Size, Canvas Size and Image
//! Rotation (right angles, flips and arbitrary), also off the new canvas; other marks still follow
//! the image, and Crop keeps today's behaviour.

use serde_json::json;

#[test]
fn ruler_straighten_with_crop_asks_the_gate_for_its_rotation() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    // Straighten Layer is off for the Background (#12033).
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [0, 0], "end": [100, 10]})).unwrap();
    let size = s.active().unwrap().doc.size;
    s.authorize = Some(|id, _| if id == "image.rotation.arbitrary" { Err(crate::EngineError::Other("denied".into())) } else { Ok(()) });
    assert!(s.execute("image.analysis.straightenLayer", json!({"crop": true})).is_err(), "the nested rotation is denied");
    s.authorize = None;
    assert_eq!(s.active().unwrap().doc.size, size, "nothing turned");
}
