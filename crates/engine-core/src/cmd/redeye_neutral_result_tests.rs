//! #12567: a Red Eye click turns the pupil dark and neutral, darkest in the middle, on the issue's
//! test document, at 8, 16 and 32 bit and in Lab (read back in RGB).

use serde_json::json;

use super::*;

/// Eye A row y=200 at Pupil 50 / Darken 50 in the reference (`x`, R, G, B).
const ROW_50_50: [(i32, [i32; 3]); 9] = [
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

fn fill(s: &mut Session, rect: [i32; 4], ellipse: bool, color: &str) {
    s.execute("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3], "ellipse": ellipse})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
}

/// 800x600, skin 200,150,120; Eye A (white r=40, red pupil r=18, 4x4 highlight) at (200,200) and
/// the orange eye (pupil 230,120,60) at (680,450).
fn test_doc(mode: &str, depth: u64) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600, "mode": mode, "depth": depth})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#c89678"})).unwrap();
    fill(&mut s, [160, 160, 80, 80], true, "#ffffff");
    fill(&mut s, [182, 182, 36, 36], true, "#d22828");
    fill(&mut s, [204, 192, 4, 4], false, "#ffffff");
    fill(&mut s, [640, 410, 80, 80], true, "#ffffff");
    fill(&mut s, [662, 432, 36, 36], true, "#e6783c");
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// The composite pixel read back as 8-bit RGB.
fn rgb8(s: &Session, x: i32, y: i32) -> [i32; 3] {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    let p = surf.rgba(x, y);
    [0, 1, 2].map(|c| (p[c].clamp(0.0, 1.0) * 255.0).round() as i32)
}

/// Every (mode, depth) the reference gives the same values in.
const CASES: [(&str, u64); 5] = [("rgb", 8), ("rgb", 16), ("rgb", 32), ("lab", 8), ("lab", 16)];

fn eye_a(s: &Session) -> Vec<[i32; 3]> {
    (160..240).flat_map(|y| (160..240).map(move |x| (x, y))).map(|(x, y)| rgb8(s, x, y)).collect()
}

fn neutral(p: [i32; 3]) -> bool {
    (p[0] - p[1]).abs() <= 10 && (p[0] - p[2]).abs() <= 10
}

#[test]
fn eye_a_turns_dark_and_neutral_at_every_depth_and_in_lab() {
    for (mode, depth) in CASES {
        let case = format!("{mode} {depth} bit");
        let mut s = test_doc(mode, depth);
        let before = eye_a(&s);
        let r = s.execute("paint.redEye", json!({"x": 200, "y": 200, "pupilSize": 50, "darken": 50})).unwrap();
        assert!(r["pixels"].as_u64().unwrap() > 900, "{case}: {r}");
        for &(x, want) in &ROW_50_50 {
            let got = rgb8(&s, x, 200);
            assert!((0..3).all(|c| (got[c] - want[c]).abs() <= 15), "{case}: x {x} {got:?} vs reference {want:?}");
        }
        assert!(rgb8(&s, 199, 200).iter().all(|&c| c <= 30), "{case}: centre {:?}", rgb8(&s, 199, 200));
        let after = eye_a(&s);
        for (b, a) in before.iter().zip(&after) {
            if (0..3).all(|c| (a[c] - b[c]).abs() <= 1) {
                continue;
            }
            assert!(neutral(*a), "{case}: changed pixel {b:?} → {a:?} is not neutral");
            assert!(b.iter().any(|&c| c < 240), "{case}: white sclera or highlight {b:?} changed to {a:?}");
        }
        for y in 192..196 {
            for x in 204..208 {
                assert!(rgb8(&s, x, y).iter().all(|&c| c >= 245), "{case}: highlight ({x},{y}) {:?}", rgb8(&s, x, y));
            }
        }
        let orange = s.execute("paint.redEye", json!({"x": 680, "y": 450})).unwrap();
        assert!(orange["pixels"].as_u64().unwrap() > 900, "{case}: {orange}");
        let c = rgb8(&s, 680, 450);
        assert!(neutral(c) && (0..3).all(|i| (c[i] - [70, 69, 69][i]).abs() <= 15), "{case}: orange centre {c:?}");
    }
}

#[test]
fn a_second_click_changes_no_channel_by_more_than_5() {
    for (mode, depth) in CASES {
        let mut s = test_doc(mode, depth);
        s.execute("paint.redEye", json!({"x": 200, "y": 200})).unwrap();
        let once = eye_a(&s);
        s.execute("paint.redEye", json!({"x": 200, "y": 200})).unwrap();
        for (a, b) in once.iter().zip(&eye_a(&s)) {
            assert!((0..3).all(|c| (a[c] - b[c]).abs() <= 5), "{mode} {depth} bit: {a:?} → {b:?}");
        }
    }
}

#[test]
fn pupil_size_and_darken_change_the_whole_pupil_and_keep_the_reference_order() {
    let centre = |pupil: u32, darken: u32| {
        let mut s = test_doc("rgb", 8);
        s.execute("paint.redEye", json!({"x": 200, "y": 200, "pupilSize": pupil, "darken": darken})).unwrap();
        for y in 183..217 {
            for x in 183..217 {
                let inside = (x as f32 + 0.5 - 200.0).hypot(y as f32 + 0.5 - 200.0) <= 17.0;
                if inside && !((204..208).contains(&x) && (192..196).contains(&y)) {
                    assert!(neutral(rgb8(&s, x, y)), "{pupil}/{darken}: ({x},{y}) {:?}", rgb8(&s, x, y));
                }
            }
        }
        rgb8(&s, 199, 200)[0]
    };
    let (p1, p50, p100) = (centre(1, 50), centre(50, 50), centre(100, 50));
    assert!(p100 < p50 && p50 < p1, "Pupil 1 → {p1}, 50 → {p50}, 100 → {p100}");
    let (d1, d100) = (centre(50, 1), centre(50, 100));
    assert!((d1 - 68).abs() <= 15, "Darken 1 centre {d1}");
    assert!(d100 <= 15, "Darken 100 centre {d100}");
}

#[test]
fn a_click_on_skin_filling_a_small_canvas_leaves_it_alone() {
    // The read window is clipped to the 64×64 canvas, so the skin never reaches the search circle;
    // the click is still a miss (#16148 review).
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#c89678"})).unwrap();
    let r = s.execute("paint.redEye", json!({"x": 32, "y": 32}));
    assert!(r.as_ref().map_or(true, |v| v["pixels"].as_u64().unwrap_or(0) == 0), "{r:?}");
    for (x, y) in [(0, 0), (32, 32), (63, 63), (10, 50)] {
        assert_eq!(rgb8(&s, x, y), [200, 150, 120], "({x},{y})");
    }
}
