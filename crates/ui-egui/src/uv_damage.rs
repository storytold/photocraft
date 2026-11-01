//! Retained COW snapshots recover sparse changes across multiple commits in one frame.
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, mask_paint::Stack};
use photocraft_geom::Rect;
use std::sync::Arc;

pub(crate) fn regions(previous: &mut Option<Arc<Document>>, doc: &Arc<Document>, fallback: Option<Rect>) -> Vec<Rect> {
    let old = previous.replace(doc.clone());
    let unchanged_document = old.as_ref().is_some_and(|old| Arc::ptr_eq(old, doc));
    // `damage_since` is the authoritative one-step damage journal for Session edits and live
    // strokes. Trust its conservative nonempty region directly: scanning every sparse mask tile
    // to rediscover the same rectangle costs O(all stack tiles) on every 3D pointer refresh.
    let hinted = fallback.filter(|rect| !rect.is_empty());
    let regions = if unchanged_document {
        hinted.into_iter().collect()
    } else if let Some(rect) = hinted {
        vec![rect]
    } else {
        match sparse(old.as_deref(), doc) {
            // Mask Paint stack edits can change the Gray UV display without changing
            // ordinary layer pixels. In that case sparse() correctly finds no raster
            // delta, and the engine's command damage is the only useful invalidation.
            Some(regions) if regions.is_empty() => fallback.filter(|rect| !rect.is_empty()).into_iter().collect(),
            Some(regions) => regions.into_iter().map(|r| hinted.map_or(r, |f| r.intersect(&f))).filter(|r| !r.is_empty()).collect(),
            None => vec![fallback.filter(|rect| !rect.is_empty()).unwrap_or_else(|| doc.bounds())],
        }
    };
    coalesce(regions.into_iter().map(|rect| rect.intersect(&doc.bounds())).filter(|rect| !rect.is_empty()).collect(), doc.bounds())
}

fn coalesce(mut regions: Vec<Rect>, bounds: Rect) -> Vec<Rect> {
    // Full adjustments must become one upload, not one submission per raster tile.
    regions.sort_by_key(|r| (r.y0, r.y1, r.x0, r.x1));
    let mut rows: Vec<Rect> = Vec::new();
    for r in regions {
        if let Some(last) = rows.last_mut()
            && last.y0 == r.y0
            && last.y1 == r.y1
            && r.x0 <= last.x1
        {
            last.x1 = last.x1.max(r.x1);
        } else {
            rows.push(r);
        }
    }
    rows.sort_by_key(|r| (r.x0, r.x1, r.y0, r.y1));
    let mut out: Vec<Rect> = Vec::new();
    for r in rows {
        if let Some(last) = out.last_mut()
            && last.x0 == r.x0
            && last.x1 == r.x1
            && r.y0 <= last.y1
        {
            last.y1 = last.y1.max(r.y1);
        } else {
            out.push(r);
        }
    }
    let area: u64 = out.iter().map(|r| u64::from(r.width()) * u64::from(r.height())).sum();
    if out.len() > 64 && area > u64::from(bounds.width()) * u64::from(bounds.height()) / 4 {
        vec![out.iter().fold(Rect::EMPTY, |a, b| a.union(b))]
    } else {
        out
    }
}

