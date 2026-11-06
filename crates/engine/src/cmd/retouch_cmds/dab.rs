use super::sampling::{AllLayers, sample_all_layers};
use super::{
    BrushSettings, Document, EngineError, Footprint, IndexedParallelIterator, LayerId, ParallelIterator, ParallelSliceMut, PixelFormat, Rect, Region, Result,
    Session, Smudge, Stroke, StrokePoint, Surface, ToneRange, UNCLIPPED, Value, alpha_index, apply_dab_stroke_clipped, bad, damage_json, dodge_burn, flag,
    footprint_within, from_rgba_into, has_pixel_layer, json, local_blur, local_sharpen, num, parse_brush, run_stroke, sponge, string, to_rgba,
};

// ---------------------------------------------------------------------------------------------
// Toning (Dodge / Burn / Sponge) and focus (Blur / Sharpen), Smudge
// ---------------------------------------------------------------------------------------------

/// Per-dab strength such that one pass over a point totals roughly `e`, whatever the spacing
/// (about `1/spacing` dabs overlap each point on the stroke's centre line).
#[inline]
fn per_dab(e: f32, spacing: f32) -> f32 {
    1.0 - (1.0 - e.clamp(0.0, 1.0)).powf(spacing.clamp(0.01, 1.0))
}

/// Apply an RGB colour transform `f(rgb, strength)` to each covered pixel of a dab (through straight
/// RGBA, so it works for Gray/CMYK/Lab too; untouched pixels are never round-tripped).
fn color_dab(fmt: &PixelFormat, work: &mut Region, fp: &Footprint, strength: f32, spacing: f32, f: impl Fn([f32; 3], f32) -> [f32; 3]) {
    let r = fp.rect.intersect(&work.rect);
    let n = fmt.channels();
    let mut enc = [0.0f32; 8];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let k = per_dab(fp.at(x, y) * strength, spacing);
            if k <= 0.0 {
                continue;
            }
            let px = work.px_mut(x, y);
            let rgba = to_rgba(fmt, px);
            let o = f([rgba[0], rgba[1], rgba[2]], k);
            from_rgba_into(fmt, [o[0], o[1], o[2], rgba[3]], &mut enc);
            let a = alpha_index(fmt);
            for c in 0..n {
                if Some(c) != a {
                    px[c] = enc[c];
                }
            }
        }
    }
}

/// Side of the cells in which Dodge and Burn remember, per touched pixel, the coverage applied so
/// far and the original pixel (copied from the working pixels the first time a dab reaches it).
/// Dense cells rather than a map per pixel: a map lookup per covered pixel made long strokes with
/// large brushes over ten times slower.
const TONE_CELL: i32 = 64;

/// The [`TONE_CELL`] cells, on the layer's grid, that `area` overlaps.
fn tone_cells(area: Rect) -> u64 {
    if area.is_empty() {
        return 0;
    }
    let span = |a: i32, b: i32| u64::try_from(i64::from((b - 1).div_euclid(TONE_CELL)) - i64::from(a.div_euclid(TONE_CELL)) + 1).unwrap_or(0);
    span(area.x0, area.x1).saturating_mul(span(area.y0, area.y1))
}

/// Dodge and Burn: a stroke tones each pixel once, from its original colour, mixed in by the
/// stroke's coverage there (`toned` is the full-exposure colour). Dabs build up coverage as in a
/// brush stroke, capped at full coverage, so Exposure caps a stroke the way Opacity caps a paint
/// stroke: overlapping dabs and passes within one stroke don't compound the tone curve, and a
/// new stroke builds on the last.
fn tone_stroke(fmt: PixelFormat, toned: impl Fn([f32; 3]) -> [f32; 3] + 'static) -> DabEffect {
    struct Cell {
        cov: Vec<f32>,
        orig: Vec<f32>,
    }
    let mut cells: std::collections::HashMap<(i32, i32), Cell> = std::collections::HashMap::new();
    let a = alpha_index(&fmt);
    let n = fmt.channels();
    Box::new(move |work, fp| {
        let r = fp.rect.intersect(&work.rect);
        if r.is_empty() {
            return;
        }
        let mut enc = [0.0f32; 8];
        for cy in r.y0.div_euclid(TONE_CELL)..=(r.y1 - 1).div_euclid(TONE_CELL) {
            for cx in r.x0.div_euclid(TONE_CELL)..=(r.x1 - 1).div_euclid(TONE_CELL) {
                let cr = Rect::new(cx * TONE_CELL, cy * TONE_CELL, cx * TONE_CELL + TONE_CELL, cy * TONE_CELL + TONE_CELL);
                let part = r.intersect(&cr);
                if part.is_empty() {
                    continue;
                }
                let len = (TONE_CELL * TONE_CELL) as usize;
                // Coverage -1: not touched yet, so `orig` isn't filled in for that pixel.
                let cell = cells.entry((cx, cy)).or_insert_with(|| Cell { cov: vec![-1.0; len], orig: vec![0.0; len * n] });
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        let k = fp.at(x, y).clamp(0.0, 1.0);
                        if k <= 0.0 {
                            continue;
                        }
                        let i = ((y - cr.y0) * TONE_CELL + (x - cr.x0)) as usize;
                        let (Some(prev), Some(src)) = (cell.cov.get_mut(i), cell.orig.get_mut(i * n..i * n + n)) else { continue };
                        if *prev < 0.0 {
                            // First touch: the working pixel is still the original.
                            src.copy_from_slice(work.px(x, y));
                            *prev = 0.0;
                        }
                        // Dabs build up the stroke's coverage as a brush stroke's do (flow), up
                        // to full coverage; the tone is applied once, from the original.
                        let k = *prev + k * (1.0 - *prev);
                        if k <= *prev {
                            continue;
                        }
                        *prev = k;
                        let o = to_rgba(&fmt, src);
                        let t = toned([o[0], o[1], o[2]]);
                        let m = [o[0] + (t[0] - o[0]) * k, o[1] + (t[1] - o[1]) * k, o[2] + (t[2] - o[2]) * k];
                        from_rgba_into(&fmt, [m[0], m[1], m[2], o[3]], &mut enc);
                        let px = work.px_mut(x, y);
                        for c in 0..n {
                            if Some(c) != a {
                                px[c] = enc[c];
                            }
                        }
                    }
                }
            }
        }
    })
}

