//! #12736: the Convert Point tool's History steps read "Change Anchor Point" (a click or drag on
//! an anchor) and "Corner Drag" (a handle drag), as in the reference. Direct Selection, the Pen's
//! ⌥ conversion and the Curvature Pen's double-click keep their names.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A square work path; knot 1 at (120, 40) is smooth with handles at (100, 40) and (140, 40).
fn with_path(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 40], "out": [140, 40], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

pub(super) fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    testing::press(app, from, mods);
    testing::move_to(app, to, mods);
    testing::release(app, to, mods);
}

pub(super) fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    testing::click(app, at, mods);
}

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

/// Runs `gesture` and returns the History steps it added.
pub(super) fn added(app: &mut PhotocraftApp, gesture: impl FnOnce(&mut PhotocraftApp)) -> Vec<String> {
    let n = names(app).len();
    gesture(app);
    names(app).split_off(n)
}

/// `tool` with the smooth anchor selected so its handles show: a ⌘-click, Direct Selection in
/// each of these tools (a selection made with another tool is dropped when the Pen is taken).
fn with_shown_handles(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = app(tool);
    with_path(&mut app);
    click(&mut app, [120.0, 40.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    app
}

#[test]
fn convert_point_handle_drags_are_corner_drag() {
    let one = vec!["Corner Drag".to_string()];
    for mods in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT] {
        let mut app = with_shown_handles(Tool::ConvertPoint);
        assert_eq!(added(&mut app, |a| drag(a, [140.0, 40.0], [140.0, 70.0], mods)), one, "{mods:?}");
        let k = app.session.active().unwrap().doc.work_path.clone().unwrap().subpaths[0].knots[1];
        assert!(!k.smooth && k.in_ctrl == photocraft_geom::Point::new(100.0, 40.0), "the handle moved alone: {k:?}");
        let click_anchor = added(&mut app, |a| click(a, [120.0, 40.0], Modifiers::NONE));
        assert_eq!(click_anchor, ["Change Anchor Point"], "click on an anchor with broken handles");
    }
}

#[test]
fn other_tools_keep_their_step_names() {
    let direct = vec!["Drag Control Point".to_string()];
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        let mut ds = with_shown_handles(Tool::DirectSelection);
        assert_eq!(added(&mut ds, |a| drag(a, [140.0, 40.0], [140.0, 70.0], mods)), direct, "Direct Selection {mods:?}");
    }
    for mods in [Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::ALT] {
        let mut convert_cmd = with_shown_handles(Tool::ConvertPoint);
        assert_eq!(added(&mut convert_cmd, |a| drag(a, [140.0, 40.0], [120.0, 60.0], mods)), direct, "Convert Point with {mods:?} is Direct Selection");
    }
    let mut pen_alt = with_shown_handles(Tool::Pen);
    assert_eq!(added(&mut pen_alt, |a| drag(a, [140.0, 40.0], [140.0, 70.0], Modifiers::ALT)), direct, "Pen ⌥ on a handle");

    let convert = vec!["Convert Point".to_string()];
    let mut pen = app(Tool::Pen);
    with_path(&mut pen);
    assert_eq!(added(&mut pen, |a| click(a, [120.0, 40.0], Modifiers::ALT)), convert, "Pen ⌥-click");
    assert_eq!(added(&mut pen, |a| drag(a, [40.0, 120.0], [40.0, 140.0], Modifiers::ALT)), convert, "Pen ⌥-drag");
    pen.ui.tool = Tool::CurvaturePen;
    assert_eq!(added(&mut pen, |a| testing::double_click(a, [120.0, 120.0], Modifiers::NONE)), convert, "Curvature Pen double-click");
}

#[test]
fn the_commands_take_the_step_name_and_refuse_any_other() {
    let mut app = app(Tool::ConvertPoint);
    with_path(&mut app);
    let convert = json!({"subpath": 0, "knot": 1});
    let handle = json!({"subpath": 0, "knot": 1, "handle": "out", "to": [150, 60], "independent": true});
    let mut run = |id: &str, params: &serde_json::Value| added(&mut app, |a| a.run(id, params.clone()).map(|_| ()).unwrap());
    assert_eq!(run("path.convertPoint", &convert), ["Convert Point"]);
    assert_eq!(run("path.moveHandle", &handle), ["Drag Control Point"]);
    let named = |p: &serde_json::Value, n: &str| {
        let mut p = p.clone();
        p["stepName"] = json!(n);
        p
    };
    assert_eq!(run("path.convertPoint", &named(&convert, "Change Anchor Point")), ["Change Anchor Point"]);
    assert_eq!(run("path.moveHandle", &named(&handle, "Corner Drag")), ["Corner Drag"]);
    let mut curvature = named(&convert, "Change Anchor Point");
    curvature["curvature"] = json!(true);
    assert_eq!(run("path.convertPoint", &curvature), ["Change Anchor Point"]);
}

#[test]
fn a_bad_step_name_is_an_error_and_changes_nothing() {
    let mut app = app(Tool::ConvertPoint);
    with_path(&mut app);
    let before = (app.session.active().unwrap().doc.work_path.clone(), names(&app));
    for bad in [json!("Corner Drag"), json!("Change Anchor Point "), json!(""), json!(7), json!(true), json!(["Change Anchor Point"])] {
        let e = app.run("path.convertPoint", json!({"subpath": 0, "knot": 1, "stepName": bad})).unwrap_err();
        assert!(e.contains("stepName"), "{bad}: {e}");
    }
    for bad in [json!("Change Anchor Point"), json!("Drag Control Point"), json!(0), json!({})] {
        let e = app.run("path.moveHandle", json!({"subpath": 0, "knot": 1, "handle": "out", "to": [150, 60], "stepName": bad})).unwrap_err();
        assert!(e.contains("stepName"), "{bad}: {e}");
    }
    assert_eq!((app.session.active().unwrap().doc.work_path.clone(), names(&app)), before);
}
