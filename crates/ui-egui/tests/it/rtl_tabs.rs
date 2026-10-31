//! The shared tab strip mirrors in Arabic: the Properties card's pills read from the right,
//! while the English strip keeps its left-to-right order. The language switches through the
//! preferences, so the whole direction plumbing is exercised.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, theme};
use serde_json::json;

fn harness(lang: &str) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().allow_missing_glyphs().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme::ThemeKind::Studio);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.theme = theme::ThemeKind::Studio;
        // The floating Properties card replaces the dock in the Studio layout.
        app.ui.panels.dock = false;
        app.run("prefs.set", json!({"path": "interface.theme", "value": theme::ThemeKind::Studio.id()})).expect("theme");
        app.run("file.new", json!({"width": 640, "height": 480})).expect("new document");
        app
    });
    h.run_steps(2);
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": lang})).expect("language");
    h.run_steps(6);
    h
}

/// The rects of the Properties card's two pills, in document order.
fn pills(h: &Harness<'static, PhotocraftApp>) -> (egui::Rect, egui::Rect) {
    let tabs = egui::Id::unique("properties-card").with("tabs");
    let first = h.ctx.read_response(tabs.with(("tab", 0usize))).expect("the first pill").rect;
    let second = h.ctx.read_response(tabs.with(("tab", 1usize))).expect("the second pill").rect;
    (first, second)
}

/// The Arabic card reads from the right: the first pill (Properties) is the rightmost one; the
/// English card keeps the first pill at the left.
#[test]
fn the_properties_pills_read_from_the_right_in_arabic() {
    for (lang, first_on_the_right) in [("ar", true), ("en", false)] {
        let h = harness(lang);
        let (first, second) = pills(&h);
        assert_eq!(first_on_the_right, first.right() > second.right(), "{lang}: the first pill is the reading-start one ({first:?} vs {second:?})");
    }
}
