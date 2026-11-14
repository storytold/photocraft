//! #13965: Layers to Files accepts `tiff` and writes `.tif`, with other extensions unchanged.

use super::{Session, export_layers_with, json};

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "l2f-test.psd", "width": 4, "height": 3, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Type"})).unwrap();
    s.execute("edit.fill", json!({"color": "#406080"})).unwrap();
    s
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn tiff_command_writes_readable_tif_files_at_each_depth() {
    for depth in [8, 16, 32] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(depth);
        let result = s.execute("file.export.layersToFiles", json!({"dir": dir.path(), "prefix": "l2f-test", "format": "tiff"})).unwrap();
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        for (file, expected) in files.iter().zip(["l2f-test_0000_Type.tif", "l2f-test_0001_Background.tif"]) {
            let path = std::path::Path::new(file.as_str().unwrap());
            assert_eq!(path.file_name().unwrap(), expected, "depth {depth}");
            let imported = photocraft_io::import("layer.tif", &std::fs::read(path).unwrap()).unwrap();
            let source = &s.active().unwrap().doc;
            assert_eq!(imported.document.size, source.size);
            assert_eq!(imported.document.depth, source.depth);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn tiff_aliases_use_tif_in_the_shared_export_path() {
    let s = session(8);
    for format in ["tiff", "tif", ".TIFF"] {
        let files = export_layers_with(&s, &json!({"prefix": "l2f-test", "format": format}), |name, bytes| {
            assert!(name.ends_with(".tif"), "format {format}: {name}");
            assert_eq!(photocraft_io::import(name, bytes).unwrap().document.size, s.active().unwrap().doc.size);
            Ok(())
        })
        .unwrap();
        assert_eq!(files, ["l2f-test_0000_Type.tif", "l2f-test_0001_Background.tif"]);
    }
}

#[test]
fn other_format_extensions_are_unchanged() {
    let s = session(8);
    for format in ["png", "jpg", "jpeg", "psd", "webp", "bmp", "tga", "pdf"] {
        let expected = [format!("l2f-test_0000_Type.{format}"), format!("l2f-test_0001_Background.{format}")];
        let files = export_layers_with(&s, &json!({"prefix": "l2f-test", "format": format}), |name, bytes| {
            assert!(expected.iter().any(|expected| expected == name));
            assert!(!bytes.is_empty());
            Ok(())
        })
        .unwrap();
        assert_eq!(files, expected);
    }
}
