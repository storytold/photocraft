//! Generic dab-driven stroke machinery for the retouching tools (Clone Stamp, Healing Brush, Spot
//! Healing, Dodge/Burn/Sponge, Blur/Sharpen, Smudge, History Brush).
//!
//! Two stroke shapes cover every retouching tool:
//!
//! * **Accumulate, then composite** ([`stroke_coverage`] + [`apply_coverage`]): the stroke's dabs build
//!   up one coverage map (flow builds up, capped at 1), and a *paint image* computed once from the
//!   pre-stroke state (a cloned source, a healed patch, an earlier history state…) is composited
//!   through it at the stroke's opacity. Pixels painted earlier in the same stroke are never re-sampled.
//! * **Sequential dabs** ([`apply_dab_stroke`]): each dab edits a working copy in place (so dab *n*
//!   sees the result of dab *n − 1*, as Dodge, Blur or Smudge need), and the finished working copy is
//!   mixed back at the stroke's opacity.
//!
//! All pixel data here is the surface's *native* channels as normalised `f32` (any depth, any colour
//! model); colour-space-specific work happens in the callbacks.

use std::collections::HashMap;

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::wrap::{fits, fold_area, wrap_coord};
use crate::{Dab, Stroke, dab_coverage, dabs};

/// A rectangle of native-channel pixels (`rect.width() × rect.height() × ch` floats, row-major).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub rect: Rect,
    pub ch: usize,
    pub data: Vec<f32>,
}

impl Region {
    /// A zeroed region.
    pub fn new(rect: Rect, ch: usize) -> Self {
        Self { rect, ch, data: vec![0.0; rect.width() as usize * rect.height() as usize * ch] }
    }
    /// Read `rect` from a surface.
    pub fn read(s: &Surface, rect: Rect) -> Self {
        Self { rect, ch: s.channels(), data: s.read_region(rect) }
    }
    pub fn width(&self) -> usize {
        self.rect.width() as usize
    }
    pub fn height(&self) -> usize {
        self.rect.height() as usize
    }
    /// Pixel index of document coordinate `(x, y)` (must lie inside `rect`).
    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y - self.rect.y0) as usize * self.width() + (x - self.rect.x0) as usize
    }
    #[inline]
    pub fn px(&self, x: i32, y: i32) -> &[f32] {
        let i = self.index(x, y) * self.ch;
        &self.data[i..i + self.ch]
    }
    #[inline]
    pub fn px_mut(&mut self, x: i32, y: i32) -> &mut [f32] {
        let i = self.index(x, y) * self.ch;
        let n = self.ch;
        &mut self.data[i..i + n]
    }
    /// Copy of the sub-rectangle `r` (clipped to this region).
    pub fn crop(&self, r: Rect) -> Region {
        let r = r.intersect(&self.rect);
        let mut out = Region::new(r, self.ch);
        let (w, n) = (r.width() as usize, self.ch);
        for y in r.y0..r.y1 {
            let s = self.index(r.x0, y) * n;
            let d = (y - r.y0) as usize * w * n;
            out.data[d..d + w * n].copy_from_slice(&self.data[s..s + w * n]);
        }
        out
    }
    /// Write this region into a surface.
    pub fn write(&self, s: &mut Surface) {
        s.write_region(self.rect, &self.data);
    }
    /// Read `rect` from a surface that repeats `wrap` apart (View › Pattern Preview): `rect` may
    /// cross its edges. `None` (or an empty tile) is [`Region::read`].
    pub fn read_wrapped(s: &Surface, rect: Rect, wrap: Option<Rect>) -> Self {
        match wrap.filter(|t| !t.is_empty()) {
            Some(tile) => Self { rect, ch: s.channels(), data: crate::wrap::read(rect, tile, s.channels(), |r| s.read_region(r)) },
            None => Self::read(s, rect),
        }
    }
}

/// One dab's footprint: coverage (hardness falloff × dab alpha) over its bounding rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub dab: Dab,
    /// Position of the dab in the stroke (0 = first).
    pub index: usize,
    pub rect: Rect,
    /// `rect.width() × rect.height()` coverage values in 0..1 (already multiplied by flow/pressure).
    pub cov: Vec<f32>,
}

impl Footprint {
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> f32 {
        self.cov[(y - self.rect.y0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize]
    }
}

/// Bounding rectangle of a dab (1 px of anti-aliasing slack included).
pub fn dab_rect(d: &Dab) -> Rect {
    crate::render::rect_around(d, d.radius)
}

/// Rasterize one dab's coverage.
pub fn footprint(d: &Dab, index: usize, hardness: f32) -> Footprint {
    // A soft tip paints past its nominal radius.
    let rect = crate::render::rect_around(d, crate::tip_reach(d.radius, hardness).max(d.radius));
    let (cx, cy) = (d.center.x as f32, d.center.y as f32);
    let mut cov = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            cov.push(dab_coverage(dist, d.radius, hardness) * d.alpha);
        }
    }
    Footprint { dab: *d, index, rect, cov }
}

