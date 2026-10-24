//! Patterns: tiles used by Pattern Overlay, pattern fill layers, pattern strokes, Edit › Fill
//! and brush textures. Pure data; rendering lives in `photocraft-compose`, the PSD mapping
//! (`Patt`/`Pat2`/`Pat3` global blocks) in `photocraft-io`.

use photocraft_geom::Rect;
use photocraft_raster::{AllocationError, Interrupt, Surface};

/// Largest pattern edge a file may declare (the PSD reader's limit; Define Pattern stops at 4000).
pub const MAX_EDGE: u32 = 30_000;

/// Most pixels a pattern may have to be painted. Renderers expand a pattern into a 16-byte-per-pixel
/// working copy, so this keeps that copy at 1 GiB; larger patterns paint nothing.
pub const MAX_PAINT_PIXELS: u64 = 1 << 26;

/// A bounded, cancellable pattern hash could not be completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternBuildError {
    InvalidDimensions,
    ScratchLimit { required: usize, available: usize },
    Allocation(AllocationError),
    Cancelled,
}

impl std::fmt::Display for PatternBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("pattern dimensions exceed native edge or paint limits"),
            Self::ScratchLimit { required, available } => write!(f, "pattern hash needs {required} scratch bytes; {available} available"),
            Self::Allocation(error) => error.fmt(f),
            Self::Cancelled => f.write_str("pattern creation cancelled"),
        }
    }
}

impl std::error::Error for PatternBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            _ => None,
        }
    }
}

/// One pattern tile. Pixels cover `0..width × 0..height` of `surface`, in any pixel format
/// (depth and colour model are the pattern's own, as in Photoshop).
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    /// Unique id (a UUID string, as Photoshop's `Idnt`).
    pub id: String,
    /// Name as stored (Photoshop's built-ins use `$$$/Patterns/…=Display Name`; see
    /// [`Pattern::display_name`]).
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub surface: Surface,
}

/// 64-bit FNV-1a, used for deterministic pattern ids.
fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

impl Pattern {
    /// Construct a pattern using at most one fallibly allocated normalized pixel
    /// row while hashing. The id matches [`Self::new`] for the same input.
    ///
    /// `max_scratch_bytes` is the caller's remaining allowance for the temporary
    /// hash row; the already-owned surface must be accounted for separately.
    /// Native edge/paint limits and scratch size are checked before allocation.
    /// Cancellation is checked before each row and before committing the result.
    pub fn try_new_in(
        name: impl Into<String>,
        surface: Surface,
        width: u32,
        height: u32,
        max_scratch_bytes: usize,
        ctl: &Interrupt<'_>,
    ) -> Result<Pattern, PatternBuildError> {
        ctl.check().map_err(|_| PatternBuildError::Cancelled)?;
        if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || u64::from(width) * u64::from(height) > MAX_PAINT_PIXELS {
            return Err(PatternBuildError::InvalidDimensions);
        }
        let row_bytes = usize::try_from(width)
            .ok()
            .and_then(|w| w.checked_mul(surface.channels()))
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or(PatternBuildError::Allocation(AllocationError::SizeOverflow))?;
        if row_bytes > max_scratch_bytes {
            return Err(PatternBuildError::ScratchLimit { required: row_bytes, available: max_scratch_bytes });
        }
        let name = name.into();
        let mut h1 = fnv(0xcbf2_9ce4_8422_2325, name.as_bytes());
        h1 = fnv(h1, &width.to_le_bytes());
        h1 = fnv(h1, &height.to_le_bytes());
        // Dimensions are already bounded by MAX_EDGE < i32::MAX.
        for y in 0..height {
            ctl.check().map_err(|_| PatternBuildError::Cancelled)?;
            let row = surface.try_read_region(Rect::new(0, y as i32, width as i32, (y + 1) as i32)).map_err(PatternBuildError::Allocation)?;
            for value in row {
                h1 = fnv(h1, &value.to_bits().to_le_bytes());
            }
        }
        ctl.check().map_err(|_| PatternBuildError::Cancelled)?;
        let h2 = fnv(h1 ^ 0x9e37_79b9_7f4a_7c15, format!("{:?}", surface.format()).as_bytes());
        let id = format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", (h1 >> 32) as u32, (h1 >> 16) as u16, h1 as u16, (h2 >> 48) as u16, h2 & 0xffff_ffff_ffff);
        Ok(Pattern { id, name, width, height, surface })
    }

    /// A pattern with an id derived from its name and pixels (same content → same id, so
    /// defining a pattern is deterministic and replayable).
    pub fn new(name: impl Into<String>, surface: Surface, width: u32, height: u32) -> Pattern {
        let name = name.into();
        let r = Rect::new(0, 0, width as i32, height as i32);
        let px = surface.read_region(r);
        let mut h1 = fnv(0xcbf2_9ce4_8422_2325, name.as_bytes());
        h1 = fnv(h1, &width.to_le_bytes());
        h1 = fnv(h1, &height.to_le_bytes());
        for v in &px {
            h1 = fnv(h1, &v.to_bits().to_le_bytes());
        }
        let h2 = fnv(h1 ^ 0x9e37_79b9_7f4a_7c15, format!("{:?}", surface.format()).as_bytes());
        let id = format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", (h1 >> 32) as u32, (h1 >> 16) as u16, h1 as u16, (h2 >> 48) as u16, h2 & 0xffff_ffff_ffff);
        Pattern { id, name, width, height, surface }
    }

