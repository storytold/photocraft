//! #12921: on a layer whose shapes sit on transparency, a Quick Selection click or drag on a soft
//! grey ramp (ten 40 px blocks, 100 to 145 in 5-level steps) selects the whole ramp, as the
//! reference application does and as the composite already did, instead of stopping at the steps.
//! Transparency is an edge for the cut. A click on a red rectangle still selects it alone.

use photocraft_geom::Rect;

use super::{CutSampler, quick_select};
use crate::segment::{Region, Sampler};

/// The issue's layer: 800 × 600, transparent but for red 1 (100,100–300,250), red 2
/// (500,100–600,200) and the grey ramp at y 350–450 (ten 40 px blocks from x 100, values 100,
/// 105, …, 145). `composite` puts it over white, as the document's composite reads.
struct Layer {
    composite: bool,
}

const RAMP: Rect = Rect { x0: 100, y0: 350, x1: 500, y1: 450 };
const RED1: Rect = Rect { x0: 100, y0: 100, x1: 300, y1: 250 };
const RED2: Rect = Rect { x0: 500, y0: 100, x1: 600, y1: 200 };
const CANVAS: Rect = Rect { x0: 0, y0: 0, x1: 800, y1: 600 };

impl Layer {
    fn px(&self, x: i32, y: i32) -> [f32; 4] {
        let red = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        if RED1.contains(x, y) || RED2.contains(x, y) {
            return red;
        }
        if RAMP.contains(x, y) {
            let v = (100 + 5 * ((x - 100) / 40)) as f32 / 255.0;
            return [v, v, v, 1.0];
        }
        if self.composite { [1.0; 4] } else { [0.0; 4] }
    }
}

impl Sampler for Layer {
    fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
        (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).map(|(x, y)| self.px(x, y)).collect()
    }
}

fn select(sampler: &dyn Sampler, points: &[(f32, f32)], size: f32) -> Region {
    quick_select(&CutSampler(sampler), CANVAS, points, size).expect("the stroke is on the canvas")
}

/// Every pixel of the ramp selected: fully, 2 px in from its outline (the outline itself is soft
/// at the half resolution an 800 × 600 document is cut at).
fn covers_ramp(r: &Region) -> bool {
    (RAMP.y0 + 2..RAMP.y1 - 2).step_by(3).all(|y| (RAMP.x0 + 2..RAMP.x1 - 2).step_by(3).all(|x| r.at(x, y) == 1.0))
}

/// The whole ramp and nothing a pixel past it.
fn is_whole_ramp(r: &Region) -> bool {
    let inside = covers_ramp(r);
    let b = r.bbox;
    inside && (b.x0 - RAMP.x0).abs() <= 1 && (b.y0 - RAMP.y0).abs() <= 1 && (b.x1 - RAMP.x1).abs() <= 1 && (b.y1 - RAMP.y1).abs() <= 1
}

#[test]
fn a_click_on_the_first_block_selects_the_whole_ramp() {
    let r = select(&Layer { composite: false }, &[(120.0, 400.0)], 30.0);
    assert!(is_whole_ramp(&r), "bounds {:?}", r.bbox);
}

#[test]
fn a_click_on_the_120_block_selects_the_whole_ramp() {
    let r = select(&Layer { composite: false }, &[(180.0, 400.0)], 30.0);
    assert!(is_whole_ramp(&r), "bounds {:?}", r.bbox);
}

#[test]
fn a_drag_along_the_ramp_selects_the_whole_ramp() {
    let drag: Vec<(f32, f32)> = (0..=16).map(|i| (180.0 + 10.0 * i as f32, 400.0)).collect();
    let r = select(&Layer { composite: false }, &drag, 30.0);
    assert!(is_whole_ramp(&r), "bounds {:?}", r.bbox);
}

#[test]
fn size_100_on_the_first_block_selects_the_whole_ramp() {
    // The whole ramp. The 100 px brush disc also reaches over the ramp's outline onto
    // transparency, and the footprint is always selected; dropping transparent pixels from the
    // result is #12918, so here only the ramp and the disc may be selected.
    let r = select(&Layer { composite: false }, &[(120.0, 400.0)], 100.0);
    assert!(covers_ramp(&r), "bounds {:?}", r.bbox);
    // The ramp's outline is soft by a pixel either side.
    let past = |x: i32, y: i32| r.at(x, y) > 0.0 && !RAMP.inflate(1).contains(x, y);
    // The footprint is the 50 px radius grown by `grow + grow_frac * radius` working pixels
    // (about 8.5 document pixels at half resolution), plus the soft half-resolution edge.
    let disc = |x: i32, y: i32| ((x as f32 - 120.0).powi(2) + (y as f32 - 400.0).powi(2)).sqrt() <= 62.0;
    assert!((CANVAS.y0..CANVAS.y1).all(|y| (CANVAS.x0..CANVAS.x1).all(|x| !past(x, y) || disc(x, y))), "nothing past the ramp but the brush disc");
}

#[test]
fn a_click_on_red_1_selects_red_1_alone() {
    let r = select(&Layer { composite: false }, &[(200.0, 175.0)], 30.0);
    let b = r.bbox;
    assert!((b.x0 - RED1.x0).abs() <= 1 && (b.y0 - RED1.y0).abs() <= 1 && (b.x1 - RED1.x1).abs() <= 1 && (b.y1 - RED1.y1).abs() <= 1, "{b:?}");
    assert_eq!(r.at(550, 150), 0.0, "red 2 stays unselected");
    assert_eq!(r.at(120, 400), 0.0, "the ramp stays unselected");
}

#[test]
fn the_composite_reads_as_before() {
    // Opaque everywhere: the cut sampler changes nothing, the ramp was already whole.
    let layer = Layer { composite: true };
    let plain = quick_select(&layer, CANVAS, &[(180.0, 400.0)], 30.0).unwrap();
    let cut = select(&layer, &[(180.0, 400.0)], 30.0);
    assert_eq!((plain.bbox, &plain.mask), (cut.bbox, &cut.mask));
    assert!(is_whole_ramp(&cut));
}
