//! The panel and context menus that were converted to the shared menu helpers must mirror
//! like every other menu: under a right-to-left direction each row's label hugs the popup's
//! right edge (the reading start), and left-to-right stays on the left.
//!
//! The fork lays a vertical popup body's widget atoms out left to right — the body scope's
//! cross-align is `Min`, so the layout does not "prefer" right to left — and the project
//! mirrors the rows by hand through [`crate::menu_keys::menu_row`]. A row built with a plain
//! `ui.button` therefore stays left to right, which is what these tests pin down.

use crate::direction::{Dir, direction_scope};
use crate::PhotocraftApp;
use serde_json::json;

/// How far a row's label may sit from the popup's reading-start edge.
const EDGE_SLACK: f32 = 32.0;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app
}

/// Render `body` as an open menu popup in `dir` and return the popup's rect with every painted
/// text galley's rect.
fn menu_rows(dir: Dir, body: impl Fn(&mut PhotocraftApp, &mut egui::Ui)) -> (egui::Rect, Vec<(String, egui::Rect)>) {
    let _scope = direction_scope(dir);
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
    let mut app = app();
    let mut popup = egui::Rect::NOTHING;
    let mut texts = Vec::new();
    for _ in 0..3 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let btn = ui.button("ellipsis");
            let _ = egui::Popup::menu(&btn)
                .open_memory(Some(egui::SetOpenCommand::Bool(true)))
                .show(|ui| {
                    crate::menus::popup_body(ui, |ui| {
                        ui.set_min_width(180.0);
                        body(&mut app, ui);
                    });
                })
                .inspect(|r| popup = r.response.rect);
        });
        texts = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(ts) => Some((ts.galley.text().to_string(), cs.shape.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        out.textures_delta.clear();
    }
    (popup, texts)
}

/// Every label in `labels` hugs the popup's reading-start edge: the right one under RTL, the
/// left one under LTR.
fn assert_rows_mirror(dir: Dir, labels: &[&str], body: impl Fn(&mut PhotocraftApp, &mut egui::Ui)) {
    let (popup, texts) = menu_rows(dir, body);
    assert!(popup.is_positive(), "{dir:?}: the popup did not open");
    for label in labels {
        let row = texts
            .iter()
            .find(|(t, _)| t.contains(label))
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("{dir:?}: the row {label} did not paint: {texts:?}"));
        if dir == Dir::Rtl {
            assert!(popup.right() - row.right() < EDGE_SLACK, "{dir:?}: {label} hugs the popup's right edge: {row:?} in {popup:?}");
        } else {
            assert!(row.left() - popup.left() < EDGE_SLACK, "{dir:?}: {label} hugs the popup's left edge: {row:?} in {popup:?}");
        }
    }
}

/// The Swatches panel's ☰ menu: every row mirrors, the ones after a separator included.
#[test]
fn swatches_panel_menu_rows_mirror_only_under_rtl() {
    let labels = ["New Swatch…", "New Swatch Group…", "Import Swatches…", "Replace Swatches…", "Export Swatches…", "Reset Swatches"];
    assert_rows_mirror(Dir::Rtl, &labels, super::swatches_ui::panel_menu);
    assert_rows_mirror(Dir::Ltr, &labels, super::swatches_ui::panel_menu);
}

/// The Actions panel's ☰ menu: every row mirrors, the ones after a separator included.
#[test]
fn actions_panel_menu_rows_mirror_only_under_rtl() {
    let labels = ["New Action…", "Play", "Start Recording", "Delete", "Insert Stop…", "Load Actions…", "Save Actions…"];
    assert_rows_mirror(Dir::Rtl, &labels, super::actions::panel_menu);
    assert_rows_mirror(Dir::Ltr, &labels, super::actions::panel_menu);
}

/// The Histogram panel's ☰ menu: its radio and checkbox rows are menu rows too, so they mirror
/// like the plain ones.
#[test]
fn histogram_panel_menu_rows_mirror_only_under_rtl() {
    let labels = ["Uncached Refresh", "Compact View", "Expanded View", "All Channels View", "Show Statistics", "Show Channels in Color"];
    assert_rows_mirror(Dir::Rtl, &labels, super::histogram_panel::panel_menu);
    assert_rows_mirror(Dir::Ltr, &labels, super::histogram_panel::panel_menu);
}

/// The Styles panel's ☰ menu.
#[test]
fn styles_panel_menu_rows_mirror_only_under_rtl() {
    let labels = ["Import Styles…", "Export Selected Styles…", "Export Styles…"];
    assert_rows_mirror(Dir::Rtl, &labels, super::preset_panels::styles_menu);
    assert_rows_mirror(Dir::Ltr, &labels, super::preset_panels::styles_menu);
}

/// The units menu the rulers and the options bar's unit fields open: its rows go through the
/// shared row helper inside a scroll area, and mirror all the same.
#[test]
fn unit_menu_rows_mirror_only_under_rtl() {
    let labels = ["Pixels", "Inches", "Centimeters"];
    assert_rows_mirror(Dir::Rtl, &labels, |_app, ui| {
        crate::rulers::unit_menu(ui, photocraft_engine::prefs::Unit::Pixels);
    });
    assert_rows_mirror(Dir::Ltr, &labels, |_app, ui| {
        crate::rulers::unit_menu(ui, photocraft_engine::prefs::Unit::Pixels);
    });
}
