//! #12934: `select.quick` refuses Indexed Color and Bitmap documents with the reference
//! application's alert, selecting nothing and adding no history, for a click and a drag.
//! RGB, Grayscale, CMYK and Lab at every depth, Multichannel and Duotone keep selecting, and
//! the Magic Wand keeps working in Indexed Color.

use photocraft_color::{ColorMode, SampleType};

use super::object_mode_refusal_tests::{selection, session, steps};
use super::*;

const INDEXED: &str = "Could not use the Quick Selection because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
const BITMAP: &str = "Could not use the Quick Selection because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// A click and a drag inside the session's light square.
fn strokes() -> [Value; 2] {
    [json!({"points": [[32, 32]], "size": 10}), json!({"points": [[26, 26], [32, 32], [38, 38]], "size": 10})]
}

#[test]
fn indexed_and_bitmap_documents_refuse_with_the_reference_words() {
    for (mode, want) in [(ColorMode::Indexed, INDEXED), (ColorMode::Bitmap, BITMAP)] {
        for params in strokes() {
            let mut s = session(mode, SampleType::U8);
            // A corner away from the light square, which the stroke would add to without the refusal.
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
            let (before, n) = (selection(&s), steps(&s));
            let err = s.execute("select.quick", params.clone()).unwrap_err().to_string();
            assert_eq!(err, want, "{mode:?} {params}");
            assert_eq!(selection(&s), before, "{mode:?} {params}: selection unchanged");
            assert_eq!(steps(&s), n, "{mode:?} {params}: no history");
        }
    }
}

#[test]
fn other_modes_keep_selecting() {
    // Only RGB is measured above 8 bits; the other depths keep today's behaviour.
    let modes = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab]
        .into_iter()
        .flat_map(|m| [SampleType::U8, SampleType::U16, SampleType::F32].map(|d| (m, d)));
    // Not measured in the reference: they keep today's behaviour.
    let unmeasured = [(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)];
    for (mode, depth) in modes.chain(unmeasured) {
        for params in strokes() {
            let mut s = session(mode, depth);
            let n = steps(&s);
            s.execute("select.quick", params.clone()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?} {params}: {e}"));
            assert!(selection(&s).is_some(), "{mode:?}/{depth:?} {params}: selected");
            assert_eq!(steps(&s), n + 1, "{mode:?}/{depth:?} {params}: one history step");
        }
    }
}

#[test]
fn the_magic_wand_keeps_working_in_indexed_color() {
    let mut s = session(ColorMode::Indexed, SampleType::U8);
    let n = steps(&s);
    s.execute("select.magicWand", json!({"x": 32, "y": 32})).unwrap();
    assert!(selection(&s).is_some(), "selected");
    assert_eq!(steps(&s), n + 1, "one history step");
}
