//! #14496: the Apply Data Set dialog's Preview checkbox, as in the reference application. Off on
//! the first open, Alt+P toggles it. On, the selected set is shown in the document (canvas and
//! Layers panel read it) without a history state, and selecting another row switches the preview;
//! off again, the document is as before the dialog. Cancel and Escape revert it; Apply with
//! Preview on leaves the same document and the one history state Apply without it does. The box
//! keeps its state for the next open after Apply, not after Cancel. A set that can't be applied
//! (its replacement file is missing) shows nothing of itself.

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, TextLayer};
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply};
use serde_json::{Value, json};

type H = Harness<'static, Box<PhotocraftApp>>;

struct Layers {
    photo: LayerId,
    badge: LayerId,
    title: LayerId,
}

/// An 8x8 blue PNG for the Pixel Replacement variable, in a folder of its own for each fixture:
/// tests run in parallel, and rewriting a shared file could hand another test a half-written one.
fn blue_png() -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pc-apply-data-set-preview-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("blue.png");
    let mut src = photocraft_engine::Session::new();
    src.execute("file.new", json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
    let bytes = photocraft_io::export(&src.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

/// The test document of the spec, small: a grey pixel layer `Photo`, a red `Badge` and a text layer
/// `Title` ("Old"), with a visibility, a text and a pixel variable. Sets in document order:
/// Zed (Badge hidden, "Alpha", Photo blue), Data Set 2 (Badge shown, "Beta", no image) and
/// Data Set 3 (Badge hidden, "Gamma", an image file that doesn't exist).
fn app() -> (Box<PhotocraftApp>, Layers) {
    app_with(photocraft_ui_egui::Services::default())
}

fn app_with(services: photocraft_ui_egui::Services) -> (Box<PhotocraftApp>, Layers) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
    let (photo, badge, title) = app
        .session
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut photo = Layer::raster("Photo", fmt);
            photo.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 64, 48), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
            let mut badge = Layer::raster("Badge", fmt);
            badge.surface_mut().unwrap().fill_rect(Rect::new(40, 0, 60, 20), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let title = Layer::new("Title", LayerContent::Text(TextLayer { text: "Old".into(), ..Default::default() }));
            let ids = (photo.id, badge.id, title.id);
            doc.layers.push(photo);
            doc.layers.push(badge);
            doc.layers.push(title);
            Ok(ids)
        })
        .unwrap();
    app.run(
        "image.variables.define",
        json!({"defs": [
            {"name": "badge", "layer": badge.0, "type": "visibility"},
            {"name": "headline", "layer": title.0, "type": "textReplacement"},
            {"name": "picture", "layer": photo.0, "type": "pixelReplacement", "method": "conform"},
        ]}),
    )
    .unwrap();
    let missing = std::env::temp_dir().join(format!("pc-apply-data-set-preview-{}-missing", std::process::id())).join("gone.png");
    app.run(
        "image.variables.dataSets",
        json!({"dataSets": [
            {"name": "Zed", "values": [
                {"variable": "badge", "kind": "visibility", "value": false},
                {"variable": "headline", "kind": "text", "value": "Alpha"},
                {"variable": "picture", "kind": "pixels", "value": blue_png()},
            ]},
            {"name": "Data Set 2", "values": [
                {"variable": "badge", "kind": "visibility", "value": true},
                {"variable": "headline", "kind": "text", "value": "Beta"},
            ]},
            {"name": "Data Set 3", "values": [
                {"variable": "badge", "kind": "visibility", "value": false},
                {"variable": "headline", "kind": "text", "value": "Gamma"},
                {"variable": "picture", "kind": "pixels", "value": missing.to_string_lossy()},
            ]},
        ]}),
    )
    .unwrap();
    // As opened from disk: nothing to save yet, so a preview that marks it modified shows.
    let st = app.session.active_mut().unwrap();
    st.saved_revision = st.revision;
    (app, Layers { photo, badge, title })
}

