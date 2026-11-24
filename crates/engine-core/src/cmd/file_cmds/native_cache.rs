//! Rebuild self-contained Smart Object pixels after the native document decoder.

use photocraft_doc::{Document, LayerContent, SmartObject, SmartSource};
use photocraft_formats::Format;
use photocraft_io::memory::{MemorySnapshot, resolve_budget};

use crate::Result;
use crate::jobs::JobCtx;

/// An affine or perspective placement is bounded by its mapped corners. Mesh warp can extend beyond them.
fn eligible_source(smart: &SmartObject) -> Option<(&str, &[u8])> {
    let SmartSource::Embedded { file_name, bytes } = &smart.source else { return None };
    (smart.cache.is_none()
        && smart.stack_mode.is_none()
        && smart.vector_preview.is_none()
        && smart.warp.as_ref().is_none_or(|warp| warp.is_identity())
        && (!smart.filters_enabled || !smart.smart_filters.iter().any(|filter| filter.visible))
        && photocraft_formats::detect("", bytes) == Some(Format::Png))
    .then_some((file_name, bytes))
}

/// A candidate still missing pixels after recovery. Rendering over an existing cache would lose them.
pub(super) fn unrestored_embedded_layer(doc: &Document) -> Option<&str> {
    doc.walk().into_iter().find_map(|(_, _, layer)| match &layer.content {
        LayerContent::Smart(smart) if eligible_source(smart).is_some() => Some(layer.name.as_str()),
        _ => None,
    })
}

/// Rebuild caches and refresh warnings only when the decoder identified a native document.
pub fn recover_decoded_native_caches(decoded: &mut photocraft_io::DecodedOpen, ctx: &JobCtx) -> Result<()> {
    if decoded.native_pcraft && recover_native_embedded_caches(&mut decoded.imported.document, ctx)? {
        decoded.imported.warnings = photocraft_io::open_warnings(&decoded.imported.document);
    }
    Ok(())
}

/// Recover missing self-contained PNG Smart Object caches on a freshly decoded native document.
/// Callers pass the document before it is admitted to a session and recompute its import warnings
/// when caches were restored. External sources and visible Smart Filters need separate authority
/// and fidelity checks. The reader can still open their editable data and report missing pixels.
pub fn recover_native_embedded_caches(doc: &mut Document, ctx: &JobCtx) -> Result<bool> {
    ctx.check()?;
    let candidates: Vec<_> = doc
        .walk()
        .into_iter()
        .filter_map(|(path, _, layer)| matches!(&layer.content, LayerContent::Smart(smart) if eligible_source(smart).is_some()).then_some(path))
        .collect();
    if candidates.is_empty() {
        return Ok(false);
    }
    // Optional recovery uses the automatic import allowance. Previously recovered tiles
    // are still live when the next source is decoded and placed.
    let mut remaining = resolve_budget(MemorySnapshot::current(), Some(crate::allocation::MAX_DENSE_BYTES)).soft_limit;
    let mut recovered = false;
    for path in candidates {
        ctx.check()?;
        let Some(LayerContent::Smart(smart)) = doc.layer_at(&path).map(|layer| &layer.content) else { continue };
        let Some((name, bytes)) = eligible_source(smart) else { continue };
        let scratch_generation = photocraft_raster::spill::read_error_generation();
        let render = crate::smart_cmds::recover_raster_placement(doc, smart, name, bytes, remaining, ctx);
        ctx.check()?;
        let Ok(Some(surface)) = render else { continue };
        // Infallible tile reads can substitute pixels on scratch failure. Do not retain them.
        if photocraft_raster::spill::check_since(scratch_generation).is_err() {
            continue;
        }
        let bytes = photocraft_raster::stored_tiles_bytes(surface.tile_count() as u64, &surface.format());
        let Some(next) = remaining.checked_sub(bytes) else { break };
        if let Some(LayerContent::Smart(smart)) = doc.layer_at_mut(&path).map(|layer| &mut layer.content) {
            smart.cache = Some(surface);
            recovered = true;
            remaining = next;
        }
    }
    Ok(recovered)
}

#[cfg(test)]
#[path = "native_cache_budget_tests.rs"]
mod budget_tests;
