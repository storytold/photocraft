//! #12103: the Healing Brush's Use Legacy and Diffusion. `paint.healingBrush` takes `legacy`
//! (bool, default false) and `diffusion` (whole number 1..7, default 5). Without those keys, and
//! with any value from 1 to 7, the stroke heals exactly as it did before the option existed: the
//! pinned levels below are the values main b88866441b gives on this document. `diffusion: 0`, `8`,
//! `-1`, `5.5` (the field holds whole numbers) and a non-number are errors that paint nothing and
//! add no History step.
//!
//! The reference application's pixels for the Diffusion values other than 5 are not measured, so
//! the field is accepted, validated and stored while every value keeps today's stroke; measuring
//! that row is the standing question on the ticket. Use Legacy on is the older blend, the source
//! texture taken to the healed area's average colour in L*a*b*, and it lands the ticket's measured
//! row on the ticket's own document within the +/-6 the "Done when" list asks for.

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

/// The healed levels main b88866441b gives at `SPOTS` (R, G, B, A in 8-bit levels).
const PINNED: [(i32, i32, i32, i32); 5] = [(0, 88, 8, 255), (116, 226, 134, 255), (136, 219, 150, 255), (11, 77, 22, 255), (144, 221, 157, 255)];

/// The Background's pixel at (`x`, `y`) in 8-bit levels.
fn levels(s: &Session, x: i32, y: i32) -> Vec<i32> {
    let st = s.active().unwrap();
    let q = |v: f32| (f64::from(v) * 255.0).round() as i32;
    st.doc.layers[0].surface().unwrap().pixel(x, y).iter().map(|&v| q(v)).collect()
}

fn read(s: &Session) -> Vec<Vec<i32>> {
    SPOTS.iter().map(|(x, y)| levels(s, *x, *y)).collect()
}

/// The pinned stroke as a vector, for the comparisons.
fn pinned() -> Vec<Vec<i32>> {
    PINNED.iter().map(|&(r, g, b, a)| vec![r, g, b, a]).collect()
}

