//! #1810: File › New starts from the last document made with the dialog (Photoshop does), with the
//! clipboard image's size still taking priority. Also the hosted dialog's test harness, which the
//! New Document tests share.

use craft::behavior::PixelSize;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{BackgroundContents, BitDepth, ColorMode, DocumentOptions};
use craft::ui::runtime::env::UnitKind;
use egui::vec2;
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The app under a window, as a frame runs it.
pub(crate) type AppHarness = Harness<'static, PhotocraftApp>;

pub(crate) fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// `app` with the hosted dialogs run each frame, the way the app's frame runs them (their pumps included).
pub(crate) fn harness(app: PhotocraftApp) -> AppHarness {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(ctx)
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::run(app, ui), app);
    h.run_steps(2);
    h
}

/// Opens File › New and lets a frame show it.
pub(crate) fn open(h: &mut AppHarness) -> DialogId {
    let id = crate::new_doc_ui::open(h.state_mut());
    h.run_steps(2);
    id
}

/// The settings the open dialog `id` shows, by their serde keys.
pub(crate) fn fields(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields).expect("the New Document dialog is open")
}

/// The settings the open dialog `id` shows.
pub(crate) fn options(app: &PhotocraftApp, id: DialogId) -> DocumentOptions {
    serde_json::from_value(fields(app, id)).expect("the dialog's fields are its settings")
}

/// Sets every setting of dialog `id` to `options`, one field at a time through the host.
pub(crate) fn set_options(h: &mut AppHarness, id: DialogId, options: &DocumentOptions) {
    let ctx = h.ctx.clone();
    let Value::Object(all) = serde_json::to_value(options).expect("settings serialise") else { panic!("settings are an object") };
    for (field, value) in all {
        h.state_mut().framework.dialogs.set_json_field(&ctx, id, &field, value).unwrap_or_else(|e| panic!("the dialog has a `{field}` field: {e}"));
    }
    h.run_steps(1);
}

/// Presses Create on dialog `id`; the frames after it run the pump.
pub(crate) fn press_create(h: &mut AppHarness, id: DialogId) -> bool {
    let enabled = crate::dialogs::testing::confirm(h.state_mut(), id).is_ok();
    h.run_steps(3);
    enabled
}

/// Opens New Document, applies `edit` to the settings it shows and presses Create.
fn create(h: &mut AppHarness, edit: impl FnOnce(&mut DocumentOptions)) -> DialogId {
    let id = open(h);
    let mut shown = options(h.state(), id);
    edit(&mut shown);
    set_options(h, id, &shown);
    press_create(h, id);
    id
}

#[test]
fn new_document_starts_from_the_last_settings() {
    let mut h = harness(app());
    // The first time: PhotoCraft's defaults (1920 × 1080, 72 ppi).
    let first = h.state_mut().new_document_options();
    assert_eq!((first.size, first.pixels_per_inch, first.name.as_str()), (PixelSize::new(1920, 1080), 72.0, "Untitled-1"));
    create(&mut h, |o| {
        o.name = "Poster".into();
        o.size = PixelSize::new(640, 480);
        o.pixels_per_inch = 300.0;
        o.color_mode = ColorMode::Cmyk;
        o.depth = BitDepth::Sixteen;
        o.background = BackgroundContents::Transparent;
        o.unit = UnitKind::Inches;
    });
    let d = &h.state().session.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (640, 480, 300.0));
    // The next time: those settings, but not the name.
    let next = h.state_mut().new_document_options();
    assert_eq!(
        (next.size, next.pixels_per_inch, next.color_mode, next.depth, next.background, next.unit, next.name.as_str()),
        (PixelSize::new(640, 480), 300.0, ColorMode::Cmyk, BitDepth::Sixteen, BackgroundContents::Transparent, UnitKind::Inches, "Untitled-1")
    );

    // A fresh install reads them from the SAVED tier's stored form.
    // PLAN: the store's round trip across a restart is hub-w1's and top-w30's (`Services::settings`); the stored value is moved by its serde form here.
    let stored = craft::ui::runtime::memory::saved_value::<Option<DocumentOptions>>(h.state().ctx(), &crate::new_doc_ui::LAST_CREATED);
    let mut restarted = app();
    craft::ui::runtime::memory::set_saved_value(restarted.ctx(), &crate::new_doc_ui::LAST_CREATED, stored);
    assert_eq!(restarted.new_document_options(), next);

    // An image on the clipboard still sets the size (the Clipboard card); the rest is remembered.
    h.state_mut().run("select.rect", json!({"x": 0, "y": 0, "width": 123, "height": 45})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#336699"})).unwrap();
    h.state_mut().run("edit.copy", json!({})).unwrap();
    let id = open(&mut h);
    let shown = options(h.state(), id);
    assert_eq!(shown.size, PixelSize::new(123, 45));
    assert_eq!((shown.color_mode, shown.depth), (ColorMode::Cmyk, BitDepth::Sixteen));
}

