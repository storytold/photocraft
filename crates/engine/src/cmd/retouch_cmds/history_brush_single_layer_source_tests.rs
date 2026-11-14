//! #12620: a History Brush source state or snapshot holding exactly one pixel layer paints into
//! any pixel layer, also one made after that state, from that layer's pixels at the same document
//! coordinates. With more layers a layer missing from the source is still refused, and a layer
//! the source has paints from itself. Measured in Photoshop 28.0.0 with the issue's document; the
//! Art History Brush was measured there too and follows the same rule.
//! Not measured, so unchanged: a one-layer source whose layer is a group, type, shape or smart
//! object, or in another colour mode or bit depth; mask, alpha channel and Quick Mask targets.

use photocraft_doc::LayerId;

use super::super::LiveRetouch;
use super::*;

const DEPTHS: [u64; 3] = [8, 16, 32];
const GREY: [f32; 3] = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0];
const GREEN: [f32; 3] = [40.0 / 255.0, 160.0 / 255.0, 60.0 / 255.0];
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

/// The issue's document: 800 × 600 RGB, created white (the open snapshot, `snapshot: 0`); the
/// Background grey left of x 400 and green right of it (state `fill`); a pixel layer "Top" with a
/// white 100 × 100 square at 600,100; a snapshot "Setup" of that (`snapshot: 1`); then the
/// Background filled red and a layer "New" made, which is active.
struct Doc {
    s: Session,
    fill: usize,
    background: LayerId,
    top: LayerId,
    new: LayerId,
}

fn active(s: &Session) -> LayerId {
    s.active().unwrap().active_layer.unwrap()
}

fn fill_rect(s: &mut Session, [x, y, width, height]: [u32; 4], color: &str) {
    s.execute("select.rect", json!({"x": x, "y": y, "width": width, "height": height})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
}

fn issue_document(depth: u64) -> Doc {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth, "mode": "rgb"})).unwrap();
    let background = active(&s);
    fill_rect(&mut s, [0, 0, 400, 600], "#808080");
    fill_rect(&mut s, [400, 0, 400, 600], "#28a03c");
    let fill = s.active().unwrap().history.past_len();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    let top = active(&s);
    fill_rect(&mut s, [600, 100, 100, 100], "#ffffff");
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("history.newSnapshot", json!({"name": "Setup"})).unwrap();
    s.execute("layer.select", json!({"layer": background.0})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "New"})).unwrap();
    let new = active(&s);
    let doc = Doc { s, fill, background, top, new };
    // The sources are what the issue describes: one layer in the open snapshot and the Fill state,
    // two in "Setup".
    for (source, layers) in [(json!({"snapshot": 0}), 1), (json!({"state": fill}), 1), (json!({"snapshot": 1}), 2)] {
        let st = doc.s.active().unwrap();
        let src = crate::history_source_cmds::paint_source(st, &source, "paint.historyBrush").unwrap();
        assert_eq!(src.layers.len(), layers, "{source}");
    }
    doc
}

/// A 40 px hard History Brush stroke through `points` from `source` (`{"snapshot": n}` or
/// `{"state": n}`).
fn stroke(source: &Value, points: Value) -> Value {
    let mut p = json!({"points": points, "size": 40, "hardness": 100});
    if let (Some(p), Some(source)) = (p.as_object_mut(), source.as_object()) {
        p.extend(source.clone());
    }
    p
}

fn select(s: &mut Session, layer: LayerId) {
    s.execute("layer.select", json!({"layer": layer.0})).unwrap();
}

