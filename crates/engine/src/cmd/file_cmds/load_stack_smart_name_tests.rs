//! #13998: with Create Smart Object, Load Files into Stack names the smart object exactly like the
//! top layer inside it (the file name, extension included), and its contents `<name>.pcraft`,
//! with and without align.

use super::*;

/// 96 x 72 textured PNG and TIFF files (texture, so Auto-Align finds features).
fn files(dir: &std::path::Path) -> Vec<String> {
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
    ["c_wide.tif", "a_base.png", "b_shift.png"]
        .iter()
        .map(|name| {
            let path = dir.join(name).to_string_lossy().into_owned();
            save_doc(&s.active().unwrap().doc, &path, None).unwrap();
            path
        })
        .collect()
}

/// The smart object layer's name, its embedded contents' file name and the top layer inside.
fn smart(s: &Session) -> (String, String, String) {
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    let layer = &doc.layers[0];
    let LayerContent::Smart(so) = &layer.content else { panic!("not a smart object") };
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &so.source else { panic!("not embedded") };
    let contents = crate::smart_cmds::decode_source(file_name, bytes).unwrap();
    (layer.name.clone(), file_name.clone(), contents.layers.last().unwrap().name.clone())
}

#[test]
fn the_smart_object_is_named_after_its_top_layer() {
    let dir = tempfile::tempdir().unwrap();
    for align in [false, true] {
        let mut s = Session::new();
        s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files(dir.path()), "align": align, "createSmartObject": true})).unwrap();
        let (name, contents, top) = smart(&s);
        assert_eq!(name, top, "align {align}: named after the top layer inside");
        assert_eq!(name, "c_wide.tif", "align {align}: extension included");
        assert_eq!(contents, "c_wide.tif.pcraft", "align {align}");
    }
}

#[test]
fn the_name_follows_whichever_file_is_on_top() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = files(dir.path());
    paths.rotate_left(1);
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": paths, "createSmartObject": true})).unwrap();
    let (name, contents, top) = smart(&s);
    assert_eq!((name.as_str(), contents.as_str()), (top.as_str(), "a_base.png.pcraft"));
}