fn harness(app: Box<PhotocraftApp>) -> H {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            photocraft_ui_egui::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// One control request, as a transport sends it.
fn control(h: &mut H, method: &str, params: Value) -> Value {
    let (request, _) = ControlRequest::new(method, params);
    let ctx = h.ctx.clone();
    let reply = match photocraft_ui_egui::control::handle(h.state_mut(), &ctx, &request) {
        Reply::Done(value) => value,
        _ => panic!("{method} did not answer at once"),
    };
    h.run_steps(3);
    reply
}

fn refused(reply: &Value) -> bool {
    reply.get("error").is_some() || reply["ok"] == json!(false)
}

/// Image › Apply Data Set… as the menu runs it, then a few frames; the dialog's id.
fn open(h: &mut H) -> String {
    let ctx = h.ctx.clone();
    let reply = photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.applyDataSet", json!({})).unwrap();
    h.run_steps(3);
    reply["dialog"].as_str().expect("the menu opens a dialog").to_string()
}

/// The dialog open on the test document, its id, and the document as it was before.
fn opened() -> (H, Layers, Arc<Document>, String) {
    let (app, layers) = app();
    let mut h = harness(app);
    let before = doc(&h);
    let id = open(&mut h);
    (h, layers, before, id)
}

fn doc(h: &H) -> Arc<Document> {
    h.state().session.active().unwrap().doc.clone()
}

/// The Apply Data Set dialog's `preview` field, as `ui.inspect` lists it.
fn preview_field(h: &mut H) -> Value {
    let listed = control(h, "ui.inspect", json!({}));
    listed["result"]["dialogs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["fields"]["__command"] == "image.applyDataSet")
        .map_or(Value::Null, |d| d["fields"]["preview"].clone())
}

fn history(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn dirty(h: &H) -> bool {
    h.state().session.active().unwrap().is_dirty()
}

fn text(d: &Document, id: LayerId) -> String {
    match &d.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => panic!("not a text layer"),
    }
}

/// Blue at the middle of `Photo`: the replacement image is in.
fn photo_blue(d: &Document, id: LayerId) -> bool {
    let s = d.layer(id).unwrap().surface().unwrap();
    let px = photocraft_raster::to_rgba(&s.format(), &s.pixel(32, 24));
    px[2] > 0.9 && px[0] < 0.1
}

/// What the Layers panel and the canvas show of the three variables.
fn shown(h: &H, l: &Layers) -> (bool, String, bool) {
    let d = doc(h);
    (d.layer(l.badge).unwrap().visible, text(&d, l.title), photo_blue(&d, l.photo))
}

fn click(h: &mut H, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn alt_p(h: &mut H) {
    h.event(egui::Event::Key { key: egui::Key::P, physical_key: Some(egui::Key::P), pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(3);
}

#[test]
fn preview_is_off_on_the_first_open_and_alt_p_toggles_it() {
    let (mut h, _l, before, _) = opened();
    assert_eq!(preview_field(&mut h), false);
    alt_p(&mut h);
    assert_eq!(preview_field(&mut h), true, "Alt+P ticks it");
    assert_ne!(*doc(&h), *before, "and the first set is shown");
    alt_p(&mut h);
    assert_eq!(preview_field(&mut h), false, "Alt+P unticks it");
    assert_eq!(*doc(&h), *before);
    assert!(h.state().dialog_open(), "the dialog stays open");
}

#[test]
fn preview_shows_the_selected_set_and_follows_the_selection_without_history() {
    let (mut h, l, before, _) = opened();
    let past = history(&h);
    // Preview off: selecting a row changes nothing.
    click(&mut h, "Data Set 2");
    assert_eq!(*doc(&h), *before);

    click(&mut h, "Preview");
    assert_eq!(preview_field(&mut h), true);
    assert_eq!(shown(&h, &l), (true, "Beta".into(), false), "Data Set 2 at once");
    click(&mut h, "Zed");
    assert_eq!(shown(&h, &l), (false, "Alpha".into(), true), "the preview switches to Zed");
    assert_eq!(history(&h), past, "previewing adds no history state");
    assert!(!dirty(&h), "nor marks the document modified");
    assert!(h.state().session.active().unwrap().doc.variables.active.is_none(), "the set isn't marked applied");

    click(&mut h, "Preview");
    assert_eq!(*doc(&h), *before, "unticked: the document as before the dialog");
    assert_eq!(history(&h), past);
    assert!(!dirty(&h));
    click(&mut h, "Data Set 2");
    assert_eq!(*doc(&h), *before, "off again: selecting changes nothing");
}

#[test]
fn cancel_and_escape_revert_the_preview_without_history() {
    for way in ["cancel", "escape"] {
        let (mut h, l, before, _) = opened();
        let past = history(&h);
        click(&mut h, "Preview");
        click(&mut h, "Data Set 2");
        assert_eq!(shown(&h, &l).1, "Beta", "{way}");
        if way == "cancel" {
            h.get_by_label("Cancel").click();
        } else {
            h.key_press(egui::Key::Escape);
        }
        h.run_steps(3);
        assert!(!h.state().dialog_open(), "{way}");
        assert_eq!(*doc(&h), *before, "{way}");
        assert_eq!(history(&h), past, "{way}");
        assert!(!dirty(&h), "{way}");
    }
}

#[test]
fn apply_with_preview_gives_the_same_document_and_one_history_state() {
    let (mut h, l, before, _) = opened();
    let past = history(&h);
    click(&mut h, "Data Set 2");
    click(&mut h, "Apply");
    let plain = doc(&h);
    assert_eq!(history(&h), past + 1);

    assert!(h.state_mut().session.undo());
    assert_eq!(*doc(&h), *before);
    open(&mut h);
    click(&mut h, "Preview");
    click(&mut h, "Data Set 2");
    assert_eq!(shown(&h, &l).1, "Beta");
    click(&mut h, "Apply");
    assert!(!h.state().dialog_open());
    assert_eq!(*doc(&h), *plain, "the same document as Apply without Preview");
    assert_eq!(history(&h), past + 1, "one history state");
    assert!(h.state_mut().session.undo());
    assert_eq!(*doc(&h), *before, "one Undo reverts the whole apply");
}

#[test]
fn preview_is_kept_for_the_next_open_after_apply_not_after_cancel() {
    let (mut h, l, _, _) = opened();
    click(&mut h, "Preview");
    click(&mut h, "Apply");
    open(&mut h);
    assert_eq!(preview_field(&mut h), true, "on stays on after Apply");
    // The first set is selected again and shown at once (Zed: already applied, so check Data Set 2).
    click(&mut h, "Data Set 2");
    assert_eq!(shown(&h, &l).1, "Beta");

    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(shown(&h, &l).1, "Alpha", "Cancel put back the applied Zed");
    open(&mut h);
    assert_eq!(preview_field(&mut h), false, "off after Cancel");
    click(&mut h, "Apply");
    open(&mut h);
    assert_eq!(preview_field(&mut h), false, "off stays off after Apply");
}

#[test]
fn a_set_whose_replacement_file_is_missing_shows_nothing_of_itself() {
    let (mut h, l, before, _) = opened();
    let past = history(&h);
    click(&mut h, "Preview");
    assert_eq!(shown(&h, &l), (false, "Alpha".into(), true));
    click(&mut h, "Data Set 3");
    assert_eq!(*doc(&h), *before, "no partial preview: neither Badge nor the text of Data Set 3");
    assert_eq!(history(&h), past);
    h.run_steps(5);
    assert_eq!(*doc(&h), *before, "and it isn't retried into something else");
    click(&mut h, "Data Set 2");
    assert_eq!(shown(&h, &l), (true, "Beta".into(), false), "another set still previews");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(*doc(&h), *before);
}

/// `ui.dialog.set` reads and sets Preview; a value the box cannot hold is refused and changes
/// nothing, never a panic.
#[test]
fn control_channel_reads_and_sets_preview() {
    let (mut h, l, before, id) = opened();
    assert_eq!(preview_field(&mut h), false);
    let set = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "preview", "value": true}));
    assert!(!refused(&set), "{set}");
    assert_eq!(shown(&h, &l), (false, "Alpha".into(), true));
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": 1}));
    assert_eq!(shown(&h, &l).1, "Beta");
    for odd in [json!("yes"), json!(1), Value::Null] {
        let reply = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "preview", "value": odd}));
        assert!(refused(&reply), "{odd}: {reply}");
        assert_eq!(shown(&h, &l).1, "Beta", "{odd}: the preview stays");
    }
    let reply = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": 99}));
    assert!(refused(&reply), "an index past the list is refused: {reply}");
    assert_eq!(shown(&h, &l).1, "Beta");
    let cancelled = control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    assert!(!refused(&cancelled), "{cancelled}");
    assert_eq!(*doc(&h), *before);
}

