use super::*;

fn linked_session() -> (Session, std::path::PathBuf, LayerId) {
    let path = contents_file();
    let mut s = session(8);
    paint(&mut s);
    let layer = LayerId(convert(&mut s));
    s.execute("layer.smartObjects.relinkToFile", json!({"path": path})).unwrap();
    (s, path, layer)
}

fn open_child(s: &mut Session, parent: usize, layer: LayerId) -> usize {
    s.set_active(parent);
    s.execute("layer.smartObjects.editContents", json!({"layer": layer.0})).unwrap();
    s.active_index().unwrap()
}

#[test]
fn competing_editors_cannot_overwrite_saved_contents() {
    let (mut s, path, layer) = linked_session();
    let other = (*s.active().unwrap().doc).clone();
    s.add_document(other, None);
    let a = open_child(&mut s, 0, layer);
    let b = open_child(&mut s, 1, layer);
    s.set_active(a);
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    let saved = std::fs::read(&path).unwrap();
    s.set_active(b);
    s.execute("layer.setProps", json!({"name": "B's unsaved edit"})).unwrap();
    let parents = [s.documents()[0].doc.clone(), s.documents()[1].doc.clone()];
    for command in ["layer.smartObjects.saveContents", "file.close", "file.closeAll"] {
        let err = s.execute(command, json!({})).unwrap_err().to_string();
        assert!(err.contains("linked file changed"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        assert_eq!(s.documents().len(), 4);
        assert!(s.documents()[b].is_dirty());
        assert_eq!(s.documents()[b].doc.layers[0].name, "B's unsaved edit");
        for (i, before) in parents.iter().enumerate() {
            assert!(Arc::ptr_eq(before, &s.documents()[i].doc));
        }
    }
    // The writer's own baseline advances: a second save must remain possible.
    s.set_active(a);
    s.execute("layer.setProps", json!({"opacity": 0.25})).unwrap();
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    let saved = decode_source("poster.pcraft", &std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.layers[0].opacity, 0.25);
    assert!(s.documents()[b].is_dirty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn external_replacement_with_same_size_cannot_be_overwritten() {
    let (mut s, path, layer) = linked_session();
    open_child(&mut s, 0, layer);
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let mut replacement = std::fs::read(&path).unwrap();
    replacement[0] ^= 1; // Equal length, different bytes; no decoding is needed to detect conflict.
    std::fs::write(&path, &replacement).unwrap();
    let parent = s.documents()[0].doc.clone();
    assert!(s.execute("layer.smartObjects.saveContents", json!({})).unwrap_err().to_string().contains("linked file changed"));
    assert_eq!(std::fs::read(&path).unwrap(), replacement);
    assert!(s.active().unwrap().is_dirty());
    assert!(Arc::ptr_eq(&parent, &s.documents()[0].doc));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn saving_refreshes_all_instances_and_keeps_each_placement_and_link() {
    for depth in DEPTHS {
        let (mut s, path, layer) = linked_session();
        let mut other = (*s.active().unwrap().doc).clone();
        other.depth = match depth {
            8 => photocraft_doc::SampleType::U8,
            16 => photocraft_doc::SampleType::U16,
            _ => photocraft_doc::SampleType::F32,
        };
        let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let sm = smart_mut(&mut other, layer).unwrap();
        sm.source = SmartSource::Linked { path: file_name.clone() };
        sm.transform = Affine::translate(5.0, 7.0);
        let mut sibling_sm = sm.clone();
        // A directory alias and .. still point at the same atomic-write target.
        let alias_dir = path.with_extension("alias-dir");
        std::fs::create_dir(&alias_dir).unwrap();
        let alias_path = format!("{}/../{file_name}", alias_dir.file_name().unwrap().to_str().unwrap());
        sibling_sm.source = SmartSource::Linked { path: alias_path.clone() };
        let sibling = Layer::new("second instance", LayerContent::Smart(sibling_sm));
        let sibling_id = sibling.id;
        other.layers.push(sibling);
        let other_path = path.parent().unwrap().join("mockup-B.pcraft").to_string_lossy().into_owned();
        s.add_document(other, Some(other_path));
        let child = open_child(&mut s, 0, layer);
        s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
        let revisions: Vec<_> = s.documents().iter().map(|d| d.revision).collect();
        s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
        assert_eq!(s.active_index(), Some(child));
        assert!(!s.active().unwrap().is_dirty());
        for (i, ids) in [(0, vec![layer]), (1, vec![layer, sibling_id])] {
            let state = &s.documents()[i];
            assert_eq!(state.revision, revisions[i] + 1, "one history step per parent");
            assert!(state.is_dirty());
            let reopened = decode_source("wall.pcraft", &encode_source(&state.doc).unwrap()).unwrap();
            for id in ids {
                let sm = smart(&state.doc, id).unwrap();
                assert_eq!(
                    sm.source,
                    SmartSource::Linked {
                        path: if i == 0 {
                            path.to_string_lossy().into_owned()
                        } else if id == sibling_id {
                            alias_path.clone()
                        } else {
                            file_name.clone()
                        }
                    }
                );
                assert_eq!(sm.transform, if i == 0 { Affine::translate(-6.0, 5.0) } else { Affine::translate(5.0, 7.0) });
                assert!(sm.cache.as_ref().unwrap().rgba(if i == 0 { -5 } else { 6 }, if i == 0 { 6 } else { 8 })[3] > 0.49);
                assert!(sm.cache.as_ref().unwrap().rgba(if i == 0 { -5 } else { 6 }, if i == 0 { 6 } else { 8 })[3] < 0.51);
                let saved_sm = smart(&reopened, id).unwrap();
                assert_eq!(saved_sm.source, sm.source);
                assert_eq!(saved_sm.cache.as_ref().unwrap().read_region(Rect::new(0, 0, W, H)), sm.cache.as_ref().unwrap().read_region(Rect::new(0, 0, W, H)));
            }
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(alias_dir).unwrap();
    }
}

#[test]
fn busy_shared_mockup_blocks_save_before_any_file_or_cache_changes() {
    let (mut s, path, layer) = linked_session();
    let other = (*s.active().unwrap().doc).clone();
    s.add_document(other, None);
    let (release, wait) = std::sync::mpsc::channel();
    let job = s
        .start_job(
            "test.busy",
            json!({}),
            "Hold mockup",
            true,
            move |_| {
                wait.recv().unwrap();
                Ok(())
            },
            |_, ()| Ok(Value::Null),
        )
        .unwrap();
    let crate::jobs::Started::Job(job) = job else { panic!("expected background job") };
    let child = open_child(&mut s, 0, layer);
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let before = std::fs::read(&path).unwrap();
    let parents = [s.documents()[0].doc.clone(), s.documents()[1].doc.clone()];
    let result = s.execute("layer.smartObjects.saveContents", json!({}));
    release.send(()).unwrap(); // Always release before asserting, including when the test fails.
    assert!(result.unwrap_err().to_string().contains("busy"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    for (i, parent) in parents.iter().enumerate() {
        assert!(Arc::ptr_eq(parent, &s.documents()[i].doc));
    }
    assert!(s.documents()[child].is_dirty());
    s.wait_job(job).unwrap();
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn explicit_discard_closes_linked_or_embedded_contents_without_saveback() {
    for linked in [false, true] {
        let (mut s, path, layer) = linked_session();
        if !linked {
            s.execute("layer.smartObjects.convertToEmbedded", json!({})).unwrap();
        }
        let parent = s.documents()[0].doc.clone();
        let original = std::fs::read(&path).unwrap();
        open_child(&mut s, 0, layer);
        s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
        for malformed in [json!(null), json!({}), json!([{}]), json!([{"document": "bad", "revision": 1}]), json!([{"document": 1}])] {
            assert!(s.execute("file.close", json!({"discardDocuments": malformed})).is_err());
            assert_eq!(s.documents().len(), 2);
            assert!(s.active().unwrap().is_dirty());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        let st = s.active().unwrap();
        let confirmations = json!([{"document": st.doc.id.0, "revision": st.revision}]);
        // A changed child cannot be thrown away with an old confirmation.
        s.execute("layer.setProps", json!({"name": "newer edit"})).unwrap();
        assert!(s.execute("file.close", json!({"discardDocuments": confirmations})).is_err());
        assert_eq!(s.documents().len(), 2);
        let st = s.active().unwrap();
        s.execute("file.close", json!({"discardDocuments": [{"document": st.doc.id.0, "revision": st.revision}]})).unwrap();
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&parent, &s.documents()[0].doc));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(s.smart_links.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn linked_contents_save_survives_deleting_the_originating_duplicate() {
    let (mut s, path, original) = linked_session();
    let duplicate = LayerId(s.execute("layer.duplicate", json!({"layer": original.0})).unwrap()["layer"].as_u64().unwrap());
    let child = open_child(&mut s, 0, original);
    s.set_active(0);
    s.execute("layer.delete", json!({"layer": original.0})).unwrap();
    let placement = smart(&s.active().unwrap().doc, duplicate).unwrap().transform;
    s.set_active(child);
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    assert!(!s.active().unwrap().is_dirty());
    let saved = decode_source("poster.pcraft", &std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.layers[0].opacity, 0.5);
    let sm = smart(&s.documents()[0].doc, duplicate).unwrap();
    assert_eq!(sm.transform, placement);
    assert_eq!(sm.source, SmartSource::Linked { path: path.to_string_lossy().into_owned() });
    assert!((sm.cache.as_ref().unwrap().rgba(-5, 6)[3] - 0.5).abs() < 0.01);
    std::fs::remove_file(path).unwrap();
}
