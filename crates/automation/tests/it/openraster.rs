//! OpenRaster through the agent surface (`doc.*` and `engine.execute`, as MCP and the CLI use
//! it): save a layered document as .ora, reopen it with its layers, edit it and save it back in
//! place.

use photocraft_automation::Headless;
use serde_json::json;

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pc-openraster-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) -> serde_json::Value {
    h.handle("engine.execute", json!({"command": command, "params": params})).unwrap()
}

#[test]
fn layered_documents_save_reopen_and_save_in_place_as_openraster() {
    let dir = tmp("roundtrip");
    let path = dir.join("art.ora");
    let path_str = path.to_str().unwrap();
    let mut h = Headless::trusted_local();
    h.handle("doc.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    exec(&mut h, "layer.new.layer", json!({}));
    exec(&mut h, "paint.stroke", json!({"points": [[4, 4], [30, 20]], "size": 6, "hardness": 1.0, "color": "#ff0000"}));
    let top = h.session.active().unwrap().doc.layers.last().unwrap().id.0;
    exec(&mut h, "layer.setProps", json!({"layer": top, "name": "Ink", "opacity": 0.5, "blend": "Multiply"}));
    let before = photocraft_compose::flatten(&h.session.active().unwrap().doc);

    let saved = h.handle("doc.save", json!({"path": path_str})).unwrap();
    assert!(saved["warnings"].as_array().is_none_or(Vec::is_empty), "{saved}");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.get(30..54), Some(&b"mimetypeimage/openraster"[..]), "a stored mimetype comes first");
    assert!(!h.session.active().unwrap().is_dirty(), "an OpenRaster save is a layered save");
    assert_eq!(h.session.active().unwrap().path.as_deref(), Some(path_str));

    let opened = h.handle("doc.open", json!({"path": path_str})).unwrap();
    let doc = &h.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 2, "{opened}");
    let ink = doc.layers.last().unwrap();
    assert_eq!((ink.name.as_str(), ink.opacity, ink.blend), ("Ink", 0.5, photocraft_doc::BlendMode::Multiply));
    assert_eq!(photocraft_compose::flatten(doc).px, before.px);

    // Edit, then a pathless save writes the .ora back in place and clears dirty.
    exec(&mut h, "image.adjustments.invert", json!({}));
    assert!(h.session.active().unwrap().is_dirty());
    h.handle("doc.save", json!({})).unwrap();
    assert!(!h.session.active().unwrap().is_dirty());
    let reread = photocraft_io::import("art.ora", &std::fs::read(&path).unwrap()).unwrap().document;
    assert_eq!(photocraft_compose::flatten(&reread).px, photocraft_compose::flatten(&h.session.active().unwrap().doc).px);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_corrupt_openraster_file_reports_an_error() {
    let dir = tmp("corrupt");
    let path = dir.join("broken.ora");
    std::fs::write(&path, b"PK\x03\x04 not really a zip").unwrap();
    let mut h = Headless::trusted_local();
    let err = h.handle("doc.open", json!({"path": path.to_str().unwrap()})).unwrap_err();
    assert!(err.to_string().contains("OpenRaster"), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}
