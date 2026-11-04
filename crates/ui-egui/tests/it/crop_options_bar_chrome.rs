//! #10888: the Crop options bar's chrome. The Straighten button carries the reference label and
//! the new spirit-level glyph; the rest of the item's checks land with their controls.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::crop_straighten::{self, Straighten};

fn harness() -> Harness<'static, Straighten> {
    let h = Harness::builder()
        .with_size(egui::vec2(260.0, 60.0))
        .with_max_steps(64)
        .build_ui_state(|ui, s: &mut Straighten| crop_straighten::options_button(s, ui), Straighten::default());
    // The SVG glyph needs the app's image loaders; the label and the click do not.
    photocraft_ui_egui::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

#[test]
fn the_straighten_button_shows_its_label_and_arms() {
    let mut h = harness();
    h.run();
    assert!(h.query_by_label("Straighten").is_some(), "the button carries the reference label");
    h.get_by_label("Straighten").click();
    h.run();
    assert!(h.state().armed, "the button arms straighten");
    h.get_by_label("Straighten").click();
    h.run();
    assert!(!h.state().armed, "clicking again disarms it");
}

#[test]
fn the_straighten_glyph_is_embedded() {
    assert!(photocraft_ui_egui::icons::exists("spirit-level"));
}
