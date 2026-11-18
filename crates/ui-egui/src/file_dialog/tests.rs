use super::*;
use crate::{Services, menus};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use craft::automation::actions::ActionStep;
use craft::host::{FileFilter, ScriptedFiles};
use craft::ui::dialog::DialogId;

/// The paths the app's writer was given.
type Written = Rc<RefCell<Vec<String>>>;

/// An app with an untitled document, whose file dialogs stay open until the test answers them
/// (`answer`), and whose writer records the paths it wrote.
fn app() -> (PhotocraftApp, Arc<ScriptedFiles>, Written) {
    app_with(Arc::new(ScriptedFiles::held()))
}

/// [`app`] over `files`, for a test whose dialogs answer themselves.
fn app_with(files: Arc<ScriptedFiles>) -> (PhotocraftApp, Arc<ScriptedFiles>, Written) {
    let written: Written = Default::default();
    let w = written.clone();
    let services = Services {
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        files: Some(files.clone()),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, files, written)
}

/// The user answers the newest open dialog.
fn answer(files: &ScriptedFiles, picked: Picked) {
    files.answer(picked).expect("a dialog is open");
}

/// The user cancels the newest open dialog.
fn cancel(files: &ScriptedFiles) {
    answer(files, Picked::Cancelled);
}

/// The answer of a dialog that chose the file or folder `path`.
fn chose(path: &str) -> Picked {
    Picked::Paths(vec![PathBuf::from(path)])
}

/// The file a Save request suggests: its folder and name together.
fn suggested(request: &PickRequest) -> String {
    let name = request.file_name.clone().unwrap_or_default();
    match &request.directory {
        Some(dir) => dir.join(name).to_string_lossy().into_owned(),
        None => name,
    }
}

/// The file the one open Save dialog suggests.
fn asked_save(files: &ScriptedFiles) -> String {
    assert_eq!(files.open(), 1, "one dialog is open");
    let requests = files.requests();
    let last = requests.last().expect("a request");
    assert_eq!(last.kind, PickKind::Save, "{last:?}");
    suggested(last)
}

/// The newest request the file service was given.
fn last_request(files: &ScriptedFiles) -> PickRequest {
    files.requests().pop().expect("a request")
}

