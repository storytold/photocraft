//! Native-brush tangent wrapping. Geometry provides the chart; the shared paint
//! renderer retains flow, stroke opacity, native formats, blend modes and masks.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::Rect;
use photocraft_paint::{BrushContext, BrushSettings, Stroke, StrokePoint, StrokeRenderer};
use photocraft_raster::Surface;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

const MAX_BATCH_STAMPS: usize = 256;

#[derive(Clone, Copy)]
struct SurfaceStamp {
    triangle: usize,
    position: [f64; 3],
    pressure: f32,
}

pub(crate) struct WrappedStroke {
    doc: DocId,
    layer: Option<LayerId>,
    revision: u64,
    key: String,
    pre: Surface,
    brush: BrushSettings,
    renderer: StrokeRenderer,
    previous: Option<[f64; 3]>,
    pub(crate) pixels_per_unit: Option<f64>,
    index: u64,
    target: Value,
}
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if s.uv.scene.is_none() {
        Err("Import a model first".into())
    } else if s.active().is_none() {
        Err("Assign an editable texture first".into())
    } else {
        Ok(())
    }
}
fn bilinear(data: &[f32], rect: Rect, p: [f64; 2]) -> f32 {
    let (x, y) = (p[0] - f64::from(rect.x0) - 0.5, p[1] - f64::from(rect.y0) - 0.5);
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = ((x - ix as f64) as f32, (y - iy as f64) as f32);
    let value = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= i64::from(rect.width()) || y >= i64::from(rect.height()) {
            0.0
        } else {
            data.get(y as usize * rect.width() as usize + x as usize).copied().unwrap_or(0.0)
        }
    };
    ((value(ix, iy) * (1.0 - fx) + value(ix + 1, iy) * fx) * (1.0 - fy) + (value(ix, iy + 1) * (1.0 - fx) + value(ix + 1, iy + 1) * fx) * fy).clamp(0.0, 1.0)
}

fn parse_stamp(value: &Value) -> Result<SurfaceStamp> {
    let triangle = value.get("triangle").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("triangle is required"))?;
    let position: [f64; 3] = serde_json::from_value(value.get("position").cloned().ok_or_else(|| bad("position is required"))?)
        .map_err(|_| bad("position must contain three finite numbers"))?;
    if position.iter().any(|v| !v.is_finite() || v.abs() > 1e15) {
        return Err(bad("Invalid surface position"));
    }
    let pressure = value
        .get("pressure")
        .map_or(Some(1.0), Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .ok_or_else(|| bad("pressure must be in 0..1"))? as f32;
    Ok(SurfaceStamp { triangle, position, pressure })
}

fn parse_stamps(p: &Value) -> Result<Vec<SurfaceStamp>> {
    if let Some(value) = p.get("stamps") {
        let values = value.as_array().ok_or_else(|| bad("stamps must be an array"))?;
        if values.is_empty() || values.len() > MAX_BATCH_STAMPS {
            return Err(bad(format!("stamps must contain 1..={MAX_BATCH_STAMPS} surface samples")));
        }
        values.iter().map(parse_stamp).collect()
    } else {
        Ok(vec![parse_stamp(p)?])
    }
}

