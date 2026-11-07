//! #11779: with Contiguous on, the Magic Wand's Subtract and Intersect grow the region from the
//! seed through the current selection only. A seed outside the selection subtracts nothing and
//! intersects to nothing.

use serde_json::json;

use crate::Session;

/// 800 × 600, white, with a grey ramp of ten 40 px blocks from x 100 (100, 105, …, 145).
fn ramp() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    for k in 0..10 {
        let v = 100 + 5 * k;
        s.execute("select.rect", json!({"x": 100 + 40 * k, "y": 350, "width": 40, "height": 100})).unwrap();
        s.execute("edit.fill", json!({"color": format!("#{v:02x}{v:02x}{v:02x}")})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn wand(s: &mut Session, x: i32, tolerance: f64, mode: &str) {
    s.execute("select.magicWand", json!({"x": x, "y": 400, "tolerance": tolerance, "mode": mode})).unwrap();
}

/// The columns of row 400 that are at least half selected: (first, end), or `None`.
fn selected(s: &Session) -> Option<(i32, i32)> {
    let doc = &s.active().unwrap().doc;
    let row = photocraft_geom::Rect::new(0, 400, 800, 401);
    let m = photocraft_algo::selection::mask_from_surface(doc.selection.as_ref(), row);
    let first = m.iter().position(|v| *v >= 0.5)?;
    let last = m.iter().rposition(|v| *v >= 0.5)?;
    Some((first as i32, last as i32 + 1))
}

/// Greys 100 to 130 (x 100–380) selected, as tolerance 32 on the first block selects them.
fn ramp_selected() -> Session {
    let mut s = ramp();
    wand(&mut s, 110, 32.0, "replace");
    assert_eq!(selected(&s), Some((100, 380)));
    s
}

#[test]
fn a_seed_outside_the_selection_subtracts_nothing() {
    let mut s = ramp_selected();
    // Grey 145 (x 460–500) is outside the selection, though its own region reaches into it.
    wand(&mut s, 480, 32.0, "subtract");
    assert_eq!(selected(&s), Some((100, 380)), "unchanged");
}

#[test]
fn a_seed_outside_the_selection_intersects_to_nothing() {
    let mut s = ramp_selected();
    wand(&mut s, 480, 32.0, "intersect");
    assert_eq!(selected(&s), None);
}

#[test]
fn a_seed_inside_the_selection_works_as_before() {
    // Grey 125 with tolerance 10 reaches greys 115–135; inside the selection that is x 220–380.
    let mut s = ramp_selected();
    wand(&mut s, 320, 10.0, "subtract");
    assert_eq!(selected(&s), Some((100, 220)));
    let mut s = ramp_selected();
    wand(&mut s, 320, 10.0, "intersect");
    assert_eq!(selected(&s), Some((220, 380)));
}
