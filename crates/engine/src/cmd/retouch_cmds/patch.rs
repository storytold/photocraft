//! The Patch Tool as a command: heal the selected area with texture from another place.
//!
//! The selection is the patch, `offset` is how far the user dragged it. In `source` mode (the
//! default) the selected area is repaired with the pixels under the dragged outline; in
//! `destination` mode the selected pixels are copied to the dragged outline and repair it. Either way the copied texture is seamless-cloned (`heal_region`, as the Healing Brush), so it
//! takes the colour and lighting around the repaired area. A feathered selection blends the result
//! in by its coverage. Only the targeted surface is read and written: in Normal mode the patch
//! samples the current layer.
//!
//! With `contentAware` (Photoshop's Patch: Content-Aware) the selected area is filled from the
//! dragged-to place content-aware, as the Content-Aware Move Tool lands its content
//! ([`super::content_aware_move::land`]): the middle is copied, fitted to the selection's
//! surroundings by `color` (0–10), and a band along the edge, set by `structure` (1–7), is
//! re-synthesised. Measured on Photoshop 25.4 (a 60 px selection dragged 150 px onto a ring-shaped
//! object): only the selection changes; Structure 3 to 7 give the same result; Color 0 keeps the
//! copied level, higher values shift it towards the surroundings by up to about `0.04 · color`
//! (3: 32 of 42 levels, 31 of 102; 10: 35 of 42, 95 of 102). It runs as a background job, like
//! the move.

use photocraft_algo::transform::Homography;

use super::content_aware_move::{mapped_bounds, placement, resample};
use super::{
    BlendMode, Document, EngineError, LayerId, PixelFormat, Rect, Region, Result, Session, Surface, Value, apply_coverage, bad, clamp_samples, damage_json,
    flag, has_pixel_layer, heal_region, json, point, poisson, string, targets_pixels,
};

const CMD: &str = "paint.patch";

/// Largest patch, in pixels of the selection's bounding box. The solve holds the two regions, the
/// result and per-channel float planes of it (on the order of 150 bytes per pixel with four
/// channels), so this keeps a patch to about 2.5 GB at worst instead of risking an allocation abort.
const MAX_PIXELS: u64 = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatchMode {
    /// The selection is repaired with texture from the dragged-to place.
    Source,
    /// The dragged-to place is repaired with texture from the selection.
    Destination,
}

/// A pixel target and a selection to patch with.
pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    has_pixel_layer(s)?;
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.is_some() { Ok(()) } else { Err("select the area to patch first".into()) }
}

/// The layer `p` targets (`None` for an alpha channel or the Quick Mask), as `parse_brush`.
pub(super) fn target_layer(s: &Session, p: &Value) -> Result<Option<LayerId>> {
    if crate::channel_cmds::is_channel_target(p) {
        return Ok(None);
    }
    crate::commands::layer_param(s, p).map(Some)
}

/// Integer drag offset, rejecting non-finite and absurd values.
pub(super) fn offset(cmd: &str, p: &Value) -> Result<(i32, i32)> {
    let (dx, dy) = point(p, "offset").ok_or_else(|| bad(cmd, "missing `offset` ([dx, dy])"))?;
    let ok = |v: f64| v.is_finite() && v.abs() < 1e7;
    if !ok(dx) || !ok(dy) {
        return Err(bad(cmd, "`offset` must be two finite numbers"));
    }
    Ok((dx.round() as i32, dy.round() as i32))
}

/// Selected area within the canvas: the selection's content bounds, or the whole canvas when the
/// selection covers everything outside its tiles (an inverted selection).
pub(super) fn selection_area(sel: &Surface, canvas: Rect) -> Rect {
    let outside = sel.default_pixel().first().copied().unwrap_or(0.0);
    let b = if outside > 0.0 { canvas } else { sel.content_bounds() };
    b.intersect(&canvas)
}

