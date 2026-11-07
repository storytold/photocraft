use serde_json::json;

use photocraft_geom::Rect;

use super::*;

/// Writes test images of the given sizes (each a flat colour) and returns their paths.
fn images(name: &str, sizes: &[(u32, u32)]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-figure-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = Vec::new();
    for (i, (w, h)) in sizes.iter().enumerate() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "background": "#808080"})).unwrap();
        let path = dir.join(format!("scan{i}.png"));
        let out = photocraft_io::export(&s.active().unwrap().doc, &path.to_string_lossy(), &photocraft_io::ExportOptions::default()).unwrap();
        std::fs::write(&path, out.bytes).unwrap();
        paths.push(path.to_string_lossy().into_owned());
    }
    (dir, paths)
}

#[test]
fn rows_match_heights_fill_the_width_and_label_each_panel() {
    // Two CT slices of different sizes and a wide reconstruction.
    let (dir, files) = images("rows", &[(512, 512), (400, 300), (800, 400)]);
    let mut s = Session::new();
    let r = s.execute(CMD, json!({"files": files, "columns": 2, "width": "double", "gutterMm": 1.0})).unwrap();
    // 175 mm at 300 dpi; a 1 mm gutter is 12 px.
    assert_eq!(r["width"].as_u64(), Some(2067), "{r}");
    let rects: Vec<[u64; 4]> = r["panels"].as_array().unwrap().iter().map(|p| serde_json::from_value(p["rect"].clone()).unwrap()).collect();
    assert_eq!(rects[0][3], rects[1][3], "same height in a row");
    assert_eq!(rects[1][0] + rects[1][2], 2067, "the row ends at the right edge");
    assert_eq!(rects[1][0], rects[0][2] + 12, "one gutter between them");
    assert_eq!(rects[2][1], rects[0][3] + 12, "the second row starts after one gutter");
    assert_eq!(rects[2][2], 2067, "a single image fills its row");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.resolution_dpi, 300.0);
    let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
    for want in ["scan0", "Label a", "scan1", "Label b", "scan2", "Label c"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    let label = doc.layers.iter().find(|l| l.name == "Label a").unwrap();
    assert!(matches!(label.content, LayerContent::Text(_)), "editable type");
    assert!(!label.effects.items.is_empty(), "outlined");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn grid_layout_fits_images_in_equal_cells() {
    let (dir, files) = images("grid", &[(300, 300), (300, 600)]);
    let mut s = Session::new();
    let r = s
        .execute(
            CMD,
            json!({"files": files, "layout": "grid", "width": "custom", "widthMm": 100, "gutterMm": 0, "labels": "(A)", "labelPosition": "bottomRight"}),
        )
        .unwrap();
    let rects: Vec<[u64; 4]> = r["panels"].as_array().unwrap().iter().map(|p| serde_json::from_value(p["rect"].clone()).unwrap()).collect();
    // Cells are square (the widest image's shape); the tall image is centred inside its cell.
    assert_eq!(rects[0][2], rects[0][3]);
    assert_eq!(rects[1][3], rects[0][3], "fitted by height");
    assert!(rects[1][2] < rects[1][3]);
    assert!(s.active().unwrap().doc.layers.iter().any(|l| l.name == "Label (B)"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn labels_count_past_z() {
    assert_eq!(label("abc", 0), "a");
    assert_eq!(label("ABC", 25), "Z");
    assert_eq!(label("abc", 26), "aa");
    assert_eq!(label("(a)", 1), "(b)");
    assert_eq!(label("A)", 2), "C)");
}

#[test]
fn bad_params_are_errors_and_open_nothing() {
    let (dir, files) = images("bad", &[(10, 10)]);
    let mut s = Session::new();
    for p in [
        json!({}),
        json!({"files": []}),
        json!({"files": [1, 2]}),
        json!({"files": files, "columns": 0}),
        json!({"files": files, "layout": "spiral"}),
        json!({"files": files, "width": "triple"}),
        json!({"files": files, "width": "custom", "widthMm": 1e9}),
        json!({"files": files, "dpi": 1e9}),
        json!({"files": files, "gutterColor": "white"}),
        json!({"files": files, "labels": "123"}),
        json!({"files": files, "labelPosition": "middle"}),
        json!({"files": files, "labelOutline": "yes"}),
        json!({"files": ["/no/such/image.png"]}),
    ] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        assert!(s.active().is_none(), "no document opened for {p}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A collage like an OsiriX capture: `rows` x `cols` panels with no gaps, each a different grey with
/// a brighter blob, on a black canvas.
fn collage(s: &mut Session, rows: u32, cols: u32, cell: (u32, u32)) {
    s.execute("file.new", json!({"width": cols * cell.0, "height": rows * cell.1, "background": "black", "resolution": 300, "name": "2.tif"})).unwrap();
    s.edit("cells", |doc, _| {
        let bg = doc.background_mut().ok_or(EngineError::Other("no background".into()))?;
        if let LayerContent::Raster(surf) = &mut bg.content {
            let fmt = surf.format();
            for r in 0..rows {
                for c in 0..cols {
                    let g = 0.15 + 0.6 * ((r * cols + c) as f32 / (rows * cols) as f32);
                    let (x0, y0) = ((c * cell.0) as i32, (r * cell.1) as i32);
                    surf.fill_rect(Rect::new(x0, y0, x0 + cell.0 as i32, y0 + cell.1 as i32), &photocraft_raster::from_rgba(&fmt, [g, g, g, 1.0]));
                    let (bx, by) = (x0 + cell.0 as i32 / 3, y0 + cell.1 as i32 / 3);
                    surf.fill_rect(Rect::new(bx, by, bx + 20, by + 20), &photocraft_raster::from_rgba(&fmt, [1.0, 1.0, 1.0, 1.0]));
                }
            }
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_collage_grid_is_found_split_and_labelled_a_to_j() {
    let mut s = Session::new();
    collage(&mut s, 2, 5, (101, 120));
    let original = photocraft_doc::Document::clone(&s.active().unwrap().doc);
    let r = s.execute(COLLAGE, json!({"width": "double", "gutterMm": 0.5, "gutterColor": "#ffffff"})).unwrap();
    assert_eq!((r["rows"].as_u64(), r["columns"].as_u64()), (Some(2), Some(5)), "{r}");
    // detectGrid on the collage reports the panel edges too.
    let first = s.documents().iter().position(|d| d.doc.name == "2.tif").unwrap();
    let mut probe = Session::new();
    probe.add_document(photocraft_doc::Document::clone(&s.documents()[first].doc), None);
    let g = probe.execute("figure.detectGrid", json!({})).unwrap();
    assert_eq!(g["x"], json!([0, 101, 202, 303, 404, 505]), "{g}");
    assert_eq!(g["y"], json!([0, 120, 240]));
    assert_eq!(r["panels"].as_array().map(Vec::len), Some(10));
    assert_eq!(r["width"].as_u64(), Some(2067), "175 mm at 300 dpi");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.name, "2 figure");
    for want in ["Label A", "Label E", "Label J"] {
        assert!(doc.layers.iter().any(|l| l.name == want), "{want}");
    }
    // The collage document itself is untouched.
    let first = s.documents().iter().map(|d| &d.doc).find(|d| d.name == "2.tif").unwrap();
    assert_eq!(**first, original);
}

#[test]
fn rows_and_columns_can_be_given_and_are_remembered() {
    let mut s = Session::new();
    collage(&mut s, 1, 3, (100, 100));
    let r = s.execute(COLLAGE, json!({"rows": 1, "columns": 3, "labels": "(a)", "dpi": 600, "width": "single", "labelBold": false})).unwrap();
    let label = s.active().unwrap().doc.layers.iter().find(|l| l.name == "Label (a)").cloned().unwrap();
    assert!(matches!(&label.content, LayerContent::Text(t) if !format!("{t:?}").contains("faux_bold: true")), "regular weight");
    assert_eq!(r["columns"].as_u64(), Some(3));
    assert_eq!(r["width"].as_u64(), Some(2008), "85 mm at 600 dpi");
    let saved = s.prefs().dialogs.get(COLLAGE).cloned().unwrap();
    assert_eq!(saved["labels"], "(a)");
    assert_eq!(saved["dpi"], 600);
    // A rejected collage changes nothing: no new document, the active one untouched, and the
    // remembered choices kept.
    let state = |s: &Session| {
        let st = s.active().unwrap();
        (s.documents().len(), st.doc.id, st.revision, s.prefs().dialogs.get(COLLAGE).cloned())
    };
    for p in [json!({"rows": 40}), json!({"labels": "xyz"}), json!({"gutterColor": "white"})] {
        let before = state(&s);
        assert!(s.execute(COLLAGE, p.clone()).is_err(), "{p}");
        assert_eq!(state(&s), before, "{p}");
    }
}

#[test]
fn more_panels_than_pixels_is_an_error() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 8, "background": "black"})).unwrap();
    let before = s.documents().len();
    assert!(s.execute(COLLAGE, json!({"rows": 10, "columns": 3})).is_err());
    assert_eq!(s.documents().len(), before);
}

#[test]
fn edge_detection_needs_an_edge_at_every_boundary() {
    // Strong edges at the quarters: four parts, not two.
    let mut scores = vec![0.01; 400];
    for at in [100, 200, 300] {
        scores[at] = 1.0;
    }
    assert_eq!(detect_parts(&scores), (4, vec![0, 100, 200, 300, 400]));
    // An edge 2 px off the equal split is found where it is.
    scores[200] = 0.01;
    scores[202] = 1.0;
    assert_eq!(detect_parts(&scores).1, vec![0, 100, 202, 300, 400]);
    assert_eq!(detect_parts(&vec![0.01; 400]), (1, vec![0, 400]), "no edges: one part");
}

#[test]
fn uneven_panels_split_on_their_real_edges() {
    // Panels 101, 101 and 98 px wide (captures are a pixel or two uneven): an equal split at 100
    // and 200 would cut a sliver of the neighbouring panel into the first two.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 100, "background": "black", "name": "uneven.tif"})).unwrap();
    s.edit("cells", |doc, _| {
        let bg = doc.background_mut().ok_or(EngineError::Other("no background".into()))?;
        if let LayerContent::Raster(surf) = &mut bg.content {
            let fmt = surf.format();
            for (x0, x1, g) in [(0, 101, 0.2f32), (101, 202, 0.8), (202, 300, 0.5)] {
                surf.fill_rect(Rect::new(x0, 0, x1, 100), &photocraft_raster::from_rgba(&fmt, [g, g, g, 1.0]));
            }
        }
        Ok(())
    })
    .unwrap();
    let g = s.execute("figure.detectGrid", json!({})).unwrap();
    assert_eq!(g["x"], json!([0, 101, 202, 300]), "{g}");
    let r = s.execute(COLLAGE, json!({"rows": 1, "width": "custom", "widthMm": 100, "dpi": 72, "gutterMm": 0, "labels": "none"})).unwrap();
    assert_eq!(r["columns"].as_u64(), Some(3), "{r}");
    // Each panel is one flat grey: no sliver of a neighbour was cut in.
    let doc = &s.active().unwrap().doc;
    for p in r["panels"].as_array().unwrap() {
        let id = photocraft_doc::LayerId(p["layer"].as_u64().unwrap());
        let [x, y, w, h]: [i32; 4] = serde_json::from_value(p["rect"].clone()).unwrap();
        let LayerContent::Raster(surf) = &doc.layer(id).unwrap().content else { panic!("pixels") };
        let (a, b) = (surf.pixel(x, y + h / 2)[0], surf.pixel(x + w - 1, y + h / 2)[0]);
        assert!((a - b).abs() < 0.02, "panel {p}: {a} vs {b}");
    }
}
