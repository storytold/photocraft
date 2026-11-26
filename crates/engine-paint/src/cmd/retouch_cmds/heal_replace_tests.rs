//! #12101: the Healing Brush's Replace mode. `paint.healingBrush {"mode":"replace"}` copies the
//! source pixels straight onto the layer: no healing solve, Normal compositing. Every covered pixel
//! equals the source pixel the stroke maps there, bit for bit, as read from the document's state
//! before the stroke. The other modes keep their results; an unknown `mode` string stays an error,
//! and the stroke's History step keeps the name "Healing Brush".

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

use super::*;

/// The test document: green 40,160,60 throughout with a 20 x 20 checker at 6,6 (5 px cells
/// alternating 60,60,60 / 200,200,200). The source patch and the click sit far apart, so a
/// straight copy and a heal cannot agree anywhere inside the dab.
fn document() -> Session {
    let mut s = crate::session();
    s.add_document(Document::with_background("t", Size::new(48, 48), ColorMode::Rgb, SampleType::U8, Color::rgb(40.0 / 255.0, 160.0 / 255.0, 60.0 / 255.0)), None);
    for cx in 0..4 {
        for cy in 0..4 {
            let color = if (cx + cy) % 2 == 0 { "#3c3c3c" } else { "#c8c8c8" };
            s.execute("select.rect", json!({"x": 6 + 5 * cx, "y": 6 + 5 * cy, "width": 5, "height": 5})).unwrap();
            s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        }
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// One click at 30,24 with a 20 px hard round tip; the source point 16,16 is sampled under it.
fn stroke(mode: Option<&str>) -> Value {
    let mut p = json!({"points": [[30, 24]], "source": [16, 16], "size": 20, "hardness": 100});
    if let Some(m) = mode {
        p["mode"] = json!(m);
    }
    p
}

fn pixels(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    (0..48).flat_map(|y| (0..48).flat_map(move |x| surf.pixel(x, y))).collect()
}

/// The pixel of the Background at (`x`, `y`).
fn pixel(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layers[0].surface().unwrap().pixel(x, y)
}

#[test]
fn replace_paints_the_source_pixels_bit_for_bit() {
    let mut s = document();
    let before = pixels(&s);
    let steps = s.active().unwrap().history.past_len();
    s.execute("paint.healingBrush", stroke(Some("replace"))).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Healing Brush"), "the stroke's History name");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "the stroke is one step");
    // Every pixel the hard tip covers fully (7 px of the 10 px radius, away from the antialiased
    // rim) is the source pixel mapped there, bit for bit; the samples are the pre-stroke pixels.
    let green: Vec<f32> = vec![40.0 / 255.0, 160.0 / 255.0, 60.0 / 255.0, 1.0];
    let ch = pixel(&s, 0, 0).len();
    let src = |x: i32, y: i32| {
        let at = usize::try_from(y * 48 + x).unwrap() * ch;
        before[at..at + ch].to_vec()
    };
    let mut inside = 0;
    for dy in -7..=7 {
        for dx in -7..=7 {
            if dx * dx + dy * dy > 49 {
                continue;
            }
            let (x, y) = (30 + dx, 24 + dy);
            assert_eq!(pixel(&s, x, y), src(x - 14, y - 8), "({x}, {y}) inside the Replace dab");
            inside += 1;
        }
    }
    assert!(inside > 0, "the dab covers pixels");
    // Outside the tip nothing changed: the dab's edge is 10 px away, these are 13.
    assert_eq!(pixel(&s, 43, 24), green, "right of the dab");
    assert_eq!(pixel(&s, 30, 37), green, "below the dab");
}

#[test]
fn an_unknown_mode_is_an_error_and_paints_nothing() {
    let mut s = document();
    let before = (pixels(&s), s.active().unwrap().history.past_len());
    let err = s.execute("paint.healingBrush", stroke(Some("threshold"))).unwrap_err().to_string();
    assert!(err.contains("unknown blend mode `threshold`"), "{err}");
    assert_eq!((pixels(&s), s.active().unwrap().history.past_len()), (before.0.clone(), before.1), "nothing painted, no step");
}

#[test]
fn replace_is_the_healing_brush_command_s_alone() {
    let mut s = document();
    let err = s.execute("paint.cloneStamp", json!({"points": [[30, 24]], "source": [16, 16], "size": 20, "hardness": 100, "mode": "replace"})).unwrap_err().to_string();
    assert!(err.contains("unknown blend mode `replace`"), "the Clone Stamp still refuses it: {err}");
    // The live stroke parser reads the same mode: a Healing live stroke starts with Replace.
    crate::cmd::retouch_cmds::clone::LiveRetouch::begin(&s, "paint.healingBrush", &stroke(Some("replace"))).unwrap();
    // And the blend parameter itself: Normal compositing for the Healing Brush, nothing else.
    assert_eq!(blend_param(&json!({"mode": "replace"}), "paint.healingBrush").unwrap(), BlendMode::Normal);
    assert!(blend_param(&json!({"mode": "replace"}), "paint.cloneStamp").is_err());
    assert!(blend_param(&json!({"mode": "Replace"}), "paint.healingBrush").is_err(), "the exact lowercase string only");
}

#[test]
fn normal_heals_the_same_pixels_as_before() {
    // Pin the healed (not copied) result byte for byte: the values from main 984c304b20. If one of
    // these moves, a mode the ticket leaves alone moved with it.
    let mut s = document();
    s.execute("paint.healingBrush", stroke(Some("normal"))).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Healing Brush"));
    let q = |v: f32| (v * 255.0).round() as i32;
    let pts = [(30, 24), (34, 26), (26, 22), (24, 24)];
    let got: Vec<Vec<i32>> = pts.iter().map(|(x, y)| pixel(&s, *x, *y).iter().map(|&v| q(v)).collect()).collect();
    // The values main 984c304b20 gives on this document (R, G, B, A in 8-bit levels).
    let pinned: Vec<Vec<i32>> = vec![vec![62, 56, 61, 255], vec![57, 63, 58, 255], vec![77, 49, 73, 255], vec![82, 53, 77, 255]];
    assert_eq!(got, pinned, "the Normal result is pinned");
    // Replace on the same document gives different pixels than Normal: the straight copy and the
    // blend are visibly two different strokes.
    let mut r = document();
    r.execute("paint.healingBrush", stroke(Some("replace"))).unwrap();
    let replaced: Vec<Vec<i32>> = pts.iter().map(|(x, y)| pixel(&r, *x, *y).iter().map(|&v| q(v)).collect()).collect();
    assert_ne!(got, replaced, "Replace is not Normal");
}
