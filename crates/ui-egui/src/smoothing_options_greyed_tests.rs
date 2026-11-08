//! #11939: at Smoothing 0 % the gear's four Smoothing Options are greyed and can't be toggled, but
//! keep their values; above 0 % they work again. For the Brush and the Eraser, the tools with this
//! gear.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const OPTIONS: [&str; 4] = ["Pulled String Mode", "Stroke Catch-up", "Catch-up on Stroke End", "Adjust for Zoom"];

fn bar(tool: Tool, smoothing: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.session.tools.brush.smoothing.amount = smoothing;
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.get_by_label("Set additional smoothing options").click();
    h.run_steps(3);
    h
}

fn checkbox<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::CheckBox).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("{label}"))
}

fn values(h: &Harness<'static, PhotocraftApp>) -> [bool; 4] {
    let s = &h.state().session.tools.brush.smoothing;
    [s.pulled_string, s.catch_up, s.catch_up_on_end, s.adjust_for_zoom]
}

#[test]
fn at_zero_the_four_are_greyed_and_keep_their_values() {
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut h = bar(tool, 0.0);
        let before = values(&h);
        for label in OPTIONS {
            assert!(checkbox(&h, label).accesskit_node().is_disabled(), "{tool:?}: {label} greyed at 0 %");
        }
        for label in OPTIONS {
            checkbox(&h, label).click();
            h.run_steps(2);
        }
        assert_eq!(values(&h), before, "{tool:?}: clicks change nothing at 0 %");
    }
}

#[test]
fn above_zero_they_work_again() {
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut h = bar(tool, 0.1);
        for label in OPTIONS {
            assert!(!checkbox(&h, label).accesskit_node().is_disabled(), "{tool:?}: {label} enabled");
        }
        let before = values(&h);
        checkbox(&h, "Pulled String Mode").click();
        h.run_steps(2);
        assert_ne!(values(&h)[0], before[0], "{tool:?}: Pulled String Mode toggles");
    }
}
