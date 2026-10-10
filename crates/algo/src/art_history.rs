//! Oriented history-state dabs: sample a snapshot patch, pose it along the stroke, and stamp.
//!
//! Tile processing reads a halo around each destination tile so a dab whose centre sits one pixel
//! inside a tile still sees the same source as an untiled pass. Scatter is seeded, so the same
//! stroke is replayable.

#![allow(clippy::needless_range_loop)]

/// Mixes the history-state index into dab scatter so two documents with different snapshots diverge.
pub const RNG_SEED: u32 = 0xA7B1_57E0;

/// Largest number of dabs one stroke may emit.
pub const MAX_DABS: usize = 4096;

/// Largest coverage rectangle, in pixels, a stroke may allocate.
pub const MAX_COVERAGE: u64 = 4096 * 4096;

/// Why [`stamp`] refused to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampError {
    /// A zero-sized buffer, no dabs, or nothing to cover.
    Empty,
    /// Source and destination disagree on channel count.
    Mismatched,
    /// The read/write window would exceed [`MAX_COVERAGE`].
    TooLarge,
}

/// Named dab layout. Multipliers are documented in the command params; keep them stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    TightShort,
    TightMedium,
    TightLong,
    LooseMedium,
    LooseLong,
    Dab,
    TightCurl,
    LooseCurl,
}

impl Style {
    /// Parse a command-style identifier (`tightMedium`, …).
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "tightShort" => Self::TightShort,
            "tightMedium" => Self::TightMedium,
            "tightLong" => Self::TightLong,
            "looseMedium" => Self::LooseMedium,
            "looseLong" => Self::LooseLong,
            "dab" => Self::Dab,
            "tightCurl" => Self::TightCurl,
            "looseCurl" => Self::LooseCurl,
            _ => return None,
        })
    }

    /// Stable id used in command params.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TightShort => "tightShort",
            Self::TightMedium => "tightMedium",
            Self::TightLong => "tightLong",
            Self::LooseMedium => "looseMedium",
            Self::LooseLong => "looseLong",
            Self::Dab => "dab",
            Self::TightCurl => "tightCurl",
            Self::LooseCurl => "looseCurl",
        }
    }

    fn spacing_mul(self) -> f32 {
        match self {
            Self::TightShort => 0.35,
            Self::TightMedium => 0.55,
            Self::TightLong => 0.85,
            Self::LooseMedium => 0.70,
            Self::LooseLong => 1.10,
            Self::Dab => 1.60,
            Self::TightCurl => 0.50,
            Self::LooseCurl => 0.80,
        }
    }

    fn scatter_mul(self) -> f32 {
        match self {
            Self::TightShort => 0.00,
            Self::TightMedium => 0.00,
            Self::TightLong => 0.05,
            Self::LooseMedium => 0.45,
            Self::LooseLong => 0.70,
            Self::Dab => 0.10,
            Self::TightCurl => 0.05,
            Self::LooseCurl => 0.40,
        }
    }

    fn curl_deg(self) -> f32 {
        match self {
            Self::TightCurl => 18.0,
            Self::LooseCurl => 28.0,
            _ => 0.0,
        }
    }

    fn size_mul(self) -> f32 {
        match self {
            Self::TightShort => 0.7,
            Self::TightMedium => 0.9,
            Self::TightLong => 1.1,
            Self::LooseMedium => 1.0,
            Self::LooseLong => 1.2,
            Self::Dab => 1.0,
            Self::TightCurl => 0.9,
            Self::LooseCurl => 1.1,
        }
    }
}

/// One posed dab in document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub x: f32,
    pub y: f32,
    pub theta: f32,
    pub radius: f32,
    pub pressure: f32,
}

/// Spacing along the stroke, in pixels.
pub fn spacing_px(size: f32, area: f32, style: Style) -> Option<f32> {
    if !(size.is_finite() && area.is_finite()) || size <= 0.0 {
        return None;
    }
    let s = size * style.spacing_mul() * (0.25 + 0.02 * area);
    if s.is_finite() && s > 0.0 { Some(s) } else { None }
}

/// Extra pixels each destination tile must read from the source.
pub fn halo_radius(size: f32, style: Style) -> Option<i32> {
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let v = (size * style.size_mul() / 2.0 + style.scatter_mul() * size + 2.0).ceil();
    if !v.is_finite() || v > 1_000_000.0 {
        return None;
    }
    Some(v as i32)
}

/// Side of the rotated source square (`ceil(size · size_mul) + 2`).
pub fn sample_side(size: f32, style: Style) -> Option<i32> {
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let v = (size * style.size_mul()).ceil() + 2.0;
    if !v.is_finite() || v > 1_000_000.0 {
        return None;
    }
    Some(v as i32)
}

