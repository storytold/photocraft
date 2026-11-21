//! #13699: Contact Sheet II cells split the page (columns by rows), each image is fitted in a box
//! inset by the spacing and centred, and the caption goes under the box. The reference bounds are
//! the issue's measurements of the epic's test folder, with its 3 px tolerance.

use super::tests::{images, tmp};
use super::*;
use photocraft_doc::Document;
use photocraft_geom::Rect;

/// The epic's test folder: a_red 200x100, b_green 100x200, c_blue 150x150, d gray 120x80, then
/// e_yellow 80x80 and f_cyan 60x90 when `all` is set.
fn folder(name: &str, all: bool) -> String {
    let dir = tmp(name);
    let sizes = [("a_red", 200, 100), ("b_green", 100, 200), ("c_blue", 150, 150), ("d gray", 120, 80), ("e_yellow", 80, 80), ("f_cyan", 60, 90)];
    for (file, w, h) in sizes.into_iter().take(if all { 6 } else { 4 }) {
        let made = images(&dir, 1, w, h);
        std::fs::rename(&made[0], format!("{dir}/{file}.png")).unwrap();
    }
    dir
}

fn sheet(dir: &str, extra: Value) -> Document {
    let mut p = json!({"input": dir});
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    let mut s = Session::new();
    let r = s.execute("file.automate.contactSheetII", p).unwrap();
    let idx = r["documents"][0].as_u64().unwrap() as usize;
    (*s.documents()[idx].doc).clone()
}

fn bounds(d: &Document, name: &str, text: bool) -> Rect {
    let l = d.layers.iter().find(|l| l.name == name && matches!(l.content, LayerContent::Text(_)) == text).unwrap();
    l.surface().unwrap().content_bounds()
}

fn assert_near(d: &Document, expected: &[(&str, [i32; 4])]) {
    for (name, [x0, y0, x1, y1]) in expected {
        let b = bounds(d, name, false);
        let off = [b.x0 - x0, b.y0 - y0, b.x1 - x1, b.y1 - y1];
        assert!(off.iter().all(|o| o.abs() <= 3), "{name}: {b:?}, reference {x0},{y0} to {x1},{y1}");
    }
}

#[test]
fn a_400_px_two_by_two_sheet_matches_the_reference_bounds() {
    let dir = folder("geometry-400", false);
    let d = sheet(&dir, json!({"units": "pixels", "width": 400, "height": 400, "resolution": 72, "columns": 2, "rows": 2}));
    assert_near(&d, &[("a_red", [2, 45, 198, 143]), ("b_green", [254, 2, 346, 185]), ("c_blue", [9, 202, 192, 385]), ("d gray", [202, 228, 398, 359])]);
    let (a, c) = (bounds(&d, "a_red", true), bounds(&d, "c_blue", true));
    assert!((a.y0 - 186).abs() <= 3 && a.y1 <= 198 + 3, "a_red caption {a:?}, reference y 186..198");
    assert!((c.y0 - 386).abs() <= 3 && c.y1 <= 398 + 3, "c_blue caption {c:?}, reference y 386..398");
    assert!(c.y0 >= bounds(&d, "c_blue", false).y1, "c_blue caption {c:?} under its image");
}

#[test]
fn the_default_sheet_matches_the_reference_bounds() {
    let dir = folder("geometry-default", true);
    let d = sheet(&dir, json!({}));
    assert_eq!((d.size.width, d.size.height), (2400, 3000));
    assert_near(
        &d,
        &[
            ("a_red", [5, 106, 474, 341]),
            ("b_green", [609, 5, 828, 442]),
            ("c_blue", [979, 6, 1416, 441]),
            ("d gray", [1442, 69, 1911, 378]),
            ("e_yellow", [1936, 8, 2375, 439]),
            ("f_cyan", [93, 506, 386, 939]),
        ],
    );
}

#[test]
fn manual_spacing_down_first_with_rotation_matches_the_reference_bounds() {
    let dir = folder("geometry-manual", false);
    let p = json!({"units": "pixels", "width": 500, "height": 600, "resolution": 100, "columns": 3, "rows": 4, "placeAcrossFirst": false,
        "autoSpacing": false, "vertical": 20, "horizontal": 10, "rotateForBestFit": true, "fontSize": 20});
    let d = sheet(&dir, p);
    assert_near(&d, &[("a_red", [5, 20, 161, 98]), ("b_green", [5, 170, 161, 248]), ("c_blue", [35, 310, 132, 407]), ("d gray", [10, 460, 156, 557])]);
    let (a, b) = (bounds(&d, "a_red", true), bounds(&d, "b_green", true));
    assert!((a.y0 - 112).abs() <= 3 && a.y1 <= 138 + 3, "a_red caption {a:?}, reference y 112..138");
    assert!((b.y0 - 262).abs() <= 3 && b.y1 <= 288 + 3, "b_green caption {b:?}, reference y 262..288");
    assert!(a.y0 >= bounds(&d, "a_red", false).y1, "a_red caption {a:?} under its image");
}

#[test]
fn unmeasured_sizes_keep_every_image_and_caption_apart_on_the_page() {
    let dir = folder("geometry-unmeasured", true);
    let runs = [
        json!({"units": "pixels", "width": 900, "height": 500, "resolution": 72, "columns": 3, "rows": 2, "fontSize": 10}),
        json!({"units": "cm", "width": 10, "height": 10, "resolution": 150, "columns": 4, "rows": 4, "autoSpacing": false, "horizontal": 0, "vertical": 0}),
        // Whole-point cells would leave about 300 px of this page empty, so the cells stay exact.
        json!({"units": "pixels", "width": 2400, "height": 300, "resolution": 300, "columns": 100, "rows": 1, "caption": false}),
        // Whole points (20.83 px) would put the sixth image's centre left of its 24 px cell.
        json!({"units": "pixels", "width": 144, "height": 100, "resolution": 300, "columns": 6, "rows": 1, "caption": false}),
        // Spacing wider than a cell still leaves a box inside it.
        json!({"units": "pixels", "width": 600, "height": 600, "resolution": 72, "columns": 3, "rows": 2, "autoSpacing": false, "horizontal": 5000, "vertical": 5000, "caption": false}),
    ];
    for p in runs {
        let d = sheet(&dir, p.clone());
        let page = d.bounds();
        // Each image's centre lies in its cell of the exact page / columns by page / rows grid, row by row.
        let (cols, rows) = (p["columns"].as_i64().unwrap() as i32, p["rows"].as_i64().unwrap() as i32);
        let images = d.layers.iter().filter(|l| matches!(l.content, LayerContent::Raster(_))).skip(1);
        for (n, l) in images.enumerate() {
            let (b, n) = (l.surface().unwrap().content_bounds(), n as i32);
            let (cx, cy) = ((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2);
            let (w, h) = (d.size.width as i32 / cols, d.size.height as i32 / rows);
            assert!(cx / w == n % cols && cy / h == n / cols, "{p}: {} centred at {cx},{cy}, outside its cell", l.name);
        }
        let placed: Vec<Rect> = d.layers.iter().skip(1).map(|l| l.surface().unwrap().content_bounds()).collect();
        assert_eq!(placed.len(), if p["caption"] == false { 6 } else { 12 }, "{p}");
        for (i, a) in placed.iter().enumerate() {
            assert!(!a.is_empty() && page.contains_rect(a), "{p}: {a:?} outside the page");
            for b in &placed[i + 1..] {
                assert!(a.intersect(b).is_empty(), "{p}: {a:?} overlaps {b:?}");
            }
        }
    }
}
