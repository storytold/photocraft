//! #16338: Background Eraser's "Limits:", Freeform Pen's "Curve Fit:" and the Note tool's "Author:" and "Color:"
//! are drawn in front of their controls, not only given to them as accessible names.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The options bar of `tool` on a new document, rendered through the real host.
fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h =
        Harness::builder().with_size(vec2(1600.0, 200.0)).with_context(app.ctx().clone()).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

#[test]
fn bare_bar_controls_show_their_labels() {
    let cases: [(Tool, &[&str]); 3] = [(Tool::BackgroundEraser, &["Limits:"]), (Tool::FreeformPen, &["Curve Fit:"]), (Tool::Note, &["Author", "Color"])];
    for (tool, labels) in cases {
        let h = bar(tool);
        let shown: Vec<String> = h.query_all_by_role(Role::Label).filter_map(|n| n.accesskit_node().label()).collect();
        for label in labels {
            let node = h.query_all_by_role(Role::Label).find(|n| n.accesskit_node().label().as_deref() == Some(label));
            let node = node.unwrap_or_else(|| panic!("{tool:?}: label {label:?} is drawn; labels shown: {shown:?}"));
            let r = node.rect();
            assert!(r.width() > 0.0 && r.height() > 0.0 && r.top() >= 0.0, "{tool:?}: {label:?} has a visible rect: {r:?}");
        }
    }
}
