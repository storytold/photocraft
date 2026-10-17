//! #5532: the Layers panel's Blend Mode list draws a line between Photoshop's six groups of modes:
//! Normal, Dissolve | Darken … Darker Color | Lighten … Lighter Color | Overlay … Hard Mix |
//! Difference … Divide | Hue … Luminosity.

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;

/// The app with a new document and a pixel layer above the Background, active.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app
    });
    h.run_steps(8);
    h
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn the_layers_blend_mode_list_has_a_line_between_each_group() {
    let mut h = harness();
    // The Layers panel's dropdown: the lowest one (the options bar and the title bar have some too).
    let button = h.query_all_by_role(Role::ComboBox).map(|n| n.rect()).max_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    click(&mut h, button.center());
    let list = h.ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id))).unwrap();
    // Each mode's row in the open list.
    let rows: Vec<Rect> = BlendMode::LAYER_MODES
        .iter()
        .map(|m| h.get_all_by_label(m.label()).map(|n| n.rect()).rfind(|r| list.contains_rect(*r)).unwrap_or_else(|| panic!("{m:?} is in the list")))
        .collect();
    let gaps: Vec<f32> = rows.windows(2).map(|w| w[1].top() - w[0].bottom()).collect();
    let row_gap = gaps.iter().copied().fold(f32::INFINITY, f32::min);
    let lines: Vec<(BlendMode, BlendMode)> =
        gaps.iter().enumerate().filter(|(_, g)| **g > row_gap + 2.0).map(|(i, _)| (BlendMode::LAYER_MODES[i], BlendMode::LAYER_MODES[i + 1])).collect();
    assert_eq!(
        lines,
        [
            (BlendMode::Dissolve, BlendMode::Darken),
            (BlendMode::DarkerColor, BlendMode::Lighten),
            (BlendMode::LighterColor, BlendMode::Overlay),
            (BlendMode::HardMix, BlendMode::Difference),
            (BlendMode::Divide, BlendMode::Hue),
        ],
        "row gaps {gaps:?}"
    );
}
