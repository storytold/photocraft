//! Image › Mode › Color Table recolours each pixel by its index where it is known, so entries of
//! one colour keep their own pixels; pixels past a shrunk table turn black (#14342).

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

const WHITE: [f32; 4] = [1.0; 4];

/// The issue's palette PNG: a 6 × 2 image over the table red, white, white, two columns per
/// index, opened as a document.
fn red_white_white() -> Session {
    let row = [0u8, 0, 1, 1, 2, 2];
    let indices: Vec<u8> = row.iter().chain(&row).copied().collect();
    let png = photocraft_codecs::encode_png_indexed(6, 2, &indices, &[[255, 0, 0], [255, 255, 255], [255, 255, 255]], None, None, None).unwrap();
    let opened = photocraft_io::import("red-white-white.png", &png).unwrap();
    let mut s = Session::new();
    s.add_document(opened.document, None);
    s
}

/// One pixel per column pair (index 0, 1 and 2 in the file).
fn pairs(s: &Session) -> [[f32; 4]; 3] {
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    [0, 2, 4].map(|x| surf.rgba(x, 1))
}

#[test]
fn an_entry_shared_with_another_keeps_its_own_pixels() {
    let mut s = red_white_white();
    assert_eq!(table(&s).colors, vec![[255, 0, 0], [255, 255, 255], [255, 255, 255]]);
    let before = (table(&s), pairs(&s));
    s.execute("image.mode.colorTable", json!({"entries": {"2": "#00ff00"}})).unwrap();
    assert_eq!(pairs(&s), [RED, WHITE, GREEN], "the index-2 pixels turn green, the index-1 pixels stay white");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((table(&s), pairs(&s)), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(pairs(&s), [RED, WHITE, GREEN]);
}

#[test]
fn indices_survive_one_colour_table_edit_after_another() {
    let mut s = red_white_white();
    s.execute("image.mode.colorTable", json!({"entries": {"1": "#0000ff"}})).unwrap();
    assert_eq!(pairs(&s), [RED, BLUE, WHITE]);
    s.execute("image.mode.colorTable", json!({"entries": {"1": "#ffffff", "2": "#ffff00"}})).unwrap();
    assert_eq!(pairs(&s), [RED, WHITE, YELLOW]);
}

#[test]
fn indices_from_the_indexed_color_conversion_tell_equal_entries_apart() {
    // Turn the green entry blue, then green again: the green bar's pixels kept their index
    // while the table had two blue entries.
    let mut s = bars();
    let (red, green, blue) = (index_of(&table(&s), RED), index_of(&table(&s), GREEN), index_of(&table(&s), BLUE));
    s.execute("image.mode.colorTable", json!({"entries": {green.to_string(): "#0000ff"}})).unwrap();
    assert_eq!(bar_colours(&s), [RED, BLUE, BLUE, YELLOW]);
    s.execute("image.mode.colorTable", json!({"entries": {green.to_string(): "#00ff00"}})).unwrap();
    assert_eq!(bar_colours(&s), [RED, GREEN, BLUE, YELLOW]);
    s.execute("image.mode.colorTable", json!({"entries": {red.to_string(): "#0000ff", blue.to_string(): "#ff00ff"}})).unwrap();
    assert_eq!(bar_colours(&s), [BLUE, GREEN, [1.0, 0.0, 1.0, 1.0], YELLOW]);
}

#[test]
fn a_saved_document_keeps_its_indices() {
    let s = red_white_white();
    let saved = photocraft_io::export(&s.active().unwrap().doc, "pcraft", &Default::default()).unwrap();
    let reopened = photocraft_io::import("red-white-white.pcraft", &saved.bytes).unwrap();
    assert!(reopened.document.color_indices.is_some());
    let mut s = Session::new();
    s.add_document(reopened.document, None);
    s.execute("image.mode.colorTable", json!({"entries": {"2": "#00ff00"}})).unwrap();
    assert_eq!(pairs(&s), [RED, WHITE, GREEN]);
}

#[test]
fn painted_pixels_fall_back_to_the_nearest_entry() {
    // After a paint stroke the indices are unknown: as before, each pixel's colour picks the
    // first matching entry.
    let mut s = red_white_white();
    s.edit("paint", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let fmt = surf.format();
        surf.write_region(Rect::from_xywh(5, 0, 1, 1), &photocraft_raster::from_rgba(&fmt, WHITE));
        Ok(())
    })
    .unwrap();
    s.execute("image.mode.colorTable", json!({"entries": {"2": "#00ff00"}})).unwrap();
    assert_eq!(pairs(&s), [RED, WHITE, WHITE]);
}

