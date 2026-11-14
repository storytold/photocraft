//! #13700: a Contact Sheet II caption is the full file name as paragraph text: a box as wide as the
//! cell and 16 pt high, its left edge at the cell's left edge, centre justified, black. The image
//! layer above it keeps the name without the extension.

use super::tests::{images, tmp};
use super::*;
use photocraft_color::Color;
use photocraft_doc::text::{TextAlign, TextShape};
use photocraft_doc::{Document, TextLayer};

/// One `w`x`h` image per file name, written in the format its extension names.
fn folder(name: &str, files: &[(&str, u32, u32)]) -> String {
    let dir = tmp(name);
    for (file, w, h) in files {
        let png = images(&dir, 1, *w, *h).remove(0);
        let doc = import(&file_name(&png), &std::fs::read(&png).unwrap()).unwrap();
        std::fs::remove_file(&png).unwrap();
        crate::file_cmds::save_doc(&doc, &format!("{dir}/{file}"), None).unwrap();
    }
    dir
}

/// Runs the command with a red foreground colour, so a black caption is the command's own choice,
/// and with Flatten All Layers off, so the caption and image layers stay apart.
fn sheet(dir: &str, extra: Value) -> Document {
    let mut p = json!({"input": dir, "flatten": false});
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    let mut s = Session::new();
    s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    let r = s.execute("file.automate.contactSheetII", p).unwrap();
    (*s.documents()[r["documents"][0].as_u64().unwrap() as usize].doc).clone()
}

fn captions(d: &Document) -> Vec<(&str, &TextLayer)> {
    d.layers
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Text(t) => Some((l.name.as_str(), t)),
            _ => None,
        })
        .collect()
}

fn images_named(d: &Document) -> Vec<&str> {
    d.layers.iter().skip(1).filter(|l| matches!(l.content, LayerContent::Raster(_))).map(|l| l.name.as_str()).collect()
}

/// Checks each caption's text, name, box `[x, y, w, h]` (x and size only), alignment and colour.
fn assert_paragraph_captions(d: &Document, expected: &[(&str, f64)], cell_w: f64, box_h: f64) {
    let found = captions(d);
    assert_eq!(found.len(), expected.len());
    for ((name, t), (file, x)) in found.into_iter().zip(expected) {
        assert_eq!((name, t.text.as_str()), (*file, *file), "caption text and layer name");
        let TextShape::Box { width, height, .. } = t.shape else { panic!("{file}: point text, not paragraph text") };
        assert!((f64::from(width) - cell_w).abs() < 0.01 && (f64::from(height) - box_h).abs() < 0.01, "{file}: box {width} x {height}");
        assert!((t.transform.m[4] - x).abs() < 0.01, "{file}: box left at {}, cell left at {x}", t.transform.m[4]);
        assert_eq!(t.paragraph_runs()[0].style.align, TextAlign::Center, "{file}");
        assert!(t.char_runs().iter().all(|r| r.style.color == Color::BLACK), "{file}: {:?}", t.color);
    }
}

/// Checks that each caption box starts `top` below the top of its row (rows of the first image's
/// cell height, from the page top).
fn assert_box_tops(d: &Document, top: f64) {
    for (name, t) in captions(d) {
        let y = t.transform.m[5];
        let row_h = if d.size.height == 3000 { 500.0 } else { 200.0 };
        let row_top = (y / row_h).floor() * row_h;
        assert!((y - row_top - top).abs() < 0.01, "{name}: box top {y}, expected {top} below the row top {row_top}");
    }
}

fn assert_point_at(t: &TextLayer, file: &str, x: f64, y: f64) {
    assert_eq!((t.text.as_str(), &t.shape), (file, &TextShape::Point), "{file}");
    let (tx, ty) = (t.transform.m[4], t.transform.m[5]);
    assert!((tx - x).abs() < 0.01 && (ty - y).abs() < 0.01, "{file}: anchor {tx},{ty}, expected {x},{y}");
}

#[test]
fn the_default_sheet_captions_each_image_with_its_file_name_in_a_cell_wide_box() {
    let dir = folder("caption-default", &[("a_red.png", 200, 100), ("b_green.jpg", 100, 200), ("d gray.tif", 120, 80)]);
    let d = sheet(&dir, json!({}));
    assert_eq!(images_named(&d), ["a_red", "b_green", "d gray"]);
    // 2400 px / 5 columns at 300 ppi in whole points: 115 pt = 479.17 px; 16 pt = 66.67 px.
    let cell = 115.0 * 300.0 / 72.0;
    assert_paragraph_captions(&d, &[("a_red.png", 0.0), ("b_green.jpg", cell), ("d gray.tif", 2.0 * cell)], cell, 16.0 * 300.0 / 72.0);
    // The box top is the caption strip's top: inset 4.79 px (1 % of 479.17) plus the image box,
    // 500 px less two insets and the 55 px strip (1.1 x 12 pt).
    let inset = cell * 0.01;
    assert_box_tops(&d, inset + (500.0 - 2.0 * inset - 55.0));
}

