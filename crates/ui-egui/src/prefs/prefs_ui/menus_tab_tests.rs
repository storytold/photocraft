//! #16433: Edit › Menus lists File › Open Recent as one row, not a row per recent file.

use super::shortcuts::listed_items;
use crate::PhotocraftApp;

#[test]
fn the_menus_tab_lists_no_recent_files() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    *app.ui.recent_files = vec!["/tmp/a.png".into(), "/dir/b.psd".into()];
    let rows = crate::menus::base_rows(&app);
    let recent: Vec<String> =
        listed_items(&rows).iter().filter(|r| r.path.iter().any(|p| p == "Open Recent")).map(|r| format!("{} {}", r.command.id, r.label)).collect();
    assert_eq!(recent, ["file.clearRecent Clear Recent File List"], "Open Recent has one row in the Menus tab");
    assert!(listed_items(&rows).iter().any(|r| r.command.id == "file.open"), "other items stay listed");
}

#[test]
fn recent_files_ignore_legacy_position_colors() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    *app.ui.recent_files = vec!["/tmp/a.png".into()];
    app.run(
        "prefs.set",
        serde_json::json!({"values": {
            "interface.showMenuColors": true,
            "menus.colors.file.openRecent.0": "red",
            "menus.colors.file.open": "blue"
        }}),
    )
    .unwrap();

    let rows = crate::menus::menu_rows(&app);
    let tint = |id| rows.iter().find(|r| r.command.id == id).unwrap().tint;
    assert!(tint("file.openRecent.0").is_none());
    assert!(tint("file.open").is_some());
}
