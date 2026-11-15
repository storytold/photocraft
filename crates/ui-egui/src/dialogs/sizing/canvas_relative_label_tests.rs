//! #14585: Canvas Size's checkbox reads "Relative to current dimension", as the reference
//! application's does. Its behaviour (fields switch between absolute size and change) is
//! unchanged.

use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const LABEL: &str = "Relative to current dimension";

fn harness() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.canvasSize").expect("the Canvas Size dialog opens");
    h.run_steps(3);
    (h, id)
}

#[test]
fn the_checkbox_reads_relative_to_current_dimension() {
    let (h, _) = harness();
    assert!(h.query_by_label(LABEL).is_some(), "the Canvas Size checkbox shows the measured label");
    assert!(h.query_by_label("Relative").is_none(), "the short label is gone from this dialog");
}

#[test]
fn toggling_it_still_switches_the_fields_to_deltas() {
    let (mut h, id) = harness();
    h.get_by_label(LABEL).click();
    h.run_steps(3);
    let f = &h.state_mut().ui.dialog_mut(id).expect("still open").fields;
    assert_eq!(f.get("relative").and_then(Value::as_bool), Some(true), "{f:?}");
    // 400 x 300 absolute becomes a zero change, as before the label change.
    assert_eq!((num(f, "width"), num(f, "height")), (0.0, 0.0), "{f:?}");
}
