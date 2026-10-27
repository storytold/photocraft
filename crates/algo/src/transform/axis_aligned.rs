//! Direct separable sampling for axis-aligned reductions. An intermediate, origin-aligned
//! resize followed by a warp samples detail twice and makes it depend on the source's origin.

use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{AllocationError, Interrupt, Surface};

use super::{Homography, Interp, bicubic, warp_output_bounds};

#[derive(Clone, Copy)]
pub(super) struct Mapping {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}

impl Mapping {
    pub(super) fn reduction(h: &Homography, interp: Interp) -> Option<Self> {
        let m = h.0;
        if !matches!(interp, Interp::Bicubic | Interp::BicubicCoverage)
            || !m.iter().all(|v| v.is_finite())
            || m[1] != 0.0
            || m[3] != 0.0
            || m[6] != 0.0
            || m[7] != 0.0
            || m[8].abs() < 1e-12
        {
            return None;
        }
        let r = Self { sx: m[0] / m[8], sy: m[4] / m[8], tx: m[2] / m[8], ty: m[5] / m[8] };
        // Keep the existing bounded prefilter for extreme reductions. At this limit a direct
        // filter has at most 257 taps per axis, independent of the source's dimensions.
        let (x, y) = (r.sx.abs(), r.sy.abs());
        (x.is_finite() && y.is_finite() && x.min(y) >= 1.0 / 64.0 && x.min(y) < 0.999).then_some(r)
    }
}

struct Tap {
    samples: Vec<(i32, f64)>,
    coverage: f64,
}

fn taps(o: i32, scale: f64, shift: f64, first: i32, end: i32) -> Tap {
    let center = (f64::from(o) + 0.5 - shift) / scale;
    // Intersect both rectangle edges with the destination pixel. A nearest-edge ramp
    // overestimates coverage when the entire reduced rectangle is narrower than a pixel.
    let left = (f64::from(first) - center) * scale.abs();
    let right = (f64::from(end) - center) * scale.abs();
    let coverage = (right.min(0.5) - left.max(-0.5)).clamp(0.0, 1.0);
    if !center.is_finite() || coverage <= 0.0 {
        return Tap { samples: Vec::new(), coverage: 0.0 };
    }
    let u = center - 0.5;
    let stretch = (1.0 / scale.abs()).max(1.0);
    let mut samples = Vec::new();
    let clamp = |i: i64| i.clamp(i64::from(first), i64::from(end) - 1) as i32;
    if stretch == 1.0 {
        let i = u.floor() as i64;
        for (j, w) in bicubic(u - u.floor()).into_iter().enumerate() {
            samples.push((clamp(i.saturating_add(j as i64 - 1)), w));
        }
    } else {
        let lo = (u - 2.0 * stretch).ceil() as i64;
        let count = ((4.0 * stretch).ceil() as usize + 1).min(257);
        let mut sum = 0.0;
        for j in 0..count {
            let i = lo.saturating_add(j as i64);
            let x = ((u - i as f64) / stretch).abs();
            // The same Keys cubic (a = -0.75) as bicubic(), widened for this axis alone.
            let w = if x < 1.0 {
                1.25 * x * x * x - 2.25 * x * x + 1.0
            } else if x < 2.0 {
                -0.75 * (x * x * x - 5.0 * x * x + 8.0 * x - 4.0)
            } else {
                0.0
            };
            samples.push((clamp(i), w));
            sum += w;
        }
        if sum > 0.0 {
            for (_, w) in &mut samples {
                *w /= sum;
            }
        }
    }
    Tap { samples, coverage }
}

fn zeroed<T: Default + Clone>(len: usize) -> Result<Vec<T>, AllocationError> {
    let bytes = len.checked_mul(std::mem::size_of::<T>()).ok_or(AllocationError::SizeOverflow)?;
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| AllocationError::NotEnoughMemory { bytes })?;
    v.resize(len, T::default());
    Ok(v)
}