#[test]
fn pdn_save_requests_a_native_copy_without_overwriting_the_source() {
    let (mut app, files, written) = app();
    let st = app.session.active_mut().unwrap();
    st.path = Some("/pics/layers.PDN".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(asked_save(&files), "/pics/layers.pcraft");
    answer(&files, chose("/pics/layers.pcraft"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["/pics/layers.pcraft"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/layers.pcraft"));
}

/// A real flat JPEG import, optionally edited to contain another layer. A filename without a
/// path is the web/download case; desktop opens also retain their source location.
fn jpeg_app(path: Option<&str>, layered: bool) -> (PhotocraftApp, Arc<ScriptedFiles>, Written) {
    let (mut app, files, written) = app();
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "My image.JPEG", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("My image.JPEG", &bytes).unwrap().document;
    app.session.add_document(doc, path.map(str::to_string));
    if layered {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    (app, files, written)
}

// PLAN: assumed dialogs-w59: `app.parked_save: Option<ParkedSave>` with `path`, `copy` and `record` as the old `jpeg_options` had, and JPEG Options hosted as `format_prompts::HostedFormatOptions` with `quality` and `format` fields.
/// What the open JPEG Options asks about: the file, whether the save is a copy, and the command
/// it records as.
fn parked(app: &PhotocraftApp) -> Option<(String, bool, &'static str)> {
    app.parked_save.as_ref().map(|p| (p.path.clone(), p.copy, p.record))
}

/// The id of the open JPEG Options dialog.
fn options_dialog(app: &PhotocraftApp) -> Option<DialogId> {
    let host = &app.framework.dialogs;
    host.stack().ids().find(|id| host.get_by_id::<craft::ui::patterns::form_dialogs::HostedFormatOptions>(*id).is_some())
}

/// The value of a field of the open JPEG Options dialog.
fn option_field(app: &PhotocraftApp, field: &str) -> Value {
    let id = options_dialog(app).expect("JPEG Options is open");
    let dialog = app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).expect("listed");
    dialog.fields.expect("the dialog reports its fields")[field].clone()
}

/// The user sets a field of the open JPEG Options dialog.
fn set_option(app: &mut PhotocraftApp, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    let id = options_dialog(app).expect("JPEG Options is open");
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap_or_else(|e| panic!("JPEG Options takes `{field}`: {e}"));
}

/// The user answers OK in the open JPEG Options.
fn confirm_options(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::format_prompts::confirm(app, &ctx);
}

/// The user cancels the open JPEG Options: the save it held is dropped.
fn cancel_options(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let id = options_dialog(app).expect("JPEG Options is open");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    crate::format_prompts::take(app, &ctx);
}

#[test]
fn layered_flat_source_save_defaults_to_psd_without_overwriting_source() {
    let ctx = egui::Context::default();
    for command in ["file.save", "file.saveAs"] {
        let (mut app, files, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(asked_save(&files), "/pics/My image.psd", "{command}");
        assert!(written.borrow().is_empty(), "asking where to save must not overwrite the JPEG");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
        answer(&files, chose("/pics/My image.psd"));
        app.poll_file_dialog();
        assert_eq!(*written.borrow(), ["/pics/My image.psd"]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.psd"));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
    }
}

#[test]
fn layered_filename_only_save_defaults_to_psd() {
    let (mut app, files, written) = jpeg_app(None, true);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    assert_eq!(asked_save(&files), "My image.psd");
    answer(&files, chose("My image.psd"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["My image.psd"]);
}

#[test]
fn save_dialog_preserves_flat_and_existing_layered_format_defaults() {
    let ctx = egui::Context::default();
    for (path, layered) in [
        ("/pics/My image.JPEG", false),
        ("/pics/My image.PSD", true),
        ("/pics/My image.psb", true),
        ("/pics/My image.pcraft", true),
        ("/pics/My image.TIF", true),
        ("/pics/My image.tiff", true),
    ] {
        let (mut app, files, written) = jpeg_app(Some(path), layered);
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        assert_eq!(asked_save(&files), path);
        cancel(&files);
        app.poll_file_dialog();
        assert!(written.borrow().is_empty());
    }
}

/// Checked in Photoshop: once some pixels of a JPEG are deleted (transparency a JPEG can't keep),
/// Save asks where to save, defaulting to PNG; while the layer is still opaque the JPEG stays the
/// default (#2223).
#[test]
fn a_jpeg_with_transparency_is_offered_as_png() {
    let ctx = egui::Context::default();
    let (mut app, files, written) = jpeg_app(Some("/pics/photo.jpg"), false);
    app.run("layer.new.layerFromBackground", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    assert_eq!(asked_save(&files), "/pics/photo.jpg", "opaque: still a JPEG");
    cancel(&files);
    app.poll_file_dialog();
    // Not the Background any more but still opaque: the JPEG keeps it, so Save writes back
    // after JPEG Options.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(files.open(), 0, "no Save dialog");
    confirm_options(&mut app);
    assert_eq!(*written.borrow(), ["/pics/photo.jpg"]);
    written.borrow_mut().clear();
    // Delete some pixels: the lone layer is now partly transparent.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    for command in ["file.save", "file.saveAs"] {
        menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(asked_save(&files), "/pics/photo.png", "{command}");
        cancel(&files);
        app.poll_file_dialog();
    }
    assert!(written.borrow().is_empty());
    // Saved as the PNG it suggests, the document lives there and keeps its transparency.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    answer(&files, chose("/pics/photo.png"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["/pics/photo.png"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/photo.png"));
}

/// A Grayscale layer without an alpha channel is opaque, never read past its colour samples.
#[test]
fn a_layer_without_alpha_is_opaque() {
    let doc = photocraft_doc::Document::new("gray", photocraft_geom::Size::new(3, 3), photocraft_doc::ColorMode::Grayscale, photocraft_color::SampleType::U8);
    let layer = photocraft_doc::Layer::raster("gray", photocraft_color::PixelFormat::GRAY8);
    assert!(crate::opaque_layer(&doc, &layer));
}

#[test]
fn transparent_png_keeps_its_format_until_its_lone_layer_becomes_smart() {
    let (mut app, files, written) = app();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layers[0].surface_mut().unwrap().write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.0]);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "transparent.PNG", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("transparent.PNG", &bytes).unwrap().document;
    assert_eq!(doc.layers.len(), 1);
    assert!(!doc.layers[0].locks.transparency, "transparent PNG imports as an unlocked raster layer");
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0)[3], 0.0);
    app.session.add_document(doc, Some("/pics/transparent.PNG".into()));
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    assert_eq!(asked_save(&files), "/pics/transparent.PNG");
    cancel(&files);
    app.poll_file_dialog();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
    assert!(matches!(app.session.active().unwrap().doc.layers[0].content, photocraft_doc::LayerContent::Smart(_)));
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    assert_eq!(asked_save(&files), "/pics/transparent.psd");
    assert!(written.borrow().is_empty());
    cancel(&files);
    app.poll_file_dialog();
}

#[test]
fn layered_save_respects_explicit_jpeg_choices() {
    let ctx = egui::Context::default();
    let (mut app, files, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    // The file service settles the typed extension's case, so the answer is already lowercase.
    answer(&files, chose("/exports/chosen.jpeg"));
    app.poll_file_dialog();
    // JPEG Options asks first (FILE-215-19, #5563). The JPEG can't hold the layers, so it is a
    // copy and the document keeps its file (#2550).
    assert_eq!(parked(&app).map(|(path, copy, _)| (path, copy)), Some(("/exports/chosen.jpeg".to_string(), true)));
    assert!(written.borrow().is_empty());
    confirm_options(&mut app);
    assert_eq!(*written.borrow(), ["/exports/chosen.jpeg"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
    // Explicit command paths carry the same choice and bypass the picker: without a JPEG answer
    // JPEG Options still asks, and a `quality` answers it.
    for command in ["file.save", "file.saveAs"] {
        let (mut app, files, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({"path": "/exports/chosen.jpg"})).unwrap();
        assert!(!app.file_dialog_open() && files.requests().is_empty());
        assert!(parked(&app).is_some_and(|(_, copy, _)| copy), "{command}");
        confirm_options(&mut app);
        assert_eq!(*written.borrow(), ["/exports/chosen.jpg"]);

        let (mut app, files, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({"path": "/exports/chosen.jpg", "quality": 8})).unwrap();
        assert!(!app.file_dialog_open() && files.requests().is_empty() && parked(&app).is_none(), "{command}");
        assert_eq!(*written.borrow(), ["/exports/chosen.jpg"]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
    }
}

/// File › Save on a flat JPEG it can't write back in place (here: transparency) asks for its
/// name; confirming its own file asks JPEG Options and, once answered, records a plain Save that
/// writes the file an action is played on (#3531).
#[test]
fn plain_save_of_a_flat_jpeg_asks_jpeg_options_and_records_a_plain_save() {
    let ctx = egui::Context::default();
    let (mut app, files, written) = jpeg_app(Some("/pics/photo.jpg"), false);
    app.run("layer.new.layerFromBackground", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    answer(&files, chose("/pics/photo.jpg"));
    app.poll_file_dialog();
    assert_eq!(parked(&app).map(|(_, copy, record)| (record, copy)), Some(("file.save", false)));
    assert!(written.borrow().is_empty());
    confirm_options(&mut app);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/photo.jpg"]);
    assert_eq!(app.session.actions.list.action(0).unwrap().steps, [ActionStep::new("file.save", json!({}))]);
}

/// A plain Save confirmed in the dialog keeps a changed JPEG Options answer in its recorded step,
/// and playing it writes the active flat JPEG with that answer, without asking.
#[test]
fn plain_save_of_a_flat_jpeg_records_and_replays_a_changed_quality() {
    let ctx = egui::Context::default();
    let (mut app, files, written) = jpeg_app(Some("/pics/photo.jpg"), false);
    let qualities: Rc<RefCell<Vec<Option<u8>>>> = Rc::default();
    let q = qualities.clone();
    app.services.export = Some(Box::new(move |_: &photocraft_doc::Document, _: &str, s: &crate::ExportSettings| {
        q.borrow_mut().push(s.jpeg_quality);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    // Some pixels deleted: no longer saved in place (#2223), so Save goes through the dialog.
    app.run("layer.new.layerFromBackground", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    answer(&files, chose("/pics/photo.jpg"));
    app.poll_file_dialog();
    set_option(&mut app, "quality", json!(3));
    set_option(&mut app, "format", json!("progressive"));
    confirm_options(&mut app);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list.action(0).unwrap().steps, [ActionStep::new("file.save", json!({"quality": 3, "jpegFormat": "progressive"}))]);
    // Played on another flat JPEG: written over itself at quality 3, with no dialog or prompt.
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "other.jpg", &Default::default()).unwrap().bytes;
    let index = app.session.add_document(photocraft_io::import("other.jpg", &bytes).unwrap().document, Some("/pics/other.jpg".into()));
    app.session.set_active(index);
    written.borrow_mut().clear();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(parked(&app).is_none(), "playback does not ask");
    assert_eq!(files.open(), 0, "no Save dialog");
    assert_eq!(*written.borrow(), ["/pics/other.jpg"]);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(3.0)));
}

#[test]
fn menu_place_starts_transform_for_each_chosen_image() {
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        let (mut app, files, _) = app();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
        let ctx = egui::Context::default();
        let initial_layers = app.session.active().unwrap().doc.layer_count();
        let initial_history = app.session.active().unwrap().history.past_len();
        let linked_dir = (command == "file.placeLinked").then(|| {
            let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("photocraft-menu-place-{}-{unique}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            dir
        });
        for index in 0..2 {
            menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
            let picked = if let Some(dir) = &linked_dir {
                let path = dir.join(format!("drawing-{index}.png"));
                std::fs::write(&path, &bytes).unwrap();
                Picked::Paths(vec![path])
            } else {
                Picked::Contents(vec![(format!("drawing-{index}.png"), bytes.clone())])
            };
            answer(&files, picked);
            app.poll_file_dialog();
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layer_count(), initial_layers + index + 1);
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, format!("drawing-{index}"));
            let transform = app.ui.transform.as_ref().expect("File > Place must offer transform handles after the picker completes");
            assert_eq!(transform.made, Some(crate::state::MadeLayer::Place));
            if index == 0 {
                let layer = st.active_layer.unwrap();
                let photocraft_doc::LayerContent::Smart(smart) = &st.doc.layer(layer).unwrap().content else { panic!("placed Smart Object") };
                let original_scale = smart.transform.m[0];
                // PLAN: assumed tools-w2: the open session is `app.ui.transform.session`, whose box `state()` is edited through `apply(TransformEdit::Box(..), now)`.
                let transform = app.ui.transform.as_mut().unwrap();
                let mut doubled = *transform.session.state();
                let left = doubled.quad[0].x;
                for corner in &mut doubled.quad {
                    corner.x = left + (corner.x - left) * 2.0;
                }
                let before = craft::behavior::Mixed::Same(*transform.session.state());
                transform
                    .session
                    .apply(craft::draw::ui::transform::TransformEdit::Box(craft::behavior::EditEvent::set(before, doubled, "Resize".into(), None)), 0.0);
                crate::transform_tool::commit(&mut app);
                assert!(app.ui.transform.is_none(), "resizing the placed image must commit");
                let st = app.session.active().unwrap();
                let photocraft_doc::LayerContent::Smart(smart) = &st.doc.layer(layer).unwrap().content else { panic!("placed Smart Object") };
                assert!((smart.transform.m[0] - original_scale * 2.0).abs() < 1e-8, "placement width must double");
                assert_eq!(app.session.active().unwrap().history.past_len(), initial_history + 1);
            } else {
                crate::transform_tool::cancel(&mut app);
                assert_eq!(app.session.active().unwrap().doc.layer_count(), initial_layers + 1);
                assert_eq!(app.session.active().unwrap().history.past_len(), initial_history + 1);
            }
        }
        if let Some(dir) = linked_dir {
            for index in 0..2 {
                std::fs::remove_file(dir.join(format!("drawing-{index}.png"))).unwrap();
            }
            std::fs::remove_dir(dir).unwrap();
        }
    }
}

/// #10031: File › Place decodes in the background; Free Transform starts when the layer lands, and
/// a second place is refused while the first runs.
#[test]
fn menu_place_runs_in_the_background_and_starts_transform_when_it_lands() {
    let (mut app, files, _) = app();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
    app.background_jobs = true;
    let ctx = egui::Context::default();
    let layers = app.session.active().unwrap().doc.layer_count();
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("drawing.png".into(), bytes.clone())]));
    app.poll_file_dialog();
    let place = app.jobs.place.clone().expect("being placed");
    assert_eq!(place.name, "drawing.png");
    assert!(app.ui.transform.is_none(), "Free Transform waits for the layer");
    assert_eq!(app.session.active().unwrap().doc.layer_count(), layers);
    // Another place is refused while this one runs, and the first one stays tracked.
    assert!(crate::jobs_ui::start_place_bytes(&mut app, "other.png", bytes, None, &serde_json::json!({})).is_err());
    assert_eq!(app.jobs.place.as_ref().map(|p| p.job), Some(place.job));
    let t = std::time::Instant::now();
    while app.jobs.place.is_some() {
        assert!(t.elapsed() < std::time::Duration::from_secs(30), "the place did not finish");
        crate::jobs_ui::tick(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer_count(), layers + 1);
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "drawing");
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
}

#[test]
fn cancelled_or_invalid_menu_place_does_not_start_transform() {
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        for reply in [Picked::Cancelled, Picked::Contents(vec![("broken.png".into(), b"not an image".to_vec())])] {
            let (mut app, files, _) = app();
            let ctx = egui::Context::default();
            let before = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
            answer(&files, reply);
            app.poll_file_dialog();
            assert!(app.ui.transform.is_none());
            assert_eq!(app.session.active().unwrap().doc, before);
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn menu_place_preserves_an_unfinished_transform() {
    for start_before_picker in [true, false] {
        let (mut app, files, _) = app();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("layer.new.layerFromBackground", json!({})).unwrap();
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
        let ctx = egui::Context::default();
        if !start_before_picker {
            menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
        }
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let transform = app.ui.transform.clone();
        let doc = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        if start_before_picker {
            assert!(menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).is_err());
            assert!(!app.file_dialog_open());
        } else {
            answer(&files, Picked::Contents(vec![("source.png".into(), bytes)]));
            app.poll_file_dialog();
        }
        assert_eq!(app.ui.transform, transform);
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn open_starts_in_the_last_used_folder() {
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    app.ui.recent_files = vec!["/pics/cat.psd".into(), "/old/dog.png".into()];
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    let asked = last_request(&files);
    assert_eq!(asked.kind, PickKind::OpenMany);
    assert_eq!(asked.directory.as_deref(), Some(Path::new("/pics")), "{asked:?}");
    assert!(asked.filters.len() > 1, "File › Open offers every format and one type each");
}

#[test]
fn ask_folder_uses_the_async_dialog_and_returns_the_selected_path() {
    let (mut app, files, _) = app();
    app.ui.recent_files = vec!["/pics/cat.psd".into()];
    let selected = Rc::new(RefCell::new(None));
    let result = selected.clone();

    assert_eq!(
        app.ask_folder(move |_, path| {
            *result.borrow_mut() = Some(path);
            Ok(Value::Null)
        })
        .unwrap(),
        json!({"fileDialog": "folder"})
    );
    let asked = last_request(&files);
    assert_eq!((asked.kind, asked.directory.as_deref()), (PickKind::Folder, Some(Path::new("/pics"))));

    answer(&files, chose("/pics/frames"));
    app.poll_file_dialog();
    assert_eq!(selected.borrow().as_deref(), Some("/pics/frames"));
}

#[test]
fn filtered_picker_requests_only_the_given_extensions() {
    let (mut app, files, _) = app();
    app.pick_file_bytes_filtered(&["cube", "3dl", "look"], |_, _, _| Ok(Value::Null)).unwrap();
    let asked = last_request(&files);
    assert_eq!(asked.kind, PickKind::Open);
    assert_eq!(asked.filters, [FileFilter::new("Supported Files", &["cube", "3dl", "look"])]);
}

#[test]
fn scripts_browse_lists_only_script_files() {
    let (mut app, files, _) = app();
    menus::invoke(&mut app, &egui::Context::default(), "file.scripts.browse", json!({})).unwrap();
    let asked = last_request(&files);
    assert_eq!(asked.kind, PickKind::Open);
    let [filter] = asked.filters.as_slice() else { panic!("one type, got {:?}", asked.filters) };
    assert_eq!(filter.extensions, ["psjs", "ccjs", "jsx", "js", "jsxbin", "json", "txt", "pcdroplet"]);
}

#[test]
fn scripts_browse_extendscript_picks_report_the_error() {
    // Photoshop scripts stay in the picker (matching its filter) and choosing one shows why it won't run.
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.scripts.browse", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("resize.jsx".into(), b"alert('hi');".to_vec())]));
    app.poll_file_dialog();
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("not Photoshop ExtendScript"), "{}", app.ui.status);
}

#[test]
fn save_and_export_dialogs_start_beside_the_document() {
    let (mut app, files, _) = app();
    let ctx = app.ctx().clone();
    let dir = std::env::temp_dir().join("PhotoCraft projects").join("图像");
    let source = dir.join("original.psd");
    app.session.active_mut().unwrap().path = Some(source.to_string_lossy().into_owned());
    for command in ["file.saveAs", "file.export.exportAs", "file.export.saveForWebLegacy"] {
        let r = menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        if let Some(dialog) = r.get("dialog") {
            let id: DialogId = serde_json::from_value(dialog.clone()).expect("a dialog id");
            crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        }
        let asked = asked_save(&files);
        assert_eq!(Path::new(&asked).parent(), Some(dir.as_path()), "{command}");
        if command == "file.saveAs" {
            assert_eq!(Path::new(&asked), source);
        }
        cancel(&files);
        app.poll_file_dialog();
    }
}

/// The path `pick_save` suggests for `name` in `dir` on this platform: it joins with `Path::join`,
/// which uses `\` on Windows, so the expected values must be built the same way (#2745).
fn joined(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

/// #1826: after the user navigates a save or export dialog elsewhere, the document's next dialog
/// starts there. Save As of a saved document still suggests its own path; other documents and
/// closed documents are unaffected.
#[test]
fn save_dialogs_remember_where_each_document_was_last_saved() {
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    let first = app.session.active().unwrap().doc.id;
    app.session.active_mut().unwrap().path = Some("/proj/original.psd".into());
    // An export starts beside the document; the user saves it under /renders instead.
    app.pick_save("result.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/proj", "result.png"));
    answer(&files, chose("/renders/result.png"));
    app.poll_file_dialog();
    // The next export starts in /renders; Save As still offers the document's own path.
    app.pick_save("again.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/renders", "again.png"));
    cancel(&files);
    app.poll_file_dialog();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    assert_eq!(asked_save(&files), "/proj/original.psd");
    cancel(&files);
    app.poll_file_dialog();
    // A cancelled dialog and an explicit directory change nothing.
    app.pick_save("/elsewhere/x.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), "/elsewhere/x.png");
    cancel(&files);
    app.poll_file_dialog();
    app.pick_save("third.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/renders", "third.png"));
    cancel(&files);
    app.poll_file_dialog();
    // Another document has its own memory: untitled, it starts wherever the shell defaults.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let second = app.session.active().unwrap().doc.id;
    assert_ne!(first, second);
    app.pick_save("fresh.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), "fresh.png");
    answer(&files, chose("/other/fresh.png"));
    app.poll_file_dialog();
    assert_eq!(app.save_dirs.get(&second).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/other"));
    assert_eq!(app.save_dirs.get(&first).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/renders"));
    // The folder is remembered for the document the dialog was asked for, not the active one.
    app.session.set_active(0);
    app.pick_save("swap.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/renders", "swap.png"));
    app.session.set_active(1);
    answer(&files, chose("/moved/swap.png"));
    app.poll_file_dialog();
    assert_eq!(app.save_dirs.get(&first).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/moved"));
    assert_eq!(app.save_dirs.get(&second).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/other"));
    // Closing a document forgets its folder at the next dialog.
    app.run("file.close", json!({"document": 1})).unwrap();
    app.pick_save("last.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/moved", "last.png"));
    assert!(!app.save_dirs.contains_key(&second));
    cancel(&files);
    app.poll_file_dialog();
}

#[test]
fn save_suggestions_preserve_explicit_directories_and_untitled_names() {
    let (mut app, files, _) = app();
    for (document_path, suggested) in
        [(None, "Untitled.png"), (Some("project/source.psd"), "exports/result.png"), (Some("project/source.psd"), "/other/result.png")]
    {
        app.session.active_mut().unwrap().path = document_path.map(str::to_string);
        app.pick_save(suggested, |_, _| Ok(Value::Null)).unwrap();
        assert_eq!(asked_save(&files), suggested);
        cancel(&files);
        app.poll_file_dialog();
    }
}

#[test]
fn the_answer_goes_to_the_document_it_was_asked_for() {
    let (mut app, files, written) = app();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.session.set_active(1);
    answer(&files, chose("/pics/first.psd"));
    app.poll_file_dialog();
    let paths: Vec<_> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/pics/first.psd".to_string()), None]);
    assert_eq!(app.session.active_index(), Some(1), "a completed save preserves the selected tab");
    assert_eq!(app.session.documents()[0].doc.name, "first.psd");
    assert_ne!(app.session.documents()[1].doc.name, "first.psd");
    // Once that document is closed, the answer has nothing to save.
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.run("file.close", json!({"document": 0})).unwrap();
    answer(&files, chose("/pics/again.psd"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["/pics/first.psd"]);
    assert!(app.ui.status_error && app.session.documents()[0].path.is_none());
}

#[test]
fn opening_another_document_before_the_save_answer_keeps_its_identity() {
    let (mut app, files, written) = app();
    let first_id = app.active_doc_id().unwrap();
    app.save_as(None).unwrap();
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-2"})).unwrap();
    let second_id = app.active_doc_id().unwrap();
    app.run("document.move", json!({"document": 0, "to": 1})).unwrap();
    answer(&files, chose("/pics/保存 first.psd"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["/pics/保存 first.psd"]);
    let first = app.session.documents().iter().find(|s| s.doc.id == first_id).unwrap();
    assert_eq!(first.doc.name, "保存 first.psd");
    assert_eq!(first.path.as_deref(), Some("/pics/保存 first.psd"));
    assert_eq!(app.active_doc_id().unwrap(), second_id);
    assert_eq!(app.session.active().unwrap().doc.name, "Untitled-2");
}

#[test]
fn failed_deferred_save_preserves_both_identities_and_the_selected_tab() {
    let (mut app, files, written) = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.save_as(None).unwrap();
    app.run("file.new", json!({"width": 4, "height": 4, "name": "second"})).unwrap();
    let before: Vec<_> = app.session.documents().iter().map(|d| (d.doc.name.clone(), d.path.clone(), d.revision, d.saved_revision)).collect();
    app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
    answer(&files, chose("failed.psd"));
    app.poll_file_dialog();
    let after: Vec<_> = app.session.documents().iter().map(|d| (d.doc.name.clone(), d.path.clone(), d.revision, d.saved_revision)).collect();
    assert_eq!(after, before);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.ui.status_error && app.ui.status == "disk full");
    assert!(written.borrow().is_empty());
}

#[test]
fn an_answer_of_the_wrong_kind_is_an_error_not_a_crash() {
    let (mut app, files, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    // A save answered with a file's contents, which a Save never gets.
    answer(&files, Picked::Contents(vec![("a.psd".into(), Vec::new())]));
    app.poll_file_dialog();
    assert!(app.ui.status_error && app.ui.status == UNEXPECTED);
    assert!(written.borrow().is_empty());
    assert!(!app.file_dialog_open());
}

/// The app with a painted layer converted to a smart object and a duplicate instance of it.
fn smart_app() -> (PhotocraftApp, Arc<ScriptedFiles>, Written, [u64; 2]) {
    let (mut app, files, written) = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let a = app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let b = app.run("layer.duplicate", json!({"layer": a})).unwrap()["layer"].as_u64().unwrap();
    app.session.select_layer(photocraft_doc::LayerId(a)).unwrap();
    (app, files, written, [a, b])
}

fn smart_source(app: &PhotocraftApp, id: u64) -> photocraft_doc::SmartSource {
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.source.clone(),
        other => panic!("not a smart object: {}", other.kind_name()),
    }
}

#[test]
fn place_embedded_uses_the_single_file_picker_and_accepts_contents() {
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    assert!(menus::is_enabled(&app, "file.placeEmbedded"));
    let source = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
    let layers_before = app.session.active().unwrap().doc.layers.len();

    // Cancel leaves the document alone and does not report an error.
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    assert_eq!(last_request(&files).kind, PickKind::Open);
    cancel(&files);
    app.poll_file_dialog();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers_before);
    assert!(!app.ui.status_error, "{}", app.ui.status);

    // The Web picker returns the selected file's name and bytes instead of a filesystem path.
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("source.png".into(), source)]));
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers_before + 1);
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(layer.name, "source");
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    assert!(matches!(
        &layer.content,
        photocraft_doc::LayerContent::Smart(sm)
            if matches!(&sm.source, photocraft_doc::SmartSource::Embedded { file_name, .. } if file_name == "source.png")
    ));

    // Finish the successful placement before starting the next picker.
    crate::transform_tool::commit(&mut app);
    assert!(app.ui.transform.is_none());

    // Invalid contents are reported without adding a layer.
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("broken.png".into(), b"not an image".to_vec())]));
    app.poll_file_dialog();
    assert!(app.ui.status_error);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers_before + 1);
}