fn tone_range(p: &Value, cmd: &str) -> Result<ToneRange> {
    match string(p, "range", "midtones") {
        "shadows" => Ok(ToneRange::Shadows),
        "midtones" => Ok(ToneRange::Midtones),
        "highlights" => Ok(ToneRange::Highlights),
        o => Err(bad(cmd, format!("unknown range `{o}` (shadows|midtones|highlights)"))),
    }
}

/// A sequential-dab tool's settings (Dodge, Burn, Sponge, Blur, Sharpen, Smudge), parsed once and
/// turned into its per-dab effect, for the commit and the live preview alike.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum DabTool {
    Tone { burn: bool, range: ToneRange, exposure: f32, protect: bool },
    Sponge { saturate: bool, vibrance: bool },
    Focus { sharpen: bool, strength: f32, protect: bool, sigma: f32 },
    Smudge { strength: f32, finger: Option<[f32; 4]>, max_size: f32 },
}

/// One dab's edit of the working copy.
pub(crate) type DabEffect = Box<dyn FnMut(&mut Region, &Footprint)>;

/// Pixels an effect reads beyond the dab it writes. Smudge's pickup is a fixed-size square,
/// independent of pressure, roundness and the dab's current angle.
#[derive(Clone, Copy)]
enum DabNeighborhood {
    Halo(i32),
    Smudge(i32),
}

impl DabNeighborhood {
    fn halo(self) -> i32 {
        match self {
            Self::Halo(radius) | Self::Smudge(radius) => radius,
        }
    }

    fn read_rect(self, rect: Rect, dab: &photocraft_paint::Dab) -> Rect {
        match self {
            Self::Halo(radius) => rect.inflate(radius),
            Self::Smudge(radius) => {
                let (x, y) = (dab.center.x.floor() as i32, dab.center.y.floor() as i32);
                rect.union(&Rect::new(
                    x.saturating_sub(radius),
                    y.saturating_sub(radius),
                    x.saturating_add(radius).saturating_add(1),
                    y.saturating_add(radius).saturating_add(1),
                ))
            }
        }
    }
}

impl DabTool {
    /// The tool for `cmd` and its history label.
    pub(super) fn parse(s: &Session, cmd: &str, p: &Value, stroke: &Stroke) -> Result<(Self, &'static str)> {
        Ok(match cmd {
            "paint.dodge" | "paint.burn" => {
                let burn = cmd == "paint.burn";
                let tool = Self::Tone {
                    burn,
                    range: tone_range(p, cmd)?,
                    exposure: num(p, "exposure", 50.0).clamp(1.0, 100.0) / 100.0,
                    protect: flag(p, "protectTones", true),
                };
                (tool, if burn { "Burn Tool" } else { "Dodge Tool" })
            }
            "paint.sponge" => {
                let saturate = match string(p, "mode", "desaturate") {
                    "desaturate" => false,
                    "saturate" => true,
                    o => return Err(bad(cmd, format!("unknown mode `{o}` (desaturate|saturate)"))),
                };
                (Self::Sponge { saturate, vibrance: flag(p, "vibrance", true) }, "Sponge Tool")
            }
            "paint.blur" | "paint.sharpen" => {
                let sharpen = cmd == "paint.sharpen";
                let tool = Self::Focus {
                    sharpen,
                    strength: num(p, "strength", 50.0).clamp(1.0, 100.0) / 100.0,
                    protect: flag(p, "protectDetail", true),
                    sigma: (stroke.brush.size * 0.03).clamp(1.0, 4.0),
                };
                (tool, if sharpen { "Sharpen Tool" } else { "Blur Tool" })
            }
            "paint.smudge" => {
                let finger = flag(p, "fingerPainting", false).then_some(s.tools.foreground);
                (Self::Smudge { strength: num(p, "strength", 50.0).clamp(1.0, 100.0) / 100.0, finger, max_size: stroke.brush.size }, "Smudge Tool")
            }
            o => return Err(bad(o, "not a sequential-dab tool")),
        })
    }

