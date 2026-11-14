//! Refine-edge parameters far outside the UI range must not overflow the halo/spread sums.
use photocraft_algo::matting::{EdgeBrush, EdgeStroke, RefineParams, refine_mask, refine_mask_brushed};
use photocraft_algo::segment::{ImageSampler, RgbImage};
use photocraft_geom::Rect;

#[test]
fn halo_and_spread_survive_extreme_params() {
    for v in [1e30_f32, f32::MAX, f32::INFINITY] {
        let p = RefineParams { radius: v, feather: v, smooth: v, smart_radius: true, ..RefineParams::default() };
        assert_eq!(p.halo(), i32::MAX);
        assert_eq!(p.spread(), i32::MAX);
    }
}

#[test]
fn brushed_refine_survives_extreme_params() {
    let canvas = Rect::new(0, 0, 64, 64);
    let img = RgbImage::new(64, 64);
    let sampler = ImageSampler { img: &img, origin: (0, 0) };
    let stroke = EdgeStroke { points: vec![(32.0, 32.0)], size: 20.0, add: true };
    let brush = EdgeBrush::new(&[stroke], canvas);
    let mask = |r: Rect| vec![1.0_f32; r.width() as usize * r.height() as usize];
    let p = RefineParams { radius: 1e30, feather: f32::MAX, smooth: f32::MAX, ..RefineParams::default() };
    let _ = refine_mask_brushed(&sampler, &mask, Rect::new(16, 16, 48, 48), canvas, &p, brush.as_ref());
}

#[test]
fn halo_and_spread_add_every_term_for_moderate_params() {
    // radius 5 -> guided-filter radius 5, so the edge term is max(5, 2 * 5) = 10; smooth 50 and
    // feather 4 give sigmas 2 and 2, so 3 * sigma = 6 each.
    let p = RefineParams { radius: 5.0, smooth: 50.0, feather: 4.0, ..RefineParams::default() };
    assert_eq!(p.halo(), 10 + 6 + 6 + 2);
    assert_eq!(p.spread(), 5 + 6 + 6 + 1);
}

#[test]
fn feather_nan_adds_nothing_and_feather_is_capped() {
    // Same radius and smooth as above (edge term 10, smooth term 6); a NaN feather adds nothing.
    let p = RefineParams { radius: 5.0, smooth: 50.0, feather: f32::NAN, ..RefineParams::default() };
    assert_eq!(p.halo(), 10 + 6 + 2);
    assert_eq!(p.spread(), 5 + 6 + 1);
    // Feather is capped at 1000 px (sigma 500, so 3 * sigma = 1500) at and above the cap.
    for feather in [1000.0, 2000.0] {
        let p = RefineParams { radius: 5.0, smooth: 50.0, feather, ..RefineParams::default() };
        assert_eq!(p.halo(), 10 + 6 + 1500 + 2);
        assert_eq!(p.spread(), 5 + 6 + 1500 + 1);
    }
}

#[test]
fn feather_nan_leaves_the_mask_unblurred() {
    // A NaN feather must act as no feather, not turn every refined pixel into NaN (unselected).
    let canvas = Rect::new(0, 0, 64, 64);
    let img = RgbImage::new(64, 64);
    let sampler = ImageSampler { img: &img, origin: (0, 0) };
    // A selected square on an unselected canvas, so the tiles are mixed and actually refined.
    let content = Rect::new(16, 16, 48, 48);
    let mask = |r: Rect| (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| if content.contains(x, y) { 1.0_f32 } else { 0.0 })).collect();
    let p = RefineParams { radius: 0.0, smooth: 0.0, feather: f32::NAN, ..RefineParams::default() };
    let out = refine_mask(&sampler, &mask, content, canvas, &p).expect("refined mask");
    assert!(out.at(32, 32) > 0.99, "centre coverage {}", out.at(32, 32));
}
