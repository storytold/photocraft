//! Per-preparation reuse of successful native raster-source work, never a source evaluator.
use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::SmartObject;
use photocraft_raster::Surface;
use std::collections::HashMap;

const MAX_ENTRIES: usize = 256;
const HASH_BLOCK: usize = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Key {
    source: [u8; 32],
    profile: Option<[u8; 32]>,
    size: [u32; 2],
    format: PixelFormat,
    transform: [u64; 6],
    dpi_light: [u32; 3],
    filters_enabled: bool,
}

fn digest(bytes: &[u8], ctx: &JobCtx) -> Result<[u8; 32]> {
    let mut hash = blake3::Hasher::new();
    for chunk in bytes.chunks(HASH_BLOCK) {
        ctx.check()?;
        hash.update(chunk);
    }
    ctx.check()?;
    Ok(*hash.finalize().as_bytes())
}

fn equal_bytes(left: &[u8], right: &[u8], ctx: &JobCtx) -> Result<bool> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, b) in left.chunks(HASH_BLOCK).zip(right.chunks(HASH_BLOCK)) {
        ctx.check()?;
        if a != b {
            return Ok(false);
        }
    }
    ctx.check()?;
    Ok(true)
}

impl Key {
    pub(super) fn new(doc: &Document, object: &SmartObject, bytes: &[u8], ctx: &JobCtx) -> Result<Option<Self>> {
        let SmartSource::Embedded { file_name, .. } = &object.source else { return Ok(None) };
        if !file_name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pcraft"))
            || !photocraft_format::is_pcraft(bytes)
            || !object.smart_filters.is_empty()
            || object.filter_mask.is_some()
            || object.warp.is_some()
            || object.perspective.is_some()
            || object.stack_mode.is_some()
            || !object.transform.m.iter().all(|value| value.is_finite())
            || ![doc.resolution_dpi, doc.global_light.angle, doc.global_light.altitude].iter().all(|value| value.is_finite())
        {
            return Ok(None);
        }
        Ok(Some(Self {
            source: digest(bytes, ctx)?,
            profile: doc.icc_profile.as_ref().map(|bytes| digest(bytes, ctx)).transpose()?,
            size: [doc.size.width, doc.size.height],
            format: doc.pixel_format(),
            transform: object.transform.m.map(f64::to_bits),
            dpi_light: [doc.resolution_dpi, doc.global_light.angle, doc.global_light.altitude].map(f32::to_bits),
            filters_enabled: object.filters_enabled,
        }))
    }
}

struct Entry {
    original: Arc<Vec<u8>>,
    profile: Option<Arc<Vec<u8>>>,
    surface: Surface,
}

#[derive(Default)]
pub(super) struct Memo {
    entries: HashMap<Key, Entry>,
}

impl Memo {
    pub(super) fn get(
        &self,
        key: &Key,
        bytes: &Arc<Vec<u8>>,
        profile: &Option<Arc<Vec<u8>>>,
        ctx: &JobCtx,
        charge: &mut impl FnMut(u64) -> Result<()>,
    ) -> Result<Option<(Arc<Vec<u8>>, Surface)>> {
        ctx.check()?;
        let Some(entry) = self.entries.get(key) else { return Ok(None) };
        if !Arc::ptr_eq(&entry.original, bytes) && !equal_bytes(&entry.original, bytes, ctx)? {
            return Ok(None);
        }
        match (&entry.profile, profile) {
            (Some(a), Some(b)) if !Arc::ptr_eq(a, b) && !equal_bytes(a, b, ctx)? => return Ok(None),
            (None, Some(_)) | (Some(_), None) => return Ok(None),
            _ => {}
        }
        charge(clone_bytes(&entry.surface)?)?;
        ctx.check()?;
        Ok(Some((entry.original.clone(), entry.surface.clone())))
    }

    pub(super) fn vacancy(&mut self, key: &Key, surface: &Surface, available: u64) -> Option<u64> {
        if self.entries.len() >= MAX_ENTRIES || self.entries.contains_key(key) {
            return None;
        }
        // Payload tiles/source/profile remain COW aliases covered by the original
        // unreleased unique-work debit. Price the additional map/entry metadata.
        let bytes = clone_bytes(surface).ok()?.checked_add(u64::try_from(std::mem::size_of::<Entry>() + std::mem::size_of::<Key>() + 128).ok()?)?;
        if bytes > available || self.entries.try_reserve(1).is_err() {
            return None;
        }
        Some(bytes)
    }

    pub(super) fn insert(&mut self, key: Key, original: Arc<Vec<u8>>, profile: Option<Arc<Vec<u8>>>, surface: &Surface) {
        self.entries.insert(key, Entry { original, profile, surface: surface.clone() });
    }
}