#[test]
fn replace_contents_picks_a_file_for_every_instance() {
    let (mut app, files, _, ids) = smart_app();
    let ctx = egui::Context::default();
    let before = ids.map(|id| smart_source(&app, id));
    // Cancelling changes nothing.
    menus::invoke(&mut app, &ctx, "layer.smartObjects.replaceContents", json!({})).unwrap();
    assert_eq!(last_request(&files).kind, PickKind::Open);
    cancel(&files);
    app.poll_file_dialog();
    assert_eq!(ids.map(|id| smart_source(&app, id)), before);
    // A picked file (by contents, as on the web) replaces the contents of both instances.
    let png = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    menus::invoke(&mut app, &ctx, "layer.smartObjects.replaceContents", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("art.png".into(), png)]));
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    for id in ids {
        assert!(matches!(smart_source(&app, id), photocraft_doc::SmartSource::Embedded { ref file_name, .. } if file_name == "art.png"));
    }
    // A file that isn't an image is an error, and the contents stay.
    menus::invoke(&mut app, &ctx, "layer.smartObjects.replaceContents", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("notes.png".into(), b"not an image".to_vec())]));
    app.poll_file_dialog();
    assert!(app.ui.status_error);
    assert!(matches!(smart_source(&app, ids[1]), photocraft_doc::SmartSource::Embedded { ref file_name, .. } if file_name == "art.png"));
}

