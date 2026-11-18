//! #12761: with the Delete Anchor Point tool, Delete / Backspace on a selected anchor edits the
//! path, never the active layer. The single anchor left of a subpath takes the whole subpath
//! (History: Delete Selection).

use egui::Modifiers;
use serde_json::json;

use super::add_anchor_delete_key_tests::{app as add_app, ctrl_click, layers, path1, press, steps};
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// The measured document: Path 1 holds a square, the open line B (400,300) to (700,300) and an
/// S-curve; the Delete Anchor Point tool is on.
fn app() -> PhotocraftApp {
    let mut app = add_app();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 100], [200, 100], [200, 200], [100, 200]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [[100, 400], [150, 350], [200, 450], [250, 400]]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.tool = Tool::DeleteAnchorPoint;
    app
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    testing::click(app, at, Modifiers::NONE);
}

/// B's end deleted by a click, then B's last anchor selected.
fn with_lone_anchor_selected() -> PhotocraftApp {
    let mut app = app();
    click(&mut app, [700.0, 300.0]);
    ctrl_click(&mut app, [400.0, 300.0]);
    assert_eq!(app.ui.direct_selection.anchors, vec![[1, 0]], "Ctrl+click selects B's last anchor");
    app
}

#[test]
fn delete_and_backspace_on_a_subpaths_last_anchor_remove_the_subpath_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = with_lone_anchor_selected();
        let (before, n, count) = (path1(&app), steps(&app), layers(&app));
        assert_eq!(before.subpaths.len(), 3, "{key:?}: B is down to one anchor");
        press(&mut app, key);
        assert_eq!(layers(&app), count, "{key:?}: every layer is still there");
        assert_eq!(path1(&app).subpaths.len(), 2, "{key:?}: Path 1 keeps the square and the S-curve");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (n + 1, Some("Delete Selection")), "{key:?}");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}: Undo brings B back");
    }
}
