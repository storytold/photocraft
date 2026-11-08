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

/// How many of the 64x64 canvas's pixels are selected (more than half), 0 with no selection.
fn selected_area(h: &Harness<'static, PhotocraftApp>) -> usize {
    let Some(m) = h.state().session.active().unwrap().doc.selection.as_ref() else { return 0 };
    (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| m.sample_channel(x, y, 0) > 0.5).count()
}

/// Seeds a 20x20 selection at (20, 20) that overlaps the painted block (10..30 x 12..28) in 10x8 pixels.
fn seed_selection(h: &mut Harness<'static, PhotocraftApp>) {
    h.state_mut().session.execute("select.rect", json!({"x": 20, "y": 20, "width": 20, "height": 20})).unwrap();
    h.run_steps(4);
    assert_eq!(selected_area(h), 400);
}

fn pick(h: &mut Harness<'static, PhotocraftApp>, layer: u64, entry: &str) {
    let p = thumb_point(h, layer);
    right_click(h, p);
    h.get_by_label(entry).click();
    h.run_steps(4);
}

#[test]
fn select_pixels_selects_exactly_the_painted_area() {
    let (mut h, _, paint) = panel();
    pick(&mut h, paint, "Select Pixels");
    assert_eq!(selected_area(&h), 20 * 16, "the painted block, pixel for pixel");
}

#[test]
fn select_pixels_replaces_an_existing_selection() {
    let (mut h, _, paint) = panel();
    seed_selection(&mut h);
    pick(&mut h, paint, "Select Pixels");
    assert_eq!(selected_area(&h), 320, "new replaces the seeded selection");
    let m = h.state().session.active().unwrap().doc.selection.clone().unwrap();
    assert!(m.sample_channel(12, 14, 0) > 0.5 && m.sample_channel(35, 35, 0) < 0.5);
}

#[test]
fn add_pixels_unites_with_the_selection() {
    let (mut h, _, paint) = panel();
    seed_selection(&mut h);
    pick(&mut h, paint, "Add Pixels to Selection");
    assert_eq!(selected_area(&h), 400 + 320 - 80, "the union of the seeded square and the block");
}

#[test]
fn subtract_pixels_removes_the_layer_from_the_selection() {
    let (mut h, _, paint) = panel();
    seed_selection(&mut h);
    pick(&mut h, paint, "Subtract Pixels from Selection");
    assert_eq!(selected_area(&h), 400 - 80, "the seeded square minus the block");
}

#[test]
fn intersect_pixels_keeps_only_the_overlap() {
    let (mut h, _, paint) = panel();
    seed_selection(&mut h);
    pick(&mut h, paint, "Intersect Pixels with Selection");
    assert_eq!(selected_area(&h), 80, "the 10x8 overlap");
}

#[test]
fn a_transparent_layer_selects_nothing() {
    let (mut h, _, _) = panel();
    let empty = h.state_mut().session.execute("layer.new.layer", json!({"name": "empty"})).unwrap()["layer"].as_u64().unwrap();
    h.run_steps(6);
    pick(&mut h, empty, "Select Pixels");
    assert_eq!(selected_area(&h), 0, "nothing painted, nothing selected");
}

#[test]
fn every_background_entry_is_disabled_and_does_nothing() {
    let (mut h, background, _) = panel();
    seed_selection(&mut h);
    let p = thumb_point(&h, background);
    for entry in ENTRIES {
        right_click(&mut h, p);
        let node = h.get_by_label(entry);
        assert!(node.accesskit_node().is_disabled(), "{entry} is disabled for the Background");
        node.click();
        h.run_steps(4);
        assert_eq!(selected_area(&h), 400, "{entry} left the selection alone");
    }
}

#[test]
fn a_mask_thumbnail_keeps_the_mask_menu() {
    let (mut h, _, paint) = panel();
    {
        let s = &mut h.state_mut().session;
        s.execute("layer.select", json!({"layer": paint, "mode": "replace"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    }
    h.run_steps(8);
    let (thumbs, _) = crate::mask_thumbs_ui::recorded(&h.ctx, paint).expect("the mask thumbnail was drawn");
    let at = thumbs.iter().find(|(k, _)| *k == crate::mask_thumbs_ui::MaskKind::Pixel).expect("a pixel mask thumbnail").1.center();
    right_click(&mut h, at);
    assert!(!menu_open(&h), "no Select Pixels entries on a mask thumbnail");
    h.get_by_label("Delete Layer Mask");
    assert!(h.query_by_label("Duplicate Layer…").is_none(), "the mask menu, not the layer menu");
}

#[test]
fn a_frame_row_keeps_the_original_menu() {
    let (mut h, _, _) = panel();
    let frame = h.state_mut().session.execute("layer.new.frame", json!({"x": 4, "y": 4, "width": 30, "height": 30})).unwrap()["layer"].as_u64().unwrap();
    h.run_steps(8);
    let r = row(&h, frame);
    let p = r.thumbnail.map_or_else(|| Pos2::new(r.row.left() + 30.0, r.row.center().y), |t| t.center());
    right_click(&mut h, p);
    assert!(!menu_open(&h), "no Select Pixels entries on a frame row");
    assert!(h.query_by_label("Duplicate Layer…").is_some() || h.query_by_label("Delete Layer").is_some(), "the layer menu opened");
}

#[test]
fn with_thumbnails_off_the_menu_is_the_original() {
    let (mut h, _, paint) = panel();
    let p = thumb_point(&h, paint);
    h.state_mut().ui.layers_panel_options.thumbnail_size = crate::state::LayerThumbnailSize::None;
    h.run_steps(8);
    assert!(row(&h, paint).thumbnail.is_none(), "thumbnails are off");
    right_click(&mut h, p);
    assert!(!menu_open(&h), "no thumbnail to right-click, no Select Pixels entries");
    assert!(h.query_by_label("Duplicate Layer…").is_some(), "the layer menu opened");
}
