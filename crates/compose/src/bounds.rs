//! `Surface::content_bounds` with the per-tile scan cached by copy-on-write tile identity, so
//! layer bounds cost a hash lookup per tile instead of a pixel scan per render tile (the CPU
//! compositor asks for an effect layer's bounds in every tile; the GPU planner every frame).
//! [`half_opaque_bounds`] caches the bounds of the pixels at least half opaque the same way.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use photocraft_geom::Rect;
use photocraft_raster::{Surface, Tile};

struct Entry {
    tile: Weak<Tile>,
    /// Content bounds inside the tile (tile-local), for one default pixel (for
    /// [`Kind::HalfOpaque`], the pixel size alone).
    default: Box<[u8]>,
    bounds: Option<(u16, u16, u16, u16)>,
}

/// What a cached tile scan measured.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// Pixels that differ from the default pixel.
    Content,
    /// 8-bit pixels with alpha of at least half (128 of 255).
    HalfOpaque,
}

/// The entries, and the size past which dead ones are dropped (see [`next_sweep`]).
struct Cache {
    entries: HashMap<(usize, Kind), Entry>,
    sweep_at: usize,
}

const MIN_SWEEP: usize = 1 << 16;

/// The size at which to sweep again after a sweep left `survivors` live entries: twice that (at
/// least [`MIN_SWEEP`]), so a cache full of live tiles isn't swept again on every call.
fn next_sweep(survivors: usize) -> usize {
    survivors.saturating_mul(2).max(MIN_SWEEP)
}

fn cache() -> &'static Mutex<Cache> {
    static C: std::sync::OnceLock<Mutex<Cache>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(Cache { entries: HashMap::new(), sweep_at: MIN_SWEEP }))
}

pub(crate) fn purge() {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.entries.clear();
    cache.sweep_at = MIN_SWEEP;
}

/// `Surface::content_bounds`, cached per tile.
pub fn content_bounds(s: &Surface) -> Rect {
    if s.tile_count() == 0 {
        return Rect::EMPTY;
    }
    let fmt = s.format();
    let mut dp = vec![0u8; fmt.bytes_per_pixel()];
    photocraft_raster::encode_pixel(&fmt, &s.default_pixel(), &mut dp);
    cached_bounds(s, Kind::Content, &dp, |t| t.content_bounds(&dp))
}

/// The bounds of an 8-bit surface's pixels with alpha of at least half (128 of 255), cached per
/// tile like [`content_bounds`]: the box a Gradient Overlay aligned with the layer spans in the
/// reference application (#14910). `None` where that rule is not measured or cannot hold: other
/// bit depths, a surface without alpha, or a default pixel that is itself half opaque (the box
/// would be unbounded). `Some(Rect::EMPTY)` when no pixel is half opaque.
pub fn half_opaque_bounds(s: &Surface) -> Option<Rect> {
    let fmt = s.format();
    if fmt.sample != photocraft_color::SampleType::U8 || !fmt.alpha || s.default_pixel().last().is_none_or(|a| *a >= HALF_OPAQUE_F32) {
        return None;
    }
    if s.tile_count() == 0 {
        return Some(Rect::EMPTY);
    }
    let bpp = fmt.bytes_per_pixel();
    let size = vec![0u8; bpp];
    Some(cached_bounds(s, Kind::HalfOpaque, &size, |t| half_opaque_tile_bounds(&t.bytes(), bpp)))
}

/// 8-bit alpha at or above this is half opaque: a 50 % fill (127.5, stored as 128) is inside the
/// box, 49 % (125) is not.
const HALF_OPAQUE_U8: u8 = 128;
/// [`HALF_OPAQUE_U8`] as a normalised default-pixel alpha.
const HALF_OPAQUE_F32: f32 = 127.5 / 255.0;

