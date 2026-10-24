//! Synthetic relationship wire regressions; independent GIMP corpus acceptance is separate.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartSource};
use photocraft_engine::{Session, file_cmds::open_bytes_as};
use photocraft_geom::{Rect, Size};
use serde_json::json;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("pc-xcf-relationships-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn resource(&self) -> PathBuf {
        self.0.join("resource.pcraft")
    }
    fn write_resource(&self, color: [f32; 4]) {
        let mut doc = Document::new("resource", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("Resource", doc.pixel_format());
        layer.surface_mut().unwrap().write_pixel(0, 0, &color);
        doc.layers.push(layer);
        std::fs::write(self.resource(), photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.resource());
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn open(bytes: &[u8], name: &str) -> Session {
    let mut session = Session::new();
    session.color.settings.ask_on_missing = false;
    open_bytes_as(&mut session, name, bytes, None, None).unwrap();
    session
}

/// One-pixel drawables written directly from the public XCF field layout. No test-only
/// production branch, private corpus path, source interpreter or source application involved.
fn source(locator: &str) -> Vec<u8> {
    fn u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn ptr(bytes: &mut Vec<u8>, value: usize) {
        bytes.extend_from_slice(&(value as u64).to_be_bytes());
    }
    fn slot(bytes: &mut Vec<u8>) -> usize {
        let position = bytes.len();
        ptr(bytes, 0);
        position
    }
    fn patch(bytes: &mut [u8], position: usize, value: usize) {
        bytes[position..position + 8].copy_from_slice(&(value as u64).to_be_bytes());
    }
    fn string(bytes: &mut Vec<u8>, value: &str) {
        u32(bytes, value.len() as u32 + 1);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    fn property(bytes: &mut Vec<u8>, kind: u32, payload: &[u8]) {
        u32(bytes, kind);
        u32(bytes, payload.len() as u32);
        bytes.extend_from_slice(payload);
    }
    fn end(bytes: &mut Vec<u8>) {
        u32(bytes, 0);
        u32(bytes, 0);
    }

    let mut bytes = b"gimp xcf v011\0".to_vec();
    for value in [4, 3, 0, 150] {
        u32(&mut bytes, value);
    }
    property(&mut bytes, 17, &[0]);
    end(&mut bytes);
    let slots = [slot(&mut bytes), slot(&mut bytes), slot(&mut bytes)];
    ptr(&mut bytes, 0);
    ptr(&mut bytes, 0);
    for (ordinal, name, kind, x, y, samples) in [
        (0, "Floating coverage", 3, -1i32, 1i32, vec![96, 128]),
        (1, "Source resource", 1, 0, 1, vec![255, 0, 0, 255]),
        (2, "Independent", 1, 2, 0, vec![0, 255, 0, 255]),
    ] {
        let at = bytes.len();
        patch(&mut bytes, slots[ordinal], at);
        for value in [1, 1, kind] {
            u32(&mut bytes, value);
        }
        string(&mut bytes, name);
        let mut offset = x.to_be_bytes().to_vec();
        offset.extend_from_slice(&y.to_be_bytes());
        property(&mut bytes, 15, &offset);
        property(&mut bytes, 9, &u32::from(ordinal < 2).to_be_bytes());
        if ordinal == 0 {
            property(&mut bytes, 5, &22u64.to_be_bytes());
        }
        if ordinal == 1 {
            let mut metadata = 3u32.to_be_bytes().to_vec();
            string(&mut metadata, locator);
            u32(&mut metadata, 1);
            u32(&mut metadata, 1);
            property(&mut bytes, 48, &metadata);
        }
        end(&mut bytes);
        let hierarchy = slot(&mut bytes);
        ptr(&mut bytes, 0);
        let at = bytes.len();
        patch(&mut bytes, hierarchy, at);
        for value in [1, 1, samples.len() as u32] {
            u32(&mut bytes, value);
        }
        let level = slot(&mut bytes);
        ptr(&mut bytes, 0);
        let at = bytes.len();
        patch(&mut bytes, level, at);
        u32(&mut bytes, 1);
        u32(&mut bytes, 1);
        let tile = slot(&mut bytes);
        ptr(&mut bytes, 0);
        let at = bytes.len();
        patch(&mut bytes, tile, at);
        bytes.extend_from_slice(&samples);
    }
    bytes
}

fn layer<'a>(session: &'a Session, name: &str) -> &'a Layer {
    session.active().unwrap().doc.layers.iter().find(|layer| layer.name == name).unwrap()
}

#[test]
fn imported_floating_and_legacy_links_use_native_movement_undo_and_reopen() {
    let bytes = source("../../unavailable-source.pcraft");
    let mut session = open(&bytes, "relationships.xcf");
    let floating = layer(&session, "Floating coverage").id;
    let resource = layer(&session, "Source resource").id;
    let independent = layer(&session, "Independent").clone();
    let before = session.active().unwrap().doc.clone();
    assert_eq!(before.layers.len(), 3);
    assert_eq!(before.mode, ColorMode::Rgb);
    assert_eq!(before.layer(floating).unwrap().link_group, Some(1));
    assert_eq!(before.layer(resource).unwrap().link_group, Some(1));
    assert!(photocraft_engine::link_watch::linked_files(&before).is_empty());
    let psd = photocraft_io::export(&before, "psd", &Default::default()).unwrap();
    let exported = photocraft_io::import("relationships.psd", &psd.bytes).unwrap().document;
    for source_layer in &before.layers {
        let native = exported.layers.iter().find(|layer| layer.name == source_layer.name).unwrap();
        assert_eq!(native.surface(), source_layer.surface());
        assert_eq!(native.link_group, source_layer.link_group);
    }
    session.execute("layer.translate", json!({"layer": floating.0, "dx": 2, "dy": -1})).unwrap();
    assert_eq!(layer(&session, "Floating coverage").surface().unwrap().content_bounds(), Rect::new(1, 0, 2, 1));
    assert_eq!(layer(&session, "Source resource").surface().unwrap().content_bounds(), Rect::new(2, 0, 3, 1));
    assert_eq!(layer(&session, "Independent"), &independent);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, before);
    session.execute("edit.redo", json!({})).unwrap();
    let expected = session.active().unwrap().doc.clone();
    let native = photocraft_format::save_to_bytes(&expected, &Default::default()).unwrap();
    let mut reopened = open(&native, "relationships.pcraft");
    assert!(!reopened.active().unwrap().source_read_only);
    assert_eq!(reopened.active().unwrap().doc.layers, expected.layers);
    assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, expected.metadata.import_conversion);
    reopened.execute("layer.setProps", json!({"layer": floating.0, "opacity": 0.25})).unwrap();
    reopened.execute("edit.undo", json!({})).unwrap();
    assert_eq!(reopened.active().unwrap().doc.layers, expected.layers);
}