/// Union of the dabs' rectangles.
pub fn stroke_bounds(ds: &[Dab]) -> Rect {
    ds.iter().fold(Rect::EMPTY, |r, d| r.union(&dab_rect(d)))
}

/// Accumulated coverage of a whole stroke: dabs build up (`c ← c + d·(ceil − c)`, "flow"), capped at
/// each dab's opacity ceiling; stroke-level masks (dual brush, texture) applied. Returns the stroke
/// bounds and `width × height` coverage values. Uses the full brush engine (tips and dynamics).
pub fn stroke_coverage(stroke: &Stroke) -> (Rect, Vec<f32>) {
    let mut r = crate::StrokeRenderer::new(&stroke.brush, None, 1.0);
    r.push(&stroke.points);
    r.finish();
    r.dense_coverage()
}

/// [`stroke_coverage`] that admits the **rendered** extent before it is made dense: `None` when the
/// stroke covers more than `max_pixels`.
///
/// Rendering itself is tiled, so it holds only the tiles the dabs touch. The bounds are read from
/// those tiles, after tips, dynamics and scattering have placed every dab, so this is the stroke's
/// real extent rather than an estimate from the input points. Only then is the `width × height`
/// buffer allocated, which is what callers need bounded.
pub fn stroke_coverage_within(stroke: &Stroke, max_pixels: u64) -> Option<(Rect, Vec<f32>)> {
    let mut r = crate::StrokeRenderer::new(&stroke.brush, None, 1.0);
    r.push(&stroke.points);
    r.finish();
    let b = r.bounds();
    if u64::from(b.width()).saturating_mul(u64::from(b.height())) > max_pixels {
        return None;
    }
    Some(r.dense_coverage())
}

/// Index of the alpha channel, if the format has one.
#[inline]
pub fn alpha_index(fmt: &PixelFormat) -> Option<usize> {
    fmt.alpha.then(|| fmt.mode.color_channels())
}

/// Normal-mode "source over destination" at coverage `k`, in native channels (colour channels are
/// channel-agnostic, so this works for RGB, Gray, CMYK, Lab… at any depth).
#[inline]
pub fn over_native(fmt: &PixelFormat, dst: &mut [f32], src: &[f32], k: f32, lock_transparency: bool) {
    match alpha_index(fmt) {
        Some(a) => {
            let (da, sa) = (dst[a], src[a] * k);
            if lock_transparency {
                // Alpha kept: source atop, the colour moves by the source alpha alone (#5783).
                for c in 0..a {
                    dst[c] += (src[c] - dst[c]) * sa;
                }
                return;
            }
            let oa = sa + da * (1.0 - sa);
            if oa > 0.0 {
                for c in 0..a {
                    dst[c] = (src[c] * sa + dst[c] * da * (1.0 - sa)) / oa;
                }
            }
            dst[a] = oa;
        }
        None => {
            for c in 0..dst.len() {
                dst[c] += (src[c] - dst[c]) * k;
            }
        }
    }
}

/// Composite a paint image through a coverage map onto `target`.
///
/// `paint` must cover `rect`; `cov` has one value per pixel of `rect`. The per-pixel weight is
/// `cov × opacity × selection`. Non-normal blend modes composite in straight RGBA (W3C formula).
#[allow(clippy::too_many_arguments)]
pub fn apply_coverage(
    target: &mut Surface,
    rect: Rect,
    cov: &[f32],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    paint: &Region,
    blend: BlendMode,
) -> Rect {
    if rect.is_empty() {
        return Rect::EMPTY;
    }
    let fmt = target.format();
    let n = fmt.channels();
    let mut region = target.read_region(rect);
    let w = rect.width() as usize;
    let a = alpha_index(&fmt);
    for (i, c) in cov.iter().enumerate() {
        let x = rect.x0 + (i % w) as i32;
        let y = rect.y0 + (i / w) as i32;
        let sel = selection.map_or(1.0, |s| s.sample_channel(x, y, 0));
        let k = (c * opacity * sel).min(1.0);
        if k <= 0.0 {
            continue;
        }
        let px = &mut region[i * n..(i + 1) * n];
        if lock_transparency && a.is_some_and(|a| px[a] <= 0.0) {
            continue;
        }
        let src = paint.px(x, y);
        if blend == BlendMode::Normal {
            over_native(&fmt, px, src, k, lock_transparency);
        } else {
            let d = to_rgba(&fmt, px);
            let s = to_rgba(&fmt, src);
            let o =
                if lock_transparency { photocraft_color::blend::composite_atop(blend, d, s, k) } else { photocraft_color::blend::composite(blend, d, s, k) };
            let mut enc = [0.0f32; 8];
            from_rgba_into(&fmt, o, &mut enc);
            px.copy_from_slice(&enc[..n]);
        }
    }
    target.write_region(rect, &region);
    rect
}

