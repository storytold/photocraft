//! #14572: Layer › New › Artboard from Layers puts the new board where the topmost selected
//! top-level layer was in the stack, as the reference application does, not at the top of the
//! document. One history step; one undo brings the original stack back.

use super::*;
use photocraft_geom::Rect;

/// The issue's document, 800 x 600 with the white Background and, bottom to top, "Red" at
/// 100,100 to 300,250, "Blue" at 400,300 to 600,450 and "Green" at 650,50 to 750,150.
fn session(depth: u32) -> (Session, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth, "background": "white"})).unwrap();
    let mut ids = [LayerId(0); 3];
    let layers = [
        ("Red", [100, 100, 300, 250], [1.0, 0.0, 0.0, 1.0]),
        ("Blue", [400, 300, 600, 450], [0.0, 0.0, 1.0, 1.0]),
        ("Green", [650, 50, 750, 150], [0.0, 1.0, 0.0, 1.0]),
    ];
    for (slot, (name, [x0, y0, x1, y1], rgba)) in ids.iter_mut().zip(layers) {
        *slot = s
            .edit(name, |doc, active| {
                let fmt = doc.pixel_format();
                let mut l = Layer::raster(name, fmt);
                l.surface_mut().unwrap().fill_rect(Rect { x0, y0, x1, y1 }, &photocraft_raster::from_rgba(&fmt, rgba));
                let id = doc.insert_above(None, l);
                *active = Some(id);
                Ok(id)
            })
            .unwrap();
    }
    (s, ids)
}

fn select(s: &mut Session, ids: &[LayerId]) {
    for (i, id) in ids.iter().enumerate() {
        s.execute("layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
}

/// The document's stack, top to bottom, with each board's children in braces (top to bottom).
fn stack(s: &Session) -> String {
    fn list(layers: &[Layer]) -> String {
        layers
            .iter()
            .rev()
            .map(|l| match l.children() {
                Some(kids) => format!("{} {{ {} }}", l.name, list(kids)),
                None => l.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
    list(&s.active().unwrap().doc.layers)
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.entries().len()
}

#[test]
fn the_board_takes_the_place_of_the_topmost_selected_layer() {
    for depth in [8, 16, 32] {
        // Red only.
        let (mut s, [red, _, _]) = session(depth);
        select(&mut s, &[red]);
        s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
        assert_eq!(stack(&s), "Green, Blue, Artboard 1 { Red }, Background", "{depth} bit, Red only");

        // Red + Blue.
        let (mut s, [red, blue, _]) = session(depth);
        select(&mut s, &[red, blue]);
        s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
        assert_eq!(stack(&s), "Green, Artboard 1 { Blue, Red }, Background", "{depth} bit, Red + Blue");

        // Red + Green, with the unselected Blue between them.
        let (mut s, [red, _, green]) = session(depth);
        select(&mut s, &[red, green]);
        s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
        assert_eq!(stack(&s), "Artboard 1 { Green, Red }, Blue, Background", "{depth} bit, Red + Green");
    }
}

#[test]
fn the_topmost_layer_still_gives_a_board_at_the_top() {
    let (mut s, [_, _, green]) = session(8);
    select(&mut s, &[green]);
    s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
    assert_eq!(stack(&s), "Artboard 1 { Green }, Blue, Red, Background");
}

#[test]
fn one_history_step_and_one_undo_restores_the_stack() {
    let (mut s, [red, blue, _]) = session(8);
    select(&mut s, &[red, blue]);
    let before = (stack(&s), steps(&s));
    s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
    assert_eq!(steps(&s), before.1 + 1, "one history step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(stack(&s), before.0, "one undo restores the original stack");
}

#[test]
fn the_new_board_is_the_active_layer() {
    let (mut s, [red, _, _]) = session(8);
    select(&mut s, &[red]);
    let r = s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
    let board = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().active_layer, Some(board));
    assert!(s.active().unwrap().doc.layer(board).is_some_and(|l| l.children().is_some()));
}
