//! #12920: above 512 x 512 pixels a Quick Selection of a hard-edged shape ends at its edge in 8-
//! and 16-bit documents (the rim lies inside it). 32-bit documents were not measured for that, so
//! they keep the plain bilinear edge: a ring of partly selected pixels just outside the shape.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

use super::*;

/// The issue's red 1 (100,100-300,250) on an 800 x 600 white document of `depth`.
fn session(depth: SampleType) -> Session {
    let mut doc = Document::with_background("t", Size::new(800, 600), ColorMode::Rgb, depth, Color::rgb(1.0, 1.0, 1.0));
    let bg = doc.layers[0].surface_mut().unwrap();
    let red = photocraft_raster::from_rgba(&bg.format(), [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]);
    bg.fill_rect(Rect::new(100, 100, 300, 250), &red);
    let mut s = crate::session();
    s.add_document(doc, None);
    s
}

/// A default click on red 1's centre: the bounds of the selected pixels and how many selected
/// pixels lie outside red 1.
fn click_on_red_1(depth: SampleType) -> ((i32, i32, i32, i32), usize) {
    let mut s = session(depth);
    s.execute("select.quick", json!({"points": [[200, 175]], "size": 30})).unwrap();
    let cover = s.active().unwrap().doc.selection.as_ref().expect("a selection").read_region(Rect::new(0, 0, 800, 600));
    let (mut b, mut outside) = ((i32::MAX, i32::MAX, i32::MIN, i32::MIN), 0);
    for (i, v) in cover.iter().enumerate() {
        let (x, y) = ((i % 800) as i32, (i / 800) as i32);
        if *v < 0.5 / 255.0 {
            continue;
        }
        b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
        if !((100..300).contains(&x) && (100..250).contains(&y)) {
            outside += 1;
        }
    }
    (b, outside)
}

#[test]
fn eight_and_sixteen_bit_selections_end_at_the_edge() {
    for depth in [SampleType::U8, SampleType::U16] {
        assert_eq!(click_on_red_1(depth), ((100, 100, 300, 250), 0), "{depth:?}");
    }
}

/// Not measured for #12920: 32-bit keeps today's ring, one pixel all round (2 x (200 + 150) + 4).
#[test]
fn a_32_bit_selection_keeps_its_ring_outside_the_edge() {
    assert_eq!(click_on_red_1(SampleType::F32), ((99, 99, 301, 251), 704));
}
