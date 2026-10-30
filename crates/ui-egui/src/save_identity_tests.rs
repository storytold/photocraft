//! Save identity belongs to the editable document, after a successful write only.

use crate::{ExportSettings, PhotocraftApp, Services, menus};
use photocraft_engine::Session;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(|_, _| Ok(()))),
            automation_write: Some(Box::new(|_, _| Ok(()))),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-1"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn identity(app: &PhotocraftApp) -> (String, Option<String>, u64, u64) {
    let st = app.session.active().unwrap();
    (st.doc.name.clone(), st.path.clone(), st.revision, st.saved_revision)
}

fn layer_target(session: &Session) -> (Option<String>, Vec<String>) {
    let st = session.active().unwrap();
    let active = st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| l.name.clone());
    let mut selected: Vec<_> = st.selected_layers().iter().map(|id| st.doc.layer(*id).unwrap().name.clone()).collect();
    selected.sort();
    (active, selected)
}

fn assert_saved_layer_target(name: &str, bytes: &[u8], expected: &(Option<String>, Vec<String>)) {
    let mut reopened = Session::new();
    reopened.add_document(photocraft_io::import(name, bytes).unwrap().document, None);
    assert_eq!(&layer_target(&reopened), expected, "{name}");
    assert!(reopened.active().unwrap().doc.saved_layer_selection.is_none());
}

