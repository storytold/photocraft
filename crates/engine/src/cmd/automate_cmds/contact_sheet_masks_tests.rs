//! #13701: with Flatten All Layers off, Contact Sheet II gives each image layer a layer mask that
//! shows its cell and hides the rest of the sheet (a lasso around the cell, then Add Layer Mask).

use super::tests::{images, tmp};
use super::*;
use photocraft_doc::{BlendMode, Document};
use photocraft_geom::Rect;

/// 2×2 cells of 100×50 px at 72 ppi (no outer margin); 10 px spacing insets each image box by 5 px,
/// so without captions a cell's mask spans x 0..100 / 100..200 and y 0..45 / 50..95.
fn sheet(s: &mut Session, dir: &str, extra: Value) -> Vec<usize> {
    let mut p = json!({"input": dir, "units": "pixels", "width": 200, "height": 100, "resolution": 72, "columns": 2, "rows": 2, "caption": false, "autoSpacing": false, "horizontal": 10, "vertical": 10});
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    let r = s.execute("file.automate.contactSheetII", p).unwrap();
    r["documents"].as_array().unwrap().iter().map(|d| d.as_u64().unwrap() as usize).collect()
}

fn mask(l: &Layer) -> &photocraft_doc::LayerMask {
    l.mask.as_ref().unwrap_or_else(|| panic!("{} has no mask", l.name))
}

fn image_layers(d: &Document) -> Vec<&Layer> {
    d.layers.iter().filter(|l| l.name.starts_with("img") && matches!(l.content, LayerContent::Raster(_))).collect()
}

#[test]
fn every_image_layer_of_an_unflattened_sheet_is_masked_to_its_cell() {
    let dir = tmp("masks-cells");
    // 40×40 thumbnails fit as 40×40 in the middle of each 90×40 box.
    images(&dir, 3, 40, 40);
    for mode in ["rgb", "gray"] {
        for depth in [8, 16] {
            let mut s = Session::new();
            let idx = sheet(&mut s, &dir, json!({"mode": mode, "depth": depth}))[0];
            let st = &s.documents()[idx];
            let d = &st.doc;
            let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
            assert_eq!(names, ["Background", "img0", "img1", "img2"], "{mode} {depth}");
            assert!(d.is_background(d.layers[0].id) && d.layers[0].mask.is_none(), "the Background stays unmasked");
            for (l, cell) in d.layers[1..].iter().zip([Rect::new(0, 0, 100, 45), Rect::new(100, 0, 200, 45), Rect::new(0, 50, 100, 95)]) {
                let m = mask(l);
                assert!(m.enabled && m.linked && m.density == 1.0 && m.feather == 0.0, "{}", l.name);
                assert_eq!((l.opacity, l.blend, l.visible), (1.0, BlendMode::Normal, true), "{}", l.name);
                assert_eq!(m.surface.content_bounds(), cell, "{} is revealed over its cell only ({mode} {depth})", l.name);
                let (x1, y1) = (cell.x1 - 1, cell.y1 - 1);
                for (x, y) in [(cell.x0, cell.y0), (x1, cell.y0), (cell.x0, y1), (x1, y1), (cell.x0 + 50, cell.y0 + 20)] {
                    assert_eq!(m.value(x, y), 1.0, "{} shows its cell at ({x},{y})", l.name);
                }
                for (x, y) in [(cell.x0 - 1, cell.y0), (x1 + 1, cell.y0), (cell.x0, cell.y0 - 1), (cell.x0, y1 + 1)] {
                    assert_eq!(m.value(x, y), 0.0, "{} hides ({x},{y}) outside its cell", l.name);
                }
            }
            assert!(d.selection.is_none(), "no selection left behind");
            let h = st.history.entries();
            let steps: Vec<&str> = h.iter().map(String::as_str).filter(|e| *e == "Lasso" || *e == "Add Layer Mask").collect();
            assert_eq!(steps, ["Lasso", "Add Layer Mask"].repeat(3), "{h:?}");
            assert!(h.iter().enumerate().all(|(i, e)| e != "Add Layer Mask" || h[i - 1] == "Lasso"), "each mask right after its lasso: {h:?}");
        }
    }
}

