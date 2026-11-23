//! Artboard from Group's selection rules (#14560): greyed with more than one group selected; run
//! directly on a layer that is not a plain group (or on an existing artboard) it does nothing,
//! without a history step or an error; with a group and other layers selected it makes one board
//! from the active group at the union of every selected layer's visible bounds and moves the
//! other selected layers into it.

use super::*;
use photocraft_geom::Rect;

const CMD: &str = "layer.new.artboardFromGroup";
const RED: Rect = Rect { x0: 100, y0: 100, x1: 300, y1: 250 };
const BLUE: Rect = Rect { x0: 400, y0: 300, x1: 600, y1: 450 };
const GREEN: Rect = Rect { x0: 650, y0: 50, x1: 750, y1: 150 };

fn square(doc: &Document, name: &str, r: Rect, rgba: [f32; 4]) -> Layer {
    let fmt = doc.pixel_format();
    let mut l = Layer::raster(name, fmt);
    l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, rgba));
    l
}

/// The issue's test document at `depth`: 800 x 600 with a white Background, "Group 1" holding
/// Blue above Red, and the pixel layer Green above the group, outside it.
struct Doc {
    s: Session,
    group: LayerId,
    red: LayerId,
    green: LayerId,
    background: LayerId,
}

fn document(depth: u32) -> Doc {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth, "background": "white"})).unwrap();
    let (group, red, green) = s
        .edit("Layers", |doc, active| {
            let red = square(doc, "Red", RED, [1.0, 0.0, 0.0, 1.0]);
            let blue = square(doc, "Blue", BLUE, [0.0, 0.0, 1.0, 1.0]);
            let green = square(doc, "Green", GREEN, [0.0, 1.0, 0.0, 1.0]);
            let red_id = red.id;
            let g = doc.insert_above(None, Layer::group("Group 1", vec![red, blue]));
            let green_id = doc.insert_above(None, green);
            *active = Some(g);
            Ok((g, red_id, green_id))
        })
        .unwrap();
    let background = s.active().unwrap().doc.background().unwrap().id;
    select(&mut s, &[group], group);
    Doc { s, group, red, green, background }
}

fn select(s: &mut Session, ids: &[LayerId], active: LayerId) {
    crate::layer_multi_cmds::set_selection(s, ids.to_vec(), Some(active), Some(active)).unwrap();
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.entries().len()
}

fn board(s: &Session, id: LayerId) -> Option<[i64; 4]> {
    s.active().unwrap().doc.layer(id).and_then(Layer::artboard).map(|a| xywh(a.rect))
}

fn children(s: &Session, id: LayerId) -> Vec<String> {
    s.active().unwrap().doc.layer(id).and_then(Layer::children).unwrap().iter().map(|l| l.name.clone()).collect()
}

#[test]
fn two_groups_selected_grey_the_command_but_not_artboard_from_layers() {
    let mut d = document(8);
    let g2 = d.s.edit("G2", |doc, _| Ok(doc.insert_above(None, Layer::group("G2", vec![])))).unwrap();
    select(&mut d.s, &[d.group, g2], g2);
    assert!(!d.s.is_enabled(CMD), "{:?}", d.s.disabled_reason(CMD));
    assert!(d.s.is_enabled("layer.new.artboardFromLayers"));
    // One group again: enabled.
    select(&mut d.s, &[g2], g2);
    assert!(d.s.is_enabled(CMD), "{:?}", d.s.disabled_reason(CMD));
}

#[test]
fn run_directly_on_a_layer_that_is_not_a_group_does_nothing_without_an_error() {
    for depth in [8, 16, 32] {
        let mut d = document(depth);
        for (what, layer) in [("pixel layer", d.green), ("child layer", d.red), ("Background", d.background)] {
            select(&mut d.s, &[layer], layer);
            assert!(!d.s.is_enabled(CMD), "{what} at {depth} bit: the menu entry stays greyed");
            let before = steps(&d.s);
            let doc = d.s.active().unwrap().doc.clone();
            d.s.execute(CMD, json!({})).unwrap_or_else(|e| panic!("{what} at {depth} bit: {e}"));
            assert_eq!(steps(&d.s), before, "{what} at {depth} bit: no history step");
            assert!(std::sync::Arc::ptr_eq(&doc, &d.s.active().unwrap().doc), "{what} at {depth} bit: the document is unchanged");
        }
    }
}

#[test]
fn run_directly_on_an_existing_artboard_does_nothing_without_an_error() {
    let mut d = document(8);
    d.s.execute(CMD, json!({})).unwrap();
    let rect = board(&d.s, d.group);
    assert!(!d.s.is_enabled(CMD), "the menu entry is greyed on an artboard");
    let before = steps(&d.s);
    d.s.execute(CMD, json!({})).unwrap();
    assert_eq!(steps(&d.s), before, "no history step");
    assert_eq!(board(&d.s, d.group), rect, "the board is unchanged");
    assert_eq!(d.s.active().unwrap().doc.artboards().len(), 1);
}

