//! A small process-wide cache for procedural brush textures.
//!
//! A stroke (and every symmetry mirror of it) used to regenerate its procedural texture in
//! [`crate::BrushContext::new`], although the image is a pure function of the style, size, seed and
//! level adjustments. The cache is bounded in entries and bytes and evicts the least recently used
//! entry first. Sampled-tip mip pyramids are not handled here (see PR #9617).
//!
//! The key holds every input of the generator plus the level adjustments (brightness, contrast,
//! invert) that are baked into the cached image, so a hit is always the image a rebuild would give.

use std::sync::{Arc, Mutex, PoisonError};

use crate::brush::{PatternStyle, Texture};
use crate::tile::PatternImage;

const PATTERN_MAX_ENTRIES: usize = 4;
const PATTERN_MAX_BYTES: usize = 16 << 20;

#[derive(PartialEq)]
struct PatternKey {
    style: PatternStyle,
    size: u32,
    seed: u32,
    brightness: u32,
    contrast: u32,
    invert: bool,
}

struct PatternEntry {
    key: PatternKey,
    image: Arc<PatternImage>,
}

static PATTERNS: Mutex<Vec<PatternEntry>> = Mutex::new(Vec::new());

/// The procedural pattern `(style, size, seed)` with `t`'s levels applied, from the cache when it
/// was built recently.
pub fn procedural_pattern(style: PatternStyle, size: u32, seed: u32, t: &Texture) -> Arc<PatternImage> {
    let key = PatternKey { style, size: size.clamp(8, 1024), seed, brightness: t.brightness.to_bits(), contrast: t.contrast.to_bits(), invert: t.invert };
    {
        let mut cache = PATTERNS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = cache.iter().position(|e| e.key == key) {
            let e = cache.remove(i);
            let image = Arc::clone(&e.image);
            cache.push(e);
            return image;
        }
    }
    let mut img = crate::procedural::pattern(style, size, seed);
    for v in &mut img.data {
        *v = t.level(*v);
    }
    let image = Arc::new(img);
    let bytes = |e: &PatternEntry| e.image.data.len() * 4;
    if image.data.len() * 4 <= PATTERN_MAX_BYTES / 2 {
        let mut cache = PATTERNS.lock().unwrap_or_else(PoisonError::into_inner);
        if !cache.iter().any(|e| e.key == key) {
            cache.push(PatternEntry { key, image: Arc::clone(&image) });
        }
        while cache.len() > PATTERN_MAX_ENTRIES || cache.iter().map(bytes).sum::<usize>() > PATTERN_MAX_BYTES {
            cache.remove(0);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_cache_keys_on_every_input() {
        let base = Texture::default();
        let a = procedural_pattern(PatternStyle::Noise, 64, 3, &base);
        assert!(Arc::ptr_eq(&a, &procedural_pattern(PatternStyle::Noise, 64, 3, &base)));
        let mut fresh = crate::procedural::pattern(PatternStyle::Noise, 64, 3);
        fresh.data.iter_mut().for_each(|v| *v = base.level(*v));
        assert_eq!(a.data, fresh.data);
        for other in [
            procedural_pattern(PatternStyle::Canvas, 64, 3, &base),
            procedural_pattern(PatternStyle::Noise, 32, 3, &base),
            procedural_pattern(PatternStyle::Noise, 64, 4, &base),
            procedural_pattern(PatternStyle::Noise, 64, 3, &Texture { invert: true, ..base.clone() }),
            procedural_pattern(PatternStyle::Noise, 64, 3, &Texture { brightness: 0.5, ..base.clone() }),
            procedural_pattern(PatternStyle::Noise, 64, 3, &Texture { contrast: 0.5, ..base.clone() }),
        ] {
            assert!(!Arc::ptr_eq(&a, &other));
        }
    }
}