fn tile(src: &Surface, rect: Rect, dst: Rect, m: Mapping, interp: Interp, ctl: &Interrupt) -> Result<Option<Vec<f32>>, AllocationError> {
    let fmt = src.format();
    let n = fmt.channels();
    let nx = dst.width() as usize;
    let len = nx.checked_mul(dst.height() as usize).and_then(|v| v.checked_mul(n)).ok_or(AllocationError::SizeOverflow)?;
    let xs: Vec<_> = (dst.x0..dst.x1).map(|x| taps(x, m.sx, m.tx, rect.x0, rect.x1)).collect();
    let ys: Vec<_> = (dst.y0..dst.y1).map(|y| taps(y, m.sy, m.ty, rect.y0, rect.y1)).collect();
    let extent = |t: &[Tap]| {
        let mut it = t.iter().flat_map(|t| t.samples.iter().filter(|(_, w)| *w != 0.0).map(|(i, _)| *i));
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), i| (lo.min(i), hi.max(i))))
    };
    let (Some((x0, x1)), Some((y0, y1))) = (extent(&xs), extent(&ys)) else { return Ok(None) };
    let footprint = Rect::new(x0, y0, x1.saturating_add(1), y1.saturating_add(1));
    if src.default_pixel().iter().all(|v| *v == 0.0) && !src.has_tiles_in(footprint) {
        return Ok(None);
    }
    let mut result = zeroed(len)?;
    let read_width = (i64::from(x1) - i64::from(x0) + 1) as usize;
    let read_channels = read_width.checked_mul(n).ok_or(AllocationError::SizeOverflow)?;
    // Each source row is filtered horizontally once and streamed into its destination rows.
    // The read is bounded to 16 MiB; the accumulators hold only this destination tile.
    let rows_per_read = ((1 << 22) / read_channels.max(1)).clamp(1, TILE_SIZE as usize) as i32;
    let mut users: Vec<Vec<(usize, f64)>> = zeroed((i64::from(y1) - i64::from(y0) + 1) as usize)?;
    for (dy, t) in ys.iter().enumerate() {
        for &(sy, weight) in &t.samples {
            if weight != 0.0
                && let Some(row) = users.get_mut((i64::from(sy) - i64::from(y0)) as usize)
            {
                row.push((dy, weight));
            }
        }
    }
    let mut acc: Vec<f64> = zeroed(len)?;
    let mut supported: Vec<bool> = zeroed(len / n)?;
    let mut invalid_alpha: Vec<bool> = zeroed(len / n)?;
    let mut horizontal: Vec<f64> = zeroed(nx * n)?;
    let mut positive: Vec<bool> = zeroed(nx)?;
    let mut horizontal_invalid: Vec<bool> = zeroed(nx)?;
    let mut y = y0;
    while y <= y1 {
        if ctl.cancelled() {
            return Ok(None);
        }
        let end = y.saturating_add(rows_per_read).min(y1.saturating_add(1));
        let px = src.try_read_region(Rect::new(x0, y, x1.saturating_add(1), end))?;
        for (sy, row) in (y..end).zip(px.chunks_exact(read_channels)) {
            horizontal.fill(0.0);
            positive.fill(false);
            horizontal_invalid.fill(false);
            for (((tap, out), support), invalid) in xs.iter().zip(horizontal.chunks_exact_mut(n)).zip(&mut positive).zip(&mut horizontal_invalid) {
                for &(sx, weight) in &tap.samples {
                    if weight == 0.0 {
                        continue;
                    }
                    let at = (i64::from(sx) - i64::from(x0)) as usize * n;
                    if let Some(p) = row.get(at..at + n) {
                        let alpha = f64::from(p.last().copied().unwrap_or(0.0));
                        *support |= weight > 0.0 && alpha > 0.0;
                        *invalid |= !alpha.is_finite() || alpha < 0.0;
                        for (c, (v, sample)) in out.iter_mut().zip(p).enumerate() {
                            *v += f64::from(*sample) * weight * if c + 1 == n { 1.0 } else { alpha };
                        }
                    }
                }
            }
            for &(dy, weight) in users.get((i64::from(sy) - i64::from(y0)) as usize).into_iter().flatten() {
                if let Some(out) = acc.get_mut(dy * nx * n..(dy + 1) * nx * n) {
                    for (v, h) in out.iter_mut().zip(&horizontal) {
                        *v += weight * h;
                    }
                }
                // The conservative signed-alpha fallback belongs to this output's kernel,
                // not the tile's read window: unrelated samples must not change its coverage.
                if let Some(out) = invalid_alpha.get_mut(dy * nx..(dy + 1) * nx) {
                    for (v, h) in out.iter_mut().zip(&horizontal_invalid) {
                        *v |= h;
                    }
                }
                if weight > 0.0
                    && let Some(out) = supported.get_mut(dy * nx..(dy + 1) * nx)
                {
                    for (v, h) in out.iter_mut().zip(&positive) {
                        *v |= h;
                    }
                }
            }
        }
        if end <= y {
            break;
        }
        y = end;
    }
    let mut any = false;
    for (dy, (((row, dst), support), invalid)) in
        acc.chunks_exact(nx * n).zip(result.chunks_exact_mut(nx * n)).zip(supported.chunks_exact(nx)).zip(invalid_alpha.chunks_exact(nx)).enumerate()
    {
        for ((((p, out), support), invalid), x) in row.chunks_exact(n).zip(dst.chunks_exact_mut(n)).zip(support).zip(invalid).zip(&xs) {
            let alpha = p.last().copied().unwrap_or(0.0);
            let coverage = x.coverage * ys.get(dy).map_or(0.0, |y| y.coverage);
            let a = (alpha * coverage).clamp(0.0, 1.0);
            if a <= 0.0 || (interp == Interp::BicubicCoverage && !invalid && !support) {
                continue;
            }
            any = true;
            for (c, (v, sample)) in out.iter_mut().zip(p).enumerate() {
                // Divide by the original alpha, before clipping its bicubic overshoot.
                *v = if c + 1 == n { a as f32 } else { (sample / alpha) as f32 };
            }
        }
    }
    Ok(any.then_some(result))
}