    /// How far around each dab the effect reads (Blur and Sharpen read a neighbourhood).
    /// Bytes the tool's effect keeps for the whole stroke beside the working copies of `area` (`ch`
    /// channels): Dodge and Burn's [`TONE_CELL`] cells, a coverage value and the original pixel for
    /// each pixel of every cell the area overlaps ([`tone_stroke`]), and Smudge's carried colour
    /// over its whole pickup square. The other tools keep nothing.
    fn retained_bytes(&self, area: Rect, ch: usize) -> u64 {
        let f32s = |n: u64| n.saturating_mul(std::mem::size_of::<f32>() as u64);
        let ch = ch as u64;
        match self {
            Self::Tone { .. } => f32s(tone_cells(area).saturating_mul((TONE_CELL * TONE_CELL) as u64).saturating_mul(ch + 1)),
            Self::Smudge { max_size, .. } => {
                let side = u64::try_from(Smudge::sampling_radius(*max_size)).map_or(0, |r| 2 * r + 1);
                f32s(side.saturating_mul(side).saturating_mul(ch))
            }
            Self::Sponge { .. } | Self::Focus { .. } => 0,
        }
    }

    /// How far past a dab the tool reads (and its footprint reaches past the canvas).
    #[cfg(test)]
    pub(super) fn halo(&self) -> i32 {
        self.neighborhood().halo()
    }

    fn neighborhood(&self) -> DabNeighborhood {
        match self {
            Self::Focus { sigma, .. } => DabNeighborhood::Halo((sigma * 3.0).ceil() as i32 + 1),
            Self::Smudge { max_size, .. } => DabNeighborhood::Smudge(Smudge::sampling_radius(*max_size)),
            _ => DabNeighborhood::Halo(0),
        }
    }

    /// The per-dab effect on a working copy in `fmt`, for a brush of `spacing`.
    pub(super) fn effect(&self, fmt: PixelFormat, spacing: f32) -> DabEffect {
        let sp = spacing;
        match self.clone() {
            Self::Tone { burn, range, exposure, protect } => tone_stroke(fmt, move |c| dodge_burn(c, exposure, range, burn, protect)),
            // Flow is already in each dab's coverage; a full-flow pass moves colours half-way.
            Self::Sponge { saturate, vibrance } => Box::new(move |work, fp| color_dab(&fmt, work, fp, 0.5, sp, |c, k| sponge(c, k, saturate, vibrance))),
            Self::Focus { sharpen, strength, protect, sigma } => {
                let a = alpha_index(&fmt);
                Box::new(move |work, fp| {
                    let r = fp.rect.intersect(&work.rect);
                    if r.is_empty() {
                        return;
                    }
                    let (w, h, n) = (work.width(), work.height(), work.ch);
                    let win = ((r.x0 - work.rect.x0) as usize, (r.y0 - work.rect.y0) as usize, (r.x1 - work.rect.x0) as usize, (r.y1 - work.rect.y0) as usize);
                    let out = if sharpen { local_sharpen(&work.data, w, h, n, a, win, 1.0, protect) } else { local_blur(&work.data, w, h, n, a, win, sigma) };
                    let rw = r.width() as usize;
                    // Row-parallel, like the blur itself (#5426).
                    let (wx0, wy0) = (work.rect.x0, work.rect.y0);
                    work.data.par_chunks_mut(w * n).enumerate().for_each(|(row, line)| {
                        let y = wy0 + row as i32;
                        if y < r.y0 || y >= r.y1 {
                            return;
                        }
                        for x in r.x0..r.x1 {
                            let k = per_dab(fp.at(x, y) * strength, sp);
                            if k <= 0.0 {
                                continue;
                            }
                            let o = ((y - r.y0) as usize * rw + (x - r.x0) as usize) * n;
                            let i = (x - wx0) as usize * n;
                            let px = &mut line[i..i + n];
                            for c in 0..n {
                                px[c] += (out[o + c] - px[c]) * k;
                            }
                        }
                    });
                })
            }
            Self::Smudge { strength, finger, max_size } => {
                let finger_px = finger.map(|fg| photocraft_raster::from_rgba(&fmt, fg));
                // Premultiplied mixing: transparent pixels carry no colour, so no dark fringes.
                let mut sm = Smudge::new(strength, finger_px, max_size).with_alpha(alpha_index(&fmt));
                Box::new(move |work, fp| sm.dab(work, fp))
            }
        }
    }
}

