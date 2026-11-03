//! Refine-edge parameters far outside the UI range must not overflow the halo/spread sums.
use photocraft_algo::matting::RefineParams;

#[test]
fn halo_and_spread_survive_extreme_params() {
    for v in [1e30_f32, f32::MAX, f32::INFINITY] {
        let p = RefineParams { radius: v, feather: v, smooth: v, smart_radius: true, ..RefineParams::default() };
        assert!(p.halo() >= 0);
        assert!(p.spread() >= 0);
    }
}