/// A command run while the dialog is open (a script or an action step calls `run` directly) works on
/// the document itself, never on the preview: its history state starts from the real document, and
/// Cancel leaves the command's change without any of the set.
#[test]
fn a_command_run_while_previewing_works_on_the_document_itself() {
    let (mut h, l, before, _) = opened();
    let past = history(&h);
    click(&mut h, "Preview");
    assert_eq!(shown(&h, &l).1, "Alpha");
    h.state_mut().run("layer.new.layer", json!({"name": "Extra"})).unwrap();
    h.run_steps(3);
    assert_eq!(history(&h), past + 1);
    assert_eq!(shown(&h, &l), (false, "Alpha".into(), true), "the dialog shows the set again");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    let d = doc(&h);
    assert!(d.walk().iter().any(|(_, _, layer)| layer.name == "Extra"), "the command's layer stays");
    assert_eq!((d.layer(l.badge).unwrap().visible, text(&d, l.title), photo_blue(&d, l.photo)), (true, "Old".into(), false), "nothing of Zed");
    assert!(h.state_mut().session.undo());
    assert_eq!(*doc(&h), *before, "the command's state undoes to the document before the dialog");
}

/// Another document made active while a set is previewed (not through a command): the previewed
/// document gets its own content back, without being made active.
#[test]
fn another_active_document_gives_the_previewed_one_its_document_back() {
    let (mut a, l) = app();
    a.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    assert!(a.session.set_active(0));
    let mut h = harness(a);
    let before = doc(&h);
    open(&mut h);
    click(&mut h, "Preview");
    assert_eq!(shown(&h, &l).1, "Alpha");
    assert!(h.state_mut().session.set_active(1));
    h.run_steps(3);
    assert_eq!(h.state().session.active_index(), Some(1), "the other document stays active");
    let first = h.state().session.documents()[0].doc.clone();
    assert_eq!(*first, *before, "the previewed document is itself again");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(*h.state().session.documents()[0].doc, *before);
}

