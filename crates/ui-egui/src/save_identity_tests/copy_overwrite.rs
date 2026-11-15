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

/// The write service of a copy that fails: `published` says the atomic writer had already renamed
/// the new bytes into place when the folder flush failed.
fn failing_writer(stamps: Rc<RefCell<std::collections::HashMap<String, u64>>>, published: bool) -> impl FnMut(&str, &[u8]) -> Result<(), String> {
    move |path: &str, _: &[u8]| {
        if published {
            stamps.borrow_mut().insert(path.to_string(), 2);
        }
        Err(format!("{path} was written, but the folder could not be flushed to disk"))
    }
}

/// A write can fail after it replaced the file: `photocraft_format::atomic` renames the new bytes
/// into place before it flushes the folder, and that error comes back with the copy already on
/// disk. Every incomplete-copy path then still invalidates the savepoint; a failure before the
/// rename leaves the file as it was, so the document stays clean.
#[test]
fn a_copy_that_failed_after_replacing_the_file_keeps_the_document_unsaved() {
    for command in ["saveACopy", "writeDocument", "app.save", "export"] {
        for published in [false, true] {
            let mut app = app_at("own.tif");
            let stamps = Rc::new(RefCell::new(std::collections::HashMap::from([("own.tif".to_string(), 1u64)])));
            let read = stamps.clone();
            app.services.file_stamp =
                Some(Box::new(move |path: &str| read.borrow().get(path).map(|len| crate::services::FileStamp { len: *len, modified: None })));
            let doc = app.session.active().unwrap().doc.id;
            app.remember_stamp(doc, "own.tif");
            let result = match command {
                "saveACopy" => {
                    app.services.write = Some(Box::new(failing_writer(stamps.clone(), published)));
                    app.replay_save("file.saveACopy", &json!({"path": "own.tif", "tiffLayers": false}))
                }
                "writeDocument" => {
                    app.services.write = Some(Box::new(failing_writer(stamps.clone(), published)));
                    app.write_document("own.tif".into(), &crate::ExportSettings { tiff_layers: false, ..Default::default() }, true).map(|_| json!({}))
                }
                "app.save" => {
                    app.services.automation_write = Some(Box::new(failing_writer(stamps.clone(), published)));
                    let choices = crate::FormatChoices::from_params(&json!({"tiffLayers": false}), "app.save").unwrap();
                    app.save_automation_with(Some("own.tif".into()), choices)
                }
                _ => {
                    app.services.write = Some(Box::new(failing_writer(stamps.clone(), published)));
                    app.replay_save(crate::export_dialog::COMMAND, &json!({"path": "own.tif", "format": "tif"}))
                }
            };
            assert!(result.is_err(), "{command}, published={published}: the write failed");
            assert_eq!(app.has_unsaved_work(), published, "{command}, published={published}: only a replaced file loses the savepoint");
            assert_eq!(app.session.active().unwrap().path.as_deref(), Some("own.tif"), "{command}: the document keeps its path");
        }
    }
}

