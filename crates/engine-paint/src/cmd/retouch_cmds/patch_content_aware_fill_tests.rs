//! Patch in Content-Aware mode fills the selection to match its surroundings (#12118),
//! instead of pasting a copy of the dragged-to area.
//!
//! The test document is the one measured in the issue: 800 x 600 RGB 8 bit; left half grey
//! 128,128,128, right half green 40,160,60; an 80 x 80 checker at 100,100 with 10 px cells of
//! 60,60,60 and 200,200,200; a 20 x 20 black spot at 190,390; a 20 x 20 white spot at 590,190;
//! a red 220,30,30 stripe x 395 to 405, y 350 to 600.

use super::*;

/// An 8-bit level as the normalised float the surfaces hold.
fn l8(v: u8) -> f32 {
    f32::from(v) / 255.0
}

/// The measured test document at 8 bit.
fn test_document() -> Session {
    test_document_at(8)
}

/// The measured test document at the given pixel `depth`.
fn test_document_at(depth: u64) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let rgb = if (100..180).contains(&x) && (100..180).contains(&y) {
                    let v = if (x / 10 + y / 10) % 2 == 0 { 60 } else { 200 };
                    [l8(v), l8(v), l8(v)]
                } else if (190..210).contains(&x) && (390..410).contains(&y) {
                    [0.0; 3]
                } else if (590..610).contains(&x) && (190..210).contains(&y) {
                    [1.0; 3]
                } else if (395..=405).contains(&x) && (350..600).contains(&y) {
                    [l8(220), l8(30), l8(30)]
                } else if x < 400 {
                    [l8(128); 3]
                } else {
                    [l8(40), l8(160), l8(60)]
                };
                data.extend(photocraft_raster::from_rgba(&fmt, [rgb[0], rgb[1], rgb[2], 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn rgb(s: &Session, x: i32, y: i32) -> [f32; 3] {
    let d = s.active().unwrap();
    let p = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [p[0], p[1], p[2]]
}

/// Channels per pixel of the active layer's surface.
fn surface_channels(s: &Session) -> usize {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().channels().max(1)
}

/// The active layer's whole-canvas samples, row-major.
fn layer_pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds())
}

/// The fresh session with the `x, y, w, h` rectangle selected and `paint.patch` Content-Aware
/// run at `offset`, `structure`, `color`.
fn patched(offset: [i32; 2], structure: u8, color: u8, x: i32, y: i32, w: u32, h: u32) -> Session {
    let mut s = test_document();
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    let r = s.execute("paint.patch", json!({"offset": offset, "contentAware": true, "structure": structure, "color": color})).unwrap();
    assert_eq!(r["contentAware"], json!(true));
    s
}

/// Every pixel of row `y` from `x0` (inclusive) to `x1` (exclusive) is within `tol` of `want`.
fn assert_row(s: &Session, y: i32, x0: i32, x1: i32, want: [f32; 3], tol: f32, what: &str) {
    for x in x0..x1 {
        let got = rgb(s, x, y);
        for c in 0..3 {
            assert!(
                (got[c] - want[c]).abs() <= tol,
                "{what}: at {x},{y} channel {c} is {} want {} (+/-{})",
                (got[c] * 255.0).round(),
                (want[c] * 255.0).round(),
                tol * 255.0
            );
        }
    }
}

const LEVELS3: f32 = 3.0 / 255.0;

/// Done when #1: the black spot (180,380, 40 x 40) dragged onto the checker (source at
/// (140,120), the offset up and left) fills to the plain grey of its surroundings, for every
/// Structure 1..7 and Color 0..10. No checker texture may appear.
#[test]
fn content_aware_patch_fills_the_black_spot_with_its_grey_surroundings_for_every_structure_and_color() {
    for structure in 1..=7u8 {
        for color in 0..=10u8 {
            let s = patched([-40, -260], structure, color, 180, 380, 40, 40);
            assert_row(&s, 400, 182, 218, [l8(128); 3], LEVELS3, &format!("structure {structure}, color {color}"));
        }
    }
}

/// The depth the replaced test had (8 and 16): the black-spot cell at 16 bit, same window and
/// the command's defaults - the fill runs at 16 bit and lands within the same 3 levels of the
/// grey (fails on main, which pastes the checker there too).
#[test]
fn content_aware_patch_fill_works_at_16_bits() {
    let mut s = test_document_at(16);
    s.execute("select.rect", json!({"x": 180, "y": 380, "width": 40, "height": 40})).unwrap();
    let r = s.execute("paint.patch", json!({"offset": [-40, -260], "contentAware": true})).unwrap();
    assert_eq!(r["contentAware"], json!(true));
    assert_row(&s, 400, 182, 218, [l8(128); 3], LEVELS3, "16 bit");
}

/// Done when #2: the white spot (580,180, 40 x 40) dragged onto the red stripe (the issue's
/// offset: the stripe lands under x 590..600 of the selection) fills to the plain green of its
/// surroundings, for every Structure 1..7 and Color 0..10. No red stripe may appear.
#[test]
fn content_aware_patch_fills_the_white_spot_with_its_green_surroundings_for_every_structure_and_color() {
    for structure in 1..=7u8 {
        for color in 0..=10u8 {
            let s = patched([-195, 200], structure, color, 580, 180, 40, 40);
            assert_row(&s, 200, 582, 618, [l8(40), l8(160), l8(60)], LEVELS3, &format!("structure {structure}, color {color}"));
        }
    }
}

/// Done when #3: only the selected area changes; the selection stays; the History step is the
/// Patch's and one undo restores the document byte for byte.
#[test]
fn content_aware_patch_changes_only_the_selection_and_keeps_the_selection_and_history() {
    let mut s = test_document();
    s.execute("select.rect", json!({"x": 180, "y": 380, "width": 40, "height": 40})).unwrap();
    let before = layer_pixels(&s);
    s.execute("paint.patch", json!({"offset": [-40, -260], "contentAware": true})).unwrap();
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["selectionBounds"], json!([180, 380, 40, 40]), "the selection stays");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Patch Selection"), "the History step as today");
    let after = layer_pixels(&s);
    let n = surface_channels(&s);
    for y in 0..600 {
        for x in 0..800 {
            let i = (y * 800 + x) * n;
            let inside = (180..220).contains(&x) && (380..420).contains(&y);
            if inside {
                continue;
            }
            assert_eq!(&after[i..i + n], &before[i..i + n], "outside the selection at {x},{y}");
        }
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layer_pixels(&s), before, "one undo restores the document");
}

