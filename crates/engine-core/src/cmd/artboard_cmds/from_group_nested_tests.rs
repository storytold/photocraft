//! Artboard from Group on a group nested in a plain group or an artboard (#14559): the group moves
//! to the top level directly above its top-level ancestor, keeps its children and becomes the
//! board, in one history step.

use super::*;
use photocraft_geom::Rect;

const RED: Rect = Rect { x0: 100, y0: 100, x1: 300, y1: 250 };
const BLUE: Rect = Rect { x0: 400, y0: 300, x1: 600, y1: 450 };
const GREEN: Rect = Rect { x0: 650, y0: 50, x1: 750, y1: 150 };

fn square(doc: &Document, name: &str, r: Rect, rgba: [f32; 4]) -> Layer {
    let fmt = doc.pixel_format();
    let mut l = Layer::raster(name, fmt);
    l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, rgba));
    l
}

/// 800 x 600 with the white Background, "Group 1" holding Red and, nested `depth` groups deep,
/// Blue in "Inner", and Green at the top level above Group 1. Returns the session and the ids of
/// Group 1 and Inner, with Inner active.
fn session(depth: usize) -> (Session, LayerId, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let ids = s
        .edit("Groups", |doc, active| {
            let red = square(doc, "Red", RED, [1.0, 0.0, 0.0, 1.0]);
            let inner = Layer::group("Inner", vec![square(doc, "Blue", BLUE, [0.0, 0.0, 1.0, 1.0])]);
            let inner_id = inner.id;
            let nested = (1..depth).fold(inner, |l, n| Layer::group(format!("Mid {n}"), vec![l]));
            let g = doc.insert_above(None, Layer::group("Group 1", vec![red, nested]));
            let green = square(doc, "Green", GREEN, [0.0, 1.0, 0.0, 1.0]);
            doc.insert_above(None, green);
            *active = Some(inner_id);
            Ok((g, inner_id))
        })
        .unwrap();
    (s, ids.0, ids.1)
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// The top-level layers bottom to top, each with its children's names; artboards are marked.
fn tree(s: &Session) -> Vec<String> {
    doc(s)
        .layers
        .iter()
        .map(|l| {
            let mark = if l.artboard().is_some() { " [artboard]" } else { "" };
            let children: Vec<&str> = l.children().unwrap_or_default().iter().map(|c| c.name.as_str()).collect();
            if children.is_empty() { format!("{}{mark}", l.name) } else { format!("{}{mark}: {}", l.name, children.join(", ")) }
        })
        .collect()
}

fn from_group(s: &mut Session, g: LayerId) -> Result<Value> {
    s.execute("layer.new.artboardFromGroup", json!({"layer": g.0}))
}

#[test]
fn a_group_in_a_plain_group_moves_out_above_it_as_a_board() {
    let (mut s, _, inner) = session(1);
    assert_eq!(has_plain_group(&s), Ok(()), "the menu item is enabled");
    let r = from_group(&mut s, inner).unwrap();
    assert_eq!(r["rect"], json!(xywh(BLUE)), "the board is the group's visible bounds");
    assert_eq!(tree(&s), ["Background", "Group 1: Red", "Inner [artboard]: Blue", "Green"]);
    assert_eq!(s.active().unwrap().active_layer, Some(inner), "the new board is active");
}

#[test]
fn a_group_in_an_artboard_moves_out_above_it_and_the_artboard_keeps_the_rest() {
    let (mut s, g, inner) = session(1);
    from_group(&mut s, g).unwrap();
    s.execute("layer.select", json!({"layer": inner.0})).unwrap();
    assert_eq!(has_plain_group(&s), Ok(()), "the menu item is enabled inside an artboard");
    let r = from_group(&mut s, inner).unwrap();
    assert_eq!(r["rect"], json!(xywh(BLUE)));
    assert_eq!(tree(&s), ["Background", "Group 1 [artboard]: Red", "Inner [artboard]: Blue", "Green"]);
}

#[test]
fn a_deeply_nested_group_moves_above_its_top_level_ancestor() {
    let (mut s, _, inner) = session(3);
    assert_eq!(has_plain_group(&s), Ok(()));
    from_group(&mut s, inner).unwrap();
    assert_eq!(tree(&s), ["Background", "Group 1: Red, Mid 2", "Inner [artboard]: Blue", "Green"]);
    let mid2 = doc(&s).layers[1].children().unwrap()[1].children().unwrap();
    assert_eq!(mid2.len(), 1, "Mid 1 stays in Mid 2, now empty");
    assert!(mid2[0].children().unwrap().is_empty());
}

#[test]
fn one_undo_puts_the_group_back_where_it_was() {
    let (mut s, _, inner) = session(2);
    let (before, past) = (doc(&s).clone(), s.active().unwrap().history.past_len());
    from_group(&mut s, inner).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    assert!(s.undo());
    assert_eq!(doc(&s).layers, before.layers, "the tree is restored");
}

#[test]
fn a_hidden_or_fully_locked_nested_group_stays_refused() {
    // Not measured in the reference, so these keep today's refusal.
    let (mut s, g, inner) = session(1);
    s.execute("layer.hideLayers", json!({"layer": g.0})).unwrap();
    s.execute("layer.select", json!({"layer": inner.0})).unwrap();
    let before = tree(&s);
    assert!(has_plain_group(&s).is_err(), "inside a hidden group");
    assert!(from_group(&mut s, inner).is_err());
    assert_eq!(tree(&s), before);

    let (mut s, g, inner) = session(1);
    s.edit("Lock", |doc, _| {
        doc.layer_mut(g).unwrap().locks.all = true;
        Ok(())
    })
    .unwrap();
    assert!(has_plain_group(&s).is_err(), "inside a fully locked group");
    assert!(from_group(&mut s, inner).is_err());
    assert_eq!(doc(&s).layer(inner).unwrap().artboard(), None);
}

const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// 800 x 600 with the white Background and "Group 1" holding Red and "Inner" (holding Blue), with
/// a yellow `over` layer clipped either inside Group 1 above Inner (`inside`) or above Group 1 at
/// the top level. Returns the session and the ids of Inner and the yellow layer.
fn clipped_session(over: Rect, inside: bool) -> (Session, LayerId, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let ids = s
        .edit("Groups", |doc, active| {
            let red = square(doc, "Red", RED, [1.0, 0.0, 0.0, 1.0]);
            let inner = Layer::group("Inner", vec![square(doc, "Blue", BLUE, [0.0, 0.0, 1.0, 1.0])]);
            let mut yellow = square(doc, "Over", over, YELLOW);
            yellow.clipped = true;
            let (inner_id, yellow_id) = (inner.id, yellow.id);
            if inside {
                doc.insert_above(None, Layer::group("Group 1", vec![red, inner, yellow]));
            } else {
                doc.insert_above(None, Layer::group("Group 1", vec![red, inner]));
                doc.insert_above(None, yellow);
            }
            *active = Some(inner_id);
            Ok((inner_id, yellow_id))
        })
        .unwrap();
    (s, ids.0, ids.1)
}

fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
    photocraft_compose::flatten(doc(s)).get(x, y)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn layers_clipped_to_the_group_stay_behind_released_as_a_move_leaves_them() {
    // Over covers Red and the corner beside it, and clips to Inner, which it doesn't overlap.
    let over = Rect { x0: 50, y0: 50, x1: 350, y1: 300 };
    let beside = (75, 75);
    let (mut s, inner, yellow) = clipped_session(over, true);
    assert!(close(px(&s, beside.0, beside.1), [1.0; 4]), "clipped to Inner, Over draws nothing there");
    from_group(&mut s, inner).unwrap();
    assert_eq!(tree(&s), ["Background", "Group 1: Red, Over", "Inner [artboard]: Blue"]);
    let after = (doc(&s).layer(yellow).unwrap().clipped, px(&s, beside.0, beside.1));

    // The same layer moved out with Move to gets the same treatment.
    let (mut moved, inner, moved_yellow) = clipped_session(over, true);
    let group = doc(&moved).layers[1].id;
    moved.execute("layer.moveTo", json!({"layer": inner.0, "target": group.0, "position": "above"})).unwrap();
    let want = (doc(&moved).layer(moved_yellow).unwrap().clipped, px(&moved, beside.0, beside.1));
    assert_eq!(after.0, want.0, "Over is released, as a move leaves it, not clipped to Red");
    assert!(!after.0);
    assert!(close(after.1, want.1) && close(after.1, YELLOW), "released, Over draws in full beside Red: {:?}", after.1);
}

#[test]
fn the_board_goes_above_the_layers_clipped_to_its_ancestor() {
    // Over covers Red and Blue and clips to Group 1 at the top level.
    let over = Rect { x0: 50, y0: 50, x1: 650, y1: 500 };
    let red_middle = (200, 175);
    let (mut s, inner, yellow) = clipped_session(over, false);
    let before = px(&s, red_middle.0, red_middle.1);
    assert!(close(before, YELLOW), "Over draws on Group 1's Red");
    from_group(&mut s, inner).unwrap();
    assert_eq!(tree(&s), ["Background", "Group 1: Red", "Over", "Inner [artboard]: Blue"]);
    assert!(doc(&s).layer(yellow).unwrap().clipped, "Over still clips to Group 1");
    assert!(!doc(&s).layer(inner).unwrap().clipped, "the board is not clipped");
    let after = px(&s, red_middle.0, red_middle.1);
    assert!(close(after, before), "outside the board the picture is unchanged: {before:?} -> {after:?}");
}