#[test]
fn captions_keep_no_mask_and_the_caption_band_is_hidden() {
    let dir = tmp("masks-captions");
    images(&dir, 3, 60, 30);
    let mut s = Session::new();
    // A 10 pt caption takes 11 px, so the image boxes are 29 px high and the cells end at y 34 / 84.
    let idx = sheet(&mut s, &dir, json!({"caption": true, "fontSize": 10}))[0];
    let d = &s.documents()[idx].doc;
    let texts: Vec<&Layer> = d.layers.iter().filter(|l| matches!(l.content, LayerContent::Text(_))).collect();
    assert_eq!(texts.len(), 3);
    assert!(texts.iter().all(|l| l.mask.is_none()), "captions are not masked");
    let layers = image_layers(d);
    assert_eq!(layers.len(), 3);
    for (l, (x0, y1)) in layers.into_iter().zip([(0, 34), (100, 34), (0, 84)]) {
        let (m, b) = (mask(l), l.surface().unwrap().content_bounds());
        assert!(!b.is_empty());
        for (x, y) in [(b.x0, b.y0), (b.x1 - 1, b.y0), (b.x0, b.y1 - 1), (b.x1 - 1, b.y1 - 1)] {
            assert_eq!(m.value(x, y), 1.0, "{} shows its thumbnail corner ({x},{y})", l.name);
        }
        assert_eq!((m.value(x0, y1 - 1), m.value(x0, y1)), (1.0, 0.0), "{} hides the caption band from y {y1}", l.name);
    }
}

#[test]
fn the_masks_change_no_pixels_and_a_flattened_sheet_gets_none() {
    let dir = tmp("masks-flatten");
    images(&dir, 4, 30, 60);
    // At 300 ppi the cells are 112.5 x 62.5 px (whole points), so they sit on fractional
    // coordinates. No captions: a caption's pixels change when it is flattened, masks or not.
    for (mode, depth) in [("rgb", 8), ("gray", 8), ("rgb", 16), ("gray", 16)] {
        let mut s = Session::new();
        let p = |flatten: bool| json!({"autoSpacing": true, "width": 230, "height": 131, "resolution": 300, "mode": mode, "depth": depth, "flatten": flatten});
        let layered = sheet(&mut s, &dir, p(false))[0];
        let flat = sheet(&mut s, &dir, p(true))[0];
        let layered: &Document = &s.documents()[layered].doc;
        let mut unmasked = layered.clone();
        unmasked.layers.iter_mut().for_each(|l| l.mask = None);
        assert!(image_layers(layered).iter().all(|l| l.mask.is_some()), "{mode} {depth}");
        assert_eq!(photocraft_compose::flatten(layered), photocraft_compose::flatten(&unmasked), "the masks hide nothing drawn ({mode} {depth})");
        let flat = &s.documents()[flat];
        assert_eq!(photocraft_compose::flatten(&flat.doc), photocraft_compose::flatten(layered), "flattened as today ({mode} {depth})");
        let h = flat.history.entries();
        assert!(!h.iter().any(|e| e == "Add Layer Mask" || e == "Lasso"), "{h:?}");
    }
}

#[test]
fn the_second_page_of_a_multi_page_sheet_is_masked_too() {
    let dir = tmp("masks-pages");
    images(&dir, 5, 40, 40);
    let mut s = Session::new();
    let docs = sheet(&mut s, &dir, json!({}));
    assert_eq!(docs.len(), 2);
    let d = &s.documents()[docs[1]].doc;
    let layers = image_layers(d);
    assert_eq!(layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["img4"]);
    assert_eq!(mask(layers[0]).surface.content_bounds(), Rect::new(0, 0, 100, 45), "the first cell of page 2");
}

#[test]
fn cells_past_the_page_edge_get_a_hide_all_mask() {
    let dir = tmp("masks-offpage");
    // 30 columns on a 20 px page: 1 px cells, the last ten start past the right edge.
    images(&dir, 25, 4, 4);
    let mut s = Session::new();
    let idx = sheet(&mut s, &dir, json!({"width": 20, "height": 20, "columns": 30, "rows": 1, "autoSpacing": true}))[0];
    let d = &s.documents()[idx].doc;
    let layers = image_layers(d);
    assert_eq!(layers.len(), 25);
    for (c, l) in layers.into_iter().enumerate() {
        let (m, x) = (mask(l), c as i32);
        if x < 20 {
            assert_eq!(m.surface.content_bounds(), Rect::new(x, 0, x + 1, 20), "{}", l.name);
        } else {
            assert!(m.surface.content_bounds().is_empty() && m.value(x, 0) == 0.0, "{} is hidden", l.name);
        }
    }
}