/// Tile-local bounds of the pixels of an 8-bit tile (`bpp` bytes each, alpha last) whose alpha is
/// at least [`HALF_OPAQUE_U8`].
fn half_opaque_tile_bounds(bytes: &[u8], bpp: usize) -> Option<(u16, u16, u16, u16)> {
    if bpp == 0 {
        return None;
    }
    let ts = photocraft_geom::TILE_SIZE as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (y, row) in bytes.chunks_exact(ts * bpp).enumerate() {
        let opaque = |px: &[u8]| px.last().is_some_and(|a| *a >= HALF_OPAQUE_U8);
        let Some(first) = row.chunks_exact(bpp).position(opaque) else { continue };
        let last = row.chunks_exact(bpp).rposition(opaque).unwrap_or(first);
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    (x0 != usize::MAX).then_some((x0 as u16, y0 as u16, x1 as u16, y1 as u16))
}

/// The union over `s`'s tiles of `scan` (tile-local bounds), each looked up by tile identity,
/// `kind` and `default` before scanning.
fn cached_bounds(s: &Surface, kind: Kind, default: &[u8], scan: impl Fn(&Tile) -> Option<(u16, u16, u16, u16)>) -> Rect {
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.entries.len() > c.sweep_at {
        c.entries.retain(|_, e| e.tile.strong_count() > 0);
        c.sweep_at = next_sweep(c.entries.len());
    }
    let mut out = Rect::EMPTY;
    for (coord, t) in s.tiles() {
        let key = (Arc::as_ptr(t) as usize, kind);
        let hit = c.entries.get(&key).filter(|e| e.tile.upgrade().is_some_and(|u| Arc::ptr_eq(&u, t)) && *e.default == *default).map(|e| e.bounds);
        let b = match hit {
            Some(b) => b,
            None => {
                let generation = photocraft_raster::spill::read_error_generation();
                let b = scan(t);
                if photocraft_raster::spill::check_since(generation).is_ok() && t.scratch_health().is_ok() {
                    c.entries.insert(key, Entry { tile: Arc::downgrade(t), default: default.into(), bounds: b });
                }
                b
            }
        };
        if let Some((x0, y0, x1, y1)) = b {
            let o = coord.rect();
            let r = Rect::new(o.x0 + x0 as i32, o.y0 + y0 as i32, o.x0 + x1 as i32, o.y0 + y1 as i32);
            out = if out.is_empty() { r } else { out.union(&r) };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeps_again_only_past_twice_the_survivors() {
        // A cache of live tiles past the minimum isn't swept on every call (it used to be).
        assert_eq!(next_sweep(0), MIN_SWEEP);
        assert_eq!(next_sweep(MIN_SWEEP / 2), MIN_SWEEP);
        assert_eq!(next_sweep(MIN_SWEEP), 2 * MIN_SWEEP);
        assert_eq!(next_sweep(usize::MAX), usize::MAX);
    }

    #[test]
    fn a_shared_tile_reads_the_same_bounds_at_every_coordinate() {
        let mut s = Surface::new(photocraft_color::PixelFormat::RGBA8);
        s.fill_rect(Rect::new(-300, 10, 1300, 900), &[0.2, 0.4, 0.6, 1.0]);
        assert_eq!(content_bounds(&s), Rect::new(-300, 10, 1300, 900));
        assert_eq!(content_bounds(&s), s.content_bounds(), "cached");
    }

    #[test]
    fn half_opaque_bounds_start_at_alpha_128() {
        use photocraft_color::PixelFormat;
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(10, 10, 20, 20), &[1.0, 0.0, 0.0, 1.0]);
        s.fill_rect(Rect::new(300, 5, 310, 15), &[1.0, 0.0, 0.0, 127.0 / 255.0]);
        assert_eq!(half_opaque_bounds(&s), Some(Rect::new(10, 10, 20, 20)));
        assert_eq!(half_opaque_bounds(&s), Some(Rect::new(10, 10, 20, 20)), "cached");
        // Editing the faint pixels to 128 splits their tile off: the box follows.
        s.fill_rect(Rect::new(300, 5, 310, 15), &[1.0, 0.0, 0.0, 128.0 / 255.0]);
        assert_eq!(half_opaque_bounds(&s), Some(Rect::new(10, 5, 310, 20)));
        // Gray with alpha: the alpha is the second channel.
        let mut g = Surface::new(PixelFormat::GRAYA8);
        g.fill_rect(Rect::new(-40, 0, -30, 3), &[0.2, 0.6]);
        g.fill_rect(Rect::new(0, 0, 5, 5), &[0.9, 0.3]);
        assert_eq!(half_opaque_bounds(&g), Some(Rect::new(-40, 0, -30, 3)));
        assert_eq!(half_opaque_bounds(&Surface::new(PixelFormat::RGBA8)), Some(Rect::EMPTY));
    }

    #[test]
    fn half_opaque_bounds_are_unmeasured_past_8_bit_or_without_alpha() {
        use photocraft_color::PixelFormat;
        for fmt in [PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAY8] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 1.0, 1.0, 1.0]);
            assert_eq!(half_opaque_bounds(&s), None, "{fmt:?}");
        }
    }
}