fn select_middle(app: &mut PhotocraftApp) {
    app.run("layer.new.layer", json!({"name": "Middle"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    app.run("layer.select", json!({"name": "Middle"})).unwrap();
}

#[test]
fn synchronous_user_and_automation_saves_keep_current_layer_targeting() {
    for automation in [false, true] {
        for ext in ["psd", "psb", "pcraft"] {
            let mut app = app();
            select_middle(&mut app);
            let bytes = Rc::new(RefCell::new(Vec::new()));
            let captured = bytes.clone();
            if automation {
                app.services.automation_write = Some(Box::new(move |_, data| {
                    *captured.borrow_mut() = data.to_vec();
                    Ok(())
                }));
            } else {
                app.services.write = Some(Box::new(move |_, data| {
                    *captured.borrow_mut() = data.to_vec();
                    Ok(())
                }));
            }
            let name = format!("target.{ext}");
            let target = layer_target(&app.session);
            if automation {
                app.save_automation(Some(name.clone())).unwrap();
            } else {
                app.save_as(Some(name.clone())).unwrap();
            }
            assert_saved_layer_target(&name, &bytes.borrow(), &target);
            app.run("layer.select", json!({"name": "Background", "mode": "add"})).unwrap();
            assert!(!app.session.active().unwrap().is_dirty(), "layer targeting leaves a clean file clean");
            let target = layer_target(&app.session);
            if automation {
                app.save_automation(None).unwrap();
            } else {
                menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap();
            }
            assert_saved_layer_target(&name, &bytes.borrow(), &target);
            app.run("select.deselectLayers", json!({})).unwrap();
            if automation {
                app.save_automation(None).unwrap();
            } else {
                menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap();
            }
            assert_saved_layer_target(&name, &bytes.borrow(), &(None, Vec::new()));
            assert!(app.session.active().unwrap().doc.saved_layer_selection.is_none(), "saving must not install its transport snapshot into live state");
        }
    }
}

#[test]
fn save_a_copy_captures_layer_targeting_before_the_file_picker_and_replay_uses_current_targeting() {
    let mut app = app();
    select_middle(&mut app);
    let bytes = Rc::new(RefCell::new(Vec::new()));
    let captured = bytes.clone();
    app.services.write = Some(Box::new(move |_, data| {
        *captured.borrow_mut() = data.to_vec();
        Ok(())
    }));
    let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("copy.psd".into()))]);
    app.services.file_dialog = Some(dialog);
    let ctx = egui::Context::default();
    let target = layer_target(&app.session);
    let before = identity(&app);
    let live = app.session.active().unwrap().doc.clone();
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    app.run("layer.select", json!({"name": "Top"})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_saved_layer_target("copy.psd", &bytes.borrow(), &target);
    assert_eq!(identity(&app).1, before.1, "a copy does not take ownership of the output path");
    assert!(std::sync::Arc::ptr_eq(&live, &app.session.active().unwrap().doc));
    assert_eq!(layer_target(&app.session).0.as_deref(), Some("Top"));
    let current = layer_target(&app.session);
    app.replay_save("file.saveACopy", &json!({"path": "replayed.psd"})).unwrap();
    assert_saved_layer_target("replayed.psd", &bytes.borrow(), &current);

    let (dialog, _) = crate::file_dialog::fake(vec![None]);
    app.services.file_dialog = Some(dialog);
    bytes.borrow_mut().clear();
    let before = identity(&app);
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(bytes.borrow().is_empty(), "cancelling the picker writes nothing");
    assert_eq!(identity(&app), before);
    app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
    assert!(app.replay_save("file.saveACopy", &json!({"path": "failed.psd"})).is_err());
    assert_eq!(identity(&app), before);
    assert_eq!(layer_target(&app.session), current);
    assert!(std::sync::Arc::ptr_eq(&live, &app.session.active().unwrap().doc));
}

#[test]
fn background_user_and_automation_saves_capture_the_original_documents_layer_target() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::{Duration, Instant};
    for automation in [false, true] {
        for initially_clean in [false, true] {
            for outcome in ["saved", "failed", "cancelled"] {
                let mut app = app();
                select_middle(&mut app);
                // Keep the name stable so pointer identity also checks the successful completion.
                Arc::make_mut(&mut app.session.active_mut().unwrap().doc).name = "background.psd".into();
                if initially_clean {
                    app.save_as(Some("background.psd".into())).unwrap();
                }
                app.background_jobs = true;
                let gate = Arc::new(AtomicBool::new(false));
                let ready = gate.clone();
                let captured = Arc::new(Mutex::new(Vec::new()));
                let written = captured.clone();
                let save: crate::services::SaveFileFn = Arc::new(move |doc, path, settings, ctl| {
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while !ready.load(Ordering::Acquire) {
                        ctl.check()?;
                        if Instant::now() >= deadline {
                            return Err("test save was not released".into());
                        }
                        std::thread::yield_now();
                    }
                    let encoded = photocraft_io::export(doc, path, &settings.io_options()).map_err(|e| e.to_string())?;
                    if outcome == "failed" {
                        return Err("disk full".into());
                    }
                    ctl.commit()?;
                    *written.lock().unwrap() = encoded.bytes;
                    Ok(encoded.warnings)
                });
                if automation {
                    app.services.automation_save = Some(save);
                } else {
                    app.services.save_file = Some(save);
                }
                let expected = layer_target(&app.session);
                let live = app.session.active().unwrap().doc.clone();
                let result = if automation { app.save_automation(Some("background.psd".into())) } else { app.save_as(Some("background.psd".into())) }.unwrap();
                assert_eq!(result["pending"], true);
                // The direct session targeting API stays view-only while a worker holds the
                // document. The command surface still blocks pixel edits during that save.
                let top = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, layer)| layer.name == "Top").unwrap().2.id;
                app.session.select_layer(top).unwrap();
                let latest = layer_target(&app.session);
                let before = identity(&app);
                assert_eq!(app.session.active().unwrap().is_dirty(), !initially_clean);
                assert!(Arc::ptr_eq(&live, &app.session.active().unwrap().doc));
                assert!(app.run("edit.fill", json!({"color": "#ff0000"})).is_err());
                if outcome == "cancelled" {
                    let job = app.jobs.saves.first().unwrap().job;
                    assert!(app.session.cancel_job(job));
                }
                app.run("file.new", json!({"width": 2, "height": 1, "name": "Another document"})).unwrap();
                let other = app.session.active().unwrap().doc.id;
                gate.store(true, Ordering::Release);
                let deadline = Instant::now() + Duration::from_secs(10);
                while app.saving() {
                    assert!(Instant::now() < deadline, "save did not finish");
                    crate::jobs_ui::tick(&mut app, &egui::Context::default());
                    std::thread::yield_now();
                }
                assert_eq!(app.session.active().unwrap().doc.id, other, "save completion preserves the current tab");
                assert!(app.session.set_active(0));
                assert_eq!(layer_target(&app.session), latest);
                assert!(app.session.active().unwrap().doc.saved_layer_selection.is_none());
                assert!(Arc::ptr_eq(&live, &app.session.active().unwrap().doc));
                if outcome == "saved" {
                    assert_saved_layer_target("background.psd", &captured.lock().unwrap(), &expected);
                    assert!(!app.session.active().unwrap().is_dirty());
                } else {
                    assert!(captured.lock().unwrap().is_empty());
                    assert_eq!(identity(&app), before);
                }
            }
        }
    }
}