#[test]
fn a_known_index_past_a_shrunk_table_stays_black_until_the_table_grows_back() {
    let mut s = bars();
    let t = table(&s);
    let yellow = index_of(&t, YELLOW);
    assert_eq!(yellow, 3, "the test needs a bar at index 3");
    s.execute("image.mode.colorTable", json!({"colors": ["#00ffff", "#ff00ff", "#ffffff"]})).unwrap();
    assert_eq!(bar_colours(&s)[3], [0.0, 0.0, 0.0, 1.0]);
    // Black is as near cyan as magenta: by colour it would take entry 0 and turn red here.
    s.execute("image.mode.colorTable", json!({"entries": {"0": "#ff0000"}})).unwrap();
    assert_eq!(bar_colours(&s)[3], [0.0, 0.0, 0.0, 1.0]);
    s.execute("image.mode.colorTable", json!({"colors": ["#ff0000", "#ff00ff", "#ffffff", "#00ff00"]})).unwrap();
    assert_eq!(bar_colours(&s)[3], GREEN, "index 3 takes the new entry 3");
}

#[test]
fn moved_pixels_fall_back_to_the_nearest_entry() {
    // Solid tiles of one colour but different indices: index 0 red, then index 2 white, then
    // index 1 white, 256 columns each. Canvas Size anchored right moves the pixels 100 columns
    // left; a moved tile can share storage with the one recorded at its place, so the indices
    // must not be trusted after a move. The pixels of index 1 must not turn green.
    let indices: Vec<u8> = (0..256).flat_map(|_| (0..768).map(|x| [0u8, 2, 1][x / 256])).collect();
    let png = photocraft_codecs::encode_png_indexed(768, 256, &indices, &[[255, 0, 0], [255, 255, 255], [255, 255, 255]], None, None, None).unwrap();
    let mut s = Session::new();
    s.add_document(photocraft_io::import("solid.png", &png).unwrap().document, None);
    s.execute("image.canvasSize", json!({"width": 668, "height": 256, "anchor": "right"})).unwrap();
    s.execute("image.mode.colorTable", json!({"entries": {"2": "#00ff00"}})).unwrap();
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    for x in [412, 500, 667] {
        assert_eq!(surf.rgba(x, 10), WHITE, "x {x} came from index 1");
    }
}

/// A palette PNG of `indices` (row by row, `w` wide) at `depth` bits over `palette`, with an
/// optional eXIf block.
fn palette_png(w: u32, indices: &[u8], depth: png::BitDepth, palette: &[[u8; 3]], exif: Option<&[u8]>) -> Vec<u8> {
    let bits = depth as usize;
    let rows: Vec<u8> = indices
        .chunks(w as usize)
        .flat_map(|row| {
            let mut packed = vec![0u8; (row.len() * bits).div_ceil(8)];
            for (x, &i) in row.iter().enumerate() {
                packed[x * bits / 8] |= i << (8 - bits - x * bits % 8);
            }
            packed
        })
        .collect();
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, (indices.len() / w as usize) as u32);
    e.set_color(png::ColorType::Indexed);
    e.set_depth(depth);
    e.set_palette(palette.concat());
    let mut wr = e.write_header().unwrap();
    if let Some(exif) = exif {
        wr.write_chunk(png::chunk::eXIf, exif).unwrap();
    }
    wr.write_image_data(&rows).unwrap();
    drop(wr);
    out
}

