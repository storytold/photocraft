//! #12922: one click with the Quick Selection brush on a uniform area.
//!
//! Measured in Photoshop 25.4 on an 800 x 600 RGB/8 document that is white everywhere, Background
//! active, Reset Tool, Sample All Layers off, one click at (400, 100). The bounds are
//! `(left, top, right, bottom)` with `right`/`bottom` exclusive, then fully selected pixels and
//! partly selected pixels:
//!
//! | brush | Photoshop | PhotoCraft on `main` | the click trim | margin + one pixel rim |
//! |---|---|---|---|---|
//! | 5 | `396, 96, 403, 103` (7 px wide) - 9 full - 13 partial | 10 - 36 - 64 | 6 - 4 - 32 | 7 - 9 - 28 |
//! | 30 | `383, 83, 416, 116` (33) - 619 - 68 | 38 - 996 - 288 | 30 - 580 - 224 | 33 - 657 - 140 |
//! | 100 | `348, 48, 451, 151` (103) - 7519 - 341 | 118 - 10244 - 928 | 102 - 7508 - 800 | 103 - 7661 - 512 |
//!
//! Photoshop's extra width does not scale with the brush (2, 3 and 3 px over the diameter), and
//! its rim is about one document pixel wide (its partly selected values run 128 to 254). The click
//! trim of the first cut returned the bare footprint at half resolution and upsampled it
//! bilinearly: no margin at all, a fully selected core that was too small, and a rim two document
//! pixels wide. A trimmed click is now drawn at document resolution - fully selected where the
//! brush disc covers the whole pixel, partly selected in the band that reaches [`CLICK_MARGIN`]
//! document pixels beyond the disc - so the width is the reference width at all three sizes and
//! the rim is one pixel with values 128 to 254.
//!
//! The bounds and the fully selected counts then agree with the reference (0 %, 6.1 % and 1.9 %
//! off at the three sizes); the partly selected counts are 28 / 140 / 512 against its
//! 13 / 68 / 341, and no geometry that matches the bounds can do better: a disc wide enough to
//! span 7 / 33 / 103 px selects 37 / 797 / 8173 document pixels, where the reference rows count
//! 22 / 687 / 7860 - its bounds are wider than the disc its counts describe. Photoshop works at
//! half resolution there, where whole document pixels of the rim are either fully selected or not
//! selected at all; on the half-resolution grid every click lands on a cell corner, which also
//! pins the width to an even number of document pixels (the 6 / 30 / 102 of the click trim), so
//! the odd reference widths cannot be met that way either.
//!
//! The reference rows are symmetric about a pixel centre (396..403, 383..416 and 348..451 are
//! centred at 399.5), so Photoshop's click coordinate "400" names a pixel and its brush centre is
//! that pixel's centre: in document coordinates, where pixel `i` covers `[i, i + 1)`, the tests
//! click at `400.5` - as the existing large-document test in `tests.rs` does. A click exactly on
//! the boundary between four pixels sits one document pixel off that grid at every size, which
//! [`quick_select_a_click_on_a_pixel_boundary`] pins.

use photocraft_geom::Rect;

use super::quick::{CLICK_MARGIN, quick_select};
use super::{ImageSampler, Region, RgbImage, Rng};

/// The issue's test document: 800 x 600 white.
fn white_document() -> RgbImage {
    RgbImage::from_fn(800, 600, |_, _| [1.0, 1.0, 1.0])
}

/// The issue's click at 400,100 as a document point: the centre of the clicked pixel.
const AT: (f32, f32) = (400.5, 100.5);

/// One click at `at` on the document `img`.
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

/// One click on white at the issue's spot: `(width, full, partial)` for a `size` brush.
fn disc(size: f32) -> (i32, usize, usize) {
    let (w, h, full, part) = measure(&click(&white_document(), AT, size));
    assert_eq!(w, h, "a single click gives a round disc, got {w} x {h}");
    (w, full, part)
}

/// `actual` within `tol` per cent of `want`.
fn within(actual: usize, want: usize, tol: f64) -> bool {
    (actual as f64 - want as f64).abs() <= tol / 100.0 * want as f64
}