/// Layer › Smart Objects › Relink to File… (and the layer context menu item) asks for a file and
/// links every instance to its path; it ran without a path before, so it always failed.
#[test]
fn relink_to_file_picks_a_file_for_every_instance() {
    let (mut app, files, _, ids) = smart_app();
    let ctx = egui::Context::default();
    let before = ids.map(|id| smart_source(&app, id));
    // Cancelling changes nothing.
    menus::invoke(&mut app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
    assert_eq!(last_request(&files).kind, PickKind::Open);
    cancel(&files);
    app.poll_file_dialog();
    assert_eq!(ids.map(|id| smart_source(&app, id)), before);
    // The web picker answers only a name and bytes: nothing to link to, so the contents stay.
    let png = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    menus::invoke(&mut app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
    answer(&files, Picked::Contents(vec![("art.png".into(), png.clone())]));
    app.poll_file_dialog();
    assert!(app.ui.status_error);
    assert_eq!(ids.map(|id| smart_source(&app, id)), before);
    // A file picked on disk links both instances to its path.
    let dir = std::env::temp_dir().join(format!("pcraft-relink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.png");
    std::fs::write(&path, &png).unwrap();
    let path = path.to_string_lossy().into_owned();
    menus::invoke(&mut app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
    answer(&files, chose(&path));
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    std::fs::remove_dir_all(&dir).ok();
    for id in ids {
        assert_eq!(smart_source(&app, id), photocraft_doc::SmartSource::Linked { path: path.clone() });
    }
}

#[test]
fn export_contents_suggests_the_contents_file_name() {
    let (mut app, files, written, _) = smart_app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.smartObjects.exportContents", json!({})).unwrap();
    assert_eq!(asked_save(&files), "Layer 1.pcraft");
    answer(&files, chose("/out/contents.pcraft"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["/out/contents.pcraft"]);
}

#[test]
fn convert_to_linked_asks_where_to_write_the_contents() {
    let (mut app, files, _, ids) = smart_app();
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join(format!("pcraft-convert-linked-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("linked.pcraft").to_string_lossy().into_owned();
    menus::invoke(&mut app, &ctx, "layer.smartObjects.convertToLinked", json!({})).unwrap();
    assert_eq!(last_request(&files).kind, PickKind::Save);
    answer(&files, chose(&path));
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(std::fs::metadata(&path).is_ok_and(|m| m.len() > 0));
    std::fs::remove_dir_all(&dir).ok();
    for id in ids {
        assert_eq!(smart_source(&app, id), photocraft_doc::SmartSource::Linked { path: path.clone() });
    }
}

#[test]
fn quick_export_selected_layers_writes_separate_pngs_with_one_prompt() {
    let (mut app, files, written) = app();
    app.run("layer.new.layer", json!({"name": "Same/name"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#ff0000"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Same/name"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#00ff00"})).unwrap();
    app.run("select.allLayers", json!({})).unwrap();
    let selected = app.session.active().unwrap().selected_layers();
    let before = app.session.active().unwrap().doc.clone();
    let sources = Rc::new(RefCell::new(Vec::new()));
    let capture = sources.clone();
    app.services.export = Some(Box::new(move |doc, path, _| {
        assert_eq!(doc.layers.len(), 1, "each PNG must contain just its source layer");
        let png = photocraft_io::export(doc, path, &Default::default()).unwrap();
        assert!(png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        capture.borrow_mut().push(doc.name.clone());
        Ok((png.bytes, Vec::new()))
    }));
    let ctx = egui::Context::default();
    assert!(!crate::layer_menu_ui::single_layer_only("layer.quickExportAsPng"));
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    assert_eq!(files.requests().len(), 1, "one destination for the entire selection");
    // Selection changes while choosing a destination must not change the captured sources.
    app.run("layer.select", json!({"layer": selected[0].0})).unwrap();
    answer(&files, chose("selected.png"));
    app.poll_file_dialog();
    assert_eq!(written.borrow().len(), selected.len());
    assert_eq!(sources.borrow().len(), selected.len());
    assert_eq!(written.borrow()[0], "selected.png");
    assert_eq!(written.borrow()[1], "selected-2-Same_name.png");
    assert_eq!(selected.len(), 2, "Select All Layers leaves the locked Background out");
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// #6599: Quick Export as PNG with Location = Same Folder asks before replacing the document's
/// own file (a document opened from photo.png), and keeps writing beside a PSD without a prompt.
#[test]
fn same_folder_quick_export_asks_before_replacing_the_documents_own_file() {
    let dir = std::env::temp_dir().join(format!("photocraft-quick-own-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let own = dir.join("photo.png").to_string_lossy().into_owned();
    std::fs::write(&own, b"original").unwrap();
    let (mut app, files, _) = app();
    app.run("prefs.set", json!({"path": "export.quickExportLocation", "value": "sameFolder"})).unwrap();
    app.session.active_mut().unwrap().path = Some(own.clone());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.export.quickExportAsPng", Value::Null).unwrap();
    assert_eq!(files.open(), 1, "the save dialog asks");
    assert_eq!(std::fs::read(&own).unwrap(), b"original", "nothing written before the answer");
    answer(&files, chose(&own));
    app.poll_file_dialog();
    assert!(std::fs::read(&own).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"), "replaced once confirmed: {}", app.ui.status);

    // Beside a PSD, Same Folder still writes without a prompt.
    app.session.active_mut().unwrap().path = Some(dir.join("photo.psd").to_string_lossy().into_owned());
    std::fs::write(&own, b"earlier export").unwrap();
    menus::invoke(&mut app, &ctx, "file.export.quickExportAsPng", Value::Null).unwrap();
    assert_eq!(files.requests().len(), 1, "no second dialog");
    assert!(std::fs::read(&own).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    std::fs::remove_dir_all(&dir).ok();
}

/// #6597: once Save As or Save a Copy writes over the file another open document is bound to,
/// that document no longer has the file and reads as unsaved (so closing it asks, and its Save
/// asks for a name), and a notice says so. Documents bound to other files are left alone.
#[test]
fn saving_over_the_file_of_another_open_document_detaches_it() {
    let dir = std::env::temp_dir().join(format!("photocraft-save-over-tab-{}", std::process::id()));
    let (a, d) = (dir.join("a.psd").to_string_lossy().into_owned(), dir.join("d.psd").to_string_lossy().into_owned());
    let (mut app, files, written) = app();
    let saved_as = |app: &mut PhotocraftApp, path: &str| {
        let st = app.session.active_mut().unwrap();
        st.path = Some(path.to_string());
        st.saved_revision = st.revision;
    };
    // A lives in a.psd and D in d.psd, both saved.
    saved_as(&mut app, &a);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    saved_as(&mut app, &d);
    // B, in another tab, is saved as a.psd.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.save_as(Some(a.clone())).unwrap();
    let docs = app.session.documents();
    assert_eq!((docs[0].path.as_deref(), docs[0].is_dirty()), (None, true), "A lost a.psd and is unsaved");
    assert_eq!((docs[1].path.as_deref(), docs[1].is_dirty()), (Some(d.as_str()), false), "D is untouched");
    assert_eq!(docs[2].path.as_deref(), Some(a.as_str()), "B lives in a.psd");
    let notice = app.ui.notices.last().expect("a notice");
    assert!(notice.title.contains("a.psd was saved over") && notice.lines.len() == 1, "{} {:?}", notice.title, notice.lines);
    // C's Save a Copy onto a.psd detaches B the same way, and C doesn't take the file.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    answer(&files, chose(&a));
    app.poll_file_dialog();
    let docs = app.session.documents();
    assert_eq!((docs[2].path.as_deref(), docs[2].is_dirty()), (None, true), "B lost a.psd and is unsaved");
    assert_eq!(docs[3].path, None, "a copy doesn't bind C to a.psd");
    assert_eq!(*written.borrow(), [a.clone(), a]);
}

#[test]
fn cancelling_multi_layer_quick_export_writes_nothing() {
    let (mut app, files, written) = app();
    for color in ["#ff0000", "#00ff00"] {
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("paint.bucket", json!({"x": 0, "y": 0, "color": color})).unwrap();
    }
    app.run("select.allLayers", json!({})).unwrap();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    cancel(&files);
    app.poll_file_dialog();
    assert!(written.borrow().is_empty());
}

/// #3354: a system with no dialog backend answers `Unavailable`; the message reaches the status
/// bar as an error, where a cancelled dialog stays silent.
#[test]
fn an_unavailable_dialog_is_reported_and_a_cancelled_one_is_not() {
    let ctx = egui::Context::default();
    let files = Arc::new(ScriptedFiles::answering([Picked::Cancelled, Picked::Unavailable("No file dialog can open".into())]));
    let (mut app, _, _) = app_with(files);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "Cancel is the user's choice");
    assert!(!app.file_dialog_open());

    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog();
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "No file dialog can open");
    assert!(!app.file_dialog_open(), "the pending dialog is over");
}

/// #3806: File › Save on an OpenRaster file that can't hold the document (here an adjustment
/// layer) doesn't overwrite it: Save As opens on a PSD with a notice saying what the .ora would
/// drop, and the PSD becomes the document's file. A plain document still saves back to its .ora.
#[test]
fn openraster_save_never_overwrites_what_it_cannot_hold() {
    let (mut app, files, written) = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session.active_mut().unwrap().path = Some("/pics/art.ora".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/art.ora"], "a plain document saves in place");
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let notices = app.ui.notices.len();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(written.borrow().len(), 1, "the .ora is not overwritten");
    assert_eq!(asked_save(&files), "/pics/art.psd", "Save As offers PSD");
    assert_eq!(app.ui.notices.len(), notices + 1);
    assert!(app.ui.notices.last().unwrap().lines.iter().any(|l| l.contains("adjustment layers")), "{:?}", app.ui.notices.last().unwrap().lines);
    assert!(app.session.active().unwrap().is_dirty(), "the edits are still unsaved");
    answer(&files, chose("/pics/art.psd"));
    app.poll_file_dialog();
    assert_eq!(written.borrow().last().map(String::as_str), Some("/pics/art.psd"));
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/art.psd"));
    assert!(!st.is_dirty());
}

/// Save As to OpenRaster of a document it can't hold writes a copy, as Photoshop does for formats
/// that can't keep everything: the document keeps its file and its unsaved state. An automation
/// save without a path won't overwrite such an .ora either.
#[test]
fn save_as_openraster_of_what_it_cannot_hold_is_a_copy() {
    let (mut app, _files, written) = app();
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    app.session.active_mut().unwrap().path = Some("/pics/art.psd".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "/pics/copy.ora"})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/copy.ora"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/art.psd"), "the document keeps its file");
    assert!(st.is_dirty());
    app.session.active_mut().unwrap().path = Some("/pics/art.ora".into());
    let err = app.save_automation(None).unwrap_err();
    assert!(err.contains("OpenRaster"), "{err}");
    assert_eq!(written.borrow().len(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn render_video_menu_asks_for_directory_and_writes_frames_only_after_confirmation() {
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join(format!(
        "pc-render-video-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let path = dir.to_string_lossy().into_owned();

    let result = menus::invoke(&mut app, &ctx, "file.export.renderVideo", json!({})).unwrap();
    assert_eq!(result["fileDialog"], "folder");
    assert_eq!(last_request(&files).kind, PickKind::Folder);
    assert!(!dir.exists(), "a menu click must not create an output directory");

    cancel(&files);
    app.poll_file_dialog();
    assert!(!dir.exists(), "cancelling the picker must not render video");

    menus::invoke(&mut app, &ctx, "file.export.renderVideo", json!({})).unwrap();
    answer(&files, chose(&path));
    app.poll_file_dialog();
    let rendered: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(rendered.len(), 1, "a document without a timeline exports one frame");
    assert_eq!(rendered[0].extension().and_then(|ext| ext.to_str()), Some("png"));
    std::fs::remove_dir_all(&dir).unwrap();

    // Commands that already supply a directory must not open a file picker.
    let asked = files.requests().len();
    let result = menus::invoke(&mut app, &ctx, "file.export.renderVideo", json!({"dir": path})).unwrap();
    assert!(result["frames"].as_u64().is_some());
    assert!(!app.file_dialog_open());
    assert_eq!(files.requests().len(), asked, "no picker");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// How many times each path was written, standing in for its size and modification time.
type Disk = Rc<RefCell<std::collections::HashMap<String, u64>>>;

/// Gives `app` file stamps from a fake disk that its own writes update too.
fn on_disk(app: &mut PhotocraftApp) -> Disk {
    let disk = Disk::default();
    let (read, wrote) = (disk.clone(), disk.clone());
    app.services.file_stamp = Some(Box::new(move |path| read.borrow().get(path).map(|&n| crate::FileStamp { len: n, modified: None })));
    let mut write = app.services.write.take().unwrap();
    app.services.write = Some(Box::new(move |path, bytes| {
        write(path, bytes)?;
        *wrote.borrow_mut().entry(path.to_string()).or_default() += 1;
        Ok(())
    }));
    disk
}

fn changed_by_another_app(disk: &Disk, path: &str) {
    *disk.borrow_mut().entry(path.to_string()).or_default() += 1;
}

/// The changed-on-disk question of a Save, when one is open.
fn disk_prompt(app: &PhotocraftApp) -> Option<DialogId> {
    crate::dialogs::alerts::prompt_tagged::<crate::disk_change_prompt::HeldSave>(app)
}

#[test]
fn save_asks_before_replacing_a_file_another_app_changed() {
    let (mut app, _files, written) = app();
    let ctx = app.ctx().clone();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, "/pics/a.psd");
    app.opened_from("/pics/a.psd");
    app.run("layer.new.layer", json!({})).unwrap();
    // Unchanged on disk: Save writes at once.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/a.psd"]);
    assert!(disk_prompt(&app).is_none());

    // Our own save isn't a change; another app's write after it is.
    changed_by_another_app(&disk, "/pics/a.psd");
    app.run("layer.new.layer", json!({})).unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["confirmationPending"], true);
    assert_eq!(written.borrow().len(), 1, "nothing is written before the answer");
    let id = disk_prompt(&app).expect("Save asks first");
    let held = crate::dialogs::alerts::prompt_tag::<crate::disk_change_prompt::HeldSave>(&app).expect("the prompt holds the save");
    assert!(held.path.contains("a.psd"), "{}", held.path);

    // Cancel leaves the file and the unsaved document alone.
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert_eq!(written.borrow().len(), 1);
    assert!(app.session.active().unwrap().is_dirty());

    // Replace writes it, and the next Save doesn't ask again.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let id = disk_prompt(&app).expect("Save asks again");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(written.borrow().len(), 2);
    assert!(!app.session.active().unwrap().is_dirty());
    app.run("layer.new.layer", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(written.borrow().len(), 3);
    assert!(disk_prompt(&app).is_none());
}

/// A document that no longer fits its flat file by the time the changed-on-disk prompt is
/// answered goes through Save As rather than being written over the original.
#[test]
fn replace_after_the_document_outgrew_its_flat_file_asks_where_to_save() {
    let path = "/pics/My image.jpg";
    let (mut app, files, written) = jpeg_app(None, false);
    let ctx = app.ctx().clone();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, path);
    app.opened_from(path);
    app.run("image.adjustments.invert", json!({})).unwrap();
    changed_by_another_app(&disk, path);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let id = disk_prompt(&app).expect("Save asks first");
    app.run("layer.new.layer", json!({})).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(last_request(&files).kind, PickKind::Save, "Save As");
    assert!(written.borrow().is_empty(), "the JPEG is not written over");
}

/// File › Save in place over a flat JPEG another app changed asks first (#6596), then JPEG
/// Options (#2223); Cancel in either writes nothing, and OK in both writes a plain Save.
#[test]
fn save_in_place_of_a_flat_file_asks_first_when_another_app_changed_it() {
    let path = "/pics/My image.jpg";
    let (mut app, files, written) = jpeg_app(None, false);
    let ctx = app.ctx().clone();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, path);
    app.opened_from(path);
    app.run("image.adjustments.invert", json!({})).unwrap();
    changed_by_another_app(&disk, path);
    // One plain Background layer: the JPEG keeps it, so Save writes back without a dialog (#5733)
    // but asks before replacing the changed file (#6596).
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["confirmationPending"], true, "{r}");
    assert!(files.requests().is_empty(), "no Save dialog");
    assert!(written.borrow().is_empty(), "the changed file is not replaced before the answer");
    let id = disk_prompt(&app).expect("Save asks first");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    // Then JPEG Options (#2223): Cancel there writes nothing either.
    assert!(written.borrow().is_empty());
    assert_eq!(parked(&app).map(|(_, copy, record)| (record, copy)), Some(("file.save", false)));
    cancel_options(&mut app);
    assert!(written.borrow().is_empty());
    assert!(app.session.active().unwrap().is_dirty());
    // Saving again asks both again; OK in both writes it.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    let id = disk_prompt(&app).expect("Save asks first");
    assert!(parked(&app).is_none(), "the changed-on-disk prompt comes first");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    confirm_options(&mut app);
    assert_eq!(*written.borrow(), [path]);
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({}))), "recorded as a plain Save");
}

/// The JPEG quality of each save, as the exporter was given it.
type Qualities = Rc<RefCell<Vec<Option<u8>>>>;

/// [`jpeg_app`] opened from `path` and edited, whose exporter records the JPEG quality of each
/// save (#2223).
fn flat_jpeg_app(path: &str) -> (PhotocraftApp, Arc<ScriptedFiles>, Written, Qualities) {
    let (mut app, files, written) = jpeg_app(Some(path), false);
    let qualities = Qualities::default();
    let q = qualities.clone();
    app.services.export = Some(Box::new(move |_: &photocraft_doc::Document, _: &str, s: &crate::ExportSettings| {
        q.borrow_mut().push(s.jpeg_quality);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    app.run("image.adjustments.invert", json!({})).unwrap();
    (app, files, written, qualities)
}

/// File › Save on a flat JPEG that still holds the document asks JPEG Options, then writes over
/// it with no Save dialog, as Photoshop does; Cancel writes nothing (#2223).
#[test]
fn save_asks_jpeg_options_then_writes_a_flat_jpeg_over_itself() {
    let ctx = egui::Context::default();
    let (mut app, files, written, qualities) = flat_jpeg_app("/pics/photo.jpg");
    let own = app.session.active().unwrap().doc.file_encoding.jpeg_quality.expect("an opened JPEG knows its quality");
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert!(files.requests().is_empty(), "no Save dialog");
    assert!(written.borrow().is_empty(), "nothing written before the options are answered");
    let start = crate::jpeg_options_ui::Start::for_active(&app);
    assert_eq!(parked(&app), Some(("/pics/photo.jpg".to_string(), false, "file.save")));
    assert_eq!(option_field(&app, "quality"), json!(start.quality));
    // Another save waits for the answer.
    assert!(menus::invoke(&mut app, &ctx, "file.save", json!({})).is_err());
    // Cancel (what the dialog does): nothing is written and the document stays unsaved.
    cancel_options(&mut app);
    assert!(written.borrow().is_empty());
    assert!(app.session.active().unwrap().is_dirty());
    // OK as it starts: the file's own quality, exactly, recorded as a plain Save.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    confirm_options(&mut app);
    assert_eq!(*written.borrow(), ["/pics/photo.jpg"]);
    assert_eq!(*qualities.borrow(), [Some(own)]);
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({}))));
    assert!(!app.session.active().unwrap().is_dirty());
    // OK at another quality and format: written with them, and recorded with them.
    app.run("image.adjustments.invert", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    set_option(&mut app, "quality", json!(3));
    set_option(&mut app, "format", json!("progressive"));
    confirm_options(&mut app);
    assert_eq!(*written.borrow(), ["/pics/photo.jpg", "/pics/photo.jpg"]);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(3.0)));
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({"quality": 3, "jpegFormat": "progressive"}))));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/photo.jpg"));
    // A script's `quality` answers it: no prompt.
    app.run("image.adjustments.invert", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({"quality": 12})).unwrap();
    assert!(parked(&app).is_none());
    assert_eq!(written.borrow().len(), 3);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(12.0)));
    // Once a layer is added, Save asks where to save a PSD instead (#2234).
    app.run("layer.new.layer", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert!(parked(&app).is_none());
    assert_eq!(asked_save(&files), "/pics/photo.psd");
    cancel(&files);
    app.poll_file_dialog();
}

/// Adds another flat JPEG, opened from `path`, and makes it active.
fn add_flat_jpeg(app: &mut PhotocraftApp, path: &str) {
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "other.jpg", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("other.jpg", &bytes).unwrap().document;
    let index = app.session.add_document(doc, Some(path.to_string()));
    app.session.set_active(index);
}

/// An action being recorded keeps File › Save over a flat JPEG as a `file.save` step with the
/// JPEG Options answer, and playing it writes the active flat JPEG over itself with that answer
/// and no prompt (#2223). A cancelled JPEG Options records nothing.
#[test]
fn flat_jpeg_saves_are_recorded_and_replay_over_the_open_file() {
    let ctx = egui::Context::default();
    let (mut app, files, written, qualities) = flat_jpeg_app("/pics/photo.jpg");
    app.run("actions.record", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    cancel_options(&mut app);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    set_option(&mut app, "quality", json!(5));
    confirm_options(&mut app);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list.action(0).unwrap().steps, [ActionStep::new("file.save", json!({"quality": 5}))]);
    add_flat_jpeg(&mut app, "/pics/other.jpg");
    written.borrow_mut().clear();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(parked(&app).is_none(), "playback does not ask");
    assert_eq!(*written.borrow(), ["/pics/other.jpg"]);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(5.0)));
    assert!(files.requests().is_empty(), "no Save dialog");
}

#[test]
fn save_of_a_flat_file_asks_after_the_dialog_when_another_app_changed_it() {
    // The file service lowercases a typed extension by default.
    let path = "/pics/My image.jpg";
    let (mut app, files, written) = jpeg_app(None, false);
    let ctx = app.ctx().clone();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, path);
    app.opened_from(path);
    // Transparency JPEG can't keep: Save goes through the dialog (which suggests a PNG; the
    // user picks the JPEG anyway).
    app.run("layer.new.layerFromBackground", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    changed_by_another_app(&disk, path);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    answer(&files, chose(path));
    app.poll_file_dialog();
    assert!(written.borrow().is_empty(), "the changed file is not replaced before the answer");
    let id = disk_prompt(&app).expect("Save asks first");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    // Then JPEG Options (FILE-215-19), whose answer is still recorded as a plain Save.
    assert!(written.borrow().is_empty());
    assert_eq!(parked(&app).map(|(_, _, record)| record), Some("file.save"));
    confirm_options(&mut app);
    assert_eq!(*written.borrow(), [path]);
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({}))), "still recorded as a plain Save");
}

/// A replayed plain Save keeps its Targa answer in the step it records, as well as its JPEG one.
#[test]
fn a_replayed_plain_save_records_its_targa_answer() {
    let (mut app, _, written) = jpeg_app(Some("/pics/photo.tga"), false);
    app.replay_save("file.save", &json!({"tgaBits": 24})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/photo.tga"]);
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({"tgaBits": 24}))));
}