/// Autosave never writes the preview: it waits while a set is shown and saves the real document
/// once the preview ends.
#[test]
fn autosave_waits_while_previewing() {
    type Saved = std::sync::Arc<std::sync::Mutex<Vec<Arc<Document>>>>;
    let saved: Saved = Default::default();
    let log = saved.clone();
    let services = photocraft_ui_egui::Services {
        autosave: Some(Box::new(move |d: &Arc<Document>, _rev: u64, _path: Option<&str>| {
            log.lock().unwrap().push(d.clone());
            Ok(())
        })),
        ..Default::default()
    };
    let (mut a, l) = app_with(services);
    // Unsaved work, so autosave has something to write.
    a.run("layer.new.layer", json!({"name": "Work"})).unwrap();
    let mut h = harness(a);
    let before = doc(&h);
    open(&mut h);
    click(&mut h, "Preview");
    assert_eq!(shown(&h, &l).1, "Alpha");
    let ctx = h.ctx.clone();
    photocraft_ui_egui::prefs_ui::autosave_now(h.state_mut());
    photocraft_ui_egui::prefs_ui::tick(h.state_mut(), &ctx, std::time::Instant::now());
    assert!(saved.lock().unwrap().is_empty(), "nothing saved while the preview shows");
    click(&mut h, "Preview");
    photocraft_ui_egui::prefs_ui::autosave_now(h.state_mut());
    photocraft_ui_egui::prefs_ui::tick(h.state_mut(), &ctx, std::time::Instant::now());
    let saved = saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(*saved[0], *before, "the document itself");
}

/// File › Save while the dialog shows a set writes the document itself, never the preview.
#[test]
fn saving_while_previewing_writes_the_document_itself() {
    let encoded: Arc<std::sync::Mutex<Vec<Document>>> = Default::default();
    let log = encoded.clone();
    let services = photocraft_ui_egui::Services {
        export: Some(Box::new(move |d: &Document, _path: &str, _settings: &photocraft_ui_egui::ExportSettings| {
            log.lock().unwrap().push(d.clone());
            Ok((Vec::new(), Vec::new()))
        })),
        write: Some(Box::new(|_path: &str, _bytes: &[u8]| Ok(()))),
        ..Default::default()
    };
    let (mut a, l) = app_with(services);
    let dir = std::env::temp_dir().join(format!("pc-apply-data-set-preview-{}-save", std::process::id()));
    a.session.active_mut().unwrap().path = Some(dir.join("own.psd").to_string_lossy().into_owned());
    a.run("layer.new.layer", json!({"name": "Work"})).unwrap();
    let mut h = harness(a);
    let before = doc(&h);
    open(&mut h);
    click(&mut h, "Preview");
    assert_eq!(shown(&h, &l).1, "Alpha");
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "file.save", json!({})).unwrap();
    let encoded = encoded.lock().unwrap();
    assert_eq!(encoded.len(), 1);
    assert_eq!(encoded[0], *before, "the document itself, nothing of Zed");
}
