//! #12103: the Healing Brush's Use Legacy and Diffusion. `paint.healingBrush` takes `legacy`
//! (bool, default false) and `diffusion` (whole number 1..7, default 5). Without those keys, and
//! with `diffusion: 5`, the stroke heals exactly as it did before the option existed: the pinned
//! levels below are the values main gives on this document. `diffusion: 0`, `8`, `-1` and a
//! non-number are errors that paint nothing and add no History step.
//!
//! What the reference's other Diffusion values do to the pixels is not measured, so the claims
//! here are ours and relative: each value from 1 to 7 gives different pixels, and the texture's
//! total variation falls as the value rises (Diffusion 5 sits at today's texture). Use Legacy on
//! gives a different blend than Use Legacy off on the same click; the reference's measured legacy
//! levels are not reproduced here.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

use super::*;

/// The test document: green 40,160,60 throughout with a 40 x 40 checker at 10,10 (5 px cells
/// alternating 60,60,60 / 200,200,200), away from where the stroke lands.
fn document() -> Session {
    let mut s = crate::session();
    s.add_document(
        Document::with_background("t", Size::new(120, 96), ColorMode::Rgb, SampleType::U8, Color::rgb(40.0 / 255.0, 160.0 / 255.0, 60.0 / 255.0)),
        None,
    );
    for cx in 0..8 {
        for cy in 0..8 {
            let color = if (cx + cy) % 2 == 0 { "#3c3c3c" } else { "#c8c8c8" };
            s.execute("select.rect", json!({"x": 10 + 5 * cx, "y": 10 + 5 * cy, "width": 5, "height": 5})).unwrap();
            s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        }
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// One click at 80,60 with a 24 px hard round tip; its source point 24,24 sits in the checker.
fn stroke(extra: Option<(&str, Value)>) -> Value {
    let mut p = json!({"points": [[80, 60]], "source": [24, 24], "size": 24, "hardness": 100});
    if let Some((key, value)) = extra {
        p[key] = value;
    }
    p
}

/// The points inside the dab the levels are read at, and every level the two blends are compared by.
const SPOTS: [(i32, i32); 5] = [(80, 60), (86, 64), (74, 56), (70, 60), (80, 52)];

/// The Background's pixel at (`x`, `y`) in 8-bit levels.
fn levels(s: &Session, x: i32, y: i32) -> Vec<i32> {
    let st = s.active().unwrap();
    let q = |v: f32| (f64::from(v) * 255.0).round() as i32;
    st.doc.layers[0].surface().unwrap().pixel(x, y).iter().map(|&v| q(v)).collect()
}

fn read(s: &Session) -> Vec<Vec<i32>> {
    SPOTS.iter().map(|(x, y)| levels(s, *x, *y)).collect()
}

/// The sum of the level steps between neighbouring pixels over the dab's box, all three colour
/// channels: a texture's detail in one number. A plain field scores 0, a checker scores high.
fn total_variation(s: &Session) -> i64 {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    let q = |v: f32| (f64::from(v) * 255.0).round() as i64;
    let at = |x: i32, y: i32| surf.pixel(x, y);
    let (x0, y0, x1, y1) = (66, 46, 94, 74);
    let mut sum = 0i64;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let here = at(x, y);
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx > x1 || ny > y1 {
                    continue;
                }
                let there = at(nx, ny);
                for c in 0..3 {
                    sum += (q(here[c]) - q(there[c])).abs();
                }
            }
        }
    }
    sum
}

#[test]
fn diffusion_five_heals_exactly_as_before_the_option() {
    // Pin the healed result byte for byte: the values main b88866441b gives on this document. If
    // one of these moves, a case the ticket leaves alone moved with it.
    let mut s = document();
    s.execute("paint.healingBrush", stroke(None)).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Healing Brush"), "the stroke's History name");
    let pinned: Vec<Vec<i32>> = vec![vec![0, 88, 8, 255], vec![116, 226, 134, 255], vec![136, 219, 150, 255], vec![11, 77, 22, 255], vec![144, 221, 157, 255]];
    assert_eq!(read(&s), pinned, "the default result is pinned");
    // The key with its default value is the same stroke: today's parameters and the bar's defaults
    // agree.
    let mut t = document();
    t.execute("paint.healingBrush", stroke(Some(("diffusion", json!(5))))).unwrap();
    assert_eq!(read(&t), read(&s), "diffusion 5 is the absent key");
    // And nothing outside the tip changed: the dab is 12 px wide, these are 20 px away.
    assert_eq!(levels(&s, 100, 60), vec![40, 160, 60, 255], "right of the dab");
    assert_eq!(levels(&s, 80, 80), vec![40, 160, 60, 255], "below the dab");
}

