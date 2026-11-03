//! A filter parameter far past anything a user can type must not overflow the halo arithmetic.

use photocraft_algo::{FilterParams, Halo};

const EXTREME: [f32; 8] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, f32::MIN, 1e30, 3.0e9, 2_147_483_648.0];

fn radius(h: Halo) -> i32 {
    match h {
        Halo::Radius(r) => r,
        other => panic!("expected a radius, got {other:?}"),
    }
}

#[test]
fn motion_blur_halo_survives_extreme_distance() {
    for v in EXTREME {
        let r = radius(FilterParams::MotionBlur { angle: 0.0, distance: v }.halo());
        assert!(r >= 0, "distance {v}: halo {r}");
    }
}

#[test]
fn emboss_halo_survives_extreme_height() {
    for v in EXTREME {
        let r = radius(FilterParams::Emboss { angle: 0.0, height: v, amount: 100.0 }.halo());
        assert!(r >= 0, "height {v}: halo {r}");
    }
}