/// A script's JPEG Options answer survives the changed-on-disk prompt: Replace writes with it,
/// without asking JPEG Options.
#[test]
fn a_scripted_quality_survives_the_changed_on_disk_prompt() {
    let path = "/pics/My image.jpg";
    let (mut app, files, written) = jpeg_app(None, false);
    let ctx = app.ctx().clone();
    let qualities: Rc<RefCell<Vec<Option<u8>>>> = Rc::default();
    let q = qualities.clone();
    app.services.export = Some(Box::new(move |_: &photocraft_doc::Document, _: &str, s: &crate::ExportSettings| {
        q.borrow_mut().push(s.jpeg_quality);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, path);
    app.opened_from(path);
    app.run("image.adjustments.invert", json!({})).unwrap();
    changed_by_another_app(&disk, path);
    // Saved in place here (#2223): no Save dialog, only the changed-on-disk prompt.
    menus::invoke(&mut app, &ctx, "file.save", json!({"quality": 5})).unwrap();
    assert!(files.requests().is_empty(), "no Save dialog");
    assert!(written.borrow().is_empty(), "the changed file is not replaced before the answer");
    let id = disk_prompt(&app).expect("Save asks first");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(parked(&app).is_none(), "the script answered JPEG Options");
    assert_eq!(*written.borrow(), [path]);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(5.0)));
    // Recorded as a plain Save keeping the answer, so a replay writes the file it's played on.
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({"quality": 5}))));
}

