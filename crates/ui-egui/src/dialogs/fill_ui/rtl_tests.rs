//! The Fill dialog's right-to-left twins: the fields read from the right while the data and the
//! behaviour stay the ones the left-to-right tests pin — only the direction is forced.

use egui::Align2;
use egui_kittest::kittest::Queryable;

use super::*;

/// The window the dialog is centred in: its width is what a mirrored row is checked against.
const WINDOW: f32 = 1280.0;

/// The harness of the left-to-right tests (`tests::harness`): the app over one document.
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

/// The open Fill dialog's Contents dropdown and Blending caption, drawn under `dir`.
fn rows(dir: crate::text::direction::Dir) -> (egui::Rect, egui::Rect) {
    let _dir = crate::text::direction::direction_scope(dir);
    let mut h = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, serde_json::json!({})).unwrap();
    h.run_steps(3);
    (h.get_by_value("Foreground Color").rect(), h.get_by_label("Blending").rect())
}

/// The rows read from the right: the dialog is centred in the window, so a mirrored row's two
/// rects add up to the window's width, and each moves to the other side of the centre.
#[test]
fn rtl_the_rows_read_from_the_right() {
    let (ltr_drop, ltr_cap) = rows(crate::text::direction::Dir::Ltr);
    let (rtl_drop, rtl_cap) = rows(crate::text::direction::Dir::Rtl);
    assert!((ltr_drop.center().x + rtl_drop.center().x - WINDOW).abs() < 2.0, "the Contents dropdown mirrors: {ltr_drop:?} / {rtl_drop:?}");
    assert!((ltr_cap.center().x + rtl_cap.center().x - WINDOW).abs() < 2.0, "the Blending caption mirrors: {ltr_cap:?} / {rtl_cap:?}");
    // The row hugs the reading-start edge of the dialog, so mirroring carries the dropdown
    // across the window's centre line: its RTL centre is right of its LTR one.
    assert!(rtl_drop.center().x > ltr_drop.center().x, "the dropdown mirrors: {ltr_drop:?} / {rtl_drop:?}");
    // The section caption starts at the reading start: the left edge in a left-to-right dialog,
    // the right edge in a mirrored one.
    assert!(ltr_cap.left() < WINDOW / 2.0 && rtl_cap.right() > WINDOW / 2.0, "the caption reads from the right: {ltr_cap:?} / {rtl_cap:?}");
}

/// The caption in its fixed column hugs the field it names: the box's right edge in a
/// left-to-right row, its left edge in a mirrored one.
#[test]
fn rtl_the_caption_hugs_its_field() {
    let r = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(112.0, 22.0));
    let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
    let (ltr_pos, ltr_align) = crate::direction::end_text_anchor(r, 6.0);
    assert!(matches!(ltr_align, Align2::RIGHT_CENTER), "the caption is right-aligned: {ltr_align:?}");
    assert!((ltr_pos.x - (r.right() - 6.0)).abs() < 0.01, "the caption sits at the right of its box: {ltr_pos:?}");
    drop(_ltr);
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let (rtl_pos, rtl_align) = crate::direction::end_text_anchor(r, 6.0);
    assert!(matches!(rtl_align, Align2::LEFT_CENTER), "the caption is left-aligned: {rtl_align:?}");
    assert!((rtl_pos.x - (r.left() + 6.0)).abs() < 0.01, "the caption sits at the left of its box: {rtl_pos:?}");
}

/// Rendered twin: in a real `body_layout` row the `label` box sits at the reading start, flush
/// against the field it names — left of the field left-to-right, right of it right-to-left.
#[test]
fn rtl_the_rendered_caption_box_hugs_the_field() {
    let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(WINDOW, 800.0));
    let row = |dir: crate::text::direction::Dir| {
        let _dir = crate::text::direction::direction_scope(dir);
        let ctx = egui::Context::default();
        let mut rects = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(area), ..Default::default() }, |ui| {
            ui.with_layout(crate::text::direction::body_layout(), |ui| {
                ui.horizontal(|ui| {
                    let cap = label(ui, "Contents:");
                    let (field, _) = ui.allocate_exact_size(egui::vec2(170.0, 22.0), egui::Sense::hover());
                    rects = (cap, field);
                });
            });
        });
        out.textures_delta.clear();
        rects
    };
    let (ltr_cap, ltr_field) = row(crate::text::direction::Dir::Ltr);
    let (rtl_cap, rtl_field) = row(crate::text::direction::Dir::Rtl);
    assert!(ltr_cap.center().x < ltr_field.center().x, "LTR: the caption leads: {ltr_cap:?} / {ltr_field:?}");
    assert!((ltr_field.left() - ltr_cap.right()).abs() <= 16.0, "LTR: the caption box is flush with the field: {ltr_cap:?} / {ltr_field:?}");
    assert!(rtl_cap.center().x > rtl_field.center().x, "RTL: the caption leads from the right: {rtl_cap:?} / {rtl_field:?}");
    assert!((rtl_cap.left() - rtl_field.right()).abs() <= 16.0, "RTL: the caption box is flush with the field: {rtl_cap:?} / {rtl_field:?}");
}
