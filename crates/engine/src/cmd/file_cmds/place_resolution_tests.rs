//! File › Place Embedded keeps a placed file's physical size (#14105): a 100 × 60 px file at
//! 300 ppi placed into a 400 × 300 px document at 72 ppi comes in at 24 × 14.4 px (0.24), as in
//! the reference application, with the smart object's contents still the 100 × 60 px file.

use super::*;

/// A new `w` × `h` document at `ppi`.
fn session(w: u32, h: u32, ppi: f64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "resolution": ppi, "depth": 8})).unwrap();
    s
}

/// A `w` × `h` image at `ppi`, written as `name`'s format (PNG keeps it in `pHYs`, TIFF in its
/// resolution tags). Checked to read back at that resolution, so a test can't pass on a file
/// that lost it.
fn file_at(name: &str, w: u32, h: u32, ppi: f64) -> Vec<u8> {
    let s = session(w, h, ppi);
    let bytes = photocraft_io::export(&s.active().unwrap().doc, name, &photocraft_io::ExportOptions::default()).unwrap().bytes;
    let back = photocraft_io::import(name, &bytes).unwrap().document;
    assert_eq!((back.size.width, back.size.height, f64::from(back.resolution_dpi)), (w, h, ppi), "{name} keeps its size and resolution");
    bytes
}

/// Places `bytes` as `name` through `file.placeEmbedded`'s bytes path (as the menu does).
fn place(s: &mut Session, name: &str, bytes: Vec<u8>) -> Value {
    place_bytes(s, name, bytes, None, &json!({})).unwrap()
}

