//! #15586: with several shape layers selected, the options bar's Fill and Stroke edit all of them, not just the
//! active one, as one History step. A locked one is left alone.

use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Three stroked rectangles, the lowest locked, all selected, with the Path Selection tool's options bar.
fn bar_with_three_selected() -> (Harness<'static, PhotocraftApp>, [LayerId; 3]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ids = [10, 70, 130].map(|x| {
        let r = app.run("shape.create", json!({"kind": "rect", "rect": [x, 10, 50, 50], "fill": "#ff0000", "stroke": {"width": 2, "color": "#00ff00"}}));
        LayerId(r.unwrap()["layer"].as_u64().unwrap())
    });
    app.run("layer.setProps", json!({"layer": ids[0].0, "locks": {"all": true}})).unwrap();
    app.run("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        app.run("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    (h, ids)
}

fn stroke_widths(h: &Harness<'static, PhotocraftApp>, ids: &[LayerId]) -> Vec<f32> {
    let st = h.state().session.active().unwrap();
    ids.iter()
        .map(|id| match &st.doc.layer(*id).unwrap().content {
            LayerContent::Shape(sh) => sh.stroke.as_ref().map_or(0.0, |s| s.width),
            _ => panic!("a shape layer"),
        })
        .collect()
}

#[test]
fn the_bars_stroke_width_edits_every_selected_unlocked_shape_in_one_step() {
    let (mut h, ids) = bar_with_three_selected();
    assert_eq!(h.state().session.active().unwrap().selected_layers().len(), 3);
    let before = h.state().session.active().unwrap().history.past_len();
    h.get_by_label("Stroke width").click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Stroke width").type_text("9");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(stroke_widths(&h, &ids), vec![2.0, 9.0, 9.0], "both unlocked shapes take the width; the locked one keeps its own");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1, "one History step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(stroke_widths(&h, &ids), vec![2.0, 2.0, 2.0], "one Undo restores both");
}