/// Selection coverage over `rect` (row-major, 0..1).
pub(super) fn coverage(sel: &Surface, rect: Rect) -> Vec<f32> {
    // Read tile by tile (a per-pixel lookup costs a tile search each).
    let n = sel.channels().max(1);
    sel.read_region(rect).chunks_exact(n).map(|px| px[0].clamp(0.0, 1.0)).collect()
}

/// Seamless clone with the membrane solved `step` times coarser: the Poisson result is the source
/// plus a harmonic (smooth) correction `h` of the boundary mismatch, so `h` is solved on a grid of
/// `step`×`step` cells and interpolated back, while the texture stays the full-resolution source.
/// The live preview's approximation of [`heal_region`] (`step` 1 is exactly it).
fn heal_region_coarse(fmt: &PixelFormat, src: &Region, dst: &Region, mask: &[bool], step: usize) -> Region {
    if step <= 1 {
        return heal_region(fmt, src, dst, mask);
    }
    let (w, h, n) = (dst.width(), dst.height(), dst.ch);
    let (cw, chh) = (w.div_ceil(step), h.div_ceil(step));
    // A cell is unknown when every pixel in it is patched; otherwise it holds the mean mismatch
    // `dst - src` of its unpatched pixels (the Dirichlet data of the coarse problem).
    let mut diff = vec![0.0f32; cw * chh * n];
    let mut hole = vec![true; cw * chh];
    let mut count = vec![0u32; cw * chh];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask[i] {
                continue;
            }
            let c = (y / step) * cw + x / step;
            hole[c] = false;
            count[c] += 1;
            for k in 0..n {
                diff[c * n + k] += dst.data[i * n + k] - src.data[i * n + k];
            }
        }
    }
    for (c, m) in count.iter().enumerate() {
        if *m > 0 {
            diff[c * n..(c + 1) * n].iter_mut().for_each(|v| *v /= *m as f32);
        }
    }
    let membrane = poisson::membrane_fill(cw, chh, n, &diff, &hole);
    // Bilinear lookup of the membrane (cell centres at (c + 0.5) · step): the two cells and the
    // weight along one axis, per pixel coordinate.
    let lerp_at = |p: usize, cells: usize| {
        let f = ((p as f32 + 0.5) / step as f32 - 0.5).clamp(0.0, (cells - 1) as f32);
        let c0 = f.floor() as usize;
        (c0, (c0 + 1).min(cells - 1), f - c0 as f32)
    };
    let xs: Vec<(usize, usize, f32)> = (0..w).map(|x| lerp_at(x, cw)).collect();
    let mut out = dst.clone();
    let fill_row = |(y, row): (usize, &mut [f32])| {
        let (y0, y1, ty) = lerp_at(y, chh);
        for (x, &(x0, x1, tx)) in xs.iter().enumerate() {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            for k in 0..n {
                let v = |cx: usize, cy: usize| membrane[(cy * cw + cx) * n + k];
                let m = (v(x0, y0) * (1.0 - tx) + v(x1, y0) * tx) * (1.0 - ty) + (v(x0, y1) * (1.0 - tx) + v(x1, y1) * tx) * ty;
                row[x * n + k] = src.data[i * n + k] + m;
            }
        }
    };
    {
        use rayon::prelude::*;
        out.data.par_chunks_mut(w * n).enumerate().for_each(fill_row);
    }
    clamp_samples(fmt, &mut out.data);
    out
}

/// What a patch does, checked: the target layer, the selected area, and where it heals from and to.
struct Plan {
    id: Option<LayerId>,
    canvas: Rect,
    area: Rect,
    /// Shift of the repaired area from the selection, and of the texture from the repaired area.
    dst_shift: (i32, i32),
    src_shift: (i32, i32),
    destination: bool,
    offset: (i32, i32),
    /// Original selection coordinates to the positioned box.
    transform: Option<Homography>,
}

fn plan(s: &Session, p: &Value) -> Result<Plan> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    plan_for(&d.doc, target_layer(s, p)?, p)
}

