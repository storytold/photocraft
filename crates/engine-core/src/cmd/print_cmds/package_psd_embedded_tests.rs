//! PSD-embedded smart objects are represented as Linked UUID sources, but their bytes live in the
//! document's preserved `lnk2` blocks. Package must leave those sources in the document rather
//! than treating UUIDs as missing files.

use super::*;
use photocraft_doc::{Affine, Layer, SmartObject, SmartSource};
use std::sync::Arc;

fn temp_dir() -> String {
    let path = std::env::temp_dir().join(format!("photocraft-package-psd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn package_skips_psd_embedded_sources_but_collects_external_links() {
    let dir = temp_dir();
    let external = format!("{dir}/external.png");
    let mut source = crate::session();
    source.execute("file.new", json!({"width": 4, "height": 4, "background": "#336699"})).unwrap();
    crate::file_cmds::save_doc(&source.active().unwrap().doc, &external, None).unwrap();

    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "#ffffff"})).unwrap();
    s.edit("PSD embedded and external links", |doc, active| {
        let embedded = photocraft_io::linked::LinkedFile { uuid: "embedded-uuid".into(), file_name: "embedded.pcraft".into(), bytes: vec![1, 2, 3] };
        let block = photocraft_io::linked::encode_linked_file(&embedded);
        doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(block)));
        let embedded_layer = Layer::new("Embedded", LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: embedded.uuid }, Affine::IDENTITY, None)));
        let external_layer =
            Layer::new("External", LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: external.clone() }, Affine::IDENTITY, None)));
        let parent = active.unwrap();
        let embedded_id = doc.insert_above(Some(parent), embedded_layer);
        doc.insert_above(Some(embedded_id), external_layer);
        Ok(())
    })
    .unwrap();

    let result = s.execute("file.package", json!({"dir": format!("{dir}/out")})).unwrap();
    assert_eq!(result["missing"], json!([]));
    assert_eq!(result["links"].as_array().unwrap().len(), 1);
    assert!(result["links"][0].as_str().unwrap().ends_with("Links/external.png"));

    let packaged_path = result["document"].as_str().unwrap();
    let packaged = photocraft_io::import("packaged.pcraft", &std::fs::read(packaged_path).unwrap()).unwrap().document;
    let embedded_sources: Vec<String> = packaged
        .walk()
        .iter()
        .filter_map(|(_, _, layer)| match &layer.content {
            LayerContent::Smart(smart) => match &smart.source {
                SmartSource::Linked { path } if path == "embedded-uuid" => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(embedded_sources, vec!["embedded-uuid"]);
}
