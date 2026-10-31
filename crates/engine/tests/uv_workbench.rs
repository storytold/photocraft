#![cfg(feature = "uv-workbench")]
use photocraft_color::SampleType;
use photocraft_engine::Session;
use serde_json::json;

fn scene() -> Session {
    let mut s = Session::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
    let result = s.execute("uv.model.import", json!({"path":path.to_str().unwrap()})).unwrap();
    assert_eq!(result["triangles"], 4);
    assert!(s.active().is_some());
    s
}
#[test]
fn project_resave_retains_view_paths_and_refuses_bad_metadata_without_replacing_file() {
    let mut s = scene();
    let source = s.active().unwrap().path.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repeat.pcuv");
    s.execute("uv.project.save", json!({"path":path,"view":{"settings":{"yaw":0.5}}})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    let sky = dir.path().join("sky.hdr");
    std::fs::write(&sky, b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 1\n\x80\x80\x80\x81").unwrap();
    let view = json!({"settings":{"yaw":1.5,"lighting":{"brightness":2.0}},"environment":sky});
    s.execute("uv.project.save", json!({"path":path,"view":view})).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(s.execute("uv.project.save", json!({"path":path,"view":{"oversized":"x".repeat(65537)}})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut restored = Session::new();
    let result = restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert_eq!(result["view"], view);
    assert_eq!(restored.active().unwrap().path, source);
    let pixel = photocraft_compose::render(&restored.active().unwrap().doc, photocraft_geom::Rect::from_xywh(2, 2, 1, 1)).px[0];
    assert!(pixel[0] > 0.9 && pixel[1] < 0.1);
    restored.execute("uv.project.save", json!({"path":path})).unwrap();
    assert_eq!(restored.execute("uv.project.load", json!({"path":path})).unwrap()["view"], view);
}
#[test]
#[ignore = "read-only external project diagnostic; set PHOTOCRAFT_UV_PROJECT"]
fn external_project_connections_can_be_saved_twice_and_reopened() {
    let source = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_UV_PROJECT").unwrap());
    let folder = source.parent().unwrap();
    let mut s = Session::new();
    s.execute("uv.project.load", json!({"path":source})).unwrap();
    for (material, prefix) in [(0, "Pants"), (1, "Jacket")] {
        for (channel, suffix) in [
            ("baseColor", "Base_color"),
            ("normal", "Normal_OpenGL"),
            ("ao", "Mixed_AO"),
            ("metallic", "Metallic"),
            ("roughness", "Roughness"),
            ("emissive", "Emissive"),
        ] {
            let path = folder.join(format!("{prefix}_{suffix}.png"));
            s.execute("uv.texture.open", json!({"path":path,"material":material,"channel":channel})).unwrap();
        }
    }
    let count = s.documents().len();
    let path = std::env::temp_dir().join(format!("uv-external-project-{}.pcuv", std::process::id()));
    let view = json!({"settings":{"yaw":1.5,"lighting":{"brightness":2.0}},"environment":folder.join("smelting_tower_01_4k.exr")});
    s.execute("uv.project.save", json!({"path":path,"view":view})).unwrap();
    println!("Saved complete project: {} MiB, {count} editable documents", std::fs::metadata(&path).unwrap().len() / (1024 * 1024));
    s.execute("layer.new.layer", json!({"name":"Repeat save test"})).unwrap();
    s.execute("uv.project.save", json!({"path":path,"view":view})).unwrap();
    drop(s);
    let mut restored = Session::new();
    let result = restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert_eq!(restored.documents().len(), count);
    assert_eq!(result["view"], view);
    assert!(restored.active().unwrap().doc.layers.iter().any(|layer| layer.name == "Repeat save test"));
    assert_eq!(restored.uv.all_bindings().flatten().count(), 12);
    std::fs::remove_file(path).unwrap();
}
fn choose(s: &mut Session) {
    let material = s.uv.material;
    let shell = s.uv.topology.as_ref().unwrap().shells.iter().position(|sh| sh.material == material).unwrap();
    s.execute("uv.shell.select", json!({"material":material,"shells":[shell]})).unwrap();
}

#[test]
fn textures_auto_open_and_uv_selection_is_one_undo_step() {
    let mut s = scene();
    choose(&mut s);
    assert!(s.active().unwrap().doc.selection.is_none());
    s.execute("uv.mask.apply", json!({"guard":8})).unwrap();
    let doc = s.active().unwrap();
    let selection = doc.doc.selection.as_ref().unwrap();
    assert_eq!(selection.read_region(photocraft_geom::Rect::new(52, 64, 53, 65)), vec![0.0]);
    assert_eq!(selection.read_region(photocraft_geom::Rect::new(30, 64, 31, 65)), vec![1.0]);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
    s.execute("edit.redo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
}

#[test]
fn zero_coverage_is_explicit_and_blocks_destructive_fill() {
    let mut s = scene();
    s.execute("uv.mask.apply", json!({})).unwrap();
    let before = photocraft_compose::render(&s.active().unwrap().doc, s.active().unwrap().doc.bounds()).px;
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(before, photocraft_compose::render(doc, doc.bounds()).px);
}

#[test]
fn fill_changes_shell_interior_but_not_seam() {
    let mut s = scene();
    choose(&mut s);
    s.execute("uv.mask.apply", json!({"guard":8})).unwrap();
    let before = photocraft_compose::render(&s.active().unwrap().doc, s.active().unwrap().doc.bounds()).px;
    // Imported flat images may have a locked Background: unlock through the existing command.
    s.execute("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let after = photocraft_compose::render(doc, doc.bounds()).px;
    assert_eq!(before[64 * 128 + 52], after[64 * 128 + 52]);
    assert_ne!(before[64 * 128 + 30], after[64 * 128 + 30]);
}

#[test]
fn protected_layer_mask_survives_deselection_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = scene();
        let material = s.uv.material;
        s.execute("uv.texture.create", json!({"material":material,"width":128,"height":128,"depth":depth})).unwrap();
        choose(&mut s);
        let before_layers = s.active().unwrap().doc.layers.len();
        s.execute("uv.layer.create", json!({"guard":8})).unwrap();
        let st = s.active().unwrap();
        assert!(st.doc.selection.is_none());
        assert_eq!(st.doc.layers.len(), before_layers + 1);
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let expected = match depth {
            8 => SampleType::U8,
            16 => SampleType::U16,
            _ => SampleType::F32,
        };
        assert_eq!(layer.mask.as_ref().unwrap().surface.format().sample, expected);
        s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
        let doc = &s.active().unwrap().doc;
        let px = photocraft_compose::render(doc, doc.bounds()).px;
        assert_eq!(px[64 * 128 + 52], [1.0; 4]);
        assert!(px[64 * 128 + 30][1] < 0.01);
    }
}

#[test]
fn project_roundtrip_keeps_scene_layers_masks_and_selection() {
    let mut s = scene();
    choose(&mut s);
    s.execute("uv.layer.create", json!({"guard":8})).unwrap();
    let material = s.uv.material;
    let duplicate = s.uv.scene.as_ref().unwrap().materials[material].clone();
    std::sync::Arc::make_mut(s.uv.scene.as_mut().unwrap()).materials.push(duplicate);
    s.uv.bindings.push(s.uv.bindings[material]);
    s.uv.normal_bindings.push(None);
    let path = std::env::temp_dir().join(format!("photocraft-uv-roundtrip-{}.pcuv", std::process::id()));
    {
        s.execute("uv.project.save", json!({"path":path.to_str().unwrap()})).unwrap();
        let mut restored = Session::new();
        restored.execute("uv.project.load", json!({"path":path.to_str().unwrap()})).unwrap();
        assert_eq!(restored.uv.scene.as_ref().unwrap().triangles.len(), 4);
        assert_eq!(restored.uv.selected, s.uv.selected);
        assert_eq!(restored.active().unwrap().doc.layers.len(), 2);
        assert!(restored.active().unwrap().doc.layers.last().unwrap().mask.is_some());
        assert_eq!(restored.uv.bindings[material], *restored.uv.bindings.last().unwrap());
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn invalid_params_preserve_documents() {
    let mut s = scene();
    let count = s.documents().len();
    let revision = s.active().unwrap().revision;
    for (id, p) in [
        ("uv.shell.select", json!({"shells":[999999]})),
        ("uv.texture.create", json!({"width":4294967295u64,"height":4294967295u64})),
        ("uv.mask.apply", json!({"guard":-1})),
        ("uv.mask.apply", json!({"horizontal":-1})),
        ("uv.settings", json!({"vertical":1025})),
        ("uv.settings", json!({"horizontal":"bad"})),
        ("uv.project.load", json!({"path":"this-file-does-not-exist.pcuv"})),
        ("uv.texture.bind", json!({"material":"wrong type"})),
    ] {
        assert!(s.execute(id, p).is_err(), "{id}");
        assert_eq!(s.documents().len(), count);
        assert_eq!(s.active().unwrap().revision, revision);
    }
}

#[test]
fn oversized_uv_file_is_refused_before_buffering_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oversized.fbx");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(photocraft_uv::MAX_FILE_BYTES + 1).unwrap();
    let mut s = Session::new();
    let error = s.execute("uv.model.import", json!({"path":path.to_string_lossy()})).unwrap_err();
    assert!(error.to_string().contains("256 MiB"));
    while !s.documents().is_empty() {
        let _ = s.close(0);
    }
    assert!(s.uv.scene.is_none());
}

#[test]
fn normal_channel_protection_visibility_and_project_roundtrip() {
    let mut s = scene();
    let material = s.uv.material;
    let base = s.uv.bindings[material];
    s.execute("uv.texture.create", json!({"channel":"normal","width":128,"height":128,"depth":16})).unwrap();
    let normal = s.uv.normal_bindings[material].unwrap();
    assert_ne!(Some(normal), base);
    assert!(s.uv.normal_channel);
    assert_eq!(s.active().unwrap().doc.depth, SampleType::U16);
    let pixel = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(32, 64);
    assert!((pixel[0] - 0.5).abs() < 0.0001 && pixel[2] == 1.0);
    choose(&mut s);
    s.execute("uv.mask.apply", json!({"guard":8})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
    assert_eq!(s.active().unwrap().doc.id, normal);
    s.execute("uv.material.visible", json!({"material":material,"visible":false})).unwrap();
    assert!(s.uv.hidden_materials.contains(&material));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("normal.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert!(restored.uv.normal_channel);
    assert!(restored.uv.hidden_materials.contains(&material));
    assert!(restored.uv.normal_bindings[material].is_some());
    assert_ne!(restored.uv.normal_bindings[material], restored.uv.bindings[material]);
    assert_eq!(restored.active().unwrap().doc.depth, SampleType::U16);
    assert!(restored.active().unwrap().doc.selection.is_some());
    let count = restored.documents().len();
    for (id, p) in [
        ("uv.material.visible", json!({"visible":"bad"})),
        ("uv.material.select", json!({"channel":"bad"})),
        ("uv.texture.create", json!({"channel":false})),
        ("uv.texture.bind", json!({"channel":"bad"})),
        ("uv.material.visible", json!({"material":999999})),
    ] {
        assert!(restored.execute(id, p).is_err());
        assert_eq!(count, restored.documents().len());
    }
}

#[test]
fn older_base_color_only_project_loads_with_default_normal_and_visibility_state() {
    let mut s = scene();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let zip = photocraft_format::zip::ZipReader::new(&bytes).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&zip.read_by_name("project.json", 128 * 1024 * 1024).unwrap()).unwrap();
    let obj = manifest.as_object_mut().unwrap();
    for key in ["normal_documents", "hidden_materials", "normal_channel"] {
        obj.remove(key);
    }
    for m in manifest["scene"]["materials"].as_array_mut().unwrap() {
        m.as_object_mut().unwrap().remove("normal_map");
    }
    for t in manifest["scene"]["triangles"].as_array_mut().unwrap() {
        for c in t["corners"].as_array_mut().unwrap() {
            c.as_object_mut().unwrap().remove("normal");
        }
    }
    let mut writer = photocraft_format::zip::ZipWriter::new();
    for entry in &zip.entries {
        if entry.name == "project.json" {
            writer.add(&entry.name, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        } else {
            writer.add(&entry.name, &zip.read(entry, 256 * 1024 * 1024).unwrap()).unwrap();
        }
    }
    std::fs::write(&path, writer.finish().unwrap()).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert!(restored.uv.normal_bindings.iter().all(Option::is_none));
    assert!(!restored.uv.normal_channel);
    assert!(restored.uv.hidden_materials.is_empty());
    assert!(restored.active().is_some());
}

#[test]
fn fbx_normal_map_auto_opens_as_numeric_rgb_document() {
    let mut s = Session::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/normals-reflected.fbx");
    s.execute("uv.model.import", json!({"path":path})).unwrap();
    let id = s.uv.normal_bindings.iter().find_map(|id| *id).unwrap();
    let doc = &s.documents().iter().find(|d| d.doc.id == id).unwrap().doc;
    assert_eq!(doc.mode, photocraft_color::ColorMode::Rgb);
    let p = photocraft_compose::render_reduced(doc, 1, 1).px[0];
    assert!((p[0] - 128.0 / 255.0).abs() < 1e-5 && (p[1] - 128.0 / 255.0).abs() < 1e-5 && p[2] > 0.999);
}

#[test]
fn margins_roundtrip_and_deselect_clears_both_map_selections() {
    let mut s = scene();
    choose(&mut s);
    s.execute("uv.settings", json!({"guard":3,"horizontal":5,"vertical":7,"padding":4})).unwrap();
    s.execute("uv.mask.apply", json!({})).unwrap();
    let color = s.active().unwrap().doc.id;
    s.execute("uv.texture.create", json!({"channel":"normal","width":128,"height":128})).unwrap();
    choose(&mut s);
    s.execute("uv.mask.apply", json!({})).unwrap();
    let normal = s.active().unwrap().doc.id;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("margins.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    assert_eq!((restored.uv.guard, restored.uv.horizontal, restored.uv.vertical, restored.uv.padding), (3.0, 5.0, 7.0, 4.0));
    s.execute("select.deselect", json!({})).unwrap();
    assert!(s.uv.selected.is_empty());
    assert!(s.documents().iter().filter(|d| [color, normal].contains(&d.doc.id)).all(|d| d.doc.selection.is_none()));
    assert_eq!(s.active().unwrap().doc.id, normal);
    restored.execute("select.deselect", json!({})).unwrap();
    assert!(restored.documents().iter().all(|d| d.doc.selection.is_none()));
}

#[test]
fn deselect_preserves_a_manually_replaced_selection_after_project_roundtrip() {
    let mut s = scene();
    choose(&mut s);
    s.execute("uv.mask.apply", json!({})).unwrap();
    let material = s.uv.material;
    let color = s.active().unwrap().doc.id;
    s.execute("uv.texture.create", json!({"channel":"normal","width":128,"height":128})).unwrap();
    choose(&mut s);
    s.execute("uv.mask.apply", json!({})).unwrap();
    let index = s.documents().iter().position(|d| d.doc.id == color).unwrap();
    s.set_active(index);
    s.execute("select.rect", json!({"x":0,"y":0,"width":8,"height":8})).unwrap();
    let manual = s.active().unwrap().doc.selection.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manual.pcuv");
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.project.load", json!({"path":path})).unwrap();
    restored.execute("select.deselect", json!({})).unwrap();
    let color = restored.uv.bindings[material].unwrap();
    assert_eq!(restored.documents().iter().find(|d| d.doc.id == color).unwrap().doc.selection, manual);
    assert!(restored.active().unwrap().doc.selection.is_none());
}

#[test]
fn deselect_shells_without_a_texture_document() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/normals-reflected.fbx");
    let mut s = Session::new();
    s.execute("uv.model.import", json!({"path":path})).unwrap();
    while !s.documents().is_empty() {
        let _ = s.close(0);
    }
    let material = s.uv.topology.as_ref().unwrap().shells[0].material;
    s.execute("uv.shell.select", json!({"material":material,"shells":[0]})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    assert!(s.uv.selected.is_empty());
}

#[test]
#[ignore = "read-only user asset diagnostic, set PHOTOCRAFT_UV_IMPORT_MODEL"]
fn import_external_model_diagnostic() {
    let path = std::env::var("PHOTOCRAFT_UV_IMPORT_MODEL").unwrap();
    let mut s = Session::new();
    let result = s.execute("uv.model.import", json!({"path":path})).unwrap();
    println!("{result}");
    for st in s.documents() {
        println!("TEXTURE {} {:?} {:?} {}x{}", st.doc.name, st.doc.mode, st.doc.depth, st.doc.size.width, st.doc.size.height);
    }
}
#[test]
fn all_map_slots_roundtrip_and_reimport_preserve_editable_documents() {
    use photocraft_uv::preview::Channel;
    let mut s = scene();
    let material = s.uv.material;
    for kind in Channel::MAPS {
        s.execute("uv.texture.create", json!({"channel":kind.key(),"width":16,"height":16})).unwrap();
        assert_eq!(s.uv.map_bindings(kind)[material], Some(s.active().unwrap().doc.id));
    }
    let path = std::env::temp_dir().join(format!("uv-maps-{}.pcuv", std::process::id()));
    s.execute("uv.project.save", json!({"path":path})).unwrap();
    s.execute("uv.project.load", json!({"path":path})).unwrap();
    assert_eq!(s.uv.current_channel(), Channel::Emissive);
    for kind in Channel::MAPS {
        assert!(s.uv.map_bindings(kind)[material].is_some());
    }
    let ids: Vec<_> = s.uv.all_bindings().copied().collect();
    s.execute("uv.model.reimport", json!({})).unwrap();
    assert_eq!(ids, s.uv.all_bindings().copied().collect::<Vec<_>>());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tangent_wrap_native_brush_crosses_seam_one_undo_and_preserves_selection() {
    let mut s = scene();
    s.execute("uv.texture.create", json!({"width":128,"height":128,"depth":16})).unwrap();
    s.execute("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
    s.execute("tools.setBrush", json!({"brush":{"size":24.0,"hardness":1.0,"opacity":0.5}})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let scene = s.uv.scene.as_ref().unwrap();
    let tri = scene
        .triangles
        .iter()
        .position(|t| {
            t.corners.iter().all(|c| c.vertex < 4)
                && photocraft_uv::topology::barycentric(
                    [-0.02, 0.0],
                    t.corners.map(|c| {
                        let p = scene.positions[c.vertex];
                        [p[0], p[1]]
                    }),
                )
                .is_some()
        })
        .unwrap();
    for point in [[-0.02, 0.0, 0.0], [-0.02, 0.05, 0.0]] {
        s.execute("paint.uvWrap", json!({"triangle":tri,"position":point,"pressure":1.0,"coalesce":"wrap-test","target":"pixels"})).unwrap();
    }
    s.execute("paint.uvWrap", json!({"end":true})).unwrap();
    let doc = &s.active().unwrap().doc;
    let image = photocraft_compose::render(doc, doc.bounds());
    assert!(image.px[64 * 128 + 52][1] < 0.8, "left shell painted");
    assert!(image.px[64 * 128 + 75][1] < 0.8, "right shell painted across fold");
    assert!(image.px[64 * 128 + 64][1] > 0.99, "atlas gap untouched");
    assert!(image.px.iter().all(|p| p[1] >= 0.49), "stroke opacity ceiling retained");
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].surface(), before.layers[0].surface());
    s.execute("uv.mask.apply", json!({})).unwrap();
    let revision = s.active().unwrap().revision;
    let history = s.active().unwrap().history.past_len();
    s.execute("paint.uvWrap", json!({"triangle":tri,"position":[-0.02,0.0,0.0],"pressure":1.0,"coalesce":"empty-wrap-test","target":"pixels"})).unwrap();
    assert_eq!(s.active().unwrap().revision, revision, "fully masked stamp leaves revision unchanged");
    assert_eq!(s.active().unwrap().history.past_len(), history, "fully masked stamp adds no empty undo");
}