pub(super) fn warp(src: &Surface, rect: Rect, m: Mapping, interp: Interp, clip: Option<Rect>, ctl: &Interrupt) -> Result<Surface, AllocationError> {
    if src.tiles().next().is_none() && src.default_pixel().iter().all(|v| *v == 0.0) {
        return Ok(Surface::new(src.format()));
    }
    // Homogeneous rescaling must not affect inverse validity or output bounds. Dispatch
    // has already normalized this diagonal mapping, so use that representation here too.
    let h = Homography([m.sx, 0.0, m.tx, 0.0, m.sy, m.ty, 0.0, 0.0, 1.0]);
    let bounds = warp_output_bounds(rect, &h).unwrap_or(Rect::EMPTY);
    let dst = clip.map_or(bounds, |c| bounds.intersect(&c));
    let count = dst.tiles().count();
    let bytes = count.checked_mul(std::mem::size_of::<Rect>()).ok_or(AllocationError::SizeOverflow)?;
    let mut tiles = Vec::new();
    tiles.try_reserve_exact(count).map_err(|_| AllocationError::NotEnoughMemory { bytes })?;
    tiles.extend(dst.tiles().map(|tc| tc.rect().intersect(&dst)));
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let out = std::sync::Mutex::new(Surface::new(src.format()));
    let work = |r: Rect| -> Result<(), AllocationError> {
        if ctl.cancelled() {
            return Ok(());
        }
        let pixels = tile(src, rect, r, m, interp, ctl)?;
        if !ctl.cancelled() {
            if let Some(pixels) = pixels {
                out.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_write_region(r, &pixels)?;
            }
            let done = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            ctl.progress(0.98 * done as f32 / total as f32);
        }
        Ok(())
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        tiles.into_par_iter().try_for_each(work)?;
    }
    #[cfg(target_arch = "wasm32")]
    for r in tiles {
        work(r)?;
    }
    let mut out = out.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    if ctl.cancelled() {
        return Ok(Surface::new(src.format()));
    }
    out.prune();
    Ok(out)
}
