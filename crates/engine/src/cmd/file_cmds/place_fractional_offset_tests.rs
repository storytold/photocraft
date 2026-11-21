//! Place keeps a half-pixel centred offset in the placed smart object's transform; only the
//! layer's pixels land on whole pixels (#14114). Each test gives the centre explicitly, so they
//! check where the image goes from a centre, not which centre Place picks.

use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use crate::Session;

fn png(w: u32, h: u32) -> Vec<u8> {
    let px = vec![200u8; (w * h * 3) as usize];
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

/// Places a `w`×`h` PNG into a 400×300 document, centred on the canvas centre; returns the
/// command's result, the smart object's transform and the left/top edge of its pixels.
fn place(name: &str, w: u32, h: u32, depth: u32) -> (Value, [f64; 6], (i32, i32)) {
    let dir = std::env::temp_dir().join(format!("photocraft-place-fractional-{}-{name}-{depth}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    std::fs::write(&path, png(w, h)).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth})).unwrap();
    let r = s.execute("file.placeEmbedded", json!({"path": path.to_string_lossy(), "fit": true, "smartObject": true, "center": [200.0, 150.0]})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let d = s.active().unwrap();
    let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let LayerContent::Smart(so) = &layer.content else { panic!("a smart object was expected") };
    let cache = so.cache.as_ref().expect("the placed pixels");
    let px = cache.content_bounds();
    (r, so.transform.m, (px.x0, px.y0))
}

/// A 1000×800 file fitted into 400×300 is 375×300, centred at 12.5,0: the transform keeps the
/// half pixel, the reported box is 12.5,0 - 387.5,300 and the pixels start at 13.
#[test]
fn a_half_pixel_centred_offset_stays_in_the_transform() {
    for depth in [8, 16, 32] {
        let (r, m, (x0, y0)) = place("wide", 1000, 800, depth);
        assert_eq!([m[0], m[3]], [0.375, 0.375], "{depth}-bit scale");
        assert_eq!([m[4], m[5]], [12.5, 0.0], "{depth}-bit transform offset");
        assert_eq!(r["bounds"], json!([12.5, 0.0, 387.5, 300.0]), "{depth}-bit reported box");
        assert_eq!((x0, y0), (13, 0), "{depth}-bit pixel bounds round the left edge to 13");
    }
}

/// A whole-pixel offset places exactly as before: 100×60 centred in 400×300 is 150,120 - 250,180.
#[test]
fn a_whole_pixel_offset_is_unchanged() {
    let (r, m, (x0, y0)) = place("small", 100, 60, 8);
    assert_eq!(m, [1.0, 0.0, 0.0, 1.0, 150.0, 120.0]);
    assert_eq!(r["bounds"], json!([150.0, 120.0, 250.0, 180.0]));
    assert_eq!((x0, y0), (150, 120));
}
