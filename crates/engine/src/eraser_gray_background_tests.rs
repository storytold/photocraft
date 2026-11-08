//! On a Grayscale document's Background, the Eraser and Edit › Fill with the background colour
//! paint that colour's grey as the reference converts it: RGB 0, 200, 0 gives 173, not its 117
//! luminance (#11940). Converting through the document's grey profile gives 153, so the two
//! Grayscale tests are ignored until the issue says which conversion the swatch uses.

use crate::Session;
use serde_json::json;

const GREEN_200: [f32; 4] = [0.0, 200.0 / 255.0, 0.0, 1.0];

fn session(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "mode": mode, "depth": 8, "background": "white"})).unwrap();
    s.tools.background = GREEN_200;
    s
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

fn level(c: f32) -> i32 {
    (c * 255.0).round() as i32
}

fn erase_at(s: &mut Session, x: i32, y: i32) {
    s.execute("paint.stroke", json!({"points": [[x, y]], "size": 9, "hardness": 1.0, "erase": true})).unwrap();
}

#[test]
#[ignore = "#11940: the grey profile gives 153 for RGB 0,200,0; the issue measures 173. Waiting on which conversion the swatch should use"]
fn the_eraser_on_a_gray_background_paints_the_background_colours_gray() {
    let mut s = session("grayscale");
    erase_at(&mut s, 20, 16);
    let p = pixel(&mut s, 20, 16);
    assert!((level(p[0]) - 173).abs() <= 2, "erased pixel {p:?}");
    assert_eq!(level(p[0]), level(p[1]));
    assert_eq!(level(p[0]), level(p[2]));
    assert_eq!(level(pixel(&mut s, 50, 16)[0]), 255, "pixels away from the click stay white");
}

#[test]
#[ignore = "#11940: the grey profile gives 153 for RGB 0,200,0; the issue measures 173. Waiting on which conversion the swatch should use"]
fn edit_fill_with_the_background_colour_gives_the_same_gray() {
    let mut s = session("grayscale");
    s.execute("edit.fill", json!({"contents": "background"})).unwrap();
    let filled = pixel(&mut s, 5, 5);
    assert!((level(filled[0]) - 173).abs() <= 2, "filled pixel {filled:?}");

    let mut e = session("grayscale");
    erase_at(&mut e, 20, 16);
    assert_eq!(level(pixel(&mut e, 20, 16)[0]), level(filled[0]), "the Eraser and Edit › Fill agree");
}

#[test]
fn rgb_documents_keep_the_background_colour_as_it_is() {
    let mut s = session("rgb");
    erase_at(&mut s, 20, 16);
    let p = pixel(&mut s, 20, 16);
    assert_eq!([level(p[0]), level(p[1]), level(p[2])], [0, 200, 0], "erased pixel {p:?}");

    let mut f = session("rgb");
    f.execute("edit.fill", json!({"contents": "background"})).unwrap();
    let q = pixel(&mut f, 5, 5);
    assert_eq!([level(q[0]), level(q[1]), level(q[2])], [0, 200, 0], "filled pixel {q:?}");
}
