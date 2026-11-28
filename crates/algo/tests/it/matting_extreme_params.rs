//! Refine-edge parameters far outside the UI range must not overflow the halo/spread sums.
use photocraft_algo::matting::RefineParams;

#[test]
fn halo_and_spread_survive_extreme_params() {
    for v in [1e30_f32, f32::MAX, f32::INFINITY] {
        let p = RefineParams { radius: v, feather: v, smooth: v, smart_radius: true, ..RefineParams::default() };
        assert_eq!(p.halo(), i32::MAX);
        assert_eq!(p.spread(), i32::MAX);
    }
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
