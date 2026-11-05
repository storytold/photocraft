use super::*;

fn app_at(path: &str) -> PhotocraftApp {
    let mut app = app();
    app.services.export = Some(Box::new(|doc, path, settings| {
        photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
    }));
    let st = app.session.active_mut().unwrap();
    st.saved_to(path.into());
    app
}

#[test]
fn discard_layers_over_the_own_tiff_keeps_the_close_warning() {
    let mut app = app_at("own.tif");
    let bytes = Rc::new(RefCell::new(Vec::new()));
    let written = bytes.clone();
    app.services.write = Some(Box::new(move |_, data| {
        *written.borrow_mut() = data.to_vec();
        Ok(())
    }));
    assert!(!app.has_unsaved_work());
    app.save_as(Some("own.tif".into())).unwrap();
    app.tiff_options.as_mut().unwrap().discard_layers = true;
    let ctx = egui::Context::default();
    crate::tiff_options_ui::confirm(&mut app, &ctx);
    assert!(photocraft_codecs::tiff_photoshop_tags(&bytes.borrow()).1.is_none(), "the written TIFF has no layers");
    assert!(app.has_unsaved_work(), "the layers now live only in memory");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("own.tif"));
    menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert!(app.discard.is_some(), "closing asks before losing the layers");
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn replayed_flat_copies_invalidate_the_own_files_savepoint() {
    for ext in ["tif", "png", "jpg", "webp", "tga", "dds"] {
        let path = format!("own.{ext}");
        let mut app = app_at(&path);
        app.replay_save("file.saveACopy", &json!({"path": path, "tiffLayers": false})).unwrap();
        assert!(app.has_unsaved_work(), "{ext}: a flat copy cannot hold the layers");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    }
}

#[test]
fn copies_elsewhere_failures_and_intact_layered_copies_keep_the_savepoint() {
    for (own, target, layers, fail) in [
        ("own.tif", "copy.tif", false, false),
        ("own.tif", "own.tif", false, true),
        ("own.tif", "own.tif", true, false),
        ("own.psd", "own.psd", true, false),
        ("own.pcraft", "own.pcraft", true, false),
        ("own.ora", "own.ora", true, false),
    ] {
        let mut app = app_at(own);
        let before = identity(&app);
        if fail {
            app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
        }
        let r = app.replay_save("file.saveACopy", &json!({"path": target, "tiffLayers": layers}));
        assert_eq!(r.is_err(), fail, "{own} -> {target}: {r:?}");
        assert_eq!(identity(&app), before, "{own} -> {target}");
        assert!(!app.has_unsaved_work());
    }
}

#[test]
fn scaled_exports_over_the_own_file_keep_the_original_unsaved() {
    for ext in ["png", "jpg", "webp", "tif", "tga"] {
        let path = format!("own.{ext}");
        let mut app = app_at(&path);
        let written_size = Rc::new(RefCell::new(None));
        let saw = written_size.clone();
        app.services.export = Some(Box::new(move |doc, _, _| {
            *saw.borrow_mut() = Some(doc.size);
            Ok((Vec::new(), Vec::new()))
        }));
        app.replay_save(crate::export_dialog::COMMAND, &json!({"path": path, "format": ext, "scale": 50})).unwrap();
        assert_eq!(written_size.borrow().unwrap(), photocraft_geom::Size::new(2, 2));
        assert!(app.has_unsaved_work(), "{ext}: the original size no longer exists in the file");
    }
}

#[test]
fn failed_exports_and_exports_elsewhere_keep_the_savepoint() {
    for (target, fail) in [("copy.png", false), ("own.png", true)] {
        let mut app = app_at("own.png");
        let before = identity(&app);
        if fail {
            app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
        }
        let r = app.replay_save(crate::export_dialog::COMMAND, &json!({"path": target, "format": "png", "scale": 50}));
        assert_eq!(r.is_err(), fail);
        assert_eq!(identity(&app), before);
    }
}

#[test]
fn a_discarded_tiff_copy_recognizes_another_spelling_of_the_own_path() {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("own.tif").to_string_lossy().into_owned();
    let alias = dir.path().join(".").join("own.tif").to_string_lossy().into_owned();
    let mut app = app_at(&own);
    std::fs::write(&own, b"old file").unwrap();
    app.services.write = Some(Box::new(|path, bytes| photocraft_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| e.to_string())));
    app.replay_save("file.saveACopy", &json!({"path": alias, "tiffLayers": false})).unwrap();
    assert!(app.session.active().unwrap().is_dirty());
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(own.as_str()));
    let bytes = std::fs::read(&own).unwrap();
    assert!(photocraft_codecs::tiff_photoshop_tags(&bytes).1.is_none());
}

