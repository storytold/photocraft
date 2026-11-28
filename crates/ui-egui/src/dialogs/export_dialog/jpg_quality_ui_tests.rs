//! #13722, the control: what the Export As dialog itself shows for JPG Quality. These tests drive
//! the dialog through egui_kittest and read the accessibility tree, so they pin the row a person
//! sees and an assistive technology reads: the slider's role, name and value, the whole step of one
//! from the keyboard, the step 6 a JPG pick brings up every time, a quality set in a cancelled
//! session, the value the row holds that is outside the seven steps, and the 1 to 100 row WebP keeps.
//!
//! The row's own span is the framework's: at the craft-libs pin of this branch `ExportAsDialog`
//! draws Quality with one 1 to 100 scale for every format
//! (`crates/ui-patterns/src/patterns/form_dialogs/export/as_dialog.rs`, `QUALITY_RANGE`), so the
//! tests below pin the seven steps in what the dialog writes and, for WebP, the 1 to 100 span of the
//! row itself. The per-format scale that lets JPG's row span 1 to 7 with the two ends captioned is
//! proposed in storytold/craft-libs#3181; `the_row_span_is_the_frameworks_and_a_value_outside_the_scale_writes_the_nearest_step`
//! fails when that lands, which is where this file then asserts the 1 to 7 span instead.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

type QualityHarness = Harness<'static, Box<PhotocraftApp>>;