fn plan_for(doc: &Document, id: Option<LayerId>, p: &Value) -> Result<Plan> {
    let mode = match string(p, "mode", "source") {
        "source" => PatchMode::Source,
        "destination" => PatchMode::Destination,
        o => return Err(bad(CMD, format!("unknown mode `{o}` (source|destination)"))),
    };
    let (dx, dy) = offset(CMD, p)?;
    let canvas = doc.bounds();
    let sel = doc.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to patch first"))?;
    let area = selection_area(sel, canvas);
    if area.is_empty() {
        return Err(bad(CMD, "the selection is empty"));
    }
    let pixels = u64::from(area.width()) * u64::from(area.height());
    if pixels > MAX_PIXELS {
        return Err(bad(CMD, format!("the selection is too large to patch ({pixels} pixels, at most {MAX_PIXELS})")));
    }
    let transform = placement(CMD, p, area, (dx, dy))?;
    if transform.is_some() && flag(p, "contentAware", false) {
        return Err(bad(CMD, "patch transforms require Normal mode"));
    }
    if transform.is_none() && (dx, dy) == (0, 0) {
        return Err(bad(CMD, "drag the selection to the area to sample from"));
    }
    // A destination patch can be cropped at the canvas edge. Its sample stays on the canvas.
    let boxed = transform.as_ref().map_or_else(|| area.translate(dx, dy), |map| mapped_bounds(map, area));
    if !canvas.contains_rect(&boxed) {
        let clip = mode == PatchMode::Destination && flag(p, "clipDestination", false) && !flag(p, "contentAware", false);
        if !clip {
            let message = if transform.is_some() && mode == PatchMode::Source {
                "the sample must stay inside the canvas"
            } else {
                "the dragged patch must stay inside the canvas"
            };
            return Err(bad(CMD, message));
        }
        if canvas.intersect(&boxed).is_empty() {
            return Err(bad(CMD, "the patch must overlap the canvas"));
        }
    }
    let (dst_shift, src_shift) = match mode {
        PatchMode::Source => ((0, 0), (dx, dy)),
        PatchMode::Destination => ((dx, dy), (-dx, -dy)),
    };
    Ok(Plan { id, canvas, area, dst_shift, src_shift, destination: mode == PatchMode::Destination, offset: (dx, dy), transform })
}

/// Heal `surf` at the selection (shifted by `dst_shift`) with texture read `src_shift` away, the
/// membrane solved `step` times coarser (1 = exact). Returns the damaged rectangle.
fn patch_surface(surf: &mut Surface, sel: &Surface, plan: &Plan, lock: bool, step: usize) -> Result<Rect> {
    if let Some(map) = &plan.transform {
        return transformed_patch(surf, sel, plan, map, lock, step);
    }
    let Plan { canvas, area, dst_shift, src_shift, .. } = *plan;
    // The repaired area plus a two-pixel ring: the ring is the Dirichlet boundary of the solve. It is
    // clipped to where both the repaired and the sampled pixels lie on the canvas, so a patch at the
    // canvas edge sees a free (Neumann) boundary there instead of the empty pixels beyond it.
    let dst_area = area.translate(dst_shift.0, dst_shift.1);
    let g = dst_area.inflate(2).intersect(&canvas).intersect(&canvas.translate(-src_shift.0, -src_shift.1));
    if g.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let fmt = surf.format();
    let dst = Region::read(surf, g);
    let mut src = Region::read(surf, g.translate(src_shift.0, src_shift.1));
    src.rect = g;
    // Selection coverage moved to the repaired place.
    let cov = coverage(sel, g.translate(-dst_shift.0, -dst_shift.1));
    let mask: Vec<bool> = cov.iter().map(|c| *c > 0.0).collect();
    let healed = heal_region_coarse(&fmt, &src, &dst, &mask, step);
    Ok(apply_coverage(surf, g, &cov, 1.0, None, lock, &healed, BlendMode::Normal))
}