/// Add one surface sample to the accumulated wrapped renderer. `true` means the
/// legacy single-sample path would have proceeded to composite its accumulated stroke.
fn accumulate_stamp(
    s: &mut Session,
    scene: &photocraft_uv::Scene,
    mesh: &photocraft_uv::wrap::Mesh,
    size: [u32; 2],
    material: usize,
    stroke: &mut WrappedStroke,
    sample: SurfaceStamp,
) -> Result<bool> {
    let SurfaceStamp { triangle, position, pressure } = sample;
    if let (Some(prev), Some(scale)) = (stroke.previous, stroke.pixels_per_unit) {
        let travelled = prev.iter().zip(position).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        if travelled * scale < f64::from(stroke.brush.size * stroke.brush.spacing).max(0.5) {
            return Ok(false);
        }
    }
    let mut brush = stroke.brush.clone();
    brush.seed = brush.seed.wrapping_add(stroke.index);
    let dabs = photocraft_paint::dabs(&Stroke { brush: brush.clone(), points: vec![StrokePoint::new(0.0, 0.0, pressure)] });
    let Some(dab) = dabs.first() else { return Ok(false) };
    let ctx = BrushContext::new(&brush);
    let rect = ctx.dab_rect(dab, false).union(&ctx.dab_rect(dab, true));
    let count = u64::from(rect.width()) * u64::from(rect.height());
    if count > 4_194_304 {
        return Err(bad("Wrap stamp exceeds 4M pixels; reduce brush size"));
    }
    let reach = [rect.x0, rect.y0, rect.x1, rect.y1].into_iter().map(|v| f64::from(v).abs()).fold(0.0, f64::max) + 1.0;
    let patches = mesh.chart_scaled(scene, triangle, position, size, reach, stroke.pixels_per_unit).map_err(|e| bad(e.to_string()))?;
    if let Some(root) = patches.first() {
        let t = scene.triangles.get(root.triangle).ok_or_else(|| bad("Missing triangle"))?;
        let (Some(a), Some(b)) = (t.corners.first().and_then(|c| scene.positions.get(c.vertex)), t.corners.get(1).and_then(|c| scene.positions.get(c.vertex)))
        else {
            return Err(bad("Missing wrap edge"));
        };
        let distance = a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        let pixels = (root.plane[0][0] - root.plane[1][0]).hypot(root.plane[0][1] - root.plane[1][1]);
        if distance > 1e-14 {
            stroke.pixels_per_unit = Some(pixels / distance);
        }
    }
    if let (Some(prev), Some(scale)) = (stroke.previous, stroke.pixels_per_unit) {
        let travelled = prev.iter().zip(position).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        if travelled * scale < f64::from(brush.size * brush.spacing).max(0.5) {
            return Ok(false);
        }
    }
    let (mut cov, mut depth, mut dual) = (Vec::new(), Vec::new(), Vec::new());
    ctx.rasterize_into(dab, false, rect, &mut cov, Some(&mut depth));
    ctx.rasterize(dab, true, rect, &mut dual);
    if depth.len() != cov.len() {
        depth.resize(cov.len(), 0.0);
    }
    // Duplicate UV coverage is maxed within a dab. Keep separate dabs separate so
    // brush flow and opacity continue accumulating in order.
    let mut tiles = BTreeMap::<(i32, i32), Vec<[f32; 3]>>::new();
    let mut visited = 0usize;
    let mut over_budget = false;
    for patch in patches {
        let topology = s.uv.topology.as_ref().cloned().ok_or_else(|| bad("Missing UV topology"))?;
        let shell = topology.triangle_shell.get(patch.triangle).copied().ok_or_else(|| bad("Missing UV shell ownership"))?;
        let padding = s.uv.padding;
        let apron = crate::uv_cmds::cached_padding(s, material, size[0], size[1], [0, 0], shell, padding)?;
        patch
            .rasterize_with_padding(scene, size, reach, Some(&apron), padding, |x, y, at| {
                visited = visited.saturating_add(1);
                if visited > 4_194_304 {
                    over_budget = true;
                    return;
                }
                let values = [bilinear(&cov, rect, at), bilinear(&dual, rect, at), bilinear(&depth, rect, at)];
                if values[0] <= 0.0 {
                    return;
                }
                let tile = tiles.entry((x.div_euclid(64), y.div_euclid(64))).or_insert_with(|| vec![[0.0; 3]; 4096]);
                if let Some(dst) = tile.get_mut((y.rem_euclid(64) * 64 + x.rem_euclid(64)) as usize) {
                    for (a, b) in dst.iter_mut().zip(values) {
                        *a = a.max(b);
                    }
                }
            })
            .map_err(|e| bad(e.to_string()))?;
        if over_budget {
            return Err(bad("Wrap footprint exceeds 4M texels; reduce brush size"));
        }
    }
    for ((x, y), tile) in tiles {
        let rect = Rect::from_xywh(x * 64, y * 64, 64, 64);
        let channels: [Vec<f32>; 3] = std::array::from_fn(|i| tile.iter().map(|p| p[i]).collect());
        if !stroke.renderer.accumulate_mapped(rect, &channels[0], &channels[1], &channels[2], dab) {
            return Err(bad("Invalid mapped brush coverage"));
        }
    }
    if stroke.renderer.bounds().is_empty() {
        return Ok(false);
    }
    stroke.previous = Some(position);
    stroke.index = stroke.index.wrapping_add(1);
    Ok(true)
}
fn stamp(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("end").and_then(Value::as_bool) == Some(true) {
        s.uv.wrapped_stroke = None;
        return Ok(Value::Null);
    }
    let stamps = parse_stamps(p)?;
    let key = p
        .get("coalesce")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| bad("A bounded gesture coalesce key is required"))?
        .to_owned();
    let scene = s.uv.scene.clone().ok_or_else(|| bad("No mesh"))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    // Validate every member before taking or mutating the active wrapped stroke. A batch
    // cannot partially paint if one later hit is stale or belongs to a different binding.
    let mut materials = Vec::with_capacity(stamps.len());
    for sample in &stamps {
        let material = scene.triangles.get(sample.triangle).ok_or_else(|| bad("Triangle out of range"))?.material;
        if s.uv.map_bindings(s.uv.current_channel()).get(material).copied().flatten() != Some(st.doc.id) || s.uv.hidden_materials.contains(&material) {
            return Err(bad("Surface does not belong to the active editable texture"));
        }
        materials.push(material);
    }
    let size = [st.doc.size.width, st.doc.size.height];
    let (doc_id, active_layer, revision) = (st.doc.id, st.active_layer, st.revision);
    let mut stroke = if let Some(stroke) = s.uv.wrapped_stroke.take().filter(|old| {
        old.doc == doc_id
            && old.layer == if crate::channel_cmds::is_channel_target(p) { None } else { active_layer }
            && old.revision == revision
            && old.key == key
            && old.target.get("target") == p.get("target")
    }) {
        stroke
    } else {
        let cmd = if p.get("pencil").and_then(Value::as_bool) == Some(true) { "paint.pencil" } else { "paint.stroke" };
        let brush = if cmd == "paint.pencil" {
            crate::brush_cmds::pencil_brush(s, p)?
        } else {
            crate::brush_cmds::with_blend_mode(crate::brush_cmds::resolve_brush(s, p, cmd)?, p)
        };
        let (layer, mut brush, _) = crate::brush_cmds::stroke_target(s, p, brush)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let mut probe = (*st.doc).clone();
        let (target, lock) = crate::channel_cmds::target_surface(&mut probe, layer, p)?;
        let pre = target.clone();
        crate::brush_cmds::erase_locked(&mut brush, lock, s.tools.background);
        WrappedStroke {
            doc: st.doc.id,
            layer,
            revision: st.revision,
            key,
            renderer: StrokeRenderer::new(&brush, Some(pre.format()), 1.0),
            pre,
            brush,
            previous: None,
            pixels_per_unit: None,
            index: 0,
            target: p.clone(),
        }
    };
    if s.uv.wrap_mesh.is_none() {
        s.uv.wrap_mesh = Some(Arc::new(photocraft_uv::wrap::Mesh::new(&scene).map_err(|e| bad(e.to_string()))?));
    }
    let mesh = s.uv.wrap_mesh.as_ref().cloned().ok_or_else(|| bad("Missing wrap topology"))?;
    let mut should_composite = false;
    for (sample, material) in stamps.into_iter().zip(materials) {
        should_composite |= accumulate_stamp(s, &scene, &mesh, size, material, &mut stroke, sample)?;
    }
    if !should_composite || stroke.renderer.bounds().is_empty() {
        s.uv.wrapped_stroke = Some(stroke);
        return Ok(Value::Null);
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut probe = (*st.doc).clone();
    let selection = probe.selection.clone();
    let (surface, lock) = crate::channel_cmds::target_surface(&mut probe, stroke.layer, p)?;
    let current = surface.clone();
    let cmyk = crate::brush_cmds::doc_cmyk_space(s);
    // All samples are accumulated in order and composited once from the gesture's original
    // surface, preserving exact dab/flow output while avoiding one cache refresh per sample.
    let damage = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || stroke.renderer.composite(&stroke.pre, surface, selection.as_ref(), lock, false));
    if *surface == current {
        s.uv.wrapped_stroke = Some(stroke);
        return Ok(json!({"damage":[0,0,0,0]}));
    }
    let painted = surface.clone();
    s.edit("Paint Wrap", |doc, _| {
        let (surface, _) = crate::channel_cmds::target_surface(doc, stroke.layer, p)?;
        *surface = painted;
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        st.last_damage = Some(damage);
        stroke.revision = st.revision;
    }
    s.uv.wrapped_stroke = Some(stroke);
    Ok(json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()]}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "paint.uvWrap",
        label: "Paint tangent wrap",
        params: r#"{"triangle":int,"position":[x,y,z],"pressure":0..1,"stamps":[{"triangle":int,"position":[x,y,z],"pressure":0..1}],"erase":bool,"pencil":bool,"target":string,"coalesce":string,"end":bool} (single-sample fields remain supported; a bounded stamps batch is applied as one edit)"#,
        enabled,
        run: stamp,
        menu: &[],
        shortcut: None,
        journal: true,
    }]
}

