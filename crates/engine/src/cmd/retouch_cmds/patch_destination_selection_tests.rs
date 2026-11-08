//! Patch in Destination mode moves a plain rectangular selection with the patch (#12119).

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = if (x / 8 + y / 8) % 2 == 0 { 0.2 } else { 0.8 };
                let px = if x >= 400 { [0.1, 0.6, 0.2, 1.0] } else { [v, v, v, 1.0] };
                data.extend(photocraft_raster::from_rgba(&fmt, px));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn bounds(s: &mut Session) -> Value {
    s.execute("document.inspect", json!({})).unwrap()["selectionBounds"].clone()
}

fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

#[test]
fn destination_moves_the_selection_with_the_patch_and_undo_restores_both() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 120, "y": 120, "width": 40, "height": 40})).unwrap();
    let before = px(&s, 600, 420);
    s.execute("paint.patch", json!({"mode": "destination", "offset": [460, 280]})).unwrap();
    assert_eq!(bounds(&mut s), json!([580, 400, 40, 40]));
    assert_ne!(px(&s, 600, 420), before, "pixels changed too");
    // One history state: a single undo brings back the pixels and the old selection.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
    assert_eq!(px(&s, 600, 420), before);
}

#[test]
fn source_and_content_aware_keep_the_selection() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 120, "y": 120, "width": 40, "height": 40})).unwrap();
    s.execute("paint.patch", json!({"offset": [260, 0]})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
    s.execute("paint.patch", json!({"offset": [260, 0], "contentAware": true})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
}