#[test]
fn an_automation_flat_copy_over_the_own_file_keeps_the_source_unsaved() {
    let mut app = app_at("own.tif");
    let choices = crate::FormatChoices::from_params(&json!({"tiffLayers": false}), "app.save").unwrap();
    app.save_automation_with(Some("own.tif".into()), choices).unwrap();
    assert!(app.has_unsaved_work());
}

#[test]
fn replaying_a_plain_tiff_save_with_discard_layers_keeps_the_source_unsaved() {
    let mut app = app_at("own.tif");
    // A plain one-layer TIFF can be the target of a recorded File > Save. Its layer is kept
    // editable in memory even when the recorded TIFF answer chooses a flattened copy.
    app.run("file.new", json!({"width": 4, "height": 4, "background": "transparent"})).unwrap();
    app.session.active_mut().unwrap().saved_to("own.tif".into());
    app.replay_save("file.save", &json!({"tiffLayers": false, "tiffCompression": "none"})).unwrap();
    assert!(app.session.active().unwrap().is_dirty());
}

#[test]
fn selected_layer_exports_over_the_own_file_keep_the_document_unsaved() {
    for quick in [false, true] {
        let mut app = app_at("own.png");
        app.run("edit.fill", json!({"color": "#3366cc"})).unwrap();
        let layers: Vec<_> = app.session.active().unwrap().doc.layers.iter().map(|layer| layer.id).collect();
        let st = app.session.active_mut().unwrap();
        st.selected_layers = layers.clone();
        st.saved_to("own.png".into());
        app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(crate::FileDialogAnswer::SaveTo("own.png".into()))]).0);
        if quick {
            crate::export_dialog::quick_export_selected_layers_png(&mut app).unwrap();
        } else {
            let fields = json!({"format": "png", "scale": 50, "__layers": layers.iter().map(|id| id.0).collect::<Vec<_>>()});
            crate::export_dialog::confirm(&mut app, fields.as_object().unwrap()).unwrap();
        }
        app.poll_file_dialog(&egui::Context::default(), None);
        assert!(app.session.active().unwrap().is_dirty(), "quick={quick}: one layer replaced the whole document's file");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("own.png"));
    }
}

#[test]
fn copy_overwrite_uses_actual_tiff_layer_support() {
    for mode in ["rgb", "gray", "cmyk", "lab"] {
        for command in ["file.saveACopy", "file.saveAs"] {
            let mut app = app_at("own.tif");
            app.run("file.close", json!({})).unwrap();
            app.run("file.new", json!({"width": 4, "height": 4, "mode": mode})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.session.active_mut().unwrap().saved_to("own.tif".into());
            let bytes = Rc::new(RefCell::new(Vec::new()));
            let written = bytes.clone();
            app.services.write = Some(Box::new(move |_, data| {
                *written.borrow_mut() = data.to_vec();
                Ok(())
            }));
            let reply = app.replay_save(command, &json!({"path": "own.tif", "tiffLayers": true})).unwrap();
            assert_eq!(photocraft_codecs::tiff_photoshop_tags(&bytes.borrow()).1.is_some(), mode != "lab", "{mode}, {command}");
            assert_eq!(app.has_unsaved_work(), mode == "lab", "{mode}, {command}");
            assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
            if mode == "lab" {
                assert!(!reply["warnings"].as_array().unwrap().is_empty());
            }
        }
    }
}

#[test]
fn copy_overwrite_preserves_a_hard_link_source_but_replaces_a_symlink_target() {
    let dir = tempfile::tempdir().unwrap();
    for export in [false, true] {
        for symlink in [false, true] {
            let own = dir.path().join(format!("own-{export}-{symlink}.tif")).to_string_lossy().into_owned();
            let alias = dir.path().join(format!("alias-{export}-{symlink}.tif")).to_string_lossy().into_owned();
            std::fs::write(&own, b"original saved file").unwrap();
            if symlink {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&own, &alias).unwrap();
                #[cfg(not(unix))]
                continue;
            } else {
                std::fs::hard_link(&own, &alias).unwrap();
            }
            let mut app = app_at(&own);
            app.services.write = Some(Box::new(|path, bytes| photocraft_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| e.to_string())));
            let (command, params) = if export {
                (crate::export_dialog::COMMAND, json!({"path": alias, "format": "tif", "scale": 50}))
            } else {
                ("file.saveACopy", json!({"path": alias, "tiffLayers": false}))
            };
            app.replay_save(command, &params).unwrap();
            assert_eq!(app.has_unsaved_work(), symlink, "export={export}, symlink={symlink}");
            assert_eq!(std::fs::read(&own).unwrap() == b"original saved file", !symlink);
        }
    }
}

