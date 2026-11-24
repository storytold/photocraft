//! Tests that run vector commands, moved from the engine-core `automate_cmds` tests: 2 test(s).

use photocraft_doc::{Layer, LayerContent};
use photocraft_engine::{EngineError, Session};
use serde_json::json;

pub(super) fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-automate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

#[test]
fn stacked_smart_object_holds_the_loaded_layers_at_its_top_level() {
    // Like Photoshop, Load Files into Stack › Create Smart Object (Statistics without alignment) and
    // Statistics with alignment embed one layer per file, bottom to top, with no wrapper group.
    let dir = tmp("stack-top");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 72, "background": "#606060"})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        for k in 0..40 {
            let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
            surf.fill_rect(photocraft_geom::Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    let source = photocraft_compose::flatten(&s.active().unwrap().doc);
    let files: Vec<String> = (0..3)
        .map(|i| {
            let path = format!("{dir}/img{i}.png");
            photocraft_engine::file_cmds::save_doc(&s.active().unwrap().doc, &path, None).unwrap();
            path
        })
        .collect();
    for align in [false, true] {
        let mut s = Session::new();
        s.execute("file.scripts.statistics", json!({"mode": "median", "input": files, "align": align})).unwrap();
        let d = &s.active().unwrap().doc;
        let LayerContent::Smart(so) = &d.layers[0].content else { panic!("not a smart object") };
        let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &so.source else { panic!("not embedded") };
        let inner = photocraft_engine::smart_cmds::decode_source(file_name, bytes).unwrap();
        let names: Vec<&str> = inner.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["img0.png", "img1.png", "img2.png"], "align {align}");
        assert_eq!(photocraft_compose::flatten(d).px, source.px, "align {align}");
    }
}

/// #4671: Load Files into Stack's Attempt to Automatically Align Source Images runs Auto-Align
/// Layers on the loaded layers, then (with Create Smart Object) puts them in one smart object;
/// without the smart object they stay loose layers, selected.
#[test]
fn load_files_into_stack_aligns_before_the_smart_object() {
    let dir = tmp("stack-align");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 72, "background": "#606060"})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        for k in 0..40 {
            let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
            surf.fill_rect(photocraft_geom::Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    let files: Vec<String> = (0..2)
        .map(|i| {
            let path = format!("{dir}/img{i}.png");
            photocraft_engine::file_cmds::save_doc(&s.active().unwrap().doc, &path, None).unwrap();
            path
        })
        .collect();
    let mut s = Session::new();
    let r = s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files, "align": true})).unwrap();
    assert_eq!(r["layers"], 2);
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.len(), 2, "loose layers");
    assert!(st.doc.layers.iter().all(|l| matches!(l.content, LayerContent::Raster(_))));
    assert_eq!(st.selected_layers().len(), 2, "Auto-Align ran on them all");
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files, "align": true, "createSmartObject": true})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers.len(), 1);
    let LayerContent::Smart(_) = &d.layers[0].content else { panic!("not a smart object") };
    // A single file has nothing to align with: it just loads.
    let mut s = Session::new();
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": [files[0].clone()], "align": true})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
}