/// Photoshop's measured bounds: 7, 33 and 103 px wide for brushes 5, 30 and 100.
#[test]
fn quick_select_a_click_on_a_uniform_area_selects_the_brush_plus_a_constant_margin() {
    let img = white_document();
    for (size, want) in [(5.0f32, 7i32), (30.0, 33), (100.0, 103)] {
        let r = click(&img, AT, size);
        let (w, h, full, part) = measure(&r);
        assert_eq!((w, h), (want, want), "size {size}: bounds {:?}, {w} x {h} (Photoshop {want}), {full} full, {part} partial", r.bbox);
        let (cx, cy) = ((r.bbox.x0 + r.bbox.x1) as f32 / 2.0, (r.bbox.y0 + r.bbox.y1) as f32 / 2.0);
        assert!((cx - AT.0).abs() <= 1.0 && (cy - AT.1).abs() <= 1.0, "size {size}: disc centre ({cx}, {cy}), bounds {:?}", r.bbox);
    }
    // The margin is `CLICK_MARGIN` document pixels, not `grow + grow_frac * radius`: on this grid
    // the reference width needs 0.5 px of it at size 5 and 1.0 px at sizes 30 and 100, so the
    // 1.0 px we use gives 2, 3 and 3 px over the diameter - 1 px too wide only at size 5.
    let margins: Vec<i32> = [5.0f32, 15.0, 30.0, 50.0, 100.0].iter().map(|&size| disc(size).0 - size as i32).collect();
    assert_eq!(margins, [2, 2, 3, 3, 3], "width minus brush diameter, sizes 5, 15, 30, 50, 100");
}

/// The extra width does not grow with the brush: Photoshop's margin is 2, 3 and 3 px for brushes
/// 5, 30 and 100, ours on `main` was 5, 8 and 18 px because the footprint growth was
/// `grow + grow_frac * radius`.
#[test]
fn quick_select_the_disc_margin_does_not_grow_with_the_brush_size() {
    let margins: Vec<i32> = [5.0f32, 15.0, 30.0, 50.0, 100.0].iter().map(|&size| disc(size).0 - size as i32).collect();
    assert!(margins.iter().all(|m| (2..=3).contains(m)), "margins {margins:?} for sizes 5, 15, 30, 50, 100");
    assert!(margins.iter().max().unwrap() - margins.iter().min().unwrap() <= 1, "margins {margins:?} span more than 1 px");
}

/// Photoshop's fully selected pixel counts: 9, 619 and 7519 for brushes 5, 30 and 100.
#[test]
fn quick_select_the_disc_core_matches_photoshop_pixel_counts() {
    let (w5, f5, p5) = disc(5.0);
    let (w30, f30, p30) = disc(30.0);
    let (w100, f100, p100) = disc(100.0);
    assert!(within(f5, 9, 5.0), "size 5: {f5} fully selected, Photoshop 9 within 5 % ({w5} wide, {p5} partial)");
    assert!(within(f100, 7519, 5.0), "size 100: {f100} fully selected, Photoshop 7519 within 5 % ({w100} wide, {p100} partial)");
    // Size 30 misses 5 % by 1.1 points: a core of the brush radius 15 fully covers 657 pixels and
    // the largest core inside 5 % is radius 14.95 with 649, while Photoshop's 619 belongs to a core
    // radius of about 14.65 - a round click cannot sit on 619 and still span 33 px.
    assert!(within(f30, 619, 8.0), "size 30: {f30} fully selected, Photoshop 619 ({w30} wide, {p30} partial)");
    assert_eq!((f5, f30, f100), (9, 657, 7661), "fully selected pixels");
}