/// The place's reported scale (%) and box.
fn scale_and_bounds(r: &Value) -> (f64, [f64; 4]) {
    let b: Vec<f64> = r["bounds"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    (r["scale"].as_f64().unwrap(), [b[0], b[1], b[2], b[3]])
}

/// The placed smart object.
fn placed(s: &Session) -> SmartObject {
    let d = s.active().unwrap();
    match &d.doc.layer(d.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(so) => so.clone(),
        _ => panic!("the place made a smart object"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// The measured case, as PNG and as TIFF: 0.24, 24 × 14.4 px centred on the canvas (188 to 212
/// across; the box starts on a whole pixel, 143 for the measured 142.8, as every place does).
#[test]
fn a_300_ppi_file_placed_into_72_ppi_comes_in_at_its_physical_size() {
    for name in ["src.png", "src.tif"] {
        let mut s = session(400, 300, 72.0);
        let (scale, [x0, y0, x1, y1]) = scale_and_bounds(&place(&mut s, name, file_at(name, 100, 60, 300.0)));
        assert!(close(scale, 24.0), "{name}: scale {scale}");
        assert!(close(x0, 188.0) && close(x1, 212.0), "{name}: across {x0}..{x1}");
        assert!(close(x1 - x0, 24.0) && close(y1 - y0, 14.4), "{name}: {}×{}", x1 - x0, y1 - y0);
        assert!(close(y0, 143.0), "{name}: top {y0}");
        // The pixels are the resampled 24 × 14.4 (its last row partly covered), not the file's 100 × 60.
        let cache = placed(&s).cache.unwrap().content_bounds();
        assert_eq!((cache.x0, cache.y0, cache.x1, cache.y1), (188, 143, 212, 158), "{name}: rendered pixels");
    }
}

/// The smart object keeps the full-resolution file: its contents are the 100 × 60 px, 300 ppi
/// file, scaled by 0.24 to land, and Edit Contents opens them at that size and resolution.
#[test]
fn the_smart_object_keeps_the_full_resolution_contents() {
    let mut s = session(400, 300, 72.0);
    let bytes = file_at("src.png", 100, 60, 300.0);
    place(&mut s, "src.png", bytes.clone());
    let so = placed(&s);
    match &so.source {
        SmartSource::Embedded { bytes: kept, .. } => assert_eq!(kept.as_slice(), bytes.as_slice(), "the file is embedded as it is"),
        _ => panic!("an embedded smart object"),
    }
    let [a, b, c, d, e, f] = so.transform.m;
    assert!(close(a, 0.24) && close(d, 0.24) && close(b, 0.0) && close(c, 0.0), "scaled by 0.24: {:?}", so.transform.m);
    assert!(close(e, 188.0) && close(f, 143.0), "at 188,143: {:?}", so.transform.m);

    s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    let contents = &s.active().unwrap().doc;
    assert_eq!((contents.size.width, contents.size.height), (100, 60), "Edit Contents shows the file's own size");
    assert!(close(f64::from(contents.resolution_dpi), 300.0), "at its own resolution: {}", contents.resolution_dpi);
}

/// The resolution comes first: a file larger than the canvas in pixels that fits once at its
/// physical size isn't fitted (1000 × 600 px at 300 ppi: 240 × 144, not today's 0.4), while one
/// still larger than the canvas at that size is fitted to it as before (2000 × 1500 at 300 ppi:
/// 480 × 360, fitted to 400 × 300).
#[test]
fn the_resolution_scale_comes_before_the_fit_to_the_canvas() {
    let mut s = session(400, 300, 72.0);
    let (scale, [x0, y0, x1, y1]) = scale_and_bounds(&place(&mut s, "big.png", file_at("big.png", 1000, 600, 300.0)));
    assert!(close(scale, 24.0), "fits at its physical size: scale {scale}");
    assert!(close(x1 - x0, 240.0) && close(y1 - y0, 144.0), "{}×{}", x1 - x0, y1 - y0);
    assert!(close(x0, 80.0) && close(y0, 78.0), "centred: {x0},{y0}");

    let mut s = session(400, 300, 72.0);
    let (scale, [x0, y0, x1, y1]) = scale_and_bounds(&place(&mut s, "huge.png", file_at("huge.png", 2000, 1500, 300.0)));
    assert!(close(scale, 20.0), "fitted to the canvas: scale {scale}");
    assert_eq!([x0, y0, x1, y1], [0.0, 0.0, 400.0, 300.0]);
}

/// A file at the document's resolution, or one at a lower resolution (not measured), places at
/// its pixel size as before; so does a file with no resolution stored, which reads as 72 ppi.
#[test]
fn a_file_at_or_below_the_document_resolution_places_as_before() {
    for (doc_ppi, file_ppi) in [(72.0, 72.0), (300.0, 300.0), (300.0, 72.0), (72.0, 36.0)] {
        let mut s = session(400, 300, doc_ppi);
        let (scale, bounds) = scale_and_bounds(&place(&mut s, "src.png", file_at("src.png", 100, 60, file_ppi)));
        assert!(close(scale, 100.0), "{file_ppi} ppi into {doc_ppi} ppi: scale {scale}");
        assert_eq!(bounds, [150.0, 120.0, 250.0, 180.0], "{file_ppi} ppi into {doc_ppi} ppi");
    }
    // A PNG without `pHYs`.
    let img = photocraft_codecs::Image::from_u8(100, 60, photocraft_codecs::ChannelLayout::Rgb, vec![128; 100 * 60 * 3]).unwrap();
    let bare = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    for doc_ppi in [72.0, 300.0] {
        let mut s = session(400, 300, doc_ppi);
        let (scale, bounds) = scale_and_bounds(&place(&mut s, "bare.png", bare.clone()));
        assert!(close(scale, 100.0), "no resolution into {doc_ppi} ppi: scale {scale}");
        assert_eq!(bounds, [150.0, 120.0, 250.0, 180.0], "no resolution into {doc_ppi} ppi");
    }
}

/// An explicit `scale` is the caller's, of the file's pixels, as before.
#[test]
fn an_explicit_scale_is_not_changed_by_the_resolution() {
    let mut s = session(400, 300, 72.0);
    let r = place_bytes(&mut s, "src.png", file_at("src.png", 100, 60, 300.0), None, &json!({"scale": 50})).unwrap();
    let (scale, [x0, y0, x1, y1]) = scale_and_bounds(&r);
    assert!(close(scale, 50.0), "scale {scale}");
    assert_eq!([x0, y0, x1, y1], [175.0, 135.0, 225.0, 165.0]);
}
