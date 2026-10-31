use super::*;
use crate::direction::Dir;
use egui_kittest::kittest::Queryable;

/// The Color Range body drawn in `dir`.
fn harness(dir: Dir) -> egui_kittest::Harness<'static, (PhotocraftApp, Map<String, Value>)> {
    let _dir = crate::direction::direction_scope(dir);
    let mut fields = Map::new();
    fields.insert("__colorRange".into(), json!(true));
    fields.insert("select".into(), json!("sampledColors"));
    fields.insert("fuzziness".into(), json!(40.0));
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(560.0, 620.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(|ui, s: &mut (PhotocraftApp, Map<String, Value>)| body(&mut s.0, ui, &mut s.1), (app, fields));
    h.run_steps(3);
    h
}

/// The select row mirrors: the caption starts the row and the dropdown ends it at the reading
/// end — left-to-right in English, right-to-left in Arabic direction.
#[test]
fn the_select_row_mirrors() {
    let ltr = harness(Dir::Ltr);
    let rtl = harness(Dir::Rtl);

    let ltr_caption = ltr.get_by_label("Select:").rect();
    let ltr_dropdown = ltr.get_by_label("Select").rect();
    assert!(ltr_dropdown.left() > ltr_caption.right(), "LTR: the dropdown ends the row: {ltr_caption:?} {ltr_dropdown:?}");

    let rtl_caption = rtl.get_by_label("Select:").rect();
    let rtl_dropdown = rtl.get_by_label("Select").rect();
    assert!(rtl_dropdown.right() < rtl_caption.left(), "RTL: the row mirrors: {rtl_caption:?} {rtl_dropdown:?}");
}

/// The rows hug the reading start of the column: the left in a left-to-right UI, the right in a
/// mirrored one.
#[test]
fn the_rows_hug_the_reading_start() {
    let ltr = harness(Dir::Ltr);
    let rtl = harness(Dir::Rtl);

    let ltr_select = ltr.get_by_label("Select:").rect();
    let ltr_preview = ltr.get_by_label("Selection Preview:").rect();
    assert!((ltr_select.left() - ltr_preview.left()).abs() < 1.0, "LTR: the rows start at the left: {ltr_select:?} {ltr_preview:?}");

    let rtl_select = rtl.get_by_label("Select:").rect();
    let rtl_preview = rtl.get_by_label("Selection Preview:").rect();
    assert!((rtl_select.right() - rtl_preview.right()).abs() < 1.0, "RTL: the rows end at the right: {rtl_select:?} {rtl_preview:?}");
}

/// The Selection / Image radio row mirrors: Selection first at the reading start, Image after it.
#[test]
fn the_view_radios_mirror() {
    let ltr = harness(Dir::Ltr);
    let rtl = harness(Dir::Rtl);

    let ltr_selection = ltr.get_by_label("Selection").rect();
    let ltr_image = ltr.get_by_label("Image").rect();
    assert!(ltr_selection.right() < ltr_image.left(), "LTR: Selection precedes Image: {ltr_selection:?} {ltr_image:?}");

    let rtl_selection = rtl.get_by_label("Selection").rect();
    let rtl_image = rtl.get_by_label("Image").rect();
    assert!(rtl_selection.left() > rtl_image.right(), "RTL: the radios mirror: {rtl_selection:?} {rtl_image:?}");
}

/// The badge's anchor mirrors about the button's centre and stays on its bottom edge.
#[test]
fn the_badge_anchor_mirrors() {
    let r = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(26.0, 26.0));
    let (ltr, rtl) = (badge_anchor(r, false), badge_anchor(r, true));
    assert!((ltr.x + rtl.x - (r.left() + r.right())).abs() < 0.01, "the badge mirrors: {ltr:?} / {rtl:?}");
    assert!((ltr.y - rtl.y).abs() < 0.01, "the badge stays on the bottom edge: {ltr:?} / {rtl:?}");
}
