//! Image › Mode › Color Table: pixels past a shrunk table turn black, and a table without
//! duplicates recolours as before (#14342).

use super::*;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// The epic's test document, scaled down: four solid bars red, green, blue and yellow, converted
/// to Indexed Color with an Exact palette, no dither and no transparency.
fn bars() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 4, "background": "white"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _ in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, [RED, GREEN, BLUE, YELLOW][x as usize / 2]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    s
}

fn table(s: &Session) -> ColorTable {
    s.active().unwrap().doc.color_table.clone().unwrap()
}

/// The colour of each bar (one pixel per bar, from the middle row).
fn bar_colours(s: &Session) -> [[f32; 4]; 4] {
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    [0, 2, 4, 6].map(|x| surf.rgba(x, 2))
}

fn index_of(t: &ColorTable, c: [f32; 4]) -> usize {
    t.colors.iter().position(|e| e.map(|v| f32::from(v) / 255.0) == [c[0], c[1], c[2]]).unwrap()
}

#[test]
fn pixels_past_a_shrunk_table_turn_black() {
    let mut s = bars();
    let before = (table(&s), bar_colours(&s));
    let indices = [RED, GREEN, BLUE, YELLOW].map(|c| index_of(&before.0, c));
    assert_eq!(before.0.colors.len(), 4);
    let new = [[0.0, 1.0, 1.0, 1.0], [1.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
    s.execute("image.mode.colorTable", json!({"colors": ["#00ffff", "#ff00ff", "#ffffff"]})).unwrap();
    let want = indices.map(|i| new.get(i).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(bar_colours(&s), want, "indices {indices:?}: the bar past the table is black, the others take their entry");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Color Table"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((table(&s), bar_colours(&s)), before);
}

/// Regression check for "a table without duplicates recolours exactly as today": passes on main too.
#[test]
fn editing_one_entry_recolours_only_its_pixels() {
    let mut s = bars();
    let before = bar_colours(&s);
    let blue = index_of(&table(&s), BLUE);
    s.execute("image.mode.colorTable", json!({"entries": {blue.to_string(): "#ff00ff"}})).unwrap();
    assert_eq!(bar_colours(&s), [RED, GREEN, [1.0, 0.0, 1.0, 1.0], YELLOW]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bar_colours(&s), before);
}
