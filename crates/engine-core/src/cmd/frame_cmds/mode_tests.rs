//! #13886: `layer.new.frame` refuses Bitmap and Indexed Color documents, naming the mode and
//! changing nothing. RGB 8/16/32, CMYK, Grayscale and Lab keep making frames; Duotone and
//! Multichannel were not measured, so they keep today's behaviour.

use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = crate::session();
    s.add_document(Document::with_background("mode", Size::new(80, 60), mode, depth, Color::rgb(0.5, 0.5, 0.5)), None);
    s
}

fn request() -> Value {
    json!({"x": 10, "y": 10, "width": 40, "height": 30})
}

#[test]
fn bitmap_and_indexed_documents_refuse_and_change_nothing() {
    for (mode, want) in [(ColorMode::Bitmap, "Bitmap"), (ColorMode::Indexed, "Indexed Color")] {
        let mut s = session(mode, SampleType::U8);
        assert!(!s.is_enabled("layer.new.frame"), "{mode:?}: the command is disabled");
        let before = s.active().unwrap().doc.clone();
        let (layers, steps, active) = {
            let st = s.active().unwrap();
            (st.doc.layers.len(), st.history.past_len(), st.active_layer)
        };
        let err = s.execute("layer.new.frame", request()).unwrap_err().to_string();
        assert!(err.contains(want), "{mode:?}: the error names the mode: {err}");
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "{mode:?}: the document is untouched");
        assert_eq!(st.doc.layers.len(), layers, "{mode:?}: no layer");
        assert_eq!(st.history.past_len(), steps, "{mode:?}: no History step");
        assert_eq!(st.active_layer, active, "{mode:?}: the active layer is unchanged");
    }
}

#[test]
fn other_modes_keep_making_frames_at_every_depth() {
    let measured = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab]
        .into_iter()
        .flat_map(|m| [SampleType::U8, SampleType::U16, SampleType::F32].map(|d| (m, d)));
    // Not measured in the reference: they keep today's behaviour.
    let unmeasured = [(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)];
    for (mode, depth) in measured.chain(unmeasured) {
        let mut s = session(mode, depth);
        assert!(s.is_enabled("layer.new.frame"), "{mode:?}/{depth:?}: the command is enabled");
        let (layers, steps) = {
            let st = s.active().unwrap();
            (st.doc.layers.len(), st.history.past_len())
        };
        let v = s.execute("layer.new.frame", request()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
        let id = LayerId(v["layer"].as_u64().unwrap_or_else(|| panic!("{mode:?}/{depth:?}: no layer id")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layers.len(), layers + 1, "{mode:?}/{depth:?}: one frame layer");
        assert_eq!(st.history.past_len(), steps + 1, "{mode:?}/{depth:?}: one History step");
        assert!(st.doc.layer(id).is_some_and(|l| l.frame.is_some()), "{mode:?}/{depth:?}: the layer is a frame");
        assert_eq!(st.active_layer, Some(id), "{mode:?}/{depth:?}: the frame is selected");
    }
}

#[test]
fn no_document_still_refuses_without_a_panic() {
    let mut s = crate::session();
    assert!(!s.is_enabled("layer.new.frame"));
    let err = s.execute("layer.new.frame", request()).unwrap_err().to_string();
    assert!(err.contains("no document open"), "{err}");
}
