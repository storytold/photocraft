//! #13803: Edit › Fade names the step it fades (`Fade Invert…`), reads plain `Fade…` and is
//! greyed when nothing can be faded, sits in its own group between separators, and the selection
//! right-click menu and `ui.menu.list` show the same label.

use super::*;
use serde_json::json;

fn app() -> crate::PhotocraftApp {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app
}

/// Edit › Fade's (label, enabled) as the menus and `ui.menu.list` report it.
fn fade(app: &crate::PhotocraftApp) -> (String, bool) {
    let item = menu_items(app).into_iter().find(|i| i.id == "edit.fade").expect("Edit › Fade");
    (item.label, item.enabled)
}

#[test]
fn the_label_names_the_step_it_fades() {
    let mut app = app();
    app.run("image.adjustments.invert", json!({})).unwrap();
    assert_eq!(fade(&app), ("Fade Invert…".into(), true));
    app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    assert_eq!(fade(&app).0, "Fade Gaussian Blur…");
    app.run("paint.stroke", json!({"points": [[8, 20], [56, 20]], "size": 6})).unwrap();
    assert_eq!(fade(&app).0, "Fade Brush Tool…");
    assert_eq!(fade_label(&app), "Fade Brush Tool…", "the right-click menu's row shows the same");
}

#[test]
fn nothing_to_fade_reads_plain_fade_and_is_greyed() {
    let mut app = app();
    assert_eq!(fade(&app), ("Fade…".into(), false), "a fresh document");
    app.run("image.adjustments.invert", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    assert_eq!(fade(&app), ("Fade…".into(), false), "after Deselect");
    assert_eq!(fade_label(&app), "Fade…");
    let empty = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_eq!(fade(&empty), ("Fade…".into(), false), "no document");
}

#[test]
fn fade_sits_in_its_own_group() {
    let items = menu_items(&app());
    let edit: Vec<&str> = items.iter().filter(|i| i.path.len() == 1 && i.path[0] == "Edit").map(|i| i.label.as_str()).take(7).collect();
    assert_eq!(edit, ["Undo", "Redo", "Toggle Last State", "---", "Fade…", "---", "Cut"], "{edit:?}");
}
