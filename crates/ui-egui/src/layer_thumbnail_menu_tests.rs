//! #11680: a right-click on a layer's thumbnail offers Select Pixels (and the add / subtract /
//! intersect variants), as in Photoshop; a right-click elsewhere on the row keeps the layer menu.

use egui::{PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::layer_row_ui::{RowRects, recorded};

const ENTRIES: [&str; 4] = ["Select Pixels", "Add Pixels to Selection", "Subtract Pixels from Selection", "Intersect Pixels with Selection"];

/// A white Background plus a "paint" layer with a 20x16 block at (10, 12); nothing selected.
fn panel() -> (Harness<'static, PhotocraftApp>, u64, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    let background = s.active().unwrap().doc.background().unwrap().id.0;
    let paint = s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 12, "width": 20, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    (h, background, paint)
}

fn row(h: &Harness<'static, PhotocraftApp>, id: u64) -> RowRects {
    recorded(&h.ctx).into_iter().find(|r| r.layer == id).expect("the row was drawn")
}

fn right_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(if pressed { 1 } else { 3 });
    }
}

fn thumb_point(h: &Harness<'static, PhotocraftApp>, id: u64) -> Pos2 {
    row(h, id).thumbnail.expect("thumbnails are on").center()
}

fn menu_open(h: &Harness<'static, PhotocraftApp>) -> bool {
    ENTRIES.iter().any(|l| h.query_by_label(l).is_some())
}

fn selection_bounds(h: &Harness<'static, PhotocraftApp>) -> Option<Rect> {
    h.state().session.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
}

#[test]
fn a_thumbnail_right_click_lists_select_pixels_first() {
    let (mut h, _, paint) = panel();
    let p = thumb_point(&h, paint);
    right_click(&mut h, p);
    let ys: Vec<f32> = ENTRIES.iter().map(|l| h.get_by_label(l).rect().top()).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "Select Pixels first, then add / subtract / intersect: {ys:?}");
    let duplicate = h.get_by_label("Duplicate Layer…").rect().top();
    assert!(duplicate > ys[3], "the layer menu follows the Select Pixels entries");
}

#[test]
fn picking_select_pixels_selects_the_layers_painted_pixels() {
    let (mut h, _, paint) = panel();
    assert_eq!(selection_bounds(&h), None);
    let p = thumb_point(&h, paint);
    right_click(&mut h, p);
    h.get_by_label("Select Pixels").click();
    h.run_steps(4);
    assert_eq!(selection_bounds(&h), Some(Rect::new(10, 12, 30, 28)), "the 20x16 block painted at (10, 12)");
    assert!(!menu_open(&h), "the menu closed");
}

#[test]
fn the_background_thumbnail_entries_are_disabled() {
    let (mut h, background, _) = panel();
    let p = thumb_point(&h, background);
    right_click(&mut h, p);
    let node = h.get_by_label("Select Pixels");
    assert!(node.accesskit_node().is_disabled(), "the Background has no transparency to load");
    node.click();
    h.run_steps(4);
    assert_eq!(selection_bounds(&h), None, "a disabled Select Pixels selects nothing");
}

#[test]
fn a_right_click_on_the_row_name_still_opens_the_layer_menu() {
    let (mut h, _, paint) = panel();
    let r = row(&h, paint);
    right_click(&mut h, r.name.expect("the name was laid out").center());
    assert!(!menu_open(&h), "no thumbnail menu away from the thumbnail");
    assert!(h.query_by_label("Duplicate Layer…").is_some(), "the layer menu opened");
}