    /// The name shown in UIs: Photoshop's localisation keys (`$$$/Patterns/…=Water`) show
    /// the text after `=`; trailing NULs are dropped.
    pub fn display_name(&self) -> &str {
        display_name(&self.name)
    }

    /// The tile's rectangle.
    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    /// Whether the tile has pixels.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Pixels in the tile (`width × height`, never overflowing).
    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Whether the tile has pixels and is small enough to paint (see [`MAX_PAINT_PIXELS`]).
    pub fn is_paintable(&self) -> bool {
        !self.is_empty() && self.pixel_count() <= MAX_PAINT_PIXELS
    }
}

/// Display form of a stored pattern name (see [`Pattern::display_name`]).
pub fn display_name(name: &str) -> &str {
    let n = name.trim_end_matches('\0');
    match n.strip_prefix("$$$/").and_then(|r| r.split_once('=')) {
        Some((_, shown)) => shown,
        None => n,
    }
}

/// Finds a pattern by id, else by stored name, else by display name (case-insensitive).
pub fn find<'a>(patterns: &'a [Pattern], id: &str, name: &str) -> Option<&'a Pattern> {
    if !id.is_empty()
        && let Some(p) = patterns.iter().find(|p| p.id == id)
    {
        return Some(p);
    }
    if name.is_empty() {
        return None;
    }
    patterns.iter().find(|p| p.name == name).or_else(|| patterns.iter().find(|p| p.display_name().eq_ignore_ascii_case(display_name(name))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn ids_are_deterministic_and_lookup_falls_back_to_names() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        let a = Pattern::new("$$$/Patterns/Defaults/Water=Water", s.clone(), 4, 4);
        let b = Pattern::new("$$$/Patterns/Defaults/Water=Water", s.clone(), 4, 4);
        assert_eq!(a.id, b.id);
        assert_eq!(a.id.len(), 36);
        assert_ne!(Pattern::new("Other", s, 4, 4).id, a.id);
        assert_eq!(a.display_name(), "Water");
        let list = vec![a.clone()];
        assert!(find(&list, &a.id, "").is_some());
        assert!(find(&list, "nope", "water").is_some());
        assert!(find(&list, "nope", "").is_none());
    }

    #[test]
    fn bounded_row_hash_matches_existing_ids_for_native_depths_models_and_alpha() {
        use photocraft_color::{ColorMode, SampleType};
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for alpha in [false, true] {
                    let format = PixelFormat::new(mode, depth, alpha);
                    let mut surface = Surface::new(format);
                    // Cross both a tile and a row boundary, including sparse pixels.
                    let rect = Rect::new(0, 0, 67, 3);
                    let values = (0..67 * 3 * format.channels()).map(|n| (n % 19) as f32 / 16.0 - 0.125).collect::<Vec<_>>();
                    surface.write_region(rect, &values);
                    let existing = Pattern::new("Original row hash", surface.clone(), 67, 4);
                    let streamed = Pattern::try_new_in("Original row hash", surface, 67, 4, 67 * format.channels() * 4, &Interrupt::NONE).unwrap();
                    assert_eq!(streamed, existing, "{mode:?}/{depth:?}/alpha={alpha}");
                }
            }
        }
    }

    #[test]
    fn bounded_hash_rejects_hostile_extents_and_scratch_before_pixel_reads() {
        for (width, height) in [(0, 1), (1, 0), (MAX_EDGE + 1, 1), (1, MAX_EDGE + 1), (MAX_EDGE, MAX_EDGE), (u32::MAX, 1)] {
            let result = Pattern::try_new_in("Too large", Surface::new(PixelFormat::RGBA8), width, height, usize::MAX, &Interrupt::NONE);
            assert_eq!(result, Err(PatternBuildError::InvalidDimensions));
        }
        assert_eq!(
            Pattern::try_new_in("No scratch", Surface::new(PixelFormat::RGBA8), 30000, 1, 0, &Interrupt::NONE),
            Err(PatternBuildError::ScratchLimit { required: 30000 * 4 * 4, available: 0 })
        );
    }

    #[test]
    fn bounded_hash_checks_cancellation_before_and_between_rows() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cancelled = || true;
        assert_eq!(
            Pattern::try_new_in("Cancelled", Surface::new(PixelFormat::RGBA8), 1, 1, 16, &Interrupt::cancel_only(&cancelled)),
            Err(PatternBuildError::Cancelled)
        );
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        let result = Pattern::try_new_in("Mid-row cancellation", Surface::new(PixelFormat::RGBA8), 1, 20, 16, &Interrupt::cancel_only(&cancel));
        assert_eq!(result, Err(PatternBuildError::Cancelled));
        assert_eq!(checks.load(Ordering::Relaxed), 4);
    }
}
