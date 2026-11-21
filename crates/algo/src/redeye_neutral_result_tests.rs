//! #12567: a Red Eye click replaces the pupil with a dark neutral grey, darkest in the middle.
//! Pupil Size widens the dark core and Darken Amount sets its depth; neither changes the search.
//! The oracle is the issue's test document (8-bit values); tolerances are the issue's "Done when".

use super::*;

const SKIN: [u8; 3] = [200, 150, 120];
const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [210, 40, 40];
const ORANGE: [u8; 3] = [230, 120, 60];

/// Eye A row y=200 at Pupil 50 / Darken 50 in the reference (`x`, R, G, B).
const ROW_50_50: [(usize, [i32; 3]); 9] = [
    (182, [70, 64, 64]),
    (186, [55, 49, 49]),
    (190, [37, 34, 35]),
    (194, [25, 24, 24]),
    (198, [20, 20, 20]),
    (202, [22, 21, 21]),
    (206, [30, 29, 29]),
    (210, [45, 42, 42]),
    (214, [64, 54, 54]),
];

/// The window the engine reads around a click at (200,200): search radius plus halo.
const ORIGIN: (i32, i32) = (200 - 70, 200 - 70);
const SIDE: usize = 141;

fn px8(c: [u8; 3]) -> [f32; 4] {
    [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0]
}

fn in_disc(x: usize, y: usize, cx: f32, cy: f32, r: f32) -> bool {
    (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) <= r
}

/// An eye at (200,200) in document coordinates: white r=40, pupil r=18 of `pupil`, and (for Eye
/// A) the 4x4 highlight at x 204–207, y 192–195. Returned as the engine's read window.
fn eye(pupil: [u8; 3], highlight: bool) -> Vec<[f32; 4]> {
    let mut v = Vec::with_capacity(SIDE * SIDE);
    for wy in 0..SIDE {
        for wx in 0..SIDE {
            let (x, y) = (wx + ORIGIN.0 as usize, wy + ORIGIN.1 as usize);
            let c = if highlight && (204..208).contains(&x) && (192..196).contains(&y) {
                WHITE
            } else if in_disc(x, y, 200.0, 200.0, 18.0) {
                pupil
            } else if in_disc(x, y, 200.0, 200.0, 40.0) {
                WHITE
            } else {
                SKIN
            };
            v.push(px8(c));
        }
    }
    v
}

fn click(px: &mut [[f32; 4]], pupil: f32, darken: f32) -> u32 {
    apply(px, SIDE, SIDE, (200, 200), ORIGIN, pupil, darken)
}

/// The document pixel (x, y) as 8-bit RGB.
fn at(px: &[[f32; 4]], x: usize, y: usize) -> [i32; 3] {
    let i = (y - ORIGIN.1 as usize) * SIDE + (x - ORIGIN.0 as usize);
    let p = px[i];
    [0, 1, 2].map(|c| (p[c].clamp(0.0, 1.0) * 255.0).round() as i32)
}

fn centre(px: &[[f32; 4]]) -> [i32; 3] {
    at(px, 199, 200)
}

#[test]
fn eye_a_at_50_50_turns_dark_and_neutral_like_the_reference_row() {
    let before = eye(RED, true);
    let mut px = before.clone();
    let n = click(&mut px, 50.0, 50.0);
    assert!(n > 900, "the whole pupil changes ({n} px)");
    for &(x, want) in &ROW_50_50 {
        let got = at(&px, x, 200);
        for c in 0..3 {
            assert!((got[c] - want[c]).abs() <= 15, "x {x}: {got:?} vs reference {want:?}");
        }
    }
    assert!(centre(&px).iter().all(|&c| c <= 30), "centre {:?}", centre(&px));
    for (b, a) in before.iter().zip(&px) {
        if b == a {
            continue;
        }
        let q = [0, 1, 2].map(|c| (a[c] * 255.0).round() as i32);
        assert!((q[0] - q[1]).abs() <= 10 && (q[0] - q[2]).abs() <= 10, "changed pixel {q:?} is not neutral");
        assert_ne!(*b, px8(WHITE), "a white sclera or highlight pixel changed");
        assert_ne!(*b, px8(SKIN), "a skin pixel changed");
    }
}

