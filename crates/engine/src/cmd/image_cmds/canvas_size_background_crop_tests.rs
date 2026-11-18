//! #14538: a Canvas Size that shrinks the canvas cuts the Background to it, as the reference
//! application does, so growing it again fills the new area with the extension colour instead of
//! bringing the cut pixels back. Normal layers keep their pixels past the canvas and are not
//! filled. Crop with Delete Cropped Pixels off and Reveal All keep their behaviour (#7086).

use photocraft_geom::Rect;
use serde_json::json;

use crate::Session;

const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// The issue's document: 400 x 300, a white Background with a blue 100 x 100 square at 300,200.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    fill(&mut s, Rect::from_xywh(300, 200, 100, 100), BLUE);
    s
}

fn fill(s: &mut Session, r: Rect, c: [f32; 4]) {
    s.execute("select.rect", json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": c})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

fn canvas(s: &mut Session, width: u32, extension: &str) {
    s.execute("image.canvasSize", json!({"width": width, "height": 300, "anchor": "topLeft", "extensionColor": extension})).unwrap();
}

fn background_at(s: &Session, x: i32, y: i32) -> [f32; 4] {
    s.active().unwrap().doc.background().unwrap().surface().unwrap().rgba(x, y)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.5 / 255.0)
}

#[test]
fn growing_back_after_a_shrink_fills_with_the_extension_colour() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        canvas(&mut s, 300, "background");
        canvas(&mut s, 400, "black");
        assert_eq!(background_at(&s, 350, 250), [0.0, 0.0, 0.0, 1.0], "{depth} bit: black, not the blue that was cut off");
        // The Background inside the smaller canvas stays.
        assert!(close(background_at(&s, 299, 250), [1.0; 4]), "{depth} bit: {:?}", background_at(&s, 299, 250));
        // A shrink that cuts the square in two keeps the part inside the canvas.
        let mut s = session(depth);
        canvas(&mut s, 350, "background");
        canvas(&mut s, 400, "black");
        assert!(close(background_at(&s, 325, 250), BLUE), "{depth} bit: {:?}", background_at(&s, 325, 250));
        assert_eq!(background_at(&s, 375, 250), [0.0, 0.0, 0.0, 1.0], "{depth} bit: the cut half is black");

        // Mid-gray as this depth stores it (32-bit documents are linear): what a plain grow adds.
        let mut plain = Session::new();
        plain.execute("file.new", json!({"width": 300, "height": 300, "depth": depth, "background": "white"})).unwrap();
        canvas(&mut plain, 400, "#808080");
        let gray = background_at(&plain, 350, 250);
        let mut s = session(depth);
        canvas(&mut s, 300, "background");
        canvas(&mut s, 400, "#808080");
        assert!(close(background_at(&s, 350, 250), gray), "{depth} bit: {:?}, want {gray:?}", background_at(&s, 350, 250));
    }
}

#[test]
fn a_centred_shrink_cuts_the_background_after_moving_it() {
    // A centred anchor moves the layers by -50 before the cut, and +50 when growing back: the
    // square (300..400) sits at 250..350 on the 300 px canvas, keeps 250..300, and comes back at
    // 300..350 with black on both new strips (CodeRabbit on #15563).
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let centred = |s: &mut Session, width: u32, extension: &str| {
            s.execute("image.canvasSize", json!({"width": width, "height": 300, "anchor": "center", "extensionColor": extension})).unwrap();
        };
        centred(&mut s, 300, "background");
        centred(&mut s, 400, "black");
        assert!(close(background_at(&s, 325, 250), BLUE), "{depth} bit: {:?}", background_at(&s, 325, 250));
        assert_eq!(background_at(&s, 375, 250), [0.0, 0.0, 0.0, 1.0], "{depth} bit: the cut part is black");
        assert_eq!(background_at(&s, 25, 250), [0.0, 0.0, 0.0, 1.0], "{depth} bit: the new left strip is black");
    }
}

#[test]
fn a_shrink_drops_the_backgrounds_pixels_outside_the_canvas() {
    let mut s = session(8);
    canvas(&mut s, 300, "background");
    let bg = s.active().unwrap().doc.background().unwrap();
    let bounds = bg.surface().unwrap().content_bounds();
    assert!(bounds.x1 <= 300, "the Background holds nothing past the 300 px canvas: {bounds:?}");
}

#[test]
fn normal_layers_keep_their_pixels_past_the_canvas_and_are_not_filled() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let edge = s.execute("layer.new.layer", json!({"name": "Edge"})).unwrap()["layer"].as_u64().unwrap();
        fill(&mut s, Rect::new(350, 125, 400, 175), GREEN);
        canvas(&mut s, 300, "background");
        canvas(&mut s, 400, "black");
        let doc = &s.active().unwrap().doc;
        let layer = doc.layer(photocraft_doc::LayerId(edge)).unwrap();
        let surface = layer.surface().unwrap();
        assert_eq!(surface.content_bounds(), Rect::new(350, 125, 400, 175), "{depth} bit: the layer survives the shrink");
        assert!(close(surface.rgba(375, 150), GREEN), "{depth} bit: green shows again");
        assert_eq!(surface.rgba(320, 20)[3], 0.0, "{depth} bit: the extension stays transparent on a normal layer");
    }
}

#[test]
fn a_grow_alone_keeps_todays_result() {
    let mut s = session(8);
    canvas(&mut s, 500, "black");
    assert!(close(background_at(&s, 350, 250), BLUE), "the square is untouched");
    assert_eq!(background_at(&s, 450, 150), [0.0, 0.0, 0.0, 1.0], "the new strip is black");
}
