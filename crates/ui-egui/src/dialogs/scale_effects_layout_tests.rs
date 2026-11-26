//! #14996: Scale Layer Effects is titled as in Photoshop and its one row is a whole-percent field whose arrow opens a
//! linear slider, instead of the schema dialog's always-shown logarithmic slider for 1..1000.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::{PhotocraftApp, Services};

const SCALE_EFFECTS: &str = "layer.layerStyle.scaleEffects";

/// The app with a document open and `command`'s dialog shown through `dialogs::run`.
fn dialog(command: &str) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    super::filter_dialog::open(&mut app, command).unwrap();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run();
    h
}

fn sliders(h: &Harness<'_, Box<PhotocraftApp>>) -> usize {
    h.query_all_by_role(Role::Slider).count()
}

fn scale(h: &Harness<'_, Box<PhotocraftApp>>) -> f64 {
    h.get_by_role_and_label(Role::SpinButton, "Scale").accesskit_node().numeric_value().expect("the Scale field has a number")
}

#[test]
fn the_dialog_is_titled_scale_layer_effects() {
    let h = dialog(SCALE_EFFECTS);
    assert!(h.query_by_role_and_label(Role::Dialog, "Scale Layer Effects").is_some());
    assert!(h.query_by_role_and_label(Role::Dialog, "Scale Effects").is_none());
}

#[test]
fn the_scale_row_is_a_field_with_a_popup_slider_and_no_drawn_slider() {
    let h = dialog(SCALE_EFFECTS);
    assert_eq!(scale(&h), 100.0);
    assert_eq!(sliders(&h), 0, "no slider is drawn under the field");
    assert!(h.query_by_role_and_label(Role::Button, "Slider").is_some(), "the field has the popup-slider arrow");
}

#[test]
fn the_popup_slider_runs_linearly_over_1_to_1000_and_sets_the_field() {
    let mut h = dialog(SCALE_EFFECTS);
    h.get_by_role_and_label(Role::Button, "Slider").click();
    h.run();
    let track = h.get_by_role(Role::Slider).rect();
    let middle = track.center();
    h.hover_at(middle);
    h.drag_at(middle);
    h.run();
    h.drop_at(middle);
    h.run();
    let value = scale(&h);
    assert!((450.0..=550.0).contains(&value), "the middle of a linear 1..1000 slider is about 500, got {value}");
    assert_eq!(value.fract(), 0.0, "whole percents");
}

#[test]
fn other_wide_range_rows_keep_their_drawn_slider() {
    let h = dialog("filter.blur.gaussianBlur");
    assert_eq!(sliders(&h), 1, "Gaussian Blur's 0.1..1000 radius keeps today's slider");
}