fn mix32(seed: u32, dab: u32, lane: u32) -> u32 {
    let mut x = seed ^ dab.wrapping_mul(0x9E37_79B9) ^ lane.wrapping_mul(0x85EB_CA6B);
    x = x.wrapping_add(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}

fn signed01(seed: u32, dab: u32, lane: u32) -> f32 {
    let u = mix32(seed, dab, lane) >> 8;
    (u as f32) * (2.0 / 16_777_215.0) - 1.0
}

fn dab_radius(size: f32, style: Style) -> Option<f32> {
    let r = size * style.size_mul() * 0.5;
    if r.is_finite() && r > 0.0 { Some(r) } else { None }
}

/// Walk `points` and emit posed dabs. `seed` is typically [`RNG_SEED`] XOR the history index.
pub fn place_dabs(points: &[[f32; 2]], size: f32, area: f32, style: Style, seed: u32) -> Result<Vec<Dab>, StampError> {
    if points.is_empty() {
        return Err(StampError::Empty);
    }
    let spacing = spacing_px(size, area, style).ok_or(StampError::Empty)?;
    let radius = dab_radius(size, style).ok_or(StampError::TooLarge)?;
    let curl = style.curl_deg().to_radians();
    let scatter = style.scatter_mul();
    let mut out = Vec::new();
    let mut emit = |i: u32, x: f32, y: f32, tx: f32, ty: f32, pressure: f32| -> Result<(), StampError> {
        if !(x.is_finite() && y.is_finite() && tx.is_finite() && ty.is_finite()) {
            return Err(StampError::Empty);
        }
        let u = signed01(seed, i, 1);
        let v = signed01(seed, i, 2);
        let w = signed01(seed, i, 3);
        let dx = scatter * size * u;
        let dy = scatter * size * v;
        let (cx, cy) = (x + dx, y + dy);
        if !(cx.is_finite() && cy.is_finite()) {
            return Err(StampError::TooLarge);
        }
        let theta = ty.atan2(tx) + (i as f32) * curl + scatter * w;
        if !theta.is_finite() {
            return Err(StampError::Empty);
        }
        if out.len() >= MAX_DABS {
            return Err(StampError::TooLarge);
        }
        out.push(Dab { x: cx, y: cy, theta, radius, pressure: pressure.clamp(0.0, 1.0) });
        Ok(())
    };

    let Some(first) = points.first() else {
        return Err(StampError::Empty);
    };
    if !first[0].is_finite() || !first[1].is_finite() {
        return Err(StampError::Empty);
    }
    let (mut px, mut py) = (first[0], first[1]);
    let (mut tx, mut ty) = (1.0f32, 0.0f32);
    if let Some(second) = points.get(1) {
        tx = second[0] - px;
        ty = second[1] - py;
        if tx == 0.0 && ty == 0.0 {
            tx = 1.0;
        }
    }
    emit(0, px, py, tx, ty, 1.0)?;

    let mut carry = 0.0f32;
    let mut i = 1u32;
    for w in points.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) {
            return Err(StampError::Empty);
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if !len.is_finite() {
            return Err(StampError::Empty);
        }
        if len < 1.0e-6 {
            continue;
        }
        tx = dx;
        ty = dy;
        let mut consumed = 0.0f32;
        while carry + (len - consumed) >= spacing {
            let need = spacing - carry;
            if !need.is_finite() || need < 0.0 {
                return Err(StampError::Empty);
            }
            consumed += need;
            if consumed > len {
                break;
            }
            let t = consumed / len;
            px = a[0] + dx * t;
            py = a[1] + dy * t;
            emit(i, px, py, tx, ty, 1.0)?;
            i = i.saturating_add(1);
            carry = 0.0;
        }
        carry += len - consumed;
        if !carry.is_finite() {
            return Err(StampError::Empty);
        }
    }
    if out.is_empty() { Err(StampError::Empty) } else { Ok(out) }
}

/// Bounding rectangle of a dab, in document pixels (1 px of anti-alias slack).
pub fn dab_bounds(dab: &Dab) -> Option<(i32, i32, i32, i32)> {
    let r = dab.radius + 1.0;
    if !r.is_finite() {
        return None;
    }
    let x0 = (dab.x - r).floor();
    let y0 = (dab.y - r).floor();
    let x1 = (dab.x + r).ceil();
    let y1 = (dab.y + r).ceil();
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return None;
    }
    let clamp = |v: f32| v.clamp(i32::MIN as f32, i32::MAX as f32) as i32;
    Some((clamp(x0), clamp(y0), clamp(x1), clamp(y1)))
}

