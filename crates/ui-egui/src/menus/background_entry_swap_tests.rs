//! Layer › New lists Layer from Background… or Background from Layer in one slot, never both (#16761).

use serde_json::json;

use super::*;
use crate::control::{ControlRequest, Reply};

const LAYER_FROM: &str = "layer.new.layerFromBackground";
const BACKGROUND_FROM: &str = "layer.new.backgroundFromLayer";

fn app(doc: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if doc {
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
    }
    app
}

/// The Layer › New rows the menu bar shows, as (id, label, enabled).
fn layer_new(app: &PhotocraftApp) -> Vec<(String, String, bool)> {
    menu_rows(app).into_iter().filter(|r| r.path == ["Layer", "New"]).map(|r| (r.command.id, r.label, r.enabled)).collect()
}

/// Which of the two entries `ui.menu.list` and Help search report.
fn listed_and_searched(app: &mut PhotocraftApp) -> (Vec<String>, Vec<String>) {
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new("ui.menu.list", json!({}));
    let Reply::Done(listed) = crate::control::handle(app, &ctx, &req) else { panic!("ui.menu.list: expected an immediate reply") };
    let pair = |id: &str| id == LAYER_FROM || id == BACKGROUND_FROM;
    let listed = listed["result"].as_array().unwrap().iter().filter_map(|it| it["id"].as_str()).filter(|id| pair(id)).map(String::from).collect();
    let searched = crate::menus::help_entries(app, &ctx).iter().map(|e| e.command().id.clone()).filter(|id| pair(id)).collect();
    (listed, searched)
}

#[test]
fn with_a_background_only_layer_from_background_is_listed() {
    let mut a = app(true);
    assert!(a.session.active().unwrap().doc.background().is_some(), "a new white document has a Background");
    let rows = layer_new(&a);
    assert_eq!(rows[1].0, LAYER_FROM, "second entry of Layer › New: {rows:?}");
    assert!(rows[1].2, "Layer from Background… runs with the Background active");
    assert!(rows.iter().all(|r| r.0 != BACKGROUND_FROM), "{rows:?}");
    let (listed, searched) = listed_and_searched(&mut a);
    assert_eq!(listed, [LAYER_FROM]);
    assert_eq!(searched, [LAYER_FROM]);
}

#[test]
fn without_a_background_only_background_from_layer_is_listed_in_that_slot() {
    let mut a = app(true);
    a.run(LAYER_FROM, json!({})).unwrap();
    assert!(a.session.active().unwrap().doc.background().is_none());
    let rows = layer_new(&a);
    assert_eq!(rows[1].0, BACKGROUND_FROM, "Background from Layer takes the same slot: {rows:?}");
    assert_eq!(rows[1].1, "Background from Layer");
    assert!(rows[1].2, "it runs on the former Background");
    assert!(rows.iter().all(|r| r.0 != LAYER_FROM), "{rows:?}");
    let (listed, searched) = listed_and_searched(&mut a);
    assert_eq!(listed, [BACKGROUND_FROM]);
    assert_eq!(searched, [BACKGROUND_FROM]);
}

#[test]
fn with_no_document_the_slot_reads_layer_from_background_greyed() {
    let a = app(false);
    let rows = layer_new(&a);
    assert_eq!(rows[1].0, LAYER_FROM, "{rows:?}");
    assert!(!rows[1].2, "greyed with no document");
    assert!(rows.iter().all(|r| r.0 != BACKGROUND_FROM), "{rows:?}");
}

/// The Keyboard Shortcuts dialog reads the rows before the swap, so either command can take a shortcut at any time.
#[test]
fn the_keyboard_shortcuts_rows_keep_both_entries() {
    for doc in [false, true] {
        let rows = base_rows(&app(doc));
        for id in [LAYER_FROM, BACKGROUND_FROM] {
            assert!(rows.iter().any(|r| r.command.id == id), "doc {doc}: {id}");
        }
    }
}