/// Run a sequential-dab stroke: `effect(work, footprint)` edits the working copy for each dab in
/// order; the result is mixed back at `stroke.brush.opacity × selection`.
///
/// The working copy covers the stroke bounds grown by `halo` pixels so neighbourhood effects (blur,
/// sharpen) can read around each dab; only the stroke bounds are written back. With
/// `lock_transparency`, alpha is preserved. Returns the damaged rectangle.
pub fn apply_dab_stroke(
    target: &mut Surface,
    stroke: &Stroke,
    selection: Option<&Surface>,
    lock_transparency: bool,
    halo: i32,
    mut effect: impl FnMut(&mut Region, &Footprint),
) -> Rect {
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let ctx = crate::BrushContext::new(&stroke.brush);
    let bounds = ds.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
    let outer = Rect::new(bounds.x0 - halo, bounds.y0 - halo, bounds.x1 + halo, bounds.y1 + halo);
    let orig = Region::read(target, outer);
    let mut work = orig.clone();
    for (i, d) in ds.iter().enumerate() {
        effect(&mut work, &ctx.footprint(d, i));
    }
    mix_back(target, &orig, &work, bounds, stroke.brush.opacity, selection, lock_transparency);
    bounds
}

/// [`apply_dab_stroke`] wrapped around `wrap` (View › Pattern Preview): each dab reads and edits
/// the pixels where it repeats on that tile, so a stroke crossing an edge carries on at the
/// opposite one. The dabs run one by one on a [`WorkTiles`] working copy, as live previews run
/// them. `None` (or an empty tile) is [`apply_dab_stroke`]. Returns the damaged rectangle.
pub fn apply_dab_stroke_wrapped(
    target: &mut Surface,
    stroke: &Stroke,
    selection: Option<&Surface>,
    lock_transparency: bool,
    halo: i32,
    wrap: Option<Rect>,
    mut effect: impl FnMut(&mut Region, &Footprint),
) -> Rect {
    let Some(tile) = wrap.filter(|t| !t.is_empty()) else {
        return apply_dab_stroke(target, stroke, selection, lock_transparency, halo, effect);
    };
    let ctx = crate::BrushContext::new(&stroke.brush);
    let mut work = WorkTiles::new(target.channels()).wrapped(Some(tile));
    let mut damage = Rect::EMPTY;
    for (i, d) in dabs(stroke).iter().enumerate() {
        let fp = ctx.footprint(d, i);
        if fp.rect.is_empty() {
            continue;
        }
        let mut region = work.read(target, fp.rect.inflate(halo));
        effect(&mut region, &fp);
        work.write_dab(target, &region, &fp);
        damage = damage.union(&fold_area(fp.rect, tile));
    }
    let written: Vec<Rect> = work.areas().map(|r| r.intersect(&tile)).filter(|r| !r.is_empty()).collect();
    for r in written {
        let edited = work.read(target, r);
        mix_back(target, &Region::read(target, r), &edited, r, stroke.brush.opacity, selection, lock_transparency);
    }
    damage
}

/// Side of the tiles a [`WorkTiles`] working copy keeps its pixels in.
const WORK_TILE: i32 = 128;

/// The working copy of a sequential-dab stroke run dab by dab (live previews,
/// [`apply_dab_stroke_wrapped`]): the target's native channels as `f32`, in tiles copied from the
/// pre-stroke surface the first time a dab writes to them. Unlike a `Surface`, it isn't rounded to
/// the document's depth between dabs, so each dab sees what [`apply_dab_stroke`]'s working copy
/// holds. Wrapped around a tile ([`Self::wrapped`]), it keeps each pixel once, where it lies on
/// the tile, and reads and writes it wherever it repeats.
pub struct WorkTiles {
    ch: usize,
    tiles: HashMap<(i32, i32), Vec<f32>>,
    wrap: Option<Rect>,
}

impl WorkTiles {
    pub fn new(ch: usize) -> Self {
        Self { ch, tiles: HashMap::new(), wrap: None }
    }