fn coverage_at(dab: &Dab, x: i32, y: i32, hardness: f32) -> f32 {
    let dx = x as f32 + 0.5 - dab.x;
    let dy = y as f32 + 0.5 - dab.y;
    let d = dx.hypot(dy);
    if !d.is_finite() || d >= dab.radius + 0.5 {
        return 0.0;
    }
    let edge = (dab.radius + 0.5 - d).clamp(0.0, 1.0);
    let h = hardness.clamp(0.0, 1.0);
    let inner = dab.radius * h;
    let falloff = if d <= inner || dab.radius - inner < 1.0e-3 {
        1.0
    } else {
        let t = ((d - inner) / (dab.radius - inner)).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    };
    edge * falloff
}

fn pixel_index(x: usize, y: usize, w: usize, h: usize, ch: usize) -> Option<usize> {
    if x >= w || y >= h || ch == 0 {
        return None;
    }
    y.checked_mul(w)?.checked_add(x)?.checked_mul(ch)
}

fn origin_index(origin: (i32, i32), x: i32, y: i32, w: usize, h: usize, ch: usize) -> Option<usize> {
    let dx = x.checked_sub(origin.0)?;
    let dy = y.checked_sub(origin.1)?;
    pixel_index(usize::try_from(dx).ok()?, usize::try_from(dy).ok()?, w, h, ch)
}

fn floor_i32(v: f32) -> i32 {
    v.clamp(-2_147_483_648.0, 2_147_483_647.0).floor() as i32
}

struct SrcView<'a> {
    data: &'a [f32],
    w: usize,
    h: usize,
    origin: (i32, i32),
    ch: usize,
}

fn sample_bilinear(src: SrcView<'_>, sx: f32, sy: f32, out: &mut [f32]) -> bool {
    let ch = src.ch;
    if ch == 0 || out.len() < ch {
        return false;
    }
    let dst = &mut out[..ch];
    dst.fill(0.0);
    if !(sx.is_finite() && sy.is_finite()) {
        return false;
    }
    let lx = sx - src.origin.0 as f32;
    let ly = sy - src.origin.1 as f32;
    if !(lx.is_finite() && ly.is_finite()) {
        return false;
    }
    let x0 = floor_i32(lx);
    let y0 = floor_i32(ly);
    let fx = (lx - x0 as f32).clamp(0.0, 1.0);
    let fy = (ly - y0 as f32).clamp(0.0, 1.0);
    let mut any = false;
    for (iy, wy) in [(0i32, 1.0 - fy), (1i32, fy)] {
        for (ix, wx) in [(0i32, 1.0 - fx), (1i32, fx)] {
            let wt = wx * wy;
            if wt <= 0.0 {
                continue;
            }
            let px = x0.saturating_add(ix);
            let py = y0.saturating_add(iy);
            if px < 0 || py < 0 {
                continue;
            }
            let (Ok(ux), Ok(uy)) = (usize::try_from(px), usize::try_from(py)) else { continue };
            let Some(base) = pixel_index(ux, uy, src.w, src.h, ch) else { continue };
            let Some(pxs) = src.data.get(base..base.saturating_add(ch)) else { continue };
            for (d, s) in dst.iter_mut().zip(pxs.iter()) {
                *d += *s * wt;
            }
            any = true;
        }
    }
    any
}

fn rotated_source(dab: &Dab, x: i32, y: i32) -> (f32, f32) {
    let dx = x as f32 + 0.5 - dab.x;
    let dy = y as f32 + 0.5 - dab.y;
    let (c, s) = (dab.theta.cos(), dab.theta.sin());
    (dab.x + c * dx + s * dy, dab.y - s * dx + c * dy)
}

fn over_native(dst: &mut [f32], src: &[f32], k: f32, alpha: Option<usize>, lock: bool) {
    if k <= 0.0 {
        return;
    }
    match alpha {
        Some(a) => {
            let Some((&da, &sa0)) = dst.get(a).zip(src.get(a)) else { return };
            let sa = sa0 * k;
            let oa = sa + da * (1.0 - sa);
            if oa > 0.0 {
                for c in 0..a {
                    let Some(d) = dst.get_mut(c) else { continue };
                    let Some(&s) = src.get(c) else { continue };
                    *d = (s * sa + *d * da * (1.0 - sa)) / oa;
                }
            }
            if !lock && let Some(d) = dst.get_mut(a) {
                *d = oa;
            }
        }
        None => {
            for (d, s) in dst.iter_mut().zip(src.iter()) {
                *d += (*s - *d) * k;
            }
        }
    }
}

