//! #13997: Load Files into Stack raises an 8-bit stack to 16 bits/channel when a 16-bit file is
//! in the list, keeping that file's 16-bit values; a 32-bit file doesn't raise it, and a 16- or
//! 32-bit bottom file keeps its depth.

use super::*;
use photocraft_color::SampleType;

/// A 12 x 8 RGB document at `depth`, filled with `rgb`, saved as `<dir>/<name>`.
fn file(dir: &std::path::Path, name: &str, depth: u32, rgb: [f32; 3]) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 12, "height": 8, "depth": depth})).unwrap();
    s.edit("fill", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        surf.fill_rect(Rect::new(0, 0, 12, 8), &[rgb[0], rgb[1], rgb[2], 1.0]);
        Ok(())
    })
    .unwrap();
    let path = dir.join(name).to_string_lossy().into_owned();
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

/// A value 16 bits hold but 8 bits round away (8 bits give 77/255 = 0.30196).
const FINE: f32 = 0.3001;

fn load(params: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", params).unwrap();
    s
}

/// The top layer's red at (1, 1), rendered on its own.
fn top_red(doc: &Document) -> f32 {
    let mut one = doc.clone();
    let top = one.layers.pop().unwrap();
    one.layers = vec![top];
    photocraft_compose::render(&one, Rect::new(1, 1, 2, 2)).px[0][0]
}

#[test]
fn a_16_bit_file_raises_an_8_bit_stack_and_keeps_its_values() {
    let dir = tempfile::tempdir().unwrap();
    let sixteen = file(dir.path(), "e_16.tif", 16, [FINE, 0.4, 0.2]);
    let base = file(dir.path(), "a_base.tif", 8, [1.0, 0.0, 0.0]);
    let s = load(json!({"paths": [sixteen, base]}));
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.mode, doc.depth), (ColorMode::Rgb, SampleType::U16));
    assert!((top_red(doc) - FINE).abs() < 3.0 / 65535.0, "{} went through 8 bits", top_red(doc));
    // The 8-bit bottom layer looks as before.
    let mut only_base = Document::clone(doc);
    only_base.layers.truncate(1);
    let px = photocraft_compose::render(&only_base, Rect::new(1, 1, 2, 2)).px[0];
    assert!((px[0] - 1.0).abs() < 1e-3 && px[1] < 1e-3 && px[2] < 1e-3, "{px:?}");
}

#[test]
fn a_32_bit_file_does_not_raise_an_8_bit_stack() {
    let dir = tempfile::tempdir().unwrap();
    let thirty_two = file(dir.path(), "f_32.tif", 32, [0.5, 0.5, 0.5]);
    let base = file(dir.path(), "a_base.tif", 8, [1.0, 0.0, 0.0]);
    let s = load(json!({"paths": [thirty_two, base]}));
    assert_eq!(s.active().unwrap().doc.depth, SampleType::U8);
}

#[test]
fn a_16_or_32_bit_bottom_file_keeps_its_depth() {
    let dir = tempfile::tempdir().unwrap();
    let eight = file(dir.path(), "a_8.tif", 8, [1.0, 0.0, 0.0]);
    for (depth, expected) in [(16, SampleType::U16), (32, SampleType::F32)] {
        let base = file(dir.path(), &format!("base_{depth}.tif"), depth, [0.2, 0.2, 0.2]);
        let s = load(json!({"paths": [eight.clone(), base]}));
        assert_eq!(s.active().unwrap().doc.depth, expected, "{depth}-bit base");
    }
}

#[test]
fn the_smart_object_contents_and_an_aligned_stack_are_16_bit() {
    let dir = tempfile::tempdir().unwrap();
    let sixteen = file(dir.path(), "e_16.tif", 16, [FINE, 0.4, 0.2]);
    let base = file(dir.path(), "a_base.tif", 8, [1.0, 0.0, 0.0]);
    let s = load(json!({"paths": [sixteen.clone(), base.clone()], "createSmartObject": true}));
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.depth, SampleType::U16);
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("not a smart object") };
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &smart.source else { panic!("not embedded") };
    assert_eq!(photocraft_io::import(file_name, bytes).unwrap().document.depth, SampleType::U16);
    // Auto-Align needs detail to match: a textured 16-bit file over a textured 8-bit one.
    let files: Vec<String> = [("t_16.tif", 16), ("t_8.tif", 8)].iter().map(|(name, depth)| textured(dir.path(), name, *depth)).collect();
    let s = load(json!({"paths": files, "align": true}));
    assert_eq!(s.active().unwrap().doc.depth, SampleType::U16);
}

/// A 96 x 72 textured RGB document at `depth`, saved as `<dir>/<name>`.
fn textured(dir: &std::path::Path, name: &str, depth: u32) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 72, "depth": depth, "background": "#606060"})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        for k in 0..40 {
            let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
            surf.fill_rect(Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    let path = dir.join(name).to_string_lossy().into_owned();
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}