/// Dodge, Burn, Sponge, Blur, Sharpen and Smudge: each dab edits a working copy in order
/// (`apply_dab_stroke_clipped`, on the canvas). Blur, Sharpen and Smudge can sample all layers.
pub(super) fn dab_cmd(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let (stroke, id) = parse_brush(s, p, cmd)?;
    let (tool, label) = DabTool::parse(s, cmd, p, &stroke)?;
    let all = matches!(tool, DabTool::Focus { .. } | DabTool::Smudge { .. }) && sample_all_layers(p);
    // The live stroke the canvas already drew for exactly this commit (`LiveDab::into_commit`):
    // take its pixels instead of running every dab again (the hitch on release).
    let live = s.live_commit.take().filter(|l| l.matches(s, cmd, p, &stroke.points));
    let from_live = live.is_some();
    let dmg = run_stroke(s, label, id, p, |pre, surf, sel, lock| {
        if let Some(l) = live {
            *surf = l.surface;
            return Ok(l.bounds);
        }
        let fmt = surf.format();
        let mut effect = tool.effect(fmt, stroke.brush.spacing);
        let mut sampler = all.then(|| AllLayers::new(pre, fmt));
        // Only the canvas changes, and only it (plus what the effect reads around a dab) is
        // copied, so a brush far larger than the image costs what the image does (#5633). The
        // copies (original, working, mixed back; Sample All Layers adds a composite and its
        // render) and what the effect keeps for the whole stroke (Dodge and Burn's tone cells,
        // Smudge's carry) are priced before they are made.
        let what = format!("the {label} stroke");
        let ch = fmt.channels();
        let admit = |area: Rect| admit_dab_area(area, ch, if all { 5 } else { 3 }, tool.retained_bytes(area, ch), &what);
        let (halo, canvas) = (tool.neighborhood().halo(), pre.bounds());
        apply_dab_stroke_clipped(surf, &stroke, sel, lock, halo, canvas, admit, |work, fp| match &mut sampler {
            Some(all) => all.dab(work, fp, |w, f| effect(w, f)),
            None => effect(work, fp),
        })
    })?;
    let mut r = json!({ "damage": damage_json(dmg) });
    if from_live {
        r["fromLive"] = json!(true);
    }
    Ok(r)
}

/// The most pixels a sequential-dab stroke works on at once: 2³⁰, a 32,768 × 32,768 area. Clipped
/// to the canvas, one dab stays far below it (a soft 5000 px tip covers about 75 megapixels), so
/// only a stroke across a huge canvas reaches it. Tests use a far lower budget, so proving a
/// refusal never comes near allocating the real one.
#[cfg(not(test))]
const DAB_BUDGET: u64 = 1 << 30;
#[cfg(test)]
const DAB_BUDGET: u64 = 1 << 20;

/// Refuse a sequential-dab working area of `ch` channels over [`DAB_BUDGET`], or one the browser
/// can't fit, before anything is allocated: `copies` `f32` buffers of it alive at once, plus the
/// `retained` bytes the effect keeps for the whole stroke ([`DabTool::retained_bytes`]).
fn admit_dab_area(area: Rect, ch: usize, copies: u64, retained: u64, what: &str) -> Result<()> {
    admit_dab_area_with(area, ch, copies, retained, what, crate::memory_gate::ensure)
}

/// [`admit_dab_area`] against the memory check `gate`. `gate` is a parameter so tests can refuse
/// at a fixed limit without the memory gate's process-wide simulation.
fn admit_dab_area_with(area: Rect, ch: usize, copies: u64, retained: u64, what: &str, gate: impl FnOnce(&str, u64) -> Result<()>) -> Result<()> {
    let (w, h) = (u64::from(area.width()), u64::from(area.height()));
    let px = w.saturating_mul(h);
    if px > DAB_BUDGET {
        return Err(crate::allocation::error(what, &format!("it would work on {w} x {h} px at once, more than the {DAB_BUDGET} px limit")));
    }
    let bytes = px.saturating_mul(ch as u64).saturating_mul(std::mem::size_of::<f32>() as u64).saturating_mul(copies);
    gate(&format!("paint {what}"), bytes.saturating_add(retained))?;
    crate::allocation::checkpoint(what)
}

/// What a live sequential-dab stroke's errors call it.
const LIVE_WHAT: &str = "a live stroke preview";

/// What a live stroke's memory checks add beyond each dab's copies: nothing. They run dab by dab,
/// so they may count only what each dab newly allocates; what the stroke keeps across dabs (its
/// working tiles, Dodge and Burn's tone cells, Smudge's carry) builds up as it goes, untracked.
const LIVE_RETAINED: u64 = 0;

/// Side of the tiles a live sequential-dab stroke keeps its working pixels in ([`WorkTiles`]).
const WORK_TILE: i32 = 128;

/// The working copy of a live sequential-dab stroke: the target's native channels as `f32`, in
/// tiles copied from the pre-stroke surface the first time a dab writes to them. Unlike a
/// `Surface`, it isn't rounded to the document's depth between dabs, so each dab sees what the
/// commit's working copy (`apply_dab_stroke_clipped`) holds and the preview is the commit's pixels.
struct WorkTiles {
    ch: usize,
    tiles: std::collections::HashMap<(i32, i32), Vec<f32>>,
}

impl WorkTiles {
    fn new(ch: usize) -> Self {
        Self { ch, tiles: std::collections::HashMap::new() }
    }

    fn tile_rect((tx, ty): (i32, i32)) -> Rect {
        Rect::new(tx * WORK_TILE, ty * WORK_TILE, tx * WORK_TILE + WORK_TILE, ty * WORK_TILE + WORK_TILE)
    }

