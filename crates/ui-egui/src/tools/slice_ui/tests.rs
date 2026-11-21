use craft::behavior::kurbo::Rect as DocRect;
use craft::behavior::{EditEvent, GestureId, Mixed};

use super::*;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app
}

/// Slice Options on an auto slice: OK promotes it and sets the fields the dialog holds.
#[test]
fn slice_options_ok_promotes_an_auto_slice_and_sets_its_fields() {
    let mut app = app();
    // An empty document is one auto slice, number 1.
    app.ui.slices.selected = Some(1);
    let id = open_options(&mut app).unwrap();
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "name", json!("footer")).unwrap();
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    assert!(doc.slices.list.iter().any(|s| s.name == "footer"), "{:?}", doc.slices.list);
    assert_eq!(app.ui.slices.options, None);
}

/// #7770: the slice under a later one is drawn as pieces; a Move commit on one piece moves the whole
/// slice by the piece's edge deltas.
#[test]
fn dragging_a_piece_moves_the_whole_covered_slice() {
    let mut app = app();
    app.run("slice.new", json!({"rect": [0, 0, 60, 60]})).unwrap();
    app.run("slice.new", json!({"rect": [40, 40, 20, 20]})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let first = doc.slices.list[0].id;
    let pieces: Vec<_> = slices::resolve(&doc).into_iter().filter(|r| r.id == Some(first)).collect();
    assert!(pieces.len() > 1, "the first slice is covered into pieces");
    let piece = &pieces[1];
    let before = to_doc_rect(piece.rect);
    let after = DocRect::new(before.x0 + 5.0, before.y0, before.x1 + 5.0, before.y1);
    apply(&mut app, SliceEvent::moved(piece.number, EditEvent::commit(GestureId::new(1), Mixed::Same(before), after)));
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(doc.slices.list[0].rect, photocraft_geom::Rect::new(5, 0, 65, 60));
    assert_eq!(app.ui.slices.preview, None);
}