#[test]
fn use_legacy_blends_otherwise_on_the_same_click() {
    let mut modern = document();
    modern.execute("paint.healingBrush", stroke(None)).unwrap();
    let mut legacy = document();
    legacy.execute("paint.healingBrush", stroke(Some(("legacy", json!(true))))).unwrap();
    let (m, l) = (read(&modern), read(&legacy));
    assert_ne!(m, l, "Use Legacy is a different blend, not a flag that changes nothing");
    // Both stay inside the tip: 24 px away on all sides the ground is untouched.
    assert_eq!(levels(&legacy, 100, 60), vec![40, 160, 60, 255], "Use Legacy paints no further than the tip");
    assert_eq!(levels(&legacy, 80, 80), vec![40, 160, 60, 255], "Use Legacy paints no further than the tip");
    // And a `false` legacy is today's stroke.
    let mut off = document();
    off.execute("paint.healingBrush", stroke(Some(("legacy", json!(false))))).unwrap();
    assert_eq!(read(&off), m, "legacy false is the absent key");
}

#[test]
fn diffusion_moves_the_texture_and_smooths_it_as_the_value_rises() {
    let mut by_value = Vec::new();
    for k in 1..=7u8 {
        let mut s = document();
        s.execute("paint.healingBrush", stroke(Some(("diffusion", json!(k))))).unwrap();
        by_value.push((k, read(&s), total_variation(&s)));
    }
    // Diffusion 5 is today's pixels: the option's default changes nothing.
    let mut plain = document();
    plain.execute("paint.healingBrush", stroke(None)).unwrap();
    let today = read(&plain);
    assert_eq!(by_value[4].1, today, "diffusion 5 heals as before the option");
    // Every value from 1 to 7 gives different pixels than its neighbour.
    for pair in by_value.windows(2) {
        assert_ne!(pair[0].1, pair[1].1, "diffusion {} and {} give different pixels", pair[0].0, pair[1].0);
    }
    // Our own metric, no reference numbers: the texture's detail falls as Diffusion rises.
    for pair in by_value.windows(2) {
        assert!(pair[0].2 > pair[1].2, "diffusion {} is smoother than {}: {} vs {}", pair[1].0, pair[0].0, pair[1].2, pair[0].2);
    }
}

#[test]
fn a_diffusion_outside_one_to_seven_is_an_error_and_paints_nothing() {
    for bad in [json!(0), json!(8), json!(-1), json!("5"), json!(true), json!([5])] {
        let mut s = document();
        let before = (read(&s), s.active().unwrap().history.past_len());
        let err = s.execute("paint.healingBrush", stroke(Some(("diffusion", bad.clone())))).unwrap_err().to_string();
        assert!(err.contains("`diffusion`"), "{bad}: the error names the key: {err}");
        assert_eq!((read(&s), s.active().unwrap().history.past_len()), (before.0.clone(), before.1), "{bad}: nothing painted, no step");
    }
    // An absent key and a null one both mean the default 5.
    for (label, key) in [("an absent `diffusion`", None), ("a null one", Some(("diffusion", Value::Null)))] {
        let mut s = document();
        s.execute("paint.healingBrush", stroke(key)).unwrap();
        let mut five = document();
        five.execute("paint.healingBrush", stroke(Some(("diffusion", json!(5))))).unwrap();
        assert_eq!(read(&s), read(&five), "{label} is the default 5");
    }
}

#[test]
fn the_healing_brush_keeps_its_other_params_working() {
    // The Clone Stamp never shows these controls (#12103) and its command keeps its own params: a
    // legacy flag or a diffusion value on that stroke changes nothing about how it copies.
    let mut plain = document();
    plain.execute("paint.cloneStamp", stroke(None)).unwrap();
    let mut with = document();
    with.execute("paint.cloneStamp", stroke(Some(("legacy", json!(true))))).unwrap();
    assert_eq!(read(&plain), read(&with), "the Clone Stamp copy ignores the healing options");
    // Replace (#12101) still copies the source straight: the new texture stage leaves it alone.
    let mut r = document();
    r.execute("paint.healingBrush", stroke(Some(("mode", json!("replace"))))).unwrap();
    let copied = read(&r);
    assert_ne!(
        copied,
        {
            let mut m = document();
            m.execute("paint.healingBrush", stroke(None)).unwrap();
            read(&m)
        },
        "Replace is still not the healed stroke"
    );
    assert_eq!(levels(&r, 80, 60), vec![60, 60, 60, 255], "Replace lays the source pixel down as it stands");
}
