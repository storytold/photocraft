//! Save identity belongs to the editable document, after a successful write only.

use crate::{ExportSettings, PhotocraftApp, Services, menus};
use photocraft_engine::Session;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map_err(|e| e.to_string()))),
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
fn delayed_save_copy_checks_the_copied_documents_source_after_switching_tabs() {
    let mut app = app();
    app.services.same_file = Some(Box::new(|a, b| a == b || (a == "source.xcf" && b == "alias.pcraft")));
    app.session.active_mut().unwrap().protected_import_source = Some("source.xcf".into());
    let copied = app.session.active().unwrap().doc.clone();
    app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
    let active = app.active_doc_id().unwrap();
    let written = Rc::new(RefCell::new(Vec::new()));
    let w = written.clone();
    app.services.write = Some(Box::new(move |path, _| {
        w.borrow_mut().push(path.to_owned());
        Ok(())
    }));
    assert!(crate::view_cmds::write_copy(&mut app, &copied, "alias.pcraft", Default::default()).is_err());
    assert!(written.borrow().is_empty(), "no source alias may reach the writer");
    assert_eq!(app.active_doc_id().unwrap(), active);
    crate::view_cmds::write_copy(&mut app, &copied, "converted.pcraft", Default::default()).unwrap();
    assert_eq!(*written.borrow(), ["converted.pcraft"]);
    assert_eq!(app.active_doc_id().unwrap(), active);
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

#[test]
fn read_only_conversion_reports_default_to_native_while_existing_format_choices_keep_priority() {
    use photocraft_doc::import_conversion::{ConversionReport, ImportConversion};
    for opaque in [false, true] {
        for (read_only, reported, path, name, expected) in [
            (false, true, Some("existing.psd"), "Display name", "existing.psd"),
            (false, true, Some("existing.pcraft"), "Display name", "existing.pcraft"),
            (false, true, None, "reopened.psd", "reopened.pcraft"),
            (false, false, None, "ordinary.psd", "ordinary.psd"),
            (true, false, Some("legacy.psd"), "Display name", "legacy.psd"),
            (true, false, None, "ordinary.xcf", "ordinary.psd"),
            (true, false, None, "paint.pdn", "paint.pcraft"),
            (true, false, None, "drawing.kra", "drawing.ora"),
            (true, true, None, "historical.arbitrary", "historical.pcraft"),
            (true, true, Some("misleading.psd"), "Display name", "misleading.pcraft"),
        ] {
            let mut app = app();
            let st = app.session.active_mut().unwrap();
            st.source_read_only = read_only;
            st.path = path.map(str::to_owned);
            let doc = std::sync::Arc::make_mut(&mut st.doc);
            doc.name = name.into();
            doc.metadata.import_conversion = reported.then(|| {
                if opaque {
                    ImportConversion::Opaque { schema_version: 77, record: std::sync::Arc::new(b"future inert report".to_vec()), attachments: Vec::new() }
                } else {
                    ImportConversion::Report(ConversionReport {
                        source_format: "arbitrary historical label, never a format dispatch".into(),
                        ..Default::default()
                    })
                }
            });
            let before = identity(&app);
            let (dialog, requests) = crate::file_dialog::fake(vec![None]);
            app.services.file_dialog = Some(dialog);
            app.save_as(None).unwrap();
            app.poll_file_dialog(&egui::Context::default(), None);
            assert_eq!(
                *requests.borrow(),
                [crate::file_dialog::FileDialogRequest::Save { suggested: expected.into() }],
                "readonly={read_only},report={reported},opaque={opaque}"
            );
            assert_eq!(identity(&app), before, "a cancelled suggestion does not rename or save the original");
            assert_eq!(app.session.active().unwrap().source_read_only, read_only);
        }
    }
}

#[test]
fn actual_xcf_bytes_and_wrappers_download_native_reports_without_confirming_a_browser_save() {
    use std::io::Write;
    // Independently specification-authored1x1 v3 RGB XCF, no source layers/channels.
    let mut plain = b"gimp xcf v003\0".to_vec();
    for value in [1u32, 1, 0, 0, 0, 0, 0] {
        plain.extend_from_slice(&value.to_be_bytes());
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&plain).unwrap();
    let wrapped = gzip.finish().unwrap();
    for (name, input) in [("plain.xcf", plain.clone()), ("misnamed.psd", plain), ("wrapped.xcf.gz", wrapped.clone()), ("wrapped-as-native.pcraft", wrapped)] {
        let original = input.clone();
        let writes = Rc::new(RefCell::new(Vec::<(String, Vec<u8>)>::new()));
        let captured = writes.clone();
        let mut app = PhotocraftApp::new(
            Session::new(),
            Services {
                import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map_err(|e| e.to_string()))),
                export: Some(Box::new(|doc, path, _| {
                    photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
                })),
                write: Some(Box::new(move |path, bytes| {
                    captured.borrow_mut().push((path.to_owned(), bytes.to_vec()));
                    Ok(())
                })),
                write_unconfirmed: true,
                ..Default::default()
            },
        );
        app.session.color.settings.ask_on_missing = false;
        app.open_bytes(name, &input).unwrap();
        assert!(app.session.active().unwrap().source_read_only);
        let report = app.session.active().unwrap().doc.metadata.import_conversion.clone();
        assert!(report.is_some());
        app.run("layer.new.layer", json!({})).unwrap();
        assert!(app.session.active().unwrap().is_dirty());
        let before_saved = app.session.active().unwrap().saved_revision;
        let expected = std::path::Path::new(name).with_extension(photocraft_format::EXTENSION).to_string_lossy().into_owned();
        // Browser behavior: no format picker; return exactly the suggested download name.
        let requests = Rc::new(RefCell::new(Vec::new()));
        let request_log = requests.clone();
        app.services.file_dialog = Some(Box::new(move |request, _, reply| {
            let crate::file_dialog::FileDialogRequest::Save { suggested } = &request else { panic!("expected Save request") };
            let suggested = suggested.clone();
            request_log.borrow_mut().push(request);
            reply.send(Some(crate::file_dialog::FileDialogAnswer::SaveTo(suggested)));
        }));
        app.save_as(None).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_eq!(*requests.borrow(), [crate::file_dialog::FileDialogRequest::Save { suggested: expected.clone() }]);
        let (download_name, bytes) = writes.borrow()[0].clone();
        assert_eq!(download_name, expected);
        assert!(photocraft_format::is_pcraft(&bytes));
        let reopened = photocraft_io::import(&download_name, &bytes).unwrap();
        assert_eq!(reopened.document.metadata.import_conversion, report);
        assert!(!reopened.source_read_only);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(expected.as_str()));
        assert_eq!(app.session.active().unwrap().saved_revision, before_saved);
        assert!(app.session.active().unwrap().is_dirty());
        assert!(app.has_unsaved_work());
        assert_eq!(input, original);
        app.services.write_unconfirmed = false;
        app.save_as(Some(expected)).unwrap();
        assert!(!app.session.active().unwrap().is_dirty());
        assert!(!app.has_unsaved_work());
        assert_eq!(app.session.active().unwrap().doc.metadata.import_conversion, report);
        // Native bytes opened through the browser have no writable path, even when their
        // misleading filename looks like PSD. A second download must preserve the report.
        let first_native_bytes = bytes.clone();
        app.open_bytes("native-reopen-misnamed.psd", &first_native_bytes).unwrap();
        let st = app.session.active().unwrap();
        assert!(!st.source_read_only);
        assert!(st.path.is_none());
        assert_eq!(st.doc.metadata.import_conversion, report);
        let second_expected = std::path::Path::new(&st.doc.name).with_extension(photocraft_format::EXTENSION).to_string_lossy().into_owned();
        app.run("layer.new.layer", json!({})).unwrap();
        let second_saved = app.session.active().unwrap().saved_revision;
        app.services.write_unconfirmed = true;
        app.save_as(None).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_eq!(requests.borrow().last(), Some(&crate::file_dialog::FileDialogRequest::Save { suggested: second_expected.clone() }));
        let (second_name, second_bytes) = writes.borrow().last().unwrap().clone();
        assert_eq!(second_name, second_expected);
        assert!(photocraft_format::is_pcraft(&second_bytes));
        let second_reopened = photocraft_io::import("second-generation-misnamed.psd", &second_bytes).unwrap();
        assert!(!second_reopened.source_read_only);
        assert_eq!(second_reopened.document.metadata.import_conversion, report);
        assert_eq!(app.session.active().unwrap().saved_revision, second_saved);
        assert!(app.session.active().unwrap().is_dirty());
        assert!(app.has_unsaved_work());
        assert_eq!(first_native_bytes, bytes);
    }
}
