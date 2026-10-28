//! Encoding-based choices validated by the release camera matrix. Never match
//! filenames, race two full decodes, or substitute a preview for sensor samples.
use crate::{
    RawFormat,
    tiff::{Tiff, tag},
};

pub(crate) fn prefer_builtin(bytes: &[u8], format: RawFormat) -> bool {
    // Performance choices were measured on native ARM64. Other targets keep
    // FlashRaw first until they have their own measurements; representation
    // compatibility (float, predictors, cRAW tone curves) is target independent.
    let tuned = cfg!(target_arch = "aarch64");
    if format == RawFormat::Dng
        && let Some(t) = Tiff::new(bytes)
        && let Some(raw) = crate::dng::raw_ifd(&t, &t.all_ifds())
    {
        let compression = t.tag_uint(&raw, tag::COMPRESSION).unwrap_or(1);
        let bits = t.tag_uint(&raw, tag::BITS_PER_SAMPLE).unwrap_or(16);
        // These representations require our stored-value/float policy. Native
        // JPEG XL and whole 16-bit strips also measured faster here; packed
        // 12/14-bit planes keep FlashRaw's SIMD unpacker.
        let tiled_jpeg = compression == 7
            && raw.has(tag::TILE_WIDTH)
            && u64::from(t.tag_uint(&raw, tag::TILE_WIDTH).unwrap_or(0))
                .saturating_mul(u64::from(t.tag_uint(&raw, tag::TILE_LENGTH).unwrap_or(0)))
                .saturating_mul(u64::from(t.tag_uint(&raw, tag::SAMPLES_PER_PIXEL).unwrap_or(1)))
                <= 1_000_000;
        return (tuned && tiled_jpeg)
            || t.tag_uint(&raw, tag::SAMPLE_FORMAT) == Some(3)
            || compression == 34892
            || (compression == 52546 && (tuned || cfg!(target_arch = "wasm32")))
            || (compression == 8 && bits < 16)
            || (tuned && compression == 1 && bits == 16 && !raw.has(tag::TILE_WIDTH));
    }
    if format == RawFormat::Arw
        && let Some(t) = Tiff::new(bytes)
        && let Some(raw) = crate::tiffep::raw_ifd(&t, &t.all_ifds())
    {
        return crate::sony::is_craw(&t, &raw);
    }
    if format == RawFormat::Rw2
        && let Some(t) = Tiff::new(bytes)
        && let Some(raw) = t.ifd_at(t.first_ifd, 0)
    {
        // RawFormat 3/4 remain FlashRaw: rare predictor cases in
        // the original implementation failed independent sample checks.
        return tuned && t.tag_uint(&raw, 0x2d) == Some(5);
    }
    if format == RawFormat::Orf
        && let Some(t) = Tiff::new(bytes)
        && let Some(i) = t.ifd_at(t.first_ifd, 0)
    {
        let pixels = u64::from(t.tag_uint(&i, tag::IMAGE_WIDTH).unwrap_or(0)) * u64::from(t.tag_uint(&i, tag::IMAGE_LENGTH).unwrap_or(0));
        let stored: u64 = t.tag_uints(&i, tag::STRIP_BYTE_COUNTS).iter().map(|v| u64::from(*v)).sum();
        return tuned && pixels > 0 && t.tag_uint(&i, tag::COMPRESSION).unwrap_or(1) == 1 && stored >= pixels.saturating_mul(2);
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::{
        DecodeBackend, Limits, decode_with_backend_traced,
        testgen::{DngSpec, DngStorage},
    };
    #[test]
    fn auto_routes_by_encoding_and_explicit_backend_still_wins() {
        for (bits, storage, wanted) in [
            (16, DngStorage::Strips { rows: 96 }, DecodeBackend::Builtin),
            (12, DngStorage::Strips { rows: 96 }, DecodeBackend::FlashRaw),
            (14, DngStorage::Lj92Tiles { width: 32, height: 32 }, DecodeBackend::Builtin),
            (14, DngStorage::Lj92Strips { rows: 96 }, DecodeBackend::FlashRaw),
        ] {
            let mut s = DngSpec::cfa(128, 96, vec![1000; 128 * 96]);
            s.bits = bits;
            s.storage = storage;
            let bytes = s.build();
            let (auto, backend) = decode_with_backend_traced(&bytes, &Limits::default(), DecodeBackend::Auto).unwrap();
            let wanted = if cfg!(target_arch = "aarch64") { wanted } else { DecodeBackend::FlashRaw };
            assert_eq!(backend, wanted, "bits={bits}");
            let (forced, backend) = decode_with_backend_traced(&bytes, &Limits::default(), DecodeBackend::FlashRaw).unwrap();
            assert_eq!(backend, DecodeBackend::FlashRaw);
            assert_eq!(auto.data, forced.data);
            assert_eq!(auto.cfa, forced.cfa);
        }
    }
}
