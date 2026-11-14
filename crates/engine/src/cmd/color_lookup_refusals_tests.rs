//! #14429: Image > Adjustments > Color Lookup is unavailable in Grayscale, Indexed, Multichannel
//! and Bitmap documents and on a hidden pixel layer (the menu entry greyed and the command
//! answering `The command "Color Lookup" is not currently available.`), the same for a step
//! targeting a layer mask; CMYK and Lab stay enabled but fail at apply; an empty pixel layer
//! answers its own message. No refusal adds a history state or touches a pixel. RGB at 8, 16 and
//! 32 bit and a smart object (a smart filter) keep working, and the other adjustments keep today's
//! checks - Brightness/Contrast still runs on a targeted layer mask.

use super::*;
use photocraft_color::ColorMode;
use photocraft_doc::{Color, Document, SampleType, Size};

/// Measured texts (#14429), spelled out here so the test fails if the app ever drifts.
const NOT_AVAILABLE: &str = "The command \"Color Lookup\" is not currently available.";
const EMPTY_LAYER: &str = "Could not complete the Color Lookup command because the active layer is empty.";

/// A 32 x 24 white-background document in `mode`.
fn session(mode: ColorMode) -> Session {
    let d = Document::with_background("lookup", Size::new(32, 24), mode, SampleType::U8, Color::WHITE);
    let mut s = Session::new();
    s.add_document(d, None);
    s
}

/// The Background's pixel at (16, 12) and the History depth, to prove a refusal changes neither.
fn snapshot(s: &Session) -> (Vec<f32>, usize) {
    let st = s.active().unwrap();
    let pixels = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(16, 12).to_vec();
    (pixels, st.history.past_len())
}

/// Run Color Lookup and require the measured refusal `want`, with nothing changed. The engine
/// wraps a greyed command's precondition text in `Disabled` and a run-level error in `Other`;
/// both must carry the measured sentence itself, which is what the reference dialog shows.
fn refused(s: &mut Session, case: &str, want: &str, p: Value) {
    let (pixels, past) = snapshot(s);
    let got = match s.execute("image.adjustments.colorLookup", p) {
        Ok(v) => panic!("{case}: ran anyway: {v}"),
        Err(EngineError::Disabled(_, why)) | Err(EngineError::Other(why)) => why,
        Err(e) => panic!("{case}: refused with the wrong error kind: {e}"),
    };
    assert_eq!(got, want, "{case}: refused with the wrong text");
    let (after, past_after) = snapshot(s);
    assert_eq!((after.as_slice(), past_after), (pixels.as_slice(), past), "{case}: the refusal changed the document");
}

#[test]
fn unavailable_modes_are_greyed_and_refused() {
    for mode in [ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Multichannel] {
        let mut s = session(mode);
        assert!(!s.is_enabled("image.adjustments.colorLookup"), "{mode:?}: the menu entry is greyed");
        refused(&mut s, &format!("{mode:?} document"), NOT_AVAILABLE, json!({"lut": "warm"}));
    }
}

#[test]
fn bitmap_keeps_the_shared_adjustments_refusal_text() {
    // Bitmap is already refused for every Image > Adjustments but Invert (PR #10545); #14429
    // only asks that this command's test suite pin it too.
    let mut s = session(ColorMode::Bitmap);
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "Bitmap greys the entry");
    refused(&mut s, "Bitmap document", "not available in Bitmap mode", json!({"lut": "warm"}));
}

#[test]
fn cmyk_and_lab_stay_enabled_but_fail_at_apply() {
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        let mut s = session(mode);
        assert!(s.is_enabled("image.adjustments.colorLookup"), "{mode:?}: the dialog opens");
        refused(&mut s, &format!("{mode:?} document, OK"), NOT_AVAILABLE, json!({"lut": "cool"}));
    }
}

#[test]
fn a_hidden_pixel_layer_is_unavailable() {
    let mut s = session(ColorMode::Rgb);
    let id = s.execute("layer.new.layer", json!({"name": "ghost"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.hideLayers", json!({"layer": id})).unwrap();
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "a hidden active pixel layer greys the entry");
    refused(&mut s, "hidden layer", NOT_AVAILABLE, json!({"lut": "warm"}));
}

#[test]
fn a_step_targeting_a_layer_mask_is_unavailable() {
    let mut s = session(ColorMode::Rgb);
    // A real layer mask, so the generic mask plumbing is happy and Color Lookup's own
    // refusal is what answers.
    s.execute("layer.layerMask.fromTransparency", json!({})).unwrap();
    let before = snapshot(&s);
    let got = match s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "target": "mask"})) {
        Ok(v) => panic!("the mask step ran anyway: {v}"),
        Err(EngineError::Disabled(_, why)) | Err(EngineError::Other(why)) => why,
        Err(e) => panic!("wrong error kind: {e}"),
    };
    assert_eq!(got, NOT_AVAILABLE, "a targeted layer mask answers the measured text");
    assert_eq!(snapshot(&s), before, "and neither layer nor mask pixels changed");
    // The other adjustments keep today's behaviour: Brightness/Contrast still runs on the mask.
    assert!(
        s.execute("image.adjustments.brightnessContrast", json!({"target": "mask", "brightness": 10})).is_ok(),
        "Brightness/Contrast keeps the mask target"
    );
}

#[test]
fn an_empty_pixel_layer_gets_its_own_message() {
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.new.layer", json!({"name": "empty"})).unwrap();
    let st = s.active().unwrap();
    let empty_is_active = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().is_some_and(|surf| surf.content_bounds().is_empty());
    assert!(empty_is_active, "layer.new.layer leaves an empty pixel layer active");
    assert!(s.is_enabled("image.adjustments.colorLookup"), "the menu entry stays enabled");
    refused(&mut s, "empty layer", EMPTY_LAYER, json!({"lut": "warm"}));
}

#[test]
fn rgb_at_every_depth_and_a_smart_object_keep_working() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = Document::with_background("rgb", Size::new(16, 12), ColorMode::Rgb, depth, Color::WHITE);
        let mut s = Session::new();
        s.add_document(d, None);
        assert!(s.is_enabled("image.adjustments.colorLookup"), "{depth:?}: enabled");
        s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).unwrap_or_else(|e| panic!("{depth:?}: {e}"));
        assert!(s.active().unwrap().history.past_len() > 0, "{depth:?}: one history step");
    }
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).unwrap();
}
