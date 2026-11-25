//! The Layers panel menu's Layer from Background… (#16765): with the Background active the menu starts with it and a
//! separator, and it converts the Background as Layer › New › Layer from Background… does; with another layer active
//! the menu is unchanged.
use craft::keymap::CommandRef;
use craft::ui::menu::PanelRow;
use serde_json::json;

use super::panel_rows;
use crate::PhotocraftApp;

const FROM_BACKGROUND: &str = "layer.new.layerFromBackground";

/// A document with its Background and a layer "Ink" above it; "Ink" is active.
fn doc_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app
}

fn background(app: &PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    st.doc.walk().into_iter().map(|(_, _, l)| l.id).find(|id| st.doc.is_background(*id)).map(|id| id.0)
}

fn select_background(app: &mut PhotocraftApp) {
    let id = background(app).expect("the document has a Background");
    app.run("layer.select", json!({"layer": id})).unwrap();
}

/// The panel menu's rows, by command id; a separator is `None`.
fn row_ids(app: &PhotocraftApp) -> Vec<Option<String>> {
    panel_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PanelRow::Command(c) => Some(Some(c.id.clone())),
            PanelRow::Separator => Some(None),
            _ => None,
        })
        .collect()
}

#[test]
fn with_the_background_active_the_menu_starts_with_layer_from_background_and_a_separator() {
    let mut app = doc_app();
    let others = row_ids(&app);
    select_background(&mut app);
    let ids = row_ids(&app);
    assert_eq!(ids.first().cloned().flatten().as_deref(), Some(FROM_BACKGROUND), "{ids:?}");
    assert_eq!(ids.get(1), Some(&None), "a separator follows: {ids:?}");
    assert_eq!(ids[2..], others[..], "the rest of the menu is the one another layer gets");
}

#[test]
fn with_another_layer_active_the_menu_has_no_layer_from_background() {
    let app = doc_app();
    let ids = row_ids(&app);
    assert!(!ids.iter().flatten().any(|id| id == FROM_BACKGROUND), "{ids:?}");
    assert_eq!(ids.first().cloned().flatten().as_deref(), Some("layer.new.layer"), "New Layer… stays first: {ids:?}");
}

#[test]
fn the_entry_converts_the_background_like_the_layer_menu_entry() {
    let mut app = doc_app();
    select_background(&mut app);
    assert!(crate::menus::is_enabled(&app, FROM_BACKGROUND), "the entry is available");
    // A picked menu row fires its command, which runs as the menu item does.
    let ctx = app.ctx().clone();
    crate::shortcut_dispatch::run_fired(&mut app, &ctx, &[crate::blend_keys::test_fired(CommandRef::new(FROM_BACKGROUND))]);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(background(&app), None, "the Background became a normal layer");
    assert!(!row_ids(&app).iter().flatten().any(|id| id == FROM_BACKGROUND), "and the entry leaves the menu");
}
