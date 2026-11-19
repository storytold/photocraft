//! Retouch options-bar behaviour that needs the whole bar host: the Content-Aware Move box
//! disabling its cells, and a stored fraction surviving the Blur, Sharpen and Smudge bars.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{answer, pending};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The options bar of `app`, rendered through the real host on the app's own context.
fn bar(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h =
        Harness::builder().with_size(vec2(1600.0, 200.0)).with_context(app.ctx().clone()).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

fn new_app(width: u32, height: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": width, "height": height})).unwrap();
    app
}

/// Lassos a square and drops it with Transform On Drop on, which raises the CAM box.
fn raise_cam_box(app: &mut PhotocraftApp) {
    app.ui.tool = Tool::ContentAwareMove;
    app.ui.tool_options.cam_transform_on_drop = true;
    let m = egui::Modifiers::NONE;
    let drag = |app: &mut PhotocraftApp, path: &[[f64; 2]]| {
        let ([x, y], rest) = (path[0], &path[1..]);
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
        for &[x, y] in rest {
            tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
        }
        let [x, y] = path[path.len() - 1];
        tool_event(app, ToolEvent::Up { x, y }, m);
    };
    drag(app, &[[6.0, 14.0], [38.0, 14.0], [38.0, 46.0], [6.0, 46.0]]);
    drag(app, &[[22.0, 30.0], [50.0, 30.0], [72.0, 30.0]]);
}

#[test]
fn a_live_transform_box_disables_the_content_aware_move_cells() {
    let mut app = new_app(100, 60);
    raise_cam_box(&mut app);
    assert!(crate::cam_transform::active(&app), "a_live_transform_box_disables_the_content_aware_move_cells: the drop raises the box: {}", app.ui.status);
    assert!(pending(&app, Tool::ContentAwareMove).is_some(), "a_live_transform_box_disables_the_content_aware_move_cells: the bar has a pending box");
    let steps = app.session.active().unwrap().history.past_len();

    let mut h = bar(app);
    for (role, label) in [
        (Role::ComboBox, "Mode"),
        (Role::SpinButton, "Structure"),
        (Role::SpinButton, "Color"),
        (Role::CheckBox, "Sample All Layers"),
        (Role::CheckBox, "Transform On Drop"),
    ] {
        let node = h
            .query_all_by_role(role)
            .find(|n| n.accesskit_node().label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("a_live_transform_box_disables_the_content_aware_move_cells: the {label} cell"));
        assert!(node.accesskit_node().is_disabled(), "a_live_transform_box_disables_the_content_aware_move_cells: {label} is disabled while the box is up");
    }
    assert!(h.state().framework.dialogs.open_dialogs().is_empty(), "a_live_transform_box_disables_the_content_aware_move_cells: no dialog opens");

    answer(h.state_mut(), Tool::ContentAwareMove, false, true);
    h.run_steps(4);
    assert!(!crate::cam_transform::active(h.state()), "a_live_transform_box_disables_the_content_aware_move_cells: cancel drops the box");
    assert_eq!(
        h.state().session.active().unwrap().history.past_len(),
        steps,
        "a_live_transform_box_disables_the_content_aware_move_cells: cancel adds no history step"
    );
}

#[test]
fn showing_a_bar_keeps_a_stored_fraction() {
    let mut app = new_app(80, 80);
    app.ui.tool_options.strength = 33.7;
    app.ui.tool = Tool::Blur;
    let mut h = bar(app);
    for tool in [Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::Blur] {
        h.state_mut().ui.tool = tool;
        h.run_steps(5);
        assert_eq!(h.state().ui.tool_options.strength, 33.7, "showing_a_bar_keeps_a_stored_fraction: showing the {tool:?} bar leaves the fraction");
    }
}
