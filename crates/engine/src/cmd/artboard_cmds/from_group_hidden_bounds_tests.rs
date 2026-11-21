//! Artboard from Group sizes the board to the group's visible layers only (#14553); hidden layers
//! stay inside it, in place and hidden.

use super::*;
use photocraft_geom::Rect;

const RED: Rect = Rect { x0: 100, y0: 100, x1: 300, y1: 250 };
const BLUE: Rect = Rect { x0: 400, y0: 300, x1: 600, y1: 450 };

fn square(doc: &Document, name: &str, r: Rect, rgba: [f32; 4]) -> Layer {
    let fmt = doc.pixel_format();
    let mut l = Layer::raster(name, fmt);
    l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, rgba));
    l
}

/// 800 x 600 with the white Background and "Group 1" holding Blue above Red; with `inner`, Blue
/// sits in its own group "Inner". Returns the session and the ids of Group 1, Red and Blue.
fn session(inner: bool) -> (Session, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let ids = s
        .edit("Group 1", |doc, active| {
            let red = square(doc, "Red", RED, [1.0, 0.0, 0.0, 1.0]);
            let blue = square(doc, "Blue", BLUE, [0.0, 0.0, 1.0, 1.0]);
            let ids = (red.id, blue.id);
            let top = if inner { Layer::group("Inner", vec![blue]) } else { blue };
            let g = doc.insert_above(None, Layer::group("Group 1", vec![red, top]));
            *active = Some(g);
            Ok([g, ids.0, ids.1])
        })
        .unwrap();
    (s, ids)
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn hide(s: &mut Session, id: LayerId) {
    s.execute("layer.hideLayers", json!({"layer": id.0})).unwrap();
}

fn from_group(s: &mut Session, g: LayerId) -> Value {
    let r = s.execute("layer.new.artboardFromGroup", json!({"layer": g.0})).unwrap();
    assert_eq!(r["rect"], json!(board_xywh(s, g)), "the reply matches the board");
    r["rect"].clone()
}

fn board_xywh(s: &Session, g: LayerId) -> [i64; 4] {
    xywh(doc(s).layer(g).unwrap().artboard().unwrap().rect)
}

fn names(layers: &[Layer]) -> Vec<&str> {
    layers.iter().map(|l| l.name.as_str()).collect()
}

#[test]
fn both_visible_the_board_covers_both_layers() {
    let (mut s, [g, ..]) = session(false);
    assert_eq!(from_group(&mut s, g), json!([100, 100, 500, 350]));
}

#[test]
fn a_hidden_layer_does_not_size_the_board_and_stays_inside_it_hidden() {
    let (mut s, [g, red, blue]) = session(false);
    hide(&mut s, blue);
    assert_eq!(from_group(&mut s, g), json!([100, 100, 200, 150]));
    let d = doc(&s);
    let board = d.layer(g).unwrap();
    assert_eq!(names(board.children().unwrap()), ["Red", "Blue"], "Blue keeps its place above Red");
    assert_eq!(d.artboard_of(blue), Some(g));
    let b = d.layer(blue).unwrap();
    assert!(!b.visible, "Blue stays hidden");
    assert_eq!(b.surface().unwrap().content_bounds(), BLUE, "Blue's pixels don't move");
    assert!(d.layer(red).unwrap().visible);
}

#[test]
fn a_hidden_group_inside_hides_its_children_too() {
    let (mut s, [g, _, blue]) = session(true);
    let inner = doc(&s).layer(g).unwrap().children().unwrap()[1].id;
    hide(&mut s, inner);
    assert_eq!(from_group(&mut s, g), json!([100, 100, 200, 150]));
    let d = doc(&s);
    assert!(!d.layer(inner).unwrap().visible && d.layer(blue).unwrap().visible);
    assert_eq!(d.artboard_of(blue), Some(g));

    // A visible group whose only layer is hidden adds nothing either.
    let (mut s, [g, _, blue]) = session(true);
    hide(&mut s, blue);
    assert_eq!(from_group(&mut s, g), json!([100, 100, 200, 150]));
}

#[test]
fn a_group_with_every_layer_hidden_is_handled_like_an_empty_group() {
    let mut empty = Session::new();
    empty.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let e = empty.edit("Group 1", |doc, _| Ok(doc.insert_above(None, Layer::group("Group 1", vec![])))).unwrap();
    let today = from_group(&mut empty, e);
    assert_eq!(today, json!([0, 0, 800, 600]), "an empty group gets the canvas");

    let (mut s, [g, red, blue]) = session(false);
    hide(&mut s, red);
    hide(&mut s, blue);
    assert_eq!(from_group(&mut s, g), today);
    assert_eq!(names(doc(&s).layer(g).unwrap().children().unwrap()), ["Red", "Blue"]);
}

/// Pins today: layer effects don't widen the board (not measured in the reference, #14553).
#[test]
fn layer_effects_keep_todays_rect() {
    let (mut s, [g, red, _]) = session(false);
    s.edit("Shadow", |doc, _| {
        for id in [red, g] {
            doc.layer_mut(id).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(from_group(&mut s, g), json!([100, 100, 500, 350]));
}

/// Pins today: a hidden group as the active layer is measured as before, hidden layers included
/// (not measured in the reference, #14553).
#[test]
fn a_hidden_active_group_keeps_todays_rect() {
    let (mut s, [g, _, blue]) = session(false);
    hide(&mut s, blue);
    hide(&mut s, g);
    assert_eq!(from_group(&mut s, g), json!([100, 100, 500, 350]));
}
