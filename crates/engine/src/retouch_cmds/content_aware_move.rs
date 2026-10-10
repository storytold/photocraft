//! The Content-Aware Move Tool as a command: move or duplicate the selected pixels, and let
//! content-aware fill close the gaps.
//!
//! The selection is the content, `offset` how far the user dragged it. At the new place the content
//! is pasted, fitted to its new surroundings by `color`, and a band along its edge, inside the
//! selection, is re-synthesised from the surroundings so that it merges in. In `move` mode the place
//! the content left is then filled content-aware (`extend` keeps it). That fill never samples the
//! moved content, so the object doesn't come back as a ghost; the edge band may sample it (it is the
//! context the band blends towards), but not the content's old place.
//!
//! Measured on Photoshop 25.4 (a 60 px selection holding a ring-shaped object, dragged 150 px):
//! * **Structure** 3 to 7 give the same result, bit for bit: the content is copied exactly up to
//!   about 8 px from the selection's edge, and that band blends into the surroundings. 2 widens the
//!   band; 1 re-synthesises the content too, so even its middle is no longer an exact copy.
//! * **Color** 0 pastes the content as it is; higher values bring its level towards the new
//!   surroundings by up to about `0.04 · color` (Patch Tool, content 42 and 102 levels brighter:
//!   3 shifts it 32 and 31 levels, 10 shifts it 35 and 95), a limited gain and bias
//!   ([`photocraft_algo::content_aware::color_amount`]).
//!
//! The selection moves with the content, as in Photoshop. A soft selection counts wherever it is
//! above zero (as the Patch Tool's); the band does the blending. The fills are PatchMatch
//! completions and can take seconds on large selections, so the command runs as a background job
//! (#210): with progress, cancellable, and the document unchanged until it applies. The Patch
//! Tool's Content-Aware mode lands its content the same way ([`land`]).

use photocraft_algo::content_aware::{FillOptions, color_amount, color_level, fill_with, level_transform};

use super::patch::{coverage, offset, selection_area, target_layer};
use super::*;

const CMD: &str = "paint.contentAwareMove";

/// Largest selection, in pixels of its bounding box (as the Patch Tool's). Each fill window is the
/// selection plus a margin; at this size one window holds about 37 M pixels and the completion's
/// working set stays within a few GB.
const MAX_PIXELS: u64 = 16 << 20;

/// Cap on the sampling margin around the content (the window grows with the selection).
const MAX_MARGIN: i32 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// The content leaves its place, which is filled from the surroundings.
    Move,
    /// The content is duplicated; its place stays.
    Extend,
}

/// A pixel target and a selection to move.
pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    has_pixel_layer(s)?;
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.is_some() { Ok(()) } else { Err("select the area to move first".into()) }
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

/// The pixels a step reads and writes: a layer (or, with `None`, the channel `p` targets), and
/// whether reading samples the visible composite (Sample All Layers).
#[derive(Clone, Copy, Debug)]
pub(super) struct Target {
    pub id: Option<LayerId>,
    pub all_layers: bool,
}

/// What a move does, checked before any work starts.
#[derive(Clone, Copy, Debug)]
struct Plan {
    target: Target,
    canvas: Rect,
    /// The selected area (where the content comes from).
    area: Rect,
    offset: (i32, i32),
    mode: Mode,
    structure: u8,
    color: u8,
}

