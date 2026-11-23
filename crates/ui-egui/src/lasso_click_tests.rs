//! #13612: a Lasso click on the pasteboard deselects only in the New selection mode, as one step Undo
//! restores; with Shift or Alt held (Add, Subtract, Intersect) it keeps the selection and records
//! nothing. A drag that starts on the pasteboard still selects.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Lasso;
    app
}

#[test]
fn pasteboard_click_deselects_only_in_new_mode_and_is_undoable() {
    for mods in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app();
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 80, "height": 80})).unwrap();
        let history = app.session.active().unwrap().history.past_len();
        testing::click(&mut app, [-20.0, 100.0], mods);
        let state = app.session.active().unwrap();
        if mods == Modifiers::NONE {
            assert!(state.doc.selection.is_none(), "New-mode click deselects");
            assert_eq!(state.history.past_len(), history + 1);
            app.run("edit.undo", json!({})).unwrap();
            assert!(app.session.active().unwrap().doc.selection.is_some(), "undo restores the selection");
        } else {
            assert!(state.doc.selection.is_some(), "modified selection mode keeps the selection ({mods:?})");
            assert_eq!(state.history.past_len(), history, "{mods:?}");
        }
    }
}

#[test]
fn lasso_drag_from_pasteboard_still_creates_a_selection() {
    let mut app = app();
    testing::press(&mut app, [-20.0, 80.0], Modifiers::NONE);
    testing::move_to(&mut app, [100.0, 80.0], Modifiers::NONE);
    testing::move_to(&mut app, [100.0, 160.0], Modifiers::NONE);
    testing::release(&mut app, [-20.0, 160.0], Modifiers::NONE);
    let selection = app.session.active().unwrap().doc.selection.as_ref().expect("a drag still selects");
    assert!(selection.sample_channel(50, 120, 0) > 0.5);
}
