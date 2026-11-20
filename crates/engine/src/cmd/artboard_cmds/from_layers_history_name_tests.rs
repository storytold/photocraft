//! Artboard from Layers adds one history step named "New Artboard From Layers" (#14578).

use super::*;
use photocraft_geom::Rect;

/// 100 x 80 RGB at `depth` with a pixel layer "Red" holding a square at 10,10 to 30,30.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80, "depth": depth, "background": "white"})).unwrap();
    s.edit("Red", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Red", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 30), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap();
    s
}

fn steps(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

fn tree(s: &Session) -> Vec<(String, bool)> {
    s.active().unwrap().doc.walk().iter().map(|(_, _, l)| (l.name.clone(), l.children().is_some())).collect()
}

#[test]
fn artboard_from_layers_adds_one_new_artboard_from_layers_step() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let before_steps = steps(&s);
        let before_tree = tree(&s);
        s.execute("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap();
        let after = steps(&s);
        assert_eq!(after.len(), before_steps.len() + 1, "exactly one step at {depth} bit: {after:?}");
        assert_eq!(after.last().map(String::as_str), Some("New Artboard From Layers"), "{depth} bit");
        s.undo();
        assert_eq!(tree(&s), before_tree, "one undo restores the layer tree at {depth} bit");
        assert!(s.active().unwrap().doc.artboards().is_empty());
    }
}