#[cfg(all(test, feature = "uv-workbench"))]
mod tests {
    use super::*;
    use serde_json::json;

    fn scene() -> Session {
        let mut session = Session::new();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        session.execute("uv.model.import", json!({"path":path})).unwrap();
        assert!(session.uv.wrap_mesh.is_some(), "model import prepares the Wrap adjacency cache before brush input");
        session.execute("uv.texture.create", json!({"width":128,"height":128,"depth":16})).unwrap();
        session.execute("tools.setBrush", json!({"brush":{"size":12.0,"hardness":1.0,"spacing":0.05,"opacity":0.5}})).unwrap();
        session
    }

    fn seam_triangle(session: &Session) -> usize {
        let scene = session.uv.scene.as_ref().unwrap();
        scene
            .triangles
            .iter()
            .position(|triangle| {
                triangle.corners.iter().all(|corner| corner.vertex < 4)
                    && photocraft_uv::topology::barycentric(
                        [-0.02, 0.0],
                        triangle.corners.map(|corner| {
                            let point = scene.positions[corner.vertex];
                            [point[0], point[1]]
                        }),
                    )
                    .is_some()
            })
            .unwrap()
    }

    fn surface(session: &Session) -> Surface {
        session.active().unwrap().doc.layers[0].surface().unwrap().clone()
    }

