//! #14006: Load Files into Stack's window is titled "Load Layers" (the menu item keeps its name)
//! and shows the hint "Choose two or more files to load into an image stack." above Use:; the
//! Statistics dialog keeps "Image Statistics" and no hint.
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

const HINT: &str = "Choose two or more files to load into an image stack.";

fn harness() -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    })
}

fn open(h: &mut Harness<'_, PhotocraftApp>, command: &str) -> u64 {
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap()["dialog"].as_u64();
    h.run_steps(4);
    id.unwrap_or_else(|| h.state().ui.dialogs.iter().rev().find(|d| owns(&d.fields)).unwrap().id)
}

fn title(h: &Harness<'_, PhotocraftApp>, id: u64) -> String {
    crate::dialogs::title(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap())
}

#[test]
fn the_window_is_load_layers_with_the_hint_above_use() {
    let mut h = harness();
    let id = open(&mut h, COMMAND);
    assert_eq!(title(&h, id), "Load Layers");
    assert!(h.query_by_label("Load Layers").is_some(), "the title is drawn");
    let hint = h.get_by_label(HINT);
    let source = h.get_by_label("Source Files");
    let use_row = h.get_by_label("Use:");
    assert!(source.rect().bottom() <= hint.rect().top() && hint.rect().bottom() <= use_row.rect().top(), "between the heading and Use:");
    // The menu item keeps its own name.
    assert_eq!(photocraft_engine::commands::find(COMMAND).unwrap().label, "Load Files into Stack…");
}

#[test]
fn statistics_keeps_its_title_and_no_hint() {
    let mut h = harness();
    let id = open(&mut h, STATISTICS);
    assert_eq!(title(&h, id), "Image Statistics");
    assert!(h.query_by_label(HINT).is_none());
}
