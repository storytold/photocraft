//! Tests that run paint commands, moved from the engine-core `float_cmds` tests: 5 test(s).

use photocraft_doc::{Document, LayerId};
use photocraft_engine::Session;
use photocraft_geom::Rect;

use serde_json::json;

use photocraft_engine::float_cmds::*;

/// An 80×60 transparent document whose layer is painted red over (10..30)²; (10..30)² selected.
fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "background": "transparent"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("paint", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    (s, id)
}

fn alpha(d: &Document, id: LayerId, x: i32, y: i32) -> f32 {
    d.layer(id).unwrap().surface().unwrap().rgba(x, y)[3]
}

/// Copying leaves the original pixels, at every depth; the move is one undo step.
#[test]
fn a_copy_leaves_the_original_at_every_depth_in_one_undo_step() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 60, "background": "transparent", "depth": depth})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        s.execute("select.float", json!({"dx": 30, "dy": 0, "copy": true})).unwrap();
        s.execute("select.drop", json!({})).unwrap();
        let st = s.active().unwrap();
        assert!(alpha(&st.doc, id, 12, 12) == 1.0 && alpha(&st.doc, id, 42, 12) == 1.0, "{depth}-bit: both");
        assert_eq!((st.history.past_len(), st.history.undo_label()), (steps + 1, Some("Move")), "{depth}-bit");
        s.undo();
        let st = s.active().unwrap();
        assert!(alpha(&st.doc, id, 12, 12) == 1.0 && alpha(&st.doc, id, 42, 12) == 0.0, "{depth}-bit: undone");
    }
}

/// Coverage of `d`'s selection at (x, y).
fn selected(d: &Document, x: i32, y: i32) -> f32 {
    d.selection.as_ref().map_or(0.0, |s| s.sample_channel(x, y, 0))
}

/// #7538: a marquee drawn loosely around an object on a transparent layer shrinks to the object
/// once its pixels move, as in Photoshop, at every depth and when moving a copy.
#[test]
fn moving_shrinks_the_selection_to_the_moved_pixels_at_every_depth() {
    for depth in [8, 16, 32] {
        for copy in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 80, "height": 60, "background": "transparent", "depth": depth})).unwrap();
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            // A marquee with empty space all round the red square.
            s.execute("select.rect", json!({"x": 4, "y": 6, "width": 34, "height": 30})).unwrap();
            s.execute("select.float", json!({"dx": 20, "dy": 5, "copy": copy})).unwrap();
            let st = s.active().unwrap();
            let outline = floating(st).unwrap().parts.outline().unwrap();
            assert_eq!(outline.content_bounds(), Rect::new(10, 10, 30, 30), "{depth}-bit, copy {copy}: the outline while floating");
            assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(4, 6, 38, 36), "the document's is untouched");
            s.execute("select.drop", json!({})).unwrap();
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.selection.as_ref().unwrap().content_bounds(), Rect::new(30, 15, 50, 35), "{depth}-bit, copy {copy}");
            assert_eq!((selected(doc, 40, 25), selected(doc, 26, 13)), (1.0, 0.0), "{depth}-bit, copy {copy}: the moved square only");
            // Undo brings back the marquee as it was drawn.
            s.undo();
            assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), Rect::new(4, 6, 38, 36), "{depth}-bit, copy {copy}");
        }
    }
}

/// #7538: an anti-aliased shape moved, dropped, then moved again from its shrunk selection leaves
/// nothing behind. With the selection shrunk to the pixels' alpha, the second move left
/// a × (1 − a) of each edge pixel (up to 25 %), a faint outline where the shape had been.
#[test]
fn moving_again_from_the_shrunk_selection_leaves_no_edge_behind() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 120, "height": 100, "background": "transparent", "depth": depth})).unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 50, "height": 40, "ellipse": true})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let total = |doc: &Document| (0..100).flat_map(|y| (0..120).map(move |x| (x, y))).map(|(x, y)| alpha(doc, id, x, y)).sum::<f32>();
        let (before, soft) = {
            let doc = &s.active().unwrap().doc;
            (total(doc), (0..100).flat_map(|y| (0..120).map(move |x| (x, y))).filter(|&(x, y)| (0.01..0.99).contains(&alpha(doc, id, x, y))).count())
        };
        assert!(soft > 20, "{depth}-bit: the ellipse has anti-aliased edges ({soft})");
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 70, "height": 60})).unwrap();
        s.execute("select.float", json!({"dx": 40, "dy": 30})).unwrap();
        s.execute("select.drop", json!({})).unwrap();
        // The ellipse now sits at (60..110, 50..90); move it back where it started.
        s.execute("select.float", json!({"dx": -40, "dy": -30})).unwrap();
        s.execute("select.drop", json!({})).unwrap();
        let doc = &s.active().unwrap().doc;
        let leftover = (50..100)
            .flat_map(|y| (55..115).map(move |x| (x, y)))
            .filter(|&(x, y)| x >= 72 || y >= 62)
            .map(|(x, y)| alpha(doc, id, x, y))
            .fold(0.0f32, f32::max);
        assert_eq!(leftover, 0.0, "{depth}-bit: nothing left where it was moved from");
        assert!((total(doc) - before).abs() < 0.05, "{depth}-bit: every edge pixel came back whole ({} vs {before})", total(doc));
    }
}

/// After a refused command the piece of [`session`], floated 15 px right, still floats: the
/// document, its selection and its History are as they were before the command (#13169).
fn assert_still_floating(s: &Session, id: LayerId, steps: usize, what: &str) {
    let st = s.active().unwrap();
    assert_eq!(floating(st).map(|f| f.offset), Some((15, 0)), "{what}: the piece still floats");
    assert!(alpha(&st.doc, id, 12, 12) == 1.0 && alpha(&st.doc, id, 35, 12) == 0.0, "{what}: nothing dropped");
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(10, 10, 30, 30), "{what}: the selection did not move");
    assert_eq!(st.history.past_len(), steps, "{what}: no History step");
}

#[test]
fn a_command_refused_on_a_hidden_layer_leaves_the_piece_floating() {
    let (mut s, id) = session();
    s.edit("hide", |doc, _| {
        doc.layer_mut(id).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let steps = s.active().unwrap().history.past_len();
    s.execute("select.float", json!({"dx": 15, "dy": 0})).unwrap();
    let refused = s.execute("paint.stroke", json!({"points": [[40, 40], [50, 50]], "color": "#00ff00"}));
    assert!(refused.unwrap_err().to_string().contains(photocraft_engine::hidden_target::HIDDEN));
    assert_still_floating(&s, id, steps, "Brush on a hidden layer");
}

/// The issue's own case: the Magic Eraser in an Indexed Color document answers with its alert
/// (#12032) and leaves the moved piece floating.
#[test]
fn a_magic_eraser_click_refused_in_indexed_color_leaves_the_piece_floating() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    s.execute("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    let refused = s.execute("paint.magicEraser", json!({"x": 30, "y": 30}));
    assert_eq!(refused.unwrap_err().to_string(), photocraft_engine::eraser_cmds::MAGIC_ERASER_INDEXED_ERROR, "the tool's alert, unchanged");
    let st = s.active().unwrap();
    assert_eq!(floating(st).map(|f| f.offset), Some((5, 0)), "the piece still floats");
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(5, 5, 15, 15));
    assert_eq!(st.history.past_len(), steps, "no History step");
}