fn buf_len(w: usize, h: usize, ch: usize) -> Option<usize> {
    w.checked_mul(h)?.checked_mul(ch)
}

/// Parameters for [`stamp`].
pub struct StampParams<'a> {
    pub dest: &'a mut [f32],
    pub dest_w: usize,
    pub dest_h: usize,
    pub dest_origin: (i32, i32),
    pub src: &'a [f32],
    pub src_w: usize,
    pub src_h: usize,
    pub src_origin: (i32, i32),
    pub ch: usize,
    pub dabs: &'a [Dab],
    pub hardness: f32,
    pub opacity: f32,
    pub flow: f32,
    pub tolerance: f32,
    /// Destination tile edge; `0` processes the whole buffer as one tile.
    pub tile: i32,
    pub halo: i32,
    pub selection: Option<&'a [f32]>,
    pub lock_transparent: bool,
    pub alpha: Option<usize>,
}

fn dest_pixels(p: &StampParams<'_>) -> Result<u64, StampError> {
    let n = buf_len(p.dest_w, p.dest_h, p.ch).ok_or(StampError::TooLarge)?;
    if p.dest.len() < n {
        return Err(StampError::Empty);
    }
    let sn = buf_len(p.src_w, p.src_h, p.ch).ok_or(StampError::TooLarge)?;
    if p.src.len() < sn {
        return Err(StampError::Empty);
    }
    if p.ch == 0 || p.dest_w == 0 || p.dest_h == 0 {
        return Err(StampError::Empty);
    }
    Ok(n as u64)
}

fn tile_grid(w: usize, h: usize, origin: (i32, i32), tile: i32) -> Vec<(i32, i32, i32, i32)> {
    let (x0, y0) = origin;
    let x1 = x0.saturating_add(w.min(i32::MAX as usize) as i32);
    let y1 = y0.saturating_add(h.min(i32::MAX as usize) as i32);
    if tile <= 0 {
        return vec![(x0, y0, x1, y1)];
    }
    let mut tiles = Vec::new();
    let mut y = y0;
    while y < y1 {
        let ny = y.saturating_add(tile).min(y1);
        let mut x = x0;
        while x < x1 {
            let nx = x.saturating_add(tile).min(x1);
            tiles.push((x, y, nx, ny));
            x = nx;
        }
        y = ny;
    }
    tiles
}