#[test]
fn background_save_does_not_mark_a_different_document_snapshot_clean() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::{Duration, Instant};
    for automation in [false, true] {
        let mut app = app();
        select_middle(&mut app);
        app.background_jobs = true;
        let gate = Arc::new(AtomicBool::new(false));
        let ready = gate.clone();
        let save: crate::services::SaveFileFn = Arc::new(move |doc, path, settings, ctl| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.load(Ordering::Acquire) {
                ctl.check()?;
                if Instant::now() >= deadline {
                    return Err("test save was not released".into());
                }
                std::thread::yield_now();
            }
            let encoded = photocraft_io::export(doc, path, &settings.io_options()).map_err(|e| e.to_string())?;
            ctl.commit()?;
            Ok(encoded.warnings)
        });
        if automation {
            app.services.automation_save = Some(save);
        } else {
            app.services.save_file = Some(save);
        }
        let previous_saved = app.session.active().unwrap().saved_revision;
        let previous_path = app.session.active().unwrap().path.clone();
        if automation {
            app.save_automation(Some("stale.psd".into())).unwrap();
        } else {
            app.save_as(Some("stale.psd".into())).unwrap();
        }
        // Simulate a lower-level caller bypassing command admission: the job's original Arc
        // guard must still reject completion instead of marking these new pixels saved.
        app.session
            .edit("forced content change", |doc, active| {
                let layer = active.and_then(|id| doc.layer_mut(id)).and_then(|layer| layer.surface_mut()).ok_or(photocraft_engine::EngineError::NoDocument)?;
                layer.write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        let modified = app.session.active().unwrap().doc.clone();
        let modified_revision = app.session.active().unwrap().revision;
        gate.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.saving() {
            assert!(Instant::now() < deadline, "save did not finish");
            crate::jobs_ui::tick(&mut app, &egui::Context::default());
            std::thread::yield_now();
        }
        let state = app.session.active().unwrap();
        assert!(state.is_dirty());
        assert_eq!((state.revision, state.saved_revision, &state.path), (modified_revision, previous_saved, &previous_path));
        assert!(Arc::ptr_eq(&modified, &state.doc));
        assert!(state.doc.saved_layer_selection.is_none());
    }
}

#[test]
fn save_settings_share_only_the_open_documents_native_cache() {
    let mut app = app();
    let first = app.save_settings().io_options().pcraft_cache.unwrap();
    assert_eq!(first, app.session.active().unwrap().pcraft_cache);
    assert_eq!(first, app.save_settings().io_options().pcraft_cache.unwrap());
    app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
    assert_ne!(first, app.save_settings().io_options().pcraft_cache.unwrap());
}

#[test]
fn new_document_psd_and_subsequent_save_as_adopt_the_written_file_name() {
    for depth in [8, 16, 32] {
        let mut app = app();
        app.run("file.new", json!({"width": 4, "height": 4, "depth": depth, "name": "Untitled-1"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let written = bytes.clone();
        app.services.write = Some(Box::new(move |_, data| {
            *written.borrow_mut() = data.to_vec();
            Ok(())
        }));
        let id = app.session.active().unwrap().doc.id;
        let revision = app.session.active().unwrap().revision;
        let ctx = egui::Context::default();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "作品/My image.PSD"})).unwrap();
        assert_eq!(identity(&app), ("My image.PSD".into(), Some("作品/My image.PSD".into()), revision, revision));
        assert_eq!(app.session.active().unwrap().doc.id, id);
        let reopened = photocraft_io::import("My image.PSD", &bytes.borrow()).unwrap();
        assert_eq!(reopened.document.size, app.session.active().unwrap().doc.size);
        assert_eq!(reopened.document.depth.bits(), depth);
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "作品/次の画像.psd"})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, "次の画像.psd");
        menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("作品/次の画像.psd"));
        app.open_bytes("existing.psd", &bytes.borrow()).unwrap();
        let id = app.active_doc_id().unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, "existing.psd");
        app.save_as(Some("renamed existing.psd".into())).unwrap();
        assert_eq!(app.active_doc_id().unwrap(), id);
        assert_eq!(app.session.active().unwrap().doc.name, "renamed existing.psd");
    }
}