fn pixel(s: &Session, layer: LayerId, x: i32, y: i32) -> [f32; 4] {
    s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(x, y)
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn assert_paints(s: &Session, layer: LayerId, (x, y): (i32, i32), rgb: [f32; 3], what: &str) {
    let got = pixel(s, layer, x, y);
    assert!(got[3] > 0.99 && (0..3).all(|i| (got[i] - rgb[i]).abs() < 0.01), "{what}: {got:?} at {x},{y}, expected {rgb:?}");
}

/// The issue's first row: the open snapshot (a single white Background) paints white into "Top"
/// and into "New", made after it.
#[test]
fn the_open_snapshot_paints_white_into_any_layer() {
    for depth in DEPTHS {
        for target in ["New", "Top"] {
            let Doc { mut s, top, new, .. } = issue_document(depth);
            let layer = if target == "Top" { top } else { new };
            select(&mut s, layer);
            let before = steps(&s);
            s.execute("paint.historyBrush", stroke(&json!({"snapshot": 0}), json!([[200, 300]]))).unwrap();
            assert_paints(&s, layer, (200, 300), WHITE, &format!("depth {depth}, {target}"));
            assert_eq!(steps(&s), before + 1, "depth {depth}, {target}: one History Brush state");
        }
    }
}

/// The issue's second row: the earlier Fill state (a single grey/green Background) paints its
/// pixels at the same document coordinates into "New": grey at 200,300, green at 600,450.
#[test]
fn an_earlier_one_layer_state_paints_its_own_pixels_into_a_later_layer() {
    for depth in DEPTHS {
        let Doc { mut s, fill, new, .. } = issue_document(depth);
        let source = json!({"state": fill});
        s.execute("paint.historyBrush", stroke(&source, json!([[200, 300]]))).unwrap();
        s.execute("paint.historyBrush", stroke(&source, json!([[600, 450]]))).unwrap();
        assert_paints(&s, new, (200, 300), GREY, &format!("depth {depth}"));
        assert_paints(&s, new, (600, 450), GREEN, &format!("depth {depth}"));
        assert!(pixel(&s, new, 200, 100)[3] < 0.01, "depth {depth}: only under the dabs");
    }
}

/// The one layer needn't be a Background: a lone transparent-document layer paints too.
#[test]
fn a_one_layer_source_that_is_not_a_background_paints_too() {
    for depth in DEPTHS {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth, "background": "transparent"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[0, 10], [40, 10]], "size": 12, "hardness": 1.0, "color": "#0000ff"})).unwrap();
        let state = steps(&s);
        assert!(!s.active().unwrap().doc.is_background(active(&s)), "depth {depth}");
        s.execute("layer.new.layer", json!({})).unwrap();
        let layer = active(&s);
        s.execute("paint.historyBrush", stroke(&json!({"state": state}), json!([[20, 10]]))).unwrap();
        assert_paints(&s, layer, (20, 10), [0.0, 0.0, 1.0], &format!("depth {depth}"));
    }
}

/// `LivePaint::History` shows what the commit paints, across the grey/green edge, from each
/// one-layer source into the later layer.
#[test]
fn the_live_stroke_matches_the_commit_from_a_one_layer_source() {
    for depth in DEPTHS {
        for open in [true, false] {
            let Doc { mut s, fill, new, .. } = issue_document(depth);
            let source = if open { json!({"snapshot": 0}) } else { json!({"state": fill}) };
            let p = stroke(&source, json!([[300, 300], [500, 320]]));
            let live = LiveRetouch::begin(&s, "paint.historyBrush", &p).unwrap_or_else(|e| panic!("depth {depth}, {source}: {e}"));
            let bounds = s.active().unwrap().doc.bounds();
            let shown = live.doc.layer(new).unwrap().surface().unwrap().read_region(bounds);
            s.execute("paint.historyBrush", p).unwrap();
            let committed = s.active().unwrap().doc.layer(new).unwrap().surface().unwrap().read_region(bounds);
            assert!(shown == committed, "depth {depth}, {source}: live and committed pixels differ");
            assert_paints(&s, new, (300, 300), if open { WHITE } else { GREY }, &format!("depth {depth}, {source}"));
        }
    }
}

