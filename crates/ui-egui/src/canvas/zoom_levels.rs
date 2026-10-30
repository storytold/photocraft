//! The canvas zoom range and Photoshop's Zoom In / Zoom Out ladder, measured on Photoshop 25.4:
//!
//! - the zoom runs up to 12800 %, and down to where the document's shorter side is 2 px
//!   (4 × 4 px: 50 %; 2000 × 1500 px: 0.1333 %);
//! - Zoom In / Zoom Out (⌘+ / ⌘−) and a Zoom tool click step along [`LADDER`], from any zoom to
//!   the next step (880 %: in to 1200 %, out to 800 %); they go no lower than 0.1 %, or the
//!   document's minimum when that is higher;
//! - ⌥ + wheel multiplies the zoom by 1.1 a notch (`wheel_nav`), and a notch that would pass a
//!   limit stops exactly on it.
//!
//! Every way of setting the zoom (wheel, Zoom tool, menu, zoom fields, Navigator, control
//! channel) clamps through here, so no control can pull the zoom back to a narrower range of
//! its own.

use photocraft_doc::DocId;

/// 12800 %.
pub const MAX: f32 = 128.0;

/// Zoom In / Zoom Out steps (1 = 100 %).
pub const LADDER: [f32; 33] = [
    0.001,
    0.002,
    0.003,
    0.004,
    0.005,
    0.007,
    0.01,
    0.015,
    0.02,
    0.03,
    0.04,
    0.05,
    0.0625,
    1.0 / 12.0,
    0.125,
    1.0 / 6.0,
    0.25,
    1.0 / 3.0,
    0.5,
    2.0 / 3.0,
    1.0,
    2.0,
    3.0,
    4.0,
    5.0,
    6.0,
    7.0,
    8.0,
    12.0,
    16.0,
    32.0,
    64.0,
    128.0,
];

/// The lowest zoom for a document of `size` px: its shorter side 2 px on screen.
pub fn min(size: [u32; 2]) -> f32 {
    match size[0].min(size[1]) {
        // No size known yet (a view before its first frame).
        0 => LADDER[0],
        short => (2.0 / short as f32).min(MAX),
    }
}

/// `zoom` within the range for a document of `size` px (a non-finite zoom becomes 100 %).
pub fn clamp(zoom: f32, size: [u32; 2]) -> f32 {
    if zoom.is_finite() { zoom.min(MAX).max(min(size)) } else { 1.0 }
}

/// A zoom field's range in percent for `view`: the zoom range, and always the view's zoom (a
/// field narrower than the zoom would clamp it and write it back every frame, which once made
/// the wheel slide the image towards the pointer at 3200 %).
pub fn percent_range(view: &crate::state::View) -> std::ops::RangeInclusive<f32> {
    let z = if view.zoom.is_finite() { view.zoom } else { 1.0 };
    (min(view.doc_size).min(z) * 100.0)..=(MAX.max(z) * 100.0)
}

/// Zoom In (`dir` > 0) or Zoom Out (`dir` < 0) within an explicit range.
pub fn step_in_range(zoom: f32, dir: i32, lo: f32, hi: f32) -> f32 {
    let z = if zoom.is_finite() { zoom.clamp(lo, hi) } else { 1.0 };
    let next = if dir > 0 {
        LADDER.iter().copied().find(|s| *s > z * 1.001).unwrap_or(hi)
    } else {
        LADDER.iter().rev().copied().find(|s| *s < z * 0.999).unwrap_or(z)
    };
    if next.is_finite() { next.clamp(lo, hi) } else { z }
}

/// Zoom In (`dir` > 0) or Zoom Out (`dir` < 0) from `zoom`, for a document of `size` px.
pub fn step(zoom: f32, dir: i32, size: [u32; 2]) -> f32 {
    step_in_range(zoom, dir, min(size), MAX)
}

/// Where a document keeps the zoom it still has to travel (`ln(target / zoom)`) after a zoom was
/// typed into a zoom field. `canvas_view` spends it a little each frame, the way egui spends a wheel
/// notch into `smooth_scroll_delta` (90 % of the way in 0.2 s), instead of jumping.
///
/// It is keyed by the document's [`DocId`], not by its index in the views: `PhotocraftApp::sync_views`
/// moves views when documents are closed or reordered, and an index would hand a glide to another
/// document. Close a document with [`forget_pending`], so nothing is left that another could take.
pub fn pending_id(doc: DocId) -> egui::Id {
    egui::Id::unique(("pc-zoom-pending", doc))
}

/// Drops the glide a document still had to travel. Call it when the document closes.
pub fn forget_pending(ctx: &egui::Context, doc: DocId) {
    ctx.data_mut(|d| d.remove::<f32>(pending_id(doc)));
}

/// The percent a zoom field shows: the zoom a typed value is still gliding to, else the zoom. The
/// field then keeps the number that was typed for the whole glide, instead of counting through it.
pub fn field_percent(ctx: &egui::Context, doc: DocId, zoom: f32) -> f32 {
    let pending = ctx.data(|d| d.get_temp::<f32>(pending_id(doc))).filter(|p| p.is_finite()).unwrap_or(0.0);
    zoom * pending.exp() * 100.0
}

