//! #12922: one click with the Quick Selection brush on a uniform area.
//!
//! Measured in Photoshop 25.4 on an 800 x 600 RGB/8 document that is white everywhere, Background
//! active, Reset Tool, Sample All Layers off, one click at (400, 100). The bounds are
//! `(left, top, right, bottom)` with `right`/`bottom` exclusive, then fully selected pixels and
//! partly selected pixels:
//!
//! | brush | Photoshop | PhotoCraft on `main` | with the click trim |
//! |---|---|---|---|
//! | 5 | `396, 96, 403, 103` (7 px wide) - 9 full - 13 partial | `395, 95, 405, 105` (10) - 36 - 64 | 6 - 4 - 32 |
//! | 30 | `383, 83, 416, 116` (33) - 619 - 68 | `381, 81, 419, 119` (38) - 996 - 288 | 30 - 580 - 224 |
//! | 100 | `348, 48, 451, 151` (103) - 7519 - 341 | `341, 41, 459, 159` (118) - 10244 - 928 | 102 - 7508 - 800 |
//!
//! So Photoshop's disc is the brush diameter plus 3 px at every size, while ours grew 5, 8 and
//! 18 px: the footprint growth scaled with the brush. The partly selected ring is still wider than
//! Photoshop's: it is the bilinear upsample of the half-resolution mask (two document pixels at
//! 64 / 128 / 191 against Photoshop's ~1 px), which is #12920's soft-rim case.

use photocraft_geom::Rect;

use super::quick::quick_select;
use super::{ImageSampler, Region, RgbImage, Rng};

/// The issue's test document: 800 x 600 white.
fn white_document() -> RgbImage {
    RgbImage::from_fn(800, 600, |_, _| [1.0, 1.0, 1.0])
}

/// One click at (400, 100) on the uniform document.
fn click(img: &RgbImage, at: (f32, f32), size: f32) -> Region {
    let s = ImageSampler { img, origin: (0, 0) };
    quick_select(&s, Rect::new(0, 0, img.w as i32, img.h as i32), &[at], size).expect("a selection")
}

/// `(width, height, fully selected pixels, partly selected pixels)` of a region.
fn measure(r: &Region) -> (i32, i32, usize, usize) {
    let (mut full, mut part) = (0, 0);
    for v in &r.mask {
        if *v == 255 {
            full += 1;
        } else if *v > 0 {
            part += 1;
        }
    }
    (r.bbox.width() as i32, r.bbox.height() as i32, full, part)
}

/// One click on white: `(width, full, partial)` for a `size` brush.
fn disc(size: f32) -> (i32, usize, usize) {
    let (w, h, full, part) = measure(&click(&white_document(), (400.0, 100.0), size));
    assert_eq!(w, h, "a single click gives a round disc, got {w} x {h}");
    (w, full, part)
}

/// Photoshop's measured bounds: the disc is the brush diameter plus 3 px, at every size.
#[test]
fn quick_select_a_click_on_a_uniform_area_selects_the_brush_diameter_plus_three() {
    // Measured widths: Photoshop 7, 33 and 103 for brushes 5, 30 and 100. Half-resolution
    // working pixels put the disc on a 2 px grid, so PhotoCraft lands 1 px inside at sizes 5 and
    // 100 and 3 px inside at size 30, where the half-resolution footprint of a 30 px brush is
    // 26 pixels across (the pixel whose centre sits exactly on the rim is left out).
    let img = white_document();
    for (size, want) in [(5.0f32, 7i32), (30.0, 33), (100.0, 103)] {
        let r = click(&img, (400.0, 100.0), size);
        let (w, h, full, part) = measure(&r);
        assert!(
            w - size as i32 <= 3 && h - size as i32 <= 3 && w >= size as i32 && h >= size as i32,
            "size {size}: bounds {:?}, {w} x {h} (Photoshop {want}), {full} full, {part} partial",
            r.bbox
        );
        let (cx, cy) = ((r.bbox.x0 + r.bbox.x1) as f32 / 2.0, (r.bbox.y0 + r.bbox.y1) as f32 / 2.0);
        assert!((cx - 400.0).abs() <= 1.5 && (cy - 100.0).abs() <= 1.5, "size {size}: disc centre ({cx}, {cy}), bounds {:?}", r.bbox);
    }
    assert_eq!(disc(5.0).0, 6);
    assert_eq!(disc(30.0).0, 30);
    assert_eq!(disc(100.0).0, 102);
}

/// The extra width does not grow with the brush: Photoshop's margin is 2, 3 and 3 px for brushes
/// 5, 30 and 100; PhotoCraft's was 5, 8 and 18 px because the footprint growth was
/// `grow + grow_frac * radius`.
#[test]
fn quick_select_the_disc_margin_does_not_grow_with_the_brush_size() {
    let margins: Vec<i32> = [5.0f32, 15.0, 30.0, 50.0, 100.0].iter().map(|&size| disc(size).0 - size as i32).collect();
    assert!(margins.iter().all(|m| (0..=3).contains(m)), "margins {margins:?} for sizes 5, 15, 30, 50, 100");
    assert!(margins.iter().max().unwrap() - margins.iter().min().unwrap() <= 3, "margins {margins:?} span more than 3 px");
}