#[test]
fn the_orange_pupil_turns_neutral_grey() {
    let mut px = eye(ORANGE, false);
    assert!(click(&mut px, 50.0, 50.0) > 900);
    let got = at(&px, 200, 200);
    for (c, want) in [70, 69, 69].into_iter().enumerate() {
        assert!((got[c] - want).abs() <= 15, "centre {got:?} vs reference 70,69,69");
    }
    for y in 182..218 {
        for x in 182..218 {
            let q = at(&px, x, y);
            assert!((q[0] - q[1]).abs() <= 10 && (q[0] - q[2]).abs() <= 10, "({x},{y}) {q:?} is not neutral");
        }
    }
}

#[test]
fn pupil_size_widens_the_core_and_darken_sets_its_depth() {
    let before = eye(RED, true);
    let run = |pupil: f32, darken: f32| {
        let mut px = before.clone();
        click(&mut px, pupil, darken);
        // The whole pupil changes, not only a patch around the click.
        for y in 182..218 {
            for x in 182..218 {
                if in_disc(x, y, 200.0, 200.0, 18.0) && !((204..208).contains(&x) && (192..196).contains(&y)) {
                    assert_ne!(at(&px, x, y), [210, 40, 40], "{pupil}/{darken}: ({x},{y}) kept its red");
                }
            }
        }
        centre(&px)[0]
    };
    let (p1, p50, p100) = (run(1.0, 50.0), run(50.0, 50.0), run(100.0, 50.0));
    assert!(p100 < p50 && p50 < p1, "centre by Pupil Size: 1 → {p1}, 50 → {p50}, 100 → {p100}");
    let (d1, d100) = (run(50.0, 1.0), run(50.0, 100.0));
    assert!((d1 - 68).abs() <= 15, "Darken 1 centre {d1}, reference 68");
    assert!(d100 <= 15, "Darken 100 centre {d100}");
}

#[test]
fn a_second_click_on_the_corrected_eye_changes_at_most_5() {
    let mut px = eye(RED, true);
    click(&mut px, 50.0, 50.0);
    let once = px.clone();
    click(&mut px, 50.0, 50.0);
    for (a, b) in once.iter().zip(&px) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() * 255.0 <= 5.0, "second click moved {a:?} to {b:?}");
        }
    }
}

#[test]
fn pupil_size_no_longer_limits_the_search() {
    // Pupil Size 1 used to search only 8 px around the click and leave the rest of the pupil red.
    let mut px = eye(RED, true);
    let n1 = click(&mut px, 1.0, 50.0);
    let mut px = eye(RED, true);
    let n50 = click(&mut px, 50.0, 50.0);
    assert_eq!(n1, n50, "Pupil Size 1 changes the same pixels as 50");
}

#[test]
fn a_second_click_on_the_highlight_leaves_the_skin_alone() {
    // A first click on the highlight still finds and corrects the red pupil.
    let mut px = eye(RED, true);
    assert!(apply(&mut px, SIDE, SIDE, (206, 194), ORIGIN, 50.0, 50.0) > 900);
    assert!(centre(&px).iter().all(|&c| c <= 30), "centre {:?}", centre(&px));
    // Clicking the same point again must not flood the skin around the eye (#16148 review).
    let once = px.clone();
    apply(&mut px, SIDE, SIDE, (206, 194), ORIGIN, 50.0, 50.0);
    for (a, b) in once.iter().zip(&px) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() * 255.0 <= 5.0, "second click on the highlight moved {a:?} to {b:?}");
        }
    }
}
