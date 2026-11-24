//! Where File › Place Embedded puts the placed layer when a group is active (#14108), as measured
//! in the reference application on a document with an open group `Grp` (a normal and a locked
//! layer) over a gray `Paint` layer and a white Background: with `Grp` itself active the layer
//! goes directly above the group, at document level; with a layer inside `Grp` active it goes
//! inside, directly above that layer.

use super::*;

/// The measured document: Background, `Paint`, then the open group `Grp` holding `Normal` and
/// `Locked` (bottom to top). Returns the session and the ids of `Paint`, `Grp`, `Normal` and
/// `Locked`.
fn session() -> (Session, [LayerId; 4]) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 100, "height": 100, "background": "white"})).unwrap();
    let id = |v: Value| LayerId(v["layer"].as_u64().unwrap());
    let paint = id(s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap());
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    let group = id(s.execute("layer.new.group", json!({"name": "Grp"})).unwrap());
    let normal = id(s.execute("layer.new.layer", json!({"name": "Normal"})).unwrap());
    let locked = id(s.execute("layer.new.layer", json!({"name": "Locked"})).unwrap());
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    assert_eq!(children(&s, group), [normal, locked], "the test document");
    (s, [paint, group, normal, locked])
}

/// The ids at the document's top level, bottom to top.
fn top(s: &Session) -> Vec<LayerId> {
    s.active().unwrap().doc.layers.iter().map(|l| l.id).collect()
}

/// The ids inside `group`, bottom to top.
fn children(s: &Session, group: LayerId) -> Vec<LayerId> {
    s.active().unwrap().doc.layer(group).unwrap().children().unwrap().iter().map(|l| l.id).collect()
}

/// Makes `layer` the active one, then places a small PNG; the placed layer.
fn place_with_active(s: &mut Session, layer: LayerId) -> LayerId {
    s.execute("layer.select", json!({"layer": layer.0})).unwrap();
    let img = photocraft_codecs::Image::from_u8(10, 6, photocraft_codecs::ChannelLayout::Rgb, vec![200; 10 * 6 * 3]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let placed = LayerId(place_bytes(s, "src.png", bytes, None, &json!({})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().active_layer, Some(placed), "the placed layer is active");
    placed
}

/// The group itself active: directly above the group, at document level, not inside it.
#[test]
fn with_an_open_group_active_the_layer_goes_above_the_group() {
    let (mut s, [paint, group, normal, locked]) = session();
    let placed = place_with_active(&mut s, group);
    assert_eq!(top(&s)[1..], [paint, group, placed]);
    assert_eq!(children(&s, group), [normal, locked], "nothing goes inside the group");
}

/// A group nested in another: above it, inside the outer group (the group's parent).
#[test]
fn a_nested_open_group_takes_the_layer_above_it_in_its_parent() {
    let (mut s, [paint, outer, normal, locked]) = session();
    s.execute("layer.select", json!({"layer": locked.0})).unwrap();
    let inner = LayerId(s.execute("layer.new.group", json!({"name": "Inner"})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(children(&s, outer), [normal, locked, inner]);
    let placed = place_with_active(&mut s, inner);
    assert_eq!(children(&s, outer), [normal, locked, inner, placed]);
    assert_eq!(children(&s, inner), [], "nothing goes inside the inner group");
    assert_eq!(top(&s)[1..], [paint, outer]);
}

/// A layer inside the group active, normal or locked: inside the group, directly above it.
#[test]
fn with_a_layer_in_the_group_active_the_layer_goes_above_it_inside_the_group() {
    let (mut s, [paint, group, normal, locked]) = session();
    let placed = place_with_active(&mut s, normal);
    assert_eq!(children(&s, group), [normal, placed, locked]);
    assert_eq!(top(&s)[1..], [paint, group]);

    let (mut s, [paint, group, normal, locked]) = session();
    let placed = place_with_active(&mut s, locked);
    assert_eq!(children(&s, group), [normal, locked, placed]);
    assert_eq!(top(&s)[1..], [paint, group]);
}

/// A plain layer at document level active: directly above it, as before.
#[test]
fn with_a_plain_layer_active_the_layer_goes_above_it() {
    let (mut s, [paint, group, ..]) = session();
    let placed = place_with_active(&mut s, paint);
    assert_eq!(top(&s)[1..], [paint, placed, group]);
}

/// A closed group (not measured) keeps today's rule: above it.
#[test]
fn a_closed_group_keeps_todays_place() {
    let (mut s, [paint, group, normal, locked]) = session();
    s.execute("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
    let placed = place_with_active(&mut s, group);
    assert_eq!(top(&s)[1..], [paint, group, placed]);
    assert_eq!(children(&s, group), [normal, locked]);
}

/// Only Place changes: Paste with the open group active still puts the pasted layer inside it,
/// as its top layer.
#[test]
fn paste_into_an_open_group_is_unchanged() {
    let (mut s, [paint, group, normal, locked]) = session();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.select", json!({"layer": group.0})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let pasted = s.active().unwrap().active_layer.unwrap();
    assert_eq!(children(&s, group), [normal, locked, pasted]);
}

/// In a document with an artboard (not measured), an open group on the board keeps today's
/// rule: content placed on the board goes inside the group.
#[test]
fn an_artboard_document_keeps_todays_place() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = |v: Value| LayerId(v["layer"].as_u64().unwrap());
    let board = id(s.execute("layer.new.artboard", json!({"name": "Board", "rect": [0, 0, 100, 100]})).unwrap());
    s.execute("layer.select", json!({"layer": board.0})).unwrap();
    let group = id(s.execute("layer.new.group", json!({"name": "Grp"})).unwrap());
    assert_eq!(children(&s, board), [group], "the group is on the board");
    let placed = place_with_active(&mut s, group);
    assert_eq!(children(&s, group), [placed]);
}