/// File › Save a Copy suggests "<name> copy", and with Preferences › File Handling › Do not
/// append "copy" to filename when saving a copy, the document's own name, as in Photoshop
/// (#8080). The preference goes through `prefs.set` like the Preferences dialog's checkbox.
#[test]
fn save_a_copy_appends_copy_unless_the_preference_says_not_to() {
    let ctx = egui::Context::default();
    for (off, name) in [(false, "My image copy.psd"), (true, "My image.psd")] {
        let want = Path::new("/pics").join(name).to_string_lossy().into_owned();
        let (mut app, files, _) = jpeg_app(Some("/pics/My image.JPEG"), false);
        app.run("prefs.set", json!({"path": "fileHandling.doNotAppendCopy", "value": off})).unwrap();
        menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
        assert_eq!(asked_save(&files), want, "{off}");
    }
}

#[test]
fn import_variable_data_sets_menu_asks_for_a_csv_and_imports_it() {
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    assert_eq!(crate::menus::invoke(&mut app, &ctx, "file.import.variableDataSets", json!({})).unwrap(), json!({"fileDialog": "open"}));
    let asked = last_request(&files);
    assert_eq!((asked.kind, asked.filters), (PickKind::Open, vec![FileFilter::new("Supported Files", &["csv", "txt"])]));
    answer(&files, Picked::Contents(vec![("sets.csv".into(), b"headline\nHello\nWorld\n".to_vec())]));
    app.poll_file_dialog();
    assert_eq!(app.session.active().map(|d| d.doc.variables.data_sets.len()), Some(2));
}

