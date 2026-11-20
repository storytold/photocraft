//! #11400: View › Guides' two Selected Guides rows act on the selected guide.

use craft::menu::Row;
use serde_json::json;

use crate::PhotocraftApp;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    app
}

fn item(app: &PhotocraftApp, id: &str) -> Row {
    crate::menus::menu_rows(app).into_iter().find(|r| r.command.id == id).unwrap_or_else(|| panic!("{id} is not in the menu"))
}

fn vertical(app: &PhotocraftApp) -> Vec<f64> {
    app.session.active().unwrap().doc.guides.vertical.clone()
}

#[test]
fn the_menu_cache_follows_the_selected_guide() {
    let mut app = app();
    let none = crate::native_menu::state_hash(&app, None);
    crate::guide_select::select(&mut app, true, 0);
    let one = crate::native_menu::state_hash(&app, None);
    assert_ne!(none, one, "selecting a guide changes the menu state");
    assert!(item(&app, "view.clearSelectedGuides").enabled);
    crate::guide_select::clear(&mut app);
    assert_eq!(crate::native_menu::state_hash(&app, None), none);
    assert!(!item(&app, "view.clearSelectedGuides").enabled);
}

#[test]
fn selected_rows_are_greyed_without_a_selected_guide_and_enabled_with_one() {
    let mut app = app();
    let ctx = app.ctx().clone();
    for id in ["view.editSelectedGuides", "view.clearSelectedGuides"] {
        assert!(!item(&app, id).enabled, "{id} with no guide selected");
        assert!(crate::menus::invoke(&mut app, &ctx, id, json!({})).is_err(), "{id} errors, not a crash");
    }
    crate::guide_select::select(&mut app, true, 0);
    for id in ["view.editSelectedGuides", "view.clearSelectedGuides"] {
        assert!(item(&app, id).enabled, "{id} with a guide selected");
    }
}

#[test]
fn clear_selected_guides_removes_only_the_selected_guide() {
    let mut app = app();
    crate::guide_select::select(&mut app, true, 1);
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "view.clearSelectedGuides", json!({})).unwrap();
    assert_eq!(vertical(&app), vec![10.0]);
}

#[test]
fn edit_selected_guides_opens_the_right_click_menus_dialog() {
    let mut app = app();
    crate::guide_select::select(&mut app, true, 0);
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "view.editSelectedGuides", json!({})).unwrap();
    let open = app.framework.dialogs.open_dialogs();
    assert_eq!(open.len(), 1);
    let (id, from_menu) = (open[0].id, open[0].fields.clone());
    app.framework.dialogs.cancel_by_id(&ctx, id);
    let opened = crate::new_guide_ui::open_edit(&mut app).unwrap();
    let again = app.framework.dialogs.open_dialogs();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].id, opened);
    assert_eq!(again[0].fields, from_menu, "the menu row and the right-click path open the same dialog");
}
