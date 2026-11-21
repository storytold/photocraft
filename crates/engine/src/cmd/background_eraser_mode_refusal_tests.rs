//! #11912: the Background Eraser refuses 32-bit and Bitmap documents with the reference's alert
//! texts, and Indexed Color documents with an error (the tool shows no alert there), before it
//! samples or erases anything: no History step, the pixels as they were. 8- and 16-bit RGB,
//! Grayscale, CMYK and Lab documents still erase.

use super::*;
use photocraft_doc::{Color, Size};

/// The measured alerts, spelled out so the test fails if the text ever drifts.
const BIT32: &str =
    "Could not use the background eraser because it does not work with 32 bit per channel images (convert image to 8 or 16 bit per channel to edit).";
const BITMAP: &str = "Could not use the background eraser because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// A 64 × 48 white document in `mode` at `depth`, its Background active.
fn session(mode: ColorMode, depth: SampleType) -> Session {
    let d = Document::with_background("meadow", Size::new(64, 48), mode, depth, Color::WHITE);
    let mut s = Session::new();
    s.add_document(d, None);
    s
}

fn stroke() -> Value {
    json!({"points": [[10, 10], [30, 20], [50, 30]], "brush": {"size": 12, "hardness": 1.0}})
}

/// The History depth and one pixel under the stroke.
fn state(s: &Session) -> (usize, Vec<f32>) {
    let st = s.active().unwrap();
    let px = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 20).to_vec();
    (st.history.past_len(), px)
}

fn refused_with(mode: ColorMode, depth: SampleType, want: &str) {
    let mut s = session(mode, depth);
    let before = state(&s);
    let err = s.execute("paint.backgroundEraser", stroke()).unwrap_err();
    assert_eq!(err.to_string(), want, "{mode:?} {depth:?}");
    assert_eq!(state(&s), before, "{mode:?} {depth:?}: nothing changed");
    assert_eq!(background_eraser_refusal(&s.active().unwrap().doc), Some(want));
    assert!(live_background_eraser(&s, &stroke()).is_err(), "the live stroke is refused too");
}

#[test]
fn a_32_bit_document_is_refused_with_the_alert() {
    refused_with(ColorMode::Rgb, SampleType::F32, BIT32);
}

#[test]
fn a_bitmap_document_is_refused_with_the_alert() {
    refused_with(ColorMode::Bitmap, SampleType::U8, BITMAP);
}

#[test]
fn an_indexed_color_document_is_refused_without_an_alert_text() {
    refused_with(ColorMode::Indexed, SampleType::U8, BACKGROUND_ERASER_INDEXED_ERROR);
}

#[test]
fn eight_and_sixteen_bit_rgb_grayscale_cmyk_and_lab_still_erase() {
    for (mode, depth) in [
        (ColorMode::Rgb, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Grayscale, SampleType::U16),
        (ColorMode::Cmyk, SampleType::U8),
        (ColorMode::Cmyk, SampleType::U16),
        (ColorMode::Lab, SampleType::U8),
        (ColorMode::Lab, SampleType::U16),
    ] {
        let mut s = session(mode, depth);
        assert_eq!(background_eraser_refusal(&s.active().unwrap().doc), None, "{mode:?} {depth:?}");
        let before = state(&s).0;
        s.execute("paint.backgroundEraser", stroke()).unwrap_or_else(|e| panic!("{mode:?} {depth:?}: {e}"));
        assert!(state(&s).0 > before, "{mode:?} {depth:?}: a History step");
    }
}

/// Duotone and Multichannel aren't measured: at 32 bits they keep today's behaviour, no refusal (CodeRabbit on #16273).
#[test]
fn unmeasured_modes_keep_erasing_at_32_bits() {
    for mode in [ColorMode::Duotone, ColorMode::Multichannel] {
        let d = Document::with_background("meadow", Size::new(8, 8), mode, SampleType::F32, Color::WHITE);
        assert_eq!(background_eraser_refusal(&d), None, "{mode:?}");
    }
}
