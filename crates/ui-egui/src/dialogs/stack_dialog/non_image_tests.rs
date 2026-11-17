//! #14005: Browse… in Files mode leaves files PhotoCraft can't open as images out of the list,
//! silently, by Folder mode's rule; the images picked with them are added in order, without
//! duplicates.
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::file_dialog::FileDialogAnswer;

fn harness(picks: Vec<Vec<&str>>) -> Harness<'static, PhotocraftApp> {
    let answers = picks.into_iter().map(|p| Some(FileDialogAnswer::Paths(p.into_iter().map(|n| format!("/stack-src/{n}")).collect()))).collect();
    let (show, _) = crate::file_dialog::fake(answers);
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() })
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(3);
    h
}

fn browse(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label("Browse…").click();
    h.run_steps(4);
}

fn listed(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let f = &h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).unwrap().fields;
    files(f).iter().map(|p| file_name(p).to_string()).collect()
}

fn alerts(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).count()
}

#[test]
fn a_text_file_picked_with_an_image_is_dropped_silently() {
    let mut h = harness(vec![vec!["readme.txt", "a_base.png"]]);
    browse(&mut h);
    assert_eq!(listed(&h), ["a_base.png"]);
    assert_eq!(alerts(&h), 0, "no message");
    assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
}

#[test]
fn the_images_keep_their_order_and_duplicates_are_skipped() {
    let mut h = harness(vec![vec!["b_shift.png", "notes.txt", "e_16.tif", "clip.mov"], vec!["a_base.png", "b_shift.png", "data.csv"]]);
    browse(&mut h);
    assert_eq!(listed(&h), ["b_shift.png", "e_16.tif"]);
    browse(&mut h);
    assert_eq!(listed(&h), ["b_shift.png", "e_16.tif", "a_base.png"]);
}

#[test]
fn a_pick_without_any_image_keeps_todays_behaviour() {
    let mut h = harness(vec![vec!["readme.txt"]]);
    browse(&mut h);
    assert_eq!(listed(&h), ["readme.txt"], "not measured: kept as before");
}
