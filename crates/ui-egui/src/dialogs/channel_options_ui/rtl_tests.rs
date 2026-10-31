//! RTL twins for the Channel Options body: the blocks read from the right and the rows inside
//! them mirror, without the dialog changing width.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

/// A harness with a 16×16 document and one saved selection (alpha channel 0), opening a dialog
/// via `open_dialog` and drawing it in the given direction. The direction scope is held for the
/// whole build: the thread-local direction must be alive while the dialog renders.
fn dialog_in(dir: crate::text::direction::Dir, open_dialog: impl FnOnce(&mut PhotocraftApp)) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let _dir = crate::text::direction::direction_scope(dir);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    app.run("select.saveSelection", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    // The app flips egui's own direction flag with the language (`i18n::sync_context`); this
    // harness draws the dialog without the app's frame, so it sets the flag itself. egui's
    // `Ui::indent` indents from the right only under that flag.
    crate::theme::set_direction(&h.ctx, dir == crate::text::direction::Dir::Rtl);
    open_dialog(h.state_mut());
    h.run_steps(4);
    h
}

/// The Channel Options dialog on the first alpha channel, rendered in the given direction.
fn channel_options(dir: crate::text::direction::Dir) -> egui_kittest::Harness<'static, PhotocraftApp> {
    dialog_in(dir, |app| {
        open(app, &json!(0)).unwrap();
    })
}

/// The dialog's own frame, as the host sized it.
fn frame(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> egui::Rect {
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog")
}

/// The body reads from the left in a left-to-right UI and from the right in a right-to-left one:
/// the section label starts at the reading start, the radios are indented from it and the name
/// field follows the caption column towards the reading end.
#[test]
fn rtl_the_channel_options_body_mirrors() {
    let ltr = channel_options(crate::text::direction::Dir::Ltr);
    let ltr_frame = frame(&ltr);
    let ltr_section = ltr.get_by_label("Color Indicates:").rect();
    let ltr_radio = ltr.get_by_label("Masked Areas").rect();
    let ltr_name = ltr.get_by_label("Name").rect();
    assert!((ltr_section.left() - ltr_frame.left()).abs() < 40.0, "the section label reads from the left: {ltr_section:?} in {ltr_frame:?}");
    assert!(
        (ltr_radio.left() - ltr_section.left()).abs() < 40.0 && ltr_radio.left() > ltr_section.left(),
        "the radios are indented from the left: {ltr_radio:?} vs {ltr_section:?}"
    );
    assert!(ltr_name.left() - ltr_radio.left() > 70.0, "the name field follows the caption column: {ltr_name:?} vs {ltr_radio:?}");

    let rtl = channel_options(crate::text::direction::Dir::Rtl);
    let rtl_frame = frame(&rtl);
    let rtl_section = rtl.get_by_label("Color Indicates:").rect();
    let rtl_radio = rtl.get_by_label("Masked Areas").rect();
    let rtl_name = rtl.get_by_label("Name").rect();
    assert!(rtl_frame.right() - rtl_section.right() < 40.0, "the section label reads from the right: {rtl_section:?} in {rtl_frame:?}");
    assert!(rtl_radio.right() < rtl_section.right(), "the radios are indented from the right: {rtl_radio:?} vs {rtl_section:?}");
    assert!(rtl_radio.right() - rtl_name.right() > 70.0, "the name field follows the caption column: {rtl_name:?} vs {rtl_radio:?}");
}

/// The mirrored body keeps the dialog as wide as its left-to-right twin.
#[test]
fn rtl_the_channel_options_dialog_keeps_its_width() {
    let width = |dir| frame(&channel_options(dir)).width();
    let ltr = width(crate::text::direction::Dir::Ltr);
    let rtl = width(crate::text::direction::Dir::Rtl);
    assert!((ltr - rtl).abs() < 1.0, "LTR {ltr} vs RTL {rtl}");
}

/// RTL twin for the Layer Mask Display Options body: the colour row reads from the right and
/// the dialog keeps its left-to-right width.
#[test]
fn rtl_the_layer_mask_options_body_mirrors() {
    let mask_options = |dir| {
        dialog_in(dir, |app| {
            open(app, &json!("mask")).unwrap();
        })
    };
    let ltr = mask_options(crate::text::direction::Dir::Ltr);
    let ltr_frame = frame(&ltr);
    let ltr_color = ltr.get_by_label("Overlay Color").rect();
    let ltr_pct = ltr.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Opacity:").rect();
    assert!(ltr_color.left() - ltr_frame.left() > 40.0, "the caption column leads on the left: {ltr_color:?} in {ltr_frame:?}");
    assert!(ltr_pct.left() > ltr_color.right(), "the opacity field follows the swatch: {ltr_pct:?} vs {ltr_color:?}");

    let rtl = mask_options(crate::text::direction::Dir::Rtl);
    let rtl_frame = frame(&rtl);
    let rtl_color = rtl.get_by_label("Overlay Color").rect();
    let rtl_pct = rtl.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Opacity:").rect();
    let ltr_inset = ltr_color.left() - ltr_frame.left();
    let rtl_inset = rtl_frame.right() - rtl_color.right();
    assert!((ltr_inset - rtl_inset).abs() < 4.0, "the colour row sits at the reading-start edge in both: LTR inset {ltr_inset} vs RTL inset {rtl_inset}");
    assert!(rtl_color.right() > rtl_pct.right(), "the opacity field follows the swatch towards the reading end: {rtl_pct:?} vs {rtl_color:?}");
    assert!((ltr_frame.width() - rtl_frame.width()).abs() < 1.0, "LTR {} vs RTL {}", ltr_frame.width(), rtl_frame.width());
}

/// RTL twin for the New Spot Channel body: the section label hugs the frame's right edge, the
/// name field follows its caption towards the reading end, and the width matches the LTR twin.
#[test]
fn rtl_the_new_spot_channel_body_mirrors() {
    let new_spot = |dir| {
        dialog_in(dir, |app| {
            open_new_spot(app).unwrap();
        })
    };
    let ltr = new_spot(crate::text::direction::Dir::Ltr);
    let ltr_frame = frame(&ltr);
    let ltr_section = ltr.get_by_label("Ink Characteristics").rect();
    let ltr_name = ltr.get_by_label("Name").rect();
    assert!((ltr_section.left() - ltr_frame.left()).abs() < 40.0, "the section label reads from the left: {ltr_section:?} in {ltr_frame:?}");
    assert!(ltr_name.left() - ltr_frame.left() > 40.0, "the name field follows its caption on the left: {ltr_name:?} in {ltr_frame:?}");

    let rtl = new_spot(crate::text::direction::Dir::Rtl);
    let rtl_frame = frame(&rtl);
    let rtl_section = rtl.get_by_label("Ink Characteristics").rect();
    let rtl_name = rtl.get_by_label("Name").rect();
    assert!(rtl_frame.right() - rtl_section.right() < 40.0, "the section label reads from the right: {rtl_section:?} in {rtl_frame:?}");
    let ltr_inset = ltr_name.left() - ltr_frame.left();
    let rtl_inset = rtl_frame.right() - rtl_name.right();
    assert!((ltr_inset - rtl_inset).abs() < 4.0, "the name field sits at the reading-start edge in both: LTR inset {ltr_inset} vs RTL inset {rtl_inset}");
    assert!((ltr_frame.width() - rtl_frame.width()).abs() < 1.0, "LTR {} vs RTL {}", ltr_frame.width(), rtl_frame.width());
}