/// #10192: the folder an export dialog starts in, as a pure decision.
#[test]
fn export_start_dir_prefers_an_existing_remembered_folder() {
    let doc = Some(PathBuf::from("/proj"));
    let all_exist = |_: &Path| true;
    let none_exist = |_: &Path| false;
    // Remembered and present: used.
    assert_eq!(export_start_dir(Some("/renders"), doc.clone(), all_exist), Some(PathBuf::from("/renders")));
    // Remembered but gone: the document's folder.
    assert_eq!(export_start_dir(Some("/renders"), doc.clone(), none_exist), doc);
    // Nothing remembered (unset or empty): the document's folder, or nothing for an untitled one.
    assert_eq!(export_start_dir(None, doc.clone(), all_exist), doc);
    assert_eq!(export_start_dir(Some(""), doc.clone(), all_exist), doc);
    assert_eq!(export_start_dir(None, None, all_exist), None);
    assert_eq!(export_start_dir(Some("/renders"), None, none_exist), None);
}

/// #10192: exports start in the last successfully exported folder, across documents; a failed or
/// cancelled export, a folder that is gone and other save dialogs change nothing.
#[test]
fn exports_start_in_the_last_exported_folder() {
    let (mut app, files, _) = app();
    let root = tempfile::tempdir().unwrap();
    let renders = root.path().join("renders");
    std::fs::create_dir(&renders).unwrap();
    let renders_text = renders.to_string_lossy().into_owned();
    app.session.active_mut().unwrap().path = Some("/proj/original.psd".into());
    let remembered = |app: &PhotocraftApp| app.session.prefs().export.last_folder.clone();
    // No export yet: beside the document. Cancelled, nothing is remembered.
    app.pick_export("a.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/proj", "a.png"));
    cancel(&files);
    app.poll_file_dialog();
    assert_eq!(remembered(&app), "");
    // The user picks another folder, but the export fails: nothing is remembered.
    app.pick_export("a.png", |_, _| Err("disk full".into())).unwrap();
    answer(&files, chose(&joined(&renders_text, "a.png")));
    app.poll_file_dialog();
    assert_eq!(remembered(&app), "");
    // ...and the next export still starts beside the document, not in the failed export's folder.
    app.pick_export("a.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/proj", "a.png"));
    cancel(&files);
    app.poll_file_dialog();
    // A successful export is remembered...
    app.pick_export("a.png", |_, _| Ok(Value::Null)).unwrap();
    answer(&files, chose(&joined(&renders_text, "a.png")));
    app.poll_file_dialog();
    assert_eq!(remembered(&app), renders_text);
    // ...and starts the next export, of any document, even an untitled one.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.pick_export("b.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined(&renders_text, "b.png"));
    // A cancelled dialog changes nothing.
    cancel(&files);
    app.poll_file_dialog();
    assert_eq!(remembered(&app), renders_text);
    // Other save dialogs keep their own start.
    app.pick_save("c.psd", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), "c.psd");
    cancel(&files);
    app.poll_file_dialog();
    // The folder disappears: back to the document's folder.
    std::fs::remove_dir(&renders).unwrap();
    app.session.set_active(0);
    app.pick_export("d.png", |_, _| Ok(Value::Null)).unwrap();
    assert_eq!(asked_save(&files), joined("/proj", "d.png"));
    cancel(&files);
    app.poll_file_dialog();
}

/// A path that is not valid Unicode, in the temp folder.
fn non_unicode_path() -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::env::temp_dir().join(std::ffi::OsStr::from_bytes(b"img\xFF.png"))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = "img".encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(".png".encode_utf16());
        std::env::temp_dir().join(std::ffi::OsString::from_wide(&wide))
    }
}