fn plan(s: &Session, p: &Value) -> Result<Plan> {
    let mode = match string(p, "mode", "move") {
        "move" => Mode::Move,
        "extend" => Mode::Extend,
        o => return Err(bad(CMD, format!("unknown mode `{o}` (move|extend)"))),
    };
    let structure = level(p, "structure", 4, 1, 7)?;
    let color = level(p, "color", 0, 0, 10)?;
    let (dx, dy) = offset(CMD, p)?;
    let id = target_layer(s, p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let sel = d.doc.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to move first"))?;
    let area = selection_area(sel, canvas);
    if area.is_empty() {
        return Err(bad(CMD, "the selection is empty"));
    }
    let pixels = u64::from(area.width()) * u64::from(area.height());
    if pixels > MAX_PIXELS {
        return Err(bad(CMD, format!("the selection is too large to move ({pixels} pixels, at most {MAX_PIXELS})")));
    }
    if (dx, dy) == (0, 0) {
        return Err(bad(CMD, "drag the selection to where the content should go"));
    }
    // Content dropped beyond the edge would be lost: keep it on the canvas, as the Patch Tool does.
    if !canvas.contains_rect(&area.translate(dx, dy)) {
        return Err(bad(CMD, "the moved selection must stay inside the canvas"));
    }
    let all_layers = flag(p, "sampleAllLayers", false) && targets_pixels(p);
    Ok(Plan { target: Target { id, all_layers }, canvas, area, offset: (dx, dy), mode, structure, color })
}

/// Width in pixels of the edge band re-synthesised at the new place, for Structure 1..7 and content
/// `min_side` pixels across. Photoshop gives the same result for 3 to 7: the band is about a patch
/// wide whatever the content's size; 2 and 1 widen it (1 so far that the content itself is
/// re-synthesised).
pub(super) fn band_width(structure: u8, min_side: u32) -> usize {
    let side = min_side as f32;
    let w = match structure {
        0 | 1 => (side * 0.3).max(12.0),
        2 => (side * 0.18).max(9.0),
        _ => 6.0,
    };
    (w.round() as usize).min(128)
}

/// The cells of a `w × h` mask whose whole `(2r+1)²` neighbourhood, as far as it lies on the grid,
/// is set (a square erosion in O(w·h) through a summed-area table). The grid's border doesn't erode:
/// a selection reaching the canvas edge keeps its content there.
fn erode(w: usize, h: usize, m: &[bool], r: usize) -> Vec<bool> {
    if r == 0 {
        return m.to_vec();
    }
    let w1 = w + 1;
    let mut sat = vec![0u32; w1 * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += u32::from(m.get(y * w + x).copied().unwrap_or(false));
            sat[(y + 1) * w1 + x + 1] = sat[y * w1 + x + 1] + row;
        }
    }
    let mut out = vec![false; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            if !m.get(y * w + x).copied().unwrap_or(false) {
                continue;
            }
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let set = sat[y1 * w1 + x1] + sat[y0 * w1 + x0] - sat[y0 * w1 + x1] - sat[y1 * w1 + x0];
            out[y * w + x] = set as usize == (x1 - x0) * (y1 - y0);
        }
    }
    out
}

/// Where a fill samples from around `r`: Photoshop's Content-Aware window
/// ([`crate::fill_cmds::sampling_window`]), at most [`MAX_MARGIN`] pixels around `r`.
pub(super) fn window(r: Rect, canvas: Rect) -> Rect {
    crate::fill_cmds::sampling_window(r, canvas).intersect(&r.inflate(MAX_MARGIN))
}

/// The pixels a step works on: the targeted surface, or with Sample All Layers the visible
/// composite (encoded in the target's format).
pub(super) fn read(doc: &mut Document, t: Target, p: &Value, rect: Rect) -> Result<Region> {
    let (surf, _) = crate::channel_cmds::target_surface(doc, t.id, p)?;
    if t.all_layers {
        let fmt = surf.format();
        return Ok(composite_region(doc, None, SampleLayers::All, rect, fmt));
    }
    Ok(Region::read(surf, rect))
}

/// Write `out` over `mask` into the target, replacing the pixels (the result already holds what
/// should show there). With the transparency lock, alpha is kept and transparent pixels stay as
/// they are. Returns the written rectangle. Shared with the Remove Tool.
pub(super) fn replace(doc: &mut Document, t: Target, p: &Value, out: &Region, mask: &[bool]) -> Result<Rect> {
    let (surf, lock) = crate::channel_cmds::target_surface(doc, t.id, p)?;
    let fmt = surf.format();
    let a = alpha_index(&fmt);
    let rect = out.rect;
    let mut px = surf.read_region(rect);
    let n = out.ch;
    if px.len() != out.data.len() || mask.len() * n != px.len() {
        return Err(EngineError::Other("internal error: fill buffers disagree in size".into()));
    }
    for ((dst, src), m) in px.chunks_exact_mut(n).zip(out.data.chunks_exact(n)).zip(mask) {
        if !*m {
            continue;
        }
        match a.filter(|_| lock) {
            Some(ai) => {
                if dst.get(ai).is_some_and(|v| *v > 0.0) {
                    for (c, (d, s)) in dst.iter_mut().zip(src).enumerate() {
                        if c != ai {
                            *d = *s;
                        }
                    }
                }
            }
            None => dst.copy_from_slice(src),
        }
    }
    surf.write_region(rect, &px);
    Ok(rect)
}