fn stamp_dab_on_rect(p: &mut StampParams<'_>, dab: &Dab, rect: (i32, i32, i32, i32), src_rect: (i32, i32, i32, i32)) -> Result<u32, StampError> {
    let (x0, y0, x1, y1) = rect;
    if x1 <= x0 || y1 <= y0 {
        return Ok(0);
    }
    let (sx0, sy0, sx1, sy1) = src_rect;
    let Some(tw) = (x1 as i64 - x0 as i64).try_into().ok().filter(|w: &usize| *w > 0) else {
        return Err(StampError::TooLarge);
    };
    let Some(th) = (y1 as i64 - y0 as i64).try_into().ok().filter(|h: &usize| *h > 0) else {
        return Err(StampError::TooLarge);
    };
    let area = (tw as u64).saturating_mul(th as u64);
    if area > MAX_COVERAGE {
        return Err(StampError::TooLarge);
    }
    let n = tw.checked_mul(th).and_then(|m| m.checked_mul(p.ch)).ok_or(StampError::TooLarge)?;
    let mut local = vec![0.0f32; n];
    let mut cov = vec![0.0f32; tw.checked_mul(th).ok_or(StampError::TooLarge)?];
    // Copy dest tile into local.
    for y in y0..y1 {
        for x in x0..x1 {
            let lx = (x - x0) as usize;
            let ly = (y - y0) as usize;
            let Some(di) = origin_index(p.dest_origin, x, y, p.dest_w, p.dest_h, p.ch) else { continue };
            let Some(li) = pixel_index(lx, ly, tw, th, p.ch) else { continue };
            if let (Some(dst), Some(src)) = (local.get_mut(li..li + p.ch), p.dest.get(di..di + p.ch)) {
                dst.copy_from_slice(src);
            }
            if let Some(c) = cov.get_mut(ly.saturating_mul(tw).saturating_add(lx)) {
                *c = coverage_at(dab, x, y, p.hardness);
            }
        }
    }

    // Cropped source window (halo). Samples outside it look empty — that is the seam if halo is short.
    let sw = (sx1 - sx0).max(0) as usize;
    let sh = (sy1 - sy0).max(0) as usize;
    let sn = sw.checked_mul(sh).and_then(|m| m.checked_mul(p.ch)).ok_or(StampError::TooLarge)?;
    let mut src_win = vec![0.0f32; sn];
    for y in sy0..sy1 {
        for x in sx0..sx1 {
            let Some(si) = origin_index(p.src_origin, x, y, p.src_w, p.src_h, p.ch) else { continue };
            let Some(wi) = origin_index((sx0, sy0), x, y, sw, sh, p.ch) else { continue };
            if let (Some(d), Some(s)) = (src_win.get_mut(wi..wi + p.ch), p.src.get(si..si + p.ch)) {
                d.copy_from_slice(s);
            }
        }
    }

    let mut sampled = vec![0.0f32; p.ch];
    let mut written = 0u32;
    let k_stroke = (p.opacity * p.flow * dab.pressure).clamp(0.0, 1.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let lx = (x - x0) as usize;
            let ly = (y - y0) as usize;
            let Some(&c) = cov.get(ly.saturating_mul(tw).saturating_add(lx)) else { continue };
            if c <= 0.0 {
                continue;
            }
            let mut k = c * k_stroke;
            if let Some(sel) = p.selection {
                let Some(si) = origin_index(p.dest_origin, x, y, p.dest_w, p.dest_h, 1) else { continue };
                let Some(&sv) = sel.get(si) else { continue };
                k *= sv.clamp(0.0, 1.0);
            }
            if k <= 0.0 {
                continue;
            }
            let (sx, sy) = rotated_source(dab, x, y);
            if !sample_bilinear(SrcView { data: &src_win, w: sw, h: sh, origin: (sx0, sy0), ch: p.ch }, sx, sy, &mut sampled) {
                continue;
            }
            let Some(li) = pixel_index(lx, ly, tw, th, p.ch) else { continue };
            let Some(dst) = local.get_mut(li..li + p.ch) else { continue };
            if p.lock_transparent && p.alpha.is_some_and(|a| dst.get(a).is_some_and(|&v| v <= 0.0)) {
                continue;
            }
            over_native(dst, &sampled, k, p.alpha, p.lock_transparent);
            written = written.saturating_add(1);
        }
    }

    // Write the dest tile back (not the halo).
    for y in y0..y1 {
        for x in x0..x1 {
            let lx = (x - x0) as usize;
            let ly = (y - y0) as usize;
            let Some(di) = origin_index(p.dest_origin, x, y, p.dest_w, p.dest_h, p.ch) else { continue };
            let Some(li) = pixel_index(lx, ly, tw, th, p.ch) else { continue };
            if let (Some(dst), Some(src)) = (p.dest.get_mut(di..di + p.ch), local.get(li..li + p.ch)) {
                dst.copy_from_slice(src);
            }
        }
    }
    Ok(written)
}

fn dab_intersects(dab: &Dab, tile: (i32, i32, i32, i32)) -> bool {
    let Some((x0, y0, x1, y1)) = dab_bounds(dab) else { return false };
    x0 < tile.2 && x1 > tile.0 && y0 < tile.3 && y1 > tile.1
}

/// Mean `|src − dest|` over the dab's covered pixels (working-space peak is 1). `None` if nothing is covered.
fn dab_mean_diff(p: &StampParams<'_>, dab: &Dab) -> Option<f32> {
    let (x0, y0, x1, y1) = dab_bounds(dab)?;
    let mut sampled = vec![0.0f32; p.ch];
    let mut sum = 0.0f32;
    let mut count = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            if coverage_at(dab, x, y, p.hardness) <= 0.0 {
                continue;
            }
            let Some(di) = origin_index(p.dest_origin, x, y, p.dest_w, p.dest_h, p.ch) else { continue };
            let Some(dst) = p.dest.get(di..di.saturating_add(p.ch)) else { continue };
            let (sx, sy) = rotated_source(dab, x, y);
            if !sample_bilinear(SrcView { data: p.src, w: p.src_w, h: p.src_h, origin: p.src_origin, ch: p.ch }, sx, sy, &mut sampled) {
                continue;
            }
            let nch = p.ch.min(dst.len()).min(sampled.len());
            if nch == 0 {
                continue;
            }
            let mut mad = 0.0f32;
            for i in 0..nch {
                let Some((&s, &d)) = sampled.get(i).zip(dst.get(i)) else { continue };
                mad += (s - d).abs();
            }
            sum += mad / nch as f32;
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return None;
    }
    let mean = sum / count as f32;
    mean.is_finite().then_some(mean)
}

