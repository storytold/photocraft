//! Whole-raster-layer global rectification. Independent guides constrain ONE matrix.
//! A selection, masks or smart objects are explicitly rejected rather than silently processed
//! as image patches. Plain photographic raster layers (including Background) are supported.

use photocraft_algo::global_perspective::{Guide, Options, Solution, solve};
use photocraft_algo::transform::{Interp, warp_surface};
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
        return Err("Global Perspective Warp V3 requires a raster photo; use a rasterized copy".into());
    };
    if layer.mask.is_some() || layer.vector_mask.is_some() {
        return Err("Global Perspective Warp V3 requires an unmasked raster photo; use a flattened copy".into());
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
    if params.solve_only || h.is_identity() {
        return Ok(json!({"document":doc_id,"layer":id.0,"changed":false,"solved":true,"scope":"wholeRasterLayer","solution":solution}));
    }
    let surf = st.doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad("no raster pixels"))?;
    // All validation and allocation happen before the transaction. A failure leaves history
    // and the document unchanged. The inverse homography renderer has no mesh seams.
    let out = warp_surface(surf, bounds, &h, params.interpolation)?;
    s.edit("Global Perspective Warp V3", move |doc, _| {
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
        assert!(s.execute(COMMAND, params()).is_err());
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