#[test]
fn a_group_and_a_pixel_layer_make_one_board_at_their_union_with_the_layer_inside() {
    for depth in [8, 16, 32] {
        let mut d = document(depth);
        select(&mut d.s, &[d.group, d.green], d.group);
        assert!(d.s.is_enabled(CMD), "{depth} bit: {:?}", d.s.disabled_reason(CMD));
        let before = steps(&d.s);
        let out = d.s.execute(CMD, json!({})).unwrap();
        assert_eq!(steps(&d.s), before + 1, "{depth} bit: one history step");
        let doc = &d.s.active().unwrap().doc;
        assert_eq!(doc.artboards().len(), 1, "{depth} bit: one board");
        assert_eq!(board(&d.s, d.group), Some([100, 50, 650, 400]), "{depth} bit: 100,50 to 750,450");
        assert_eq!(out["rect"], json!([100, 50, 650, 400]));
        assert_eq!(children(&d.s, d.group), ["Red", "Blue", "Green"], "{depth} bit: Green moved in, above the group's layers");
        assert!(doc.layers.iter().all(|l| l.id != d.green), "{depth} bit: Green is no longer top-level");
        d.s.undo();
        let doc = &d.s.active().unwrap().doc;
        assert!(doc.artboards().is_empty() && doc.layers.iter().any(|l| l.id == d.green), "{depth} bit: one undo restores it");
    }
}

#[test]
fn a_hidden_selected_layer_does_not_widen_the_board() {
    let mut d = document(8);
    d.s.execute("layer.hideLayers", json!({"layer": d.green.0})).unwrap();
    select(&mut d.s, &[d.group, d.green], d.group);
    d.s.execute(CMD, json!({})).unwrap();
    assert_eq!(board(&d.s, d.group), Some([100, 100, 500, 350]));
    assert_eq!(children(&d.s, d.group), ["Red", "Blue", "Green"], "the hidden layer still moves in");
}

#[test]
fn a_single_group_still_gives_100_100_to_600_450() {
    for depth in [8, 16, 32] {
        let mut d = document(depth);
        d.s.execute(CMD, json!({})).unwrap();
        assert_eq!(board(&d.s, d.group), Some([100, 100, 500, 350]), "{depth} bit");
        assert_eq!(children(&d.s, d.group), ["Red", "Blue"], "{depth} bit");
    }
}

#[test]
fn selected_layers_on_both_sides_keep_their_side_of_the_group() {
    let mut d = document(8);
    // Root, bottom to top: Background, A, A2, Group 1 (Red, Blue), Green.
    let (a, a2) =
        d.s.edit("Below", |doc, _| {
            let a = square(doc, "A", RED, [1.0, 1.0, 0.0, 1.0]);
            let a2 = square(doc, "A2", BLUE, [0.0, 1.0, 1.0, 1.0]);
            let ids = (a.id, a2.id);
            doc.layers.insert(1, a2);
            doc.layers.insert(1, a);
            Ok(ids)
        })
        .unwrap();
    select(&mut d.s, &[a, a2, d.group, d.green], d.group);
    d.s.execute(CMD, json!({})).unwrap();
    assert_eq!(children(&d.s, d.group), ["A", "A2", "Red", "Blue", "Green"], "Green stays above the group's layers");
}

#[test]
fn run_directly_on_a_pixel_layer_leaves_a_floating_selection_floating() {
    let mut d = document(8);
    select(&mut d.s, &[d.green], d.green);
    d.s.execute("select.rect", json!({"x": 650, "y": 50, "width": 100, "height": 100})).unwrap();
    d.s.execute("select.float", json!({"dx": 10, "dy": 0})).unwrap();
    let before = steps(&d.s);
    d.s.execute(CMD, json!({})).unwrap();
    assert!(crate::float_cmds::floating(d.s.active().unwrap()).is_some(), "the piece still floats");
    assert_eq!(steps(&d.s), before, "no history step");
    // Run on the group by id, it drops the piece first (its own step) and makes the board.
    d.s.execute(CMD, json!({"layer": d.group.0})).unwrap();
    assert!(crate::float_cmds::floating(d.s.active().unwrap()).is_none(), "the piece dropped");
    assert_eq!(steps(&d.s), before + 2, "the drop and the board");
    assert!(board(&d.s, d.group).is_some());
}

#[test]
fn run_directly_on_a_layer_that_is_not_there_is_an_error() {
    for depth in [8, 16, 32] {
        let mut d = document(depth);
        let before = steps(&d.s);
        let doc = d.s.active().unwrap().doc.clone();
        let err = d.s.execute(CMD, json!({"layer": 999_999})).expect_err("an unknown layer id is an error");
        assert!(err.to_string().contains("no such layer"), "{depth} bit: {err}");
        assert_eq!(steps(&d.s), before, "{depth} bit: no history step");
        assert!(std::sync::Arc::ptr_eq(&doc, &d.s.active().unwrap().doc), "{depth} bit: the document is unchanged");
    }
}

#[test]
fn an_unknown_layer_is_an_error_that_leaves_a_floating_selection_floating() {
    let mut d = document(8);
    select(&mut d.s, &[d.green], d.green);
    d.s.execute("select.rect", json!({"x": 650, "y": 50, "width": 100, "height": 100})).unwrap();
    d.s.execute("select.float", json!({"dx": 10, "dy": 0})).unwrap();
    let before = steps(&d.s);
    for layer in [json!(999_999), json!("No Such Layer")] {
        assert!(d.s.execute(CMD, json!({"layer": layer})).is_err(), "{layer}: an error");
        assert!(crate::float_cmds::floating(d.s.active().unwrap()).is_some(), "{layer}: the piece still floats");
        assert_eq!(steps(&d.s), before, "{layer}: no history step");
    }
}