    /// The tiles `r` overlaps.
    fn coords(r: Rect) -> impl Iterator<Item = (i32, i32)> {
        let (x0, x1) = (r.x0.div_euclid(WORK_TILE), (r.x1 - 1).div_euclid(WORK_TILE));
        let (y0, y1) = (r.y0.div_euclid(WORK_TILE), (r.y1 - 1).div_euclid(WORK_TILE));
        let empty = r.is_empty();
        (y0..=y1).filter(move |_| !empty).flat_map(move |ty| (x0..=x1).map(move |tx| (tx, ty)))
    }

    /// The working pixels of `r`: the pre-stroke pixels `pre` where no dab has written yet. An
    /// error rather than an abort when the allocator can't supply the copy.
    fn read(&self, pre: &Surface, r: Rect) -> Result<Region> {
        let n = self.ch;
        let len = (r.width() as usize)
            .checked_mul(r.height() as usize)
            .and_then(|px| px.checked_mul(n))
            .ok_or_else(|| crate::allocation::error(LIVE_WHAT, "requested size overflowed"))?;
        let mut out = Region { rect: r, ch: n, data: crate::allocation::filled(len, 0.0, LIVE_WHAT)? };
        let mut buf = Vec::new();
        for c in Self::coords(r) {
            let tr = Self::tile_rect(c);
            let part = r.intersect(&tr);
            let w = part.width() as usize * n;
            let tile = self.tiles.get(&c);
            if tile.is_none() {
                pre.read_region_into(part, &mut buf);
            }
            for y in part.y0..part.y1 {
                let src = match tile {
                    Some(t) => t.get(((y - tr.y0) * WORK_TILE + (part.x0 - tr.x0)) as usize * n..).and_then(|s| s.get(..w)),
                    None => buf.get((y - part.y0) as usize * w..).and_then(|s| s.get(..w)),
                };
                let dst = out.index(part.x0, y) * n;
                if let (Some(s), Some(d)) = (src, out.data.get_mut(dst..dst + w)) {
                    d.copy_from_slice(s);
                }
            }
        }
        Ok(out)
    }

    /// Store `region` as the working pixels of its rectangle.
    fn write(&mut self, pre: &Surface, region: &Region) {
        let (r, n) = (region.rect, self.ch);
        for c in Self::coords(r) {
            let tr = Self::tile_rect(c);
            let tile = self.tiles.entry(c).or_insert_with(|| pre.read_region(tr));
            let part = r.intersect(&tr);
            let w = part.width() as usize * n;
            for y in part.y0..part.y1 {
                let dst = ((y - tr.y0) * WORK_TILE + (part.x0 - tr.x0)) as usize * n;
                let src = region.index(part.x0, y) * n;
                if let (Some(s), Some(d)) = (region.data.get(src..src + w), tile.get_mut(dst..dst + w)) {
                    d.copy_from_slice(s);
                }
            }
        }
    }
}

/// A Dodge, Burn, Sponge, Blur, Sharpen or Smudge stroke shown while it is drawn: each new dab
/// runs the tool's effect on a working copy of the target, in order, and the changed area is
/// mixed back over the original at the brush opacity, as `apply_dab_stroke_clipped` does on
/// release. The working copy keeps full precision ([`WorkTiles`]), so the pixels are the
/// commit's, and the commit takes them over ([`Session::offer_live_dab`]) instead of running
/// every dab again. Sample All Layers isn't previewed (its composite spans the whole stroke).
pub struct LiveDab {
    /// The active document with the stroke so far.
    pub doc: std::sync::Arc<Document>,
    cmd: String,
    ctx: photocraft_paint::BrushContext,
    generator: photocraft_paint::dynamics::DabGenerator,
    /// Only the current batch is retained; sequential effects don't use accumulated coverage.
    pending: Vec<photocraft_paint::Dab>,
    dual_pending: Vec<photocraft_paint::Dab>,
    done: usize,
    effect: DabEffect,
    neighborhood: DabNeighborhood,
    /// Where the stroke changes pixels: the canvas for the tools whose commit is clipped to it
    /// ([`Self::begin`]), [`UNCLIPPED`] for the ones whose commit isn't (Color Replacement,
    /// Background Eraser, Impressionist), so each preview is its commit.
    clip: Rect,
    pre_surf: Surface,
    colors: Option<crate::color_channels::ColorChannels>,
    work: WorkTiles,
    id: Option<LayerId>,
    params: Value,
    opacity: f32,
    sel: Option<Surface>,
    lock: bool,
    bounds: Rect,
    /// Show the dabs finishing the stroke would add, so the preview at the last point is the
    /// commit. Only for effects that a repeated dab doesn't change (Color Replacement): the
    /// tail runs on a scratch copy at every push, so an effect that carries paint from dab to
    /// dab (Smudge) would drift.
    preview_tail: bool,
    /// Where the shown tail is.
    tail: Rect,
    /// The tool and brush the stroke started with, which its commit must still have: the
    /// document revision doesn't cover the session's brush or foreground colour. `None` for
    /// strokes whose commit renders itself (Color Replacement).
    tool: Option<(DabTool, Box<BrushSettings>)>,
    /// The document and revision the stroke started on, for strokes started from the session's
    /// active document ([`Self::with_effect`]); `None` for strokes on a prepared copy.
    base: Option<(photocraft_doc::DocId, u64)>,
    /// Every point rendered so far, in order.
    points: Vec<StrokePoint>,
}