    /// Keep the pixels of a stroke wrapped around `tile` (View › Pattern Preview). `None` or an
    /// empty tile doesn't wrap.
    pub fn wrapped(mut self, tile: Option<Rect>) -> Self {
        self.wrap = tile.filter(|t| !t.is_empty());
        self
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

    /// The working pixels of `r`: the pre-stroke pixels `pre` where no dab has written yet.
    /// Wrapped, `r` may cross the tile's edges or be bigger than it.
    pub fn read(&self, pre: &Surface, r: Rect) -> Region {
        match self.wrap {
            Some(tile) => Region { rect: r, ch: self.ch, data: crate::wrap::read(r, tile, self.ch, |part| self.read_at(pre, part).data) },
            None => self.read_at(pre, r),
        }
    }

    fn read_at(&self, pre: &Surface, r: Rect) -> Region {
        let mut out = Region::new(r, self.ch);
        let n = self.ch;
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
        out
    }

    /// Store a dab's edit: the pixels of `region` (which covers the dab) over the dab's rectangle
    /// `fp.rect`. Wrapped, they land where they lie on the tile; a dab bigger than the tile covers
    /// some pixels more than once, and each keeps the copy the dab covers most.
    pub fn write_dab(&mut self, pre: &Surface, region: &Region, fp: &Footprint) {
        let r = fp.rect.intersect(&region.rect);
        let Some(tile) = self.wrap else {
            return self.write_at(pre, &region.crop(r));
        };
        if fits(r, tile) {
            for (part, (dx, dy)) in crate::wrap::pieces(r, tile) {
                let mut piece = region.crop(part.translate(dx, dy));
                piece.rect = part;
                self.write_at(pre, &piece);
            }
            return;
        }
        let area = fold_area(r, tile);
        let mut folded = self.read_at(pre, area);
        let mut most = vec![0.0f32; area.width() as usize * area.height() as usize];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let (c, tx, ty) = (fp.at(x, y), wrap_coord(x, tile.x0, tile.x1), wrap_coord(y, tile.y0, tile.y1));
                if let Some(m) = most.get_mut(folded.index(tx, ty))
                    && c > *m
                {
                    *m = c;
                    folded.px_mut(tx, ty).copy_from_slice(region.px(x, y));
                }
            }
        }
        self.write_at(pre, &folded);
    }

    /// Store `region` as the working pixels of its rectangle.
    fn write_at(&mut self, pre: &Surface, region: &Region) {
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

    /// The rectangles of the tiles dabs have written to.
    pub fn areas(&self) -> impl Iterator<Item = Rect> + '_ {
        self.tiles.keys().map(|&c| Self::tile_rect(c))
    }
}

/// Mix a sequential-dab stroke's working pixels `work` back over the original `orig` at
/// `opacity × selection` inside `rect`, and write that area to `target`: the last step of
/// [`apply_dab_stroke`], shared with live previews that run the dabs as they come.
pub fn mix_back(target: &mut Surface, orig: &Region, work: &Region, rect: Rect, opacity: f32, selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = target.format();
    let a = alpha_index(&fmt);
    let mut out = orig.crop(rect);
    let n = out.ch;
    let opacity = opacity.clamp(0.0, 1.0);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let k = opacity * selection.map_or(1.0, |s| s.sample_channel(x, y, 0));
            if k <= 0.0 {
                continue;
            }
            let wv = work.px(x, y);
            let i = out.index(x, y) * n;
            let o = &mut out.data[i..i + n];
            // Untouched by the dabs: keep the pixel exactly (the premultiplied mix can round it).
            if *o == *wv {
                continue;
            }
            match a {
                // Alpha changes: mix premultiplied, so colour under transparent pixels doesn't bleed in.
                Some(a) if !lock_transparency => mix_premultiplied(o, wv, a, k),
                _ => {
                    for c in 0..n {
                        if lock_transparency && Some(c) == a {
                            continue;
                        }
                        o[c] += (wv[c] - o[c]) * k;
                    }
                }
            }
        }
    }
    out.write(target);
}

/// `dst ← lerp(dst, src, k)` in premultiplied space (straight in, straight out; `a` = alpha index).
/// Where the result is fully transparent the colour is mixed straight, so it stays defined.
#[inline]
pub fn mix_premultiplied(dst: &mut [f32], src: &[f32], a: usize, k: f32) {
    let (Some(&da), Some(&sa)) = (dst.get(a), src.get(a)) else { return };
    let oa = da + (sa - da) * k;
    for (c, (d, s)) in dst.iter_mut().zip(src).enumerate() {
        if c == a {
            continue;
        }
        *d = if oa > 1e-6 { (*d * da + (*s * sa - *d * da) * k) / oa } else { *d + (*s - *d) * k };
    }
    if let Some(d) = dst.get_mut(a) {
        *d = oa;
    }
}