/// Done when #4 (the not measured case): on textured surroundings the fill stays a texture fill,
/// fed by the sampling area that holds the dragged-to place. Guard test, passes before and after:
/// it pins that the mode keeps synthesising from texture and never flattens.
#[test]
fn content_aware_patch_on_textured_surroundings_still_fills_with_texture() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 200, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = if x / 20 % 2 == 0 { 200 } else { 40 };
                data.extend(photocraft_raster::from_rgba(&fmt, [l8(v), l8(v), l8(v), 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    // The selection sits on the stripes; the dragged-to place (same stripes) is inside the
    // sampling window.
    s.execute("select.rect", json!({"x": 60, "y": 90, "width": 40, "height": 40})).unwrap();
    s.execute("paint.patch", json!({"offset": [120, 0], "contentAware": true, "structure": 4, "color": 0})).unwrap();
    // The hole carries the stripe texture: every interior row spans both stripe levels.
    for y in 96..124 {
        let (mut lo, mut hi) = (1.0f32, 0.0f32);
        for x in 66..94 {
            let p = rgb(&s, x, y);
            assert!((p[0] - p[1]).abs() < 0.02 && (p[1] - p[2]).abs() < 0.02, "grey kept at {x},{y}: {p:?}");
            lo = lo.min(p[0]);
            hi = hi.max(p[0]);
        }
        assert!(lo < l8(80) && hi > l8(160), "row {y} flattened: min {} max {}", (lo * 255.0).round(), (hi * 255.0).round());
    }
    // The dragged-to area itself is untouched (only the selection changes).
    assert_eq!(rgb(&s, 190, 110), [l8(40); 3], "sampled area unchanged");
    assert_eq!(rgb(&s, 210, 110), [l8(200); 3], "sampled area unchanged");
}

/// The Normal mode of the Patch tool is unchanged: in the white-spot case the stripe is copied
/// into the selection (measured in the issue), Content-Aware off.
#[test]
fn normal_mode_patch_still_copies_the_red_stripe_into_the_selection() {
    let mut s = test_document();
    s.execute("select.rect", json!({"x": 580, "y": 180, "width": 40, "height": 40})).unwrap();
    s.execute("paint.patch", json!({"offset": [-195, 200]})).unwrap();
    let red = rgb(&s, 595, 200);
    assert!(red[0] - red[1] > 0.2 && red[0] - red[2] > 0.2, "the stripe is copied at 595,200: {red:?}");
    let green = rgb(&s, 612, 200);
    assert!(green[1] - green[0] > 0.2 && green[1] - green[2] > 0.15, "the sampled green follows: {green:?}");
}

/// A selection touching the canvas edge: the Content-Aware fill still matches the surroundings
/// (the magenta half the selection lies in), not the blue half it was dragged onto.
#[test]
fn content_aware_patch_at_the_canvas_edge_fills_from_the_surroundings_not_the_dragged_to_place() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 120, "height": 60, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let rgb = if x < 40 { [l8(200), l8(60), l8(60)] } else { [l8(60), l8(60), l8(200)] };
                data.extend(photocraft_raster::from_rgba(&fmt, [rgb[0], rgb[1], rgb[2], 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 18, "width": 32, "height": 24})).unwrap();
    s.execute("paint.patch", json!({"offset": [40, 0], "contentAware": true, "structure": 4, "color": 0})).unwrap();
    let tol = 8.0 / 255.0;
    for y in 24..40 {
        for x in 6..26 {
            let got = rgb(&s, x, y);
            let want = [l8(200), l8(60), l8(60)];
            for c in 0..3 {
                assert!((got[c] - want[c]).abs() <= tol, "edge fill at {x},{y} channel {c}: {} want {}", (got[c] * 255.0).round(), (want[c] * 255.0).round());
            }
        }
    }
}

/// The command's own checks stay: an empty drag and out-of-range Structure / Color are refused
/// before any work, and the document is untouched.
#[test]
fn content_aware_patch_refuses_the_empty_drag_and_bad_options() {
    let mut s = test_document();
    s.execute("select.rect", json!({"x": 180, "y": 380, "width": 40, "height": 40})).unwrap();
    let h = s.active().unwrap().history.past_len();
    for p in [
        json!({"offset": [0, 0], "contentAware": true}),
        json!({"offset": [-40, -260], "contentAware": true, "structure": 9}),
        json!({"offset": [-40, -260], "contentAware": true, "color": -1}),
    ] {
        assert!(s.execute("paint.patch", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), h, "no history entry for the refusals");
}

/// An empty (fully transparent) source layer: the Content-Aware fill runs and the layer stays
/// empty; nothing panics on the all-default window.
#[test]
fn content_aware_patch_on_an_empty_layer_leaves_it_empty() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": 8, "mode": "rgb"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 30, "y": 20, "width": 20, "height": 20})).unwrap();
    let r = s.execute("paint.patch", json!({"offset": [30, 0], "contentAware": true})).unwrap();
    assert_eq!(r["contentAware"], json!(true));
    for y in [22i32, 30, 38] {
        for x in [32i32, 40, 47] {
            let d = s.active().unwrap();
            let px = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
            assert_eq!(px, [0.0; 4], "empty source stays empty at {x},{y}");
        }
    }
}
