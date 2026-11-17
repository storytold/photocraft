//! Layers to Files: a layer skipped by Visible Layers Only keeps its number (#13959).

use super::*;

/// Layers top to bottom: `Top`, `Hidden` (hidden), `Blue`, `Background`.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    for name in ["Blue", "Hidden", "Top"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        if name == "Hidden" {
            s.execute("layer.setProps", json!({"visible": false})).unwrap();
        }
    }
    s
}

fn export(s: &Session, visible_only: bool) -> LayerExportReport {
    export_layers_report_with(s, &json!({"prefix": "doc", "visibleOnly": visible_only}), |_, _| Ok(())).unwrap()
}

#[test]
fn visible_only_leaves_the_hidden_layers_number_unused() {
    let report = export(&session(), true);
    assert_eq!(report.files, ["doc_0000_Top.png", "doc_0002_Blue.png", "doc_0003_Background.png"]);
    let skipped: Vec<_> = report.skipped.iter().map(|l| (l.name.as_str(), l.reason)).collect();
    assert_eq!(skipped, [("Hidden", "hidden by visibleOnly")]);
}

#[test]
fn numbering_without_visible_only_is_unchanged() {
    let report = export(&session(), false);
    assert_eq!(report.files, ["doc_0000_Top.png", "doc_0001_Hidden.png", "doc_0002_Blue.png", "doc_0003_Background.png"]);
    assert!(report.skipped.is_empty());
}

/// Recorded Export Layers to Files counts art layers and groups independently at each level.
#[test]
fn recursive_layers_to_files_numbers_hidden_siblings_and_nested_groups() {
    let mut s = session();
    s.edit("Groups", |doc, _| {
        let fmt = doc.pixel_format();
        let mut hidden = Layer::group("Hidden group", vec![Layer::raster("Hidden child", fmt)]);
        hidden.visible = false;
        let nested = Layer::group("Nested", vec![Layer::raster("Deep", fmt)]);
        doc.layers.insert(2, Layer::group("Group", vec![nested, Layer::raster("Child", fmt)]));
        doc.layers.push(hidden);
        Ok(())
    })
    .unwrap();
    let report = export_layers_report_with(&s, &json!({"prefix":"doc","groups":"recursive"}), |_, _| Ok(())).unwrap();
    assert_eq!(
        report.files,
        ["doc_0000_Top.png", "doc_0002_Blue.png", "doc_0003_Background.png", "doc_0001s_0000_Child.png", "doc_0001s_0000s_0000_Deep.png",]
    );
    let all = export_layers_report_with(&s, &json!({"prefix":"doc","groups":"recursive","visibleOnly":false}), |_, _| Ok(())).unwrap();
    assert!(all.files.contains(&"doc_0000s_0000_Hidden child.png".into()));
    assert!(all.files.contains(&"doc_0001_Hidden.png".into()));
}

#[test]
fn recursive_layers_to_files_keeps_parent_opacity_masks_and_clipping() {
    use photocraft_color::SampleType;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = session();
        s.edit("Context", |doc, _| {
            doc.depth = depth;
            let mut leaf = Layer::raster("Child", doc.pixel_format());
            leaf.surface_mut().unwrap().write_region(Rect::new(0, 0, 4, 4), &[1.0; 4 * 4 * 4]);
            let mut group = Layer::group("Group", vec![leaf.clone()]);
            group.opacity = 0.5;
            let mut clipped = leaf.clone();
            clipped.id = LayerId::fresh();
            clipped.name = "Clipped".into();
            clipped.clipped = true;
            leaf.id = LayerId::fresh();
            let mut masked = Layer::group("Masked", vec![leaf]);
            masked.mask = Some(photocraft_doc::LayerMask::hide_all());
            doc.layers = vec![group, clipped, masked];
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let mut images = std::collections::BTreeMap::new();
        export_layers_report_with(&s, &json!({"prefix":"doc","groups":"recursive"}), |name, bytes| {
            images.insert(name.to_string(), photocraft_codecs::decode(bytes).unwrap().to_rgba8());
            Ok(())
        })
        .unwrap();
        let opacity = &images["doc_0001s_0000_Child.png"];
        assert!((126..=129).contains(&opacity[3]), "parent opacity at {depth:?}: {opacity:?}");
        assert_eq!(images["doc_0000_Clipped.png"][3], 0, "a hidden clipping base must not be replaced by unclipped pixels");
        assert_eq!(images["doc_0000s_0000_Child.png"][3], 0, "parent mask must survive isolation");
        assert_eq!(s.active().unwrap().doc, before, "export must not change the session document");
    }
}

#[test]
fn recursive_layers_to_files_rejects_bad_modes_before_writing() {
    for mode in [json!(true), json!("flatten"), json!(1)] {
        let mut writes = 0;
        assert!(
            export_layers_report_with(&session(), &json!({"groups":mode}), |_, _| {
                writes += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(writes, 0);
    }
}

#[test]
fn recursive_layers_to_files_bounds_hidden_group_nesting_before_writing() {
    let mut s = session();
    s.edit("Deep groups", |doc, _| {
        let mut layer = Layer::raster("Leaf", doc.pixel_format());
        for _ in 0..66 {
            layer = Layer::group("Group", vec![layer]);
        }
        layer.visible = false;
        doc.layers.push(layer);
        Ok(())
    })
    .unwrap();
    let mut writes = 0;
    assert!(
        export_layers_report_with(&s, &json!({"groups":"recursive"}), |_, _| {
            writes += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(writes, 0);
}

#[test]
fn recursive_layers_to_files_exports_adjustment_and_empty_art_layers() {
    let mut s = session();
    s.edit("Empty layers", |doc, _| {
        doc.layers = vec![Layer::raster("Empty", doc.pixel_format()), Layer::new("Adjustment", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert))];
        Ok(())
    })
    .unwrap();
    let report = export_layers_report_with(&s, &json!({"prefix":"doc","groups":"recursive","format":"jpg"}), |_, bytes| {
        let rgba = photocraft_codecs::decode(bytes).unwrap().to_rgba8();
        assert!(rgba.iter().all(|v| *v == 255), "isolated empty/adjustment layers are white JPEGs");
        Ok(())
    })
    .unwrap();
    assert_eq!(report.files, ["doc_0000_Adjustment.jpg", "doc_0001_Empty.jpg"]);
    assert!(report.skipped.is_empty());
}

#[test]
fn recursive_layers_to_files_names_keep_recorded_punctuation_and_limit_the_whole_stem() {
    for (layer, expected) in [
        (" a+b!(x) &Z-_.' ", "Probe_0005_ a+b!(x) &Z-_.' .jpg"),
        ("A:B/C\\D*E?F\"G<H>I|J", "Probe_0005_A_B_C_D_E_F_G_H_I_J.jpg"),
        (".dots.", "Probe_0005_.dots..jpg"),
        ("trailing. ", "Probe_0005_trailing. .jpg"),
    ] {
        assert_eq!(recursive_filename("Probe", 5, layer, "jpg").unwrap(), expected);
    }
    assert_eq!(recursive_filename("Probe", 1, &"L".repeat(160), "jpg").unwrap(), format!("Probe_0001_{}.jpg", "L".repeat(109)));
    assert!(recursive_filename("Probe", 0, "bad\nname", "jpg").is_err());
    let mut writes = 0;
    assert!(
        export_layers_report_with(&session(), &json!({"groups":"recursive","prefix":"P".repeat(121)}), |_, _| {
            writes += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(writes, 0, "truncation collisions must be detected before the first write");
}
