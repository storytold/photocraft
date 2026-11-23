//! Duplicate Layer's Destination › Document list leaves out documents a layer can't be copied
//! into, as Photoshop leaves out Indexed Color documents (#14612). The rest keep tab order and
//! their own document on OK.
use super::destinations;
use crate::PhotocraftApp;
use crate::new_layer_ui::testing::{set, top};
use photocraft_color::ColorMode;
use serde_json::json;

/// A command id and its params.
type Step = (&'static str, serde_json::Value);

/// Poster (RGB, active, with an Ink layer), then one document per `(name, mode, steps)`: a new
/// 8-bit document of `mode` with the mode commands in `steps` run on it.
fn app_with(docs: &[(&str, &str, &[Step])]) -> Box<PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "Poster"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    for (name, mode, steps) in docs {
        s.execute("file.new", json!({"width": 8, "height": 8, "name": name, "mode": mode})).unwrap();
        for (id, params) in *steps {
            s.execute(id, params.clone()).unwrap();
        }
    }
    s.set_active(0);
    PhotocraftApp::new(s, crate::Services::default())
}

fn indexed() -> Step {
    ("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false}))
}

fn keys(app: &PhotocraftApp) -> Vec<String> {
    destinations(app).into_iter().map(|(key, _)| key).collect()
}

/// The keys the documents at these tab indices have: their ids.
fn ids(app: &PhotocraftApp, tabs: &[usize]) -> Vec<String> {
    tabs.iter().map(|&i| app.session.documents()[i].doc.id.0.to_string()).collect()
}

#[test]
fn indexed_documents_are_left_out_and_the_other_modes_keep_tab_order() {
    let app = app_with(&[
        ("Palette", "rgb", &[indexed()]),
        ("Print", "cmyk", &[]),
        ("Gray", "grayscale", &[]),
        ("Lab", "rgb", &[("image.mode.lab", json!({}))]),
        ("Deep", "rgb", &[("image.mode.bits16", json!({}))]),
    ]);
    let modes: Vec<_> = app.session.documents().iter().map(|d| (d.doc.mode, d.doc.depth.bits())).collect();
    assert_eq!(modes[1].0, ColorMode::Indexed);
    assert_eq!(modes[4].0, ColorMode::Lab);
    assert_eq!(modes[5], (ColorMode::Rgb, 16));
    let names: Vec<_> = destinations(&app).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["Poster", "Print", "Gray", "Lab", "Deep"]);
    assert_eq!(keys(&app), ids(&app, &[0, 2, 3, 4, 5]), "each entry is keyed by its document");
}

#[test]
fn bitmap_and_multichannel_documents_are_left_out_too() {
    let app = app_with(&[
        ("Mono", "grayscale", &[("image.mode.bitmap", json!({"method": "threshold"}))]),
        ("Plates", "rgb", &[("image.mode.multichannel", json!({}))]),
        ("Card", "rgb", &[]),
    ]);
    assert_eq!(app.session.documents()[1].doc.mode, ColorMode::Bitmap);
    assert_eq!(app.session.documents()[2].doc.mode, ColorMode::Multichannel);
    assert_eq!(keys(&app), ids(&app, &[0, 3]), "layer.copyToDocument refuses both modes");
}

#[test]
fn ok_on_the_document_after_an_indexed_one_copies_into_that_document() {
    let mut app = app_with(&[("Palette", "rgb", &[indexed()]), ("Card", "rgb", &[])]);
    assert_eq!(keys(&app), ids(&app, &[0, 2]));
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "layer.duplicate", json!({})).unwrap();
    let dialog = top(&app);
    let palette_layers = app.session.documents()[1].doc.layer_count();
    let palette = ids(&app, &[1]).remove(0);
    assert!(set(&mut app, dialog, "document", json!(palette)).is_err(), "the Indexed document is not a choice");
    let card = ids(&app, &[2]).remove(0);
    set(&mut app, dialog, "document", json!(card)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
    // The passes after OK deliver the held answer to the family that runs it.
    for _ in 0..2 {
        crate::dialogs::testing::show(&mut app);
    }
    let names: Vec<_> = app.session.documents()[2].doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect();
    assert!(names.contains(&"Ink copy".to_string()), "Card receives the copy: {names:?}");
    assert_eq!(app.session.documents()[1].doc.layer_count(), palette_layers, "the Indexed document is untouched");
    assert_eq!(app.session.documents()[0].doc.layer_count(), 2, "the source is untouched");
}

#[test]
fn the_source_stays_listed_whatever_its_mode() {
    let mut app = app_with(&[("Palette", "rgb", &[indexed()]), ("Card", "rgb", &[])]);
    app.session.set_active(1);
    assert_eq!(keys(&app), ids(&app, &[0, 1, 2]), "duplicating within the source is a plain duplicate");
}
