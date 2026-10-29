use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn os_clipboard_bridge() {
    let os: OsClip = Arc::default();
    let (a, b) = (os.clone(), os.clone());
    let services = Services {
        clipboard_set_image: Some(Box::new(move |w: u32, h: u32, px: &[u8]| {
            *a.lock().unwrap() = Some((w, h, px.to_vec()));
            Ok(())
        })),
        clipboard_get_image: Some(Box::new(move || Ok(b.lock().unwrap().clone()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.run("select.rect", serde_json::json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
    app.run("edit.copy", serde_json::json!({})).unwrap();
    let (w, h, px) = os.lock().unwrap().clone().unwrap();
    assert_eq!((w, h, px.len()), (8, 4, 8 * 4 * 4));
    // Our own image comes back unchanged (no re-import, keeps the original position).
    assert!(!app.import_os_clipboard().unwrap());
    // Text copied elsewhere replaces our image: nothing to paste, not the stale image.
    let ours = os.lock().unwrap().take();
    assert!(!app.import_os_clipboard().unwrap());
    assert!(app.session.clipboard.is_none());
    *os.lock().unwrap() = ours;
    app.run("edit.copy", serde_json::json!({})).unwrap();
    // Another app puts a 3×2 red image on the clipboard: ⌘V pastes it.
    *os.lock().unwrap() = Some((3, 2, [255u8, 0, 0, 255].repeat(6)));
    app.run("edit.paste", serde_json::json!({})).unwrap();
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_eq!(surf.content_bounds().width(), 3);
}

#[test]
fn os_clipboard_honours_export_clipboard_preference() {
    let os: OsClip = Arc::default();
    let a = os.clone();
    let services = Services {
        clipboard_set_image: Some(Box::new(move |w: u32, h: u32, px: &[u8]| {
            *a.lock().unwrap() = Some((w, h, px.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.session.execute("prefs.set", serde_json::json!({"values": {"general.exportClipboard": false}})).unwrap();
    app.session.execute("file.new", serde_json::json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.run("select.rect", serde_json::json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
    app.run("edit.copy", serde_json::json!({})).unwrap();
    assert!(os.lock().unwrap().is_none(), "copy does not mirror to OS clipboard when export_clipboard is off");
}

type OsClip = Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>;

#[test]
fn failed_clipboard_reads_preserve_editable_contents_until_retry() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace", "file.newFromClipboard"] {
        let os = OsClip::default();
        let busy = Arc::new(AtomicBool::new(false));
        let (write, read, unavailable) = (os.clone(), os.clone(), busy.clone());
        let services = Services {
            clipboard_set_image: Some(Box::new(move |w, h, bytes| {
                *write.lock().unwrap() = Some((w, h, bytes.to_vec()));
                Ok(())
            })),
            clipboard_get_image: Some(Box::new(move || {
                if unavailable.load(Ordering::SeqCst) { Err("clipboard is busy".into()) } else { Ok(read.lock().unwrap().clone()) }
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.run("file.new", serde_json::json!({"width": 80, "height": 60})).unwrap();
        app.run("shape.create", serde_json::json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        app.run("file.new", serde_json::json!({"width": 80, "height": 60})).unwrap();
        let signature = app.os_clip_sig;
        assert!(signature.is_some());
        let before = app.session.active().unwrap().doc.walk().len();
        busy.store(true, Ordering::SeqCst);
        let result = crate::menus::invoke(&mut app, &egui::Context::default(), id, serde_json::json!({}));
        assert_eq!(result.unwrap_err(), "clipboard is busy", "{id}");
        assert!(app.ui.status_error);
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active().unwrap().doc.walk().len(), before);
        assert_eq!(app.os_clip_sig, signature);
        assert!(app.session.clipboard.as_ref().unwrap().layers.is_some());
        // The New dialog also reads the clipboard; a failed preset probe must preserve it.
        app.new_document_fields();
        assert_eq!(app.os_clip_sig, signature);
        assert!(app.session.clipboard.as_ref().unwrap().layers.is_some());
        // An automation error must not leave script events disabled by the dispatch guard.
        app.automation_input = true;
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
        assert!(app.run(id, serde_json::json!({})).is_err());
        assert!(app.session.prefs().script_events.enabled);
        app.automation_input = false;
        busy.store(false, Ordering::SeqCst);
        app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap();
        let state = app.session.active().unwrap();
        assert!(matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Shape(_)));
        assert!(!app.ui.status_error);
    }
}

#[test]
fn internal_only_clipboard_remains_usable_when_native_access_fails() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for export in [true, false] {
        for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace", "file.newFromClipboard"] {
            let os = OsClip::default();
            let unavailable = Arc::new(AtomicBool::new(true));
            let (read, failed) = (os.clone(), unavailable.clone());
            let services = Services {
                clipboard_set_image: Some(Box::new(|_, _, _| Err("clipboard unavailable".into()))),
                clipboard_get_image: Some(Box::new(move || {
                    if failed.load(Ordering::SeqCst) { Err("clipboard unavailable".into()) } else { Ok(read.lock().unwrap().clone()) }
                })),
                ..Default::default()
            };
            let mut app = PhotocraftApp::new(Session::new(), services);
            app.session.edit_prefs(|prefs| prefs.general.export_clipboard = export);
            app.run("file.new", serde_json::json!({"width": 80, "height": 60})).unwrap();
            app.run("shape.create", serde_json::json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
            app.run("edit.copy", serde_json::json!({})).unwrap();
            app.run("file.new", serde_json::json!({"width": 80, "height": 60})).unwrap();
            assert!(app.os_clip_sig.is_none() && !app.clip_external);
            let bounds = app.session.clipboard.as_ref().unwrap().bounds;
            assert_eq!(app.new_document_fields()["__preset"], "Clipboard");
            crate::menus::invoke(&mut app, &egui::Context::default(), id, serde_json::json!({})).unwrap();
            assert!(app.session.clipboard.as_ref().unwrap().layers.is_some());
            assert!(!app.ui.status_error);
            if id == "file.newFromClipboard" {
                assert_eq!(app.session.documents().len(), 3);
                let doc = &app.session.active().unwrap().doc;
                assert_eq!((doc.size.width, doc.size.height), (bounds.width(), bounds.height()));
            } else {
                assert_eq!(app.session.documents().len(), 2);
                let state = app.session.active().unwrap();
                assert!(matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Shape(_)));
            }
            app.run("edit.purge.clipboard", serde_json::json!({})).unwrap();
            assert!(app.session.clipboard.is_none());
            assert!(app.run("edit.paste", serde_json::json!({})).is_err(), "purged content must not be a fallback");
            // The fallback is only for failed reads: a later successful external image wins.
            *os.lock().unwrap() = Some((3, 2, [0u8, 0, 255, 255].repeat(6)));
            unavailable.store(false, Ordering::SeqCst);
            app.run("edit.paste", serde_json::json!({})).unwrap();
            assert!(app.clip_external && app.os_clip_sig.is_some());
            let clip = app.session.clipboard.as_ref().unwrap();
            assert!(clip.layers.is_none());
            assert_eq!((clip.bounds.width(), clip.bounds.height()), (3, 2));
        }
    }
}

#[test]
fn newer_unexported_copies_do_not_track_an_older_os_signature() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for imported in [true, false] {
        for disable_export in [true, false] {
            for copy_id in ["edit.copy", "edit.cut", "edit.copyMerged"] {
                let os = OsClip::default();
                let write_fails = Arc::new(AtomicBool::new(false));
                let read_fails = Arc::new(AtomicBool::new(false));
                let (write, read, failed_write, failed_read) = (os.clone(), os.clone(), write_fails.clone(), read_fails.clone());
                let services = Services {
                    clipboard_set_image: Some(Box::new(move |w, h, bytes| {
                        if failed_write.load(Ordering::SeqCst) {
                            return Err("clipboard unavailable".into());
                        }
                        *write.lock().unwrap() = Some((w, h, bytes.to_vec()));
                        Ok(())
                    })),
                    clipboard_get_image: Some(Box::new(move || {
                        if failed_read.load(Ordering::SeqCst) { Err("clipboard unavailable".into()) } else { Ok(read.lock().unwrap().clone()) }
                    })),
                    ..Default::default()
                };
                let mut app = PhotocraftApp::new(Session::new(), services);
                app.run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
                if imported {
                    *os.lock().unwrap() = Some((2, 2, [255u8, 0, 0, 255].repeat(4)));
                    assert!(app.import_os_clipboard().unwrap());
                } else {
                    app.run("edit.fill", serde_json::json!({"color": "#ff0000"})).unwrap();
                    app.run("edit.copy", serde_json::json!({})).unwrap();
                }
                let old_signature = app.os_clip_sig;
                assert!(old_signature.is_some() && !app.clip_unmirrored);
                app.run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
                app.run("edit.fill", serde_json::json!({"color": "#0000ff"})).unwrap();
                write_fails.store(true, Ordering::SeqCst);
                app.session.edit_prefs(|prefs| prefs.general.export_clipboard = !disable_export);
                app.run(copy_id, serde_json::json!({})).unwrap();
                let copied = app.session.clipboard.clone().unwrap();
                assert!(app.clip_unmirrored && !app.clip_external);
                assert_eq!(app.os_clip_sig, old_signature);
                assert_eq!(copied.surface.rgba(0, 0), [0.0, 0.0, 1.0, 1.0]);
                // An old successful export/import must not block the newer internal copy.
                read_fails.store(true, Ordering::SeqCst);
                app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap();
                assert_eq!(app.os_clip_sig, old_signature);
                assert!(app.clip_unmirrored && !app.clip_external);
                read_fails.store(false, Ordering::SeqCst);
                assert!(!app.import_os_clipboard().unwrap(), "unchanged old OS image must not overwrite the newer copy");
                assert_eq!(app.session.clipboard.as_ref().unwrap().surface.rgba(0, 0), copied.surface.rgba(0, 0));
                *os.lock().unwrap() = None;
                assert!(!app.import_os_clipboard().unwrap(), "empty OS clipboard must not discard an unexported copy");
                assert!(app.os_clip_sig.is_none() && app.clip_unmirrored);
                assert_eq!(app.session.clipboard.as_ref().unwrap().surface.rgba(0, 0), copied.surface.rgba(0, 0));
                // A different external image supersedes the internal copy and disables fallback.
                *os.lock().unwrap() = Some((3, 2, [0u8, 255, 0, 255].repeat(6)));
                assert!(app.import_os_clipboard().unwrap());
                assert!(app.clip_external && !app.clip_unmirrored);
                let clip = app.session.clipboard.as_ref().unwrap();
                assert_eq!((clip.bounds.width(), clip.bounds.height()), (3, 2));
                read_fails.store(true, Ordering::SeqCst);
                assert!(app.run("edit.paste", serde_json::json!({})).is_err());
            }
        }
    }
}

#[test]
fn failed_clipboard_reads_do_not_paste_an_external_or_missing_copy() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for external in [true, false] {
        let unavailable = Arc::new(AtomicBool::new(false));
        let failed = unavailable.clone();
        let services = Services {
            clipboard_get_image: Some(Box::new(move || {
                if failed.load(Ordering::SeqCst) { Err("clipboard unavailable".into()) } else { Ok(Some((3, 2, [255u8, 0, 0, 255].repeat(6)))) }
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.run("file.new", serde_json::json!({"width": 80, "height": 60})).unwrap();
        if external {
            assert!(app.import_os_clipboard().unwrap());
        }
        let signature = app.os_clip_sig;
        let before = app.session.active().unwrap().doc.walk().len();
        unavailable.store(true, Ordering::SeqCst);
        for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace", "file.newFromClipboard"] {
            let result = crate::menus::invoke(&mut app, &egui::Context::default(), id, serde_json::json!({}));
            assert_eq!(result.unwrap_err(), "clipboard unavailable", "{id}");
            assert!(app.ui.status_error);
            assert_eq!(app.session.documents().len(), 1);
            assert_eq!(app.session.active().unwrap().doc.walk().len(), before);
            assert_eq!(app.os_clip_sig, signature);
            assert_eq!(app.session.clipboard.is_some(), external);
            assert_eq!(app.clip_external, external);
        }
    }
}

/// An app whose OS clipboard is `os`, counting every read in `reads`.
fn app_with_os_clipboard(os: &OsClip, reads: &Arc<std::sync::atomic::AtomicUsize>) -> PhotocraftApp {
    let (b, n) = (os.clone(), reads.clone());
    let get = move || {
        n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(b.lock().unwrap().clone())
    };
    let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
    app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app
}

/// Paste is enabled from the clipboard service alone (no read); the explicit paste imports
/// the OS image, once, and centres it in the view like Photoshop does for foreign clipboard
/// content.
#[test]
fn external_clipboard_enables_and_centres_paste() {
    let (os, reads) = (OsClip::default(), Arc::default());
    let mut app = app_with_os_clipboard(&os, &reads);
    *os.lock().unwrap() = Some((4, 4, [255u8, 0, 0, 255].repeat(16)));
    assert!(app.session.clipboard.is_none());
    assert!(crate::menus::is_enabled(&app, "edit.paste"));
    assert!(crate::menus::is_enabled(&app, "edit.pasteSpecial.pasteInPlace"));
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "enablement never reads the clipboard");
    app.ui.views[0].center = [40.0, 20.0];
    let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
    assert_eq!(r["offset"], serde_json::json!([38, 18]), "external image is centred in the view: {r}");
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "one read per explicit paste");
    // Without a clipboard service Paste greys until something is copied in the app.
    let mut plain = PhotocraftApp::new(Session::new(), Services::default());
    plain.session.execute("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
    assert!(!crate::menus::is_enabled(&plain, "edit.paste"));
}

/// #368: an image copied in another app opens as a document of its own, from File › New
/// from Clipboard or from Paste with nothing open.
#[test]
fn os_clipboard_image_becomes_a_new_document() {
    let (os, reads) = (OsClip::default(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
    let (b, n) = (os.clone(), Arc::clone(&reads));
    let get = move || {
        n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(b.lock().unwrap().clone())
    };
    let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
    let ctx = egui::Context::default();
    // Listed right after File › New…, enabled without reading the clipboard.
    let items = crate::menus::menu_items(&app);
    let at = items.iter().position(|i| i.id == "file.new").unwrap();
    assert_eq!(items[at + 1].id, "file.newFromClipboard");
    assert!(items[at + 1].enabled && crate::menus::is_enabled(&app, "edit.paste"));
    assert!(!crate::menus::is_enabled(&app, "edit.pasteSpecial.pasteInPlace"), "Paste in Place needs a document");
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    // An empty clipboard is a quiet no-op.
    let r = crate::menus::invoke(&mut app, &ctx, "file.newFromClipboard", serde_json::json!({})).unwrap();
    assert_eq!(r["pasted"], serde_json::json!(false));
    assert!(app.session.documents().is_empty() && !app.ui.status_error);
    // Paste with nothing open makes the document.
    *os.lock().unwrap() = Some((5, 3, [0u8, 0, 255, 255].repeat(15)));
    crate::menus::invoke(&mut app, &ctx, "edit.paste", serde_json::json!({})).unwrap();
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height, d.layers.len()), (5, 3, 1));
    assert_eq!(app.ui.views.len(), 1, "the new document has a view");
    assert!(app.has_unsaved_work(), "closing after Paste must ask to save");
    // New from Clipboard with a document open adds another.
    crate::menus::invoke(&mut app, &ctx, "file.newFromClipboard", serde_json::json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert!(app.session.documents().iter().all(|d| d.is_dirty()));
    assert!(crate::discard_ui::intercept(&mut app, "file.close", &serde_json::json!({})), "Close must ask before discarding clipboard pixels");
}

#[test]
fn pasting_an_empty_or_non_image_os_clipboard_is_a_quiet_no_op() {
    let (os, reads) = (OsClip::default(), Arc::default());
    let mut app = app_with_os_clipboard(&os, &reads);
    let layers = app.session.active().unwrap().doc.layers.len();
    let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
    assert_eq!(r["pasted"], serde_json::json!(false));
    assert_eq!(app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap()["pasted"], serde_json::json!(false));
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
    assert!(!app.ui.status_error && !app.ui.status.is_empty());
}

#[test]
fn idle_frames_never_read_the_os_clipboard() {
    let (os, reads) = (OsClip::default(), Arc::<std::sync::atomic::AtomicUsize>::default());
    *os.lock().unwrap() = Some((4, 4, [255u8, 0, 0, 255].repeat(16)));
    let (b, n) = (os.clone(), reads.clone());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let get = move || {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(b.lock().unwrap().clone())
        };
        let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
        app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app
    });
    for _ in 0..20 {
        h.step();
    }
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "the OS clipboard is read only on an explicit paste");
    assert!(h.state().session.clipboard.is_none());
}

/// #2928: with the layer mask targeted, ⌘C copies the mask's values as grayscale, not the
/// layer's pixels — so ⌘V (targeting another layer's mask, #1035) moves the mask itself.
#[test]
fn copy_with_the_mask_targeted_copies_the_mask_not_the_pixels() {
    let (os, reads) = (OsClip::default(), Arc::default());
    let mut app = app_with_os_clipboard(&os, &reads);
    app.run("layer.new.layer", serde_json::json!({})).unwrap();
    app.run("layer.layerMask.hideAll", serde_json::json!({})).unwrap();
    // Black pixels, a white square in the mask: the copy must carry the mask.
    app.run("select.all", serde_json::json!({})).unwrap();
    app.run("edit.fill", serde_json::json!({"color": "#000000"})).unwrap();
    app.run("select.rect", serde_json::json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    app.run("edit.fill", serde_json::json!({"color": "#ffffff", "target": "mask"})).unwrap();
    app.run("select.deselect", serde_json::json!({})).unwrap();
    app.ui.mask_target = true;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.copy", serde_json::json!({})).unwrap();
    let clip = app.session.clipboard.clone().expect("the mask was copied");
    let st = app.session.active().unwrap();
    let (w, h) = (st.doc.size.width as i32, st.doc.size.height as i32);
    assert_eq!(
        [clip.bounds.x0, clip.bounds.y0, clip.bounds.width() as i32, clip.bounds.height() as i32],
        [0, 0, w, h],
        "the mask, whole — not the black pixels"
    );
    let px = clip.surface.rgba(6, 6);
    assert!(px[0] > 0.99 && px[3] > 0.99, "white mask values: {px:?}");
    assert!(clip.surface.rgba(20, 20)[0] < 0.01, "the hidden area copies as black");
}

/// #1035: with the layer mask targeted (its thumbnail clicked), ⌘V pastes the clipboard's
/// luminosity into the mask instead of making a layer.
#[test]
fn paste_with_the_mask_targeted_goes_into_the_mask() {
    let (os, reads) = (OsClip::default(), Arc::default());
    let mut app = app_with_os_clipboard(&os, &reads);
    app.run("layer.new.layer", serde_json::json!({})).unwrap();
    app.run("layer.layerMask.hideAll", serde_json::json!({})).unwrap();
    app.ui.mask_target = true;
    app.ui.views[0].center = [32.0, 32.0];
    *os.lock().unwrap() = Some((4, 4, [128u8, 128, 128, 255].repeat(16)));
    let layers = app.session.active().unwrap().doc.layer_count();
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer_count(), layers, "no new layer");
    let mask = st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
    assert!((mask.value(32, 32) - 128.0 / 255.0).abs() < 2.0 / 255.0, "the grey, centred in the view: {}", mask.value(32, 32));
    assert_eq!(mask.value(0, 0), 0.0, "the rest of the mask is unchanged");
}

/// #2166, measured in Photoshop 25.1: adding a layer mask targets it with white/black, and
/// the pixels and the mask each keep their own foreground/background pair.
#[test]
fn targeting_a_mask_swaps_to_its_own_colour_pair() {
    use serde_json::json;
    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    let masked = app.run("layer.new.layer", json!({})).unwrap()["layer"].clone();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    let pair = |app: &PhotocraftApp| (app.session.tools.foreground, app.session.tools.background);
    let target = |app: &mut PhotocraftApp, mask: bool| {
        app.ui.mask_target = mask;
        app.sync_views();
    };
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    assert!(app.ui.mask_target, "the new mask is targeted");
    assert_eq!(pair(&app), (WHITE, BLACK));
    target(&mut app, false);
    assert_eq!(pair(&app), (RED, BLUE), "the pixels get their colours back");
    target(&mut app, true);
    assert_eq!(pair(&app), (WHITE, BLACK));
    target(&mut app, false);
    app.run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
    target(&mut app, true);
    assert_eq!(pair(&app), (WHITE, BLACK));
    target(&mut app, false);
    assert_eq!(pair(&app), (GREEN, BLUE));
    // A layer without a mask edits its pixels: their pair.
    target(&mut app, true);
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(pair(&app), (GREEN, BLUE));
    app.run("layer.select", json!({"layer": masked})).unwrap();
    target(&mut app, true);
    assert_eq!(pair(&app), (WHITE, BLACK));
    app.run("layer.layerMask.delete", json!({})).unwrap();
    assert_eq!(pair(&app), (GREEN, BLUE), "deleting the targeted mask goes back to the pixels");
}

/// Changed pixels must win even when dimensions and all formerly sampled bytes match.
#[test]
fn changed_os_clipboard_pixels_reach_every_paste_route() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut fixtures = vec![(10, 10, [0u8, 255, 0, 255].repeat(100), [0u8, 0, 255, 255].repeat(100))];
    for channel in 1..4 {
        let before = vec![0, 128, 128, 255];
        let mut after = before.clone();
        after[channel] -= 64;
        fixtures.push((1, 1, before, after));
    }
    let before = [0u8, 255, 0, 255].repeat(400);
    let mut after = before.clone();
    after[1598] = 255; // beyond the first sample, between the old 997-byte samples
    fixtures.push((20, 20, before, after));
    let mut stale = Vec::new();
    for (w, h, before, after) in fixtures {
        for mirrored in [false, true] {
            for route in ["run", "menu", "inPlace", "new"] {
                let os = OsClip::default();
                let reads = Arc::new(AtomicUsize::new(0));
                let (write, read, count) = (os.clone(), os.clone(), reads.clone());
                let services = Services {
                    clipboard_set_image: Some(Box::new(move |w, h, bytes| {
                        *write.lock().unwrap() = Some((w, h, bytes.to_vec()));
                        Ok(())
                    })),
                    clipboard_get_image: Some(Box::new(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        Ok(read.lock().unwrap().clone())
                    })),
                    ..Default::default()
                };
                let mut app = PhotocraftApp::new(Session::new(), services);
                app.run("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
                if mirrored {
                    let bounds = photocraft_geom::Rect::new(7, 9, 7 + w as i32, 9 + h as i32);
                    let surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, bounds, &before);
                    app.session.clipboard = Some(photocraft_engine::edit_cmds::Clip { surface, bounds, layers: None, resolution: None, source: None });
                    app.export_os_clipboard();
                    assert!(!app.clip_external && !app.clip_unmirrored);
                } else {
                    *os.lock().unwrap() = Some((w, h, before.clone()));
                    assert!(app.import_os_clipboard().unwrap());
                }
                *os.lock().unwrap() = Some((w, h, after.clone()));
                reads.store(0, Ordering::SeqCst);
                let old_history = app.session.active().unwrap().history.past_len();
                let old_layers = app.session.active().unwrap().doc.layer_count();
                let id = match route {
                    "inPlace" => "edit.pasteSpecial.pasteInPlace",
                    "new" => "file.newFromClipboard",
                    _ => "edit.paste",
                };
                if route == "run" {
                    app.run(id, serde_json::json!({})).unwrap();
                } else {
                    crate::menus::invoke(&mut app, &egui::Context::default(), id, serde_json::json!({})).unwrap();
                }
                assert_eq!(reads.load(Ordering::SeqCst), 1, "one read for {route}");
                assert_eq!(app.session.documents().len(), if route == "new" { 2 } else { 1 });
                let state = app.session.active().unwrap();
                assert!(state.is_dirty());
                if route != "new" {
                    assert_eq!(state.history.past_len(), old_history + 1);
                    assert_eq!(state.doc.layer_count(), old_layers + 1);
                } else {
                    assert_eq!((state.doc.size.width, state.doc.size.height), (w, h));
                    assert_eq!(state.doc.layer_count(), 1);
                }
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
                let surface = layer.surface().unwrap();
                let bounds = surface.content_bounds();
                let mut pixels = vec![[0u8; 4]; (w * h) as usize];
                surface.read_rgba8_into(photocraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x0 + w as i32, bounds.y0 + h as i32), &mut pixels);
                let clip = app.session.clipboard.as_ref().unwrap();
                if pixels.into_iter().flatten().collect::<Vec<_>>() != after
                    || !app.clip_external
                    || clip.bounds != photocraft_geom::Rect::new(0, 0, w as i32, h as i32)
                    || clip.layers.is_some()
                    || clip.source.is_some()
                    || clip.resolution.is_some()
                {
                    stale.push(format!("{w}x{h} mirrored={mirrored} route={route} external={}", app.clip_external));
                }
                // Repeated unchanged probes are no-ops, while an explicit paste still adds a layer.
                assert!(!app.import_os_clipboard().unwrap());
                let layers = app.session.active().unwrap().doc.layer_count();
                app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc.layer_count(), layers + 1);
                app.run("edit.undo", serde_json::json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc.layer_count(), layers);
            }
        }
    }
    assert!(stale.is_empty(), "stale pasted pixels or classification: {stale:?}");
}

#[test]
fn unchanged_os_mirrors_preserve_rich_high_depth_copies() {
    for depth in [16, 32] {
        let os = OsClip::default();
        let (write, read) = (os.clone(), os.clone());
        let mut app = PhotocraftApp::new(
            Session::new(),
            Services {
                clipboard_set_image: Some(Box::new(move |w, h, bytes| {
                    *write.lock().unwrap() = Some((w, h, bytes.to_vec()));
                    Ok(())
                })),
                clipboard_get_image: Some(Box::new(move || Ok(read.lock().unwrap().clone()))),
                ..Default::default()
            },
        );
        app.run("file.new", serde_json::json!({"width": 80, "height": 60, "depth": depth, "resolution": 240})).unwrap();
        app.run("shape.create", serde_json::json!({"kind": "rect", "rect": [10, 12, 20, 16], "fill": "#00ff00"})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        let copied = app.session.clipboard.clone().unwrap();
        assert!(copied.layers.is_some() && copied.source.is_some() && copied.resolution.is_some());
        assert_ne!(copied.surface.format(), photocraft_color::PixelFormat::RGBA8);
        for _ in 0..2 {
            assert!(!app.import_os_clipboard().unwrap());
            assert!(!app.clip_external && !app.clip_unmirrored);
            let clip = app.session.clipboard.as_ref().unwrap();
            assert_eq!(clip.bounds, copied.bounds);
            assert_eq!(clip.surface.format(), copied.surface.format());
            assert_eq!(clip.surface.rgba(15, 15), copied.surface.rgba(15, 15));
            assert_eq!(clip.source, copied.source);
            assert_eq!(clip.resolution, copied.resolution);
            assert!(clip.layers.is_some());
            let steps = app.session.active().unwrap().history.past_len();
            app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap();
            let state = app.session.active().unwrap();
            assert_eq!(state.history.past_len(), steps + 1);
            let pasted = state.doc.layer(state.active_layer.unwrap()).unwrap();
            assert!(matches!(pasted.content, photocraft_doc::LayerContent::Shape(_)));
            assert_eq!(pasted.surface().unwrap().content_bounds(), copied.bounds);
        }
        app.run("file.newFromClipboard", serde_json::json!({})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.resolution_dpi, copied.resolution.unwrap());
        assert_eq!(doc.depth, copied.surface.format().sample);
        assert_eq!((doc.size.width, doc.size.height), (copied.bounds.width(), copied.bounds.height()));
        assert!(app.session.clipboard.as_ref().unwrap().layers.is_some());
    }
}

#[test]
fn clipboard_dimensions_and_invalid_images_keep_existing_paste_policy() {
    let (os, reads) = (OsClip::default(), Arc::default());
    let mut app = app_with_os_clipboard(&os, &reads);
    *os.lock().unwrap() = Some((1, 2, [0u8, 255, 0, 255].repeat(2)));
    assert!(app.import_os_clipboard().unwrap());
    // The same bytes with a different shape are a different external image.
    *os.lock().unwrap() = Some((2, 1, [0u8, 255, 0, 255].repeat(2)));
    app.run("file.newFromClipboard", serde_json::json!({})).unwrap();
    let state = app.session.active().unwrap();
    assert_eq!((state.doc.size.width, state.doc.size.height), (2, 1));
    assert!(app.clip_external);
    for image in [(0, 1, vec![]), (1, 0, vec![]), (1, 1, vec![0; 3])] {
        *os.lock().unwrap() = Some((1, 1, vec![0, 255, 0, 255]));
        assert!(app.import_os_clipboard().unwrap());
        *os.lock().unwrap() = Some(image);
        let before = app.session.active().unwrap().history.past_len();
        let result = app.run("edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(result["pasted"], false);
        assert!(app.session.clipboard.is_none() && app.os_clip_sig.is_none());
        assert!(!app.clip_external && !app.ui.status_error);
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
    }
}
