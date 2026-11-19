//! Opting out of native RAW Smart Object render caches through scripted saves.

use std::path::Path;

use photocraft_automation::Headless;
use photocraft_doc::Document;
use photocraft_raw::testgen::{DngSpec, mosaic, scene};
use serde_json::json;

pub(super) fn raw_smart_object(path: &Path) -> Document {
    let (width, height) = (40, 24);
    let samples = mosaic(&scene(width, height), width, [0, 1, 1, 2], 0, 65535);
    std::fs::write(path, DngSpec::cfa(width, height, samples).build()).unwrap();
    let mut session = photocraft_engine::Session::new();
    session.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    session.active().unwrap().doc.as_ref().clone()
}

pub(super) fn assert_saved_cache(path: &Path, keep: bool) {
    let saved = photocraft_format::load_path(path).unwrap();
    let photocraft_doc::LayerContent::Smart(smart) = &saved.layers[0].content else { panic!("expected Smart Object in {}", path.display()) };
    assert!(smart.raw_render_recipe.is_some(), "{}", path.display());
    assert_eq!(smart.cache.is_some(), keep, "{}", path.display());
}

#[test]
fn json_rpc_pcraft_render_cache_opt_out_for_zip_and_directory() {
    let root = std::env::temp_dir().join(format!("pc-rpc-render-cache-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut headless = Headless::trusted_local();
    headless.session.add_document(raw_smart_object(&root.join("source.dng")), None);
    let zip = root.join("out.pcraft");
    let dir = root.join("bundle.pcraft");
    std::fs::create_dir(&dir).unwrap();
    for path in [&zip, &dir] {
        for choice in [None, Some(false), Some(true)] {
            let mut params = json!({"path": path});
            if let Some(keep) = choice {
                params["pcraftRenderCaches"] = json!(keep);
            }
            headless.handle("doc.save", params).unwrap();
            assert_saved_cache(path, choice.unwrap_or(true));
        }
    }
    let before = std::fs::read(&zip).unwrap();
    let error = headless.handle("doc.save", json!({"path": zip, "pcraftRenderCaches": "false"})).unwrap_err();
    assert!(error.to_string().contains("`pcraftRenderCaches` must be true or false"), "{error}");
    assert_eq!(std::fs::read(zip).unwrap(), before);
    assert!(headless.session.active().unwrap().doc.layers[0].surface().is_some());
    std::fs::remove_dir_all(root).unwrap();
}
