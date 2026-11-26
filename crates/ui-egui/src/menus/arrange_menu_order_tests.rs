//! Window › Arrange: the reference app's order and a document-named last entry (#16888).

use super::*;

fn app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn arrange(app: &PhotocraftApp) -> Vec<craft::menu::Row> {
    menu_rows(app).into_iter().filter(|r| r.path.len() == 2 && r.path[0] == "Window" && r.path[1] == "Arrange").collect()
}

fn ids(rows: &[craft::menu::Row]) -> Vec<String> {
    rows.iter().map(|r| if r.label == "---" { "---".to_string() } else { r.command.id.clone() }).collect()
}

fn new_window_label(app: &PhotocraftApp) -> String {
    arrange(app).into_iter().find(|r| r.command.id == "window.arrange.newWindowForDocument").unwrap().label
}

#[test]
fn arrange_rows_are_in_the_reference_order() {
    let got = ids(&arrange(&app()));
    let got: Vec<&str> = got.iter().map(String::as_str).collect();
    assert_eq!(
        got,
        vec![
            "window.arrange.tileAllVertically",
            "window.arrange.tileAllHorizontally",
            "window.arrange.twoUpHorizontal",
            "window.arrange.twoUpVertical",
            "window.arrange.threeUpHorizontal",
            "window.arrange.threeUpVertical",
            "window.arrange.threeUpStacked",
            "window.arrange.fourUp",
            "window.arrange.sixUp",
            "window.arrange.consolidateAllToTabs",
            "---",
            "window.arrange.cascade",
            "window.arrange.tile",
            "window.arrange.floatInWindow",
            "window.arrange.floatAllInWindows",
            "---",
            "window.arrange.matchZoom",
            "window.arrange.matchLocation",
            "window.arrange.matchRotation",
            "window.arrange.matchAll",
            "---",
            "window.arrange.newWindowForDocument",
        ]
    );
}

#[test]
fn new_window_entry_follows_the_active_document() {
    let mut app = app();
    assert_eq!(new_window_label(&app), "New Window", "no document keeps the fixed label");

    app.run("file.new", json!({"width": 32, "height": 32, "name": "W1"})).unwrap();
    app.run("file.new", json!({"width": 32, "height": 32, "name": "W2"})).unwrap();
    assert_eq!(new_window_label(&app), "New Window for W2");

    app.session.set_active(0);
    assert_eq!(new_window_label(&app), "New Window for W1");
}

#[test]
fn new_window_ids_shortcuts_and_rules_are_unchanged() {
    let mut app = app();
    app.run("file.new", json!({"width": 32, "height": 32, "name": "W1"})).unwrap();
    let rows = arrange(&app);
    let last = rows.last().unwrap();
    assert_eq!(last.command.id, "window.arrange.newWindowForDocument");
    assert!(last.enabled, "the entry is live with a document open");
    assert!(rows.iter().all(|r| r.shortcut.is_none()), "no Arrange entry has a shortcut");
    assert_eq!(
        super::state::translated_menu_label(crate::i18n::Lang::EN, &last.command.id, &last.label),
        "New Window for W1",
        "the document name is kept when the sentence is translated"
    );
}