// Compare only changed encoded tiles. Equal rows are a byte comparison, without
// allocating float buffers or decoding untouched texels at a distant UV seam.
fn changed_bounds(a: Option<&[u8]>, b: Option<&[u8]>, default: &[u8], bpp: usize) -> Option<Rect> {
    let side = photocraft_geom::TILE_SIZE as usize;
    let stride = side.checked_mul(bpp)?;
    if stride == 0 {
        return None;
    }
    let mut bounds = Rect::EMPTY;
    for y in 0..side {
        let range = y * stride..(y + 1) * stride;
        let (a, b) = (a.and_then(|a| a.get(range.clone())), b.and_then(|b| b.get(range)));
        if a.zip(b).is_some_and(|(a, b)| a == b) {
            continue;
        }
        let equal = |x: usize| {
            let range = x * bpp..(x + 1) * bpp;
            a.and_then(|v| v.get(range.clone())).unwrap_or(default) == b.and_then(|v| v.get(range)).unwrap_or(default)
        };
        if let Some(left) = (0..side).find(|x| !equal(*x)) {
            let right = (left..side).rfind(|x| !equal(*x)).unwrap_or(left) + 1;
            bounds = bounds.union(&Rect::new(left as i32, y as i32, right as i32, y as i32 + 1));
        }
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// Compare the mask inputs that can affect rendering without invoking `LayerMask`/`Stack`
/// `PartialEq`, which walks every byte in every sparse tile. Surface tiles are copy-on-write, so
/// shared Arcs prove that their pixels are unchanged; a changed Arc is conservatively a change.
fn same_surface_storage(a: &photocraft_raster::Surface, b: &photocraft_raster::Surface) -> bool {
    if a.format() != b.format() || a.default_bytes() != b.default_bytes() {
        return false;
    }
    let mut a_tiles = a.tiles();
    let mut b_tiles = b.tiles();
    loop {
        match (a_tiles.next(), b_tiles.next()) {
            (None, None) => return true,
            (Some((a_coord, a_tile)), Some((b_coord, b_tile))) => {
                if a_coord != b_coord || !Arc::ptr_eq(a_tile, b_tile) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

fn same_mask_stack_inputs(a: &Stack, b: &Stack) -> bool {
    a.layers.len() == b.layers.len()
        && same_surface_storage(&a.base, &b.base)
        && a.layers.iter().zip(&b.layers).all(|(a, b)| {
            a.visible == b.visible
                && a.opacity == b.opacity
                && a.blend == b.blend
                && a.kind == b.kind
                && a.fill_map_size == b.fill_map_size
                && a.fill_bounds == b.fill_bounds
                && same_surface_storage(&a.surface, &b.surface)
                && match (a.fill_map.as_ref(), b.fill_map.as_ref()) {
                    (Some(a), Some(b)) => same_surface_storage(a, b),
                    (None, None) => true,
                    _ => false,
                }
        })
}

fn same_mask_pixel_inputs(a: Option<&LayerMask>, b: Option<&LayerMask>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.enabled == b.enabled
                && a.linked == b.linked
                && a.density == b.density
                && a.feather == b.feather
                && same_surface_storage(&a.surface, &b.surface)
                && match (a.paint.as_ref(), b.paint.as_ref()) {
                    (Some(a), Some(b)) => same_mask_stack_inputs(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

fn layer_masks_unchanged(old: &[Layer], new: &[Layer], depth: usize) -> bool {
    depth < 256
        && old.len() == new.len()
        && old.iter().zip(new).all(|(old, new)| {
            same_mask_pixel_inputs(old.mask.as_ref(), new.mask.as_ref())
                && match (old.children(), new.children()) {
                    (Some(old), Some(new)) => layer_masks_unchanged(old, new, depth + 1),
                    (None, None) => true,
                    _ => false,
                }
        })
}

fn sparse(old: Option<&Document>, doc: &Document) -> Option<Vec<Rect>> {
    let old = old.filter(|old| old.id == doc.id && old.size == doc.size && old.layers.len() == doc.layers.len())?;
    if !layer_masks_unchanged(&old.layers, &doc.layers, 0) {
        return None;
    }
    let mut probe = doc.clone();
    let mut regions = Vec::new();
    // Group/effect/adjustment changes use the compositor's conservative damage path.
    for (a, b) in old.layers.iter().zip(&mut probe.layers) {
        match (&a.content, &mut b.content) {
            (LayerContent::Raster(x), LayerContent::Raster(y)) => {
                if x.format() != y.format() || x.default_bytes() != y.default_bytes() {
                    return None;
                }
                let coords: std::collections::BTreeSet<_> = x.tiles().chain(y.tiles()).map(|(coord, _)| *coord).collect();
                for coord in coords {
                    let (a, b) = (x.tile(coord), y.tile(coord));
                    if a.zip(b).is_some_and(|(a, b)| Arc::ptr_eq(a, b)) {
                        continue;
                    }
                    if let Some(local) = changed_bounds(a.map(|t| t.bytes()), b.map(|t| t.bytes()), x.default_bytes(), x.format().bytes_per_pixel()) {
                        let origin = coord.rect();
                        regions.push(Rect::new(origin.x0 + local.x0, origin.y0 + local.y0, origin.x0 + local.x1, origin.y0 + local.y1));
                    }
                }
                *y = x.clone();
            }
            _ => return None,
        }
    }
    if !photocraft_engine::layer_multi_cmds::same_pixels_ignoring_masks(old, &probe) {
        return None;
    }
    let reach = crate::canvas::effect_reach(&doc.layers);
    Some(regions.into_iter().map(|r| r.inflate(reach).intersect(&doc.bounds())).filter(|r| !r.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_adjustments_merge_tiles_but_distant_islands_remain_separate() {
        let _gpu = photocraft_testkit::gpu_lock();
        let bounds = Rect::from_xywh(0, 0, 4096, 4096);
        let tiles = (0..16).flat_map(|y| (0..16).map(move |x| Rect::from_xywh(x * 256, y * 256, 256, 256))).collect();
        assert_eq!(coalesce(tiles, bounds), vec![bounds]);
        let islands = vec![Rect::from_xywh(1, 1, 20, 20), Rect::from_xywh(3800, 1, 20, 20)];
        assert_eq!(coalesce(islands.clone(), bounds), islands);
    }
    #[test]
    fn uv_damage_recovers_distant_tiles_across_multiple_commits_and_undo() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", serde_json::json!({"width":4096,"height":4096})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let mut previous = Some(before.clone());
        for x in [20, 3800] {
            s.edit("point", |doc, layer| {
                doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap().write_pixel(x, 20, &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        }
        let current = s.active().unwrap().doc.clone();
        // An empty damage hint means "no useful bound"; sparse changes are still recoverable.
        let rects = regions(&mut previous, &current, Some(Rect::EMPTY));
        assert_eq!(rects.len(), 2);
        assert_eq!(rects.iter().map(|r| u64::from(r.width()) * u64::from(r.height())).sum::<u64>(), 2);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(regions(&mut previous, &s.active().unwrap().doc, None).len(), 1);
    }

    #[test]
    fn empty_fallback_uses_canvas_only_when_sparse_diff_is_unavailable() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width":64,"height":48})).unwrap();
        let doc = session.active().unwrap().doc.clone();
        let mut no_snapshot = None;
        assert_eq!(regions(&mut no_snapshot, &doc, Some(Rect::EMPTY)), vec![doc.bounds()]);

        let mut unchanged_snapshot = Some(doc.clone());
        assert!(regions(&mut unchanged_snapshot, &doc, Some(Rect::EMPTY)).is_empty());
    }

    #[test]
    fn mask_change_detection_uses_cow_storage_and_ignores_stack_selection_metadata() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width":64,"height":48})).unwrap();
        session.execute("layer.layerMask.revealAll", serde_json::json!({})).unwrap();
        let doc = session.active().unwrap().doc.clone();
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let mut first = doc.layer(layer_id).unwrap().mask.clone().unwrap();
        first.paint = Some(Stack {
            base: first.surface.clone(),
            layers: vec![
                photocraft_doc::mask_paint::Paint {
                    name: "source".into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8),
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Paint,
                },
                photocraft_doc::mask_paint::Paint {
                    name: "blur".into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8),
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Blur { radius: 4 },
                },
            ],
            active: 0,
        });
        let mut metadata = first.clone();
        metadata.paint.as_mut().unwrap().active = 1;
        metadata.paint.as_mut().unwrap().layers[0].name = "renamed".into();
        assert!(same_mask_pixel_inputs(Some(&first), Some(&metadata)));

        let mut changed_tile = first.clone();
        changed_tile.paint.as_mut().unwrap().layers[0].surface.write_pixel(8, 8, &[1.0, 1.0, 1.0, 1.0]);
        assert!(!same_mask_pixel_inputs(Some(&first), Some(&changed_tile)), "a COW tile replacement invalidates mask damage");

        let mut changed = first.clone();
        changed.paint.as_mut().unwrap().layers[1].opacity = 0.5;
        assert!(!same_mask_pixel_inputs(Some(&first), Some(&changed)), "effect opacity changes mask pixels");

        let mut mask_doc = (*doc).clone();
        mask_doc.layer_mut(layer_id).unwrap().mask = Some(first.clone());
        let mut changed_mask_doc = mask_doc.clone();
        let changed_mask_surface = &mut changed_mask_doc.layer_mut(layer_id).unwrap().mask.as_mut().unwrap().surface;
        changed_mask_surface.write_pixel(8, 8, &[0.0]);
        assert!(photocraft_engine::layer_multi_cmds::same_pixels_ignoring_masks(&mask_doc, &changed_mask_doc));
        assert!(
            !photocraft_engine::layer_multi_cmds::same_pixels(&mask_doc, &changed_mask_doc),
            "regular pixel comparison still detects flattened mask cache changes"
        );
        let hinted_damage = Rect::new(4, 4, 16, 16);
        let mut hinted_snapshot = Some(Arc::new(mask_doc.clone()));
        assert_eq!(
            regions(&mut hinted_snapshot, &Arc::new(changed_mask_doc.clone()), Some(hinted_damage)),
            vec![hinted_damage],
            "a trusted live/Session hint avoids rescanning unchanged COW tile maps"
        );

        let mut previous = Some(doc.clone());
        session
            .edit("change mask effect", |doc, _| {
                let mask = doc.layer_mut(layer_id).unwrap().mask.as_mut().unwrap();
                mask.paint = Some(changed.paint.take().unwrap());
                Ok(())
            })
            .unwrap();
        let after = session.active().unwrap().doc.clone();
        assert_eq!(regions(&mut previous, &after, Some(Rect::EMPTY)), vec![after.bounds()], "a stack-only change falls back to canvas damage");
    }
}