#[test]
fn a_400_px_sheet_puts_each_caption_box_at_its_cell_left_edge_under_the_image() {
    let dir = folder("caption-400", &[("a_red.png", 200, 100), ("b_green.png", 100, 200), ("c_blue.png", 150, 150), ("d gray.tif", 120, 80)]);
    let d = sheet(&dir, json!({"units": "pixels", "width": 400, "height": 400, "resolution": 72, "columns": 2, "rows": 2}));
    assert_eq!(images_named(&d), ["a_red", "b_green", "c_blue", "d gray"]);
    assert_paragraph_captions(&d, &[("a_red.png", 0.0), ("b_green.png", 200.0), ("c_blue.png", 0.0), ("d gray.tif", 200.0)], 200.0, 16.0);
    // Inset 2 px, image box 200 - 4 - 13.2 (1.1 x 12 pt) = 182.8 px: each box starts at 184.8 in its row.
    assert_box_tops(&d, 184.8);
    for (name, t) in captions(&d) {
        let image = d.layers.iter().find(|l| l.name == name.split('.').next().unwrap()).unwrap().surface().unwrap().content_bounds();
        assert!(t.transform.m[5] >= f64::from(image.y1), "{name}: box top {}, image {image:?}", t.transform.m[5]);
    }
}

#[test]
fn a_caption_that_does_not_fit_its_cell_on_one_line_stays_point_text_under_the_image() {
    // Not measured: wrapping or clipping a long name. 100 px cells at 72 ppi, inset 1 px, image box
    // 98 x 84.8 px: the point text anchor is the box's centre, 0.75 x the 13.2 px strip under it.
    let dir = folder("caption-long", &[("a rather long file name for a narrow cell.png", 60, 30), ("b.png", 60, 30)]);
    let d = sheet(&dir, json!({"units": "pixels", "width": 200, "height": 100, "resolution": 72, "columns": 2, "rows": 1}));
    let found = captions(&d);
    assert_eq!(found[0].0, "a rather long file name for a narrow cell.png");
    assert_point_at(found[0].1, found[0].0, 50.0, 1.0 + 84.8 + 0.75 * 13.2);
    assert!(matches!(found[1].1.shape, TextShape::Box { .. }), "a short name in the same sheet is paragraph text");
}

#[test]
fn other_font_sizes_keep_point_text_with_the_full_file_name() {
    // #13699's third measured case at 20 pt and 100 ppi: 166.67 x 150 px cells down first, insets
    // 5 and 10 px, a 30.56 px strip (1.1 x 20 pt), so a_red's image box is 156.67 x 99.44 px.
    let dir = folder("caption-20pt", &[("a_red.png", 200, 100), ("b_green.png", 100, 200), ("c_blue.png", 150, 150), ("d gray.tif", 120, 80)]);
    let p = json!({"units": "pixels", "width": 500, "height": 600, "resolution": 100, "columns": 3, "rows": 4, "placeAcrossFirst": false,
        "autoSpacing": false, "vertical": 20, "horizontal": 10, "rotateForBestFit": true, "fontSize": 20});
    let d = sheet(&dir, p);
    let found = captions(&d);
    assert_eq!(found.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["a_red.png", "b_green.png", "c_blue.png", "d gray.tif"]);
    let strip = 20.0 * 100.0 / 72.0 * 1.1;
    assert_point_at(found[0].1, "a_red.png", 5.0 + 156.667 / 2.0, 10.0 + (150.0 - 20.0 - strip) + 0.75 * strip);
    for (name, t) in &found {
        assert_eq!((t.text.as_str(), &t.shape), (*name, &TextShape::Point), "{name}");
    }
    // A short name at 10 pt would fit a 16 pt box, but 10 pt was not measured either.
    let d = sheet(&dir, json!({"units": "pixels", "width": 400, "height": 400, "resolution": 72, "columns": 2, "rows": 2, "fontSize": 10}));
    assert!(captions(&d).iter().all(|(_, t)| t.shape == TextShape::Point));
}
