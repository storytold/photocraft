//! #12582: the Spot Healing Brush refuses an Indexed Color document with the reference application's message,
//! healing nothing and adding no history. RGB at 8, 16 and 32 bit and Grayscale, CMYK and Lab keep healing; Bitmap,
//! Duotone and Multichannel (not measured) don't get the refusal.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

use super::*;

/// A 64 × 64 `mode` document of `depth`, mid grey with a black spot for the brush to heal.
fn spotted(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    s.add_document(Document::with_background("t", Size::new(64, 64), mode, depth, Color::rgb(0.5, 0.5, 0.5)), None);
    s.execute("select.rect", json!({"x": 28, "y": 28, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn click() -> Value {
    json!({"points": [[32, 32]], "size": 20, "hardness": 100})
}

/// The Background's native samples and the History length.
fn state(s: &Session) -> (Vec<f32>, usize) {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    ((0..64).flat_map(|y| (0..64).flat_map(move |x| surf.pixel(x, y))).collect(), st.history.past_len())
}

#[test]
fn an_indexed_document_refuses_with_the_reference_words() {
    let mut s = spotted(ColorMode::Indexed, SampleType::U8);
    let before = state(&s);
    let err = s.execute("paint.spotHealing", click()).unwrap_err().to_string();
    assert_eq!(err, super::SPOT_INDEXED_REFUSAL);
    assert!(state(&s) == before, "nothing healed, no history state");
}

#[test]
fn the_other_measured_modes_and_depths_keep_healing() {
    let cases = [
        (ColorMode::Rgb, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Rgb, SampleType::F32),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Cmyk, SampleType::U8),
        (ColorMode::Lab, SampleType::U8),
    ];
    for (mode, depth) in cases {
        let mut s = spotted(mode, depth);
        let (pixels, steps) = state(&s);
        s.execute("paint.spotHealing", click()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
        let after = state(&s);
        assert!(after.0 != pixels, "{mode:?}/{depth:?}: healed");
        assert_eq!(after.1, steps + 1, "{mode:?}/{depth:?}: one history step");
    }
}

#[test]
fn unmeasured_modes_never_get_the_indexed_refusal() {
    for mode in [ColorMode::Bitmap, ColorMode::Duotone, ColorMode::Multichannel] {
        let mut s = spotted(mode, SampleType::U8);
        let err = s.execute("paint.spotHealing", click()).err().map(|e| e.to_string());
        assert_ne!(err.as_deref(), Some(super::SPOT_INDEXED_REFUSAL), "{mode:?}");
    }
}
