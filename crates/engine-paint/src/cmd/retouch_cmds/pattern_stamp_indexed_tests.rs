//! #12887: in an Indexed Color document the Pattern Stamp refuses a pattern whose source is not
//! an Indexed image (RGB, Grayscale, CMYK, Lab, or one read from a file) with the reference
//! application's message, painting nothing and adding no history; the live strokes are refused the
//! same way. #13071: a pattern Edit › Define Pattern took from an Indexed image keeps painting
//! there, and RGB, Grayscale, CMYK and Lab documents at every depth keep painting either way.

use super::super::LiveRetouch;
use super::*;

/// A new `mode` document of `depth` bits, 64 × 64, white.
fn new_doc(s: &mut Session, mode: &str, depth: u64) {
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": depth, "mode": mode})).unwrap();
}

/// The reference's test pattern: 20 × 20 with red, green, blue and white quadrants, defined from a
/// `mode` document. Returns its id.
fn quadrants(s: &mut Session, mode: &str) -> String {
    s.execute("file.new", json!({"width": 20, "height": 20, "depth": 8, "mode": mode})).unwrap();
    for (x, y, color) in [(0, 0, "#ff0000"), (10, 0, "#00ff00"), (0, 10, "#0000ff")] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": format!("Quadrants {mode}")})).unwrap();
    r["pattern"].as_str().unwrap().to_string()
}

fn to_indexed(s: &mut Session) {
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
}

fn stroke(pattern: &str, impressionist: bool) -> Value {
    json!({"points": [[12, 32], [52, 32]], "size": 30, "hardness": 100, "pattern": pattern, "impressionist": impressionist})
}

/// The active layer's pixels and the History length.
fn state(s: &Session) -> (Vec<[f32; 4]>, usize) {
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    ((0..64).flat_map(|y| (0..64).map(move |x| surf.rgba(x, y))).collect(), st.history.past_len())
}

#[test]
fn an_rgb_pattern_is_refused_in_an_indexed_document_with_the_reference_words() {
    let mut s = crate::session();
    let pat = quadrants(&mut s, "rgb");
    new_doc(&mut s, "rgb", 8);
    to_indexed(&mut s);
    let before = state(&s);
    for impressionist in [false, true] {
        let err = s.execute("paint.patternStamp", stroke(&pat, impressionist)).unwrap_err().to_string();
        assert_eq!(err, INDEXED_REFUSAL, "impressionist {impressionist}");
        assert_eq!(state(&s), before, "impressionist {impressionist}: nothing painted, no history");
    }
    // The press check the canvas alerts with, and the live strokes, which preview nothing.
    assert_eq!(press_refusal(&s, &stroke(&pat, false)).as_deref(), Some(INDEXED_REFUSAL));
    let live = LiveRetouch::begin(&s, "paint.patternStamp", &stroke(&pat, false)).err().map(|e| e.to_string());
    assert_eq!(live.as_deref(), Some(INDEXED_REFUSAL), "live");
    let dabs = live_impressionist(&s, &stroke(&pat, true)).err().map(|e| e.to_string());
    assert_eq!(dabs.as_deref(), Some(INDEXED_REFUSAL), "live Impressionist");
}

#[test]
fn other_documents_keep_painting_with_the_rgb_pattern() {
    let mut s = crate::session();
    let pat = quadrants(&mut s, "rgb");
    for mode in ["rgb", "grayscale", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            new_doc(&mut s, mode, depth);
            let (pixels, steps) = state(&s);
            assert_eq!(press_refusal(&s, &stroke(&pat, false)), None, "{mode}/{depth}");
            s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_or_else(|e| panic!("{mode}/{depth}: {e}"));
            let after = state(&s);
            assert_ne!(after.0, pixels, "{mode}/{depth}: painted");
            assert_eq!(after.1, steps + 1, "{mode}/{depth}: one history step");
        }
    }
}

#[test]
fn grayscale_cmyk_and_lab_patterns_are_refused_in_an_indexed_document_too() {
    let mut s = crate::session();
    let patterns = ["grayscale", "cmyk", "lab"].map(|mode| quadrants(&mut s, mode));
    new_doc(&mut s, "rgb", 8);
    to_indexed(&mut s);
    let before = state(&s);
    for pat in patterns {
        let err = s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_err().to_string();
        assert_eq!(err, INDEXED_REFUSAL, "{pat}");
        assert_eq!(press_refusal(&s, &stroke(&pat, false)).as_deref(), Some(INDEXED_REFUSAL), "{pat}");
        assert_eq!(state(&s), before, "{pat}: nothing painted, no history");
    }
}

