//! The Stroke dialog's right-to-left twins: only the direction is forced — the data and the
//! behaviour stay the ones the left-to-right tests pin.

use egui_kittest::kittest::Queryable;

use super::*;

/// The window the dialog is centred in: its width is what a mirrored row is checked against.
const WINDOW: f32 = 1280.0;

/// The harness pattern of the left-to-right tests: the app over one document.
fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(WINDOW, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width": 200, "height": 120})).unwrap();
        app
    });
    h.run_steps(6);
    h
}

/// The open Stroke dialog's frame, Width field and Location dropdown, drawn under `dir`.
fn rows(dir: crate::text::direction::Dir) -> (egui::Rect, egui::Rect, egui::Rect) {
    let _dir = crate::text::direction::direction_scope(dir);
    let mut h = harness();
    // The app flips egui's own direction flag with the language (`i18n::sync_context`); this
    // harness draws the dialog without the app's frame, so it sets the flag itself. egui's
    // `Ui::indent` indents from the right only under that flag.
    crate::theme::set_direction(&h.ctx, dir == crate::text::direction::Dir::Rtl);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, serde_json::json!({})).unwrap();
    h.run_steps(3);
    let id = h.state().ui.dialogs.last().unwrap().id;
    let frame = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog");
    (h.get_by_label(tl!("Width:")).rect(), h.get_by_value(tl!("Center")).rect(), frame)
}

/// The rows read from the right: the dialog is centred in the window, so a mirrored row's two
/// rects add up to the window's width, and each row group is packed at the reading start — the
/// field follows the 112 pt caption column in from the frame's edge.
#[test]
fn rtl_the_rows_read_from_the_right() {
    let (ltr_field, ltr_drop, ltr_frame) = rows(crate::text::direction::Dir::Ltr);
    let (rtl_field, rtl_drop, rtl_frame) = rows(crate::text::direction::Dir::Rtl);
    assert!((ltr_field.center().x + rtl_field.center().x - WINDOW).abs() < 2.0, "the Width field mirrors: {ltr_field:?} / {rtl_field:?}");
    assert!((ltr_drop.center().x + rtl_drop.center().x - WINDOW).abs() < 2.0, "the Location dropdown mirrors: {ltr_drop:?} / {rtl_drop:?}");
    // The caption column sits between the frame's reading start and its control: the Width
    // field starts one 112 pt caption plus padding in from the frame's left edge in a
    // left-to-right dialog and from its right edge in a mirrored one.
    let ltr_from_start = ltr_field.left() - ltr_frame.left();
    let rtl_from_start = rtl_frame.right() - rtl_field.right();
    assert!((120.0..140.0).contains(&ltr_from_start), "the caption column precedes the field from the left: {ltr_from_start} in {ltr_frame:?}");
    assert!((120.0..140.0).contains(&rtl_from_start), "the caption column precedes the field from the right: {rtl_from_start} in {rtl_frame:?}");
}

/// The caption in its fixed column hugs the field it names: the box's right edge in a
/// left-to-right row, its left edge in a mirrored one.
#[test]
fn rtl_the_caption_hugs_its_field() {
    let r = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(112.0, 22.0));
    let (ltr_pos, ltr_align) = caption_anchor(r, false);
    assert!(matches!(ltr_align, Align2::RIGHT_CENTER), "the caption is right-aligned: {ltr_align:?}");
    assert!((ltr_pos - (r.right() - 6.0)).abs() < 0.01, "the caption sits at the right of its box: {ltr_pos:?}");
    let (rtl_pos, rtl_align) = caption_anchor(r, true);
    assert!(matches!(rtl_align, Align2::LEFT_CENTER), "the caption is left-aligned: {rtl_align:?}");
    assert!((rtl_pos - (r.left() + 6.0)).abs() < 0.01, "the caption sits at the left of its box: {rtl_pos:?}");
}
