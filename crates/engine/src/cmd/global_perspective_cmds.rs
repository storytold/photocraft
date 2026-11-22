//! Whole-raster-layer global rectification. Independent guides constrain ONE matrix.
//! A selection, masks or smart objects are explicitly rejected rather than silently processed
//! as image patches. Plain photographic raster layers (including Background) are supported.

use photocraft_algo::global_perspective::{Guide, MAX_OUTPUT_PIXELS, Options, Solution, solve};
use photocraft_algo::transform::{Homography, Interp, warp_rendered_output_size, warp_surface};
use photocraft_color::PixelFormat;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_geom::Rect;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const COMMAND: &str = "edit.perspectiveWarpGlobal";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    guides: Vec<Guide>,
    #[serde(default)]
    options: Options,
    #[serde(default)]
    layer: Option<u64>,
    #[serde(default)]
    document: Option<u64>,
    #[serde(default)]
    solve_only: bool,
    #[serde(default)]
    interpolation: Interp,
}

/// Shared GUI/engine eligibility. Do not silently lift selected pixels or discard masks.
pub fn validate_target(s: &Session, id: LayerId) -> std::result::Result<Rect, String> {
    let st = s.active().ok_or("no document open")?;
    if st.doc.selection.is_some() {
        return Err("Global Perspective Warp transforms the whole photo: deselect first".into());
    }
    let layer = st.doc.layer(id).ok_or("no target layer")?;
    let LayerContent::Raster(surface) = &layer.content else {
        return Err("Global Perspective Warp requires a raster photo; use a rasterized copy".into());
    };
    if layer.mask.is_some() || layer.vector_mask.is_some() {
        return Err("Global Perspective Warp requires an unmasked raster photo; use a flattened copy".into());
    }
    let inherited = crate::transform_cmds::group_locks(&st.doc, id);
    let locks = layer.locks.union(inherited);
    if locks.all || locks.pixels || (locks.position && !st.doc.is_background(id)) {
        return Err("the target layer or its parent group is locked".into());
    }
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err("the target photo has no pixels".into());
    }
    if bounds.width() > 32768 || bounds.height() > 32768 {
        return Err("Global Perspective Warp supports at most 32768 pixels per image side".into());
    }
    Ok(bounds)
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    let id = s.active().and_then(|d| d.active_layer).ok_or("no active layer")?;
    validate_target(s, id).map(|_| ())
}

/// Price exactly the renderer's post-Bicubic-reduction output rectangle before allocating tiles.
/// The solver's original-domain budget remains a separate geometry check.
fn validate_render_budget(bounds: Rect, h: &Homography, interp: Interp, format: PixelFormat, budget: u64) -> Result<()> {
    let (width, height) = warp_rendered_output_size(bounds, h, interp).ok_or_else(|| bad("invalid global perspective render bounds"))?;
    if width.saturating_mul(height) > MAX_OUTPUT_PIXELS as u64 {
        return Err(bad("global rectification output exceeds the 64-megapixel safety budget"));
    }
    crate::transform_cmds::check_output_budget((width, height), "global perspective layer", format, budget)
}

