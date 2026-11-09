//! Select › Save Selection with Document: New (a new Multichannel document holding the
//! selection), and with another open document as the destination.

use super::planes::read_plane;
use super::*;
use photocraft_color::SampleType;

/// A `w`×`h` document at `depth` bits with a feathered rectangle selected, so the saved
/// channel has partial values as well as 0 and 1.
fn feathered(depth: u32, w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth, "resolution": 150})).unwrap();
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 8})).unwrap();
    s.execute("select.modify.feather", json!({"radius": 2})).unwrap();
    s
}

/// The selection of document `i` over its canvas, row by row.
fn selection_plane(s: &Session, i: usize) -> Vec<f32> {
    let d = &s.documents()[i].doc;
    sel::mask_from_surface(d.selection.as_ref(), d.bounds())
}

#[test]
fn save_to_a_new_document_makes_a_multichannel_image_of_the_selection() {
    for (depth, sample) in [(8, SampleType::U8), (16, SampleType::U16), (32, SampleType::F32)] {
        let mut s = feathered(depth, 24, 16);
        let selection = selection_plane(&s, 0);
        assert!(selection.iter().any(|v| *v > 0.0 && *v < 1.0), "the feather leaves partial values");
        let before = s.active().unwrap().history.past_len();
        let r = s.execute("select.saveSelection", json!({"document": "new"})).unwrap();
        assert_eq!(r, json!({"channel": 0, "name": "Alpha 1", "document": 1}));
        assert_eq!(s.active_index(), Some(1), "the new document is the active one");
        let d = &s.documents()[1].doc;
        assert_eq!(d.name, "Untitled-1");
        assert_eq!(d.mode, ColorMode::Multichannel);
        assert_eq!((d.size, d.depth, d.resolution_dpi), (s.documents()[0].doc.size, sample, 150.0));
        assert!(d.layers.is_empty() && d.selection.is_none());
        assert_eq!(d.channels.len(), 1);
        let ch = &d.channels[0];
        assert_eq!(ch.name, "Alpha 1");
        assert_eq!(ch.spot, Some((Color::BLACK, 0.0)), "a black channel, as a converted Lab image's Alpha channels");
        assert_eq!(ch.surface.format(), PixelFormat::new(ColorMode::Grayscale, sample, false));
        // Ink = the unselected amount, so the image shows the selection white on black.
        let ink = read_plane(&ch.surface, d.bounds());
        let tol = if depth == 32 { 1e-6 } else { 0.5 / 255.0 + 1e-6 };
        for (k, (a, b)) in ink.iter().zip(&selection).enumerate() {
            assert!((a - (1.0 - b)).abs() <= tol, "{depth}-bit pixel {k}: ink {a} for selection {b}");
        }
        let shown = photocraft_compose::flatten(d);
        let at = |x: usize, y: usize| shown.px[y * 24 + x];
        assert!(at(10, 8)[0] > 0.99 && at(10, 8)[2] > 0.99, "selected: white, {:?}", at(10, 8));
        assert!(at(0, 0)[0] < 0.01, "unselected: black, {:?}", at(0, 0));
        // The source keeps its selection and gets no channel or history step.
        let src = &s.documents()[0];
        assert!(src.doc.channels.is_empty());
        assert_eq!(src.history.past_len(), before);
        assert_eq!(selection_plane(&s, 0), selection);
    }
}

#[test]
fn the_new_documents_channel_loads_back_as_the_selection() {
    let mut s = feathered(16, 24, 16);
    let selection = selection_plane(&s, 0);
    s.execute("select.saveSelection", json!({"document": "new", "name": "Keep"})).unwrap();
    assert_eq!(s.documents()[1].doc.channels[0].name, "Keep");
    s.set_active(0);
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("select.loadSelection", json!({"document": 1, "channel": "Keep"})).unwrap();
    let loaded = selection_plane(&s, 0);
    assert_eq!(loaded.len(), selection.len());
    for (k, (a, b)) in loaded.iter().zip(&selection).enumerate() {
        assert!((a - b).abs() <= 1.0 / 255.0 + 1e-6, "pixel {k}: loaded {a}, saved {b}");
    }
}

#[test]
fn each_new_document_takes_the_next_untitled_name() {
    let mut s = feathered(8, 24, 16);
    s.execute("select.saveSelection", json!({"document": "new", "name": "  "})).unwrap();
    s.set_active(0);
    let r = s.execute("select.saveSelection", json!({"document": "new", "channel": "new", "operation": "new"})).unwrap();
    assert_eq!(r["document"], 2);
    let names: Vec<_> = s.documents().iter().map(|d| (d.doc.name.clone(), d.doc.channels.iter().map(|c| c.name.clone()).collect::<Vec<_>>())).collect();
    assert_eq!(names[1], ("Untitled-1".to_string(), vec!["Alpha 1".to_string()]), "a blank name is the default");
    assert_eq!(names[2], ("Untitled-2".to_string(), vec!["Alpha 1".to_string()]));
}

#[test]
fn a_new_document_has_no_channel_to_replace_or_combine_with() {
    let mut s = feathered(8, 24, 16);
    s.execute("select.saveSelection", json!({"name": "Mine"})).unwrap();
    for p in [
        json!({"document": "new", "channel": 0}),
        json!({"document": "new", "channel": "Mine"}),
        json!({"document": "new", "channel": "Mine", "operation": "replace"}),
        json!({"document": "new", "operation": "add"}),
        json!({"document": "new", "operation": "intersect"}),
        json!({"document": "new", "operation": "bogus"}),
    ] {
        assert!(s.execute("select.saveSelection", p.clone()).is_err(), "{p}");
        assert_eq!(s.documents().len(), 1, "{p}: no document made");
        assert_eq!(s.active_index(), Some(0));
    }
}

#[test]
fn save_to_another_open_document_of_the_same_size() {
    let mut s = feathered(8, 24, 16);
    let selection = selection_plane(&s, 0);
    s.execute("file.new", json!({"width": 24, "height": 16, "depth": 16})).unwrap();
    s.execute("file.new", json!({"width": 30, "height": 16})).unwrap();
    s.set_active(0);
    let r = s.execute("select.saveSelection", json!({"document": 1, "name": "Elsewhere"})).unwrap();
    assert_eq!(r, json!({"channel": 0, "name": "Elsewhere", "document": 1}));
    assert_eq!(s.active_index(), Some(0), "the source stays active");
    let d = &s.documents()[1].doc;
    assert_eq!(d.channels[0].surface.format().sample, SampleType::U16, "in the destination's depth");
    let saved = read_plane(&d.channels[0].surface, d.bounds());
    assert!(saved.iter().zip(&selection).all(|(a, b)| (a - b).abs() <= 1.0 / 255.0), "the selection as it is");
    assert!(s.execute("select.saveSelection", json!({"document": 2})).is_err(), "a different size is refused");
    assert!(s.documents()[2].doc.channels.is_empty());
}
