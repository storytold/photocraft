//! Artboard from Group on a group without bounds (#14558): the canvas when the document has no
//! artboard yet, else a board of the topmost artboard's size, `GAP` right of it at the same top.
//! A hidden group keeps the canvas (not measured, #14553).

use super::*;
use photocraft_geom::Rect;

/// 800 x 600 with the white Background, the artboards `boards` (`[x, y, w, h]`, bottom to top)
/// and an empty top-level group "Empty" above them. Returns the session and Empty's id.
fn session_with(boards: &[[i32; 4]]) -> (Session, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    for b in boards {
        s.execute("layer.new.artboard", json!({"rect": b})).unwrap();
    }
    let id = s.edit("Empty", |doc, active| Ok(*active.insert(doc.insert_above(None, Layer::group("Empty", vec![]))))).unwrap();
    (s, id)
}

/// [`session_with`] with no board, or with "Artboard 1" at 100,100 to 600,450.
fn session(board: bool) -> (Session, LayerId) {
    session_with(if board { &[[100, 100, 500, 350]] } else { &[] })
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn from_group(s: &mut Session, id: LayerId) -> Result<Value> {
    s.execute("layer.new.artboardFromGroup", json!({"layer": id.0}))
}

fn board_rect(s: &Session, id: LayerId) -> Rect {
    doc(s).layer(id).unwrap().artboard().unwrap().rect
}

#[test]
fn an_empty_group_goes_right_of_the_topmost_board() {
    let (mut s, id) = session(true);
    let r = from_group(&mut s, id).unwrap();
    assert_eq!(r["rect"], json!([700, 100, 500, 350]));
    assert_eq!(board_rect(&s, id), Rect::new(700, 100, 1200, 450));
    assert!(doc(&s).bounds().contains_rect(&Rect::new(700, 100, 1200, 450)), "the canvas grew to show it: {:?}", doc(&s).bounds());
}

#[test]
fn without_artboards_an_empty_group_covers_the_canvas() {
    let (mut s, id) = session(false);
    from_group(&mut s, id).unwrap();
    assert_eq!(board_rect(&s, id), Rect::new(0, 0, 800, 600));
    assert_eq!(doc(&s).bounds(), Rect::new(0, 0, 800, 600));
}

/// A hidden empty group keeps today's canvas rectangle even when a board exists: #14553 leaves a
/// hidden top-level group unmeasured.
#[test]
fn a_hidden_empty_group_keeps_the_canvas_beside_a_board() {
    let (mut s, id) = session(true);
    s.edit("Hide", |doc, _| {
        doc.layer_mut(id).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let canvas = doc(&s).bounds();
    from_group(&mut s, id).unwrap();
    assert_eq!(board_rect(&s, id), canvas);
}

/// A visible group whose children are all hidden counts as empty (#14553), so it takes the slot.
#[test]
fn a_visible_group_of_hidden_layers_goes_right_of_the_board() {
    let (mut s, id) = session(true);
    s.edit("Hidden red", |doc, _| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Hidden red", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 50), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        l.visible = false;
        doc.layer_mut(id).and_then(Layer::children_mut).unwrap().push(l);
        Ok(())
    })
    .unwrap();
    from_group(&mut s, id).unwrap();
    assert_eq!(board_rect(&s, id), Rect::new(700, 100, 1200, 450));
}

/// A group with content keeps its own bounds when boards exist.
#[test]
fn a_group_with_content_keeps_its_bounds_beside_boards() {
    let (mut s, id) = session(true);
    s.edit("Red", |doc, _| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Red", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(650, 50, 750, 150), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        doc.layer_mut(id).and_then(Layer::children_mut).unwrap().push(l);
        Ok(())
    })
    .unwrap();
    from_group(&mut s, id).unwrap();
    assert_eq!(board_rect(&s, id), Rect::new(650, 50, 750, 150));
}

/// With several boards (not measured) the slot follows the topmost board in the Layers panel,
/// not the rightmost one.
#[test]
fn the_slot_follows_the_topmost_board_not_the_rightmost() {
    // The topmost board is a small one left of the 100,100 to 600,450 board.
    let (mut s, id) = session_with(&[[100, 100, 500, 350], [0, 480, 60, 40]]);
    from_group(&mut s, id).unwrap();
    assert_eq!(board_rect(&s, id), Rect::new(60 + GAP, 480, 120 + GAP, 520));
}

/// #931: a topmost board at the far right edge leaves no room right of it; the command says so
/// and changes nothing, instead of wrapping.
#[test]
fn no_room_right_of_a_far_right_board_is_an_error() {
    let (mut s, id) = session(false);
    s.edit("Far", |doc, _| {
        let mut g = Layer::group("Far", vec![]);
        set_artboard(&mut g, Artboard::new(Rect::new(i32::MAX - 600, 0, i32::MAX - 50, 300)));
        doc.insert_above(None, g);
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    let e = from_group(&mut s, id).unwrap_err().to_string();
    assert!(e.contains("no room"), "{e}");
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc), "the document changed");
}
