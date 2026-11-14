//! #14509: Image › Image Rotation › Arbitrary in a Bitmap document.
//!
//! Measured in the reference application: in a Bitmap document only Arbitrary… is greyed in the
//! Image Rotation submenu, and running the turn from a script answers "Could not complete the
//! command because it does not work with Bitmap images (convert image to Grayscale mode to edit)."
//! with the document unchanged. The five other Image Rotation items — 180°, 90° Clockwise, 90°
//! Counter Clockwise and the two Flip Canvas items — stay available there, and every other colour
//! mode keeps what it has today.
//!
//! The Crop tool's straighten turn shares `rotate_arbitrary_with` with this command and is *not
//! measured* in Bitmap mode, so it keeps today's behaviour; the last test pins that the new check
//! does not reach it.

use std::sync::Arc;

use super::mode_cmds::rotated_size;
use super::*;

const ARBITRARY: &str = "image.rotation.arbitrary";

/// The reference's words, measured (#14509), spelled out here rather than read from the command so
/// the test fails if what the app answers with ever drifts from what was measured. The engine wraps
/// a greyed command's reason in `EngineError::Disabled`, so a scripted call reads this sentence
/// after the usual `command "…" is not available right now: ` prefix.
const REFUSAL: &str = "Could not complete the command because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// The five Image Rotation items the reference leaves available in a Bitmap document (#14509), each
/// with where the black 8 x 8 block at the top-left corner of a 32 x 24 canvas lands after it, and
/// the canvas it leaves (the two 90° turns swap the sides).
type TurnCase = (&'static str, (i32, i32), (u32, u32));

const TURNS: [TurnCase; 5] = [
    ("image.imageRotation.180", (29, 21), (32, 24)),
    ("image.imageRotation.90cw", (21, 2), (24, 32)),
    ("image.imageRotation.90ccw", (2, 29), (24, 32)),
    ("image.imageRotation.flipCanvasHorizontal", (29, 2), (32, 24)),
    ("image.imageRotation.flipCanvasVertical", (2, 21), (32, 24)),
];

/// A `w x h` document in `mode` at 8 bits, with a white Background.
fn doc_in(mode: &str, w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "mode": mode, "background": "white"})).unwrap();
    s
}

/// A Bitmap document as Image › Mode › Bitmap leaves one: flattened and 1-bit (stored expanded as
/// Gray 8-bit) and, once [`mark`] has run, a white field with a black block at the top-left.
fn bitmap(w: u32, h: u32) -> Session {
    let mut s = doc_in("grayscale", w, h);
    s.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
    assert_eq!(doc_of(&s).mode, ColorMode::Bitmap, "the conversion made it a Bitmap document");
    mark(&mut s);
    s
}

/// The black 8 x 8 block at the canvas's top-left corner that makes a turn or a flip visible.
fn mark(s: &mut Session) {
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        // The layer's own channel count, so the same block works in Bitmap, Grayscale and RGB.
        let black = vec![0.0; surf.channels()];
        surf.fill_rect(Rect::new(0, 0, 8, 8), &black);
        Ok(())
    })
    .unwrap();
}

fn doc_of(s: &Session) -> Arc<Document> {
    s.active().unwrap().doc.clone()
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn size_of(s: &Session) -> (u32, u32) {
    let d = doc_of(s);
    (d.size.width, d.size.height)
}

/// The Background's level at `(x, y)`: 0.0 black, 1.0 white.
fn level(s: &Session, x: i32, y: i32) -> f32 {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().sample_channel(x, y, 0)
}

/// Everything a refused turn must leave exactly as it was: the document itself, the canvas, the
/// History depth and the two corners of the mark.
type Snapshot = (Arc<Document>, (u32, u32), usize, [f32; 2]);

fn snapshot(s: &Session) -> Snapshot {
    (doc_of(s), size_of(s), steps(s), [level(s, 2, 2), level(s, 30, 23)])
}

#[test]
fn arbitrary_is_greyed_out_in_a_bitmap_document() {
    let s = bitmap(32, 24);
    assert!(!s.is_enabled(ARBITRARY), "Image › Image Rotation › Arbitrary is greyed out in a Bitmap document");
    assert_eq!(s.disabled_reason(ARBITRARY).as_deref(), Some(REFUSAL), "with the reason the reference gives");
    // Every param shape is off, including a null angle that would have changed nothing anyway.
    for p in [json!({}), json!({"angle": 30}), json!({"angle": 90, "direction": "ccw"})] {
        assert!(!s.is_enabled_with(ARBITRARY, &p), "{p}: still greyed");
    }
    // With no document at all it stays off for the reason it already had.
    assert_eq!(Session::new().disabled_reason(ARBITRARY).as_deref(), Some("no document open"), "no document open, as before");
}

#[test]
fn a_scripted_arbitrary_turn_in_a_bitmap_document_answers_the_reference_error_and_changes_nothing() {
    let mut s = bitmap(32, 24);
    let history = steps(&s);
    for p in [json!({"angle": 30}), json!({"angle": 90, "direction": "ccw"}), json!({"angle": 45, "interpolation": "nearest"}), json!({})] {
        let (doc, size, step_count, marks) = snapshot(&s);
        let e = s.execute(ARBITRARY, p.clone()).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(id, why) if id == ARBITRARY && why == REFUSAL), "{p}: refused as a greyed command, got {e}");
        assert_eq!(
            e.to_string(),
            format!("command `{ARBITRARY}` is not available right now: {REFUSAL}"),
            "{p}: the measured sentence is what the caller reads"
        );
        // Nothing at all happened: not a History step, not a pixel, not a corner of the canvas.
        let (after, size_after, steps_after, marks_after) = snapshot(&s);
        assert!(Arc::ptr_eq(&doc, &after), "{p}: the document is untouched");
        assert_eq!(size_after, size, "{p}: the canvas keeps its size");
        assert_eq!(size_after, (32, 24), "{p}: still the 32 x 24 canvas it was");
        assert_eq!(steps_after, step_count, "{p}: no History step");
        assert_eq!(marks_after, marks, "{p}: the mark stays put");
        assert_eq!([level(&s, 2, 2), level(&s, 30, 23)], [0.0, 1.0], "{p}: black at the top-left, white at the bottom-right");
    }
    // The refusal took nothing away either: the document is still Bitmap and its History still holds
    // the steps made before the call.
    assert_eq!(doc_of(&s).mode, ColorMode::Bitmap, "still a Bitmap document");
    assert_eq!(steps(&s), history, "the whole run added no History step");
    assert!(s.undo(), "the steps made before the refusal are still there to undo");
}