#[test]
fn copy_overwrite_refreshes_only_its_own_successful_write_stamp() {
    for automation in [false, true] {
        for (target, fail) in [("own.tif", false), ("own.tif", true), ("copy.tif", false)] {
            let mut app = app_at("own.tif");
            let stamps = Rc::new(RefCell::new(std::collections::HashMap::from([("own.tif".to_string(), 1)])));
            let read = stamps.clone();
            app.services.file_stamp = Some(Box::new(move |path| read.borrow().get(path).map(|len| crate::services::FileStamp { len: *len, modified: None })));
            let doc = app.session.active().unwrap().doc.id;
            app.remember_stamp(doc, "own.tif");
            let written = stamps.clone();
            let writer = move |path: &str, _: &[u8]| {
                if fail {
                    return Err("disk full".into());
                }
                written.borrow_mut().insert(path.into(), 2);
                Ok(())
            };
            let params = json!({"path": target, "tiffLayers": false});
            let reply = if automation {
                app.services.automation_write = Some(Box::new(writer));
                let choices = crate::FormatChoices::from_params(&params, "app.save").unwrap();
                app.save_automation_with(Some(target.into()), choices)
            } else {
                app.services.write = Some(Box::new(writer));
                app.replay_save("file.saveACopy", &params)
            };
            assert_eq!(reply.is_err(), fail);
            assert!(!app.changed_on_disk(doc, "own.tif"), "the app's own write is already acknowledged");
            assert_eq!(app.disk_stamps[&doc].0, "own.tif");
            assert_eq!(app.disk_stamps[&doc].1.len, if target == "own.tif" && !fail { 2 } else { 1 });
            stamps.borrow_mut().insert("own.tif".into(), 3);
            assert!(app.changed_on_disk(doc, "own.tif"), "a later external write must still be detected");
        }
    }
}

#[test]
fn copy_overwrite_from_the_tiff_prompt_uses_actual_layer_support() {
    let mut app = app_at("own.tif");
    app.run("file.close", json!({})).unwrap();
    app.run("file.new", json!({"width": 4, "height": 4, "mode": "lab"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session.active_mut().unwrap().saved_to("own.tif".into());
    app.save_as(Some("own.tif".into())).unwrap();
    assert!(!app.tiff_options.as_ref().unwrap().discard_layers);
    crate::tiff_options_ui::confirm(&mut app, &egui::Context::default());
    assert!(app.has_unsaved_work(), "the encoder falls back to flat TIFF despite the Layers answer");
}

#[test]
fn copy_overwrite_recreating_a_deleted_backing_file_through_an_alias_is_unsaved() {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("own.tif").to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    let alias = dir.path().join("nested").join("..").join("own.tif").to_string_lossy().into_owned();
    std::fs::write(&own, b"saved original").unwrap();
    let mut app = app_at(&own);
    std::fs::remove_file(&own).unwrap();
    app.services.write = Some(Box::new(|path, bytes| photocraft_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| e.to_string())));
    app.replay_save("file.saveACopy", &json!({"path": alias, "tiffLayers": false})).unwrap();
    assert!(app.session.active().unwrap().is_dirty(), "the recreated backing file no longer holds the editable layers");
    assert!(photocraft_codecs::tiff_photoshop_tags(&std::fs::read(&own).unwrap()).1.is_none());
}

#[test]
fn copy_overwrite_of_a_flat_background_tiff_remains_conservatively_unsaved() {
    for copy in [false, true] {
        let mut app = app_at("own.tif");
        app.run("file.close", json!({})).unwrap();
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.session.active_mut().unwrap().saved_to("own.tif".into());
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let written = bytes.clone();
        app.services.write = Some(Box::new(move |_, data| {
            *written.borrow_mut() = data.to_vec();
            Ok(())
        }));
        let command = if copy { "file.saveACopy" } else { "file.saveAs" };
        app.replay_save(command, &json!({"path": "own.tif", "tiffLayers": true})).unwrap();
        assert!(photocraft_codecs::tiff_photoshop_tags(&bytes.borrow()).1.is_none());
        assert_eq!(app.session.active().unwrap().is_dirty(), copy, "ordinary saves retain their existing classification");
    }
}
