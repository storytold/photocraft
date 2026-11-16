//! #13012: a Pen path that starts a Work Path is the History step "New Work Path".
use super::*;

/// A 200 by 200 document with the Pen in Path mode.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "path".into();
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    pen_down(app, x, y, false);
    pen_up(app);
}

fn undo_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn enter_records_new_work_path_and_the_next_path_again() {
    let mut app = app();
    for offset in [0.0, 50.0] {
        let n = steps(&app);
        click(&mut app, 20.0 + offset, 20.0);
        click(&mut app, 80.0 + offset, 20.0);
        pen_enter(&mut app);
        assert_eq!(steps(&app), n + 1);
        assert_eq!(undo_label(&app).as_deref(), Some("New Work Path"));
        assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.len(), 1, "replaces the old Work Path");
    }
}

#[test]
fn escape_and_closing_on_the_first_anchor_record_new_work_path() {
    let mut app = app();
    click(&mut app, 20.0, 20.0);
    click(&mut app, 80.0, 20.0);
    pen_end(&mut app);
    assert_eq!(undo_label(&app).as_deref(), Some("New Work Path"));

    let mut app = self::app();
    for [x, y] in [[20.0, 20.0], [80.0, 20.0], [80.0, 80.0], [20.0, 20.0]] {
        click(&mut app, x, y);
    }
    assert!(app.ui.pen.is_none(), "the click on the first anchor commits");
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);
    assert_eq!(undo_label(&app).as_deref(), Some("New Work Path"));
}

#[test]
fn a_path_added_to_the_selected_work_path_keeps_its_label() {
    let mut app = app();
    click(&mut app, 20.0, 20.0);
    click(&mut app, 80.0, 20.0);
    pen_end(&mut app);
    // Esc leaves the Work Path selected, so the next path joins it.
    click(&mut app, 20.0, 80.0);
    click(&mut app, 80.0, 80.0);
    pen_end(&mut app);
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.len(), 2);
    assert_eq!(undo_label(&app).as_deref(), Some("Work Path"));
}

#[test]
fn new_work_path_label_is_refused_for_saved_paths_and_shapes() {
    let mut app = app();
    let path = json!({"subpaths": [{"knots": [[20, 20], [80, 80]]}]});
    let n = steps(&app);
    assert!(app.run("path.set", json!({"name": "Path 1", "path": path, "historyName": "New Work Path"})).is_err());
    assert!(app.run("shape.create", json!({"kind": "path", "path": path, "historyName": "New Work Path"})).is_err());
    assert_eq!(steps(&app), n);
}