#[test]
fn the_other_image_rotation_items_stay_available_in_a_bitmap_document() {
    for (id, (bx, by), size) in TURNS {
        let mut s = bitmap(32, 24);
        assert!(s.is_enabled(id), "{id} stays available in a Bitmap document");
        let history = steps(&s);
        s.execute(id, json!({})).unwrap_or_else(|e| panic!("{id} turns in a Bitmap document, got {e}"));
        assert_eq!(size_of(&s), size, "{id}: the canvas it leaves");
        assert_eq!([level(&s, bx, by), level(&s, 2, 2)], [0.0, 1.0], "{id}: the mark turned to ({bx}, {by})");
        assert_eq!(steps(&s), history + 1, "{id}: one History step");
        assert!(s.undo(), "{id}: and it undoes");
        assert_eq!([level(&s, 2, 2), level(&s, 30, 23)], [0.0, 1.0], "{id}: back to the mark at the top-left");
    }
}

#[test]
fn arbitrary_stays_available_and_turns_in_rgb_and_grayscale() {
    for (mode, want) in [("grayscale", ColorMode::Grayscale), ("rgb", ColorMode::Rgb)] {
        let mut s = doc_in(mode, 32, 24);
        mark(&mut s);
        assert!(s.is_enabled(ARBITRARY), "{mode}: still available");
        let r = s.execute(ARBITRARY, json!({"angle": 30})).unwrap_or_else(|e| panic!("{mode}: turns as today, got {e}"));
        let d = doc_of(&s);
        assert_eq!(d.size, rotated_size(Size::new(32, 24), 30.0), "{mode}: the canvas grows to fit, as today");
        assert_eq!(
            (r["width"].as_u64(), r["height"].as_u64()),
            (Some(u64::from(d.size.width)), Some(u64::from(d.size.height))),
            "{mode}: the result reports the new canvas"
        );
        assert!(d.size.width > 32 && d.size.height > 24, "{mode}: the turned canvas");
        assert_eq!(d.mode, want, "{mode}: and the mode is untouched");
    }
}

#[test]
fn only_a_bitmap_document_greys_the_arbitrary_turn_out() {
    // The other modes, the other expanded-pixel ones included, answer as they do today.
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel, ColorMode::Duotone] {
        let d = Document::with_background("modes", Size::new(32, 24), mode, SampleType::U8, Color::WHITE);
        let mut s = Session::new();
        s.add_document(d, None);
        assert!(s.is_enabled(ARBITRARY), "{mode:?}: not measured, so it stays available");
    }
    let mut s = bitmap(32, 24);
    assert!(!s.is_enabled(ARBITRARY), "Bitmap: the one measured refusal");
    // Grayscale, the mode the reason points at, answers with the turn.
    s.execute("image.mode.grayscale", json!({})).unwrap_or_else(|e| panic!("Image › Mode › Grayscale, got {e}"));
    assert!(s.is_enabled(ARBITRARY), "Grayscale again: available again");
    s.execute(ARBITRARY, json!({"angle": 30})).unwrap();
    assert_eq!(doc_of(&s).size, rotated_size(Size::new(32, 24), 30.0), "and it turns");
}

#[test]
fn the_crop_straighten_turn_still_turns_a_bitmap_document() {
    // Not measured in the reference, so it keeps today's behaviour: `image.crop` with an `angle`
    // turns through the same `rotate_arbitrary_with` the menu item runs, and stays ungated.
    let mut s = bitmap(32, 24);
    assert!(s.is_enabled("image.crop"), "Crop stays available in a Bitmap document");
    let history = steps(&s);
    let r = s
        .execute("image.crop", json!({"x": 4, "y": 4, "width": 24, "height": 16, "angle": 10}))
        .unwrap_or_else(|e| panic!("the Crop straighten turn keeps today's behaviour, got {e}"));
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(24), Some(16)), "{r}");
    assert_eq!(size_of(&s), (24, 16), "turned and cropped, as today");
    assert_eq!(steps(&s), history + 1, "one History step");
}