/// The issue's third row: "Setup" holds Background and Top, so "New" (made after it), and a new
/// layer that replaces a deleted Top under the same name, are refused: no pixels, no state, and no
/// live stroke.
#[test]
fn a_multi_layer_source_still_refuses_a_layer_it_does_not_hold() {
    for depth in DEPTHS {
        let Doc { mut s, top, new, .. } = issue_document(depth);
        let setup = json!({"snapshot": 1});
        let before = steps(&s);
        assert!(s.execute("paint.historyBrush", stroke(&setup, json!([[200, 300]]))).is_err(), "depth {depth}: New");
        assert!(LiveRetouch::begin(&s, "paint.historyBrush", &stroke(&setup, json!([[200, 300]]))).is_err(), "depth {depth}: New, live");
        assert_eq!(steps(&s), before, "depth {depth}: no state");
        assert!(pixel(&s, new, 200, 300)[3] < 0.01, "depth {depth}: nothing painted");

        // Correspondence is by identity, not name.
        s.execute("layer.delete", json!({"layer": top.0})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        let before = steps(&s);
        assert!(s.execute("paint.historyBrush", stroke(&setup, json!([[650, 150]]))).is_err(), "depth {depth}: a new Top");
        assert_eq!(steps(&s), before, "depth {depth}: a new Top adds no state");
    }
}

/// The issue's fourth row: with "Setup" as the source, the Background and Top each paint from
/// themselves in it (today's behaviour).
#[test]
fn a_layer_in_a_multi_layer_source_paints_from_itself() {
    for depth in DEPTHS {
        let Doc { mut s, background, top, .. } = issue_document(depth);
        let setup = json!({"snapshot": 1});
        select(&mut s, background);
        s.execute("paint.historyBrush", stroke(&setup, json!([[200, 300]]))).unwrap();
        assert_paints(&s, background, (200, 300), GREY, &format!("depth {depth}, Background"));
        select(&mut s, top);
        s.execute("paint.historyBrush", stroke(&setup, json!([[650, 150]]))).unwrap();
        assert_paints(&s, top, (650, 150), WHITE, &format!("depth {depth}, Top"));
    }
}

/// Not measured, so refused as before: a one-layer source whose layer is a group, type layer,
/// shape or smart object, painting a layer made after it.
#[test]
fn a_one_layer_source_that_is_not_a_pixel_layer_keeps_refusing() {
    type Make = fn(&mut Session);
    let kinds: [(&str, Make); 4] = [
        ("group", |s| {
            s.execute("paint.stroke", json!({"points": [[0, 10], [40, 10]], "size": 12, "hardness": 1.0, "color": "#ff0000"})).unwrap();
            s.execute("layer.groupLayers", json!({})).unwrap();
        }),
        ("type", |s| {
            let first = active(s);
            s.execute("type.create", json!({"x": 2, "y": 15, "text": "Hi", "size": 12, "color": "#000000"})).unwrap();
            s.execute("layer.delete", json!({"layer": first.0})).unwrap();
        }),
        ("shape", |s| {
            let first = active(s);
            s.execute("shape.create", json!({"kind": "rect", "rect": [1, 1, 5, 5], "fill": "#0000ff"})).unwrap();
            s.execute("layer.delete", json!({"layer": first.0})).unwrap();
        }),
        ("smart object", |s| {
            s.execute("paint.stroke", json!({"points": [[0, 10], [40, 10]], "size": 12, "hardness": 1.0, "color": "#ff0000"})).unwrap();
            s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        }),
    ];
    for (kind, make) in kinds {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        make(&mut s);
        let state = steps(&s);
        assert_eq!(s.active().unwrap().doc.layers.len(), 1, "{kind}: one root layer");
        s.execute("layer.new.layer", json!({})).unwrap();
        let p = stroke(&json!({"state": state}), json!([[20, 10]]));
        let before = steps(&s);
        assert!(s.execute("paint.historyBrush", p.clone()).is_err(), "{kind}");
        assert!(LiveRetouch::begin(&s, "paint.historyBrush", &p).is_err(), "{kind}: live");
        assert_eq!(steps(&s), before, "{kind}: no state");
    }
}

/// Not measured, so refused as before: a one-layer source in another colour mode or bit depth
/// than the document, painting a layer made after the change.
#[test]
fn a_one_layer_source_in_another_mode_or_depth_keeps_refusing() {
    for change in ["image.mode.cmyk", "image.mode.lab", "image.mode.bits16", "image.mode.indexedColor"] {
        let Doc { mut s, .. } = issue_document(8);
        s.execute(change, json!({})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let p = stroke(&json!({"snapshot": 0}), json!([[200, 300]]));
        let before = steps(&s);
        assert!(s.execute("paint.historyBrush", p.clone()).is_err(), "{change}");
        assert!(LiveRetouch::begin(&s, "paint.historyBrush", &p).is_err(), "{change}: live");
        assert_eq!(steps(&s), before, "{change}: no state");
    }
}

/// Not measured, so as before: a layer mask, an alpha channel or the Quick Mask that did not exist
/// in a one-layer source is refused.
#[test]
fn masks_channels_and_the_quick_mask_keep_refusing() {
    let open = json!({"snapshot": 0});
    let mut targets: Vec<(&str, Session, Value)> = Vec::new();
    let Doc { mut s, .. } = issue_document(8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    targets.push(("layer mask", s, json!("mask")));
    let Doc { mut s, .. } = issue_document(8);
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    targets.push(("alpha channel", s, json!({"channel": 0})));
    let Doc { mut s, .. } = issue_document(8);
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    targets.push(("Quick Mask", s, json!("quickMask")));
    for (what, mut s, target) in targets {
        let mut p = stroke(&open, json!([[200, 300]]));
        p["target"] = target;
        let before = steps(&s);
        assert!(s.execute("paint.historyBrush", p.clone()).is_err(), "{what}");
        assert!(LiveRetouch::begin(&s, "paint.historyBrush", &p).is_err(), "{what}: live");
        assert_eq!(steps(&s), before, "{what}: no state");
    }
}

/// The Art History Brush, measured in Photoshop 28.0.0 on the same kind of document: from the open
/// snapshot and from the earlier one-layer state it paints a layer made after them, each dab
/// coloured from that source (it works at 8 bits only); from "Setup", with two layers, it refuses
/// that layer, adding no state.
#[test]
fn the_art_history_brush_follows_the_same_one_layer_rule() {
    for open in [true, false] {
        let Doc { mut s, fill, new, .. } = issue_document(8);
        let (source, rgb) = if open { (json!({"snapshot": 0}), WHITE) } else { (json!({"state": fill}), GREY) };
        let mut p = stroke(&source, json!([[150, 300], [200, 300]]));
        p["tolerance"] = json!(0);
        let before = steps(&s);
        s.execute("paint.artHistoryBrush", p).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(steps(&s), before + 1, "{source}: one state");
        // The dabs land within reach of the stroke, left of the grey/green edge at x 400.
        let opaque: Vec<[f32; 4]> =
            (60..340).flat_map(|x| (200..400).map(move |y| (x, y))).map(|(x, y)| pixel(&s, new, x, y)).filter(|p| p[3] > 0.99).collect();
        assert!(!opaque.is_empty(), "{source}: painted");
        assert!(opaque.iter().all(|p| (0..3).all(|i| (p[i] - rgb[i]).abs() < 0.01)), "{source}: every opaque pixel is {rgb:?}");
    }

    let Doc { mut s, new, .. } = issue_document(8);
    let mut p = stroke(&json!({"snapshot": 1}), json!([[150, 300], [200, 300]]));
    p["tolerance"] = json!(0);
    let before = steps(&s);
    assert!(s.execute("paint.artHistoryBrush", p).is_err(), "Setup");
    assert_eq!(steps(&s), before, "Setup: no state");
    assert!(pixel(&s, new, 175, 300)[3] < 0.01, "Setup: nothing painted");
}
