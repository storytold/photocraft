//! #14004: Load Files into Stack's file list is multi-select: a click selects one row, Ctrl+click
//! (Cmd+click on the Mac) adds a row or takes it out again, and Remove removes every selected row
//! at once, keeping the others' order, then clears the selection.
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

const LIST: [&str; 4] = ["e_16.tif", "c_wide.tif", "b_shift.png", "a_base.png"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(3);
    let paths: Vec<String> = LIST.iter().map(|n| format!("/stack-src/{n}")).collect();
    fields(&mut h).insert("files".into(), json!(paths));
    h.run_steps(3);
    h
}

fn fields<'a>(h: &'a mut Harness<'_, PhotocraftApp>) -> &'a mut Map<String, Value> {
    &mut h.state_mut().ui.dialogs.iter_mut().find(|d| owns(&d.fields)).unwrap().fields
}

fn listed(h: &mut Harness<'_, PhotocraftApp>) -> Vec<String> {
    files(fields(h)).iter().map(|p| file_name(p).to_string()).collect()
}

fn rows(h: &mut Harness<'_, PhotocraftApp>) -> Vec<usize> {
    selected(fields(h))
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str, modifiers: Modifiers) {
    h.get_by_label(label).click_modifiers(modifiers);
    h.run_steps(3);
}

fn remove_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    !h.get_by_label("Remove").accesskit_node().is_disabled()
}

#[test]
fn ctrl_click_selects_several_rows_and_remove_removes_them_all() {
    let mut h = harness();
    assert!(!remove_enabled(&h), "greyed with no selection");
    click(&mut h, "c_wide.tif", Modifiers::NONE);
    click(&mut h, "b_shift.png", Modifiers::COMMAND);
    assert_eq!(rows(&mut h), [1, 2]);
    assert!(remove_enabled(&h));
    click(&mut h, "Remove", Modifiers::NONE);
    assert_eq!(listed(&mut h), ["e_16.tif", "a_base.png"], "the measured example");
    assert!(rows(&mut h).is_empty(), "nothing selected afterwards");
    assert!(!remove_enabled(&h), "greyed again");
}

#[test]
fn a_plain_click_selects_one_row_and_ctrl_click_takes_a_row_out() {
    let mut h = harness();
    click(&mut h, "e_16.tif", Modifiers::NONE);
    click(&mut h, "a_base.png", Modifiers::COMMAND);
    click(&mut h, "c_wide.tif", Modifiers::COMMAND);
    assert_eq!(rows(&mut h), [0, 1, 3]);
    click(&mut h, "a_base.png", Modifiers::COMMAND);
    assert_eq!(rows(&mut h), [0, 1], "Ctrl+click on a selected row takes it out");
    click(&mut h, "b_shift.png", Modifiers::NONE);
    assert_eq!(rows(&mut h), [2], "a plain click selects that row alone");
    click(&mut h, "b_shift.png", Modifiers::COMMAND);
    assert!(rows(&mut h).is_empty());
    assert!(!remove_enabled(&h));
}

#[test]
fn sort_by_name_still_works_after_a_multi_row_selection() {
    let mut h = harness();
    click(&mut h, "e_16.tif", Modifiers::NONE);
    click(&mut h, "a_base.png", Modifiers::COMMAND);
    click(&mut h, "Sort By Name", Modifiers::NONE);
    assert_eq!(listed(&mut h), ["a_base.png", "b_shift.png", "c_wide.tif", "e_16.tif"]);
    assert!(rows(&mut h).is_empty(), "the list changed, so the selection is cleared");
}
