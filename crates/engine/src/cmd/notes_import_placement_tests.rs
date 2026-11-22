//! #13914: File › Import › Notes… multiplies each note's icon and popup top-left by the target
//! canvas height over the source height, keeps their sizes, clamps nothing, and reports when an
//! icon's top-left lands outside the canvas.

use super::*;

/// The issue's three notes on a 600 × 400 source page: icon top-left (92, 90), (292, 190) and
/// (582, 370); popups 240 × 140 next to each.
fn source() -> Vec<Note> {
    [[92.0, 90.0], [292.0, 190.0], [582.0, 370.0]]
        .into_iter()
        .map(|[x, y]| Note { position: [x, y], popup: [x + 20.0, y, x + 260.0, y + 140.0], ..Default::default() })
        .collect()
}

fn placed(target: [u32; 2]) -> (Vec<Note>, bool) {
    place_notes(source(), target, 400).unwrap()
}

#[test]
fn the_measured_targets_give_the_measured_positions() {
    // (target, s, note 1, note 3) as measured in the issue.
    let cases: [([u32; 2], [f64; 2], [f64; 2]); 6] = [
        ([600, 400], [92.0, 90.0], [582.0, 370.0]),
        ([1200, 800], [184.0, 180.0], [1164.0, 740.0]),
        ([300, 200], [46.0, 45.0], [291.0, 185.0]),
        ([1200, 200], [46.0, 45.0], [291.0, 185.0]),
        ([2400, 400], [92.0, 90.0], [582.0, 370.0]),
        ([1200, 1600], [368.0, 360.0], [2328.0, 1480.0]),
    ];
    for (target, n1, n3) in cases {
        let (notes, _) = placed(target);
        assert_eq!(notes[0].position, n1, "{target:?}");
        assert_eq!(notes[2].position, n3, "{target:?}");
    }
}

#[test]
fn icon_and_popup_sizes_stay_and_the_popup_moves_with_the_scale() {
    let (notes, _) = placed([1200, 800]);
    let p = notes[0].popup;
    assert_eq!([p[0], p[1]], [224.0, 180.0], "the popup's top-left is scaled too");
    assert_eq!([p[2] - p[0], p[3] - p[1]], [240.0, 140.0], "its size stays");
}

#[test]
fn nothing_is_clamped_and_off_canvas_notes_are_reported() {
    let (notes, outside) = placed([1200, 1600]);
    assert_eq!(notes[2].position[0], 2328.0, "past the right edge, kept");
    assert!(outside, "note 3's top-left is outside the 1200 px wide canvas");
    // Note 3's icon sticks out over the edge at 300 × 200, but its top-left is inside: no report.
    let (_, outside) = placed([300, 200]);
    assert!(!outside);
}

#[test]
fn fractions_are_not_rounded() {
    let (notes, _) = place_notes(source(), [700, 700], 400).unwrap();
    assert_eq!(notes[0].position, [92.0 * 1.75, 90.0 * 1.75]);
    let (notes, _) = place_notes(source(), [100, 300], 400).unwrap();
    assert_eq!(notes[0].position, [69.0, 67.5]);
}

#[test]
fn a_zero_height_source_or_an_overflow_is_an_error() {
    assert!(place_notes(source(), [600, 400], 0).is_err());
    let huge = vec![Note { position: [f64::MAX, 1.0], popup: [0.0, 0.0, 1.0, 1.0], ..Default::default() }];
    assert!(place_notes(huge, [600, u32::MAX], 1).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_import_reports_outside_and_keeps_the_history_step() {
    let mut src = Document::new("src", photocraft_doc::Size::new(600, 400), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    src.notes = source();
    let bytes = photocraft_format::save_to_bytes(&src, &Default::default()).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1200, "height": 1600})).unwrap();
    let before = s.active().unwrap().history.past_len();
    let r = import_notes_from(&mut s, "notes.pcraft", &bytes).unwrap();
    assert_eq!(r["imported"], json!(3));
    assert_eq!(r["outside"], json!(true));
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    assert_eq!(s.active().unwrap().doc.notes[2].position, [2328.0, 1480.0]);
    s.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    let r = import_notes_from(&mut s, "notes.pcraft", &bytes).unwrap();
    assert_eq!(r["outside"], json!(false));
}