/// In Destination, the box moves both the sample and its mask onto the repaired area.
/// In Source, the mask stays put and the box describes the pixels sampled through it.
fn transformed_patch(surf: &mut Surface, sel: &Surface, plan: &Plan, map: &Homography, lock: bool, step: usize) -> Result<Rect> {
    let Plan { canvas, area, destination, .. } = *plan;
    let boxed = mapped_bounds(map, area);
    let (dst_area, sample_map) =
        if destination { (boxed.intersect(&canvas), map.inverse().ok_or_else(|| bad(CMD, "the transform collapses the selection"))?) } else { (area, *map) };
    let g = dst_area.inflate(2).intersect(&canvas);
    let samples = mapped_bounds(&sample_map, g).inflate(1).intersect(&canvas);
    if dst_area.is_empty() || samples.is_empty() {
        return Err(bad(CMD, "the transformed patch must overlap the canvas"));
    }
    for rect in [g, samples] {
        if u64::from(rect.width()) * u64::from(rect.height()) > MAX_PIXELS {
            return Err(bad(CMD, "the transformed patch is too large"));
        }
    }
    let cov = if destination {
        let padded = area.inflate(1);
        let mut data = coverage(sel, padded);
        for (y, row) in data.chunks_exact_mut(padded.width() as usize).enumerate() {
            for (x, value) in row.iter_mut().enumerate() {
                let x = padded.x0 + x as i32;
                let y = padded.y0 + y as i32;
                if x < area.x0 || x >= area.x1 || y < area.y0 || y >= area.y1 {
                    *value = 0.0;
                }
            }
        }
        resample(&Region { rect: padded, ch: 1, data }, &sample_map, g).data
    } else {
        coverage(sel, g)
    };
    let src = resample(&Region::read(surf, samples), &sample_map, g);
    let dst = Region::read(surf, g);
    let mask: Vec<bool> = cov.iter().map(|c| *c > 0.0).collect();
    if !mask.iter().any(|c| *c) {
        return Err(bad(CMD, "the transformed selection covers no pixels"));
    }
    let healed = heal_region_coarse(&surf.format(), &src, &dst, &mask, step);
    Ok(apply_coverage(surf, g, &cov, 1.0, None, lock, &healed, BlendMode::Normal))
}

pub(super) fn patch(s: &mut Session, p: &Value) -> Result<Value> {
    if flag(p, "contentAware", false) {
        return patch_content_aware(s, p);
    }
    let plan = plan(s, p)?;
    let label = if plan.destination { "Patch (Destination)" } else { "Patch" };
    let dmg = s.edit(label, |doc, _| {
        let sel = doc.selection.clone().ok_or_else(|| bad(CMD, "select the area to patch first"))?;
        let (surf, lock) = crate::channel_cmds::target_surface(doc, plan.id, p)?;
        let dmg = patch_surface(surf, &sel, &plan, lock, 1)?;
        if flag(p, "deselect", false) {
            doc.selection = None;
        }
        Ok(dmg)
    })?;
    s.hint_damage(dmg);
    Ok(json!({ "damage": damage_json(dmg), "offset": [plan.offset.0, plan.offset.1] }))
}

/// A snapshot of the patch inputs. The preview can run independently of the editor session.
pub struct PatchPreviewJob {
    doc: std::sync::Arc<Document>,
    params: Value,
    layer: Option<LayerId>,
    colors: Option<crate::color_channels::ColorChannels>,
    max_cells: u64,
    min_step: u32,
}

