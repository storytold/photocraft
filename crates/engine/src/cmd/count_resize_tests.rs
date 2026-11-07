//! #11762: count points keep their document coordinates through Image Size, Canvas Size and Image
//! Rotation (right angles, flips and arbitrary), also off the new canvas; other marks still follow
//! the image, and Crop keeps today's behaviour.

use serde_json::{Value, json};

use crate::Session;

const POINT: [f64; 2] = [99.3, 99.7];

fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    s.execute("count.add", json!({"x": POINT[0], "y": POINT[1]})).unwrap();
    s.execute("notes.add", json!({"x": 100, "y": 100})).unwrap();
    s
}

fn count_point(s: &Session) -> [f64; 2] {
    s.active().unwrap().doc.measurement.count_groups[0].points[0]
}

fn note(s: &Session) -> [f64; 2] {
    s.active().unwrap().doc.notes[0].position
}

#[test]
fn count_points_keep_their_coordinates() {
    let steps: [(&str, Value); 8] = [
        ("image.imageSize", json!({"width": 300, "height": 200})),
        ("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"})),
        ("image.imageRotation.90cw", json!({})),
        ("image.imageRotation.90ccw", json!({})),
        ("image.imageRotation.180", json!({})),
        ("image.imageRotation.flipCanvasHorizontal", json!({})),
        ("image.imageRotation.flipCanvasVertical", json!({})),
        ("image.rotation.arbitrary", json!({"angle": 30})),
    ];
    for (cmd, p) in steps {
        let mut s = doc();
        let before = note(&s);
        s.execute(cmd, p).unwrap();
        assert_eq!(count_point(&s), POINT, "{cmd}");
        assert_ne!(note(&s), before, "{cmd}: notes still follow the image");
        s.undo();
        assert_eq!((count_point(&s), note(&s)), (POINT, before), "{cmd}: undo");
    }
}

#[test]
fn all_three_in_a_row_leave_the_point_where_it_was() {
    let mut s = doc();
    s.execute("image.imageSize", json!({"width": 300, "height": 200})).unwrap();
    s.execute("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"})).unwrap();
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    assert_eq!(count_point(&s), POINT);
}

#[test]
fn crop_still_moves_count_points() {
    let mut s = doc();
    s.execute("image.crop", json!({"x": 50, "y": 50, "width": 300, "height": 200})).unwrap();
    assert_ne!(count_point(&s), POINT, "not measured: Crop keeps today's behaviour");
}
