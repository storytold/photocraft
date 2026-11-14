//! Package: a layer named after a link whose copy got a new name takes the new base name, in
//! the packaged document only (#14079).

use super::*;

/// (layer name, linked file's file name) of every linked layer, top to bottom.
fn linked_layers(doc: &photocraft_doc::Document) -> Vec<(String, String)> {
    doc.walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Linked { path } => Some((l.name.clone(), file_name(path))),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn base(file: &str) -> String {
    file.rsplit_once('.').map_or(file, |(b, _)| b).to_string()
}

/// Two `logo.png` files (folders `a` and `b`), layers `logo a`, `logo a again`, `logo b` and
/// `other` (a second placement of `b/logo.png` whose name does not start with `logo`).
fn package_the_document(tag: &str) -> (String, Session, serde_json::Value) {
    let dir = {
        let d = std::env::temp_dir().join(format!("photocraft-pkg-rename-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().replace('\\', "/")
    };
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 90, "name": "Print Me.psd", "background": "#3366cc"})).unwrap();
    let path_of = |sub: &str, colour: &str| {
        std::fs::create_dir_all(format!("{dir}/{sub}")).unwrap();
        let link = format!("{dir}/{sub}/logo.png");
        let mut src = Session::new();
        src.execute("file.new", json!({"width": 4, "height": 4, "background": colour})).unwrap();
        crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
        link
    };
    let a = path_of("a", "#ff0000");
    let b = path_of("b", "#00ff00");
    for (link, name) in [(&a, "logo a"), (&a, "logo a again"), (&b, "logo b"), (&b, "other")] {
        s.execute("file.placeLinked", json!({"path": link})).unwrap();
        s.execute("layer.renameLayer", json!({"name": name})).unwrap();
    }
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    (dir, s, r)
}

#[test]
fn a_layer_named_after_a_renamed_link_takes_the_new_base_name() {
    let (dir, s, r) = package_the_document("rename");
    let docp = r["document"].as_str().unwrap();
    let back = photocraft_io::import("Print Me.pcraft", &std::fs::read(docp).unwrap()).unwrap().document;
    let packaged = linked_layers(&back);
    let open = linked_layers(&s.active().unwrap().doc);
    assert_eq!(packaged.len(), 4, "{packaged:?}");
    let by_name = |list: &[(String, String)], name: &str| list.iter().find(|(n, _)| n.as_str() == name).cloned();
    // Whichever copy was renamed, it is the one the clash gave a new file name.
    let renamed: Vec<&(String, String)> = packaged.iter().filter(|(_, f)| base(f) != "logo").collect();
    assert!(!renamed.is_empty(), "one link's copy got a new name: {packaged:?}");
    let new_file = &renamed[0].1;
    let new_base = base(new_file);
    assert_ne!(new_base, "logo");
    // The renamed link is `b/logo.png`: its layers are `logo b` and `other`.
    assert_eq!(by_name(&packaged, &format!("{new_base} b")).map(|(_, f)| f), Some(new_file.clone()), "{packaged:?}");
    assert!(by_name(&packaged, "logo b").is_none(), "{packaged:?}");
    // A layer named otherwise, linked to the renamed copy, keeps its name.
    assert_eq!(by_name(&packaged, "other").map(|(_, f)| f), Some(new_file.clone()), "{packaged:?}");
    // Layers whose link kept its name keep theirs.
    for name in ["logo a", "logo a again"] {
        assert_eq!(by_name(&packaged, name).map(|(_, f)| f), Some("logo.png".to_string()), "{name}: {packaged:?}");
    }
    // The open document is unchanged.
    let names: Vec<&str> = open.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["logo a", "logo a again", "logo b", "other"] {
        assert!(names.contains(&want), "{want}: {open:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
