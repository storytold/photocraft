//! Shared texture caching and raster preview helpers for preset assets.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use std::hash::{Hash, Hasher};

/// Fingerprint every input that affects a preset preview before consulting the texture cache.
/// Callers must include identity separately when two assets with equal content are distinct.
pub(crate) fn content_fingerprint(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Fingerprint surface samples without allocating a second full-size pixel buffer.
pub(crate) fn sample_fingerprint(width: u32, height: u32, format: impl Hash, samples: &[f32]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    format.hash(&mut hasher);
    for sample in samples {
        sample.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Clone)]
struct CachedTexture {
    fingerprint: u64,
    texture: TextureHandle,
}

/// Cache under stable asset identity, replacing the existing texture when preview content changes.
/// Keeping the fingerprint as entry metadata avoids accumulating one egui temp entry per revision.
pub(crate) fn cached_texture(
    ctx: &egui::Context,
    key: impl std::hash::Hash + std::fmt::Debug,
    fingerprint: u64,
    make: impl FnOnce() -> ColorImage,
) -> TextureHandle {
    let id = egui::Id::new(("preset-tex", key));
    if let Some(entry) = ctx.data(|d| d.get_temp::<CachedTexture>(id))
        && entry.fingerprint == fingerprint
    {
        return entry.texture;
    }
    let tex = ctx.load_texture(format!("preset-{id:?}"), make(), TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, CachedTexture { fingerprint, texture: tex.clone() }));
    tex
}

pub(crate) fn c32(c: [f32; 4]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

#[cfg(test)]
mod tests {
    use super::{cached_texture, content_fingerprint, sample_fingerprint};
    use egui::{Color32, ColorImage};
    use std::cell::Cell;

    #[test]
    fn content_fingerprint_changes_when_preview_inputs_change() {
        assert_ne!(content_fingerprint(&("shape", "Star", "path-a")), content_fingerprint(&("shape", "Star", "path-b")));
        assert_eq!(content_fingerprint(&("shape", "Star", "path-a")), content_fingerprint(&("shape", "Star", "path-a")));
    }

    #[test]
    fn sample_fingerprint_invalidates_changed_pixels_and_dimensions() {
        let original = sample_fingerprint(2, 1, "rgba8", &[0.0, 0.5]);
        assert_ne!(original, sample_fingerprint(2, 1, "rgba8", &[0.0, 0.75]));
        assert_ne!(original, sample_fingerprint(1, 2, "rgba8", &[0.0, 0.5]));
        assert_ne!(original, sample_fingerprint(2, 1, "rgba16", &[0.0, 0.5]));
    }

    #[test]
    fn cached_texture_hits_matching_fingerprint_and_replaces_changed_content() {
        let ctx = egui::Context::default();
        let builds = Cell::new(0);
        let make = || {
            builds.set(builds.get() + 1);
            ColorImage::new([1, 1], vec![Color32::WHITE])
        };

        let first = cached_texture(&ctx, ("pattern", "stable-id"), 10, make);
        let hit = cached_texture(&ctx, ("pattern", "stable-id"), 10, || {
            builds.set(builds.get() + 1);
            ColorImage::new([1, 1], vec![Color32::BLACK])
        });
        assert_eq!(builds.get(), 1, "matching fingerprint should reuse cached texture");
        assert_eq!(first.id(), hit.id(), "cache hit should return the existing texture");

        let replacement = cached_texture(&ctx, ("pattern", "stable-id"), 11, make);
        assert_eq!(builds.get(), 2, "changed fingerprint should rebuild the texture");
        assert_ne!(first.id(), replacement.id(), "changed content should replace the cached texture");

        let replacement_hit = cached_texture(&ctx, ("pattern", "stable-id"), 11, make);
        assert_eq!(builds.get(), 2, "replacement should be cached under the stable identity");
        assert_eq!(replacement.id(), replacement_hit.id());
    }
}