/// An app with a 20 x 10 document; its exporter and writer accept everything. The Export
/// preferences are the shipped ones except that WebP is asked lossy, so its Quality row is drawn
/// (the shipped preference is lossless, where the dialog offers no Quality row for WebP at all).
fn quality_app() -> Box<PhotocraftApp> {
    let services =
        crate::Services { export: Some(Box::new(|_, _, _| Ok((Vec::new(), Vec::new())))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.session.prefs.edit(|p| p.export.webp_lossless = false);
    app
}

/// The Export As dialog open, driven through its own frames.
fn quality_harness() -> QualityHarness {
    let app = quality_app();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    open(h.state_mut()).unwrap();
    h.run_steps(4);
    h
}

/// Picks the format named `label` in the dialog's Format list, as a person clicks it.
fn pick_format(h: &mut QualityHarness, label: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Format").click();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// The settings the open dialog holds.
fn shown(h: &QualityHarness) -> ExportOptions {
    h.state().framework.dialogs.get::<HostedExportAs>().expect("Export As is open").options().clone()
}

/// The Quality row's slider, as the accessibility tree holds it.
fn quality_slider(h: &QualityHarness) -> egui_kittest::Node<'_> {
    h.query_all_by_role(Role::Slider)
        .find(|node| node.accesskit_node().label().as_deref() == Some("Quality"))
        .expect("the Quality slider is in the accessibility tree")
}

/// What the tree says about the Quality slider: the value it announces and the two ends of the
/// scale its row offers.
fn quality_tree(h: &QualityHarness) -> (Option<f64>, Option<f64>, Option<f64>) {
    let node = quality_slider(h).accesskit_node();
    (node.numeric_value(), node.min_numeric_value(), node.max_numeric_value())
}

/// Types `text` into the Quality row's number field and commits it with Enter.
fn type_quality(h: &mut QualityHarness, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, "Quality").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::SpinButton, "Quality").type_text(text);
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// The encoder quality the open dialog's settings would write for the format it is on.
fn written_quality(h: &QualityHarness) -> Option<u8> {
    let options = shown(h);
    let s = settings(&settings_from(&options, PixelSize::new(20, 10)));
    if options.format == "webp" { s.webp_quality } else { s.jpeg_quality }
}

/// JPG Quality is one named slider, and what it announces is the step: role Slider, name "Quality",
/// value 6, and the file writes step 6's encoder quality.
#[test]
fn the_jpg_quality_row_is_a_named_slider_that_announces_the_step() {
    let mut h = quality_harness();
    pick_format(&mut h, "JPG");
    let node = quality_slider(&h).accesskit_node();
    assert_eq!(node.role(), Role::Slider, "the row is a slider in the tree");
    assert_eq!(node.label().as_deref(), Some("Quality"), "and the tree names it");
    assert_eq!(node.value().as_deref(), Some("6"), "and announces the bare number, no percent unit");
    assert_eq!(
        h.get_by_role_and_label(Role::SpinButton, "Quality").accesskit_node().numeric_value(),
        Some(6.0),
        "the value field beside it carries the same step"
    );
    assert_eq!(quality_tree(&h).0, Some(6.0), "the tree announces the step the row shows");
    assert_eq!(shown(&h).quality, 6, "and the settings hold it as a whole step");
    assert_eq!(written_quality(&h), Some(93), "step 6 writes encoder quality 93");
}

/// Picking JPG puts the row on 6 every time: on the first pick, after a pick of another format, and
/// in a dialog that was closed and opened again.
#[test]
fn picking_jpg_puts_quality_on_six_every_time() {
    let mut h = quality_harness();
    pick_format(&mut h, "JPG");
    assert_eq!(shown(&h).quality, 6, "the first pick");
    pick_format(&mut h, "WebP");
    assert_eq!(shown(&h).quality, 85, "WebP takes its own preference");
    pick_format(&mut h, "PNG");
    pick_format(&mut h, "JPG");
    assert_eq!(quality_tree(&h).0, Some(6.0), "a second pick of JPG, as the tree announces it");
    let ctx = h.ctx.clone();
    let id = h.state().export_open.as_ref().map(|o| o.dialog).expect("the dialog is open");
    h.state_mut().framework.dialogs.cancel_by_id(&ctx, id);
    h.run_steps(3);
    open(h.state_mut()).unwrap();
    h.run_steps(3);
    pick_format(&mut h, "JPG");
    assert_eq!(shown(&h).quality, 6, "a reopened dialog picks JPG on 6 as well");
}

/// The row steps by one whole step at a time from the keyboard: a right arrow from 6 is 7 and a
/// left arrow from there is back to 6, one step per key.
#[test]
fn the_quality_row_steps_by_one_from_the_keyboard() {
    let mut h = quality_harness();
    pick_format(&mut h, "JPG");
    quality_slider(&h).focus();
    h.run_steps(2);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(shown(&h).quality, 7, "one right arrow is one step");
    assert_eq!(quality_tree(&h).0, Some(7.0), "and the tree announces it");
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(3);
    assert_eq!(shown(&h).quality, 6, "one left arrow is one step back");
}

/// A Quality value the row can hold but the scale cannot (its span is the framework's, 1 to 100 at
/// this pin) still writes the nearest step, so the seven steps decide the file. This is the row
/// storytold/craft-libs#3181 would replace with a 1 to 7 scale; when the pin moves, the two ends the
/// tree announces change here first.
#[test]
fn the_row_span_is_the_frameworks_and_a_value_outside_the_scale_writes_the_nearest_step() {
    let mut h = quality_harness();
    pick_format(&mut h, "JPG");
    type_quality(&mut h, "40");
    assert_eq!(shown(&h).quality, 40, "the row still holds the framework's 1 to 100");
    assert_eq!(quality_tree(&h), (Some(40.0), Some(1.0), Some(100.0)), "as the tree announces it");
    assert_eq!(written_quality(&h), Some(35), "and the file writes step 2's encoder quality, the nearest of the seven");
    assert_eq!(quality_slider(&h).accesskit_node().value().as_deref(), Some("40"), "the announced number carries no percent unit");
    let captioned = h.root().children_recursive().any(|node| matches!(node.accesskit_node().label().as_deref(), Some("Low") | Some("High")));
    assert!(!captioned, "the two ends of the scale carry no caption at this pin");
}

/// A quality set in a session that ends without an export does not open the next one: JPG on 7,
/// Escape closes the dialog, and the reopened dialog picks JPG on 6 again.
#[test]
fn a_quality_set_in_a_cancelled_session_does_not_open_the_next_one() {
    let mut h = quality_harness();
    pick_format(&mut h, "JPG");
    type_quality(&mut h, "7");
    assert_eq!(shown(&h).quality, 7, "the session did hold 7 before it was cancelled");
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(h.state().export_open.is_none(), "Escape closed the dialog");
    open(h.state_mut()).unwrap();
    h.run_steps(3);
    pick_format(&mut h, "JPG");
    assert_eq!(shown(&h).quality, 6, "the next session picks JPG on 6, not the 7 of the cancelled one");
    assert_eq!(quality_tree(&h).0, Some(6.0), "and the tree announces that 6");
}

/// WebP keeps its own 1 to 100 control: the tree announces its row as spanning 1 to 100, it takes a
/// typed 100, and the encoder writes 100 rather than a JPG step.
#[test]
fn webp_quality_row_keeps_its_1_to_100_range() {
    let mut h = quality_harness();
    pick_format(&mut h, "WebP");
    assert_eq!(shown(&h).quality, 85, "WebP opens on its own preference");
    assert_eq!(quality_tree(&h), (Some(85.0), Some(1.0), Some(100.0)), "WebP's row spans 1 to 100");
    type_quality(&mut h, "100");
    assert_eq!(shown(&h).quality, 100, "WebP takes the top of its 1 to 100 row");
    assert_eq!(quality_tree(&h).0, Some(100.0), "and the tree announces it");
    assert_eq!(written_quality(&h), Some(100), "WebP writes its own quality");
    let s = settings(&settings_from(&shown(&h), PixelSize::new(20, 10)));
    assert_eq!(s.jpeg_quality, None, "and no JPEG quality");
}