/// The reference's measured case: one Indexed Color document holds the 20 × 20 test tile (red,
/// green and blue quadrants on white, an RGB document converted with its exact palette) and Edit ›
/// Define Pattern takes the pattern from that same document, so it carries its colour table
/// (#13071). The active document is the Indexed one.
fn indexed_doc_and_pattern(s: &mut Session) -> String {
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    for (x, y, color) in [(0, 0, "#ff0000"), (10, 0, "#00ff00"), (0, 10, "#0000ff")] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    to_indexed(s);
    let r = s.execute("edit.definePattern", json!({"name": "Indexed quadrants", "rect": [0, 0, 20, 20]})).unwrap();
    r["pattern"].as_str().unwrap().to_string()
}

#[test]
fn a_pattern_defined_from_an_indexed_image_keeps_painting_in_an_indexed_document() {
    let mut s = crate::session();
    let pat = indexed_doc_and_pattern(&mut s);
    assert_eq!(s.patterns.items.iter().find(|q| q.id == pat).unwrap().source_mode, Some(ColorMode::Indexed));
    let (pixels, steps) = state(&s);
    assert_eq!(press_refusal(&s, &stroke(&pat, false)), None, "a press over the tile paints");
    s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_or_else(|e| panic!("indexed source: {e}"));
    let after = state(&s);
    assert_ne!(after.0, pixels, "the stroke painted its tile");
    assert_eq!(after.1, steps + 1, "one history step");
    // An aligned stroke keeps the document origin, so (x, y) takes the tile's (x mod 20, y mod 20):
    // the pattern's own samples land, unconverted, in colours of the document's palette.
    assert_eq!(after.0[25 * 64 + 5], [1.0, 0.0, 0.0, 1.0], "the tile's red at (5, 25)");
    assert_eq!(after.0[25 * 64 + 12], [0.0, 1.0, 0.0, 1.0], "the tile's green at (12, 25)");
    assert_eq!(after.0[60 * 64 + 60], [1.0, 1.0, 1.0, 1.0], "outside the stroke the canvas stays white");
    // The live strokes take the same path as the commit.
    let live = LiveRetouch::begin(&s, "paint.patternStamp", &stroke(&pat, false));
    assert!(live.is_ok(), "live: {:?}", live.err().map(|e| e.to_string()));
    let dabs = live_impressionist(&s, &stroke(&pat, true));
    assert!(dabs.is_ok(), "live Impressionist: {:?}", dabs.err().map(|e| e.to_string()));
}

#[test]
fn an_indexed_source_pattern_paints_in_other_documents_as_before() {
    let mut s = crate::session();
    let pat = indexed_doc_and_pattern(&mut s);
    for mode in ["rgb", "grayscale", "cmyk", "lab"] {
        new_doc(&mut s, mode, 8);
        let (pixels, steps) = state(&s);
        assert_eq!(press_refusal(&s, &stroke(&pat, false)), None, "{mode}");
        s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_or_else(|e| panic!("{mode}: {e}"));
        let after = state(&s);
        assert_ne!(after.0, pixels, "{mode}: painted");
        assert_eq!(after.1, steps + 1, "{mode}: one history step");
    }
}

