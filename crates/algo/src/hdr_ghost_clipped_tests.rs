//! #14020: Remove ghosts catches a moved bright object even where its pixels are clipped or black
//! in every frame (so no frame is well exposed there), and leaves unmoved areas alone.

use super::*;

const W: usize = 300;
const H: usize = 200;

/// The issue's scene in linear light: a grey ramp (y 0-99), a mid-grey band (y 100-119) and red,
/// green, blue, white and black 60 x 60 patches at y 120-179 (x 0/60/120/180/240), on mid grey.
/// `ghost`: the white patch moved 30 px right and no black patch.
fn scene(ghost: bool) -> Vec<[f32; 3]> {
    let mut px = vec![[0.0f32; 3]; W * H];
    for y in 0..H {
        for x in 0..W {
            let s = |v: f32| srgb_to_linear(v);
            let c = if y < 100 {
                let v = s(((x / 10) as f32 * 10.0 / 299.0).min(1.0));
                [v, v, v]
            } else if !(120..180).contains(&y) {
                [s(0.5); 3]
            } else {
                let (white, black) = if ghost { (210..270, 300..300) } else { (180..240, 240..300) };
                match x {
                    _ if white.contains(&x) => [1.0, 1.0, 1.0],
                    _ if black.contains(&x) => [0.0, 0.0, 0.0],
                    0..60 => [s(0.8), s(0.05), s(0.05)],
                    60..120 => [s(0.05), s(0.8), s(0.05)],
                    120..180 => [s(0.05), s(0.05), s(0.8)],
                    _ => [s(0.5); 3],
                }
            };
            px[y * W + x] = c;
        }
    }
    px
}

/// One frame made with Exposure `ev`: the scene times 2^ev in linear light, clipped, as 8-bit sRGB.
fn frame(scene: &[[f32; 3]], ev: f32) -> Vec<[f32; 4]> {
    scene
        .iter()
        .map(|c| {
            let e = |v: f32| (linear_to_srgb((v * 2f32.powf(ev)).clamp(0.0, 1.0)) * 255.0).round() / 255.0;
            [e(c[0]), e(c[1]), e(c[2]), 1.0]
        })
        .collect()
}

/// The merge of frames EV -2, 0 and +2 (the +2 one moved when `ghost`), base EV -2.
fn run(ghost: bool, remove_ghosts: bool) -> Merged {
    let still = scene(false);
    let frames = [frame(&still, -2.0), frame(&still, 0.0), frame(&scene(ghost), 2.0)];
    let refs: Vec<&[[f32; 4]]> = frames.iter().map(|f| f.as_slice()).collect();
    let opts = MergeOptions { exposures: vec![0.25, 1.0, 4.0], remove_ghosts, ghost_base: Some(0), response: None };
    merge(W, H, &refs, &opts)
}

/// A radiance pixel as 8-bit sRGB levels (the merge is scaled so the base keeps its values).
fn levels(q: [f32; 4]) -> [f32; 3] {
    [0, 1, 2].map(|c| linear_to_srgb(q[c].clamp(0.0, 1.0)) * 255.0)
}

fn diff(a: [f32; 4], b: [f32; 4]) -> f32 {
    let (a, b) = (levels(a), levels(b));
    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max)
}

fn moved(i: usize) -> bool {
    let (x, y) = (i % W, i / W);
    (120..180).contains(&y) && x >= 180
}

#[test]
fn the_moved_patch_comes_from_the_base_image() {
    let on = run(true, true);
    let off = run(true, false);
    // What the merge gives when nothing moved: the patch where the base image has it.
    let still = run(false, false);
    let at = |x: usize, y: usize| y * W + x;
    // Where the base has the black patch, and where the white patch was left behind.
    for (x, y) in [(270, 150), (250, 150), (195, 150)] {
        let i = at(x, y);
        assert!(diff(on.px[i], still.px[i]) <= 10.0, "({x},{y}): {:?} vs {:?}", levels(on.px[i]), levels(still.px[i]));
        assert!(diff(off.px[i], still.px[i]) > 10.0, "({x},{y}) was a ghost without the fix");
    }
    // The white patch shows in one place only: almost the whole moved area matches the still merge.
    let area: Vec<usize> = (0..W * H).filter(|&i| moved(i)).collect();
    let near = area.iter().filter(|&&i| diff(on.px[i], still.px[i]) <= 10.0).count();
    assert!(near * 100 >= area.len() * 95, "{near} of {}", area.len());
}

#[test]
fn unmoved_areas_are_left_alone() {
    let on = run(true, true);
    let off = run(true, false);
    let worst = (0..W * H).filter(|&i| !moved(i) && (i % W < 170 || i / W < 110)).map(|i| diff(on.px[i], off.px[i])).fold(0.0, f32::max);
    assert!(worst <= 2.0, "an unmoved pixel changed by {worst} levels");
}

#[test]
fn without_a_ghost_or_with_remove_ghosts_off_nothing_changes() {
    let still_on = run(false, true);
    let still_off = run(false, false);
    let worst = (0..W * H).map(|i| diff(still_on.px[i], still_off.px[i])).fold(0.0, f32::max);
    assert!(worst <= 2.0, "a ghost-free set changed by {worst} levels");
    assert_eq!(still_on.ghost_fraction, 0.0);
}