/// Applies the percent a zoom field reported as changed. A scrub (a drag on the field or its
/// label) follows the pointer at once; a typed or stepped value only leaves the distance to
/// travel, for `canvas_view` to spend.
pub fn set_from_field(ctx: &egui::Context, doc: DocId, view: &mut crate::state::View, scrub: bool, percent: f32) {
    let target = clamp(percent / 100.0, view.doc_size);
    view.fit_pending = false;
    // A zoom field sets the camera itself: no camera transition runs under it.
    view.zoom_animating = false;
    view.zoom_anchor = None;
    let pending = if scrub || view.zoom.is_nan() || view.zoom <= 0.0 {
        view.zoom = target;
        0.0
    } else {
        (target / view.zoom).ln()
    };
    ctx.data_mut(|d| d.insert_temp(pending_id(doc), pending));
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHOTO: [u32; 2] = [2000, 1500];

    fn pct(z: f32) -> f32 {
        (z * 100.0 * 1000.0).round() / 1000.0
    }

    #[test]
    fn the_ladder_is_photoshops() {
        // Photoshop 25.4, View › Zoom In from 100 %, then Zoom Out from 100 % (2000 × 1500 px).
        let mut z = 1.0;
        let mut up = Vec::new();
        while z < MAX {
            z = step(z, 1, PHOTO);
            up.push(pct(z));
        }
        assert_eq!(up, [200.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0, 1200.0, 1600.0, 3200.0, 6400.0, 12800.0]);
        assert_eq!(step(MAX, 1, PHOTO), MAX, "Zoom In stops at 12800 %");
        let mut z = 1.0;
        let mut down = Vec::new();
        for _ in 0..20 {
            z = step(z, -1, PHOTO);
            down.push(pct(z));
        }
        assert_eq!(
            down,
            [66.667, 50.0, 33.333, 25.0, 16.667, 12.5, 8.333, 6.25, 5.0, 4.0, 3.0, 2.0, 1.5, 1.0, 0.7, 0.5, 0.4, 0.3, 0.2, 0.133],
            "the last step is the document's minimum (1500 px tall: 2 / 1500)"
        );
        assert_eq!(step(z, -1, PHOTO), z, "Zoom Out stops at the minimum");
    }

    #[test]
    fn off_ladder_zooms_step_to_the_next_rung() {
        assert_eq!(step(8.8, 1, PHOTO), 12.0);
        assert_eq!(step(8.8, -1, PHOTO), 8.0);
        assert_eq!(step(1.331, 1, PHOTO), 2.0);
        assert_eq!(step(1.331, -1, PHOTO), 1.0);
    }

    #[test]
    fn the_minimum_follows_the_shorter_side() {
        assert_eq!(min([4, 4]), 0.5);
        assert_eq!(pct(min([300, 1000])), 0.667);
        assert_eq!(pct(min([1000, 300])), 0.667);
        // A huge document: the wheel goes below 0.1 %, Zoom Out stops there.
        let big = [12_000, 12_000];
        assert!(min(big) < 0.001);
        assert_eq!(step(0.0015, -1, big), 0.001);
        assert_eq!(step(0.001, -1, big), 0.001);
        assert_eq!(step(0.0007, -1, big), 0.0007, "below the ladder Zoom Out stays put");
        // A 1 px document can't go below 200 %.
        assert_eq!(min([1, 1]), 2.0);
        assert_eq!(step(2.0, -1, [1, 1]), 2.0);
        assert_eq!(min([0, 0]), 0.001, "unknown size");
    }

    #[test]
    fn step_in_range_respects_custom_bounds() {
        assert_eq!(step_in_range(1.0, 1, 0.0001, 16.0), 2.0);
        assert!((step_in_range(1.0, -1, 0.0001, 16.0) - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(step_in_range(16.0, 1, 0.0001, 16.0), 16.0, "at max stays put");
        assert_eq!(step_in_range(0.0001, -1, 0.0001, 16.0), 0.0001, "at min stays put");
        assert_eq!(step_in_range(f32::NAN, 1, 0.0001, 16.0), 2.0, "NaN → 100% → step");
        assert_eq!(step_in_range(8.8, 1, 0.0001, 16.0), 12.0, "off-ladder steps up");
        assert_eq!(step_in_range(8.8, -1, 0.0001, 16.0), 8.0, "off-ladder steps down");
        assert_eq!(step(1.0, 1, PHOTO), step_in_range(1.0, 1, min(PHOTO), MAX), "step delegates");
    }

    #[test]
    fn clamp_keeps_the_range_and_survives_bad_input() {
        assert_eq!(clamp(500.0, PHOTO), MAX);
        assert_eq!(clamp(0.0, PHOTO), min(PHOTO));
        assert_eq!(clamp(f32::NAN, PHOTO), 1.0);
        assert_eq!(clamp(f32::INFINITY, PHOTO), 1.0);
        assert_eq!(clamp(3.3, PHOTO), 3.3);
    }

    #[test]
    fn a_typed_zoom_waits_for_the_canvas_and_a_scrub_does_not() {
        let ctx = egui::Context::default();
        let pending = |doc| ctx.data(|d| d.get_temp::<f32>(pending_id(doc)));
        let mut v = crate::state::View { zoom: 0.5, doc_size: PHOTO, fit_pending: true, ..Default::default() };
        set_from_field(&ctx, DocId(0), &mut v, false, 150.0);
        assert_eq!(v.zoom, 0.5, "typing does not move the zoom itself");
        assert!(!v.fit_pending);
        assert!((pending(DocId(0)).unwrap() - 3.0_f32.ln()).abs() < 1e-6, "the distance left is ln(150 / 50)");
        set_from_field(&ctx, DocId(0), &mut v, true, 200.0);
        assert_eq!(v.zoom, 2.0, "a scrub applies at once");
        assert_eq!(pending(DocId(0)), Some(0.0), "and drops what was left");
        set_from_field(&ctx, DocId(1), &mut v, false, 1.0e9);
        assert!((pending(DocId(1)).unwrap() - (MAX / 2.0).ln()).abs() < 1e-5, "the target is clamped to the zoom range first");
    }

    /// The distance a typed zoom leaves is `ln(target / zoom)`, out or in, for a tiny step and a
    /// huge one, and nothing for the zoom it already has.
    #[test]
    fn a_typed_zoom_leaves_the_log_distance_to_the_clamped_target() {
        let ctx = egui::Context::default();
        // (zoom now, percent typed)
        let cases = [(2.0_f32, 25.0_f32), (0.1, 800.0), (1.0, 100.5), (0.37, 1234.0), (1.0, 100.0), (1.0, 1.0e9)];
        for (i, (zoom, percent)) in cases.into_iter().enumerate() {
            let doc = DocId(i as _);
            let mut v = crate::state::View { zoom, doc_size: PHOTO, ..Default::default() };
            set_from_field(&ctx, doc, &mut v, false, percent);
            let left = ctx.data(|d| d.get_temp::<f32>(pending_id(doc))).expect("a distance is stored");
            let want = (clamp(percent / 100.0, PHOTO) / zoom).ln();
            assert!((left - want).abs() < 1e-5, "{percent} % over {zoom}: {left} vs {want}");
            assert_eq!(v.zoom, zoom, "typing never moves the zoom itself");
        }
    }

    #[test]
    fn a_zoom_field_shows_the_zoom_it_is_gliding_to() {
        let ctx = egui::Context::default();
        let mut v = crate::state::View { zoom: 2.0, doc_size: PHOTO, ..Default::default() };
        assert!((field_percent(&ctx, DocId(0), v.zoom) - 200.0).abs() < 1e-3, "no glide: the zoom");
        set_from_field(&ctx, DocId(0), &mut v, false, 100.0);
        assert!((field_percent(&ctx, DocId(0), v.zoom) - 100.0).abs() < 1e-3, "typed 100, zoom still 200 %");
        // Halfway through the glide, the way `canvas_view` spends the distance.
        let pending = ctx.data(|d| d.get_temp::<f32>(pending_id(DocId(0)))).unwrap();
        v.zoom *= (pending / 2.0).exp();
        ctx.data_mut(|d| d.insert_temp(pending_id(DocId(0)), pending / 2.0));
        let shown = field_percent(&ctx, DocId(0), v.zoom);
        assert!((shown - 100.0).abs() < 1e-3, "still 100 half way: {shown}");
    }

    /// The glide belongs to the document, not to the place its view has in the list: when views move
    /// (`sync_views` after a close), each document keeps its own target, and a closed one leaves none.
    #[test]
    fn a_glide_follows_its_document_and_goes_when_it_closes() {
        let ctx = egui::Context::default();
        let (a, b) = (DocId(10), DocId(20));
        let mut va = crate::state::View { zoom: 1.0, doc_size: PHOTO, ..Default::default() };
        let mut vb = crate::state::View { zoom: 1.0, doc_size: PHOTO, ..Default::default() };
        set_from_field(&ctx, a, &mut va, false, 200.0);
        set_from_field(&ctx, b, &mut vb, false, 50.0);
        // `a` closes, `b` moves to `a`'s old index: nothing about `a` may reach it.
        forget_pending(&ctx, a);
        assert_eq!(ctx.data(|d| d.get_temp::<f32>(pending_id(a))), None, "a closed document leaves no glide");
        assert!((field_percent(&ctx, b, vb.zoom) - 50.0).abs() < 1e-3, "b keeps its own target");
        assert!((field_percent(&ctx, a, 1.0) - 100.0).abs() < 1e-3, "nothing pending for a: just the zoom");
        // A new document that happens to take a's old position starts with no glide.
        let c = DocId(30);
        assert!((field_percent(&ctx, c, 1.0) - 100.0).abs() < 1e-3);
    }
}