/// Smudge state: a colour buffer carried from dab to dab (centred on the dab), which is laid down at
/// `strength` and then re-picks-up the pixels under the dab. Use with [`apply_dab_stroke`].
#[derive(Clone, Debug)]
pub struct Smudge {
    /// 0..1: how much carried colour each dab lays down (and keeps).
    pub strength: f32,
    /// Finger painting: start the stroke with this native-channel colour instead of the pixels under
    /// the first dab.
    pub finger: Option<Vec<f32>>,
    radius: i32,
    /// Alpha channel index: when set, the carried colour is premultiplied, so smudging into or out
    /// of transparency never drags in the (meaningless) colour of transparent pixels.
    alpha: Option<usize>,
    /// Carried colour (premultiplied when `alpha` is set).
    carry: Option<Vec<f32>>,
}

impl Smudge {
    /// Radius of the carried-colour buffer, including pixels outside a pressure-sized dab.
    /// Both the first pickup and every subsequent pickup need this entire neighbourhood.
    pub fn sampling_radius(max_brush_size: f32) -> i32 {
        ((max_brush_size / 2.0).ceil() as i32).saturating_add(2)
    }

    pub fn new(strength: f32, finger: Option<Vec<f32>>, max_brush_size: f32) -> Self {
        Self { strength: strength.clamp(0.0, 1.0), finger, radius: Self::sampling_radius(max_brush_size), alpha: None, carry: None }
    }

    /// Mix in premultiplied space using alpha channel `a` (the format's [`alpha_index`]).
    pub fn with_alpha(mut self, a: Option<usize>) -> Self {
        self.alpha = a;
        self
    }

    /// Apply one dab to the working copy.
    pub fn dab(&mut self, work: &mut Region, fp: &Footprint) {
        let (r, n) = (self.radius, work.ch);
        let side = (2 * r + 1) as usize;
        let (cx, cy) = (fp.dab.center.x.floor() as i32, fp.dab.center.y.floor() as i32);
        let wr = work.rect;
        let alpha = self.alpha.filter(|a| *a < n);
        let clamp_px = |work: &Region, x: i32, y: i32| -> Vec<f32> {
            let mut v = work.px(x.clamp(wr.x0, wr.x1 - 1), y.clamp(wr.y0, wr.y1 - 1)).to_vec();
            premultiply(&mut v, alpha);
            v
        };
        let carry = self.carry.get_or_insert_with(|| {
            let mut c = vec![0.0f32; side * side * n];
            for qy in 0..side {
                for qx in 0..side {
                    let v = match &self.finger {
                        Some(f) => {
                            let mut f = f.clone();
                            f.resize(n, 1.0);
                            premultiply(&mut f, alpha);
                            f
                        }
                        None => clamp_px(work, cx + qx as i32 - r, cy + qy as i32 - r),
                    };
                    c[(qy * side + qx) * n..(qy * side + qx + 1) * n].copy_from_slice(&v[..n]);
                }
            }
            c
        });
        // Lay down the carried colour.
        for y in fp.rect.y0.max(wr.y0)..fp.rect.y1.min(wr.y1) {
            for x in fp.rect.x0.max(wr.x0)..fp.rect.x1.min(wr.x1) {
                let k = fp.at(x, y) * self.strength;
                let (qx, qy) = (x - cx + r, y - cy + r);
                if k <= 0.0 || qx < 0 || qy < 0 || qx >= side as i32 || qy >= side as i32 {
                    continue;
                }
                let q = (qy as usize * side + qx as usize) * n;
                let px = work.px_mut(x, y);
                match alpha {
                    Some(a) => {
                        // Back to straight for the mix (premultiplied lerp towards the carried colour).
                        let ca = carry[q + a];
                        let mut buf = [0.0f32; 16];
                        let m = n.min(buf.len());
                        buf[..m].copy_from_slice(&carry[q..q + m]);
                        let src = &mut buf[..m];
                        if ca > 1e-6 {
                            for (c, v) in src.iter_mut().enumerate() {
                                if c != a {
                                    *v /= ca;
                                }
                            }
                        }
                        mix_premultiplied(px, src, a, k);
                    }
                    None => {
                        for c in 0..n {
                            px[c] += (carry[q + c] - px[c]) * k;
                        }
                    }
                }
            }
        }
        // Pick up: the carried colour keeps `strength` of itself.
        let keep = self.strength;
        for qy in 0..side {
            for qx in 0..side {
                let (x, y) = (cx + qx as i32 - r, cy + qy as i32 - r);
                if !wr.contains(x, y) {
                    continue;
                }
                let q = (qy * side + qx) * n;
                let px = work.px(x, y);
                let pa = alpha.map_or(1.0, |a| px[a]);
                for c in 0..n {
                    let v = if alpha.is_some_and(|a| a != c) { px[c] * pa } else { px[c] };
                    carry[q + c] = carry[q + c] * keep + v * (1.0 - keep);
                }
            }
        }
    }
}

