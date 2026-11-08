//! #11975: with a selection, the Perspective Crop tool's first drag (or first corner click) drops
//! it as its own "Deselect" step, so the selection is gone after a commit and after Esc.

use serde_json::json;

use super::*;

/// A 200 × 150 document with a selection and the Perspective Crop tool.
fn app(selection: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    if selection {
        app.run("select.rect", json!({"x": 30, "y": 30, "width": 60, "height": 40})).unwrap();
    }
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

fn selected(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.selection.is_some()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 });
    pointer(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 });
    pointer(app, ToolEvent::Up { x: to[0], y: to[1] });
}

#[test]
fn the_first_drag_deselects_as_its_own_step() {
    let mut app = app(true);
    let before = steps(&app);
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    assert!(!selected(&app), "the selection is gone");
    assert_eq!(steps(&app), before + 1, "one step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Deselect"));
    assert!(app.ui.perspective_crop.is_some(), "the quad appears");
    // Esc afterwards leaves the document deselected.
    cancel(&mut app);
    assert!(!selected(&app));
}

#[test]
fn the_first_corner_click_deselects_once() {
    let mut app = app(true);
    let before = steps(&app);
    for p in [[20.0, 20.0], [180.0, 25.0], [175.0, 130.0], [25.0, 125.0]] {
        drag(&mut app, p, p);
    }
    assert!(!selected(&app));
    assert_eq!(steps(&app), before + 1, "one Deselect for the four clicks");
    assert!(app.ui.perspective_crop.is_some(), "four clicks made the quad");
}

#[test]
fn without_a_selection_no_step_is_added() {
    let mut app = app(false);
    let before = steps(&app);
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    assert_eq!(steps(&app), before, "no Deselect state");
    // Editing the quad later adds none either.
    let q = app.ui.perspective_crop.unwrap();
    drag(&mut app, q[0], [30.0, 30.0]);
    assert_eq!(steps(&app), before);
}

#[test]
fn choosing_the_tool_keeps_the_selection_and_commit_leaves_it_gone() {
    let mut app = app(true);
    assert!(selected(&app), "choosing the tool keeps the selection");
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    commit(&mut app);
    assert!(!selected(&app));
    assert!(app.ui.perspective_crop.is_none(), "committed");
}

#[test]
fn a_selection_made_between_corner_clicks_is_kept() {
    let mut app = app(true);
    let p = [[20.0, 20.0], [180.0, 25.0], [175.0, 130.0], [25.0, 125.0]];
    drag(&mut app, p[0], p[0]);
    // Select All after the first corner: the later clicks belong to the same quad.
    app.run("select.all", json!({})).unwrap();
    let before = steps(&app);
    for c in &p[1..] {
        drag(&mut app, *c, *c);
    }
    assert!(selected(&app), "only the quad's first press deselects");
    assert_eq!(steps(&app), before, "no second Deselect step");
    assert!(app.ui.perspective_crop.is_some(), "four clicks made the quad");
}
