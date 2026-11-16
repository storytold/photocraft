//! #13999: after Load Files into Stack's auto-align the canvas grows to the union of the layers,
//! so no aligned layer is cut off and the new area is transparent; without align, or when nothing
//! moves, the canvas keeps the largest file's size.

use super::*;

const W: i32 = 160;
const H: i32 = 120;

/// A textured W x H picture saved as `<dir>/<name>`, its texture drawn `(dx, dy)` further right
/// and down (the same scene, moved).
fn picture(dir: &std::path::Path, name: &str, dx: i32, dy: i32) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "background": "#606060"})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        for k in 0..120 {
            let (x, y, v) = ((k * 37) % 150 + dx, (k * 23) % 110 + dy, (k % 7) as f32 / 6.0);
            surf.fill_rect(Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    let path = dir.join(name).to_string_lossy().into_owned();
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

fn pair(dir: &std::path::Path) -> Vec<String> {
    vec![picture(dir, "n_base.png", 0, 0), picture(dir, "n_shift.png", 12, 8)]
}

/// Grown by the +12,+8 move: Auto-Align's own estimate lands within a pixel or two of it.
fn assert_grown(doc: &Document) {
    let (w, h) = (doc.size.width as i32, doc.size.height as i32);
    assert!((w - (W + 12)).abs() <= 2 && (h - (H + 8)).abs() <= 2, "{w} x {h}");
}

fn load(params: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", params).unwrap();
    s
}

#[test]
fn the_canvas_grows_to_the_aligned_layers() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(json!({"paths": pair(dir.path()), "align": true}));
    let doc = &s.active().unwrap().doc;
    // The canvas is exactly the union of the layers: every pixel of each layer lies on it.
    let union = doc.layers.iter().filter_map(Layer::surface).map(|sf| sf.content_bounds()).fold(Rect::new(0, 0, W, H), |a, b| a.union(&b));
    assert_eq!(doc.bounds(), union);
    assert_grown(doc);
    assert!(doc.layers.iter().all(|l| matches!(l.content, LayerContent::Raster(_))), "still pixel layers");
    // Top right: inside neither layer, so the new area there is transparent.
    let corner = photocraft_compose::render(doc, Rect::new(W + 11, 1, W + 12, 2)).px[0];
    assert_eq!(corner[3], 0.0, "{corner:?}");
}

#[test]
fn without_align_or_movement_the_canvas_keeps_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(json!({"paths": pair(dir.path())}));
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (W as u32, H as u32));
    // Two identical pictures: the align moves nothing.
    let same = vec![picture(dir.path(), "a.png", 0, 0), picture(dir.path(), "b.png", 0, 0)];
    let s = load(json!({"paths": same, "align": true}));
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (W as u32, H as u32));
}

#[test]
fn the_smart_object_covers_the_grown_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(json!({"paths": pair(dir.path()), "align": true, "createSmartObject": true}));
    let doc = &s.active().unwrap().doc;
    assert_grown(doc);
    let LayerContent::Smart(so) = &doc.layers[0].content else { panic!("not a smart object") };
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &so.source else { panic!("not embedded") };
    let contents = crate::smart_cmds::decode_source(file_name, bytes).unwrap();
    assert_eq!(contents.size, doc.size);
}
