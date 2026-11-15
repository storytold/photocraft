//! Outer Glow at Range 1 % (#14958): the same document with a black Softer glow, size 10, Spread 0.
//! The reference's R channel outwards from the edge (no glow = 255).

use super::inner_glow_range_one_tests::{close, glow, profiles, rect};
use super::*;

const RANGE_1: [i32; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 39, 255];
const RANGE_25: [i32; 10] = [0, 0, 0, 42, 103, 153, 194, 225, 245, 255];
const RANGE_50: [i32; 10] = [26, 72, 113, 148, 179, 204, 225, 240, 250, 255];
const RANGE_100: [i32; 10] = [140, 163, 184, 201, 217, 229, 240, 247, 252, 255];

#[test]
fn range_one_is_eight_solid_pixels_then_a_light_one() {
    let m = glow_map(&rect(), &glow(0.01), false, (0, 0));
    for (i, p) in profiles(&m, false, 255.0).iter().enumerate() {
        assert!(close(p, &RANGE_1, 3), "edge {i}: {p:?}");
    }
}

#[test]
fn ranges_25_50_100_are_unchanged() {
    for (range, want) in [(0.25, RANGE_25), (0.5, RANGE_50), (1.0, RANGE_100)] {
        let m = glow_map(&rect(), &glow(range), false, (0, 0));
        for (i, p) in profiles(&m, false, 255.0).iter().enumerate() {
            assert!(close(p, &want, 4), "range {range} edge {i}: {p:?}");
        }
    }
}
