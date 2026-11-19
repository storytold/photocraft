use super::*;

#[test]
fn canvas_rotation_rounds_to_hundredths_before_editing() {
    for direction in ["cw", "ccw"] {
        for (angle, width, height, edits) in [
            (0.001, 400, 300, 0),
            (0.004, 400, 300, 0),
            (-0.004, 400, 300, 0),
            (0.0, 400, 300, 0),
            (360.0, 400, 300, 0),
            (-360.0, 400, 300, 0),
            (720.0, 400, 300, 0),
            (359.996, 400, 300, 0),
            (0.005, 401, 301, 1),
            (-0.005, 401, 301, 1),
            (359.99, 401, 301, 1),
            (361.0, 406, 307, 1),
        ] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
            let before = s.active().unwrap().history.past_len();
            let result = s.execute("image.rotation.arbitrary", json!({"angle": angle, "direction": direction})).unwrap();
            assert_eq!(result, json!({"width": width, "height": height}), "{direction} {angle}");
            assert_eq!(s.active().unwrap().history.past_len(), before + edits, "{direction} {angle}");
        }
    }
}

#[test]
fn straightening_still_applies_a_sub_hundredth_turn() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    let before = s.active().unwrap().history.past_len();
    let result = rotate_arbitrary_with(&mut s, &json!({"angle": 0.004}), false).unwrap();
    assert_eq!(result, json!({"width": 41, "height": 31}));
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
}