    #[test]
    fn batched_surface_stamps_match_legacy_calls_and_commit_once() {
        let mut batched = scene();
        let mut singles = scene();
        let triangle = seam_triangle(&batched);
        assert_eq!(triangle, seam_triangle(&singles));
        let samples = [
            json!({"triangle":triangle,"position":[-0.02,0.0,0.0],"pressure":1.0}),
            json!({"triangle":triangle,"position":[-0.02,0.025,0.0],"pressure":0.65}),
            json!({"triangle":triangle,"position":[-0.02,0.05,0.0],"pressure":0.9}),
        ];
        let before = surface(&batched);
        let history_before = batched.active().unwrap().history.past_len();
        let revision_before = batched.active().unwrap().revision;
        let common = json!({"coalesce":"batch-equivalence","target":"pixels"});
        let mut batch = common.clone();
        batch["stamps"] = Value::Array(samples.to_vec());
        batched.execute("paint.uvWrap", batch).unwrap();
        assert_eq!(batched.active().unwrap().revision, revision_before + 1, "one Session edit per batch");
        assert_eq!(batched.active().unwrap().history.past_len(), history_before + 1);

        for sample in samples {
            let mut single = common.clone();
            single["triangle"] = sample["triangle"].clone();
            single["position"] = sample["position"].clone();
            single["pressure"] = sample["pressure"].clone();
            singles.execute("paint.uvWrap", single).unwrap();
        }
        assert_eq!(surface(&batched), surface(&singles), "batch preserves ordered pressure and seam coverage");
        assert_ne!(surface(&batched), before, "the wrapped samples paint the atlas");

        batched.execute("edit.undo", json!({})).unwrap();
        assert_eq!(surface(&batched), before, "a batch is undone as a single edit");
    }

    #[test]
    fn invalid_late_stamp_rejects_the_whole_batch_without_editing() {
        let mut session = scene();
        let triangle = seam_triangle(&session);
        let before = surface(&session);
        let revision = session.active().unwrap().revision;
        let history = session.active().unwrap().history.past_len();
        let result = session.execute(
            "paint.uvWrap",
            json!({
                "coalesce":"invalid-batch",
                "target":"pixels",
                "stamps":[
                    {"triangle":triangle,"position":[-0.02,0.0,0.0],"pressure":1.0},
                    {"triangle":u64::MAX,"position":[-0.02,0.05,0.0],"pressure":1.0}
                ]
            }),
        );
        assert!(result.is_err(), "out-of-range later sample is rejected");
        assert_eq!(surface(&session), before, "validation occurs before any samples are applied");
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
}
