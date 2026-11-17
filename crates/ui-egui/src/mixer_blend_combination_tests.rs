//! The Mixer Brush bar's Useful mixer brush combinations list (#11946): a pick sets Wet, Load and
//! Mix in one `tools.setBrush` change and leaves Flow.

use craft::draw::ui::MIXER_COMBINATIONS;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const LIST: &str = "Useful mixer brush combinations";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"mixer": {"flow": 0.42}}})).unwrap();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

fn has_fonts(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

/// An entry of the open list (not the Wet field, which shares the "Wet" label).
fn entry_node<'a>(h: &'a Harness<'static, PhotocraftApp>, entry: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(entry).rfind(|n| n.accesskit_node().role() != egui::accesskit::Role::SpinButton).unwrap_or_else(|| panic!("no entry {entry}"))
}

fn set_brush_count(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.journal.iter().filter(|(id, _)| id == "tools.setBrush").count()
}

#[test]
fn each_entry_sets_wet_load_and_mix_in_one_change_and_keeps_flow() {
    let mut h = harness();
    if !has_fonts(&h) {
        return;
    }
    let (index, name) = (6, "Moist, Heavy Mix");
    let (wet, load, mix) = MIXER_COMBINATIONS[index].expect("a combination with values");
    let before = set_brush_count(&h);
    h.get_by_label(LIST).click();
    h.run_steps(3);
    entry_node(&h, name).click();
    h.run_steps(3);
    let m = &h.state().session.tools.brush.mixer;
    assert_eq!((m.wet, m.load, m.mix), (wet as f32, load as f32, mix as f32), "the mixer stores f32");
    assert!((m.flow - 0.42).abs() < 1e-6, "Flow stays");
    assert_eq!(set_brush_count(&h), before + 1, "one tools.setBrush change");
    assert_eq!(h.state().ui.tool_options.mixer_combination, index);
}
