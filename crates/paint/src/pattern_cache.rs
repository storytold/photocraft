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
        // Another thread may have built the same key meanwhile: keep its image, but it was just asked for.
        match cache.iter().position(|e| e.key == key) {
            Some(i) => {
                let e = cache.remove(i);
                cache.push(e);
            }
            None => cache.push(PatternEntry { key, image: Arc::clone(&image) }),
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
        let variants = [
            (PatternStyle::Canvas, 64, 3, base.clone()),
            (PatternStyle::Noise, 32, 3, base.clone()),
            (PatternStyle::Noise, 64, 4, base.clone()),
            (PatternStyle::Noise, 64, 3, Texture { invert: true, ..base.clone() }),
            (PatternStyle::Noise, 64, 3, Texture { brightness: 0.5, ..base.clone() }),
            (PatternStyle::Noise, 64, 3, Texture { contrast: 0.5, ..base.clone() }),
        ];
        for (style, size, seed, texture) in variants {
            // Look the base up again so it is the newest entry and still cached when the variant is
            // asked for; a key that ignored the changed input would then hand back the base image.
            procedural_pattern(PatternStyle::Noise, 64, 3, &base);
            let other = procedural_pattern(style, size, seed, &texture);
            assert!(!Arc::ptr_eq(&a, &other) && other.data != a.data, "{style:?} {size} {seed}");
        }
    }

    /// The cache holds a few entries: the oldest one goes first. Seeds are used by no other test, so
    /// no hit can refresh `first`.
    #[test]
    fn pattern_cache_evicts_the_oldest_entry_past_its_limit() {
        let base = Texture::default();
        let first = procedural_pattern(PatternStyle::Noise, 16, 90_001, &base);
        assert!(Arc::ptr_eq(&first, &procedural_pattern(PatternStyle::Noise, 16, 90_001, &base)));
        for seed in 90_002..90_002 + PATTERN_MAX_ENTRIES as u32 {
            procedural_pattern(PatternStyle::Noise, 16, seed, &base);
        }
        assert!(!Arc::ptr_eq(&first, &procedural_pattern(PatternStyle::Noise, 16, 90_001, &base)));
    }
}
