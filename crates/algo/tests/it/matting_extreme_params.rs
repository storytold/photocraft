//! Refine-edge parameters far outside the UI range must not overflow the halo/spread sums.
use photocraft_algo::matting::{EdgeBrush, EdgeStroke, RefineParams, refine_mask_brushed};
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
