//! Scripted `.pcraft` saves write the full-size composite when they ask for it
//! (`pcraftFullComposite`), and leave it out by default, as the app's Save does.

use photocraft_automation::{Headless, files};
use photocraft_format::zip::ZipReader;
use serde_json::json;

fn full_composite(bytes: &[u8]) -> ((u32, u32), Vec<u8>) {
    let png = photocraft_format::read_full_composite(bytes).unwrap().expect("the manifest names the full-size composite");
    let image = photocraft_codecs::decode(&png).unwrap();
    (image.dimensions(), image.to_rgba8())
}

fn has_full_composite(bytes: &[u8]) -> bool {
    photocraft_format::read_full_composite(bytes).unwrap().is_some() || ZipReader::new(bytes).unwrap().find("composite/full.png").is_some()
}

/// Both reduced PNGs retain independently selected profile bytes and unconverted native samples.
fn assert_reduced_profiles(bytes: &[u8], doc: &photocraft_doc::Document, icc: &[u8]) {
    let zip = ZipReader::new(bytes).unwrap();
    for (name, side) in [("thumb.png", 256), ("composite/preview.png", 1024)] {
        let png = zip.read_by_name(name, usize::MAX).unwrap();
        let image = photocraft_codecs::decode(&png).unwrap();
        assert_eq!(image.icc.as_deref(), Some(icc), "{name}");
        assert_eq!(image.to_rgba8(), photocraft_compose::thumbnail(doc, side).pixels);
    }
}

#[test]
/// Byte and file saves retain tagged reduced previews while the full preview remains opt-in.
fn scripted_native_saves_write_the_full_size_composite_when_asked() {
    let dir = std::env::temp_dir().join(format!(
        "pc-native-full-composite-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut headless = Headless::trusted_local();
    headless.handle("doc.new", json!({"width": 40, "height": 30, "name": "Full"})).unwrap();
    headless.handle("engine.execute", json!({"command":"image.adjustments.invert", "params":{}})).unwrap();
    let doc = headless.session.active().unwrap().doc.clone();
    let srgb = headless.session.color.resolve("srgb", None, None).unwrap().to_bytes();
    let flattened = photocraft_compose::render(&doc, doc.bounds()).to_rgba8().pixels;

    let (bytes, _) = files::save_bytes(&doc, "Full.pcraft", None, &Default::default()).unwrap();
    assert!(!has_full_composite(&bytes), "off by default");
    assert_reduced_profiles(&bytes, &doc, &srgb);
    let on = photocraft_io::ExportOptions { pcraft_full_composite: true, ..Default::default() };
    let (bytes, _) = files::save_bytes(&doc, "Full.pcraft", None, &on).unwrap();
    assert_eq!(full_composite(&bytes), ((40, 30), flattened.clone()));
    assert_reduced_profiles(&bytes, &doc, &srgb);

    let path = dir.join("Full.pcraft");
    headless.handle("doc.save", json!({"path": path.to_str().unwrap()})).unwrap();
    assert!(!has_full_composite(&std::fs::read(&path).unwrap()), "off by default");
    assert_reduced_profiles(&std::fs::read(&path).unwrap(), &doc, &srgb);
    headless.handle("doc.save", json!({"path": path.to_str().unwrap(), "pcraftFullComposite": true})).unwrap();
    assert_eq!(full_composite(&std::fs::read(&path).unwrap()), ((40, 30), flattened));
    assert_reduced_profiles(&std::fs::read(&path).unwrap(), &doc, &srgb);
    for bad in [json!("true"), json!(1)] {
        let err = headless.handle("doc.save", json!({"path": dir.join("Bad.pcraft").to_str().unwrap(), "pcraftFullComposite": bad})).unwrap_err();
        assert!(format!("{err:?}").contains("`pcraftFullComposite` must be true or false"), "{bad}: {err:?}");
        assert!(!dir.join("Bad.pcraft").exists(), "{bad}: nothing is written");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Assigning Display P3 changes metadata, not samples; scripted byte and file saves retain
/// that exact non-sRGB profile in both reduced PNGs with the full preview enabled or disabled.
#[test]
fn scripted_native_saves_keep_assigned_display_p3_profile_and_samples() {
    let dir = std::env::temp_dir().join(format!("pc-native-p3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut headless = Headless::trusted_local();
    headless.handle("doc.new", json!({"width": 3, "height": 2, "name": "P3"})).unwrap();
    headless
        .session
        .edit("Native RGB samples", |doc, _| {
            let mut surface = photocraft_raster::Surface::new(doc.pixel_format());
            surface.write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
            surface.write_pixel(1, 0, &[0.8, 0.2, 0.1, 0.4]);
            doc.layers = vec![photocraft_doc::Layer::new("Samples", photocraft_doc::LayerContent::Raster(surface))];
            Ok(())
        })
        .unwrap();
    let native = photocraft_compose::thumbnail(&headless.session.active().unwrap().doc, 1024).pixels;
    let p3 = headless.session.color.resolve("display-p3", None, None).unwrap().to_bytes();
    let srgb = headless.session.color.resolve("srgb", None, None).unwrap().to_bytes();
    assert_ne!(p3, srgb, "the independent expected profile must distinguish P3 from sRGB");
    headless.handle("engine.execute", json!({"command": "edit.assignProfile", "params": {"profile": "display-p3"}})).unwrap();
    let doc = headless.session.active().unwrap().doc.clone();
    assert_eq!(doc.icc_profile.as_deref().map(Vec::as_slice), Some(p3.as_slice()));
    assert_eq!(photocraft_compose::thumbnail(&doc, 1024).pixels, native, "assigning a profile does not convert native samples");
    for full in [false, true] {
        let opts = photocraft_io::ExportOptions { pcraft_full_composite: full, ..Default::default() };
        let (bytes, _) = files::save_bytes(&doc, "P3.pcraft", None, &opts).unwrap();
        assert_reduced_profiles(&bytes, &doc, &p3);
        assert_eq!(has_full_composite(&bytes), full);
        let path = dir.join(format!("P3-{full}.pcraft"));
        headless.handle("doc.save", json!({"path": path.to_str().unwrap(), "pcraftFullComposite": full})).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_reduced_profiles(&bytes, &doc, &p3);
        assert_eq!(has_full_composite(&bytes), full);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