impl LiveDab {
    /// Start a live stroke of `cmd` from its params; their `points` are rendered.
    pub fn begin(s: &Session, cmd: &str, p: &Value) -> Result<Self> {
        has_pixel_layer(s).map_err(EngineError::Other)?;
        if sample_all_layers(p) {
            return Err(bad(cmd, "Sample All Layers strokes aren't previewed live"));
        }
        let (stroke, id) = parse_brush(s, p, cmd)?;
        let (tool, _) = DabTool::parse(s, cmd, p, &stroke)?;
        let (neighborhood, spacing, brush) = (tool.neighborhood(), stroke.brush.spacing, Box::new(stroke.brush.clone()));
        let effect = tool.clone();
        // Clipped to the canvas, as `dab_cmd` commits it (#5633).
        let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
        let mut live = Self::with_neighborhood(s, cmd, p, stroke, id, neighborhood, canvas, |pre| effect.effect(pre.format(), spacing))?;
        live.tool = Some((tool, brush));
        Ok(live)
    }

    /// [`Self::begin`] running `effect` at each dab in place of the tool's own, so a test can see
    /// what the clipped stroke hands an effect.
    #[cfg(test)]
    pub(super) fn begin_with_effect(s: &Session, cmd: &str, p: &Value, effect: DabEffect) -> Result<Self> {
        let (stroke, id) = parse_brush(s, p, cmd)?;
        let (tool, _) = DabTool::parse(s, cmd, p, &stroke)?;
        let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
        Self::with_neighborhood(s, cmd, p, stroke, id, tool.neighborhood(), canvas, |_| effect)
    }

    /// How many of the stroke's dabs the preview has run.
    #[cfg(test)]
    pub(super) fn dabs_done(&self) -> usize {
        self.done
    }