/// Photoshop's fully selected pixel counts, 9 / 619 / 7519 for brushes 5 / 30 / 100.
#[test]
fn quick_select_the_disc_core_matches_photoshop_pixel_counts() {
    // Size 100: Photoshop 7519 fully selected, within 5 % (619 and 9 at sizes 30 and 5 cannot be
    // reached at half resolution: the fully selected core there jumps from 580 to 740 pixels in
    // steps of one working pixel, and 619 sits between the two).
    let (w30, f30, p30) = disc(30.0);
    let (w100, f100, p100) = disc(100.0);
    assert!((7519 - 7519 / 20..=7519 + 7519 / 20).contains(&f100), "size 100: {f100} fully selected, {p100} partly (Photoshop 7519 within 5 %), width {w100}");
    assert_eq!((w30, f30, p30), (30, 580, 224), "size 30: Photoshop 33 wide, 619 full, 68 partial");
}

/// The partly selected ring: it comes with the footprint now (no extra growth), and it is still
/// the bilinear upsample of the half-resolution mask - values 16 / 48 / 64 / 112 / 143 / 191 /
/// 207 / 239, a ring two document pixels wide. Photoshop's own rim is thinner (341 partly selected
/// pixels against our 800 at size 100): that is #12920's case, not this one.
#[test]
fn quick_select_the_soft_rim_of_a_uniform_disc_stays_the_bilinear_step() {
    let r = click(&white_document(), (400.0, 100.0), 100.0);
    let mut values: Vec<u8> = r.mask.iter().copied().filter(|v| *v > 0 && *v < 255).collect();
    values.sort_unstable();
    values.dedup();
    assert_eq!(values, [16, 48, 64, 112, 143, 191, 207, 239], "rim values");
    // The ring no longer carries the footprint growth: 800 partly selected pixels, was 928.
    assert_eq!(measure(&r).3, 800, "partly selected pixels at size 100");
    assert_eq!(measure(&click(&white_document(), (400.0, 100.0), 30.0)).3, 224, "partly selected pixels at size 30");
}

/// Neighbour (a brush far smaller than the working pixel): the click keeps a small disc, it does
/// not vanish and it does not reach the footprint growth of a large brush.
#[test]
fn quick_select_a_one_pixel_brush_selects_a_small_patch() {
    for size in [1.0f32, 2.0] {
        let (w, h, full, part) = measure(&click(&white_document(), (400.0, 100.0), size));
        assert_eq!((w, h, full, part), (4, 4, 0, 16), "size {size}");
    }
}

/// Neighbour (a click at the canvas edge): the disc is clipped by the canvas, and the canvas edge
/// is not an edge the selection grows to.
#[test]
fn quick_select_a_click_at_the_canvas_edge_stays_a_clipped_disc() {
    let (w, h, full, part) = measure(&click(&white_document(), (795.0, 100.0), 100.0));
    assert_eq!((w, h), (55, 102), "clipped at x = 800");
    assert_eq!((full, part), (4202, 416));
}

/// Neighbour (a document that works at full resolution, *not measured* by the issue): the click
/// keeps its footprint there too; with no half-resolution mask there is no rim, so the disc is the
/// brush diameter exactly.
#[test]
fn quick_select_a_click_on_a_small_uniform_document_selects_the_brush_disc() {
    let img = RgbImage::from_fn(400, 400, |_, _| [1.0, 1.0, 1.0]);
    let (w, h, full, part) = measure(&click(&img, (200.0, 200.0), 30.0));
    assert_eq!((w, h), (30, 30), "small document, size 30");
    assert_eq!((full, part), (716, 0), "small document, size 30");
}

/// Neighbour (a slightly noisy area): the disc stays a disc, so the smaller reported footprint
/// does not turn a click on a textured uniform area into an unbounded leak.
#[test]
fn quick_select_a_click_on_a_noisy_uniform_area_stays_a_disc() {
    let mut rng = Rng::new(11);
    let img = RgbImage::from_fn(800, 600, |_, _| {
        let n: f32 = 0.01 * rng.normal();
        [1.0 - n.abs(), 1.0 - n.abs(), 1.0 - n.abs()]
    });
    let (w, _, full, part) = measure(&click(&img, (400.0, 100.0), 30.0));
    assert_eq!((w, full, part), (30, 580, 224), "size 30 on a slightly noisy white field");
}

/// Neighbour (a drag, *not measured* by the issue): a two-point stroke keeps today's result,
/// footprint growth included. Guard test: it passes before and after the change on purpose.
#[test]
fn quick_select_a_drag_keeps_the_settled_footprint() {
    let s = ImageSampler { img: &white_document(), origin: (0, 0) };
    let r = quick_select(&s, Rect::new(0, 0, 800, 600), &[(360.0, 100.0), (440.0, 100.0)], 30.0).expect("a selection");
    assert_eq!((r.bbox.width() as i32, r.bbox.height() as i32), (118, 42), "drag bbox");
    assert_eq!(measure(&r).2, 4044, "drag fully selected pixels");
}

/// Neighbour (the issue's second *Done when* bullet, measured): a shape with clear edges still
/// grows to those edges. Guard test: passes before and after the change on purpose.
#[test]
fn quick_select_a_click_inside_a_rectangle_still_takes_the_whole_rectangle() {
    // A 200 x 150 red rectangle on white, as the issue's second bullet names.
    let img = RgbImage::from_fn(800, 600, |x, y| if (300..500).contains(&x) && (200..350).contains(&y) { [0.78, 0.23, 0.23] } else { [1.0, 1.0, 1.0] });
    let r = click(&img, (400.0, 275.0), 30.0);
    for y in 300..350 {
        for x in 300..500 {
            assert!(r.at(x, y) >= 0.5, "({x},{y}) inside the rectangle not selected");
        }
    }
    for y in 0..600 {
        for x in 0..800 {
            if !(300..500).contains(&x) || !(200..350).contains(&y) {
                assert!(r.at(x, y) < 0.5, "leaked outside the rectangle at ({x},{y})");
            }
        }
    }
}