/// A layered TIFF copy keeps the document only when the file it wrote holds it: an Indexed,
/// Bitmap, Duotone or Channel-multichannel document reopens as RGB or Grayscale, without its
/// palette, bitmap bits, duotone inks or channels, so its savepoint goes
/// (`tiff_layers::keeps_document`).
#[test]
fn copy_overwrite_uses_actual_tiff_mode_support() {
    // (the document's mode, the Image > Mode conversion from it, kept by a layered TIFF)
    let cases: [(&str, Option<(&str, &str)>, bool); 8] = [
        ("rgb", None, true),
        ("gray", None, true),
        ("cmyk", None, true),
        ("rgb", Some(("image.mode.lab", "{}")), false),
        ("rgb", Some(("image.mode.indexedColor", "{}")), false),
        ("gray", Some(("image.mode.bitmap", "{}")), false),
        ("gray", Some(("image.mode.duotone", r#"{"type": "monotone"}"#)), false),
        ("rgb", Some(("image.mode.multichannel", "{}")), false),
    ];
    for (base, conversion, kept) in cases {
        let mut app = app_at("own.tif");
        app.run("file.close", json!({})).unwrap();
        app.run("file.new", json!({"width": 4, "height": 4, "mode": base})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        if let Some((command, params)) = conversion {
            app.run(command, serde_json::from_str(params).unwrap()).unwrap();
        }
        let doc = app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().saved_to("own.tif".into());
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let written = bytes.clone();
        app.services.write = Some(Box::new(move |_, data| {
            *written.borrow_mut() = data.to_vec();
            Ok(())
        }));
        app.replay_save("file.saveACopy", &json!({"path": "own.tif", "tiffLayers": true})).unwrap();
        let back = photocraft_io::import("own.tif", &bytes.borrow()).unwrap().document;
        assert_eq!(back.mode == doc.mode, kept, "{conversion:?} from {base}: the file reopens as {:?}, not {:?}", back.mode, doc.mode);
        assert_eq!(app.has_unsaved_work(), !kept, "{conversion:?} from {base}");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("own.tif"), "{conversion:?}: the document keeps its file");
    }
}

/// The layered writer stores at most `MAX_EXTRA_SAMPLES` extra channels and drops the rest: a copy
/// that loses one leaves the document unsaved, while one that fits keeps it clean.
#[test]
fn copy_overwrite_past_the_tiff_channel_limit_keeps_the_document_unsaved() {
    for extra in [photocraft_codecs::MAX_EXTRA_SAMPLES, photocraft_codecs::MAX_EXTRA_SAMPLES + 1] {
        let mut app = app_at("own.tif");
        app.run("file.close", json!({})).unwrap();
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        for _ in 0..extra {
            app.run("channel.new", json!({})).unwrap();
        }
        assert_eq!(app.session.active().unwrap().doc.channels.len(), extra, "the channels are there");
        app.session.active_mut().unwrap().saved_to("own.tif".into());
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let written = bytes.clone();
        app.services.write = Some(Box::new(move |_, data| {
            *written.borrow_mut() = data.to_vec();
            Ok(())
        }));
        app.replay_save("file.saveACopy", &json!({"path": "own.tif", "tiffLayers": true})).unwrap();
        let back = photocraft_io::import("own.tif", &bytes.borrow()).unwrap().document;
        assert_eq!(back.channels.len(), extra.min(photocraft_codecs::MAX_EXTRA_SAMPLES), "{extra} channels written");
        assert_eq!(app.has_unsaved_work(), extra > photocraft_codecs::MAX_EXTRA_SAMPLES, "{extra} channels");
    }
}

/// The disk stamp as the platform reports it: length and modification time from the real file.
fn real_stamps(app: &mut PhotocraftApp) {
    app.services.file_stamp = Some(Box::new(|path: &str| {
        let meta = std::fs::metadata(path).ok()?;
        Some(crate::services::FileStamp { len: meta.len(), modified: meta.modified().ok() })
    }));
}

/// A writer that really publishes `bytes` at `path` through `photocraft_format::atomic`, then
/// reports the folder-flush failure that comes back *after* the rename; with `publish` false it
/// fails before touching the file.
fn failing_after_write(publish: bool) -> impl FnMut(&str, &[u8]) -> Result<(), String> {
    move |path: &str, bytes: &[u8]| {
        if publish {
            photocraft_format::atomic_write(std::path::Path::new(path), bytes).map_err(|e| e.to_string())?;
            return Err(format!("{path} was written, but the folder could not be flushed to disk"));
        }
        Err(format!("{path}: disk full"))
    }
}

/// A failed copy that published its bytes through an alias of the document's own file
/// (`nested/../own.tif`, a symlink, another case on a case-insensitive volume) still costs the
/// savepoint: the stamp is read at the bound path, not at the spelling the write was given. The
/// pre-publication half of the loop is the control that the bound stamp does not over-invalidate.
/// Expected from source; this is not an executed result until hosted CI runs it.
#[test]
fn a_failed_copy_through_an_alias_that_published_keeps_the_document_unsaved() {
    for command in ["file.saveACopy", crate::export_dialog::COMMAND] {
        for published in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let own = dir.path().join("own.tif").to_string_lossy().into_owned();
            std::fs::create_dir(dir.path().join("nested")).unwrap();
            let alias = dir.path().join("nested").join("..").join("own.tif").to_string_lossy().into_owned();
            std::fs::write(&own, b"saved original").unwrap();
            let mut app = app_at(&own);
            real_stamps(&mut app);
            let doc = app.session.active().unwrap().doc.id;
            app.remember_stamp(doc, &own);
            app.services.write = Some(Box::new(failing_after_write(published)));
            let r = app.replay_save(command, &json!({"path": alias, "format": "tif", "tiffLayers": false}));
            assert!(r.is_err(), "{command}, published={published}: the write reported a failure");
            assert_eq!(
                std::fs::read(&own).unwrap() != b"saved original",
                published,
                "{command}, published={published}: the bound file is replaced only when the write published"
            );
            assert_eq!(app.has_unsaved_work(), published, "{command}, published={published}: only a replaced file costs the savepoint");
            assert_eq!(app.session.active().unwrap().path.as_deref(), Some(own.as_str()), "{command}: the document keeps its path");
        }
    }
}

/// The same failure through a hard link to the document's own file: the link's entry is what the
/// rename replaced, the document's own entry keeps its bytes, so it stays clean. This constrains the
/// semantics (it does not, on its own, kill every `same_file`-based alternative: a post-write
/// `same_file` check is also false once the rename breaks the link).
#[test]
fn a_failed_copy_over_a_hard_link_leaves_the_source_clean() {
    for command in ["file.saveACopy", crate::export_dialog::COMMAND] {
        for published in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let own = dir.path().join("own.tif").to_string_lossy().into_owned();
            let alias = dir.path().join("alias.tif").to_string_lossy().into_owned();
            std::fs::write(&own, b"saved original").unwrap();
            std::fs::hard_link(&own, &alias).unwrap();
            let mut app = app_at(&own);
            real_stamps(&mut app);
            let doc = app.session.active().unwrap().doc.id;
            app.remember_stamp(doc, &own);
            app.services.write = Some(Box::new(failing_after_write(published)));
            let r = app.replay_save(command, &json!({"path": alias, "format": "tif", "tiffLayers": false}));
            assert!(r.is_err(), "{command}, published={published}: the write reported a failure");
            assert_eq!(std::fs::read(&own).unwrap(), b"saved original", "{command}, published={published}: the bound file keeps its bytes");
            if published {
                assert_ne!(std::fs::read(&alias).unwrap(), b"saved original", "{command}: the link entry is what was replaced");
            }
            assert!(!app.has_unsaved_work(), "{command}, published={published}: the document's own file was never touched");
        }
    }
}

/// A copy of A over another open document B's file that publishes and then fails must detach B the
/// way a successful copy does: B loses the file that now holds A. `A` copies completely here (two
/// layers, layered TIFF), so its own `invalidate` is false — the case where nothing used to happen
/// at all. The `published = false` half is the control that a failure before the rename leaves B
/// alone. Expected from source; not an executed result until hosted CI runs it.
#[test]
fn a_failed_copy_that_published_detaches_the_other_documents_file() {
    for entry in ["file.saveACopy", "writeDocument", "app.save"] {
        for published in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let own = dir.path().join("a.tif").to_string_lossy().into_owned();
            let other = dir.path().join("b.tif").to_string_lossy().into_owned();
            std::fs::write(&other, b"saved original").unwrap();
            let mut app = app_at(&own);
            real_stamps(&mut app);
            app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
            let b = app.session.active().unwrap().doc.id;
            app.session.active_mut().unwrap().saved_to(other.clone());
            app.remember_stamp(b, &other);
            let a = app.session.documents()[0].doc.id;
            app.session.set_active(0);
            app.services.write = Some(Box::new(failing_after_write(published)));
            let settings = crate::ExportSettings { tiff_layers: true, ..Default::default() };
            let r = match entry {
                "file.saveACopy" => app.replay_save(entry, &json!({"path": other, "tiffLayers": true})),
                "writeDocument" => app.write_document(other.clone(), &settings, true).map(|_| json!({})),
                _ => app.save_automation_with(Some(other.clone()), crate::FormatChoices::from_params(&json!({"tiffLayers": true}), "app.save").unwrap()),
            };
            assert!(r.is_err(), "{entry}, published={published}: the write reported a failure");
            let b_state = app.session.documents().iter().find(|st| st.doc.id == b).unwrap();
            assert_eq!(b_state.path.is_none(), published, "{entry}, published={published}: the other tab loses a file that now holds this document");
            assert_eq!(b_state.is_dirty(), published, "{entry}, published={published}: the other tab is unsaved once its file holds another document");
            assert_eq!(std::fs::read(&other).unwrap() != b"saved original", published, "{entry}: the file holds the copy only when the write published");
            let a_state = app.session.documents().iter().find(|st| st.doc.id == a).unwrap();
            assert!(!a_state.is_dirty(), "{entry}, published={published}: the copying document was not saved by the failed copy");
            assert_eq!(a_state.path.as_deref(), Some(own.as_str()), "{entry}: the copying document keeps its file");
        }
    }
}