#[test]
fn palette_pngs_of_every_bit_depth_keep_their_indices() {
    // Odd widths, so rows end in a partly used byte.
    for (depth, w) in [(png::BitDepth::One, 11), (png::BitDepth::Two, 7), (png::BitDepth::Four, 5), (png::BitDepth::Eight, 3)] {
        let max = (1u16 << depth as u8) - 1;
        let row: Vec<u8> = (0..w).map(|x| (x as u16 % (max + 1)).min(max) as u8).collect();
        let indices: Vec<u8> = row.iter().chain(&row).copied().collect();
        let palette: Vec<[u8; 3]> = (0..=max).map(|_| [255, 255, 255]).collect();
        let png = palette_png(w, &indices, depth, &palette, None);
        let doc = photocraft_io::import("p.png", &png).unwrap().document;
        let layer = &doc.layers[0];
        let known = doc.color_indices.as_ref().unwrap().known(layer.id, layer.surface().unwrap(), doc.bounds());
        assert_eq!(known.as_deref(), Some(&indices[..]), "{depth:?}");
    }
}

#[test]
fn a_transparent_entry_keeps_its_index() {
    let png = photocraft_codecs::encode_png_indexed(3, 1, &[0, 1, 2], &[[0, 0, 0], [255, 255, 255], [255, 255, 255]], Some(0), None, None).unwrap();
    let mut s = Session::new();
    s.add_document(photocraft_io::import("t.png", &png).unwrap().document, None);
    assert!(s.active().unwrap().doc.color_indices.is_some());
    s.execute("image.mode.colorTable", json!({"entries": {"2": "#00ff00"}})).unwrap();
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    assert_eq!([0, 1, 2].map(|x| surf.rgba(x, 0)), [[0.0; 4], WHITE, GREEN]);
}

#[test]
fn a_turned_palette_png_records_no_indices() {
    // EXIF orientation 2 (mirrored): the file's indices aren't where its pixels open, and with
    // two white entries the colours can't show it.
    let exif = b"MM\x00\x2a\x00\x00\x00\x08\x00\x01\x01\x12\x00\x03\x00\x00\x00\x01\x00\x02\x00\x00\x00\x00\x00\x00";
    let png = palette_png(4, &[0, 0, 1, 1], png::BitDepth::Eight, &[[255, 255, 255], [255, 255, 255]], Some(exif));
    let opened = photocraft_io::import("turned.png", &png).unwrap();
    assert_eq!(opened.document.color_table.as_ref().map(|t| t.colors.len()), Some(2), "opens as Indexed Color");
    assert!(opened.document.color_indices.is_none());
    // Control: the same file without the eXIf block keeps them.
    let png = palette_png(4, &[0, 0, 1, 1], png::BitDepth::Eight, &[[255, 255, 255], [255, 255, 255]], None);
    assert!(photocraft_io::import("upright.png", &png).unwrap().document.color_indices.is_some());
}

#[test]
fn a_flip_saved_and_reopened_leaves_the_indices_unknown() {
    // White, red, white: mirror-symmetric colours, so the flipped tiles have the same bytes as
    // the recorded ones, and a reopened file loads identical tiles as one. The save must not
    // carry the indices from before the flip.
    let row = [0u8, 0, 1, 1, 2, 2];
    let indices: Vec<u8> = row.iter().chain(&row).copied().collect();
    let png = photocraft_codecs::encode_png_indexed(6, 2, &indices, &[[255, 255, 255], [255, 0, 0], [255, 255, 255]], None, None, None).unwrap();
    let mut s = Session::new();
    s.add_document(photocraft_io::import("wrw.png", &png).unwrap().document, None);
    s.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap();
    let saved = photocraft_io::export(&s.active().unwrap().doc, "pcraft", &Default::default()).unwrap();
    let reopened = photocraft_io::import("wrw.pcraft", &saved.bytes).unwrap();
    assert!(reopened.document.color_indices.is_none());
    let mut s = Session::new();
    s.add_document(reopened.document, None);
    s.execute("image.mode.colorTable", json!({"entries": {"2": "#00ff00"}})).unwrap();
    assert_eq!(pairs(&s), [WHITE, RED, WHITE], "the right columns came from index 0");
}