pub(super) fn clone_bytes(surface: &Surface) -> Result<u64> {
    u64::try_from(surface.tile_count())
        .ok()
        .and_then(|count| count.checked_mul(128))
        .and_then(|bytes| bytes.checked_add(surface.default_bytes().len() as u64))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Surface>() as u64 + 64))
        .ok_or_else(|| limit("memoized cache metadata"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::{LayerMask, SmartFilter};
    use photocraft_geom::{Affine, Size};

    fn fixture() -> (Document, SmartObject, Arc<Vec<u8>>) {
        let doc = Document::with_background("Memo fixture", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::BLACK);
        let bytes = Arc::new(smart_cmds::encode_source(&doc).unwrap());
        let object = SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: bytes.clone() }, Affine::IDENTITY, None);
        (doc, object, bytes)
    }

    #[test]
    fn memo_keys_qualify_complete_parent_context_and_geometry() {
        let (doc, object, bytes) = fixture();
        let ctx = JobCtx::new();
        let key = Key::new(&doc, &object, &bytes, &ctx).unwrap().unwrap();
        for variant in 0..7 {
            let mut changed = doc.clone();
            match variant {
                0 => changed.size.width = 2,
                1 => changed.depth = SampleType::U16,
                2 => changed.mode = ColorMode::Grayscale,
                3 => changed.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()),
                4 => changed.resolution_dpi = 144.,
                5 => changed.global_light.angle = 45.,
                _ => changed.global_light.altitude = 60.,
            }
            assert!(Key::new(&changed, &object, &bytes, &ctx).unwrap().unwrap() != key, "context{variant}");
        }
        let mut moved = object.clone();
        moved.transform = Affine::translate(1., 0.);
        assert!(Key::new(&doc, &moved, &bytes, &ctx).unwrap().unwrap() != key);
        moved.transform.m[0] = f64::NAN;
        assert!(Key::new(&doc, &moved, &bytes, &ctx).unwrap().is_none());
    }

    #[test]
    fn memo_does_not_accept_stored_filter_masks_or_external_sources() {
        let (doc, object, bytes) = fixture();
        let ctx = JobCtx::new();
        let mut filtered = object.clone();
        filtered.smart_filters.push(SmartFilter {
            command: "image.adjustments.invert".into(),
            params: serde_json::json!({}),
            blend: photocraft_doc::BlendMode::Normal,
            opacity: 1.,
            visible: true,
        });
        assert!(Key::new(&doc, &filtered, &bytes, &ctx).unwrap().is_none());
        let mut masked = object.clone();
        masked.filter_mask = Some(LayerMask::reveal_all());
        assert!(Key::new(&doc, &masked, &bytes, &ctx).unwrap().is_none());
        let mut projective = object.clone();
        projective.perspective = Some([1., 0., 0., 0., 1., 0., 0., 0., 1.]);
        assert!(Key::new(&doc, &projective, &bytes, &ctx).unwrap().is_none());
        let mut linked = object;
        linked.source = SmartSource::Linked { path: "not-permission.pcraft".into() };
        assert!(Key::new(&doc, &linked, &bytes, &ctx).unwrap().is_none());
    }

    #[test]
    fn digest_collisions_still_require_exact_bytes_and_profile_and_obey_cancellation() {
        let (mut doc, object, bytes) = fixture();
        doc.icc_profile = Some(Arc::new(vec![1, 2, 3]));
        let ctx = JobCtx::new();
        let key = Key::new(&doc, &object, &bytes, &ctx).unwrap().unwrap();
        let surface = doc.layers[0].surface().unwrap();
        let mut memo = Memo::default();
        memo.insert(key, bytes.clone(), doc.icc_profile.clone(), surface);
        let mut changed = bytes.as_ref().clone();
        changed[0] ^= 1;
        assert!(memo.get(&key, &Arc::new(changed), &doc.icc_profile, &ctx, &mut |_| Ok(())).unwrap().is_none(), "forced same key cannot bypass byte equality");
        assert!(memo.get(&key, &bytes, &Some(Arc::new(vec![1, 2, 4])), &ctx, &mut |_| Ok(())).unwrap().is_none(), "forced same key cannot bypass ICC equality");
        let cancelled = JobCtx::new();
        cancelled.cancel();
        assert!(matches!(memo.get(&key, &bytes, &doc.icc_profile, &cancelled, &mut |_| Ok(())), Err(EngineError::Cancelled)));
        assert!(matches!(Key::new(&doc, &object, &bytes, &cancelled), Err(EngineError::Cancelled)));
        let after_charge = JobCtx::new();
        let mut debited = 0;
        assert!(matches!(
            memo.get(&key, &bytes, &doc.icc_profile, &after_charge, &mut |amount| {
                debited = amount;
                after_charge.cancel();
                Ok(())
            }),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(debited, clone_bytes(surface).unwrap());
    }

    #[test]
    fn memo_entry_count_and_extra_cow_metadata_are_bounded() {
        let (doc, object, bytes) = fixture();
        let key = Key::new(&doc, &object, &bytes, &JobCtx::new()).unwrap().unwrap();
        let surface = doc.layers[0].surface().unwrap();
        let mut memo = Memo::default();
        assert!(memo.vacancy(&key, surface, 0).is_none());
        for index in 0..MAX_ENTRIES {
            let mut entry_key = key;
            entry_key.source[..8].copy_from_slice(&(index as u64).to_le_bytes());
            assert!(memo.vacancy(&entry_key, surface, u64::MAX).is_some());
            memo.insert(entry_key, bytes.clone(), None, surface);
        }
        let mut next = key;
        next.source[..8].copy_from_slice(&(MAX_ENTRIES as u64).to_le_bytes());
        assert!(memo.vacancy(&next, surface, u64::MAX).is_none());
    }
}