#[test]
fn a_corrupt_remembered_value_is_ignored() {
    let mut h = harness(app());
    create(&mut h, |o| o.size = PixelSize::new(800, 600));
    let kept = craft::ui::runtime::memory::saved_value::<Option<DocumentOptions>>(h.state().ctx(), &crate::new_doc_ui::LAST_CREATED);
    assert_eq!(kept.as_ref().map(|o| o.size), Some(PixelSize::new(800, 600)));
    let documents = h.state().session.documents().len();
    let recent = h.state().session.presets.recent_documents.clone();
    // A Create the engine refuses (an unknown profile) stores nothing and adds no recent entry.
    create(&mut h, |o| {
        o.size = PixelSize::new(321, 123);
        o.profile = Some("no-such-profile".into());
    });
    assert_eq!(h.state().session.documents().len(), documents, "no document was made");
    assert_eq!(craft::ui::runtime::memory::saved_value::<Option<DocumentOptions>>(h.state().ctx(), &crate::new_doc_ui::LAST_CREATED), kept, "unchanged");
    assert_eq!(h.state().session.presets.recent_documents, recent);
}

/// #3131: File › New › Recent lists the documents created with the dialog, newest first, and
/// survives a restart.
#[test]
fn created_documents_fill_the_recent_category() {
    let mut h = harness(app());
    create(&mut h, |o| {
        o.size = PixelSize::new(640, 480);
        o.pixels_per_inch = 300.0;
        o.color_mode = ColorMode::Cmyk;
        o.depth = BitDepth::Sixteen;
        o.background = BackgroundContents::Transparent;
        o.unit = UnitKind::Inches;
    });
    create(&mut h, |o| {
        o.size = PixelSize::new(1920, 1080);
        o.pixels_per_inch = 72.0;
        o.color_mode = ColorMode::Rgb;
        o.depth = BitDepth::Eight;
        o.background = BackgroundContents::White;
        o.unit = UnitKind::Pixels;
    });
    let recent = h.state().session.presets.recent_documents.clone();
    assert_eq!(recent.iter().map(|r| (r.width, r.unit.as_str())).collect::<Vec<_>>(), [(1920, "px"), (640, "in")]);
    // The settings are kept with the preferences.
    let mut restarted = photocraft_engine::Session::new();
    restarted.load_prefs_json(&h.state().session.prefs_to_json()).unwrap();
    assert_eq!(restarted.presets.recent_documents, recent);
    // The list the dialog opens with (its bound `recent`) holds every setting of each entry.
    let listed: Vec<DocumentOptions> = restarted.presets.recent_documents.iter().map(crate::new_doc_ui::to_options).collect();
    assert_eq!(
        (listed[1].size, listed[1].unit, listed[1].color_mode, listed[1].depth),
        (PixelSize::new(640, 480), UnitKind::Inches, ColorMode::Cmyk, BitDepth::Sixteen)
    );
    assert_eq!(listed[1].background, BackgroundContents::Transparent);
}