impl PatchPreviewJob {
    /// Apply the preview to a copy of the document and return the damaged rectangle.
    pub fn run(self) -> Result<(Document, Rect)> {
        let plan = plan_for(&self.doc, self.layer, &self.params)?;
        let area = match &plan.transform {
            Some(map) if plan.destination => mapped_bounds(map, plan.area).intersect(&plan.canvas),
            _ => plan.area,
        };
        let pixels = u64::from(area.width() + 4) * u64::from(area.height() + 4);
        let need = (pixels as f64 / self.max_cells.max(1) as f64).sqrt().ceil() as usize;
        let step = need.max(self.min_step as usize).clamp(1, 64);
        let mut doc = (*self.doc).clone();
        let sel = self.doc.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to patch first"))?;
        let (surf, lock) = crate::channel_cmds::target_surface(&mut doc, plan.id, &self.params)?;
        let damage = patch_surface(surf, sel, &plan, lock, step)?;
        if let (Some(colors), Some(id)) = (self.colors, plan.id) {
            crate::channel_cmds::restrict_to_color(&self.doc, &mut doc, id, colors)?;
        }
        Ok((doc, damage))
    }
}

/// Capture the live preview inputs. `max_cells` limits the resolution of the colour-fit solve.
/// The sample texture and selection coverage retain their full resolution.
pub fn prepare_preview(s: &Session, p: &Value, max_cells: u64, min_step: u32) -> Result<PatchPreviewJob> {
    let params = crate::channel_cmds::inject_target(s, CMD, p.clone());
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let layer = target_layer(s, &params)?;
    let colors = crate::color_channels::target(s, &params)?;
    Ok(PatchPreviewJob { doc: d.doc.clone(), params, layer, colors, max_cells, min_step })
}

/// Render a patch preview synchronously.
pub fn preview(s: &Session, p: &Value, max_cells: u64, min_step: u32) -> Result<(Document, Rect)> {
    prepare_preview(s, p, max_cells, min_step)?.run()
}

/// Optional whole-number param in `lo..=hi`; absent or null gives `default`.
fn level(p: &Value, k: &str, default: u8, lo: u8, hi: u8) -> Result<u8> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && (f64::from(lo)..=f64::from(hi)).contains(&x) => Ok(x.round() as u8),
            _ => Err(bad(CMD, format!("`{k}` must be a number {lo}..{hi}"))),
        },
    }
}

/// Patch: Content-Aware. The selection is filled from the place it was dragged to (see the module
/// docs), as a background job.
fn patch_content_aware(s: &mut Session, p: &Value) -> Result<Value> {
    use super::content_aware_move::{Target, land, mask, read, window};
    const LABEL: &str = "Patch (Content-Aware)";
    let plan = plan(s, p)?;
    let structure = level(p, "structure", 4, 1, 7)?;
    let color = level(p, "color", 0, 0, 10)?;
    let target = Target { id: plan.id, all_layers: flag(p, "sampleAllLayers", false) && targets_pixels(p) };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let p = p.clone();
    let (dx, dy) = plan.offset;
    let (area, canvas) = (plan.area, plan.canvas);
    crate::jobs::run(
        s,
        LABEL,
        true,
        {
            let p = p.clone();
            move |ctx| {
                ctx.progress(0.0, LABEL);
                let mut work = (*doc).clone();
                let sel = doc.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to patch first"))?;
                let win = window(area, canvas);
                let mut content = read(&mut work, target, &p, win.translate(dx, dy))?;
                content.rect = win;
                let selected = mask(sel, win);
                let avoid = vec![false; selected.len()];
                let damage = land(&mut work, target, &p, win, &selected, &content, &avoid, structure, color, (0.0, 1.0), ctx, LABEL)?;
                let surf = crate::channel_cmds::target_surface(&mut work, target.id, &p)?.0.clone();
                Ok((surf, damage))
            }
        },
        move |s, (surf, damage)| {
            s.edit(LABEL, |doc, _| {
                *crate::channel_cmds::target_surface(doc, target.id, &p)?.0 = surf;
                Ok(())
            })?;
            s.hint_damage(damage);
            Ok(json!({ "damage": damage_json(damage), "offset": [dx, dy], "contentAware": true }))
        },
    )
}