/// Only skip sub-pixel roundoff on the validated image domain. Projective coefficients
/// alone cannot bound displacement: their effect grows with document coordinates.
fn is_pixel_no_op(bounds: Rect, h: &Homography) -> bool {
    const TOLERANCE_PX: f64 = 0.01;
    let (x0, y0, x1, y1) = (bounds.x0 as f64, bounds.y0 as f64, bounds.x1 as f64, bounds.y1 as f64);
    [x0, (x0 + x1) / 2.0, x1].into_iter().all(|x| {
        [y0, (y0 + y1) / 2.0, y1].into_iter().all(|y| {
            let (u, v) = h.apply(x, y);
            let displacement = (u - x).hypot(v - y);
            displacement.is_finite() && displacement <= TOLERANCE_PX
        })
    })
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("guides").and_then(Value::as_array).is_some_and(|a| a.len() > photocraft_algo::global_perspective::MAX_GUIDES) {
        return Err(bad("too many guides (maximum 128)"));
    }
    let params: Params = serde_json::from_value(p.clone()).map_err(|e| bad(format!("invalid global perspective parameters: {e}")))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if params.document.is_some_and(|id| id != st.doc.id.0) {
        return Err(bad("global perspective belongs to another document"));
    }
    let id = params.layer.map(LayerId).or(st.active_layer).ok_or_else(|| bad("no target layer"))?;
    let bounds = validate_target(s, id).map_err(bad)?;
    let solution: Solution = solve(&params.guides, bounds, &params.options).map_err(bad)?;
    let doc_id = st.doc.id.0;
    let h = solution.homography();
    let no_op = is_pixel_no_op(bounds, &h);
    let surf = st.doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad("no raster pixels"))?;
    // solveOnly must report the same pixel and memory eligibility as a real render.
    // Check the post-reduction bounds before warp_surface can allocate destination tiles.
    if !no_op {
        validate_render_budget(bounds, &h, params.interpolation, surf.format(), crate::transform_cmds::active_budget())?;
    }
    if params.solve_only || no_op {
        return Ok(json!({"document":doc_id,"layer":id.0,"changed":false,"solved":true,"scope":"wholeRasterLayer","solution":solution}));
    }
    // All validation and allocation happen before the transaction. A failure leaves history
    // and the document unchanged. The inverse homography renderer has no mesh seams.
    let out = warp_surface(surf, bounds, &h, params.interpolation)?;
    s.edit("Global Perspective Warp", move |doc, _| {
        crate::extra_cmds::background_to_layer(doc, id);
        let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let LayerContent::Raster(surface) = &mut layer.content else {
            return Err(bad("target is no longer a raster layer"));
        };
        *surface = out;
        Ok(())
    })?;
    Ok(json!({"document":doc_id,"layer":id.0,"changed":true,"solved":true,"scope":"wholeRasterLayer","solution":solution}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Global Perspective Warp",
        menu: &[],
        shortcut: None,
        params: r#"{"guides":[{"id":integer,"corners":[[x,y]x4],"enabled":true,"weight":1}...],"options":{"fitCanvas":true,"aspect":1},"layer":id?,"document":id?,"solveOnly":false,"interpolation":"bicubic|bilinear|nearest"} -- one global homography for the WHOLE raster layer, not per-guide regions. Guide corners TL TR BR BL, up to 128; weight 0.05..20; aspect 0.25..4. Rejects selections, masks and smart objects. solveOnly returns diagnostics without changing the document."#,
        enabled,
        journal: true,
        run,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::Surface;
    fn session(depth: u64) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":160,"height":140,"depth":depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("fixture", |doc, id| {
            let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 160, 140), &[0.25, 0.5, 0.75, 1.0]);
            for y in 0..140 {
                surf.write_pixel(80, y, &[1.0, 0.0, 0.0, 1.0]);
            }
            Ok(())
        })
        .unwrap();
        s
    }
    fn surf(s: &Session) -> Surface {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().clone()
    }
    fn params() -> Value {
        json!({"guides":[{"id":1,"corners":[[25,20],[100,16],[109,105],[21,110]]}]})
    }

    /// The original-domain solver budget admits exactly 8192² pixels, but
    /// Bicubic's outward-rounded half-size source has an 8194² render footprint.
    /// Validate this geometry without ever allocating a 64-megapixel raster.
    #[test]
    fn bicubic_downscale_budget_matches_actual_renderer_geometry() {
        let bounds = Rect::new(1, 1, 8191, 32761);
        let h = Homography([1.0, 0.0, 0.0, 0.0, 0.25, 12285.75, 0.0, 0.0, 1.0]);
        assert_eq!(photocraft_algo::transform::warp_output_size(bounds, &h), Some((8192, 8192)));
        assert_eq!(warp_rendered_output_size(bounds, &h, Interp::Bicubic), Some((8194, 8194)));
        assert_eq!(8192_u64 * 8192_u64, MAX_OUTPUT_PIXELS as u64);
        assert_eq!(8194_u64 * 8194_u64 - MAX_OUTPUT_PIXELS as u64, 32_772);
        let budget = MAX_OUTPUT_PIXELS as u64 * PixelFormat::RGBA8.bytes_per_pixel() as u64;
        let error = validate_render_budget(bounds, &h, Interp::Bicubic, PixelFormat::RGBA8, budget).unwrap_err().to_string();
        assert!(error.contains("64-megapixel"), "{error}");
        // Bilinear does not pre-reduce, so its actual footprint stays within budget.
        assert!(validate_render_budget(bounds, &h, Interp::Bilinear, PixelFormat::RGBA8, budget).is_ok());
        assert!(validate_render_budget(Rect::new(0, 0, 160, 140), &h, Interp::Bicubic, PixelFormat::RGBA8, budget).is_ok());
    }

    #[test]
    fn bicubic_memory_budget_prices_the_reduced_render_footprint() {
        let bounds = Rect::new(1, 1, 1023, 4089);
        let h = Homography([1.0, 0.0, 0.0, 0.0, 0.25, 1533.75, 0.0, 0.0, 1.0]);
        let (w, hgt) = photocraft_algo::transform::warp_output_size(bounds, &h).unwrap();
        let (rw, rh) = warp_rendered_output_size(bounds, &h, Interp::Bicubic).unwrap();
        assert!(rw * rh > w * hgt);
        assert!(rw * rh < MAX_OUTPUT_PIXELS as u64);
        let format = PixelFormat::RGBA32F;
        let budget = w * hgt * format.bytes_per_pixel() as u64;
        assert!(validate_render_budget(bounds, &h, Interp::Bilinear, format, budget).is_ok());
        let error = validate_render_budget(bounds, &h, Interp::Bicubic, format, budget).unwrap_err().to_string();
        assert!(error.contains("Memory Usage"), "{error}");
        // Raising only the byte limit admits the very same geometry.
        let render_bytes = rw * rh * format.bytes_per_pixel() as u64;
        assert!(validate_render_budget(bounds, &h, Interp::Bicubic, format, render_bytes).is_ok());
    }

    /// Two painted pixels bound a large output without allocating a large source raster.
    fn sparse_session(mode: &str, depth: u32, alpha: bool) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":16,"height":16,"mode":mode,"depth":depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("sparse fixture", |doc, id| {
            let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            let format = PixelFormat::new(surface.format().mode, surface.format().sample, alpha);
            *surface = Surface::new(format);
            let pixel = vec![0.5; format.channels()];
            surface.write_pixel(0, 0, &pixel);
            surface.write_pixel(1199, 1199, &pixel);
            Ok(())
        })
        .unwrap();
        s
    }

    fn rectangular_guides() -> Value {
        json!({"guides":[{"id":1,"corners":[[20,20],[100,20],[100,120],[20,120]]}]})
    }

    #[test]
    fn global_memory_admission_uses_output_format_and_preserves_history() {
        // Unit tests use the shared fixed 20 MiB budget, independent of set_budget.
        let budget = crate::transform_cmds::active_budget();
        for (mode, depth, alpha, refused) in
            [("rgb", 16, true, false), ("gray", 32, false, false), ("rgb", 32, true, true), ("rgb", 32, false, true), ("cmyk", 32, true, true)]
        {
            let mut s = sparse_session(mode, depth, alpha);
            s.edit("redo sentinel", |doc, id| {
                let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                surface.write_pixel(600, 600, &vec![0.75; surface.format().channels()]);
                Ok(())
            })
            .unwrap();
            let redo_doc = s.active().unwrap().doc.clone();
            assert!(s.undo());
            let before_pixels = surf(&s);
            let st = s.active().unwrap();
            let (before, revision, steps) = (st.doc.clone(), st.revision, st.history.past_len());
            assert!(st.history.can_redo());
            let mut p = rectangular_guides();
            p["options"] = json!({"fitCanvas":false,"aspect":4});
            let parsed: Params = serde_json::from_value(p.clone()).unwrap();
            let bounds = before_pixels.content_bounds();
            let h = solve(&parsed.guides, bounds, &parsed.options).unwrap().homography();
            assert!(!is_pixel_no_op(bounds, &h));
            let format = before_pixels.format();
            for interpolation in ["nearest", "bilinear", "bicubic"] {
                p["interpolation"] = json!(interpolation);
                let interp: Interp = serde_json::from_value(p["interpolation"].clone()).unwrap();
                let (w, hgt) = warp_rendered_output_size(bounds, &h, interp).unwrap();
                let pixels = w * hgt;
                assert!(pixels < MAX_OUTPUT_PIXELS as u64);
                let output_format = PixelFormat::new(format.mode, format.sample, true);
                assert_eq!(pixels * output_format.bytes_per_pixel() as u64 > budget, refused);
                if mode == "rgb" && !alpha {
                    assert!(pixels * (format.bytes_per_pixel() as u64) < budget, "only the added output alpha takes this over budget");
                }
                for solve_only in [true, false] {
                    // Exercise accepted preflight without rendering a dense passing control.
                    if !refused && !solve_only {
                        continue;
                    }
                    p["solveOnly"] = json!(solve_only);
                    let result = s.execute(COMMAND, p.clone());
                    if refused {
                        let error = result.unwrap_err().to_string();
                        assert!(error.contains("Memory Usage"), "{mode}/{depth}/{alpha}/{interpolation}/{solve_only}: {error}");
                    } else {
                        assert_eq!(result.unwrap()["changed"], false);
                    }
                    let st = s.active().unwrap();
                    assert!(st.doc == before, "memory admission changed the document");
                    assert_eq!(surf(&s), before_pixels);
                    assert_eq!((st.revision, st.history.past_len()), (revision, steps));
                    assert!(st.history.can_redo());
                }
            }
            assert!(s.redo());
            assert!(s.active().unwrap().doc == redo_doc, "memory admission changed the redo state");
        }
    }

    #[test]
    fn identity_above_memory_budget_remains_a_no_op() {
        let mut s = sparse_session("rgb", 32, true);
        let surface = surf(&s);
        let (w, h) = warp_rendered_output_size(surface.content_bounds(), &Homography::IDENTITY, Interp::Bicubic).unwrap();
        assert!(w * h * surface.format().bytes_per_pixel() as u64 > crate::transform_cmds::active_budget());
        let st = s.active().unwrap();
        let (before, revision, steps) = (st.doc.clone(), st.revision, st.history.past_len());
        for solve_only in [false, true] {
            let mut p = rectangular_guides();
            p["solveOnly"] = json!(solve_only);
            assert_eq!(s.execute(COMMAND, p).unwrap()["changed"], false);
            let st = s.active().unwrap();
            assert!(st.doc == before);
            assert_eq!((st.revision, st.history.past_len()), (revision, steps));
        }
    }

    #[test]
    fn projective_no_op_is_measured_in_document_pixels() {
        let bounds = Rect::new(0, 0, 32768, 1024);
        let h = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5e-10, 0.0, 1.0]);
        assert!(h.is_identity(), "reproduce the old coefficient-based false no-op");
        let p = h.apply(32768.0, 1024.0);
        assert!((p.0 - 32768.0).hypot(p.1 - 1024.0) > 0.2);
        assert!(!is_pixel_no_op(bounds, &h));
        assert!(is_pixel_no_op(bounds, &Homography::IDENTITY));
        let roundoff = Homography([1.0, 0.0, 1e-12, 0.0, 1.0, -1e-12, 1e-15, 0.0, 1.0]);
        assert!(is_pixel_no_op(bounds, &roundoff));
        // A large translated domain must also use document coordinates, not just its size.
        assert!(!is_pixel_no_op(Rect::new(900000, 0, 901024, 1024), &h));

        // Endpoints alone miss projective displacement along a long, thin image.
        let thin = Rect::new(0, 0, 32768, 1);
        let curved = Homography([1.0 + 1e-7 * 32768.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1e-7, 0.0, 1.0]);
        for (x, y) in [(0.0, 0.0), (32768.0, 0.0), (32768.0, 1.0), (0.0, 1.0)] {
            let (u, v) = curved.apply(x, y);
            assert!((u - x).hypot(v - y) < 0.01);
        }
        assert!((curved.apply(16384.0, 0.5).0 - 16384.0).abs() > 20.0);
        assert!(!is_pixel_no_op(thin, &curved));
    }

    #[test]
    fn headless_global_command_is_registered_with_product_wording() {
        let spec = crate::commands::command_specs().iter().find(|c| c.id == COMMAND).unwrap();
        assert_eq!(spec.label, "Global Perspective Warp");
        assert!(spec.menu.is_empty(), "global rectification stays headless");
        assert!(spec.params.contains("WHOLE raster layer"));
        let s = session(8);
        assert!((spec.enabled)(&s).is_ok());
    }

    #[test]
    fn legacy_then_global_warp_share_atomic_undo_redo_history() {
        let mut s = session(8);
        let original = surf(&s);
        let steps = s.active().unwrap().history.past_len();
        // One convex plane covering the raster, translated without folds or a horizon.
        let src = [[0.0, 0.0], [160.0, 0.0], [160.0, 140.0], [0.0, 140.0]];
        let dst = src.map(|p| [p[0] + 5.0, p[1] + 3.0]);
        let result = s.execute(crate::distort_cmds::PERSPECTIVE, json!({"planes":[{"src":src,"dst":dst}],"interpolation":"bilinear"})).unwrap();
        assert_eq!(result["changed"], true);
        let legacy = surf(&s);
        assert_ne!(legacy, original, "legacy warp must change the raster");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);

        assert_eq!(s.execute(COMMAND, params()).unwrap()["changed"], true);
        let global = surf(&s);
        assert_ne!(global, legacy, "global warp must change the legacy result");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 2);
        assert!(!s.active().unwrap().history.can_redo());

        assert!(s.undo());
        assert_eq!(surf(&s), legacy, "undo global restores the exact legacy raster");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.undo());
        assert_eq!(surf(&s), original, "undo legacy restores the exact original raster");
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!(s.active().unwrap().history.can_redo());

        assert!(s.redo());
        assert_eq!(surf(&s), legacy);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo());
        assert_eq!(surf(&s), global);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 2);
        assert!(!s.active().unwrap().history.can_redo());
    }

    #[test]
    fn global_command_depths_single_history_undo_redo() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            let before = surf(&s);
            let n = s.active().unwrap().history.past_len();
            let r = s.execute(COMMAND, params()).unwrap();
            assert_eq!(r["changed"], true);
            assert_eq!(r["solution"]["model"], "singleGlobalHomography");
            assert_eq!(s.active().unwrap().history.past_len(), n + 1);
            let after = surf(&s);
            assert_ne!(before, after);
            assert_eq!(after.format().sample, before.format().sample);
            s.undo();
            assert_eq!(surf(&s), before);
            s.redo();
            assert_eq!(surf(&s), after);
        }
    }
    #[test]
    fn solve_only_and_errors_never_change_document_or_history() {
        let mut s = session(8);
        let before = surf(&s);
        let n = s.active().unwrap().history.past_len();
        let mut p = params();
        p["solveOnly"] = json!(true);
        assert_eq!(s.execute(COMMAND, p).unwrap()["changed"], false);
        for patch in [
            json!({"typo":1}),
            json!({"document":u64::MAX}),
            json!({"layer":"wrong"}),
            json!({"solveOnly":"yes"}),
            json!({"options":{"aspect":0}}),
            json!({"guides":[]}),
        ] {
            let mut p = params();
            for (k, v) in patch.as_object().unwrap() {
                p[k] = v.clone();
            }
            assert!(s.execute(COMMAND, p).is_err());
            assert_eq!(surf(&s), before);
            assert_eq!(s.active().unwrap().history.past_len(), n);
        }
    }
    #[test]
    fn selections_are_rejected_instead_of_warped_as_regions() {
        let mut s = session(8);
        s.execute("select.all", json!({})).unwrap();
        let before = surf(&s);
        let error = s.execute(COMMAND, params()).unwrap_err().to_string();
        assert!(error.contains("Global Perspective Warp"), "unexpected rejection: {error}");
        assert!(!error.contains("V3"), "internal version leaked into the user-facing error: {error}");
        assert_eq!(surf(&s), before);
    }
    #[test]
    fn identity_is_not_an_extra_history_step() {
        let mut s = session(8);
        let n = s.active().unwrap().history.past_len();
        let r = s.execute(COMMAND, json!({"guides":[{"id":1,"corners":[[20,20],[100,20],[100,120],[20,120]]}]})).unwrap();
        assert_eq!(r["changed"], false);
        assert_eq!(s.active().unwrap().history.past_len(), n);
    }
    #[test]
    fn global_matrix_has_no_interior_alpha_holes() {
        let mut s = session(8);
        let r = s.execute(COMMAND, params()).unwrap();
        let out = surf(&s);
        let a: Vec<f64> = r["solution"]["matrix"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let h = photocraft_algo::transform::Homography(a.try_into().unwrap());
        let inv = h.inverse().unwrap();
        for y in 0..140 {
            for x in 0..160 {
                let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
                if (3.0..157.0).contains(&u) && (3.0..137.0).contains(&v) {
                    assert!(out.pixel(x, y)[3] > 0.99, "hole at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn background_conversion_is_atomic_and_undoable() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":160,"height":140,"background":"white"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        assert!(s.active().unwrap().doc.is_background(id));
        let before = surf(&s);
        assert_eq!(s.execute(COMMAND, params()).unwrap()["changed"], true);
        assert!(!s.active().unwrap().doc.is_background(id));
        s.undo();
        assert!(s.active().unwrap().doc.is_background(id));
        assert_eq!(surf(&s), before);
    }
    #[test]
    fn pixel_locks_are_never_bypassed() {
        let mut s = session(8);
        s.edit("lock", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().locks.pixels = true;
            Ok(())
        })
        .unwrap();
        let before = surf(&s);
        assert!(s.execute(COMMAND, params()).is_err());
        assert_eq!(surf(&s), before);
    }
    #[test]
    fn mask_is_rejected_without_silent_loss() {
        let mut s = session(8);
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let before = surf(&s);
        assert!(s.execute(COMMAND, params()).is_err());
        assert_eq!(surf(&s), before);
        let st = s.active().unwrap();
        assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().mask.is_some());
    }
    #[test]
    fn global_raster_is_not_rgb_specific() {
        for mode in ["gray", "cmyk", "lab"] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":160,"height":140,"depth":16,"mode":mode})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            s.edit("fixture", |doc, id| {
                let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                let n = surface.format().channels();
                let mut p = vec![0.4; n];
                if surface.format().alpha {
                    p[n - 1] = 1.0;
                }
                surface.fill_rect(Rect::new(0, 0, 160, 140), &p);
                Ok(())
            })
            .unwrap();
            let before = surf(&s).format();
            s.execute(COMMAND, params()).unwrap();
            let after = surf(&s).format();
            assert_eq!(before.mode, after.mode);
            assert_eq!(before.sample, after.sample);
        }
    }
}
