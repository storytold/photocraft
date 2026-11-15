//! #13995: Load Files into Stack puts the first file of the list on top and the last at the
//! bottom; the last file gives the stack its mode, depth, resolution and profile. The order holds
//! inside the smart object and with auto-align.

use super::*;

/// Saves a `w` x `h` document filled with `color` as `<dir>/<name>`; `gray` makes it Grayscale
/// at 150 ppi.
fn file(dir: &std::path::Path, name: &str, w: u32, h: u32, color: &str, gray: bool) -> String {
    let mut s = Session::new();
    let mut p = json!({"width": w, "height": h, "background": color});
    if gray {
        p["mode"] = json!("grayscale");
        p["resolution"] = json!(150);
    }
    s.execute("file.new", p).unwrap();
    let path = dir.join(name).to_string_lossy().into_owned();
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

/// Layer names top to bottom.
fn top_down(layers: &[Layer]) -> Vec<String> {
    layers.iter().rev().map(|l| l.name.clone()).collect()
}

fn three(dir: &std::path::Path) -> Vec<String> {
    vec![file(dir, "a_red.png", 20, 10, "#ff0000", false), file(dir, "b_green.png", 30, 15, "#00ff00", false), file(dir, "c_gray.png", 10, 12, "#808080", true)]
}

#[test]
fn the_first_file_is_the_top_layer_and_the_last_the_base() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": three(dir.path())})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(top_down(&doc.layers), ["a_red.png", "b_green.png", "c_gray.png"]);
    // The base is the last file: Grayscale at 150 ppi; the canvas still holds the largest file.
    assert_eq!(doc.mode, ColorMode::Grayscale);
    assert!((doc.resolution_dpi - 150.0).abs() < 0.5, "{}", doc.resolution_dpi);
    assert_eq!((doc.size.width, doc.size.height), (30, 15));
    // The red file, now Grayscale, is still the top layer's content at its own spot.
    let top = photocraft_compose::render(doc, Rect::new(1, 1, 2, 2)).px[0];
    assert!(top[3] > 0.99 && (top[0] - top[1]).abs() < 0.01, "{top:?}");
}

#[test]
fn reversing_the_list_reverses_the_stack_and_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = three(dir.path());
    files.reverse();
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(top_down(&doc.layers), ["c_gray.png", "b_green.png", "a_red.png"]);
    assert_eq!(doc.mode, ColorMode::Rgb, "the base is now a_red.png");
}

#[test]
fn the_smart_object_keeps_the_order_inside() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        file(dir.path(), "c_wide.png", 40, 10, "#0000ff", false),
        file(dir.path(), "a_base.png", 20, 10, "#ff0000", false),
        file(dir.path(), "b_shift.png", 20, 10, "#00ff00", false),
    ];
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files, "createSmartObject": true})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("not a smart object") };
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &smart.source else { panic!("not embedded") };
    let contents = photocraft_io::import(file_name, bytes).unwrap().document;
    assert_eq!(top_down(&contents.layers), ["c_wide.png", "a_base.png", "b_shift.png"]);
}

#[test]
fn auto_align_keeps_the_first_file_on_top() {
    let dir = tempfile::tempdir().unwrap();
    // Textured, so Auto-Align finds features.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 72, "background": "#606060"})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        for k in 0..40 {
            let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
            surf.fill_rect(Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    let mut files = Vec::new();
    for name in ["n_base.png", "n_shift.png"] {
        let path = dir.path().join(name).to_string_lossy().into_owned();
        save_doc(&s.active().unwrap().doc, &path, None).unwrap();
        files.push(path);
    }
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files, "align": true})).unwrap();
    assert_eq!(top_down(&s.active().unwrap().doc.layers), ["n_base.png", "n_shift.png"]);
}
