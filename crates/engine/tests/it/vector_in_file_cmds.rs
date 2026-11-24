//! Tests that run vector commands, moved from the engine-core `file_cmds` tests: 1 test(s).

use photocraft_engine::Session;

use photocraft_engine::EngineError;

use photocraft_doc::{Layer, LayerContent};

use serde_json::json;

/// Two textured PNGs named a_base.png and b_shift.png (texture, so Auto-Align finds features).
fn two_files(dir: &std::path::Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 72, "background": "#606060"}))?;
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
        for k in 0..40 {
            let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
            surf.fill_rect(photocraft_geom::Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
        }
        Ok(())
    })?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let mut paths = Vec::new();
    for name in ["a_base.png", "b_shift.png"] {
        let path = dir.join(name).to_string_lossy().into_owned();
        photocraft_engine::file_cmds::save_doc(doc, &path, None)?;
        paths.push(path);
    }
    Ok(paths)
}

/// The smart object keeps the first file's name with and without aligning; only the document's
/// name changes.
#[test]
fn the_smart_object_keeps_the_first_files_name() {
    let dir = tempfile::tempdir().unwrap();
    let files = two_files(dir.path()).unwrap();
    for align in [false, true] {
        let mut s = Session::new();
        s.execute("file.scripts.loadFilesIntoStack", json!({"paths": files, "align": align, "createSmartObject": true})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.name, "Untitled-1", "align {align}");
        assert_eq!(st.path, None, "align {align}");
        assert_eq!(st.doc.layers.len(), 1, "align {align}");
        assert!(matches!(st.doc.layers[0].content, LayerContent::Smart(_)), "align {align}");
        assert_eq!(st.doc.layers[0].name, "a_base", "align {align}");
    }
}