/// Stamp posed dabs into `dest`, reading `src` with a halo around each destination tile.
///
/// Sequential dabs overlap: dab *n* sees dab *n − 1*. Returns how many destination pixels were
/// written (tolerance-skipped dabs count as zero).
pub fn stamp(p: &mut StampParams<'_>) -> Result<u32, StampError> {
    dest_pixels(p)?;
    if p.src_w == 0 || p.src_h == 0 {
        return Err(StampError::Empty);
    }
    if p.dabs.is_empty() {
        return Err(StampError::Empty);
    }
    let dest_area = (p.dest_w as u64).saturating_mul(p.dest_h as u64);
    if dest_area > MAX_COVERAGE {
        return Err(StampError::TooLarge);
    }
    let tiles = tile_grid(p.dest_w, p.dest_h, p.dest_origin, p.tile);
    let halo = p.halo.max(0);
    let mut written = 0u32;
    let dabs: Vec<Dab> = p.dabs.to_vec();
    for dab in &dabs {
        let Some(bounds) = dab_bounds(dab) else { continue };
        if p.tolerance > 0.0 && dab_mean_diff(p, dab).is_some_and(|m| m < p.tolerance) {
            continue;
        }
        let overlapping: Vec<(i32, i32, i32, i32)> = tiles.iter().copied().filter(|t| dab_intersects(dab, *t)).collect();
        if overlapping.is_empty() {
            continue;
        }
        // Sequential tiles: each writes dest before the next dab. Parallel tiles of one dab do not
        // share destination pixels, so they can run independently; we still apply them in order
        // here so overlapping dabs stay deterministic.
        for tile in overlapping {
            let src_rect = (tile.0.saturating_sub(halo), tile.1.saturating_sub(halo), tile.2.saturating_add(halo), tile.3.saturating_add(halo));
            let (bx0, by0, bx1, by1) = bounds;
            let dest_rect = (tile.0.max(bx0), tile.1.max(by0), tile.2.min(bx1), tile.3.min(by1));
            written = written.saturating_add(stamp_dab_on_rect(p, dab, dest_rect, src_rect)?);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(w: usize, h: usize, ch: usize, f: impl Fn(i32, i32, usize) -> f32) -> Vec<f32> {
        let mut v = vec![0.0; w * h * ch];
        for y in 0..h {
            for x in 0..w {
                for c in 0..ch {
                    v[(y * w + x) * ch + c] = f(x as i32, y as i32, c);
                }
            }
        }
        v
    }

    fn horizontal(len: f32) -> Vec<[f32; 2]> {
        vec![[10.0, 32.0], [10.0 + len, 32.0]]
    }

    #[test]
    fn tight_short_emits_more_dabs_than_tight_long() {
        let pts = horizontal(100.0);
        let short = place_dabs(&pts, 10.0, 50.0, Style::TightShort, RNG_SEED).unwrap();
        let long = place_dabs(&pts, 10.0, 50.0, Style::TightLong, RNG_SEED).unwrap();
        assert!(short.len() > long.len(), "short {} long {}", short.len(), long.len());
        let s = spacing_px(10.0, 50.0, Style::TightShort).unwrap();
        let l = spacing_px(10.0, 50.0, Style::TightLong).unwrap();
        assert!(s < l, "{s} {l}");
    }

    #[test]
    fn tight_curl_angles_increase_and_loose_curl_scatters() {
        let pts = horizontal(80.0);
        let tight = place_dabs(&pts, 12.0, 50.0, Style::TightCurl, RNG_SEED).unwrap();
        let loose = place_dabs(&pts, 12.0, 50.0, Style::LooseCurl, RNG_SEED).unwrap();
        assert!(tight.len() >= 3 && loose.len() >= 3);
        let dt: Vec<f32> = tight.windows(2).map(|w| w[1].theta - w[0].theta).collect();
        assert!(dt.iter().all(|d| *d > 0.05), "tight curl steps {dt:?}");
        let dl: Vec<f32> = loose.windows(2).map(|w| w[1].theta - w[0].theta).collect();
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean(&dl) > mean(&dt), "loose curl {dl:?} vs tight {dt:?}");
        let scatter_t: f32 = tight.iter().map(|d| (d.y - 32.0).abs()).sum();
        let scatter_l: f32 = loose.iter().map(|d| (d.y - 32.0).abs()).sum();
        assert!(scatter_l > scatter_t, "loose scatter {scatter_l} tight {scatter_t}");
    }

    #[test]
    fn seeded_scatter_is_replayable() {
        let pts = horizontal(60.0);
        let a = place_dabs(&pts, 16.0, 40.0, Style::LooseMedium, RNG_SEED ^ 3).unwrap();
        let b = place_dabs(&pts, 16.0, 40.0, Style::LooseMedium, RNG_SEED ^ 3).unwrap();
        assert_eq!(a, b);
        let c = place_dabs(&pts, 16.0, 40.0, Style::LooseMedium, RNG_SEED ^ 4).unwrap();
        assert_ne!(a, c);
    }

    fn fixture(ch: usize) -> (usize, usize, Vec<f32>, Vec<f32>, Dab) {
        let (w, h) = (128usize, 96usize);
        let src = fill(w, h, ch, |x, y, c| if c + 1 == ch && ch > 1 { 1.0 } else { ((x * 13 + y * 7 + c as i32 * 3).rem_euclid(251) as f32) / 255.0 });
        let dest0 = fill(w, h, ch, |_, _, c| if c + 1 == ch && ch > 1 { 1.0 } else { 0.1 });
        // Centre 1 px inside a 64-tile so the rotated read crosses the seam.
        let dab = Dab { x: 63.0, y: 40.0, theta: 0.4, radius: 18.0, pressure: 1.0 };
        (w, h, src, dest0, dab)
    }

    #[allow(clippy::too_many_arguments)]
    fn stamp_run(w: usize, h: usize, ch: usize, src: &[f32], dest: &mut [f32], dab: &Dab, tile: i32, halo: i32) -> u32 {
        let mut p = StampParams {
            dest,
            dest_w: w,
            dest_h: h,
            dest_origin: (0, 0),
            src,
            src_w: w,
            src_h: h,
            src_origin: (0, 0),
            ch,
            dabs: std::slice::from_ref(dab),
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            tolerance: 0.0,
            tile,
            halo,
            selection: None,
            lock_transparent: false,
            alpha: (ch > 1).then_some(ch - 1),
        };
        stamp(&mut p).unwrap()
    }

    fn max_abs(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
    }

    fn max_u8(a: &[f32], b: &[f32]) -> i32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| {
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
                (q(*x) - q(*y)).abs()
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn tiled_stamp_matches_untiled_at_a_tile_seam() {
        let halo = halo_radius(40.0, Style::LooseMedium).unwrap();
        let (w, h, src, dest0, dab) = fixture(4);
        let mut tiled = dest0.clone();
        let mut flat = dest0.clone();
        let n_t = stamp_run(w, h, 4, &src, &mut tiled, &dab, 64, halo);
        let n_f = stamp_run(w, h, 4, &src, &mut flat, &dab, 0, halo);
        assert!(n_t > 0 && n_f > 0, "wrote {n_t} tiled / {n_f} flat");
        assert_eq!(max_u8(&tiled, &flat), 0, "8-bit seam");
        assert!(max_abs(&tiled, &flat) <= 1.0 / 32_767.0, "16-bit seam {}", max_abs(&tiled, &flat));
        assert!(max_abs(&tiled, &flat) <= 1.0e-5, "32f seam {}", max_abs(&tiled, &flat));

        let (w, h, src, dest0, dab) = fixture(1);
        let mut tiled = dest0.clone();
        let mut flat = dest0;
        stamp_run(w, h, 1, &src, &mut tiled, &dab, 64, halo);
        stamp_run(w, h, 1, &src, &mut flat, &dab, 0, halo);
        assert_eq!(max_u8(&tiled, &flat), 0, "gray 8-bit seam");
    }

    #[test]
    fn short_halo_disagrees_with_untiled_reference() {
        let (w, h, src, dest0, dab) = fixture(4);
        let halo = halo_radius(40.0, Style::LooseMedium).unwrap();
        let mut short = dest0.clone();
        let mut full = dest0;
        stamp_run(w, h, 4, &src, &mut short, &dab, 64, 1);
        stamp_run(w, h, 4, &src, &mut full, &dab, 0, halo);
        assert!(max_abs(&short, &full) > 1.0e-3, "a 1 px halo must miss the rotated read");
    }

    #[test]
    fn halo_covers_rotated_read_reach() {
        let size = 40.0;
        for style in
            [Style::TightShort, Style::TightMedium, Style::TightLong, Style::LooseMedium, Style::LooseLong, Style::Dab, Style::TightCurl, Style::LooseCurl]
        {
            let halo = halo_radius(size, style).unwrap();
            let side = sample_side(size, style).unwrap();
            let reach = (side + 1) / 2;
            assert!(halo >= reach, "{:?}: halo {halo} < reach {reach} (side {side})", style.as_str());
        }
    }

    #[test]
    fn tolerance_skips_an_unchanged_layer() {
        let (w, h, ch) = (48usize, 48usize, 4);
        let src = fill(w, h, ch, |x, y, c| if c == 3 { 1.0 } else { (x + y) as f32 / 100.0 });
        let mut dest = src.clone();
        let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
        let mut p = StampParams {
            dest: &mut dest,
            dest_w: w,
            dest_h: h,
            dest_origin: (0, 0),
            src: &src,
            src_w: w,
            src_h: h,
            src_origin: (0, 0),
            ch,
            dabs: std::slice::from_ref(&dab),
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            tolerance: 1.0,
            tile: 0,
            halo: 4,
            selection: None,
            lock_transparent: false,
            alpha: Some(3),
        };
        let n = stamp(&mut p).unwrap();
        assert_eq!(n, 0);
        assert_eq!(dest, src);
    }

    #[test]
    fn tolerance_zero_writes_where_history_differs() {
        let (w, h, ch) = (48usize, 48usize, 4);
        let src = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.9 });
        let mut dest = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.1 });
        let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
        let mut p = StampParams {
            dest: &mut dest,
            dest_w: w,
            dest_h: h,
            dest_origin: (0, 0),
            src: &src,
            src_w: w,
            src_h: h,
            src_origin: (0, 0),
            ch,
            dabs: std::slice::from_ref(&dab),
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            tolerance: 0.0,
            tile: 0,
            halo: 4,
            selection: None,
            lock_transparent: false,
            alpha: Some(3),
        };
        let n = stamp(&mut p).unwrap();
        assert!(n > 0);
        let i = (24 * w + 24) * ch;
        assert!((dest[i] - 0.9).abs() < 1.0e-5, "{}", dest[i]);
    }

    #[test]
    fn empty_or_mismatched_buffers_error() {
        let mut dest = vec![0.0f32; 16];
        let src = vec![0.0f32; 16];
        let dab = Dab { x: 1.0, y: 1.0, theta: 0.0, radius: 1.0, pressure: 1.0 };
        let mut p = StampParams {
            dest: &mut dest,
            dest_w: 0,
            dest_h: 4,
            dest_origin: (0, 0),
            src: &src,
            src_w: 4,
            src_h: 4,
            src_origin: (0, 0),
            ch: 1,
            dabs: std::slice::from_ref(&dab),
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            tolerance: 0.0,
            tile: 0,
            halo: 1,
            selection: None,
            lock_transparent: false,
            alpha: None,
        };
        assert_eq!(stamp(&mut p), Err(StampError::Empty));
        let mut dest = vec![0.0f32; 16];
        let src = vec![0.0f32; 8];
        let mut p = StampParams {
            dest: &mut dest,
            dest_w: 4,
            dest_h: 4,
            dest_origin: (0, 0),
            src: &src,
            src_w: 2,
            src_h: 2,
            src_origin: (0, 0),
            ch: 4,
            dabs: std::slice::from_ref(&dab),
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            tolerance: 0.0,
            tile: 0,
            halo: 1,
            selection: None,
            lock_transparent: false,
            alpha: None,
        };
        assert!(stamp(&mut p).is_err());
        assert!(place_dabs(&[], 10.0, 50.0, Style::TightMedium, 0).is_err());
        assert!(Style::parse("nope").is_none());
        assert_eq!(Style::parse("tightMedium"), Some(Style::TightMedium));
    }

    #[test]
    fn two_stamps_with_the_same_seed_match() {
        let pts = horizontal(50.0);
        let dabs = place_dabs(&pts, 12.0, 50.0, Style::LooseCurl, RNG_SEED).unwrap();
        let (w, h, ch) = (80usize, 64usize, 4);
        let src = fill(w, h, ch, |x, y, c| if c == 3 { 1.0 } else { (x + 3 * y) as f32 / 200.0 });
        let dest0 = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.2 });
        let run = |dest: &mut [f32]| {
            let mut p = StampParams {
                dest,
                dest_w: w,
                dest_h: h,
                dest_origin: (0, 0),
                src: &src,
                src_w: w,
                src_h: h,
                src_origin: (0, 0),
                ch,
                dabs: &dabs,
                hardness: 0.8,
                opacity: 1.0,
                flow: 1.0,
                tolerance: 0.0,
                tile: 64,
                halo: halo_radius(12.0, Style::LooseCurl).unwrap(),
                selection: None,
                lock_transparent: false,
                alpha: Some(3),
            };
            stamp(&mut p).unwrap()
        };
        let mut a = dest0.clone();
        let mut b = dest0;
        assert_eq!(run(&mut a), run(&mut b));
        assert_eq!(a, b);
    }
}