/// #6601: a chosen path that is not valid Unicode is an error, never another file's name: Open
/// would fail with "No such file" and Save would write a second, renamed file.
#[test]
fn a_picked_path_that_is_not_valid_unicode_is_an_error() {
    let bad = non_unicode_path();
    let good = std::env::temp_dir().join("ok.png");
    let (mut app, files, _) = app();
    let reached = Rc::new(Cell::new(false));
    let ask_save = |app: &mut PhotocraftApp| {
        let seen = reached.clone();
        app.pick_save("x.png", move |_, path| {
            seen.set(true);
            assert!(!path.is_empty());
            Ok(Value::Null)
        })
        .unwrap();
    };
    ask_save(&mut app);
    answer(&files, Picked::Paths(vec![bad.clone()]));
    app.poll_file_dialog();
    assert!(!reached.get(), "the action never sees the path");
    assert!(app.ui.status_error && app.ui.status.contains("not valid Unicode") && app.ui.status.contains("another name"), "{}", app.ui.status);

    app.ui.status_error = false;
    ask_save(&mut app);
    answer(&files, Picked::Paths(vec![good.clone()]));
    app.poll_file_dialog();
    assert!(reached.get() && !app.ui.status_error, "{}", app.ui.status);

    // One bad name among several chosen files fails the whole answer, with the wording of Open.
    let documents = app.session.documents().len();
    app.open_dialog_file().unwrap();
    answer(&files, Picked::Paths(vec![good, bad]));
    app.poll_file_dialog();
    assert!(app.ui.status_error && app.ui.status.contains("open it again"), "{}", app.ui.status);
    assert_eq!(app.session.documents().len(), documents, "nothing opened");
}

/// A format the importer reads but no filter lists is invisible in File › Open.
#[test]
fn every_open_format_has_its_own_filter() {
    let types = open_filters(PickRequest::open("document"), &[]).filters;
    assert_eq!(types[0], FileFilter::new("All Formats", filters::OPEN_EXTS), "All Formats comes first");
    let mut named: Vec<&str> = filters::OPEN_FILTERS.iter().flat_map(|(_, exts)| exts.iter().copied()).collect();
    let total = named.len();
    named.sort_unstable();
    named.dedup();
    assert_eq!(named.len(), total, "an extension is listed under two filters");
    let mut all = filters::OPEN_EXTS.to_vec();
    all.sort_unstable();
    assert_eq!(named, all, "the named filters cover All Formats exactly");
    let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
    for name in ["Photoshop", "Affinity", "HEIF", "PNG", "JPEG", "TIFF", "Portable Bitmap", "OpenEXR", "Radiance HDR", "Paint.NET"] {
        assert!(names.contains(&name), "{name}");
    }
    // A command's own list replaces them all.
    assert_eq!(open_filters(PickRequest::open("file"), &["cube"]).filters, [FileFilter::new("Supported Files", &["cube"])]);
}

/// The save types of a Save dialog suggesting `suggested`: (name, extensions).
fn save_types(suggested: &str) -> Vec<(String, Vec<String>)> {
    save_request("document", suggested).filters.into_iter().map(|f| (f.name, f.extensions)).collect()
}

/// Every flat format Save As offers is one PhotoCraft writes, and ICO is among them (#4357).
#[test]
fn save_as_offers_only_writable_formats_including_ico() {
    // Layered formats, and PDF, are written by `photocraft-io`, not a codec.
    let layered = ["psd", "psb", "pcraft", "ora", "pdf"];
    for (name, exts) in filters::SAVE_FILTERS {
        for ext in exts.iter().filter(|e| !layered.contains(e)) {
            let format = photocraft_codecs::from_extension(ext).unwrap_or_else(|| panic!("{name}: .{ext} is no known format"));
            assert!(format.caps().write, "{name}: .{ext} can't be written");
        }
    }
    for ext in ["ico", "gif", "bmp", "dds", "pnm", "hdr", "dib", "jpeg", "tiff"] {
        assert!(save_types("Untitled-1.psd").iter().any(|(_, exts)| exts.iter().any(|e| e == ext)), ".{ext} is offered");
    }
    // The codec writes P5/P6/P7/PF from the image, never from the name, so Netpbm offers only the
    // neutral .pnm (#3609; CodeRabbit on #9775): a .pbm, .pgm or .pfm name would hold other data.
    let netpbm = save_types("Untitled").into_iter().find(|(name, _)| name == "Netpbm").map(|(_, exts)| exts);
    assert_eq!(netpbm, Some(vec!["pnm".to_string()]));
    // The suggested name itself becomes .pnm, as macOS keeps it (CodeRabbit on #9775).
    let named = |suggested: &str| save_request("document", suggested).file_name;
    assert_eq!(named("scans/image.pgm").as_deref(), Some("image.pnm"));
    assert_eq!(named("image.PFM").as_deref(), Some("image.pnm"));
    assert_eq!(named("image.png").as_deref(), Some("image.png"));
    assert_eq!(named("Untitled-1").as_deref(), Some("Untitled-1"));
    assert_eq!(save_request("document", "scans/image.pgm").directory.as_deref(), Some(Path::new("scans")));
    // Also when Save As starts from a subtype's name: no filter offers that subtype again.
    for name in ["image.pbm", "image.pgm", "image.ppm", "image.pam", "image.pfm"] {
        let types = save_types(name);
        assert_eq!(types[0], ("Netpbm".to_string(), vec!["pnm".to_string()]), "{name}");
        let subtype = name.rsplit('.').next().unwrap_or_default();
        assert!(!types.iter().any(|(_, exts)| exts.iter().any(|e| e == subtype)), "{name}: .{subtype} is offered");
    }
    // File Open lists readable Save As formats in their own type. Export-only formats must
    // stay absent from both Open filters rather than advertising an unavailable decoder.
    for (name, exts) in filters::SAVE_FILTERS {
        for ext in exts.iter().filter(|e| **e != "pdf") {
            if photocraft_codecs::from_extension(ext).is_some_and(|format| !format.caps().read) {
                assert!(!filters::OPEN_EXTS.contains(ext), "{name}: export-only .{ext} must not appear in All Formats");
                assert!(!filters::OPEN_FILTERS.iter().any(|(_, open)| open.contains(ext)), "{name}: export-only .{ext} must not appear in an Open type");
                continue;
            }
            assert!(filters::OPEN_EXTS.contains(ext), "{name}: Open's All Formats lacks .{ext}");
            assert!(filters::OPEN_FILTERS.iter().any(|(_, open)| open.contains(ext)), "{name}: no Open type lists .{ext}");
        }
    }
    assert_eq!(save_types("icon.ico")[0].0, "ICO", "an .ico name leads with its own type");
    // What Save As writes there: an icon up to 256×256, and a clear error past that.
    let doc = |side: u32| {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": side, "height": side})).unwrap();
        s.active().unwrap().doc.clone()
    };
    let opts = Default::default();
    let icon = photocraft_io::export(&doc(256), "icon.ico", &opts).unwrap();
    assert_eq!(icon.bytes[..4], [0, 0, 1, 0], "an ICO header");
    // And what the Netpbm and Radiance rows write.
    let pnm = photocraft_io::export(&doc(4), "x.pnm", &opts).unwrap();
    assert_eq!(pnm.bytes[..2], *b"P6", "an RGB document saves as a binary pixmap");
    let hdr = photocraft_io::export(&doc(4), "x.hdr", &opts).unwrap();
    assert!(hdr.bytes.starts_with(b"#?RADIANCE"), "a Radiance header");
    let err = photocraft_io::export(&doc(300), "icon.ico", &opts).map(|_| ()).unwrap_err().to_string();
    assert!(err.contains("256"), "{err}");
}

/// #4357's sibling: `.psb` is its own Save type. As an extension of the PSD type it was hidden, and
/// KDE's dialog, which adds the type's first extension, turned `art.psb` into `art.psb.psd`. (The
/// chosen-type half of this rule is the framework portal's.)
#[test]
fn psb_is_its_own_save_type() {
    let types = save_types("Untitled-1.psd");
    let (psd, psb) = (&types[0], &types[1]);
    // The PSD type takes a typed .psb too, so Windows doesn't add .psd after it.
    assert_eq!(psd.1, vec!["psd".to_string(), "psb".to_string()], "{psd:?}");
    assert_eq!(psb.1, vec!["psb".to_string()], "{psb:?}");
    let big = save_types("big.psb");
    assert_eq!((big[0].0.as_str(), big[0].1.clone()), ("PSB Large Document", vec!["psb".to_string()]), "a .psb starts on its own type");
}

/// The document formats, then the rest by name, so a long list stays easy to search.
#[test]
fn save_types_list_the_document_formats_then_the_rest_by_name() {
    let names: Vec<&str> = filters::SAVE_FILTERS.iter().map(|(name, _)| *name).collect();
    assert_eq!(names[..3], ["PSD Document", "PSB Large Document", "PhotoCraft"]);
    let key = |name: &&str| name.to_ascii_lowercase();
    assert!(names[3..].is_sorted_by_key(key), "{:?}", &names[3..]);
    let names: Vec<&str> = filters::OPEN_FILTERS.iter().map(|(name, _)| *name).collect();
    assert_eq!(names[..2], ["Photoshop", "PhotoCraft"]);
    let formats = names.iter().position(|name| *name == "Brushes").expect("the app's own resources");
    assert!(names[2..formats].is_sorted_by_key(key), "{:?}", &names[2..formats]);
}