#[test]
fn diffusion_five_heals_exactly_as_before_the_option() {
    // Pin the healed result byte for byte: the values main b88866441b gives on this document. If
    // one of these moves, a case the ticket leaves alone moved with it.
    let mut s = document();
    s.execute("paint.healingBrush", stroke(None)).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Healing Brush"), "the stroke's History name");
    assert_eq!(read(&s), pinned(), "the default result is pinned");
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
    assert_eq!(m, pinned(), "Use Legacy off is the pinned stroke");
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
fn use_legacy_on_the_measured_document_lands_its_measured_row() {
    // The ticket's own document: 800 x 600 RGB/8, an 80 x 80 checker at 100,100 with 10 px cells
    // alternating 60,60,60 / 200,200,200, plain green 40,160,60 from x 400 to 800; a 40 px hard
    // round tip, the source point 140,140 and one single click at 600,450. Its measured row y 450
    // with Use Legacy on: dark cells 0,78..89,0, light cells 127..131,229..239,136..141, and a
    // darker rim at x 620 of 32,152,52 - the issue's table, within +/-6 per channel as its
    // "Done when" row says.
    let on = measured(true);
    let within = |at: (i32, i32), want: [(i32, i32); 3]| {
        let got = levels(&on, at.0, at.1);
        for (c, (lo, hi)) in want.into_iter().enumerate() {
            assert!(got[c] >= lo - 6 && got[c] <= hi + 6, "({} , {}) channel {c}: {got:?} outside {lo}..{hi} plus six: {:?}", at.0, at.1, got);
        }
    };
    // One pixel in the first dark cell, two light cells, and the rim the ticket names.
    within((585, 450), [(0, 0), (78, 89), (0, 0)]);
    within((595, 450), [(127, 131), (229, 239), (136, 141)]);
    within((614, 450), [(127, 131), (229, 239), (136, 141)]);
    within((620, 450), [(32, 32), (152, 152), (52, 52)]);
    // The same click with Use Legacy off stays the ticket's other row: dark 0,79..94,0 and light
    // 112..114,232..234,132..134.
    let off = measured(false);
    for at in [(585, 450), (605, 450)] {
        let got = levels(&off, at.0, at.1);
        assert_eq!((got[0], got[2]), (0, 0), "Modern at ({}, {}): {got:?}", at.0, at.1);
        assert!(got[1] >= 79 && got[1] <= 94, "Modern dark green at ({}, {}): {got:?}", at.0, at.1);
    }
    for at in [(595, 450), (614, 450)] {
        let got = levels(&off, at.0, at.1);
        for (c, (lo, hi)) in [(112, 114), (232, 234), (132, 134)].into_iter().enumerate() {
            assert!(got[c] >= lo - 6 && got[c] <= hi + 6, "Modern at ({}, {}) channel {c}: {got:?}", at.0, at.1);
        }
    }
    assert_ne!(levels(&off, 614, 450), levels(&on, 614, 450), "the two blends differ on the ticket's own click");
}

/// The ticket's measured document with one Healing Brush click: an 800 x 600 RGB/8 image, white,
/// green 40,160,60 from x 400 to 800, an 80 x 80 checker at 100,100 with 10 px cells alternating
/// 60,60,60 / 200,200,200; the source point 140,140 and a single click at 600,450 with a 40 px
/// hard round tip. `legacy` turns Use Legacy on.
fn measured(legacy: bool) -> Session {
    let mut s = crate::session();
    s.add_document(Document::with_background("m", Size::new(800, 600), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    s.execute("select.rect", json!({"x": 400, "y": 0, "width": 400, "height": 600})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#28a03c"})).unwrap();
    for cx in 0..8 {
        for cy in 0..8 {
            let color = if (cx + cy) % 2 == 0 { "#3c3c3c" } else { "#c8c8c8" };
            s.execute("select.rect", json!({"x": 100 + 10 * cx, "y": 100 + 10 * cy, "width": 10, "height": 10})).unwrap();
            s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        }
    }
    s.execute("select.deselect", json!({})).unwrap();
    let mut p = json!({"points": [[600, 450]], "source": [140, 140], "size": 40, "hardness": 100});
    if legacy {
        p["legacy"] = json!(true);
    }
    s.execute("paint.healingBrush", p).unwrap();
    s
}

#[test]
fn every_accepted_diffusion_heals_exactly_as_today() {
    // The reference's pixels for the values other than 5 are not measured, and the ticket's rule
    // for a case that was not measured is that today's behaviour stays. So the field is accepted,
    // validated and remembered while every whole value 1..7 gives the pinned stroke: the option
    // costs nothing until those pixels are measured.
    for k in 1..=7u8 {
        let mut s = document();
        s.execute("paint.healingBrush", stroke(Some(("diffusion", json!(k))))).unwrap();
        assert_eq!(read(&s), pinned(), "diffusion {k} heals exactly as before the field existed");
    }
    // And the whole-number forms the bar and an action step send are the same values.
    for k in [json!(1.0), json!(7.0), json!("not a number")] {
        let mut s = document();
        let painted = s.execute("paint.healingBrush", stroke(Some(("diffusion", k.clone()))));
        if k.is_number() {
            assert!(painted.is_ok(), "{k} is a whole number in range");
            assert_eq!(read(&s), pinned(), "{k} heals as today");
        } else {
            assert!(painted.is_err(), "{k} is refused");
        }
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
    // The field holds whole numbers, so a fraction is refused rather than rounded to its nearest.
    for part in [json!(5.5), json!(1.25), json!(6.999)] {
        let mut s = document();
        let before = (read(&s), s.active().unwrap().history.past_len());
        let err = s.execute("paint.healingBrush", stroke(Some(("diffusion", part.clone())))).unwrap_err().to_string();
        assert!(err.contains("`diffusion`"), "{part}: a fraction is an error, not a rounded value: {err}");
        assert_eq!((read(&s), s.active().unwrap().history.past_len()), (before.0.clone(), before.1), "{part}: nothing painted, no step");
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
    // Replace (#12101) still copies the source straight, and stays that stroke at any Diffusion
    // value the command accepts.
    let mut r = document();
    r.execute("paint.healingBrush", stroke(Some(("mode", json!("replace"))))).unwrap();
    let copied = read(&r);
    assert_eq!(levels(&r, 80, 60), vec![60, 60, 60, 255], "Replace lays the source pixel down as it stands");
    for k in [1u8, 4, 7] {
        let mut s = document();
        let mut p = stroke(Some(("mode", json!("replace"))));
        p["diffusion"] = json!(k);
        s.execute("paint.healingBrush", p).unwrap();
        assert_eq!(read(&s), copied, "Replace is the same stroke at diffusion {k}");
    }
    // And Replace is still not the healed stroke.
    assert_ne!(copied, pinned(), "Replace is not Normal");
}