/// Selection coverage over `rect` as a mask (set wherever it is above zero).
pub(super) fn mask(sel: &Surface, rect: Rect) -> Vec<bool> {
    coverage(sel, rect).into_iter().map(|c| c > 0.0).collect()
}

/// Fill options: the fill's own colour adaptation (Edit › Content-Aware Fill's default), seeded so
/// the result is reproducible.
pub(super) fn fill_options() -> FillOptions {
    FillOptions { seed: 0x00ca_4e00, ..FillOptions::default() }
}

/// A ring of `w` pixels just outside `m` (within the grid).
fn outer_ring(gw: usize, gh: usize, m: &[bool], w: usize) -> Vec<bool> {
    let inside: Vec<bool> = m.iter().map(|v| !*v).collect();
    erode(gw, gh, &inside, w).iter().zip(m).map(|(keep, sel)| !*keep && !*sel).collect()
}

/// Content landing at `dst_area`: the pixels of the window around it, with `selected` (the
/// selection's shape there) to be replaced by the content found `shift` away (`content` = that
/// window, so `content` pixel = window pixel + `shift`). The content's middle is copied, its level
/// fitted to the new surroundings by `color`; a `band_width(structure)` band inside the selection's
/// edge is re-synthesised from the surroundings and that middle, never sampling `avoid`. Writes the
/// result into the target and returns the rectangle it changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn land(
    doc: &mut Document,
    t: Target,
    p: &Value,
    win: Rect,
    selected: &[bool],
    content: &Region,
    avoid: &[bool],
    structure: u8,
    color: u8,
    stage: (f32, f32),
    ctx: &crate::jobs::JobCtx,
    label: &str,
) -> Result<Rect> {
    let (w, h) = (win.width() as usize, win.height() as usize);
    let here = read(doc, t, p, win)?;
    let n = here.ch;
    let (sx0, sy0, sx1, sy1) = selected.iter().enumerate().filter(|(_, s)| **s).fold((w, h, 0, 0), |b, (i, _)| {
        let (x, y) = (i % w, i / w);
        (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1))
    });
    let side = (sx1.saturating_sub(sx0)).min(sy1.saturating_sub(sy0)) as u32;
    let core = erode(w, h, selected, band_width(structure, side));
    ctx.check()?;
    // Colour: the content's surroundings (where it came from) against the new ones, both just
    // outside the selection's shape.
    let ring = outer_ring(w, h, selected, 6);
    let pick = |r: &Region| -> Vec<f32> { ring.iter().enumerate().filter(|(_, k)| **k).flat_map(|(i, _)| r.data[i * n..(i + 1) * n].to_vec()).collect() };
    let gb = level_transform(n, &pick(content), &pick(&here), color_amount(color));
    let fmt = crate::channel_cmds::target_surface(doc, t.id, p)?.0.format();
    let alpha = alpha_index(&fmt);
    let mut buf = here;
    for (i, _) in core.iter().enumerate().filter(|(_, c)| **c) {
        for (c, &(g, b)) in gb.iter().enumerate() {
            let v = content.data[i * n + c];
            // Colour channels are fitted; alpha is copied.
            buf.data[i * n + c] = if Some(c) == alpha { v } else { g * v + b };
        }
    }
    clamp_samples(&fmt, &mut buf.data);
    // Structure: the band between the kept core and the selection's edge comes from the
    // surroundings and the kept core, never from what `avoid` marks.
    let band: Vec<bool> = selected.iter().zip(&core).map(|(m, c)| *m && !*c).collect();
    if band.iter().any(|b| *b) {
        let allowed: Vec<bool> = avoid.iter().map(|o| !*o).collect();
        let opts = FillOptions { color_adaptation: if color == 0 { color_level("default") } else { color_amount(color) }, ..fill_options() };
        buf.data = ctx.stage(stage.0, stage.1, label, |ctl| fill_with(w, h, n, &buf.data, &band, &allowed, &opts, ctl)).map_err(|_| EngineError::Cancelled)?;
    }
    replace(doc, t, p, &buf, selected)
}

