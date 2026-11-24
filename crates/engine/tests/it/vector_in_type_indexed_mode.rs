//! Tests that run vector commands, moved from the engine-core `type_indexed_mode` tests: 3 test(s).

use photocraft_color::ColorMode;
use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use photocraft_engine::cmd::type_cmds::name_selection_commit;
use serde_json::{Value, json};

/// The typing session's coalesce key, as `ui-egui`'s `session_key` gives one.
const SESSION: &str = "type-session-12041";

/// The text the tool commits, at a baseline inside every document used here. `coalesce` for the
/// tool's own calls (one step for the session), without it for a script's or the pin's.
fn typed(coalesce: bool) -> Value {
    let mut p = json!({ "text": "Hi", "x": 20, "y": 80, "size": 48 });
    if coalesce {
        p["coalesce"] = json!(SESSION);
    }
    p
}

/// A blank RGB document, to be converted to the mode under test.
fn blank() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 240, "height": 120, "background": "transparent"})).unwrap();
    s
}

fn indexed() -> Session {
    let mut s = blank();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_eq!(mode(&s), ColorMode::Indexed, "the setup made an Indexed Color document");
    s
}

fn bitmap() -> Session {
    let mut s = blank();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.mode.bitmap", json!({"method": "threshold", "frequency": 12, "shape": "diamond"})).unwrap();
    assert_eq!(mode(&s), ColorMode::Bitmap, "the setup made a Bitmap document");
    s
}

fn mode(s: &Session) -> ColorMode {
    s.active().unwrap().doc.mode
}

/// The Type tool's commit of the session's working layer in a document [`commits_as_selection`]
/// names: the Type Mask tools' conversion, in the session's one step, which keeps the name the step
/// was opened with.
fn commit(s: &mut Session, layer: u64) {
    s.execute("type.createSelection", json!({"layer": layer, "removeLayer": true, "operation": "new", "coalesce": SESSION})).unwrap();
    name_selection_commit(s);
}

fn text_layers(s: &Session) -> usize {
    s.active().unwrap().doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).count()
}

fn layers(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

/// The selection's coverage over the whole canvas, in document pixels.
fn coverage(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap().doc.clone();
    let b = d.bounds();
    let mut out = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            out.push(d.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0)));
        }
    }
    out
}

/// The selection's bounds, as its corners.
fn selection_corners(s: &Session) -> Option<[i32; 4]> {
    s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds()).map(|r| [r.x0, r.y0, r.x1, r.y1])
}

/// Every layer's pixels, quantised: what the commit must leave exactly as it was.
fn pixels(s: &Session) -> Vec<u8> {
    let d = s.active().unwrap().doc.clone();
    let b = d.bounds();
    let mut out = Vec::new();
    for (_, _, l) in d.walk() {
        let Some(surface) = l.surface() else { continue };
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                out.extend(surface.rgba(x, y).map(|v| (v * 255.0).round() as u8));
            }
        }
    }
    out
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

#[test]
fn committing_in_indexed_color_or_bitmap_leaves_the_glyphs_selected_and_no_layer() {
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let (steps, layers_before, pixels_before) = (history(&s).len(), layers(&s), pixels(&s));

        // The tool's session: a type layer to edit and show while typing, as measured.
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(text_layers(&s), 1, "{name}: typing happens on a layer the tool can show");

        commit(&mut s, layer);

        assert_eq!(text_layers(&s), 0, "{name}: committing leaves no type layer");
        assert_eq!(layers(&s), layers_before, "{name}: the working layer went with it");
        assert!(s.active().unwrap().doc.selection.is_some(), "{name}: the glyphs are selected");
        let selected = s.active().unwrap().doc.selection.as_ref().unwrap();
        assert!(!selected.content_bounds().is_empty(), "{name}: the selection has a shape");
        assert!(coverage(&s).iter().any(|v| *v > 0.0), "{name}: the selected pixels are the glyphs'");
        assert_eq!(pixels(&s), pixels_before, "{name}: the commit paints nothing");

        // One step for the whole session, named as Photoshop names it.
        let h = history(&s);
        assert_eq!(h.len(), steps + 1, "{name}: the session is one history state: {h:?}");
        assert_eq!(h.last().map(String::as_str), Some("New Type Layer"), "{name}: its name");

        // And undoing that one state gives the document back as it was.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none(), "{name}: undo takes the selection");
        assert_eq!(layers(&s), layers_before, "{name}: undo takes the whole session");
        assert_eq!(pixels(&s), pixels_before, "{name}: nothing else was changed");
    }
}

#[test]
fn the_selection_is_the_glyph_shape_the_type_mask_of_the_same_text_makes() {
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        commit(&mut s, layer);
        let (got, got_bounds) = (coverage(&s), selection_corners(&s));

        // The pin: the same text straight through the Type Mask command.
        let mut pin = make();
        pin.execute("type.createSelection", typed(false)).unwrap();
        assert_eq!(got_bounds, selection_corners(&pin), "{name}: same selection bounds");
        assert_eq!(got, coverage(&pin), "{name}: same selection coverage");
    }
}

/// An edit that does not carry the session's `coalesce` key ends its coalesce group - in the tool
/// that is the deliberately per-press Alt+arrow kerning step. The commit then makes a state of its
/// own, which is what the Type Mask tools' commit and the plain layer's rename do in the same
/// situation today; nothing is mis-named and nothing else moves.
#[test]
fn an_uncoalesced_edit_between_typing_and_the_commit_splits_the_states() {
    for (name, make) in [("Indexed Color", indexed as fn() -> Session), ("Bitmap", bitmap as fn() -> Session)] {
        let mut s = make();
        let layer = s.execute("type.create", typed(true)).unwrap()["layer"].as_u64().unwrap();
        // The stand-in for that kerning press: a real edit under no coalesce key.
        s.execute("type.edit", json!({"layer": layer, "antialias": "sharp"})).unwrap();
        let before = history(&s);
        assert_eq!(before.last().map(String::as_str), Some("Edit Type"), "{name}: the unkeyed edit is a state of its own");

        commit(&mut s, layer);

        let h = history(&s);
        assert_eq!(h.len(), before.len() + 1, "{name}: the commit is a further state once the group ended: {h:?}");
        assert_eq!(h.last().map(String::as_str), Some("New Type Layer"), "{name}: named for the type layer either way");
        assert_eq!(h[h.len() - 2].as_str(), "Edit Type", "{name}: the edit between them keeps its own name");
        assert_eq!(text_layers(&s), 0, "{name}: still no type layer is left");
        assert!(s.active().unwrap().doc.selection.is_some(), "{name}: still the glyphs are selected");

        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none(), "{name}: one undo takes the conversion");
        assert_eq!(text_layers(&s), 1, "{name}: which brings the working layer back, as the mask tools' undo does");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(text_layers(&s), 1, "{name}: the next undo takes the unkeyed edit, not the session");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(text_layers(&s), 0, "{name}: and the third one takes the typing session");
    }
}
