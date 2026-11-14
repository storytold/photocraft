//! #14496: the Apply Data Set dialog's Preview checkbox, as in the reference application. Off on
//! the first open, tooltip "Preview the selected data set", Alt+P toggles it. On, the selected set
//! is shown in the document (canvas and Layers panel read it) without a history state, and
//! selecting another row switches the preview; off again, the document is as before the dialog.
//! Cancel, Escape and the close box revert it; Apply with Preview on leaves the same document and
//! the one history state Apply without it does. The box keeps its state for the next open after
//! Apply, not after Cancel. A set that can't be applied (its replacement file is missing) shows
//! nothing of itself.

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, TextLayer};
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use serde_json::{Value, json};

struct Layers {
    photo: LayerId,
    badge: LayerId,
    title: LayerId,
}

/// An 8x8 blue PNG for the Pixel Replacement variable.
fn blue_png() -> String {
    let dir = std::env::temp_dir().join(format!("pc-apply-data-set-preview-{}", std::process::id()));
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
fn app() -> (PhotocraftApp, Layers) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
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

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            photocraft_ui_egui::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, photocraft_ui_egui::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// Image › Apply Data Set… as the menu runs it, then a few frames.
fn open(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.applyDataSet", json!({})).unwrap();
    h.run_steps(3);
}

/// The dialog open on the test document, and the document as it was before.
fn opened() -> (Harness<'static, PhotocraftApp>, Layers, Arc<Document>) {
    let (app, layers) = app();
    let mut h = harness(app);
    let before = doc(&h);
    open(&mut h);
    (h, layers, before)
}

fn doc(h: &Harness<'static, PhotocraftApp>) -> Arc<Document> {
    h.state().session.active().unwrap().doc.clone()
}

fn preview_field(h: &Harness<'static, PhotocraftApp>) -> Value {
    h.state().ui.dialogs.last().unwrap().fields["preview"].clone()
}

fn history(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn dirty(h: &Harness<'static, PhotocraftApp>) -> bool {
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
fn shown(h: &Harness<'static, PhotocraftApp>, l: &Layers) -> (bool, String, bool) {
    let d = doc(h);
    (d.layer(l.badge).unwrap().visible, text(&d, l.title), photo_blue(&d, l.photo))
}

fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn preview_is_off_on_the_first_open_with_its_tooltip_and_alt_p_toggles_it() {
    let (mut h, _l, before) = opened();
    assert_eq!(preview_field(&h), false);
    h.get_by_label("Preview").hover();
    h.run_steps(30);
    h.get_by_label("Preview the selected data set");

    h.event(egui::Event::Key { key: egui::Key::P, physical_key: Some(egui::Key::P), pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(3);
    assert_eq!(preview_field(&h), true, "Alt+P ticks it");
    assert_ne!(*doc(&h), *before, "and the first set is shown");
    h.event(egui::Event::Key { key: egui::Key::P, physical_key: Some(egui::Key::P), pressed: true, repeat: false, modifiers: egui::Modifiers::ALT });
    h.run_steps(3);
    assert_eq!(preview_field(&h), false, "Alt+P unticks it");
    assert_eq!(*doc(&h), *before);
    assert_eq!(h.state().ui.dialogs.len(), 1, "the dialog stays open");
}

#[test]
fn preview_shows_the_selected_set_and_follows_the_selection_without_history() {
    let (mut h, l, before) = opened();
    let past = history(&h);
    // Preview off: selecting a row changes nothing.
    click(&mut h, "Data Set 2");
    assert_eq!(*doc(&h), *before);

    click(&mut h, "Preview");
    assert_eq!(preview_field(&h), true);
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
fn cancel_escape_and_the_close_box_revert_the_preview_without_history() {
    for way in ["cancel", "escape", "close"] {
        if way == "close" && cfg!(target_os = "macos") {
            continue; // No close box on macOS.
        }
        let (mut h, l, before) = opened();
        let past = history(&h);
        click(&mut h, "Preview");
        click(&mut h, "Data Set 2");
        assert_eq!(shown(&h, &l).1, "Beta", "{way}");
        match way {
            "cancel" => h.get_by_label("Cancel").click(),
            "escape" => h.key_press(egui::Key::Escape),
            _ => h.get_by_label("Close").click(),
        }
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "{way}");
        assert_eq!(*doc(&h), *before, "{way}");
        assert_eq!(history(&h), past, "{way}");
        assert!(!dirty(&h), "{way}");
    }
}

#[test]
fn apply_with_preview_gives_the_same_document_and_one_history_state() {
    let (mut h, l, before) = opened();
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
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(*doc(&h), *plain, "the same document as Apply without Preview");
    assert_eq!(history(&h), past + 1, "one history state");
    assert!(h.state_mut().session.undo());
    assert_eq!(*doc(&h), *before, "one Undo reverts the whole apply");
}

#[test]
fn preview_is_kept_for_the_next_open_after_apply_not_after_cancel() {
    let (mut h, l, _) = opened();
    click(&mut h, "Preview");
    click(&mut h, "Apply");
    open(&mut h);
    assert_eq!(preview_field(&h), true, "on stays on after Apply");
    // The first set is selected again and shown at once (Zed: already applied, so check Data Set 2).
    click(&mut h, "Data Set 2");
    assert_eq!(shown(&h, &l).1, "Beta");

    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(shown(&h, &l).1, "Alpha", "Cancel put back the applied Zed");
    open(&mut h);
    assert_eq!(preview_field(&h), false, "off after Cancel");
    click(&mut h, "Apply");
    open(&mut h);
    assert_eq!(preview_field(&h), false, "off stays off after Apply");
}

#[test]
fn a_set_whose_replacement_file_is_missing_shows_nothing_of_itself() {
    let (mut h, l, before) = opened();
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

fn control(h: &mut Harness<'static, PhotocraftApp>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    let reply = match handle(h.state_mut(), &ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    };
    h.run_steps(3);
    reply
}

#[test]
fn control_channel_reads_and_sets_preview() {
    let (mut h, l, before) = opened();
    let inspected = control(&mut h, "ui.inspect", json!({}));
    let d = inspected["result"]["dialogs"].as_array().unwrap().iter().find(|d| d["fields"]["__command"] == "image.applyDataSet").cloned().unwrap();
    assert_eq!(d["fields"]["preview"], false);
    let id = d["id"].clone();
    let set = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "preview", "value": true}));
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(shown(&h, &l), (false, "Alpha".into(), true));
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": 1}));
    assert_eq!(shown(&h, &l).1, "Beta");
    // Anything else in `preview` is off, never a panic.
    for odd in [json!("yes"), json!(1), Value::Null] {
        control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "preview", "value": odd}));
        assert_eq!(*doc(&h), *before, "{odd}");
    }
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "preview", "value": true}));
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": 99}));
    assert_eq!(*doc(&h), *before, "an index past the list shows nothing");
    let cancelled = control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    assert_eq!(*doc(&h), *before);
}