    /// Start a live stroke of `cmd` that runs `effect` (made from the target before the stroke)
    /// at each dab of `stroke`, reading `halo` pixels around it. `p` gives the target. Not clipped,
    /// like the commits of the tools that use it (`apply_dab_stroke`).
    pub(crate) fn with_effect(
        s: &Session,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        halo: i32,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        Self::with_neighborhood(s, cmd, p, stroke, id, DabNeighborhood::Halo(halo), UNCLIPPED, effect)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_neighborhood(
        s: &Session,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        neighborhood: DabNeighborhood,
        clip: Rect,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        let captured = crate::channel_cmds::inject_target(s, cmd, p.clone());
        let p = &captured;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let base = (st.doc.id, st.revision);
        let doc = (*st.doc).clone();
        let mut live = Self::with_neighborhood_on(doc, cmd, p, stroke, id, neighborhood, clip, effect)?;
        live.base = Some(base);
        live.colors = crate::color_channels::target(s, p)?;
        if let Some(colors) = live.colors {
            let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut live.doc), live.id, &live.params)?;
            crate::color_channels::restrict_region(&live.pre_surf, surf, colors, live.bounds)?;
        }
        Ok(live)
    }

    /// [`Self::with_effect`] on `doc`, a copy of the active document with what the commit's own
    /// edit does first (the Background Eraser's Background turned into a normal layer, say).
    pub(crate) fn with_effect_on(
        doc: Document,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        halo: i32,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        Self::with_neighborhood_on(doc, cmd, p, stroke, id, DabNeighborhood::Halo(halo), UNCLIPPED, effect)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_neighborhood_on(
        mut doc: Document,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        neighborhood: DabNeighborhood,
        clip: Rect,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        let colors = crate::color_channels::explicit(p, crate::channel_cmds::color_count(&doc))?;
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(&mut doc, id, p)?;
        let pre_surf = surf.clone();
        let effect = effect(&pre_surf);
        // Generate the same dabs as `apply_dab_stroke_clipped` (zoom 1), without a StrokeRenderer:
        // its accumulated coverage isn't used here, and each effect rasterizes its own footprint.
        let ctx = photocraft_paint::BrushContext::new(&stroke.brush);
        let generator = photocraft_paint::dynamics::DabGenerator::new(&stroke.brush, 1.0);
        let mut live = Self {
            doc: std::sync::Arc::new(doc),
            cmd: cmd.into(),
            ctx,
            generator,
            pending: Vec::new(),
            dual_pending: Vec::new(),
            done: 0,
            effect,
            neighborhood,
            clip,
            work: WorkTiles::new(pre_surf.channels()),
            pre_surf,
            colors,
            id,
            params: p.clone(),
            opacity: stroke.brush.opacity,
            sel,
            lock,
            bounds: Rect::EMPTY,
            preview_tail: false,
            tail: Rect::EMPTY,
            tool: None,
            base: None,
            points: Vec::new(),
        };
        live.push(&stroke.points)?;
        Ok(live)
    }

    /// Also show the stroke's finishing dabs (see `preview_tail`).
    pub(crate) fn with_tail_preview(mut self) -> Result<Self> {
        self.preview_tail = true;
        self.push(&[])?;
        Ok(self)
    }

    /// Everything the stroke has touched so far.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Run this batch's primary dabs in order, rasterizing each footprint exactly once.
    fn run_new_dabs(&mut self) -> Result<Rect> {
        let ctx = &self.ctx;
        let mut new = std::mem::take(&mut self.pending);
        // As in `apply_dab_stroke`, sequential effects consume only primary footprints. Keep
        // generating dual dabs so the generator's state advances just as it did in StrokeRenderer.
        self.dual_pending.clear();
        let (clip, ch) = (self.clip, self.work.ch);
        let ring = clip.inflate(self.neighborhood.halo());
        let mut dmg = Rect::EMPTY;
        for (k, d) in new.iter().enumerate() {
            // As in `apply_dab_stroke_clipped`: a dab off the canvas does nothing, and each dab
            // copies only its part of the canvas plus what its effect reads around it (#5633).
            // Its footprint also covers its part of the halo ring past the canvas, which is kept
            // here for later dabs to read, as the commit's working copy keeps it, but never shown.
            // Smudge's region (its pickup square) can be narrower than that ring, and Smudge never
            // picks up from the ring, so its footprint stops at the region.
            let r = ctx.dab_rect(d, false).intersect(&clip);
            if r.is_empty() {
                continue;
            }
            let read = self.neighborhood.read_rect(r, d);
            admit_dab_area(read, ch, 2, LIVE_RETAINED, LIVE_WHAT)?;
            let mut region = self.work.read(&self.pre_surf, read)?;
            let fp = footprint_within(ctx, d, self.done.saturating_add(k), ring.intersect(&read), clip);
            (self.effect)(&mut region, &fp);
            self.work.write(&self.pre_surf, &region.crop(fp.rect));
            dmg = dmg.union(&r);
        }
        self.done = self.done.saturating_add(new.len());
        new.clear();
        self.pending = new;
        Ok(dmg)
    }

    /// Mix the working pixels of `r` back over the original into the shown document.
    fn show(&mut self, r: Rect, work: &Region) -> Result<()> {
        let orig = Region { rect: r, ch: self.work.ch, data: crate::allocation::read_region(&self.pre_surf, r, LIVE_WHAT)? };
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        photocraft_paint::retouch::mix_back(surf, &orig, work, r, self.opacity, self.sel.as_ref(), self.lock);
        Ok(())
    }

    /// The working pixels of `r`, for [`Self::show`]: priced first, with the original and the
    /// mixed result alongside.
    fn work_for_show(&self, r: Rect) -> Result<Region> {
        admit_dab_area(r, self.work.ch, 3, LIVE_RETAINED, LIVE_WHAT)?;
        self.work.read(&self.pre_surf, r)
    }

    /// Render more points; returns the rectangle that changed.
    /// Invalid coordinates reject the whole batch without changing the preview.
    /// Only the position and pressure of `pts` are used: the commands take no tilt or times.
    pub fn push(&mut self, pts: &[StrokePoint]) -> Result<Rect> {
        let damage = self.push_inner(pts)?;
        if let Some(colors) = self.colors {
            let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
            crate::color_channels::restrict_region(&self.pre_surf, surf, colors, damage)?;
        }
        Ok(damage)
    }

    fn push_inner(&mut self, pts: &[StrokePoint]) -> Result<Rect> {
        crate::brush_cmds::check_coords(pts, &self.cmd)?;
        let pts: Vec<_> = pts.iter().map(|q| StrokePoint::new(q.x, q.y, q.pressure)).collect();
        self.points.extend_from_slice(&pts);
        self.generator.push(&pts, &mut self.pending, &mut self.dual_pending);
        let shown = self.render_batch();
        if shown.is_err() {
            // The batch stopped part-way (a copy over the budget, or one the allocator couldn't
            // supply): these pixels are no longer the command's, so its commit renders itself.
            self.tool = None;
        }
        shown
    }

    /// Run the generated dabs and show them (with the finishing dabs, if previewed).
    fn render_batch(&mut self) -> Result<Rect> {
        let dmg = self.run_new_dabs()?;
        let ctx = &self.ctx;
        let (clip, halo) = (self.clip, self.neighborhood.halo());
        // The finishing dabs, run on a scratch copy of the working pixels.
        let mut tail = None;
        if self.preview_tail {
            let mut generator = self.generator.clone();
            let (mut ds, mut dual) = (Vec::new(), Vec::new());
            generator.finish(&mut ds, &mut dual);
            let r = ds.iter().fold(Rect::EMPTY, |a, d| a.union(&ctx.dab_rect(d, false).intersect(&clip)));
            if !r.is_empty() {
                let read = r.inflate(halo);
                admit_dab_area(read, self.work.ch, 2, LIVE_RETAINED, LIVE_WHAT)?;
                let mut region = self.work.read(&self.pre_surf, read)?;
                for (k, d) in ds.iter().enumerate() {
                    if ctx.dab_rect(d, false).intersect(&clip).is_empty() {
                        continue;
                    }
                    (self.effect)(&mut region, &footprint_within(ctx, d, self.done.saturating_add(k), clip.inflate(halo), clip));
                }
                tail = Some((r, region.crop(r)));
            }
        }
        // Back to the stroke without the previous tail.
        let old = std::mem::replace(&mut self.tail, Rect::EMPTY);
        let shown = dmg.union(&old);
        let tail_rect = tail.as_ref().map_or(Rect::EMPTY, |(r, _)| *r);
        if shown.is_empty() && tail_rect.is_empty() {
            return Ok(Rect::EMPTY);
        }
        self.bounds = self.bounds.union(&dmg).union(&tail_rect);
        if !shown.is_empty() {
            let work = self.work_for_show(shown)?;
            self.show(shown, &work)?;
        }
        if let Some((r, work)) = tail {
            self.show(r, &work)?;
            self.tail = r;
        }
        Ok(shown.union(&tail_rect))
    }

    /// The finished stroke as its commit would take it over ([`Session::offer_live_dab`]): the
    /// finishing dabs are run, and the pixels are the ones the command with the same params and
    /// points paints. Only while the brush, tool settings and layer are still the ones the stroke
    /// started with: the document revision doesn't cover those.
    pub(crate) fn into_commit(mut self, s: &Session) -> Result<crate::brush_cmds::LiveCommit> {
        let (Some((tool, brush)), Some(base)) = (self.tool.take(), self.base) else {
            return Err(bad(&self.cmd, "this stroke's commit renders itself"));
        };
        let (stroke, id) = parse_brush(s, &self.params, &self.cmd)?;
        let (now, _) = DabTool::parse(s, &self.cmd, &self.params, &stroke)?;
        if now != tool || stroke.brush != *brush || id != self.id {
            return Err(bad(&self.cmd, "the brush, tool options or layer changed during the stroke"));
        }
        self.generator.finish(&mut self.pending, &mut self.dual_pending);
        let dmg = self.run_new_dabs()?;
        let shown = dmg.union(&self.tail);
        if !shown.is_empty() {
            let work = self.work_for_show(shown)?;
            self.show(shown, &work)?;
        }
        self.bounds = self.bounds.union(&dmg);
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        if let Some(colors) = self.colors {
            crate::color_channels::restrict_region(&self.pre_surf, surf, colors, shown)?;
        }
        let surface = surf.clone();
        Ok(crate::brush_cmds::LiveCommit {
            mixer: None,
            cmd: self.cmd,
            base,
            params: self.params,
            seed: None,
            points: self.points,
            surface,
            bounds: self.bounds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A memory check that admits up to `limit` bytes.
    fn up_to(limit: u64) -> impl FnOnce(&str, u64) -> Result<()> {
        move |what, need| if need <= limit { Ok(()) } else { Err(EngineError::Other(format!("{what}: {need} bytes, {limit} available"))) }
    }

    #[test]
    fn dab_admission_counts_what_the_effect_keeps_for_the_stroke() {
        // A 290 × 190 px RGBA working area, at the commit's 3 copies. Dodge and Burn also keep the
        // 5 × 4 tone cells it overlaps (64 px squares on the layer's grid, a coverage value and the
        // original pixel per pixel); a 200 px Smudge keeps its 205 px pickup square's carry. With
        // memory for exactly the copies, the estimate before counted neither, and both fit; now
        // they are refused, and admitted once the memory covers them too.
        let (area, ch) = (Rect::new(10, 10, 300, 200), 4);
        let copies: u64 = 290 * 190 * 4 * 4 * 3;
        let dodge = DabTool::Tone { burn: false, range: ToneRange::Midtones, exposure: 0.5, protect: true };
        let smudge = DabTool::Smudge { strength: 0.5, finger: None, max_size: 200.0 };
        assert_eq!(dodge.retained_bytes(area, ch), 5 * 4 * (64 * 64) * (4 + 1) * 4);
        assert_eq!(smudge.retained_bytes(area, ch), 205 * 205 * 4 * 4);
        for tool in [&dodge, &smudge] {
            let retained = tool.retained_bytes(area, ch);
            assert!(admit_dab_area_with(area, ch, 3, 0, "t", up_to(copies)).is_ok(), "{tool:?}: the copies alone fit");
            assert!(admit_dab_area_with(area, ch, 3, retained, "t", up_to(copies)).is_err(), "{tool:?}: refused");
            assert!(admit_dab_area_with(area, ch, 3, retained, "t", up_to(copies + retained)).is_ok(), "{tool:?}: fits");
        }
        // Sponge, Blur and Sharpen keep nothing beyond the copies.
        for tool in [DabTool::Sponge { saturate: true, vibrance: false }, DabTool::Focus { sharpen: false, strength: 0.5, protect: false, sigma: 2.0 }] {
            assert_eq!(tool.retained_bytes(area, ch), 0, "{tool:?}");
        }
        // Cells are whole and on the layer's grid: 2 px across a cell edge keep two of them, a pixel
        // left of and above the origin keeps the cell there, and no area keeps none.
        let cell = (64 * 64) * (4 + 1) * 4;
        assert_eq!(dodge.retained_bytes(Rect::new(63, 0, 65, 1), ch), 2 * cell);
        assert_eq!(dodge.retained_bytes(Rect::new(-1, -1, 0, 0), ch), cell);
        assert_eq!(dodge.retained_bytes(Rect::EMPTY, ch), 0);
    }
}