/// The new target surface and the rectangle it changed: the content pasted at the offset, then
/// (`move`) its old place filled. Works on `doc`, a copy of the document.
fn run_move(doc: &mut Document, sel: &Surface, plan: &Plan, p: &Value, ctx: &crate::jobs::JobCtx, label: &str) -> Result<(Surface, Rect)> {
    let (dx, dy) = plan.offset;
    let opts = fill_options();
    let split = if plan.mode == Mode::Move { 0.4 } else { 1.0 };
    let cancelled = |_| EngineError::Cancelled;

    // 1. The content at its new place: the content with its original surroundings laid over the
    // window there; never sampling the content's old place.
    let win = window(plan.area.translate(dx, dy), plan.canvas);
    let mut content = read(doc, plan.target, p, win.translate(-dx, -dy))?;
    content.rect = win;
    let moved = mask(sel, win.translate(-dx, -dy));
    let old_place = mask(sel, win);
    let mut damage = land(doc, plan.target, p, win, &moved, &content, &old_place, plan.structure, plan.color, (0.0, split), ctx, label)?;

    // 2. Move: fill the place the content left (where the content didn't land on it).
    if plan.mode == Mode::Move {
        ctx.check()?;
        let win = window(plan.area, plan.canvas);
        let (w, h) = (win.width() as usize, win.height() as usize);
        let img = read(doc, plan.target, p, win)?;
        let landed = mask(sel, win.translate(-dx, -dy));
        let hole: Vec<bool> = mask(sel, win).iter().zip(&landed).map(|(s, l)| *s && !*l).collect();
        if hole.iter().any(|h| *h) {
            let allowed: Vec<bool> = landed.iter().map(|l| !*l).collect();
            let data = ctx.stage(split, 1.0, label, |ctl| fill_with(w, h, img.ch, &img.data, &hole, &allowed, &opts, ctl)).map_err(cancelled)?;
            damage = damage.union(&replace(doc, plan.target, p, &Region { rect: win, ch: img.ch, data }, &hole)?);
        }
    }
    let surf = crate::channel_cmds::target_surface(doc, plan.target.id, p)?.0.clone();
    Ok((surf, damage))
}

pub(super) fn content_aware_move(s: &mut Session, p: &Value) -> Result<Value> {
    let plan = plan(s, p)?;
    let label = if plan.mode == Mode::Move { "Content-Aware Move" } else { "Content-Aware Extend" };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let p = p.clone();
    crate::jobs::run(
        s,
        label,
        true,
        {
            let p = p.clone();
            move |ctx| {
                ctx.progress(0.0, label);
                let mut work = (*doc).clone();
                let sel = doc.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to move first"))?;
                run_move(&mut work, sel, &plan, &p, ctx, label)
            }
        },
        move |s, (surf, damage)| {
            let (dx, dy) = plan.offset;
            s.edit(label, |doc, _| {
                *crate::channel_cmds::target_surface(doc, plan.target.id, &p)?.0 = surf;
                // The selection follows the content, ready for another drag.
                if let Some(sel) = doc.selection.as_ref() {
                    doc.selection = Some(crate::layer_multi_cmds::shift_surface(sel, dx, dy));
                }
                Ok(())
            })?;
            if let Some(st) = s.active_mut() {
                st.last_damage = Some(damage);
            }
            let mode = if plan.mode == Mode::Move { "move" } else { "extend" };
            Ok(json!({ "damage": damage_json(damage), "offset": [dx, dy], "mode": mode }))
        },
    )
}
