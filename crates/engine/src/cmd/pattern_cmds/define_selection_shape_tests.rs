use photocraft_color::PixelFormat;
use photocraft_doc::Rect;
use photocraft_raster::Surface;
use serde_json::json;

use super::*;

fn new_filled_doc(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 8})).unwrap();
    s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
    s
}

#[test]
fn elliptical_feathered_and_multi_part_selections_disable_and_reject_define() {
    let mut s = new_filled_doc(160, 120);
    let before = s.patterns.items.len();

    s.execute("select.ellipse", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
    assert!(!s.is_enabled("edit.definePattern"));
    assert!(s.execute("edit.definePattern", json!({})).is_err());

    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 60, "height": 40, "feather": 5})).unwrap();
    assert!(!s.is_enabled("edit.definePattern"));
    assert!(s.execute("edit.definePattern", json!({})).is_err());

    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("select.rect", json!({"x": 65, "y": 45, "width": 20, "height": 20, "mode": "add"})).unwrap();
    assert!(!s.is_enabled("edit.definePattern"));
    assert!(s.execute("edit.definePattern", json!({})).is_err());

    assert_eq!(s.patterns.items.len(), before);
}

#[test]
fn single_sharp_rectangle_and_clipped_selection_and_explicit_rect_succeed() {
    let mut s = new_filled_doc(120, 80);

    assert!(s.is_enabled("edit.definePattern"));
    let whole = s.execute("edit.definePattern", json!({"name": "Whole"})).unwrap();
    assert_eq!((whole["info"]["width"], whole["info"]["height"]), (json!(120), json!(80)));

    s.execute("select.rect", json!({"x": 60, "y": 30, "width": 50, "height": 45})).unwrap();
    assert!(s.is_enabled("edit.definePattern"));
    let sub = s.execute("edit.definePattern", json!({"name": "Sub"})).unwrap();
    assert_eq!((sub["info"]["width"], sub["info"]["height"]), (json!(50), json!(45)));

    s.execute("select.rect", json!({"x": 100, "y": 60, "width": 40, "height": 40})).unwrap();
    assert!(s.is_enabled("edit.definePattern"));
    let clipped = s.execute("edit.definePattern", json!({"name": "Clipped"})).unwrap();
    assert_eq!((clipped["info"]["width"], clipped["info"]["height"]), (json!(20), json!(20)));

    s.execute("select.ellipse", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    let explicit = s.execute("edit.definePattern", json!({"name": "Explicit", "rect": [5, 5, 25, 25]})).unwrap();
    assert_eq!((explicit["info"]["width"], explicit["info"]["height"]), (json!(20), json!(20)));
}

#[test]
fn odd_selection_masks_never_panic() {
    let mut s = new_filled_doc(64, 64);

    // Empty mask
    s.active_mut().unwrap().doc.selection = Some(Surface::new(PixelFormat::GRAY32F));
    assert!(!s.is_enabled("edit.definePattern"));
    assert!(s.execute("edit.definePattern", json!({})).is_err());

    // One-pixel sharp selection
    let mut one_px = Surface::new(PixelFormat::GRAY32F);
    one_px.write_region(Rect::new(10, 10, 11, 11), &[1.0]);
    s.active_mut().unwrap().doc.selection = Some(one_px);
    assert!(s.is_enabled("edit.definePattern"));
    let one = s.execute("edit.definePattern", json!({"name": "OnePx"})).unwrap();
    assert_eq!((one["info"]["width"], one["info"]["height"]), (json!(1), json!(1)));

    // Larger than the canvas
    let mut huge = Surface::new(PixelFormat::GRAY32F);
    huge.write_region(Rect::new(-10, -10, 80, 80), &vec![1.0; 90 * 90]);
    s.active_mut().unwrap().doc.selection = Some(huge);
    assert!(s.is_enabled("edit.definePattern"));
    let full = s.execute("edit.definePattern", json!({"name": "Full"})).unwrap();
    assert_eq!((full["info"]["width"], full["info"]["height"]), (json!(64), json!(64)));
}