#[test]
fn failed_encode_or_write_keeps_the_previous_name_path_and_dirty_state() {
    for saved_before in [false, true] {
        let mut app = app();
        if saved_before {
            app.save_as(Some("before.psd".into())).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
        }
        let before = identity(&app);
        app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
        assert!(app.save_as(Some("after.psd".into())).is_err());
        assert_eq!(identity(&app), before);
        assert!(app.save_automation(Some("after.psd".into())).is_ok());
        app.run("layer.new.layer", json!({})).unwrap();
        let before = identity(&app);
        app.services.automation_write = Some(Box::new(|_, _| Err("denied".into())));
        assert!(app.save_automation(Some("another.psd".into())).is_err());
        assert_eq!(identity(&app), before);
        app.services.export = Some(Box::new(|_, _, _| Err("cannot encode".into())));
        assert!(app.save_as(Some("after.psd".into())).is_err());
        assert_eq!(identity(&app), before);
    }
}

#[test]
fn save_copy_and_export_keep_the_editable_document_identity() {
    let mut app = app();
    app.save_as(Some("original.psd".into())).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let before = identity(&app);
    app.write_document("copy.psd".into(), &ExportSettings::default(), true).unwrap();
    assert_eq!(identity(&app), before);
    let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("export.png".into()))]);
    app.services.file_dialog = Some(dialog);
    let ctx = egui::Context::default();
    crate::export_dialog::confirm(&mut app, json!({"format": "png", "scale": 100}).as_object().unwrap()).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(identity(&app), before);
    assert!(app.ui.status.starts_with("Exported export.png"), "{}", app.ui.status);
}

#[test]
fn web_download_and_automation_saves_use_the_same_identity_completion() {
    let mut app = app();
    let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("Untitled-1.psd".into()))]);
    app.services.file_dialog = Some(dialog);
    app.save_as(None).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_eq!(app.session.active().unwrap().doc.name, "Untitled-1.psd");
    app.save_automation(Some("保存/agent save.psd".into())).unwrap();
    assert_eq!(app.session.active().unwrap().doc.name, "agent save.psd");
}

/// Save As to a flat format writes a copy when the flat file can't hold the document: its layers,
/// or the layered file it already lives in. The document keeps its file, so Save still writes the
/// layered original and the edits stay unsaved (#2550).
#[test]
fn save_as_flat_keeps_the_layered_original() {
    let ctx = egui::Context::default();
    // A layered document, and a one-layer one (the issue's case: the dialog offered a JPEG).
    for layered in [true, false] {
        let mut app = app();
        if !layered {
            app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-2"})).unwrap();
        }
        let written = Rc::new(RefCell::new(Vec::new()));
        let w = written.clone();
        app.services.write = Some(Box::new(move |path, _| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        }));
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "art.psd"})).unwrap();
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
        let before = identity(&app);
        for flat in ["art.jpg", "art.png"] {
            // A JPEG Options answer, so the JPEG doesn't wait on the prompt.
            menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": flat, "jpegFormat": "baseline"})).unwrap();
            assert_eq!(identity(&app), before, "{flat}, layered: {layered}");
        }
        menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        assert_eq!(*written.borrow(), ["art.psd", "art.jpg", "art.png", "art.psd"]);
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision);
    }
    // An untitled layered document saved as a JPEG is still untitled and unsaved.
    let mut app = app();
    let written = Rc::new(RefCell::new(Vec::new()));
    let w = written.clone();
    app.services.write = Some(Box::new(move |path, _| {
        w.borrow_mut().push(path.to_string());
        Ok(())
    }));
    let before = identity(&app);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "flat.jpg", "jpegFormat": "baseline"})).unwrap();
    assert_eq!(*written.borrow(), ["flat.jpg"]);
    assert_eq!(identity(&app), before);
}

/// A flat document that isn't in a layered file becomes the flat file it is saved as, as before.
#[test]
fn save_as_flat_retargets_a_flat_document() {
    let mut app = app();
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-2"})).unwrap();
    app.save_as(Some("shot.png".into())).unwrap();
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("shot.png"));
    app.save_as(Some("shot.jpg".into())).unwrap();
    // JPEG Options asks first; OK writes.
    crate::jpeg_options_ui::confirm(&mut app, &egui::Context::default());
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("shot.jpg", Some("shot.jpg")));
    assert_eq!(st.saved_revision, st.revision);
}

/// `app.save` (control channel, MCP bridge) to a flat format is a copy, as in the headless server
/// (#1547); only a layered format becomes the document's file (#2579).
#[test]
fn automation_flat_save_is_a_copy() {
    let mut app = app();
    let before = identity(&app);
    for flat in ["flat.png", "flat.jpg", "flat.tif"] {
        app.save_automation(Some(flat.into())).unwrap();
        assert_eq!(identity(&app), before, "{flat}");
    }
    app.save_automation(Some("layers.psd".into())).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("layers.psd", Some("layers.psd")));
    assert_eq!(st.saved_revision, st.revision);
}

