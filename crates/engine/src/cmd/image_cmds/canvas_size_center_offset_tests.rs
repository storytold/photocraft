//! #14536: on a centred axis Canvas Size moves the layers by (new - old) / 2 truncated toward
//! zero, as the reference application does, so an odd change never shifts them the extra half
//! pixel. Even changes and the corner anchors keep their whole-pixel offsets.

use serde_json::{Value, json};

use crate::Session;

/// The issue's document: 400 x 300 px with a 50 x 50 layer at 175,125.
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 175, "y": 125, "width": 50, "height": 50})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// Runs Canvas Size on a fresh document; returns the reported offset and the layer's new origin.
fn run_canvas_size(p: Value) -> (Value, (i32, i32)) {
    let mut s = doc();
    let r = s.execute("image.canvasSize", p).unwrap();
    let b = s.active().unwrap().doc.layers[1].surface().unwrap().content_bounds();
    (r["offset"].clone(), (b.x0, b.y0))
}

#[test]
fn an_odd_change_with_the_centre_anchor_truncates_the_half_pixel() {
    for (w, h, off, at) in [(401, 301, [0, 0], (175, 125)), (403, 303, [1, 1], (176, 126)), (399, 299, [0, 0], (175, 125)), (397, 297, [-1, -1], (174, 124))] {
        let p = json!({"width": w, "height": h, "anchor": "center"});
        assert_eq!(run_canvas_size(p), (json!(off), at), "{w} x {h}");
    }
}

#[test]
fn shrinking_to_one_pixel_moves_the_layers_by_minus_199_minus_149() {
    for p in [
        json!({"width": 1, "height": 1}),
        json!({"width": 0, "height": 0}),
        json!({"width": -5, "height": -5}),
        json!({"width": -400, "height": -300, "relative": true}),
    ] {
        assert_eq!(run_canvas_size(p.clone()), (json!([-199, -149]), (-24, -24)), "{p}");
    }
}

#[test]
fn the_middle_edge_anchors_truncate_on_their_centred_axis() {
    for (anchor, grow, shrink) in [("top", [1, 0], [-1, 0]), ("bottom", [1, 3], [-1, -3]), ("left", [0, 1], [0, -1]), ("right", [3, 1], [-3, -1])] {
        let (off, _) = run_canvas_size(json!({"width": 403, "height": 303, "anchor": anchor}));
        assert_eq!(off, json!(grow), "{anchor} to 403 x 303");
        let (off, _) = run_canvas_size(json!({"width": 397, "height": 297, "anchor": anchor}));
        assert_eq!(off, json!(shrink), "{anchor} to 397 x 297");
    }
}

#[test]
fn even_changes_and_the_corner_anchors_keep_their_offsets() {
    let (off, at) = run_canvas_size(json!({"width": 404, "height": 302, "anchor": "center"}));
    assert_eq!((off, at), (json!([2, 1]), (177, 126)));
    let (off, _) = run_canvas_size(json!({"width": 396, "height": 298, "anchor": "center"}));
    assert_eq!(off, json!([-2, -1]));
    for (anchor, even) in [("top", [2, 0]), ("bottom", [2, 2]), ("left", [0, 1]), ("right", [4, 1])] {
        let (off, _) = run_canvas_size(json!({"width": 404, "height": 302, "anchor": anchor}));
        assert_eq!(off, json!(even), "{anchor} to 404 x 302");
    }
    for (anchor, grow, shrink) in
        [("topLeft", [0, 0], [0, 0]), ("topRight", [3, 0], [-3, 0]), ("bottomLeft", [0, 3], [0, -3]), ("bottomRight", [3, 3], [-3, -3])]
    {
        let (off, _) = run_canvas_size(json!({"width": 403, "height": 303, "anchor": anchor}));
        assert_eq!(off, json!(grow), "{anchor} to 403 x 303");
        let (off, _) = run_canvas_size(json!({"width": 397, "height": 297, "anchor": anchor}));
        assert_eq!(off, json!(shrink), "{anchor} to 397 x 297");
    }
}
