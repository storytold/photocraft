//! A command refused while a piece floats leaves it floating (#13169).

use photocraft_engine::Session;
use photocraft_engine::eraser_cmds::MAGIC_ERASER_INDEXED_ERROR;
use photocraft_engine::float_cmds::floating;
use photocraft_geom::Rect;
use serde_json::json;

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
    assert_eq!(refused.unwrap_err().to_string(), MAGIC_ERASER_INDEXED_ERROR, "the tool's alert, unchanged");
    let st = s.active().unwrap();
    assert_eq!(floating(st).map(|f| f.offset), Some((5, 0)), "the piece still floats");
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(5, 5, 15, 15));
    assert_eq!(st.history.past_len(), steps, "no History step");
}
