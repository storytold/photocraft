//! #12531: the Paint Bucket's Mode › Behind.
//!
//! The issue's document (800 x 600, white Background, an opaque red square on the active layer
//! "Top", foreground green, Tolerance 32, Anti-alias and Contiguous on) is shrunk to 80 x 60 with
//! the square at 40..60 x 10..30. Measured: Behind changes nothing on the opaque square and fills
//! a transparent area exactly like Normal. Unmeasured cases (semi-transparent pixels, Opacity
//! below 100 %) follow the Brush's Behind: the fill goes under the pixel at
//! `fill α × coverage × (1 − α)`.

use super::*;
use photocraft_geom::Rect;

/// The shrunk test document at `depth`, with "Top" active.
fn document(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.edit("red square", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(40, 10, 60, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Paints `color` into `rect` of the active layer, outside history.
fn paint(s: &mut Session, rect: Rect, color: [f32; 4]) {
    s.edit("setup", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(rect, &color);
        Ok(())
    })
    .unwrap();
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds())
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// The pixel as 8-bit levels.
fn levels(s: &Session, x: i32, y: i32) -> [i32; 4] {
    rgba(s, x, y).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i32)
}

fn near(got: [i32; 4], want: [i32; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2)
}

fn bucket(x: i32, y: i32, mode: &str) -> Value {
    json!({"x": x, "y": y, "color": "#00ff00", "tolerance": 32, "antiAlias": true, "contiguous": true, "mode": mode})
}

#[test]
fn behind_on_the_opaque_square_changes_nothing() {
    for depth in [8, 16, 32] {
        let mut s = document(depth);
        let before = pixels(&s);
        s.execute("paint.bucket", bucket(50, 20, "behind")).unwrap_or_else(|e| panic!("depth {depth}: {e}"));
        assert!(pixels(&s) == before, "depth {depth}: the opaque square takes nothing");
    }
}

#[test]
fn behind_on_transparency_fills_exactly_like_normal_in_one_state() {
    for depth in [8, 16, 32] {
        let mut normal = document(depth);
        normal.execute("paint.bucket", bucket(5, 5, "normal")).unwrap();
        let mut behind = document(depth);
        let states = behind.active().unwrap().history.past_len();
        behind.execute("paint.bucket", bucket(5, 5, "behind")).unwrap_or_else(|e| panic!("depth {depth}: {e}"));
        assert!(pixels(&behind) == pixels(&normal), "depth {depth}: Behind on transparency is Normal");
        assert_eq!(levels(&behind, 5, 5), [0, 255, 0, 255], "depth {depth}");
        assert_eq!(levels(&behind, 50, 20), [255, 0, 0, 255], "depth {depth}: the square stays");
        let h = &behind.active().unwrap().history;
        assert_eq!(h.past_len(), states + 1, "depth {depth}: one state");
        assert_eq!(h.undo_label(), Some("Paint Bucket"), "depth {depth}");
    }
}

/// Not measured: semi-transparent pixels take the fill underneath, as the Brush's Behind lays it.
/// Black at alpha 0.5, green behind at Opacity 100 → (0, 0.5, 0, 1); at Opacity 50 → the fill's
/// share 0.5 × 0.5 = 0.25 under alpha 0.5: alpha 0.75, green 0.25 / 0.75.
#[test]
fn behind_goes_under_semi_transparent_pixels_like_the_brush() {
    for (opacity, want) in [(100, [0, 128, 0, 255]), (50, [0, 85, 0, 191])] {
        let mut s = document(8);
        paint(&mut s, Rect::new(10, 40, 20, 50), [0.0, 0.0, 0.0, 0.5]);
        let mut p = bucket(15, 45, "behind");
        p["opacity"] = json!(opacity);
        s.execute("paint.bucket", p).unwrap_or_else(|e| panic!("opacity {opacity}: {e}"));
        let got = levels(&s, 15, 45);
        assert!(near(got, want), "opacity {opacity}: {got:?}, want {want:?}");
        assert_eq!(levels(&s, 5, 5), [0, 0, 0, 0], "opacity {opacity}: the transparent area is not part of the region");
    }
}

/// #12528's rim with Behind (rule decided on the issue's own terms): a black alpha-40 edge pixel
/// next to a filled transparent area keeps the rim's coverage `1 − α` and takes the fill under
/// it: about (0, 209, 0, 221), the alpha Normal gives (0, 248, 0, 221), with the black on top.
#[test]
fn behind_lays_the_rim_under_an_edge_pixel() {
    for (mode, want) in [("behind", [0, 209, 0, 221]), ("normal", [0, 248, 0, 221])] {
        let mut s = document(8);
        paint(&mut s, Rect::new(10, 50, 11, 51), [0.0, 0.0, 0.0, 40.0 / 255.0]);
        s.execute("paint.bucket", bucket(5, 5, mode)).unwrap_or_else(|e| panic!("{mode}: {e}"));
        let got = levels(&s, 10, 50);
        assert!(near(got, want), "{mode}: edge {got:?}, want {want:?}");
    }
}
