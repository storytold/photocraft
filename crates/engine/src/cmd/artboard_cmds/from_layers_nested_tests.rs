//! #14573: Artboard from Layers works when selected layers sit inside an artboard. They move out
//! into the new board, the old board keeps its other children and its rect, and the command is one
//! History step. It is greyed only when an artboard itself is selected (and without a document).
//!
//! The document is 800 × 600: a white Background, pixel layers Plum 100,100 to 300,250, Teal
//! 400,300 to 600,450 above it and Gold 650,50 to 750,150 on top. Plum is first made into
//! "Artboard 1", moved down above the Background where Plum was, and a layer "Keep" is added
//! inside that board.

use super::*;
use photocraft_geom::Rect;

const FROM_LAYERS: &str = "layer.new.artboardFromLayers";

fn ids(s: &Session, name: &str) -> LayerId {
    s.active().unwrap().doc.walk().iter().map(|(_, _, l)| l).find(|l| l.name == name).unwrap_or_else(|| panic!("{name}")).id
}

fn select(s: &mut Session, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        let id = ids(s, name);
        s.execute("layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    for (name, [x0, y0, x1, y1]) in [("Plum", [100, 100, 300, 250]), ("Teal", [400, 300, 600, 450]), ("Gold", [650, 50, 750, 150])] {
        s.edit(name, |doc, active| {
            let fmt = doc.pixel_format();
            let mut l = Layer::raster(name, fmt);
            l.surface_mut().unwrap().fill_rect(Rect::new(x0, y0, x1, y1), &photocraft_raster::from_rgba(&fmt, [0.4, 0.2, 0.6, 1.0]));
            *active = Some(doc.insert_above(None, l));
            Ok(())
        })
        .unwrap();
    }
    select(&mut s, &["Plum"]);
    s.execute(FROM_LAYERS, json!({"name": "Artboard 1"})).unwrap();
    // As in the reference document, the board sits where Plum was, above the Background.
    select(&mut s, &["Artboard 1"]);
    s.execute("layer.arrange.sendToBack", json!({})).unwrap();
    select(&mut s, &["Plum"]);
    s.execute("layer.new.layer", json!({"name": "Keep"})).unwrap();
    s
}

/// The top-level layers, top to bottom, with their children (top to bottom) and a board's rect.
fn tree(s: &Session) -> Vec<(String, Vec<String>, Option<[i32; 4]>)> {
    let doc = &s.active().unwrap().doc;
    doc.layers
        .iter()
        .rev()
        .map(|l| {
            let kids = l.children().map(|c| c.iter().rev().map(|k| k.name.clone()).collect()).unwrap_or_default();
            let rect = l.artboard().map(|a| [a.rect.x0, a.rect.y0, a.rect.x1, a.rect.y1]);
            (l.name.clone(), kids, rect)
        })
        .collect()
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn it_is_available_inside_an_artboard_and_greyed_on_an_artboard_itself() {
    let mut s = session();
    select(&mut s, &["Plum"]);
    assert!(s.is_enabled(FROM_LAYERS), "a layer inside an artboard");
    select(&mut s, &["Artboard 1"]);
    assert!(!s.is_enabled(FROM_LAYERS), "the artboard itself");
    assert!(!Session::new().is_enabled(FROM_LAYERS), "no document");
}

#[test]
fn a_nested_and_a_top_level_layer_move_into_a_board_on_top() {
    let mut s = session();
    let before = tree(&s);
    let steps = s.active().unwrap().history.past_len();
    select(&mut s, &["Plum", "Gold"]);
    s.execute(FROM_LAYERS, json!({"name": "Artboard 2"})).unwrap();
    assert_eq!(
        tree(&s),
        vec![
            ("Artboard 2".to_string(), names(&["Gold", "Plum"]), Some([100, 50, 750, 250])),
            ("Teal".to_string(), vec![], None),
            ("Artboard 1".to_string(), names(&["Keep"]), Some([100, 100, 300, 250])),
            ("Background".to_string(), vec![], None),
        ]
    );
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one History step");
    s.undo();
    assert_eq!(tree(&s), before, "one undo restores both boards");
}

#[test]
fn a_nested_layer_alone_gets_a_board_of_its_own_above_the_old_one() {
    let mut s = session();
    select(&mut s, &["Plum"]);
    s.execute(FROM_LAYERS, json!({"name": "Artboard 2"})).unwrap();
    assert_eq!(
        tree(&s),
        vec![
            ("Gold".to_string(), vec![], None),
            ("Teal".to_string(), vec![], None),
            ("Artboard 2".to_string(), names(&["Plum"]), Some([100, 100, 300, 250])),
            ("Artboard 1".to_string(), names(&["Keep"]), Some([100, 100, 300, 250])),
            ("Background".to_string(), vec![], None),
        ]
    );
}

#[test]
fn layers_in_two_different_artboards_are_still_refused() {
    let mut s = session();
    select(&mut s, &["Teal"]);
    s.execute(FROM_LAYERS, json!({"name": "Artboard 3"})).unwrap();
    select(&mut s, &["Plum", "Teal"]);
    assert!(s.execute(FROM_LAYERS, json!({})).is_err());
}

#[test]
fn top_level_layers_keep_todays_board_on_top() {
    let mut s = session();
    select(&mut s, &["Teal"]);
    s.execute(FROM_LAYERS, json!({"name": "Artboard 2"})).unwrap();
    assert_eq!(tree(&s)[0].0, "Artboard 2");
}
