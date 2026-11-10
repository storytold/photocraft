//! #12731: the Add Anchor Point options bar holds the preset picker and "No options for the Add
//! Anchor Point Tool." centred in the rest of the bar, as in Photoshop; the Delete Anchor Point bar
//! keeps its hint.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const NO_OPTIONS: &str = "No options for the Add Anchor Point Tool.";
const WIDTH: f32 = 1280.0;

fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = Harness::builder().with_size(vec2(WIDTH, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(6);
    h.state_mut().ui.tool = tool;
    h.run_steps(4);
    h
}

#[test]
fn add_anchor_point_bar_says_it_has_no_options() {
    let h = harness(Tool::AddAnchorPoint);
    let text = h.get_by_label(NO_OPTIONS);
    assert!(h.query_by_label_contains("Click a segment to add an anchor point").is_none(), "the Alt hint is gone");
    // Centred between the preset picker and the bar's right edge; the tolerance covers the bar's padding.
    // The toolbox has a button of the same name; the bar's picker is the one on the text's row.
    let rect = text.rect();
    let picker = h.query_all_by_label(Tool::AddAnchorPoint.label()).map(|n| n.rect()).find(|r| r.y_range().contains(rect.center().y)).unwrap();
    let centre = (picker.max.x + WIDTH) / 2.0;
    let mid = (rect.min.x + rect.max.x) / 2.0;
    assert!((mid - centre).abs() < 20.0, "text centre {mid} vs remaining bar centre {centre}");
}

#[test]
fn delete_anchor_point_bar_keeps_its_hint() {
    let h = harness(Tool::DeleteAnchorPoint);
    let hint = crate::i18n::fmt("Click an anchor point to delete it · {alt}-click a segment to add one", &[("alt", &crate::shortcuts::pretty("Alt"))]);
    h.get_by_label(&hint);
    assert!(h.query_by_label(NO_OPTIONS).is_none());
}
