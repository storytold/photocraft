//! Tests for the shared right-to-left single-line edit helpers ([`super::rtl_singleline`],
//! [`super::rtl_singleline_layouter`], [`super::rtl_singleline_edit`],
//! [`super::rtl_multiline_edit`]) and for [`super::text_field`]'s own base direction.

use egui_kittest::Harness;

use super::{rtl_multiline_edit, rtl_singleline, rtl_singleline_edit, rtl_singleline_layouter, text_field};
use crate::text::direction::{Dir, direction_scope};

/// The base direction of the painted galley showing `text`, if it was painted this frame.
fn painted_direction<S>(h: &Harness<'static, S>, text: &str) -> Option<egui::epaint::text::TextDirection> {
    fn walk(shape: &egui::Shape, text: &str, found: &mut Option<egui::epaint::text::TextDirection>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, found)),
            egui::Shape::Text(t) if t.galley.text() == text => *found = Some(t.galley.job.direction),
            _ => {}
        }
    }
    let mut found = None;
    h.output().shapes.iter().for_each(|c| walk(&c.shape, text, &mut found));
    found
}

/// `rtl_singleline` pins the galley's base direction right-to-left in an RTL UI and leaves
/// egui's default (auto) edit alone in an LTR one.
#[test]
fn rtl_singleline_pins_the_base_direction_only_in_rtl() {
    let probe = |dir: Dir| {
        let _scope = direction_scope(dir);
        let mut h = Harness::builder()
            .with_size(egui::vec2(300.0, 80.0))
            .build_ui_state(
                |ui, s: &mut String| {
                    let mut layouter = rtl_singleline_layouter(egui::FontId::proportional(12.5));
                    ui.add(rtl_singleline(s, &mut layouter).desired_width(200.0).hint_text("name"));
                },
                "abc".to_string(),
            );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        painted_direction(&h, "abc")
    };
    assert_eq!(probe(Dir::Ltr), Some(egui::epaint::text::TextDirection::Auto), "LTR: the plain edit is unchanged");
    assert_eq!(probe(Dir::Rtl), Some(egui::epaint::text::TextDirection::RightToLeft), "RTL: the base direction is pinned");
}

/// The shared `text_field` lays its value out right-to-left in an RTL UI (not just right-aligned),
/// while a monospaced hex/code field keeps the default direction in both directions.
#[test]
fn text_field_pins_the_value_direction_in_rtl_but_not_for_mono() {
    let probe = |dir: Dir, mono: bool| {
        let _scope = direction_scope(dir);
        let mut h = Harness::builder()
            .with_size(egui::vec2(300.0, 80.0))
            .build_ui_state(
                move |ui, s: &mut String| drop(text_field(ui, egui::Id::unique("tf"), s, "hint", 200.0, mono)),
                "abc".to_string(),
            );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        painted_direction(&h, "abc")
    };
    assert_eq!(probe(Dir::Ltr, false), Some(egui::epaint::text::TextDirection::Auto), "LTR: unchanged");
    assert_eq!(probe(Dir::Rtl, false), Some(egui::epaint::text::TextDirection::RightToLeft), "RTL: pinned");
    assert_eq!(probe(Dir::Rtl, true), Some(egui::epaint::text::TextDirection::Auto), "RTL mono: code stays auto/LTR");
}

/// The `rtl_singleline_edit`/`rtl_multiline_edit` wrappers pin the galley's base direction
/// right-to-left in an RTL UI and leave egui's default edits alone in an LTR one.
#[test]
fn rtl_edit_wrappers_pin_the_base_direction_only_in_rtl() {
    let probe = |dir: Dir| {
        let _scope = direction_scope(dir);
        let mut h = Harness::builder()
            .with_size(egui::vec2(300.0, 120.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, s: &mut (String, String)| {
                    rtl_singleline_edit(ui, &mut s.0, 200.0, "name");
                    rtl_multiline_edit(ui, &mut s.1, 3, 200.0);
                },
                ("abc".to_string(), "note".to_string()),
            );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        (painted_direction(&h, "abc"), painted_direction(&h, "note"))
    };
    assert_eq!(
        probe(Dir::Ltr),
        (Some(egui::epaint::text::TextDirection::Auto), Some(egui::epaint::text::TextDirection::Auto)),
        "LTR: both plain edits are unchanged"
    );
    assert_eq!(
        probe(Dir::Rtl),
        (Some(egui::epaint::text::TextDirection::RightToLeft), Some(egui::epaint::text::TextDirection::RightToLeft)),
        "RTL: both base directions are pinned"
    );
}