#[test]
fn source_locator_does_not_authorize_reads_or_watch_but_explicit_native_relink_does() {
    static DENIED: AtomicUsize = AtomicUsize::new(0);
    fn deny_resource_path(id: &str, params: &serde_json::Value) -> photocraft_engine::Result<()> {
        if id == "layer.smartObjects.relinkToFile" && params.get("path").is_some() {
            DENIED.fetch_add(1, Ordering::Relaxed);
            return Err(photocraft_engine::EngineError::Other("resource path denied by test authority".into()));
        }
        Ok(())
    }
    let files = Files::new();
    files.write_resource([0.0, 0.0, 1.0, 1.0]);
    let bytes = source(files.resource().to_str().unwrap());
    let mut session = open(&bytes, "relationships.xcf");
    let id = layer(&session, "Source resource").id;
    assert!(matches!(layer(&session, "Source resource").content, LayerContent::Raster(_)));
    assert_eq!(layer(&session, "Source resource").surface().unwrap().read_region(Rect::new(0, 1, 1, 2)), vec![1.0, 0.0, 0.0, 1.0]);
    let before = session.active().unwrap().doc.clone();
    assert!(photocraft_engine::link_watch::linked_files(&before).is_empty());
    assert_eq!(session.execute("layer.smartObjects.updateChanged", json!({})).unwrap()["updated"], json!([]));
    files.write_resource([0.0, 1.0, 1.0, 1.0]);
    assert_eq!(session.execute("layer.smartObjects.updateChanged", json!({})).unwrap()["updated"], json!([]));
    assert_eq!(session.active().unwrap().doc, before);
    let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
    std::fs::remove_file(files.resource()).unwrap();
    let reopened = open(&native, "relationships.pcraft");
    assert!(photocraft_engine::link_watch::linked_files(&reopened.active().unwrap().doc).is_empty());
    assert_eq!(reopened.active().unwrap().doc.layers, before.layers);
    files.write_resource([0.0, 1.0, 1.0, 1.0]);
    let new_id = session.execute("layer.smartObjects.convertToSmartObject", json!({"layer": id.0})).unwrap()["layer"].as_u64().unwrap();
    let before_denied = session.active().unwrap().doc.clone();
    session.authorize = Some(deny_resource_path);
    assert!(session.execute("layer.smartObjects.relinkToFile", json!({"layer": new_id, "path": files.resource()})).is_err());
    assert_eq!(DENIED.load(Ordering::Relaxed), 1);
    assert_eq!(session.active().unwrap().doc, before_denied);
    session.authorize = None;
    session.execute("layer.smartObjects.relinkToFile", json!({"layer": new_id, "path": files.resource()})).unwrap();
    let LayerContent::Smart(smart) = &session.active().unwrap().doc.layer(LayerId(new_id)).unwrap().content else { panic!("native smart") };
    assert!(matches!(&smart.source, SmartSource::Linked { path } if path == files.resource().to_str().unwrap()));
    assert_eq!(smart.cache.as_ref().unwrap().read_region(Rect::new(0, 1, 1, 2)), vec![0.0, 1.0, 1.0, 1.0]);
    let linked = session.active().unwrap().doc.clone();
    files.write_resource([0.0, 0.0, 1.0, 1.0]);
    session.execute("layer.smartObjects.updateModifiedContent", json!({"layer": new_id})).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, linked);
}