/// Premultiply a straight pixel in place (no-op without alpha).
fn premultiply(px: &mut [f32], alpha: Option<usize>) {
    let Some(pa) = alpha.and_then(|a| px.get(a).copied()) else { return };
    for (c, v) in px.iter_mut().enumerate() {
        if Some(c) != alpha {
            *v *= pa;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_geom::Point;

    fn stroke(points: &[(f64, f64)], size: f32, hardness: f32) -> Stroke {
        Stroke {
            brush: BrushSettings { size, hardness, spacing: 0.25, pressure_size: false, ..Default::default() },
            points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
        }
    }

    #[test]
    fn coverage_matches_apply_stroke_bounds() {
        let st = stroke(&[(10.0, 10.0), (40.0, 12.0)], 8.0, 1.0);
        let (r, cov) = stroke_coverage(&st);
        assert!(r.contains(10, 10) && r.contains(40, 12));
        let w = r.width() as usize;
        assert!((cov[(10 - r.y0) as usize * w + (25 - r.x0) as usize] - 1.0).abs() < 1e-6);
        assert!(cov.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn coverage_composite_copies_exactly_at_full_hardness() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8, PixelFormat::GRAYA8] {
            let mut s = Surface::new(fmt);
            s.fill_rect(
                Rect::new(0, 0, 64, 64),
                &vec![0.2; fmt.channels()].iter().enumerate().map(|(i, v)| if i + 1 == fmt.channels() { 1.0 } else { *v }).collect::<Vec<_>>(),
            );
            let st = stroke(&[(20.0, 20.0)], 10.0, 1.0);
            let (r, cov) = stroke_coverage(&st);
            let mut paint = Region::new(r, fmt.channels());
            let n = fmt.channels();
            paint.data.iter_mut().enumerate().for_each(|(i, v)| *v = if fmt.alpha && i % n == n - 1 { 1.0 } else { 0.8 });
            apply_coverage(&mut s, r, &cov, 1.0, None, false, &paint, BlendMode::Normal);
            let p = s.pixel(20, 20);
            assert!(p[..n - 1].iter().all(|v| (v - 0.8).abs() < 1e-2) && p[n - 1] == 1.0, "{fmt:?} {p:?}");
            assert!((s.pixel(40, 40)[0] - 0.2).abs() < 1e-2);
        }
    }

    #[test]
    fn coverage_composite_under_lock_blends_source_atop() {
        // Locked half-transparent white, black at coverage 0.5: colour halfway, alpha kept (#5783).
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            for mode in [BlendMode::Normal, BlendMode::Multiply] {
                let mut s = Surface::new(fmt);
                s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 1.0, 1.0, 0.5]);
                let r = Rect::new(0, 0, 4, 4);
                let mut paint = Region::new(r, 4);
                paint.data.chunks_mut(4).for_each(|p| p.copy_from_slice(&[0.0, 0.0, 0.0, 1.0]));
                apply_coverage(&mut s, r, &[1.0; 16], 0.5, None, true, &paint, mode);
                let p = s.rgba(1, 1);
                assert!((p[0] - 0.5).abs() < 0.01 && (p[3] - 0.5).abs() < 0.01, "{fmt:?} {mode:?} {p:?}");
            }
        }
    }

    #[test]
    fn dab_stroke_runs_sequentially_and_respects_selection() {
        let mut s = Surface::new(PixelFormat::GRAY8);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 30, 100), &[1.0]);
        let st = stroke(&[(10.0, 10.0), (50.0, 10.0)], 6.0, 1.0);
        let mut seen = Vec::new();
        apply_dab_stroke(&mut s, &st, Some(&sel), false, 2, |work, fp| {
            seen.push(fp.index);
            for y in fp.rect.y0..fp.rect.y1 {
                for x in fp.rect.x0..fp.rect.x1 {
                    if fp.at(x, y) > 0.5 {
                        work.px_mut(x, y)[0] += 0.1;
                    }
                }
            }
        });
        assert!(seen.windows(2).all(|w| w[1] == w[0] + 1));
        assert!(s.pixel(20, 10)[0] > 0.1, "dabs accumulate");
        assert_eq!(s.pixel(40, 10)[0], 0.0, "outside the selection untouched");
    }

    #[test]
    fn smudge_drags_colour_along_the_stroke() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 100, 40), &[1.0, 1.0, 1.0, 1.0]);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.0, 0.0, 1.0]);
            let st = stroke(&[(20.0, 20.0), (70.0, 20.0)], 12.0, 1.0);
            let mut sm = Smudge::new(0.8, None, 12.0);
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp));
            // Red dragged to the right of the edge, fading with distance.
            let g = |x| s.pixel(x, 20)[1];
            assert!(g(35) < 0.5, "{fmt:?} {}", g(35));
            assert!(g(35) < g(60), "{fmt:?}");
            assert_eq!(s.pixel(35, 35)[1], 1.0, "off-stroke untouched");
            // Finger painting starts with the given colour.
            let mut s2 = Surface::new(fmt);
            s2.fill_rect(Rect::new(0, 0, 100, 40), &[1.0, 1.0, 1.0, 1.0]);
            let mut fg = Smudge::new(0.8, Some(vec![0.0, 0.0, 1.0, 1.0]), 12.0);
            apply_dab_stroke(&mut s2, &st, None, false, 0, |w, fp| fg.dab(w, fp));
            assert!(s2.pixel(22, 20)[0] < 0.5);
        }
    }

    #[test]
    fn smudge_into_transparency_leaves_no_dark_fringe() {
        // #207: straight mixing dragged the black of transparent pixels into the colour.
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.8, 0.0, 1.0]);
            let st = stroke(&[(20.0, 20.0), (70.0, 20.0)], 12.0, 0.5);
            let mut sm = Smudge::new(0.8, None, 12.0).with_alpha(Some(3));
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp));
            let mut smudged = 0;
            for x in 30..80 {
                for y in 14..27 {
                    let p = s.rgba(x, y);
                    if p[3] > 0.02 {
                        smudged += 1;
                        assert!(p[0] > 0.97 && (p[1] - 0.8).abs() < 0.03, "{fmt:?} ({x},{y}): {p:?} darkened");
                    }
                }
            }
            assert!(smudged > 50, "{fmt:?}: colour was dragged into the transparent area");
            // And at partial opacity the mix back is premultiplied too.
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 30, 40), &[1.0, 0.8, 0.0, 1.0]);
            let mut st = st.clone();
            st.brush.opacity = 0.5;
            let mut sm = Smudge::new(0.8, None, 12.0).with_alpha(Some(3));
            apply_dab_stroke(&mut s, &st, None, false, 0, |w, fp| sm.dab(w, fp));
            let p = s.rgba(34, 20);
            assert!(p[3] > 0.02 && p[0] > 0.97, "{fmt:?}: {p:?}");
        }
    }

    #[test]
    fn premultiplied_mix_ignores_colour_of_transparent_pixels() {
        let mut d = [0.0, 0.0, 0.0, 0.0];
        mix_premultiplied(&mut d, &[1.0, 0.5, 0.0, 1.0], 3, 0.25);
        assert_eq!(d, [1.0, 0.5, 0.0, 0.25]);
        let mut d = [0.2, 0.2, 0.2, 1.0];
        mix_premultiplied(&mut d, &[0.0, 0.0, 0.0, 0.0], 3, 1.0);
        assert_eq!(d[3], 0.0);
        // Out-of-range alpha index: untouched, no panic.
        let mut d = [0.5, 0.5];
        mix_premultiplied(&mut d, &[1.0, 1.0], 7, 0.5);
        assert_eq!(d, [0.5, 0.5]);
    }

    #[test]
    fn footprint_stays_inside_the_tips_reach() {
        let radius = 10.5f32;
        for hardness in [1.0, 0.5, 0.0] {
            let dab = Dab::round(Point::new(32.0, 32.0), radius, 1.0);
            let fp = footprint(&dab, 0, hardness);
            let mut painted = 0usize;
            let mut farthest = 0.0f64;
            for y in fp.rect.y0..fp.rect.y1 {
                for x in fp.rect.x0..fp.rect.x1 {
                    let dist = ((x as f64 + 0.5 - dab.center.x).powi(2) + (y as f64 + 0.5 - dab.center.y).powi(2)).sqrt();
                    if fp.at(x, y) <= 0.0 {
                        continue;
                    }
                    painted += 1;
                    farthest = farthest.max(dist);
                    let reach = f64::from(crate::tip_reach(radius, hardness));
                    assert!(
                        dist < reach.max(radius as f64) + 0.5,
                        "hardness {hardness}: pixel ({x}, {y}) at {dist} px from \
                         the centre is painted past the Full Size ring of \
                         {radius} px"
                    );
                }
            }
            assert!(painted > 0, "hardness {hardness}: nothing painted");
            if hardness == 1.0 {
                assert!(
                    farthest > radius as f64,
                    "a hard tip painted only {farthest} px out, so the Full \
                     Size ring at {radius} px is never reached"
                );
            }
        }
    }

    /// The canvas the Pattern Preview strokes below wrap around.
    const TILE: Rect = Rect::new(0, 0, 40, 30);

    /// `area` of a surface showing the canvas repeated: every pixel of the canvas has its own colour.
    fn tiled(fmt: PixelFormat, area: Rect) -> Surface {
        let mut s = Surface::new(fmt);
        let px = (area.y0..area.y1).flat_map(|y| (area.x0..area.x1).map(move |x| (wrap_coord(x, 0, 40), wrap_coord(y, 0, 30))));
        let data: Vec<f32> = px.flat_map(|(x, y)| [x as f32 / 40.0, y as f32 / 30.0, ((x * 7 + y * 3) % 13) as f32 / 13.0, 1.0]).collect();
        s.write_region(area, &data);
        s
    }

    fn smudge(size: f32) -> impl FnMut(&mut Region, &Footprint) {
        let mut sm = Smudge::new(0.8, None, size).with_alpha(Some(3));
        move |work, fp| sm.dab(work, fp)
    }

    #[test]
    fn a_wrapped_sequential_stroke_paints_the_canvas_as_its_tiled_copies_would() {
        // View › Pattern Preview: a smudge across a corner of the canvas copy it is drawn on gives
        // the canvas exactly what it gives the canvas tiled three by three, where it runs into the
        // next copies and carries their colours along. (The same points both times: moved points
        // would place the dabs a rounding apart.)
        let fmt = PixelFormat::RGBA32F;
        let far = stroke(&[(68.0, 48.0), (92.0, 70.0)], 12.0, 0.5);
        let halo = Smudge::sampling_radius(12.0);
        let mut canvas = tiled(fmt, TILE);
        let dmg = apply_dab_stroke_wrapped(&mut canvas, &far, None, false, halo, Some(TILE), smudge(12.0));
        let three = Rect::new(0, 0, 120, 90);
        let (before, mut big) = (tiled(fmt, three), tiled(fmt, three));
        apply_dab_stroke(&mut big, &far, None, false, halo, smudge(12.0));
        let mut changed = 0;
        for y in 0..30 {
            for x in 0..40 {
                let copies = [(40, 30), (80, 30), (40, 60), (80, 60)].map(|(dx, dy)| (x + dx, y + dy));
                let moved: Vec<_> = copies.into_iter().filter(|&(u, v)| big.pixel(u, v) != before.pixel(u, v)).collect();
                let want = match moved.as_slice() {
                    [] => before.pixel(x, y),
                    [(u, v)] => big.pixel(*u, *v),
                    _ => panic!("({x}, {y}): two copies under the stroke"),
                };
                assert_eq!(canvas.pixel(x, y), want, "({x}, {y})");
                changed += usize::from(!moved.is_empty());
            }
        }
        assert!(changed > 300, "the smudge crossed both edges ({changed} px)");
        assert!(dmg.x0 >= 0 && dmg.y0 >= 0 && dmg.x1 <= 40 && dmg.y1 <= 30, "{dmg:?}");
        assert!((-12..0).all(|x| canvas.pixel(x, 20)[3] == 0.0) && (40..52).all(|x| canvas.pixel(x, 20)[3] == 0.0), "nothing off the canvas");
    }

    #[test]
    fn a_wrapped_sequential_stroke_inside_the_canvas_paints_as_before() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA32F] {
            let st = stroke(&[(16.0, 12.0), (24.0, 18.0)], 6.0, 0.8);
            let run = |wrap| {
                let mut s = tiled(fmt, TILE);
                let dmg = apply_dab_stroke_wrapped(&mut s, &st, None, false, Smudge::sampling_radius(6.0), wrap, smudge(6.0));
                (s, dmg)
            };
            let ((plain, d0), (wrapped, d1)) = (run(None), run(Some(TILE)));
            assert_eq!(d0, d1, "{fmt:?}");
            assert!((0..30).all(|y| (0..40).all(|x| plain.pixel(x, y) == wrapped.pixel(x, y))), "{fmt:?}");
            assert!(plain.pixel(20, 15) != tiled(fmt, TILE).pixel(20, 15), "{fmt:?}: the stroke smudged");
        }
    }

    #[test]
    fn a_wrapped_dab_bigger_than_the_canvas_edits_all_of_it_and_nothing_else() {
        let small = Rect::new(0, 0, 10, 8);
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.fill_rect(small, &[1.0, 0.0, 0.0, 1.0]);
        let darken = |work: &mut Region, fp: &Footprint| {
            let r = fp.rect.intersect(&work.rect);
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    work.px_mut(x, y)[0] *= 1.0 - 0.5 * fp.at(x, y);
                }
            }
        };
        let dmg = apply_dab_stroke_wrapped(&mut s, &stroke(&[(5.0, 4.0), (9.0, 6.0)], 30.0, 0.0), None, false, 0, Some(small), darken);
        assert_eq!(dmg, small);
        assert!((0..8).all(|y| (0..10).all(|x| s.pixel(x, y)[0] < 1.0)), "every pixel under the dab's copies darkens");
        assert!((-5..15).all(|x| s.pixel(x, -1)[3] == 0.0 && s.pixel(x, 8)[3] == 0.0), "nothing off the canvas");
    }
}