/// The same failed copy through a hard link to B's file: the rename replaces the link's entry and
/// leaves B's own entry with its bytes, so B keeps its file and stays clean.
#[test]
fn a_failed_copy_over_a_hard_link_leaves_the_other_documents_file_alone() {
    for entry in ["file.saveACopy", "writeDocument", "app.save"] {
        for published in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let own = dir.path().join("a.tif").to_string_lossy().into_owned();
            let other = dir.path().join("b.tif").to_string_lossy().into_owned();
            let alias = dir.path().join("b-alias.tif").to_string_lossy().into_owned();
            std::fs::write(&other, b"saved original").unwrap();
            std::fs::hard_link(&other, &alias).unwrap();
            let mut app = app_at(&own);
            real_stamps(&mut app);
            app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
            let b = app.session.active().unwrap().doc.id;
            app.session.active_mut().unwrap().saved_to(other.clone());
            app.remember_stamp(b, &other);
            app.session.set_active(0);
            app.services.write = Some(Box::new(failing_after_write(published)));
            let settings = crate::ExportSettings { tiff_layers: true, ..Default::default() };
            let r = match entry {
                "file.saveACopy" => app.replay_save(entry, &json!({"path": alias, "tiffLayers": true})),
                "writeDocument" => app.write_document(alias.clone(), &settings, true).map(|_| json!({})),
                _ => app.save_automation_with(Some(alias.clone()), crate::FormatChoices::from_params(&json!({"tiffLayers": true}), "app.save").unwrap()),
            };
            assert!(r.is_err(), "{entry}, published={published}: the write reported a failure");
            let b_state = app.session.documents().iter().find(|st| st.doc.id == b).unwrap();
            assert_eq!(b_state.path.as_deref(), Some(other.as_str()), "{entry}, published={published}: B keeps its own file");
            assert!(!b_state.is_dirty(), "{entry}, published={published}: B's file never changed");
            assert_eq!(std::fs::read(&other).unwrap(), b"saved original".to_vec(), "{entry}: B's bytes are untouched");
            if published {
                assert_ne!(std::fs::read(&alias).unwrap(), b"saved original".to_vec(), "{entry}: the link entry is what was replaced");
            }
        }
    }
}