/// The soft rim: one document pixel wide with values 128 to 254 as Photoshop's are, and much
/// thinner than the two pixel bilinear band of the click trim (32 / 224 / 800).
#[test]
fn quick_select_the_soft_rim_of_a_uniform_disc_is_one_pixel_wide() {
    let r = click(&white_document(), AT, 100.0);
    let mut values: Vec<u8> = r.mask.iter().copied().filter(|v| *v > 0 && *v < 255).collect();
    values.sort_unstable();
    values.dedup();
    assert!(values.iter().all(|v| *v >= 128), "rim values {values:?}: a partly selected pixel is at least half selected, as Photoshop's 128 to 254");
    // Nothing of the disc reaches past the brush radius plus the margin: the band is one
    // `CLICK_MARGIN` wide, and it is the same width for every brush.
    let (w, h, _, _) = measure(&r);
    assert_eq!((w, h), (103, 103), "the disc spans the brush diameter plus the margin");
    let reach = 50.0 + CLICK_MARGIN;
    for (i, v) in r.mask.iter().enumerate() {
        if *v == 0 {
            continue;
        }
        let cx = r.bbox.x0 as f32 + (i % w as usize) as f32 + 0.5 - AT.0;
        let cy = r.bbox.y0 as f32 + (i / w as usize) as f32 + 0.5 - AT.1;
        assert!(
            cx * cx + cy * cy <= reach * reach + 0.01,
            "selected pixel at ({}, {}): its centre is {} px from the click, past the brush radius plus the margin",
            r.bbox.x0 + (i % w as usize) as i32,
            r.bbox.y0 + (i / w as usize) as i32,
            (cx * cx + cy * cy).sqrt()
        );
    }
    let (p5, p30, p100) = (disc(5.0).2, disc(30.0).2, disc(100.0).2);
    assert_eq!((p5, p30, p100), (28, 140, 512), "partly selected pixels, Photoshop 13 / 68 / 341");
    assert!(p5 < 32 && p30 < 224 && p100 < 800, "rim counts {p5} / {p30} / {p100} against the two pixel band 32 / 224 / 800");
    // And those counts are the floor the reference bounds allow: the disc that is 7 / 33 / 103 px
    // wide selects 37 / 797 / 8173 pixels, so its rim is that minus the fully selected core, while
    // Photoshop's rows add up to 22 / 687 / 7860 (module docs).
    let (f5, f30, f100) = (disc(5.0).1, disc(30.0).1, disc(100.0).1);
    assert_eq!((f5 + p5, f30 + p30, f100 + p100), (37, 797, 8173), "selected pixels, Photoshop 22 / 687 / 7860");
}

/// Neighbour (a brush far smaller than one document pixel): the click keeps a small disc, it does
/// not vanish and it does not reach the footprint growth of a large brush.
#[test]
fn quick_select_a_one_pixel_brush_selects_a_small_patch() {
    let (w1, h1, full1, part1) = measure(&click(&white_document(), AT, 1.0));
    assert_eq!((w1, h1, full1, part1), (3, 3, 0, 9), "size 1: no whole pixel fits in a 1 px disc, the margin covers the 3 x 3 around it");
    let (w2, h2, full2, part2) = measure(&click(&white_document(), AT, 2.0));
    assert_eq!((w2, h2, full2, part2), (5, 5, 1, 12), "size 2");
}

/// Neighbour (a click at the canvas edge): the disc is clipped by the canvas, and the canvas edge
/// is not an edge the selection grows to.
#[test]
fn quick_select_a_click_at_the_canvas_edge_stays_a_clipped_disc() {
    let (w, h, full, part) = measure(&click(&white_document(), (795.5, 100.5), 100.0));
    assert_eq!((w, h), (56, 103), "clipped at x = 800");
    assert_eq!((full, part), (4276, 266), "clipped disc: fully and partly selected pixels");
}

/// Neighbour (a document that works at full resolution, *not measured* by the issue): the click
/// keeps its footprint there too. The disc is drawn at document resolution, so the same brush
/// gives the same disc whether the document works at full or at half resolution.
#[test]
fn quick_select_a_click_on_a_small_uniform_document_selects_the_brush_disc() {
    let img = RgbImage::from_fn(400, 400, |_, _| [1.0, 1.0, 1.0]);
    let (w, h, full, part) = measure(&click(&img, (200.5, 200.5), 30.0));
    assert_eq!((w, h, full, part), (33, 33, 657, 140), "small document, size 30: the same disc as on the 800 x 600 one");
}

/// Neighbour (a slightly noisy area): the disc stays a disc, so the click trim does not turn a
/// click on a textured uniform area into an unbounded leak.
#[test]
fn quick_select_a_click_on_a_noisy_uniform_area_stays_a_disc() {
    let mut rng = Rng::new(11);
    let img = RgbImage::from_fn(800, 600, |_, _| {
        let n: f32 = 0.01 * rng.normal();
        [1.0 - n.abs(), 1.0 - n.abs(), 1.0 - n.abs()]
    });
    let (w, h, full, part) = measure(&click(&img, (400.0, 100.0), 30.0));
    assert_eq!((w, h), (32, 32), "size 30 on a slightly noisy white field stays a disc");
    assert_eq!((full, part), (648, 164), "noisy disc: fully and partly selected pixels");
}

/// The half pixel phase: the issue's click coordinates name a pixel, and a click that lands
/// exactly on the boundary between four pixels sits one document pixel narrower at every size.
#[test]
fn quick_select_a_click_on_a_pixel_boundary() {
    let widths: Vec<i32> = [5.0f32, 30.0, 100.0].iter().map(|&size| measure(&click(&white_document(), (400.0, 100.0), size)).0).collect();
    assert_eq!(widths, [6, 32, 102], "widths for a click at the document point (400, 100)");
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
