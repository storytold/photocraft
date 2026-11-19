use photocraft_doc::LayerContent;
use photocraft_engine::{Session, prefs::RawDemosaic};
use photocraft_raw::{DevelopOptions, testgen::DngSpec};
use serde_json::json;

#[test]
fn smart_raw_pixels_match_the_explicit_open_recipe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pattern.dng");
    let samples = (0..64 * 64).map(|i| (528 + ((i % 64) * 60 + (i / 64) * 7) as u16 % 4000).min(3567)).collect();
    let bytes = DngSpec::cfa(64, 64, samples).build();
    std::fs::write(&path, &bytes).unwrap();

    let mut cached = Vec::new();
    for method in [RawDemosaic::Ahd, RawDemosaic::Bilinear, RawDemosaic::Mhc] {
        let mut app = Session::new();
        app.edit_prefs(|p| p.raw_defaults.demosaic = method);
        app.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
        let doc = &app.active().unwrap().doc;
        let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected the raw smart layer") };

        let opts = DevelopOptions { demosaic: method.method(), ..DevelopOptions::default() };
        let imported = photocraft_io::raw::import_raw_with("pattern.dng", &bytes, &opts).unwrap();
        assert!(!imported.preview_only);
        assert_eq!(imported.document.icc_profile, doc.icc_profile);
        assert_eq!(imported.document.pixel_format(), doc.pixel_format());
        let rebuilt = photocraft_compose::flatten_to_surface(&imported.document, doc.pixel_format(), None);
        assert_eq!(smart.cache.as_ref(), Some(&rebuilt), "raw method {method:?}");
        let mut unloaded = smart.clone();
        unloaded.cache = None;
        let rebuilt_from_recipe = photocraft_io::raw::render_smart_object_recipe(doc, &unloaded, 512 << 20, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(smart.cache, rebuilt_from_recipe, "native recipe can rebuild the actual saved cache");
        unloaded.transform.m[4] = 1.0;
        assert!(photocraft_io::raw::render_smart_object_recipe(doc, &unloaded, 512 << 20, &photocraft_raster::Interrupt::NONE).unwrap().is_none());
        assert_saved_recipe(doc, method, &bytes);
        assert_compact_cache_policy(doc);
        cached.push(rebuilt);
        assert_engine_save_honors_override(&mut app, &directory.path().join(format!("{}.pcraft", method.method().id())));
    }
    assert_ne!(cached[0], cached[1], "the settings are observable");
    assert_ne!(cached[0], cached[2], "the settings are observable");
}

fn assert_saved_recipe(doc: &photocraft_doc::Document, method: RawDemosaic, source: &[u8]) {
    let saved = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let archive = photocraft_format::zip::ZipReader::new(&saved).unwrap();
    let manifest = archive.read(archive.find("manifest.json").unwrap(), 64 * 1024).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let layer = &json["document"]["layers"][0]["content"];
    assert!(layer["cache"].is_object(), "regular saves still carry the rendered cache");
    assert_eq!(layer["raw_render_recipe"]["demosaic"], json!(method.method().id()));
    assert_eq!(layer["raw_render_recipe"]["version"], json!(1));
    assert_eq!(layer["raw_render_recipe"]["source_hash"], json!(blake3::hash(source).as_bytes()));
}

fn assert_compact_cache_policy(doc: &photocraft_doc::Document) {
    let keep = photocraft_io::ExportOptions { pcraft_full_composite: true, ..Default::default() };
    let compact = photocraft_io::ExportOptions { pcraft_render_caches: false, ..keep.clone() };
    let regular = photocraft_io::export(doc, "ordinary.pcraft", &keep).unwrap();
    let smaller = photocraft_io::export(doc, "ordinary.pcraft", &compact).unwrap();
    let regular_doc = photocraft_format::load_from_bytes(&regular.bytes).unwrap();
    let smaller_doc = photocraft_format::load_from_bytes(&smaller.bytes).unwrap();
    let LayerContent::Smart(regular_smart) = &regular_doc.layers[0].content else { panic!("expected smart object") };
    let LayerContent::Smart(compact_smart) = &smaller_doc.layers[0].content else { panic!("expected smart object") };
    assert!(regular_smart.cache.is_some());
    assert!(compact_smart.cache.is_none());
    assert_eq!(compact_smart.source, regular_smart.source);
    assert_eq!(compact_smart.raw_render_recipe, regular_smart.raw_render_recipe);
    assert!(smaller.bytes.len() < regular.bytes.len());
    assert_eq!(photocraft_format::read_full_composite(&smaller.bytes).unwrap(), photocraft_format::read_full_composite(&regular.bytes).unwrap());
    assert_native_reopens(&smaller.bytes, regular_smart.cache.as_ref().unwrap());
    assert_unusable_recipe_warns(smaller_doc);

    let mut changed = doc.clone();
    let LayerContent::Smart(smart) = &mut changed.layers[0].content else { panic!("expected smart object") };
    smart.cache.as_mut().unwrap().write_region(photocraft_geom::Rect::new(0, 0, 1, 1), &[0.0, 1.0, 0.0, 1.0]);
    let changed_cache = smart.cache.clone();
    let rebuilt = photocraft_io::export(&changed, "changed.pcraft", &compact).unwrap();
    let changed_back = photocraft_format::load_from_bytes(&rebuilt.bytes).unwrap();
    let LayerContent::Smart(saved) = &changed_back.layers[0].content else { panic!("expected smart object") };
    assert_eq!(saved.cache, changed_cache, "a cache with different rendered pixels cannot be discarded");
}

fn assert_native_reopens(bundle: &[u8], expected: &photocraft_raster::Surface) {
    let mut reopened = Session::new();
    reopened.edit_prefs(|prefs| prefs.raw_defaults.demosaic = RawDemosaic::Bilinear);
    let result = photocraft_engine::file_cmds::open_bytes_as(&mut reopened, "ordinary.pcraft", bundle, None, None).unwrap();
    let doc = &reopened.active().unwrap().doc;
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected smart object") };
    assert_eq!(smart.cache.as_ref(), Some(expected), "native open reconstructs the exact recorded pixels using the saved demosaic");
    assert_eq!(result["warnings"], json!([]));
}

fn assert_unusable_recipe_warns(mut doc: photocraft_doc::Document) {
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected smart object") };
    let mut stale = smart.raw_render_recipe.clone().unwrap();
    stale.source_hash[0] ^= 1;
    for recipe in [None, Some(stale)] {
        let LayerContent::Smart(smart) = &mut doc.layers[0].content else { panic!("expected smart object") };
        smart.raw_render_recipe = recipe;
        let bundle = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut reopened = Session::new();
        let result = photocraft_engine::file_cmds::open_bytes_as(&mut reopened, "legacy.pcraft", &bundle, None, None).unwrap();
        assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
        assert!(result["warnings"][0].as_str().unwrap().contains("pattern"));
    }
}

fn assert_engine_save_honors_override(session: &mut Session, path: &std::path::Path) {
    session.execute("file.saveACopy", json!({"path": path, "pcraftRenderCaches": false})).unwrap();
    let written = photocraft_format::load_path(path).unwrap();
    let LayerContent::Smart(smart) = &written.layers[0].content else { panic!("expected smart object") };
    assert!(smart.cache.is_none(), "Save a Copy must pass the explicit omission option to the encoder");
}