/// Preferences › File Handling › Image Previews reaches the exporter of Save As, Save and
/// automation saves: Always and Ask When Saving embed the PSD thumbnail, Never doesn't.
#[test]
fn image_previews_preference_reaches_the_exporter() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = app();
    let log = seen.clone();
    app.services.export = Some(Box::new(move |_, _, settings: &ExportSettings| {
        log.borrow_mut().push(settings.image_previews);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    let ctx = egui::Context::default();
    for (value, expect) in [("always", true), ("never", false), ("ask", true)] {
        app.run("prefs.set", json!({"path": "fileHandling.imagePreviews", "value": value})).unwrap();
        seen.borrow_mut().clear();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": format!("{value}.psd")})).unwrap();
        menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        app.save_automation(Some(format!("{value} agent.psd"))).unwrap();
        assert_eq!(*seen.borrow(), [expect; 3], "{value}");
    }
}

/// Preferences › File Handling › Maximize PSD and PSB File Compatibility reaches the exporter of
/// Save As, Save and automation saves: Always and Ask When Saving keep the PSD's real merged
/// image, Never doesn't.
#[test]
fn maximize_compatibility_preference_reaches_the_exporter() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = app();
    let log = seen.clone();
    app.services.export = Some(Box::new(move |_, _, settings: &ExportSettings| {
        log.borrow_mut().push(settings.psd_maximize_compatibility);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    let ctx = egui::Context::default();
    for (value, expect) in [("always", true), ("never", false), ("ask", true)] {
        app.run("prefs.set", json!({"path": "fileHandling.maximizePsdCompatibility", "value": value})).unwrap();
        seen.borrow_mut().clear();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": format!("{value}.psd")})).unwrap();
        menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        app.save_automation(Some(format!("{value} agent.psd"))).unwrap();
        assert_eq!(*seen.borrow(), [expect; 3], "{value}");
    }
}

/// #2399: Save and Save As start at the opened file's own encoding: a JPEG's estimated quality,
/// a WebP lossy (at its quality) or lossless; a new document keeps the codec defaults.
#[test]
fn save_settings_start_at_the_opened_files_encoding() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let px: Vec<u8> = (0..32u32 * 24).flat_map(|i| [(i * 3) as u8, (i / 32 * 9) as u8, (i % 97) as u8]).collect();
    let img = Image::from_raw(32, 24, ChannelLayout::Rgb, SampleType::U8, px).unwrap();
    let mut app = app();
    assert_eq!(app.save_settings().jpeg_quality, None);
    assert!(app.save_settings().webp_lossless);

    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 65, ..Default::default() }).unwrap();
    app.open_bytes("photo.jpg", &jpeg).unwrap();
    let q = app.save_settings().jpeg_quality.unwrap();
    assert!(q.abs_diff(65) <= 2, "estimated {q}");

    let lossy = photocraft_codecs::encode(&img, Format::WebP, &EncodeOptions { webp_lossless: false, webp_quality: 80, ..Default::default() }).unwrap();
    app.open_bytes("photo.webp", &lossy).unwrap();
    let s = app.save_settings();
    assert!(!s.webp_lossless);
    assert!(s.webp_quality.unwrap().abs_diff(80) <= 2, "{:?}", s.webp_quality);

    let lossless = photocraft_codecs::encode(&img, Format::WebP, &EncodeOptions::default()).unwrap();
    app.open_bytes("art.webp", &lossless).unwrap();
    assert!(app.save_settings().webp_lossless);
}

/// A browser download is only requested: the document takes the name but keeps its unsaved
/// changes, so the leave-page guard stays armed (#4962). A confirmed write still saves it.
#[test]
fn unconfirmed_download_names_the_document_but_keeps_it_unsaved() {
    assert_eq!(crate::saved_revision_after_write(true, 7, 3), 7);
    assert_eq!(crate::saved_revision_after_write(false, 7, 3), 3);
    let mut app = app();
    app.services.write_unconfirmed = true;
    let (_, _, revision, saved) = identity(&app);
    app.save_as(Some("download.psd".into())).unwrap();
    assert_eq!(identity(&app), ("download.psd".into(), Some("download.psd".into()), revision, saved));
    assert!(app.session.active().unwrap().is_dirty());
    assert!(app.has_unsaved_work());
    app.services.write_unconfirmed = false;
    app.save_as(Some("download.psd".into())).unwrap();
    assert!(!app.has_unsaved_work());
}
