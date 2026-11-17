//! #14515: Image › Calculations with Result New Document makes a Multichannel document
//! "Untitled-N" with no layers and one channel "Alpha 1" holding the result, at the source's size,
//! resolution and depth, as the reference application does. Its history is the opening snapshot
//! and one "Calculations" state, and it becomes the active document.

use photocraft_color::ColorMode;
use serde_json::{Value, json};

use crate::Session;

/// A 40 x 20 document at `depth` and 150 ppi with two alpha channels: the left half selected in
/// the first, the right half in the second.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "Source", "width": 40, "height": 20, "depth": depth, "resolution": 150, "background": "white"})).unwrap();
    for x in [0, 20] {
        s.execute("select.rect", json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap();
        s.execute("select.saveSelection", json!({})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// Screen of the two halves: white everywhere, as a new document.
fn calculations(s: &mut Session, extra: Value) -> Value {
    let mut p = json!({"source1": {"channel": 0}, "source2": {"channel": 1}, "blending": "screen", "result": "newDocument"});
    if let (Value::Object(p), Value::Object(extra)) = (&mut p, extra) {
        p.extend(extra);
    }
    s.execute("image.calculations", p).unwrap()
}

/// The result as the channel reads (its tone: the stored ink inverted, as for any spot channel).
fn tone(s: &Session, x: i32, y: i32) -> f32 {
    1.0 - s.active().unwrap().doc.channels[0].surface.pixel(x, y)[0]
}

#[test]
fn a_new_document_is_multichannel_with_one_alpha_1_channel() {
    for depth in [8, 16] {
        let mut s = session(depth);
        let source = s.active().unwrap().doc.clone();
        let r = calculations(&mut s, json!({}));
        assert_eq!(r["document"], json!(1));
        assert_eq!(s.active_index(), Some(1), "{depth} bit: the new document is active");
        let d = &s.active().unwrap().doc;
        assert_eq!(d.mode, ColorMode::Multichannel, "{depth} bit");
        assert!(d.layers.is_empty(), "{depth} bit: no layers");
        assert_eq!(d.channels.len(), 1, "{depth} bit");
        assert_eq!(d.channels[0].name, "Alpha 1", "{depth} bit");
        assert_eq!((d.size, d.depth, d.resolution_dpi), (source.size, source.depth, source.resolution_dpi), "{depth} bit");
        // Screen of the two halves is white on both sides.
        assert!((tone(&s, 5, 5) - 1.0).abs() < 0.01 && (tone(&s, 35, 5) - 1.0).abs() < 0.01, "{depth} bit");
    }
}

#[test]
fn the_channel_holds_the_result() {
    let mut s = session(8);
    calculations(&mut s, json!({"blending": "multiply"}));
    // Multiply of two disjoint halves is black everywhere; Normal keeps source 1 (the left half).
    assert!(tone(&s, 5, 5).abs() < 0.01 && tone(&s, 35, 5).abs() < 0.01);
    s.set_active(0);
    calculations(&mut s, json!({"blending": "normal"}));
    assert!((tone(&s, 5, 5) - 1.0).abs() < 0.01, "left half selected in source 1");
    assert!(tone(&s, 35, 5).abs() < 0.01, "right half not");
}

#[test]
fn the_name_is_the_next_untitled_n() {
    let mut s = session(8);
    calculations(&mut s, json!({}));
    assert_eq!(s.active().unwrap().doc.name, "Untitled-1");
    s.set_active(0);
    calculations(&mut s, json!({}));
    assert_eq!(s.active().unwrap().doc.name, "Untitled-2", "the next free number");
    s.set_active(0);
    calculations(&mut s, json!({"name": "Result"}));
    assert_eq!(s.active().unwrap().doc.name, "Result", "a `name` param still names the document");
}

#[test]
fn the_history_is_the_opening_snapshot_and_one_calculations_state() {
    let mut s = session(8);
    calculations(&mut s, json!({}));
    let st = s.active().unwrap();
    assert_eq!(st.history.entries(), vec!["Calculations".to_string()]);
    if let Some(first) = st.snapshots.first() {
        assert_eq!(first.name, "Untitled-1", "the opening snapshot is named after the document");
    }
    // The source document's history is untouched.
    assert!(!s.documents()[0].history.entries().iter().any(|e| e == "Calculations"));
}
