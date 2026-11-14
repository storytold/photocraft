//! #12920: a Quick Selection worked at half resolution (documents above 512 x 512 pixels) ends at
//! a hard-edged shape's edge. Its soft rim lies inside the shape (191 on the edge pixels, 143 at
//! the corners) and no pixel outside the shape is selected, as measured in the reference
//! application. The test document is the issue's: 800 x 600, white, red rectangles 100,100-300,250
//! ("red 1") and 500,100-600,200 ("red 2"), a grey ramp of ten 40 px blocks (100, 105, ..., 145)
//! over 100,350-500,450 and a blue square 600,350-700,450, sampled as one composite.

use std::collections::BTreeMap;

use photocraft_geom::Rect;

use super::upsample;
use crate::segment::{ImageSampler, Region, RgbImage, quick};

const RED: [f32; 3] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0];
const BLUE: [f32; 3] = [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0];

/// The issue's 800 x 600 test document as one composite.
fn document() -> RgbImage {
    let inside = |x: usize, y: usize, x0: usize, y0: usize, x1: usize, y1: usize| (x0..x1).contains(&x) && (y0..y1).contains(&y);
    RgbImage::from_fn(800, 600, |x, y| {
        if inside(x, y, 100, 100, 300, 250) || inside(x, y, 500, 100, 600, 200) {
            RED
        } else if inside(x, y, 100, 350, 500, 450) {
            [(100 + 5 * ((x - 100) / 40)) as f32 / 255.0; 3]
        } else if inside(x, y, 600, 350, 700, 450) {
            BLUE
        } else {
            [1.0; 3]
        }
    })
}

/// A click with the default 30 px brush at `(x, y)`.
fn click(img: &RgbImage, x: f32, y: f32) -> Region {
    let sampler = ImageSampler { img, origin: (0, 0) };
    quick::quick_select(&sampler, Rect::new(0, 0, img.w as i32, img.h as i32), &[(x, y)], 30.0).expect("a selection")
}

/// What the issue reports for a selection: the bounds of its selected pixels (left, top, right,
/// bottom), how many are fully selected, and the 8-bit value of each partly selected one with
/// its count.
fn summary(r: &Region) -> ((i32, i32, i32, i32), usize, BTreeMap<u8, usize>) {
    let (mut b, mut full, mut partial) = ((i32::MAX, i32::MAX, i32::MIN, i32::MIN), 0, BTreeMap::new());
    for y in r.bbox.y0..r.bbox.y1 {
        for x in r.bbox.x0..r.bbox.x1 {
            let v = r.mask[(y - r.bbox.y0) as usize * r.bbox.width() as usize + (x - r.bbox.x0) as usize];
            if v == 0 {
                continue;
            }
            b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
            if v == 255 {
                full += 1;
            } else {
                *partial.entry(v).or_insert(0) += 1;
            }
        }
    }
    (b, full, partial)
}

#[test]
fn a_click_on_red_1_ends_at_its_edge_with_the_rim_inside() {
    let got = summary(&click(&document(), 200.0, 175.0));
    // 198 x 148 full pixels, the 1 px rim of 2 x (198 + 148) at 191 and the 4 corners at 143.
    assert_eq!(got, ((100, 100, 300, 250), 29304, BTreeMap::from([(143, 4), (191, 692)])));
}

#[test]
fn a_click_on_red_2_ends_at_its_edge_with_the_rim_inside() {
    let got = summary(&click(&document(), 550.0, 150.0));
    assert_eq!(got, ((500, 100, 600, 200), 9604, BTreeMap::from([(143, 4), (191, 392)])));
}

#[test]
fn a_click_on_the_ramp_takes_it_whole_with_the_rim_inside() {
    // On the composite PhotoCraft already grows over the whole ramp (#12921 is the layer case).
    let got = summary(&click(&document(), 120.0, 400.0));
    assert_eq!(got, ((100, 350, 500, 450), 39004, BTreeMap::from([(143, 4), (191, 992)])));
}

#[test]
fn upsampling_keeps_unselected_working_pixels_empty() {
    // 4 x 3 working pixels with the middle two of the middle row selected, step 2, and a frame
    // whose last column and row fall in a half cell (7 x 5).
    let low = [false, false, false, false, false, true, true, false, false, false, false, false];
    let out = upsample(&low, 4, 3, 2, 7, 5);
    let rows: Vec<&[u8]> = out.chunks(7).collect();
    assert_eq!(rows[0], [0; 7]);
    assert_eq!(rows[1], [0; 7]);
    assert_eq!(rows[2], [0, 0, 143, 191, 191, 143, 0]);
    assert_eq!(rows[3], [0, 0, 143, 191, 191, 143, 0]);
    assert_eq!(rows[4], [0; 7]);
    // Full resolution is the working selection as it is.
    assert_eq!(upsample(&[true, false, false, true], 2, 2, 1, 2, 2), [255, 0, 0, 255]);
}