/// The one measured row of #12887 this change knowingly does not keep: the reference refuses an
/// Indexed pattern from ANOTHER image, whose colours are not in the document's colour table, and
/// paints only the one that carries that table. PhotoCraft records the source mode, not the table,
/// so this paints - the deviation tracked as #16642, pinned here exactly as it behaves, with the
/// landed colours shown to be outside the document's own table.
#[test]
fn a_pattern_defined_in_another_indexed_document_paints_today() {
    let mut s = crate::session();
    let pat = indexed_doc_and_pattern(&mut s);
    // A second Indexed document, cyan over magenta: its own exact palette, which holds none of the
    // tile's red, green and blue.
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    for (y, color) in [(0, "#00ffff"), (32, "#ff00ff")] {
        s.execute("select.rect", json!({"x": 0, "y": y, "width": 64, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    to_indexed(&mut s);
    let table = s.active().unwrap().doc.color_table.clone().expect("an Indexed document has a colour table");
    assert!(!table.colors.contains(&[255, 0, 0]), "the second table holds {table:?}");
    let before = state(&s);
    assert_eq!(press_refusal(&s, &stroke(&pat, false)), None, "no refusal for a foreign Indexed source");
    s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_or_else(|e| panic!("foreign Indexed source: {e}"));
    let after = state(&s);
    assert_eq!(after.1, before.1 + 1, "one history step");
    assert_eq!(after.0[25 * 64 + 5], [1.0, 0.0, 0.0, 1.0], "the tile's red lands, though this table has no red");
    assert_eq!(after.0[25 * 64 + 12], [0.0, 1.0, 0.0, 1.0], "and its green, which this table lacks too");
}

#[test]
fn a_pattern_whose_source_is_not_an_indexed_image_is_refused_in_an_indexed_document() {
    let mut s = crate::session();
    // A library pattern, as the built-ins and a `.pat` file are: no recorded source.
    s.patterns.items.push(Pattern {
        id: "no-source".into(),
        name: "No source".into(),
        width: 4,
        height: 4,
        source_mode: None,
        surface: Surface::new(PixelFormat::RGBA8),
    });
    new_doc(&mut s, "rgb", 8);
    to_indexed(&mut s);
    let before = state(&s);
    for key in ["no-source", "Checkerboard"] {
        let err = s.execute("paint.patternStamp", stroke(key, false)).unwrap_err().to_string();
        assert_eq!(err, INDEXED_REFUSAL, "{key}");
        assert_eq!(press_refusal(&s, &stroke(key, false)).as_deref(), Some(INDEXED_REFUSAL), "{key}");
        assert_eq!(state(&s), before, "{key}: nothing painted, no history");
    }
}

/// A `.pat` file names its pixels' model, not the document they came from, so an imported pattern
/// records no source mode and an Indexed Color document keeps refusing it (#12887).
#[test]
fn a_pattern_imported_from_a_pat_file_stays_refused_in_an_indexed_document() {
    let mut s = crate::session();
    let pat = indexed_doc_and_pattern(&mut s);
    let stored = s.patterns.items.iter().find(|q| q.id == pat).cloned().unwrap();
    let bytes = photocraft_io::pattern_map::write_pat(std::slice::from_ref(&stored)).unwrap();
    let mut t = crate::session();
    t.execute("pattern.import", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(t.patterns.items.iter().find(|q| q.id == pat).unwrap().source_mode, None, "the file records no source");
    new_doc(&mut t, "rgb", 8);
    to_indexed(&mut t);
    let before = state(&t);
    let err = t.execute("paint.patternStamp", stroke(&pat, false)).unwrap_err().to_string();
    assert_eq!(err, INDEXED_REFUSAL);
    assert_eq!(press_refusal(&t, &stroke(&pat, false)).as_deref(), Some(INDEXED_REFUSAL));
    assert_eq!(state(&t), before, "nothing painted, no history");
}

/// The Patterns panel and `pattern.list` still show the Indexed-source pattern as the RGB tile it
/// is: only the source mode was added, the pixels and what the panel lists did not change.
#[test]
fn the_pattern_list_shows_an_indexed_source_pattern_as_the_rgb_tile_it_is() {
    let mut s = crate::session();
    let pat = indexed_doc_and_pattern(&mut s);
    let list = s.execute("pattern.list", json!({})).unwrap();
    let entry = list.as_array().unwrap().iter().find(|e| e["id"].as_str() == Some(&pat)).unwrap();
    assert_eq!(entry["name"], "Indexed quadrants");
    assert_eq!((entry["width"].as_u64(), entry["height"].as_u64()), (Some(20), Some(20)));
    assert_eq!(entry["mode"], "Rgb", "the tile's own pixels, as the panel has always shown it");
    assert_eq!(entry["paintable"], true);
}

/// The refusal is about the pattern, not the tool: the Clone Stamp still paints in an Indexed
/// Color document, as #12613 measured.
#[test]
fn the_clone_stamp_still_paints_in_an_indexed_document() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 6, "height": 6})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    to_indexed(&mut s);
    let steps = s.active().unwrap().history.past_len();
    s.execute("paint.cloneStamp", json!({"points": [[40, 32]], "source": [10, 10], "size": 8, "hardness": 100}))
        .unwrap_or_else(|e| panic!("clone stamp in Indexed Color: {e}"));
    let after = state(&s);
    assert_eq!(after.1, steps + 1, "one history step");
    assert_eq!(after.0[32 * 64 + 40], [1.0, 0.0, 0.0, 1.0], "the cloned red lands");
}
