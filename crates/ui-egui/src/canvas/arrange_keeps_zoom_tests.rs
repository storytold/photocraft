//! #16885: Window › Arrange's n-up and Tile layouts keep each document's zoom and view centre instead of fitting every
//! document to its new pane.

use egui_kittest::Harness;
use serde_json::json;

use super::{PhotocraftApp, document_area};

type Area = Harness<'static, (Box<PhotocraftApp>, bool)>;

/// Documents of different sizes, each larger than any pane, so fitting them would give each a zoom of its own below
/// 100 %.
const SIZES: [(u32, u32); 6] = [(1600, 1200), (1200, 1600), (2000, 900), (900, 900), (1400, 1000), (1000, 1400)];

/// The document area showing `count` documents, after their first passes placed their views.
fn area(count: usize) -> Area {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for (i, (width, height)) in SIZES.iter().take(count).enumerate() {
        app.run("file.new", json!({"width": width, "height": height, "name": format!("doc {i}")})).unwrap();
    }
    app.sync_views();
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
            }
        },
        (app, false),
    );
    h.state_mut().1 = true;
    h.run_steps(4);
    h
}

fn set_zoom(h: &mut Area, doc: usize, zoom: f32) {
    let view = &mut h.state_mut().0.ui.views[doc];
    view.zoom = zoom;
    view.clear_pending();
}

fn arrange(h: &mut Area, layout: &str) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(&mut h.state_mut().0, &ctx, &format!("window.arrange.{layout}"), json!({})).unwrap();
    h.run_steps(4);
}

/// Each view's zoom and the document point at its centre.
fn cameras(h: &Area) -> Vec<(f32, [f32; 2])> {
    h.state().0.ui.views.iter().map(|view| (view.zoom, view.center)).collect()
}

fn assert_kept(before: &[(f32, [f32; 2])], after: &[(f32, [f32; 2])], what: &str) {
    assert_eq!(before.len(), after.len(), "{what}: every document keeps its view");
    for (d, ((zoom, center), (now, now_center))) in before.iter().zip(after).enumerate() {
        assert!((zoom - now).abs() < 1e-4, "{what}: document {d} keeps its zoom {zoom}, not {now}");
        let moved = (center[0] - now_center[0]).hypot(center[1] - now_center[1]);
        assert!(moved < 1e-2, "{what}: document {d} keeps its view centre {center:?}, not {now_center:?}");
    }
}

#[test]
fn every_layout_keeps_each_document_at_100_percent_and_centred() {
    let layouts = [
        "twoUpVertical",
        "twoUpHorizontal",
        "threeUpVertical",
        "threeUpHorizontal",
        "threeUpStacked",
        "fourUp",
        "sixUp",
        "tile",
        "tileAllVertically",
        "tileAllHorizontally",
    ];
    for layout in layouts {
        let mut h = area(SIZES.len());
        for doc in 0..SIZES.len() {
            set_zoom(&mut h, doc, 1.0);
        }
        h.run_steps(3);
        let before = cameras(&h);
        let active = h.state().0.session.active_index();
        arrange(&mut h, layout);
        // Tile All Vertically / Horizontally dock as an arrangement only once the framework does (#16665); until then
        // they float the documents, which never fitted them either.
        if !layout.starts_with("tileAll") {
            assert!(h.state().0.ui.document_windows.arrangement.is_some(), "{layout} arranges the document area");
        }
        assert_kept(&before, &cameras(&h), layout);
        assert_eq!(h.state().0.session.active_index(), active, "{layout} keeps the active document");
    }
}

#[test]
fn a_zoomed_document_keeps_its_zoom_through_a_layout_and_back_to_tabs() {
    let mut h = area(2);
    set_zoom(&mut h, 0, 3.0);
    h.run_steps(3);
    let before = cameras(&h);
    arrange(&mut h, "twoUpVertical");
    assert_kept(&before, &cameras(&h), "2-up Vertical");
    arrange(&mut h, "consolidateAllToTabs");
    assert!(h.state().0.ui.document_windows.arrangement.is_none(), "the tabs are back");
    assert_kept(&before, &cameras(&h), "Consolidate All to Tabs");
}
