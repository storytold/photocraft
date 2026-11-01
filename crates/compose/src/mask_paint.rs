//! Incremental compositor for editable mask stack effects.
use photocraft_color::BlendMode;
use photocraft_doc::{
    LayerMask,
    mask_paint::{EffectKind, Stack},
};
use photocraft_geom::Rect;

const MAX_MASK_REGION_PIXELS: usize = 16 * 1024 * 1024;
const MAX_SHELL_BLUR_REGION_PIXELS: usize = 4 * 1024 * 1024;
const MAX_SHELL_ID_PIXELS: usize = 64 * 1024 * 1024;

/// `refresh(None)` has no canvas rectangle to clip a persisted Fill to. Keep its fallback work
/// bounded and let document callers use `refresh_in_canvas` for larger valid canvases.
const MAX_UNBOUNDED_FILL_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_UNBOUNDED_FILL_TILES: u64 = 256;

/// Borrowed row-major shell ownership raster. Zero is off-shell; nonzero values identify UV
/// shells. Coordinates outside `bounds` are treated as off-shell. Construct with [`Self::new`]
/// to validate the pixel count before passing it to a compositor entry point.
#[derive(Clone, Copy, Debug)]
pub struct ShellIdMap<'a> {
    bounds: Rect,
    ids: &'a [u32],
}

impl<'a> ShellIdMap<'a> {
    pub fn new(bounds: Rect, ids: &'a [u32]) -> Option<Self> {
        if bounds.is_empty() {
            return None;
        }
        let width = usize::try_from(bounds.width()).ok()?;
        let height = usize::try_from(bounds.height()).ok()?;
        let count = width.checked_mul(height)?;
        (count == ids.len() && count <= MAX_SHELL_ID_PIXELS).then_some(Self { bounds, ids })
    }

    fn id_at(self, x: i32, y: i32) -> u32 {
        if x < self.bounds.x0 || y < self.bounds.y0 || x >= self.bounds.x1 || y >= self.bounds.y1 {
            return 0;
        }
        let width = usize::try_from(self.bounds.width()).unwrap_or(0);
        let row = usize::try_from(i64::from(y) - i64::from(self.bounds.y0)).ok();
        let column = usize::try_from(i64::from(x) - i64::from(self.bounds.x0)).ok();
        let offset = row.and_then(|row| row.checked_mul(width)).and_then(|row| column.and_then(|column| row.checked_add(column)));
        offset.and_then(|index| self.ids.get(index).copied()).unwrap_or(0)
    }
}

/// Refresh changed tiles after an edit; property/order changes rebuild the stack.
pub fn refresh_changes(mask: &mut LayerMask, old: Option<&Stack>) {
    let _ = refresh_changes_bounded(mask, old, None, None);
}

/// Keep procedural Fill cache work inside the document even when its transform extends far
/// beyond the canvas. Source Paint surfaces remain intact and retain their sparse edit path.
pub fn refresh_changes_in_canvas(mask: &mut LayerMask, old: Option<&Stack>, canvas: Rect) {
    let needs_canvas = mask
        .paint
        .iter()
        .chain(old)
        .flat_map(|stack| &stack.layers)
        .any(|layer| (layer.fill_bounds.is_some() && matches!(&layer.kind, EffectKind::Fill { .. })) || matches!(layer.kind, EffectKind::Blur { .. }));
    let _ = refresh_changes_bounded(mask, old, needs_canvas.then_some(canvas), None);
}

/// Incrementally refresh mask changes with UV-shell-aware Blur. When stack properties require a
/// full rebuild, the supplied map is used for that rebuild as well as any sparse damage regions.
pub fn refresh_changes_in_canvas_with_shell_ids(mask: &mut LayerMask, old: Option<&Stack>, canvas: Rect, shell_ids: ShellIdMap<'_>) -> bool {
    if canvas.is_empty() {
        return false;
    }
    refresh_changes_bounded(mask, old, Some(canvas), Some(shell_ids))
}

fn refresh_changes_bounded(mask: &mut LayerMask, old: Option<&Stack>, canvas: Option<Rect>, shell_ids: Option<ShellIdMap<'_>>) -> bool {
    let Some(stack) = &mask.paint else { return false };
    let Some(old) = old else {
        return refresh_with_shell_ids(mask, canvas, shell_ids);
    };
    if stack.base != old.base || stack.layers.len() != old.layers.len() {
        return refresh_with_shell_ids(mask, canvas, shell_ids);
    }
    if stack
        .layers
        .iter()
        .chain(&old.layers)
        .any(|layer| matches!(&layer.kind, EffectKind::Paint) && layer.surface.default_pixel().get(3).is_some_and(|alpha| *alpha != 0.0))
    {
        return refresh_with_shell_ids(mask, canvas, shell_ids);
    }
    let mut changed = std::collections::BTreeSet::new();
    for (index, (now, before)) in stack.layers.iter().zip(&old.layers).enumerate() {
        let mut layer_changed = std::collections::BTreeSet::new();
        if now.fill_map != before.fill_map || now.fill_map_size != before.fill_map_size || now.fill_bounds != before.fill_bounds {
            return refresh_with_shell_ids(mask, canvas, shell_ids);
        }
        if now.surface.default_pixel() != before.surface.default_pixel() {
            if !matches!(&now.kind, EffectKind::Paint) || !matches!(&before.kind, EffectKind::Paint) {
                return refresh_with_shell_ids(mask, canvas, shell_ids);
            }
            layer_changed.extend(now.surface.tiles().map(|(coord, _)| *coord));
            layer_changed.extend(before.surface.tiles().map(|(coord, _)| *coord));
        }
        if now.visible != before.visible || now.opacity != before.opacity || now.kind != before.kind || now.blend != before.blend {
            // Fill and Levels can affect every pixel in the accumulated mask.
            if !matches!(&now.kind, EffectKind::Paint) || !matches!(&before.kind, EffectKind::Paint) {
                return refresh_with_shell_ids(mask, canvas, shell_ids);
            }
            layer_changed.extend(now.surface.tiles().map(|(coord, _)| *coord));
            layer_changed.extend(before.surface.tiles().map(|(coord, _)| *coord));
        }
        for (coord, tile) in now.surface.tiles() {
            if before.surface.tile(*coord).is_none_or(|prior| !std::sync::Arc::ptr_eq(prior, tile)) {
                layer_changed.insert(*coord);
            }
        }
        for (coord, _) in before.surface.tiles() {
            if now.surface.tile(*coord).is_none() {
                layer_changed.insert(*coord);
            }
        }
        let halo = stack
            .layers
            .iter()
            .skip(index.saturating_add(1))
            .filter(|layer| layer.visible && layer.opacity > 0.0)
            .filter_map(|layer| match &layer.kind {
                EffectKind::Blur { radius } => Some(i32::from(*radius)),
                _ => None,
            })
            .fold(0i32, i32::saturating_add);
        if halo > 0 {
            let mut expanded = std::collections::BTreeSet::new();
            for coord in layer_changed {
                let bounds = coord.rect().inflate(halo);
                let bounds = canvas.map_or(bounds, |canvas| bounds.intersect(&canvas));
                expanded.extend(bounds.tiles());
            }
            changed.extend(expanded);
        } else {
            changed.extend(layer_changed);
        }
    }
    if changed.is_empty() {
        return true;
    }
    if changed.len() <= 1 {
        for coord in changed {
            if !refresh_region_with_shell_ids(mask, coord.rect(), shell_ids) {
                return false;
            }
        }
        true
    } else {
        refresh_tiles_with_shell_ids(mask, changed.into_iter().collect(), shell_ids)
    }
}

fn refresh_with_shell_ids(mask: &mut LayerMask, canvas: Option<Rect>, shell_ids: Option<ShellIdMap<'_>>) -> bool {
    match (canvas, shell_ids) {
        (Some(canvas), Some(shell_ids)) => refresh_in_canvas_with_shell_ids(mask, canvas, shell_ids),
        (Some(canvas), None) => {
            refresh(mask, Some(canvas));
            true
        }
        (None, _) => {
            refresh(mask, None);
            true
        }
    }
}

/// Recompose only damaged output pixels, in bounded scanline bands. `region` describes source
/// pixels that changed; Blur layers above the active Paint layer can expand the output damage.
pub fn refresh(mask: &mut LayerMask, region: Option<Rect>) {
    let Some(stack) = &mask.paint else { return };
    let default = composite_default(stack);
    mask.surface.set_default_pixel(&[default]);
    let Some(bounds) = region else {
        let mut tiles = std::collections::BTreeSet::new();
        tiles.extend(mask.surface.tiles().map(|(c, _)| *c));
        tiles.extend(stack.base.tiles().map(|(c, _)| *c));
        for paint in &stack.layers {
            if matches!(&paint.kind, EffectKind::Paint) {
                tiles.extend(paint.surface.tiles().map(|(c, _)| *c));
            }
            if matches!(&paint.kind, EffectKind::Fill { .. })
                && let Some([x0, y0, x1, y1]) = paint.fill_bounds
                && x1 > x0
                && y1 > y0
            {
                let area = u64::try_from(i64::from(x1) - i64::from(x0))
                    .ok()
                    .and_then(|w| u64::try_from(i64::from(y1) - i64::from(y0)).ok().and_then(|h| w.checked_mul(h)));
                if area.is_none_or(|area| area > MAX_UNBOUNDED_FILL_PIXELS) {
                    continue;
                }
                let bounds = match &paint.kind {
                    EffectKind::Fill { rotation, .. } if rotation.is_finite() && rotation.rem_euclid(360.0) != 0.0 => {
                        let angle = f64::from(rotation.rem_euclid(360.0)).to_radians();
                        let (sin, cos) = angle.sin_cos();
                        let width = f64::from(x1) - f64::from(x0);
                        let height = f64::from(y1) - f64::from(y0);
                        let center_x = (f64::from(x0) + f64::from(x1)) * 0.5;
                        let center_y = (f64::from(y0) + f64::from(y1)) * 0.5;
                        let half_width = (cos.abs() * width + sin.abs() * height) * 0.5;
                        let half_height = (sin.abs() * width + cos.abs() * height) * 0.5;
                        Rect::new(
                            (center_x - half_width).floor() as i32,
                            (center_y - half_height).floor() as i32,
                            (center_x + half_width).ceil() as i32,
                            (center_y + half_height).ceil() as i32,
                        )
                    }
                    _ => Rect::new(x0, y0, x1, y1),
                };
                let width = (i64::from(bounds.x1) - i64::from(bounds.x0)).max(0) as u64;
                let height = (i64::from(bounds.y1) - i64::from(bounds.y0)).max(0) as u64;
                let tile_count = width.div_ceil(256).checked_mul(height.div_ceil(256));
                if u64::from(bounds.width()) * u64::from(bounds.height()) > MAX_UNBOUNDED_FILL_PIXELS
                    || tile_count.is_none_or(|tiles| tiles > MAX_UNBOUNDED_FILL_TILES)
                {
                    continue;
                }
                tiles.extend(bounds.tiles());
            }
        }
        let halo = stack
            .layers
            .iter()
            .filter(|layer| layer.visible && layer.opacity > 0.0)
            .filter_map(|layer| match &layer.kind {
                EffectKind::Blur { radius } => Some(i32::from(*radius)),
                _ => None,
            })
            .fold(0i32, i32::saturating_add);
        if halo > 0 {
            let source_tiles = tiles.iter().copied().collect::<Vec<_>>();
            for coord in source_tiles {
                tiles.extend(coord.rect().inflate(halo).tiles());
            }
        }
        refresh_tiles(mask, tiles.into_iter().collect());
        return;
    };
    if bounds.is_empty() {
        return;
    }
    let bounds = bounds.inflate(downstream_blur_reach(stack, stack.active));
    refresh_region(mask, bounds);
}

/// Refresh a live Paint edit, including its downstream Blur halo and clipping to the document.
/// The active stack layer is the edited Paint layer; earlier Blurs do not depend on its pixels.
pub fn refresh_damage_in_canvas(mask: &mut LayerMask, damage: Rect, canvas: Rect) {
    if damage.is_empty() || canvas.is_empty() {
        return;
    }
    let Some(stack) = &mask.paint else { return };
    let bounds = damage.inflate(downstream_blur_reach(stack, stack.active)).intersect(&canvas);
    refresh_region(mask, bounds);
}

/// Rebuild a mask cache with UV-shell-aware Blur. Existing [`refresh_in_canvas`] retains ordinary
/// image-wide Blur semantics when shell ownership is unavailable.
pub fn refresh_in_canvas_with_shell_ids(mask: &mut LayerMask, canvas: Rect, shell_ids: ShellIdMap<'_>) -> bool {
    if canvas.is_empty() {
        return false;
    }
    refresh_region_with_shell_ids(mask, canvas, Some(shell_ids))
}

/// Refresh live damage with UV-shell-aware Blur while preserving the normal bounded damage halo.
pub fn refresh_damage_in_canvas_with_shell_ids(mask: &mut LayerMask, damage: Rect, canvas: Rect, shell_ids: ShellIdMap<'_>) -> bool {
    if damage.is_empty() || canvas.is_empty() {
        return false;
    }
    let Some(stack) = &mask.paint else { return false };
    let bounds = damage.inflate(downstream_blur_reach(stack, stack.active)).intersect(&canvas);
    refresh_region_with_shell_ids(mask, bounds, Some(shell_ids))
}

/// Compose an immutable mask-stack snapshot only over changed output pixels, including the
/// spatial reach of visible Blur effects below the active Paint layer. The returned tuple is
/// `(default_value, output_bounds, row_major_pixels)` and can be applied to a transient cache.
/// `None` means the requested region is invalid or exceeds the bounded compositor budget.
pub fn compose_damage(stack: &Stack, damage: Rect, canvas: Rect) -> Option<(f32, Rect, Vec<f32>)> {
    if damage.is_empty() || canvas.is_empty() {
        return None;
    }
    let output = damage.inflate(downstream_blur_reach(stack, stack.active)).intersect(&canvas);
    if output.is_empty() {
        return None;
    }
    let pixels = compose_stack_region(stack, stack.layers.len(), output)?;
    Some((composite_default(stack), output, pixels))
}

/// Compose bounded live mask damage while constraining Blur to UV shell ownership.
pub fn compose_damage_with_shell_ids(stack: &Stack, damage: Rect, canvas: Rect, shell_ids: ShellIdMap<'_>) -> Option<(f32, Rect, Vec<f32>)> {
    if damage.is_empty() || canvas.is_empty() {
        return None;
    }
    let output = damage.inflate(downstream_blur_reach(stack, stack.active)).intersect(&canvas);
    if output.is_empty() {
        return None;
    }
    let pixels = compose_stack_region_with_shell_ids(stack, stack.layers.len(), output, shell_ids)?;
    Some((composite_default(stack), output, pixels))
}

/// Spatial reach of visible Blur layers after `index`, in document pixels.
pub fn downstream_blur_reach(stack: &Stack, index: usize) -> i32 {
    stack
        .layers
        .iter()
        .skip(index.saturating_add(1))
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .filter_map(|layer| match &layer.kind {
            EffectKind::Blur { radius } => Some(i32::from(*radius)),
            _ => None,
        })
        .fold(0i32, i32::saturating_add)
}

fn refresh_region(mask: &mut LayerMask, bounds: Rect) {
    let _ = refresh_region_with_shell_ids(mask, bounds, None);
}

fn refresh_region_with_shell_ids(mask: &mut LayerMask, bounds: Rect, shell_ids: Option<ShellIdMap<'_>>) -> bool {
    let Some(stack) = &mask.paint else { return false };
    if bounds.is_empty() {
        return true;
    }
    // Keep Blur regions narrow enough to expose parallel work even when one brush dab spans
    // the full 4K canvas width. Non-Blur stacks retain the wider bands to avoid extra writes.
    let bands = refresh_bands(bounds, if refresh_halo(stack) > 0 { 512 } else { 8_192 });
    if let Some(shell_ids) = shell_ids {
        // A failed shell-aware compose must not leave a partially refreshed surface. Clone the
        // sparse Surface (tile storage is copy-on-write), prepare bounded bands in parallel, then
        // swap it in only after all bands have completed successfully. A shell-aware Blur band is
        // independent because its full sequential halo is composed from the immutable stack.
        let mut updated = mask.surface.clone();
        updated.set_default_pixel(&[composite_default(stack)]);
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;

            let worker_count = rayon::current_num_threads().max(1);
            if worker_count > 1 && bands.len() > 1 {
                // Each band keeps several padded float buffers for sequential Gaussian stages.
                // Bound parallel scratch/output memory to 64 MiB; large halos reduce the batch
                // size automatically, down to one band at a time.
                const WORKING_SET_BUDGET_BYTES: usize = 64 * 1024 * 1024;
                let halo = refresh_halo(stack);
                let largest_region_bytes = bands
                    .iter()
                    .map(|band| {
                        let padded = band.inflate(halo);
                        usize::try_from(padded.width())
                            .ok()
                            .and_then(|width| usize::try_from(padded.height()).ok().and_then(|height| width.checked_mul(height)))
                            .and_then(|pixels| pixels.checked_mul(8 * std::mem::size_of::<f32>()))
                            .unwrap_or(usize::MAX)
                    })
                    .max()
                    .unwrap_or(0);
                let memory_workers = if largest_region_bytes == 0 { worker_count } else { (WORKING_SET_BUDGET_BYTES / largest_region_bytes).max(1) };
                let batch_size = worker_count.min(memory_workers).max(1);
                for batch in bands.chunks(batch_size) {
                    let composed =
                        batch.par_iter().map(|band| compose_stack_region_with_shell_ids(stack, stack.layers.len(), *band, shell_ids)).collect::<Vec<_>>();
                    if composed.iter().any(Option::is_none) {
                        return false;
                    }
                    for (band, pixels) in batch.iter().zip(composed) {
                        let Some(pixels) = pixels else { return false };
                        updated.write_region(*band, &pixels);
                    }
                }
            } else {
                for band in bands {
                    let Some(pixels) = compose_stack_region_with_shell_ids(stack, stack.layers.len(), band, shell_ids) else { return false };
                    updated.write_region(band, &pixels);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        for band in bands {
            let Some(pixels) = compose_stack_region_with_shell_ids(stack, stack.layers.len(), band, shell_ids) else { return false };
            updated.write_region(band, &pixels);
        }
        mask.surface = updated;
        return true;
    }
    mask.surface.set_default_pixel(&[composite_default(stack)]);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;

        let worker_count = rayon::current_num_threads().max(1);
        let halo = refresh_halo(stack);
        if worker_count > 1 && bands.len() > 1 && halo > 0 {
            // A Blur stage may keep several full-sized working buffers alive at once.
            // Bound concurrent regions to an estimated 64 MiB total (8 f32 buffers per
            // padded pixel); very large halo regions therefore fall back to one worker.
            const WORKING_SET_BUDGET_BYTES: usize = 64 * 1024 * 1024;
            let largest_region_bytes = bands
                .iter()
                .map(|band| {
                    let padded = band.inflate(halo);
                    usize::try_from(padded.width())
                        .ok()
                        .and_then(|width| usize::try_from(padded.height()).ok().and_then(|height| width.checked_mul(height)))
                        .and_then(|pixels| pixels.checked_mul(8 * std::mem::size_of::<f32>()))
                        .unwrap_or(usize::MAX)
                })
                .max()
                .unwrap_or(0);
            let memory_workers = if largest_region_bytes == 0 { worker_count } else { (WORKING_SET_BUDGET_BYTES / largest_region_bytes).max(1) };
            let batch_size = worker_count.min(memory_workers).max(1);
            for batch in bands.chunks(batch_size) {
                let composed = batch.par_iter().map(|band| compose_stack_region_impl(stack, stack.layers.len(), *band, shell_ids)).collect::<Vec<_>>();
                // Keep all Surface writes on this thread. Bands do not overlap, and
                // indexed Rayon collection preserves input order for deterministic writes.
                for (band, pixels) in batch.iter().zip(composed) {
                    if let Some(pixels) = pixels {
                        mask.surface.write_region(*band, &pixels);
                    }
                }
            }
            return true;
        }
    }

    for band in bands {
        if let Some(pixels) = compose_stack_region_impl(stack, stack.layers.len(), band, shell_ids) {
            mask.surface.write_region(band, &pixels);
        }
    }
    true
}

fn refresh_bands(bounds: Rect, max_width: i32) -> Vec<Rect> {
    let mut bands = Vec::new();
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let end = y.saturating_add(32).min(bounds.y1);
        let mut x = bounds.x0;
        while x < bounds.x1 {
            let end_x = x.saturating_add(max_width.max(1)).min(bounds.x1);
            bands.push(Rect::new(x, y, end_x, end));
            x = end_x;
        }
        y = end;
    }
    bands
}

fn refresh_halo(stack: &Stack) -> i32 {
    stack
        .layers
        .iter()
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .filter_map(|layer| match &layer.kind {
            EffectKind::Blur { radius } => Some(i32::from(*radius)),
            _ => None,
        })
        .fold(0i32, i32::saturating_add)
}

/// Rebuild a transient mask cache while clipping procedural Fill work to its document canvas.
/// Fill bounds are persisted input and can extend far beyond the canvas; iterating every tile in
/// those bounds is unnecessary because only canvas pixels are consumed by document rendering.
pub fn refresh_in_canvas(mask: &mut LayerMask, canvas: Rect) {
    if canvas.is_empty() {
        return;
    }
    let Some(stack) = &mask.paint else { return };
    mask.surface.set_default_pixel(&[composite_default(stack)]);
    refresh_region(mask, canvas);
}

/// Evaluate one output pixel of a Paint mask stack without allocating or materializing its
/// flattened surface. This is used for bounded transient display sampling; commits still call
/// [`refresh`] for the full-quality cache.
pub fn sample_stack_pixel(stack: &Stack, x: i32, y: i32) -> f32 {
    if stack.layers.iter().any(|layer| matches!(layer.kind, EffectKind::Blur { .. })) {
        return compose_stack_region(stack, stack.layers.len(), Rect::new(x, y, x.saturating_add(1), y.saturating_add(1)))
            .and_then(|pixels| pixels.first().copied())
            .unwrap_or_default();
    }
    let mut value = stack.base.sample_channel(x, y, 0);
    for layer in &stack.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.kind {
            EffectKind::Paint => {
                let source = layer.surface.sample_channel(x, y, 0);
                let alpha = if layer.surface.channels() > 3 { layer.surface.sample_channel(x, y, 3) } else { 0.0 } * layer.opacity;
                value = blend_coverage(layer.blend, value, source, alpha);
            }
            EffectKind::Fill { value: fill, .. } => {
                if let Some(fill) = fill_sample(layer, *fill, x, y) {
                    value = blend_coverage(layer.blend, value, fill, layer.opacity);
                }
            }
            EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                let adjusted = levels(value, *input_black, *gamma, *input_white, *output_black, *output_white);
                value = blend_coverage(layer.blend, value, adjusted, layer.opacity);
            }
            EffectKind::Blur { .. } => {}
        }
    }
    value
}

fn composite_default(stack: &Stack) -> f32 {
    let mut value = stack.base.default_pixel().first().copied().unwrap_or(1.0);
    for layer in &stack.layers {
        if layer.visible && layer.opacity > 0.0 {
            value = match &layer.kind {
                EffectKind::Paint => {
                    let source = layer.surface.default_pixel();
                    let alpha = source.get(3).copied().unwrap_or(0.0) * layer.opacity;
                    blend_coverage(layer.blend, value, source.first().copied().unwrap_or(0.0), alpha)
                }
                EffectKind::Fill { value: fill, .. } => {
                    let map_value = layer.fill_map.as_ref().map(|map| map.default_pixel().first().copied().unwrap_or(*fill)).unwrap_or(*fill);
                    if layer.fill_bounds.is_none() { blend_coverage(layer.blend, value, map_value, layer.opacity) } else { value }
                }
                EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                    let adjusted = levels(value, *input_black, *gamma, *input_white, *output_black, *output_white);
                    blend_coverage(layer.blend, value, adjusted, layer.opacity)
                }
                EffectKind::Blur { .. } => value,
            };
        }
    }
    value
}

fn levels(value: f32, input_black: f32, gamma: f32, input_white: f32, output_black: f32, output_white: f32) -> f32 {
    let width = (input_white - input_black).max(f32::EPSILON);
    let normalized = ((value - input_black) / width).clamp(0.0, 1.0);
    let exponent = 1.0 / gamma.clamp(0.01, 100.0);
    output_black + normalized.powf(exponent) * (output_white - output_black)
}

/// Composite the stack prefix into an unquantized grayscale buffer for one bounded region.
/// `end_layer` is exclusive; tile-aligned regions are recommended for stroke preview caches.
pub fn compose_prefix(stack: &Stack, end_layer: usize, region: Rect) -> Option<Vec<f32>> {
    if end_layer > stack.layers.len() || region.is_empty() {
        return None;
    }
    let count = usize::try_from(region.width()).ok()?.checked_mul(usize::try_from(region.height()).ok()?)?;
    if count > 256 * 256 * 4 {
        return None;
    }
    compose_stack_region(stack, end_layer, region)
}

/// Compose a stack prefix while preserving UV-shell-aware semantics for Blur stages.
/// `shell_ids` must cover the document canvas that contains `region`; regions outside it
/// continue to use the shell map's off-shell behavior, matching ordinary padded composition.
pub fn compose_prefix_with_shell_ids(stack: &Stack, end_layer: usize, region: Rect, shell_ids: ShellIdMap<'_>) -> Option<Vec<f32>> {
    if end_layer > stack.layers.len() || region.is_empty() {
        return None;
    }
    compose_stack_region_with_shell_ids(stack, end_layer, region, shell_ids)
}

/// Apply stack layers from `start_layer` onward to a cached prefix buffer and write one region.
/// `initial` must be an unquantized, row-major grayscale buffer matching `region` exactly.
pub fn refresh_from_prefix(mask: &mut LayerMask, region: Rect, start_layer: usize, initial: &[f32]) -> bool {
    if region.is_empty() {
        return false;
    }
    let Some(stack) = mask.paint.as_ref() else { return false };
    let Some(count) = usize::try_from(region.width()).ok().and_then(|w| usize::try_from(region.height()).ok().and_then(|h| w.checked_mul(h))) else {
        return false;
    };
    if count != initial.len()
        || start_layer > stack.layers.len()
        || count > 256 * 256 * 4
        || stack
            .layers
            .get(start_layer..)
            .unwrap_or_default()
            .iter()
            .any(|layer| layer.visible && layer.opacity > 0.0 && matches!(layer.kind, EffectKind::Blur { .. }))
    {
        return false;
    }
    let pixels = composite_layers(stack, start_layer, stack.layers.len(), region, initial.to_vec());
    mask.surface.set_default_pixel(&[composite_default(stack)]);
    mask.surface.write_region(region, &pixels);
    true
}

fn composite_layers(stack: &Stack, start_layer: usize, end_layer: usize, region: Rect, mut pixels: Vec<f32>) -> Vec<f32> {
    for layer in stack.layers.get(start_layer..end_layer).unwrap_or_default() {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.kind {
            EffectKind::Paint => {
                composite_paint_region(&mut pixels, &layer.surface, region, layer.opacity, layer.blend);
            }
            EffectKind::Fill { value, .. } => {
                for (offset, dst) in pixels.iter_mut().enumerate() {
                    let x = region.x0 + (offset as i32 % region.width() as i32);
                    let y = region.y0 + (offset as i32 / region.width() as i32);
                    if let Some(fill) = fill_sample(layer, *value, x, y) {
                        *dst = blend_coverage(layer.blend, *dst, fill, layer.opacity);
                    }
                }
            }
            EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                for dst in &mut pixels {
                    let adjusted = levels(*dst, *input_black, *gamma, *input_white, *output_black, *output_white);
                    *dst = blend_coverage(layer.blend, *dst, adjusted, layer.opacity);
                }
            }
            EffectKind::Blur { .. } => {}
        }
    }
    pixels
}

fn composite_paint_region(dst: &mut [f32], surface: &photocraft_raster::Surface, region: Rect, opacity: f32, mode: BlendMode) {
    let tile = photocraft_geom::TILE_SIZE;
    if mode == BlendMode::Normal
        && region.width() == tile as u32
        && region.height() == tile as u32
        && region.x0.rem_euclid(tile) == 0
        && region.y0.rem_euclid(tile) == 0
    {
        let coord = photocraft_geom::TileCoord::new(region.x0.div_euclid(tile), region.y0.div_euclid(tile));
        if composite_rgba_tile_into_gray(dst, surface, coord, opacity) {
            return;
        }
    }
    let source = surface.read_region(region);
    for (dst, px) in dst.iter_mut().zip(source.as_chunks::<4>().0) {
        let alpha = px[3] * opacity;
        *dst = blend_coverage(mode, *dst, px[0], alpha);
    }
}

fn refresh_tiles(mask: &mut LayerMask, coords: Vec<photocraft_geom::TileCoord>) {
    let _ = refresh_tiles_with_shell_ids(mask, coords, None);
}

fn refresh_tiles_with_shell_ids(mask: &mut LayerMask, coords: Vec<photocraft_geom::TileCoord>, shell_ids: Option<ShellIdMap<'_>>) -> bool {
    let Some(stack) = mask.paint.as_ref() else { return false };
    if coords.is_empty() {
        return true;
    }
    if let Some(shell_ids) = shell_ids {
        let default = [composite_default(stack)];
        let regions = merge_adjacent_tiles(&coords);
        let mut updated = mask.surface.clone();
        updated.set_default_pixel(&default);
        for (region, tiles) in regions {
            if let Some(pixels) = compose_stack_region_with_shell_ids(stack, stack.layers.len(), region, shell_ids) {
                updated.write_region(region, &pixels);
                continue;
            }
            for coord in tiles {
                let tile_region = coord.rect();
                let Some(pixels) = compose_stack_region_with_shell_ids(stack, stack.layers.len(), tile_region, shell_ids) else { return false };
                updated.write_region(tile_region, &pixels);
            }
        }
        mask.surface = updated;
        return true;
    }
    let default = [composite_default(stack)];
    mask.surface.set_default_pixel(&default);
    let format = mask.surface.format();
    let default = mask.surface.default_pixel();
    let shared_default_tile = mask.surface.solid_tile(&default);
    let compose_tile = |coord: &photocraft_geom::TileCoord| {
        let region = coord.rect();
        let pixels = compose_stack_region_impl(stack, stack.layers.len(), region, shell_ids).unwrap_or_default();
        let mut output = photocraft_raster::Surface::with_default(format, &default);
        output.write_region(region, &pixels);
        let tile = output.tile(*coord).cloned().unwrap_or_else(|| shared_default_tile.clone());
        (*coord, tile)
    };
    let regions = merge_adjacent_tiles(&coords);
    let compose_region = |(region, tiles): &(Rect, Vec<photocraft_geom::TileCoord>)| {
        compose_stack_region_impl(stack, stack.layers.len(), *region, shell_ids).map(|pixels| {
            let mut output = photocraft_raster::Surface::with_default(format, &default);
            output.write_region(*region, &pixels);
            tiles.iter().map(|coord| (*coord, output.tile(*coord).cloned().unwrap_or_else(|| shared_default_tile.clone()))).collect::<Vec<_>>()
        })
    };
    #[cfg(not(target_arch = "wasm32"))]
    let region_output = {
        use rayon::prelude::*;
        if regions.len() >= 2 * rayon::current_num_threads() {
            regions.par_iter().map(compose_region).collect::<Vec<_>>()
        } else {
            regions.iter().map(compose_region).collect::<Vec<_>>()
        }
    };
    #[cfg(target_arch = "wasm32")]
    let region_output = regions.iter().map(compose_region).collect::<Vec<_>>();
    let mut output = Vec::with_capacity(coords.len());
    let mut fallback = Vec::new();
    for ((_, tiles), result) in regions.iter().zip(region_output) {
        if let Some(tiles) = result {
            output.extend(tiles);
        } else {
            // A merged region may exceed the bounded compositor budget even though its member
            // tiles were individually serviceable. Preserve the old exact per-tile fallback.
            fallback.extend(tiles.iter().copied());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        if fallback.len() >= 2 * rayon::current_num_threads() {
            output.extend(fallback.par_iter().map(compose_tile).collect::<Vec<_>>());
        } else {
            output.extend(fallback.iter().map(compose_tile));
        }
    }
    #[cfg(target_arch = "wasm32")]
    output.extend(fallback.iter().map(compose_tile));
    mask.surface.put_tiles(output);
    true
}

/// Merge changed tiles only when they form a complete rectangle. Any sparse gaps stay
/// separate, so a merged Blur halo never causes unrelated output pixels to be rewritten.
fn merge_adjacent_tiles(coords: &[photocraft_geom::TileCoord]) -> Vec<(Rect, Vec<photocraft_geom::TileCoord>)> {
    use photocraft_geom::TileCoord;
    let mut ordered = coords.to_vec();
    ordered.sort_unstable_by_key(|coord| (coord.ty, coord.tx));
    ordered.dedup();
    let mut remaining: std::collections::HashSet<_> = ordered.iter().copied().collect();
    let mut regions = Vec::new();
    for first in ordered {
        if !remaining.remove(&first) {
            continue;
        }
        let mut row = vec![first];
        let mut x = first.tx;
        while let Some(next_x) = x.checked_add(1) {
            let next = TileCoord::new(next_x, first.ty);
            if !remaining.remove(&next) {
                break;
            }
            row.push(next);
            x = next_x;
        }
        let mut tiles = row.clone();
        let mut y = first.ty;
        while let Some(next_y) = y.checked_add(1) {
            let next_row: Vec<_> = row.iter().map(|coord| TileCoord::new(coord.tx, next_y)).collect();
            if !next_row.iter().all(|coord| remaining.contains(coord)) {
                break;
            }
            for coord in &next_row {
                remaining.remove(coord);
            }
            tiles.extend(next_row);
            y = next_y;
        }
        let bounds = tiles.iter().map(TileCoord::rect).reduce(|a, b| a.union(&b)).unwrap_or(Rect::EMPTY);
        regions.push((bounds, tiles));
    }
    regions
}

fn blend_coverage(mode: BlendMode, below: f32, source: f32, coverage: f32) -> f32 {
    if mode == BlendMode::Normal {
        return source * coverage + below * (1.0 - coverage);
    }
    let blended = photocraft_color::blend::blend_channel(mode, below, source);
    below + (blended - below) * coverage.clamp(0.0, 1.0)
}

/// Compose a stack prefix while carrying only the halo required by Blur layers.
/// The halo is bounded to 64 pixels per effect by command and format validation.
fn compose_stack_region(stack: &Stack, end_layer: usize, requested: Rect) -> Option<Vec<f32>> {
    compose_stack_region_impl(stack, end_layer, requested, None)
}

/// Compose a bounded mask-stack region using UV shell IDs for every Blur stage. The ID map is a
/// row-major grid in document coordinates; IDs use zero for off-shell and nonzero shell labels.
/// Off-shell values can receive outward Blur from occupied pixels, but cannot feed pixels inside
/// a shell. Existing callers can keep using the ordinary image-wide composition path.
pub fn compose_stack_region_with_shell_ids(stack: &Stack, end_layer: usize, requested: Rect, shell_ids: ShellIdMap<'_>) -> Option<Vec<f32>> {
    let output_width = usize::try_from(requested.width()).ok()?;
    let output_height = usize::try_from(requested.height()).ok()?;
    let count = output_width.checked_mul(output_height)?;
    if requested.is_empty() || count > MAX_MASK_REGION_PIXELS {
        return None;
    }
    let mut output = Vec::new();
    output.try_reserve_exact(count).ok()?;
    output.resize(count, 0.0f32);
    let mut split_budget = 4096usize;
    compose_shell_region_into(stack, end_layer, requested, requested, shell_ids, &mut output, &mut split_budget).then_some(output)
}

fn compose_shell_region_into(
    stack: &Stack,
    end_layer: usize,
    region: Rect,
    output_bounds: Rect,
    shell_ids: ShellIdMap<'_>,
    output: &mut [f32],
    split_budget: &mut usize,
) -> bool {
    if let Some(pixels) = compose_stack_region_impl(stack, end_layer, region, Some(shell_ids)) {
        let region_width = match usize::try_from(region.width()) {
            Ok(width) => width,
            Err(_) => return false,
        };
        let output_width = match usize::try_from(output_bounds.width()) {
            Ok(width) => width,
            Err(_) => return false,
        };
        let x_offset = match usize::try_from(i64::from(region.x0) - i64::from(output_bounds.x0)) {
            Ok(offset) => offset,
            Err(_) => return false,
        };
        let y_offset = match usize::try_from(i64::from(region.y0) - i64::from(output_bounds.y0)) {
            Ok(offset) => offset,
            Err(_) => return false,
        };
        let height = match usize::try_from(region.height()) {
            Ok(height) => height,
            Err(_) => return false,
        };
        for row in 0..height {
            let Some(source_start) = row.checked_mul(region_width) else { return false };
            let Some(target_start) =
                y_offset.checked_add(row).and_then(|target_row| target_row.checked_mul(output_width)).and_then(|target_row| target_row.checked_add(x_offset))
            else {
                return false;
            };
            let Some(source) = pixels.get(source_start..source_start.saturating_add(region_width)) else { return false };
            let Some(target) = output.get_mut(target_start..target_start.saturating_add(region_width)) else { return false };
            target.copy_from_slice(source);
        }
        return true;
    }

    if *split_budget == 0 || region.width() <= 1 && region.height() <= 1 {
        return false;
    }
    *split_budget -= 1;
    let width = i64::from(region.x1) - i64::from(region.x0);
    let height = i64::from(region.y1) - i64::from(region.y0);
    let (first, second) = if width >= height && width > 1 {
        let middle = (i64::from(region.x0) + width / 2) as i32;
        (Rect::new(region.x0, region.y0, middle, region.y1), Rect::new(middle, region.y0, region.x1, region.y1))
    } else if height > 1 {
        let middle = (i64::from(region.y0) + height / 2) as i32;
        (Rect::new(region.x0, region.y0, region.x1, middle), Rect::new(region.x0, middle, region.x1, region.y1))
    } else {
        return false;
    };
    compose_shell_region_into(stack, end_layer, first, output_bounds, shell_ids, output, split_budget)
        && compose_shell_region_into(stack, end_layer, second, output_bounds, shell_ids, output, split_budget)
}

fn compose_stack_region_impl(stack: &Stack, end_layer: usize, requested: Rect, shell_ids: Option<ShellIdMap<'_>>) -> Option<Vec<f32>> {
    if end_layer > stack.layers.len() || requested.is_empty() {
        return None;
    }
    let halo = stack
        .layers
        .iter()
        .take(end_layer)
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .filter_map(|layer| match &layer.kind {
            EffectKind::Blur { radius } => Some(i32::from(*radius)),
            _ => None,
        })
        .fold(0i32, i32::saturating_add);
    let region = requested.inflate(halo);
    let width = usize::try_from(region.width()).ok()?;
    let height = usize::try_from(region.height()).ok()?;
    let count = width.checked_mul(height)?;
    if count > MAX_MASK_REGION_PIXELS {
        return None;
    }
    let shell_aware_blur = shell_ids.is_some()
        && stack.layers.iter().take(end_layer).any(|layer| layer.visible && layer.opacity > 0.0 && matches!(layer.kind, EffectKind::Blur { .. }));
    if shell_aware_blur && count > MAX_SHELL_BLUR_REGION_PIXELS {
        return None;
    }
    let mut pixels = stack.base.read_region(region);
    let mut current = region;
    for layer in stack.layers.iter().take(end_layer) {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.kind {
            EffectKind::Paint => {
                composite_paint_region(&mut pixels, &layer.surface, current, layer.opacity, layer.blend);
            }
            EffectKind::Fill { value, .. } => {
                for (offset, dst) in pixels.iter_mut().enumerate() {
                    let x = current.x0 + (offset % current.width() as usize) as i32;
                    let y = current.y0 + (offset / current.width() as usize) as i32;
                    if let Some(fill) = fill_sample(layer, *value, x, y) {
                        *dst = blend_coverage(layer.blend, *dst, fill, layer.opacity);
                    }
                }
            }
            EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                for dst in &mut pixels {
                    let adjusted = levels(*dst, *input_black, *gamma, *input_white, *output_black, *output_white);
                    *dst = blend_coverage(layer.blend, *dst, adjusted, layer.opacity);
                }
            }
            EffectKind::Blur { radius } => {
                let (blurred, next_region) = match shell_ids {
                    Some(shell_ids) => gaussian_blur_region_with_shell_ids(&pixels, current, *radius, shell_ids)?,
                    None => gaussian_blur_region(&pixels, current, *radius)?,
                };
                let r = usize::from(*radius);
                let old_width = usize::try_from(current.width()).ok()?;
                let new_width = usize::try_from(next_region.width()).ok()?;
                let mut mixed = Vec::with_capacity(blurred.len());
                for y in 0..usize::try_from(next_region.height()).ok()? {
                    for x in 0..new_width {
                        let old = pixels.get((y + r).checked_mul(old_width)?.checked_add(x + r)?)?;
                        let value = blurred.get(y.checked_mul(new_width)?.checked_add(x)?)?;
                        mixed.push(blend_coverage(layer.blend, *old, *value, layer.opacity));
                    }
                }
                pixels = mixed;
                current = next_region;
            }
        }
    }
    let ox = usize::try_from(requested.x0 - current.x0).ok()?;
    let oy = usize::try_from(requested.y0 - current.y0).ok()?;
    let rw = usize::try_from(requested.width()).ok()?;
    let rh = usize::try_from(requested.height()).ok()?;
    let cw = usize::try_from(current.width()).ok()?;
    let mut out = Vec::with_capacity(rw.checked_mul(rh)?);
    for y in oy..oy.checked_add(rh)? {
        out.extend_from_slice(pixels.get(y.checked_mul(cw)?.checked_add(ox)?..y.checked_mul(cw)?.checked_add(ox)?.checked_add(rw)?)?);
    }
    Some(out)
}

fn gaussian_blur_region(input: &[f32], region: Rect, radius: u8) -> Option<(Vec<f32>, Rect)> {
    let r = i32::from(radius);
    let output = Rect::new(region.x0.saturating_add(r), region.y0.saturating_add(r), region.x1.saturating_sub(r), region.y1.saturating_sub(r));
    if output.is_empty() {
        return None;
    }
    let width = usize::try_from(region.width()).ok()?;
    let height = usize::try_from(region.height()).ok()?;
    if width.checked_mul(height)? != input.len() {
        return None;
    }
    let sigma = (f32::from(radius) / 3.0).max(0.5);
    let mut kernel = Vec::with_capacity(usize::from(radius).saturating_mul(2).saturating_add(1));
    let mut total = 0.0f32;
    for dx in -r..=r {
        let weight = (-0.5 * (dx as f32 / sigma).powi(2)).exp();
        kernel.push(weight);
        total += weight;
    }
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    for weight in &mut kernel {
        *weight /= total;
    }
    let ow = usize::try_from(output.width()).ok()?;
    let oh = usize::try_from(output.height()).ok()?;
    let mut temp = vec![0.0; ow.checked_mul(height)?];
    for y in 0..height {
        for x in 0..ow {
            let center = x + usize::try_from(r).ok()?;
            let mut value = 0.0;
            for (k, weight) in kernel.iter().enumerate() {
                let sx = center + k - usize::try_from(r).ok()?;
                value += input[y * width + sx] * weight;
            }
            temp[y * ow + x] = value;
        }
    }
    let mut output_pixels = vec![0.0; ow.checked_mul(oh)?];
    for y in 0..oh {
        let center = y + usize::try_from(r).ok()?;
        for x in 0..ow {
            let mut value = 0.0;
            for (k, weight) in kernel.iter().enumerate() {
                let sy = center + k - usize::try_from(r).ok()?;
                value += temp[sy * ow + x] * weight;
            }
            output_pixels[y * ow + x] = value;
        }
    }
    Some((output_pixels, output))
}

/// Separable shell-aware Gaussian. For an inside-shell output, each pass keeps samples with the
/// same shell ID and divides by the accumulated support weight, so constants stay constant up to
/// the shell boundary. For an off-shell output, the source pass keeps only occupied shell pixels
/// and does not renormalize; this allows Blur to fade outward without allowing off-shell values
/// to flow back into shells.
fn gaussian_blur_region_with_shell_ids(input: &[f32], region: Rect, radius: u8, shell_ids: ShellIdMap<'_>) -> Option<(Vec<f32>, Rect)> {
    let r = i32::from(radius);
    let output = if r == 0 {
        region
    } else {
        Rect::new(region.x0.saturating_add(r), region.y0.saturating_add(r), region.x1.saturating_sub(r), region.y1.saturating_sub(r))
    };
    if output.is_empty() {
        return None;
    }
    let width = usize::try_from(region.width()).ok()?;
    let height = usize::try_from(region.height()).ok()?;
    if width.checked_mul(height)? != input.len() || input.len() > MAX_SHELL_BLUR_REGION_PIXELS {
        return None;
    }
    if r == 0 {
        return Some((input.to_vec(), output));
    }
    let mut kernel = Vec::new();
    kernel.try_reserve_exact(usize::from(radius).checked_mul(2)?.checked_add(1)?).ok()?;
    let sigma = (f32::from(radius) / 3.0).max(0.5);
    let mut total = 0.0f32;
    for delta in -r..=r {
        let weight = (-0.5 * (delta as f32 / sigma).powi(2)).exp();
        kernel.push(weight);
        total += weight;
    }
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    for weight in &mut kernel {
        *weight /= total;
    }

    let output_width = usize::try_from(output.width()).ok()?;
    let output_height = usize::try_from(output.height()).ok()?;
    let horizontal_len = output_width.checked_mul(height)?;
    let mut shell_values = Vec::new();
    shell_values.try_reserve_exact(horizontal_len).ok()?;
    shell_values.resize(horizontal_len, 0.0f32);
    let mut shell_weights = Vec::new();
    shell_weights.try_reserve_exact(horizontal_len).ok()?;
    shell_weights.resize(horizontal_len, 0.0f32);
    let mut outward_values = Vec::new();
    outward_values.try_reserve_exact(horizontal_len).ok()?;
    outward_values.resize(horizontal_len, 0.0f32);

    for y in 0..height {
        let absolute_y = region.y0.saturating_add(i32::try_from(y).ok()?);
        for x in 0..output_width {
            let qx = region.x0.saturating_add(r).saturating_add(i32::try_from(x).ok()?);
            let q_shell = shell_ids.id_at(qx, absolute_y);
            let output_index = y.checked_mul(output_width)?.checked_add(x)?;
            for (kernel_index, weight) in kernel.iter().copied().enumerate() {
                let sx = x.checked_add(kernel_index)?;
                let source_x = region.x0.saturating_add(i32::try_from(sx).ok()?);
                let source_shell = shell_ids.id_at(source_x, absolute_y);
                let value = *input.get(y.checked_mul(width)?.checked_add(sx)?)?;
                if source_shell > 0 {
                    outward_values[output_index] += value * weight;
                }
                if q_shell > 0 && source_shell == q_shell {
                    shell_values[output_index] += value * weight;
                    shell_weights[output_index] += weight;
                }
            }
        }
    }

    let mut output_pixels = Vec::new();
    output_pixels.try_reserve_exact(output_width.checked_mul(output_height)?).ok()?;
    output_pixels.resize(output_width.checked_mul(output_height)?, 0.0f32);
    for y in 0..output_height {
        let py = region.y0.saturating_add(r).saturating_add(i32::try_from(y).ok()?);
        for x in 0..output_width {
            let px = region.x0.saturating_add(r).saturating_add(i32::try_from(x).ok()?);
            let p_shell = shell_ids.id_at(px, py);
            let mut value = 0.0f32;
            let mut support = 0.0f32;
            for (kernel_index, weight) in kernel.iter().copied().enumerate() {
                let sy = y.checked_add(kernel_index)?;
                let qy = region.y0.saturating_add(i32::try_from(sy).ok()?);
                let index = sy.checked_mul(output_width)?.checked_add(x)?;
                if p_shell == 0 {
                    value += *outward_values.get(index)? * weight;
                } else if shell_ids.id_at(px, qy) == p_shell {
                    value += *shell_values.get(index)? * weight;
                    support += *shell_weights.get(index)? * weight;
                }
            }
            let output_index = y.checked_mul(output_width)?.checked_add(x)?;
            output_pixels[output_index] = if p_shell == 0 {
                value
            } else if support > 0.0 {
                value / support
            } else {
                0.0
            };
        }
    }
    Some((output_pixels, output))
}

/// Compose a mask stack into a small preview grid. Spatial Blur radii are scaled to the
/// preview resolution so callers never perform a full-resolution convolution for each sample.
pub fn sample_stack_resampled(stack: &Stack, region: Rect, size: [u32; 2]) -> Option<Vec<f32>> {
    if region.is_empty() || size[0] == 0 || size[1] == 0 || size[0] > 256 || size[1] > 256 {
        return None;
    }
    let width = usize::try_from(size[0]).ok()?;
    let height = usize::try_from(size[1]).ok()?;
    let sx = region.width() as f32 / size[0] as f32;
    let sy = region.height() as f32 / size[1] as f32;
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return None;
    }
    // A later Blur consumes the already blurred lower prefix, so its support adds to
    // every earlier Blur's support. Keep that full halo around the requested preview
    // grid and crop only after all effects have run. Clamping each Gaussian at the
    // requested grid edge makes previews disagree with the same region in a larger
    // UV map and creates visible seams while panning.
    let blur_radii: Vec<usize> = stack
        .layers
        .iter()
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .filter_map(|layer| match &layer.kind {
            EffectKind::Blur { radius } => Some((f32::from(*radius) / sx.max(sy)).round().clamp(0.0, 64.0) as usize),
            _ => None,
        })
        .collect();
    let padding = blur_radii.iter().try_fold(0usize, |total, radius| total.checked_add(*radius))?;
    let padded_width = width.checked_add(padding.checked_mul(2)?)?;
    let padded_height = height.checked_add(padding.checked_mul(2)?)?;
    let count = padded_width.checked_mul(padded_height)?;
    // Keep temporary Gaussian buffers bounded. Returning None lets the caller skip
    // an unsupported extreme preview rather than silently drawing a clipped Blur.
    if count > MAX_MASK_REGION_PIXELS / 3 {
        return None;
    }
    let sample_x = |x: usize| region.x0.saturating_add(((x as f32 - padding as f32 + 0.5) * sx).floor() as i32);
    let sample_y = |y: usize| region.y0.saturating_add(((y as f32 - padding as f32 + 0.5) * sy).floor() as i32);
    let mut values = vec![0.0; count];
    for y in 0..padded_height {
        for x in 0..padded_width {
            values[y * padded_width + x] = stack.base.sample_channel(sample_x(x), sample_y(y), 0);
        }
    }
    let mut blur_index = 0;
    for layer in &stack.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.kind {
            EffectKind::Paint => {
                for y in 0..padded_height {
                    for x in 0..padded_width {
                        let dx = sample_x(x);
                        let dy = sample_y(y);
                        let source = layer.surface.sample_channel(dx, dy, 0);
                        let alpha = if layer.surface.channels() > 3 { layer.surface.sample_channel(dx, dy, 3) } else { 0.0 } * layer.opacity;
                        let index = y * padded_width + x;
                        values[index] = blend_coverage(layer.blend, values[index], source, alpha);
                    }
                }
            }
            EffectKind::Fill { value, .. } => {
                for y in 0..padded_height {
                    for x in 0..padded_width {
                        let dx = sample_x(x);
                        let dy = sample_y(y);
                        if let Some(fill) = fill_sample(layer, *value, dx, dy) {
                            let index = y * padded_width + x;
                            values[index] = blend_coverage(layer.blend, values[index], fill, layer.opacity);
                        }
                    }
                }
            }
            EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } => {
                for value in &mut values {
                    let adjusted = levels(*value, *input_black, *gamma, *input_white, *output_black, *output_white);
                    *value = blend_coverage(layer.blend, *value, adjusted, layer.opacity);
                }
            }
            EffectKind::Blur { .. } => {
                let scaled = u8::try_from(*blur_radii.get(blur_index)?).ok()?;
                blur_index += 1;
                if scaled > 0
                    && let Some(blurred) = gaussian_blur_grid(&values, padded_width, padded_height, scaled)
                {
                    for (value, blur) in values.iter_mut().zip(blurred) {
                        *value = blend_coverage(layer.blend, *value, blur, layer.opacity);
                    }
                }
            }
        }
    }
    let output_count = width.checked_mul(height)?;
    let mut output = vec![0.0; output_count];
    for y in 0..height {
        let source_start = (y + padding) * padded_width + padding;
        let target_start = y * width;
        output[target_start..target_start + width].copy_from_slice(&values[source_start..source_start + width]);
    }
    Some(output)
}

fn gaussian_blur_grid(input: &[f32], width: usize, height: usize, radius: u8) -> Option<Vec<f32>> {
    if width.checked_mul(height)? != input.len() {
        return None;
    }
    let r = usize::from(radius);
    let sigma = (f32::from(radius) / 3.0).max(0.5);
    let mut kernel = Vec::with_capacity(r.saturating_mul(2).saturating_add(1));
    let mut total = 0.0;
    for offset in -(r as i32)..=(r as i32) {
        let weight = (-0.5 * (offset as f32 / sigma).powi(2)).exp();
        kernel.push(weight);
        total += weight;
    }
    if total <= 0.0 || !total.is_finite() {
        return None;
    }
    for weight in &mut kernel {
        *weight /= total;
    }
    let mut temp = vec![0.0; input.len()];
    let mut output = vec![0.0; input.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = 0.0;
            for (i, weight) in kernel.iter().enumerate() {
                let ox = i as isize - r as isize;
                let sx = (x as isize + ox).clamp(0, width.saturating_sub(1) as isize) as usize;
                value += input[y * width + sx] * weight;
            }
            temp[y * width + x] = value;
        }
    }
    for y in 0..height {
        for x in 0..width {
            let mut value = 0.0;
            for (i, weight) in kernel.iter().enumerate() {
                let oy = i as isize - r as isize;
                let sy = (y as isize + oy).clamp(0, height.saturating_sub(1) as isize) as usize;
                value += temp[sy * width + x] * weight;
            }
            output[y * width + x] = value;
        }
    }
    Some(output)
}

fn fill_sample(layer: &photocraft_doc::mask_paint::Paint, value: f32, x: i32, y: i32) -> Option<f32> {
    let photocraft_doc::mask_paint::EffectKind::Fill { rotation, repeat_u, repeat_v, mirror_u, mirror_v, .. } = &layer.kind else { return Some(value) };
    if layer.fill_bounds.is_none() {
        // Legacy unbounded fills use canvas-pixel coordinates. Keep this path
        // separate from the normalized UV mapping used by spatial fills below.
        let Some(map) = &layer.fill_map else { return Some(value) };
        let [width, height] = layer.fill_map_size;
        if width == 0 || height == 0 {
            return Some(value);
        }
        let sample_axis = |local: i32, map_size: u32, repeat: bool, mirror: bool| {
            let size = i64::from(map_size);
            let local = i64::from(local);
            if repeat {
                let tile = local.div_euclid(size);
                let offset = local.rem_euclid(size);
                if mirror && tile.rem_euclid(2) == 1 { (size - 1 - offset) as i32 } else { offset as i32 }
            } else {
                ((local.max(0) * size / i64::from(i32::MAX)).min(size - 1)) as i32
            }
        };
        return Some(map.sample_channel(sample_axis(x, width, *repeat_u, *mirror_u), sample_axis(y, height, *repeat_v, *mirror_v), 0));
    }

    let [x0, y0, x1, y1] = layer.fill_bounds?;
    let repeat_u = layer.fill_map.is_some() && *repeat_u;
    let repeat_v = layer.fill_map.is_some() && *repeat_v;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let span_x = (i64::from(x1) - i64::from(x0)) as f32;
    let span_y = (i64::from(y1) - i64::from(y0)) as f32;
    let (doc_x, doc_y) = if rotation.is_finite() && rotation.rem_euclid(360.0) != 0.0 {
        let angle = rotation.rem_euclid(360.0).to_radians();
        let (sin, cos) = angle.sin_cos();
        let center_x = (x0 as f32 + x1 as f32) * 0.5;
        let center_y = (y0 as f32 + y1 as f32) * 0.5;
        let dx = x as f32 + 0.5 - center_x;
        let dy = y as f32 + 0.5 - center_y;
        (cos * dx + sin * dy + center_x, -sin * dx + cos * dy + center_y)
    } else {
        (x as f32 + 0.5, y as f32 + 0.5)
    };
    let u = (doc_x - x0 as f32) / span_x;
    let v = (doc_y - y0 as f32) / span_y;
    if (!repeat_u && !(0.0..1.0).contains(&u)) || (!repeat_v && !(0.0..1.0).contains(&v)) {
        return None;
    }
    let Some(map) = &layer.fill_map else { return Some(value) };
    let [width, height] = layer.fill_map_size;
    if width == 0 || height == 0 {
        return Some(value);
    }
    let sample_axis = |coordinate: f32, map_size: u32, repeat: bool, mirror: bool| {
        let tile = coordinate.floor() as i64;
        let mut fraction = coordinate - tile as f32;
        if repeat && mirror && tile.rem_euclid(2) == 1 {
            fraction = 1.0 - fraction;
        }
        (fraction * map_size as f32).floor().clamp(0.0, (map_size - 1) as f32) as i32
    };
    let sx = sample_axis(u, width, repeat_u, *mirror_u);
    let sy = sample_axis(v, height, repeat_v, *mirror_v);
    Some(map.sample_channel(sx, sy, 0))
}

/// Blend the red channel of a straight RGBA Paint tile directly into its grayscale destination.
/// Other layouts keep the canonical region-decoding fallback.
fn composite_rgba_tile_into_gray(dst: &mut [f32], surface: &photocraft_raster::Surface, coord: photocraft_geom::TileCoord, opacity: f32) -> bool {
    use photocraft_color::{ColorMode, SampleType};
    let format = surface.format();
    if format.mode != ColorMode::Rgb || !format.alpha || format.channels() != 4 || dst.len() != (photocraft_geom::TILE_SIZE as usize).pow(2) {
        return false;
    }
    let Some(tile) = surface.tile(coord) else {
        let default = surface.default_pixel();
        let Some((&red, &alpha)) = default.first().zip(default.get(3)) else { return false };
        let a = alpha * opacity;
        for value in dst {
            *value = red * a + *value * (1.0 - a);
        }
        return true;
    };
    let bytes = tile.bytes();
    match format.sample {
        SampleType::U8 => {
            for (out, px) in dst.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
                let a = f32::from(px[3]) / 255.0 * opacity;
                *out = f32::from(px[0]) / 255.0 * a + *out * (1.0 - a);
            }
        }
        SampleType::U16 => {
            for (out, px) in dst.iter_mut().zip(bytes.as_chunks::<8>().0.iter()) {
                let value = f32::from(u16::from_ne_bytes([px[0], px[1]])) / 65535.0;
                let alpha = f32::from(u16::from_ne_bytes([px[6], px[7]])) / 65535.0 * opacity;
                *out = value * alpha + *out * (1.0 - alpha);
            }
        }
        SampleType::F32 => {
            for (out, px) in dst.iter_mut().zip(bytes.as_chunks::<16>().0.iter()) {
                let value = f32::from_ne_bytes([px[0], px[1], px[2], px[3]]);
                let alpha = f32::from_ne_bytes([px[12], px[13], px[14], px[15]]) * opacity;
                *out = value * alpha + *out * (1.0 - alpha);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::mask_paint::Paint;
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;

    #[test]
    fn transformed_fill_cache_is_bounded_to_canvas() {
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut fill = layer(EffectKind::Fill { value: 1.0, rotation: 30.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 1.0);
        fill.fill_bounds = Some([-1_000_000, -1_000_000, 1_000_000, 1_000_000]);
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack { base, layers: vec![fill], active: 0 });
        let canvas = Rect::new(0, 0, 32, 32);
        refresh_changes_in_canvas(&mut mask, None, canvas);
        assert!(mask.surface.sample_channel(16, 16, 0) > 0.99);
        assert!(mask.surface.tiles().count() <= 1, "procedural bounds must not allocate outside the canvas");
        let old = mask.paint.clone().unwrap();
        mask.paint.as_mut().unwrap().layers[0].fill_bounds = Some([500_000, 500_000, 1_000_000, 1_000_000]);
        refresh_changes_in_canvas(&mut mask, Some(&old), canvas);
        assert!(mask.surface.sample_channel(16, 16, 0) < 0.01, "moving Fill away clears old coverage");
        assert!(mask.surface.tiles().count() <= 1);
    }

    #[test]
    fn unbounded_refresh_skips_pathological_persisted_fill_bounds() {
        let mut mask = LayerMask::hide_all();
        let mut fill = layer(EffectKind::Fill { value: 1.0, rotation: 45.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false }, 1.0);
        fill.fill_bounds = Some([i32::MIN, i32::MIN, i32::MAX, i32::MAX]);
        mask.paint = Some(Stack { base: Surface::with_default(PixelFormat::GRAY8, &[0.0]), layers: vec![fill], active: 0 });
        refresh(&mut mask, None);
        assert!(mask.surface.tiles().next().is_none(), "global fallback must not enumerate an unbounded Fill rectangle");

        refresh_in_canvas(&mut mask, Rect::new(0, 0, 2, 2));
        assert!(mask.surface.tiles().count() <= 1, "canvas refresh stays clipped even for hostile bounds");
    }

    #[test]
    fn unbounded_fill_keeps_legacy_repeat_and_constant_sampling() {
        let mut fill_layer = layer(EffectKind::Fill { value: 0.37, rotation: 0.0, repeat_u: true, repeat_v: false, mirror_u: true, mirror_v: false }, 1.0);
        let mut map = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        map.write_region(Rect::new(0, 0, 2, 2), &[0.1, 0.2, 0.7, 0.9]);
        fill_layer.fill_map = Some(map);
        fill_layer.fill_map_size = [2, 2];
        assert!((fill_sample(&fill_layer, 0.37, -1, -1).unwrap_or_default() - 0.1).abs() < 0.01);
        assert!((fill_sample(&fill_layer, 0.37, -2, -1).unwrap_or_default() - 0.2).abs() < 0.01);
        assert!((fill_sample(&fill_layer, 0.37, -2, i32::MAX).unwrap_or_default() - 0.9).abs() < 0.01);

        let constant = layer(EffectKind::Fill { value: 0.37, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false }, 1.0);
        assert_eq!(fill_sample(&constant, 0.37, -12, -34), Some(0.37));
    }

    fn layer(kind: EffectKind, opacity: f32) -> Paint {
        Paint {
            name: "effect".into(),
            visible: true,
            opacity,
            blend: BlendMode::Normal,
            surface: Surface::new(PixelFormat::RGBA8),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind,
        }
    }

    #[test]
    fn bounded_point_sampling_matches_refreshed_mask_stack() {
        let mut base = Surface::with_default(PixelFormat::GRAY8, &[0.2]);
        base.write_region(Rect::new(3, 4, 4, 5), &[0.8]);
        let mut paint_surface = Surface::with_default(PixelFormat::RGBA8, &[0.6, 0.0, 0.0, 0.5]);
        paint_surface.write_region(Rect::new(8, 9, 9, 10), &[0.9, 0.0, 0.0, 0.75]);
        let mut fill = layer(EffectKind::Fill { value: 0.3, rotation: 27.0, repeat_u: true, repeat_v: false, mirror_u: true, mirror_v: false }, 0.7);
        fill.fill_bounds = Some([4, 3, 20, 19]);
        fill.fill_map = Some(Surface::with_default(PixelFormat::GRAY8, &[0.4]));
        fill.fill_map_size = [4, 4];
        let levels = layer(EffectKind::Levels { input_black: 0.05, gamma: 1.7, input_white: 0.9, output_black: 0.1, output_white: 0.95 }, 0.6);
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack {
            base,
            layers: vec![
                Paint {
                    name: "paint".into(),
                    visible: true,
                    opacity: 0.8,
                    blend: BlendMode::Normal,
                    surface: paint_surface,
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: EffectKind::Paint,
                },
                fill,
                levels,
            ],
            active: 2,
        });

        refresh(&mut mask, Some(Rect::new(0, 0, 32, 32)));
        let stack = mask.paint.as_ref().unwrap();
        for (x, y) in [(0, 0), (3, 4), (8, 9), (12, 8), (19, 18), (26, 25)] {
            let sampled = sample_stack_pixel(stack, x, y);
            let cached = mask.surface.sample_channel(x, y, 0);
            assert!((sampled - cached).abs() <= 0.01, "point ({x},{y}): sampled {sampled}, refreshed {cached}");
        }
    }

    #[test]
    fn many_tile_opacity_refresh_preserves_untouched_cow_tiles() {
        let gray = PixelFormat::GRAY8;
        let rgba = PixelFormat::RGBA8;
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::with_default(gray, &[0.0]);
        let base = Surface::with_default(gray, &[0.2]);
        let mut surface = Surface::with_default(rgba, &[0.7, 0.1, 0.05, 0.0]);
        for x in 0..40 {
            let x = x * photocraft_geom::TILE_SIZE;
            surface.write_region(Rect::new(x, 0, x + 1, 1), &[0.9, 0.2, 0.1, 0.75]);
        }
        mask.paint = Some(Stack {
            base: base.clone(),
            layers: vec![Paint {
                name: "many tiles".into(),
                visible: true,
                opacity: 1.0,
                blend: BlendMode::Normal,
                surface: surface.clone(),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            }],
            active: 0,
        });
        refresh(&mut mask, None);
        let distant = photocraft_geom::TileCoord::new(50, 0);
        mask.surface.write_region(Rect::new(50 * photocraft_geom::TILE_SIZE, 0, 50 * photocraft_geom::TILE_SIZE + 1, 1), &[0.4]);
        let untouched = mask.surface.tile(distant).cloned();
        let old = mask.paint.as_ref().cloned();
        if let Some(stack) = &mut mask.paint {
            stack.layers[0].opacity = 0.25;
        }
        refresh_changes(&mut mask, old.as_ref());
        assert_eq!(mask.surface.default_pixel(), Surface::with_default(gray, &[0.2]).default_pixel());
        let tile = photocraft_geom::TileCoord::new(0, 0);
        let expected = surface.read_region(Rect::new(0, 0, 1, 1));
        let base_value = base.read_region(Rect::new(0, 0, 1, 1))[0];
        assert_eq!(
            mask.surface.read_region(Rect::new(0, 0, 1, 1)),
            Surface::with_default(gray, &[expected[0] * expected[3] * 0.25 + base_value * (1.0 - expected[3] * 0.25)]).default_pixel()
        );
        assert!(std::sync::Arc::ptr_eq(untouched.as_ref().expect("test tile"), mask.surface.tile(distant).expect("preserved tile")));
        assert!(mask.surface.tile(tile).is_some());
    }

    #[test]
    fn rgba_tile_kernel_matches_region_fallback_for_each_sample_type() {
        use photocraft_geom::TileCoord;
        for sample in [photocraft_color::SampleType::U8, photocraft_color::SampleType::U16, photocraft_color::SampleType::F32] {
            let gray = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, sample, false);
            let rgba = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true);
            let mut base = Surface::with_default(gray, &[0.23]);
            let mut surface = Surface::with_default(rgba, &[0.72, 0.13, 0.04, 0.0]);
            let rect = Rect::new(7, 9, 19, 17);
            let mut src = Vec::new();
            let mut dst = Vec::new();
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let v = ((x * 13 + y * 7) % 31) as f32 / 31.0;
                    src.extend_from_slice(&[0.1 + v * 0.8, 0.05 + v * 0.4, 0.02 + v * 0.2, 0.15 + v * 0.75]);
                    dst.push(0.1 + v * 0.7);
                }
            }
            surface.write_region(rect, &src);
            base.write_region(rect, &dst);
            let mut optimized = LayerMask::hide_all();
            optimized.surface = Surface::with_default(gray, &[0.0]);
            optimized.paint = Some(Stack {
                base,
                layers: vec![Paint {
                    name: "source".into(),
                    visible: true,
                    opacity: 0.73,
                    blend: BlendMode::Normal,
                    surface,
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: EffectKind::Paint,
                }],
                active: 0,
            });
            let mut oracle = optimized.clone();
            refresh(&mut optimized, None);
            refresh(&mut oracle, Some(TileCoord::new(0, 0).rect()));
            assert_eq!(
                optimized.surface.tile(TileCoord::new(0, 0)).map(|t| t.bytes()),
                oracle.surface.tile(TileCoord::new(0, 0)).map(|t| t.bytes()),
                "sample {sample:?}"
            );
        }
    }

    #[test]
    fn rgba_tile_kernel_matches_defaults_and_negative_coordinates() {
        use photocraft_geom::TileCoord;
        let gray = PixelFormat::GRAY8;
        let rgba = PixelFormat::RGBA8;
        let mut base = Surface::with_default(gray, &[0.3]);
        let mut surface = Surface::with_default(rgba, &[0.8, 0.2, 0.1, 0.4]);
        let painted = Rect::new(-255, 2, -254, 3);
        base.write_region(painted, &[0.75]);
        surface.write_region(painted, &[0.1, 0.7, 0.2, 0.9]);
        base.write_region(Rect::new(256, 0, 257, 1), &[0.6]);
        let mut optimized = LayerMask::hide_all();
        optimized.surface = Surface::with_default(gray, &[0.0]);
        optimized.paint = Some(Stack {
            base,
            layers: vec![Paint {
                name: "negative source".into(),
                visible: true,
                opacity: 0.65,
                blend: BlendMode::Normal,
                surface,
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            }],
            active: 0,
        });
        let mut oracle = optimized.clone();
        refresh(&mut optimized, None);
        refresh(&mut oracle, Some(TileCoord::new(-1, 0).rect()));
        refresh(&mut oracle, Some(TileCoord::new(1, 0).rect()));
        assert_eq!(optimized.surface.default_pixel(), oracle.surface.default_pixel());
        for coord in [TileCoord::new(-1, 0), TileCoord::new(1, 0)] {
            assert_eq!(optimized.surface.tile(coord).map(|tile| tile.bytes()), oracle.surface.tile(coord).map(|tile| tile.bytes()), "tile {coord:?}");
        }
    }

    #[test]
    fn changed_nonzero_paint_default_recomposes_sparse_tiles() {
        let gray = PixelFormat::GRAY8;
        let rgba = PixelFormat::RGBA8;
        let base = Surface::with_default(gray, &[0.2]);
        let mut surface = Surface::with_default(rgba, &[0.8, 0.2, 0.1, 0.4]);
        surface.write_region(Rect::new(1, 1, 2, 2), &[0.1, 0.2, 0.3, 0.9]);
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::with_default(gray, &[0.0]);
        mask.paint = Some(Stack {
            base,
            layers: vec![Paint {
                name: "default".into(),
                visible: true,
                opacity: 0.5,
                blend: BlendMode::Normal,
                surface: surface.clone(),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            }],
            active: 0,
        });
        refresh(&mut mask, None);
        let old = mask.paint.as_ref().cloned();
        if let Some(stack) = &mut mask.paint {
            stack.layers[0].surface.set_default_pixel(&[0.3, 0.2, 0.1, 0.6]);
        }
        refresh_changes(&mut mask, old.as_ref());
        assert!((mask.surface.default_pixel()[0] - (0.3 * 0.6 * 0.5 + 0.2 * (1.0 - 0.6 * 0.5))).abs() < 0.02);
    }

    #[test]
    fn fill_and_levels_compose_in_order_and_respect_opacity() {
        let mut mask = LayerMask::hide_all();
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.25]);
        mask.surface = base.clone();
        mask.paint = Some(Stack {
            base,
            layers: vec![
                layer(EffectKind::Fill { value: 0.75, rotation: 0.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 0.5),
                layer(EffectKind::Levels { input_black: 0.0, gamma: 1.0, input_white: 1.0, output_black: 0.0, output_white: 1.0 }, 0.5),
            ],
            active: 0,
        });
        refresh(&mut mask, None);
        // Fill: .25 -> .5; Levels identity at 50% opacity remains .5.
        assert!((mask.surface.read_region(Rect::new(0, 0, 1, 1))[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn fill_image_repeats_and_mirrors_inside_its_bounds() {
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut map = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        map.write_region(Rect::new(0, 0, 2, 2), &[0.0, 0.25, 0.5, 0.75]);
        let mut fill = layer(EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 1.0);
        fill.fill_map = Some(map);
        fill.fill_map_size = [2, 2];
        fill.fill_bounds = Some([0, 0, 4, 4]);
        let mut mask = LayerMask::hide_all();
        mask.surface = base.clone();
        mask.paint = Some(Stack { base, layers: vec![fill], active: 0 });

        refresh(&mut mask, None);
        let bounds = Rect::new(0, 0, 4, 4);
        let expected = [0.0, 0.0, 0.25, 0.25, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 0.5, 0.5, 0.75, 0.75];
        assert!(mask.surface.read_region(bounds).iter().zip(expected).all(|(actual, expected)| (actual - expected).abs() < 0.01));

        let fill = mask.paint.as_mut().and_then(|stack| stack.layers.first_mut());
        if let Some(fill) = fill
            && let EffectKind::Fill { mirror_u, mirror_v, .. } = &mut fill.kind
        {
            *mirror_u = true;
            *mirror_v = true;
        }
        refresh(&mut mask, None);
        let mirrored = mask.surface.read_region(bounds);
        assert!(mirrored.iter().zip(expected).all(|(actual, expected)| (actual - expected).abs() < 0.01), "Mirror does not alter the base tile");

        let base_tile = mirrored;
        let fill = mask.paint.as_mut().and_then(|stack| stack.layers.first_mut());
        if let Some(fill) = fill
            && let EffectKind::Fill { repeat_u, repeat_v, .. } = &mut fill.kind
        {
            *repeat_u = false;
            *repeat_v = false;
        }
        refresh(&mut mask, None);
        assert!(
            mask.surface.read_region(bounds).iter().zip(base_tile).all(|(actual, expected)| (actual - expected).abs() < 0.01),
            "toggling Repeat preserves pixels in the original Fill tile"
        );
    }

    #[test]
    fn fill_repeat_and_mirror_extend_past_the_source_bounds() {
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut map = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        map.write_region(Rect::new(0, 0, 2, 2), &[0.0, 0.25, 0.5, 0.75]);
        let mut fill = layer(EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 1.0);
        fill.fill_map = Some(map);
        fill.fill_map_size = [2, 2];
        fill.fill_bounds = Some([2, 2, 4, 4]);
        let mut mask = LayerMask::hide_all();
        mask.surface = base.clone();
        mask.paint = Some(Stack { base, layers: vec![fill], active: 0 });

        refresh(&mut mask, Some(Rect::new(0, 0, 6, 6)));
        let bounds = Rect::new(0, 0, 6, 6);
        let expected = [0.0, 0.25, 0.0, 0.25, 0.0, 0.25, 0.5, 0.75, 0.5, 0.75, 0.5, 0.75];
        assert!(mask.surface.read_region(bounds).iter().take(12).zip(expected).all(|(actual, expected)| (actual - expected).abs() < 0.01));

        let fill = mask.paint.as_mut().and_then(|stack| stack.layers.first_mut());
        if let Some(fill) = fill
            && let EffectKind::Fill { mirror_u, mirror_v, .. } = &mut fill.kind
        {
            *mirror_u = true;
            *mirror_v = true;
        }
        refresh(&mut mask, Some(bounds));
        let expected_top = [0.75, 0.5, 0.5, 0.75, 0.75, 0.5];
        let actual_top = mask.surface.read_region(Rect::new(0, 0, 6, 1));
        assert!(actual_top.iter().zip(expected_top).all(|(actual, expected)| (actual - expected).abs() < 0.01), "{actual_top:?}");
        assert!((mask.surface.read_region(Rect::new(4, 4, 5, 5))[0] - 0.75).abs() < 0.01);
    }

    #[test]
    fn fill_repeats_in_rotated_local_coordinates() {
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut map = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        map.write_region(Rect::new(0, 0, 2, 2), &[0.0, 0.25, 0.5, 0.75]);
        let mut fill = layer(EffectKind::Fill { value: 1.0, rotation: 90.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 1.0);
        fill.fill_map = Some(map);
        fill.fill_map_size = [2, 2];
        fill.fill_bounds = Some([2, 2, 4, 6]);
        let mut mask = LayerMask::hide_all();
        mask.surface = base.clone();
        mask.paint = Some(Stack { base, layers: vec![fill], active: 0 });

        refresh(&mut mask, None);
        let region = Rect::new(1, 3, 5, 5);
        let expected = [0.5, 0.5, 0.0, 0.0, 0.75, 0.75, 0.25, 0.25];
        let actual = mask.surface.read_region(region);
        assert!(actual.iter().zip(expected).all(|(actual, expected)| (actual - expected).abs() < 0.01), "{actual:?}");
        // (1, 3) lies outside the unrotated bounds [2, 2, 4, 6], but inside the rotated fill.
        assert!((mask.surface.read_region(Rect::new(1, 3, 2, 4))[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn levels_adjustment_is_non_destructive_and_applies_at_stack_position() {
        let mut mask = LayerMask::hide_all();
        let gray16 = PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U16, false);
        let base = Surface::with_default(gray16, &[0.25]);
        mask.surface = base.clone();
        mask.paint = Some(Stack {
            base,
            layers: vec![layer(EffectKind::Levels { input_black: 0.0, gamma: 1.0, input_white: 0.5, output_black: 0.0, output_white: 1.0 }, 1.0)],
            active: 0,
        });
        refresh(&mut mask, None);
        assert!((mask.surface.read_region(Rect::new(0, 0, 1, 1))[0] - 0.5).abs() < 0.001);
        let stored_base = mask.paint.as_ref().map(|s| s.base.default_pixel()[0]);
        assert!((stored_base.unwrap_or_default() - 0.25).abs() < 0.00002);
    }

    #[test]
    fn blur_is_a_global_gaussian_stack_effect_with_halo_correct_incremental_damage() {
        let gray = PixelFormat::GRAY8;
        let rgba = PixelFormat::RGBA8;
        let mut surface = Surface::with_default(rgba, &[0.0, 0.0, 0.0, 0.0]);
        surface.write_region(Rect::new(255, 8, 256, 9), &[1.0, 1.0, 1.0, 1.0]);
        let base = Surface::with_default(gray, &[0.0]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = surface;
        let blur = layer(EffectKind::Blur { radius: 4 }, 1.0);
        let mut mask = LayerMask::hide_all();
        mask.surface = base.clone();
        mask.paint = Some(Stack { base, layers: vec![paint, blur], active: 0 });

        refresh_in_canvas(&mut mask, Rect::new(0, 0, 512, 16));
        assert!(mask.surface.sample_channel(255, 8, 0) > 0.08, "cache and point sample must preserve the blurred impulse");
        assert!(mask.surface.sample_channel(256, 8, 0) > 0.01, "blur crosses the 256px tile boundary");
        assert!(mask.surface.sample_channel(262, 8, 0) < 0.01, "finite Gaussian support leaves distant pixels unchanged");
        let stack = mask.paint.as_ref().unwrap();
        for x in 250..=262 {
            assert!((sample_stack_pixel(stack, x, 8) - mask.surface.sample_channel(x, 8, 0)).abs() < 0.01);
        }

        let old = mask.paint.clone();
        if let Some(stack) = &mut mask.paint {
            stack.layers[0].surface.write_region(Rect::new(255, 8, 256, 9), &[0.0, 0.0, 0.0, 0.0]);
        }
        refresh_changes_in_canvas(&mut mask, old.as_ref(), Rect::new(0, 0, 512, 16));
        assert!(mask.surface.sample_channel(256, 8, 0) < 0.01, "changing source Paint invalidates downstream Blur halo");
    }

    #[test]
    fn shell_aware_blur_blocks_off_shell_input_but_bleeds_outward_and_isolates_shells() {
        let canvas = Rect::new(0, 0, 9, 3);
        let mut ids = vec![0u32; 9 * 3];
        for y in 0..3usize {
            ids[y * 9 + 1..=y * 9 + 3].fill(1);
            ids[y * 9 + 5..=y * 9 + 7].fill(2);
        }
        let shell_ids = ShellIdMap::new(canvas, &ids).unwrap();

        let make_stack = |source_x: i32| {
            let mut source = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
            source.write_region(Rect::new(source_x, 1, source_x + 1, 2), &[1.0, 1.0, 1.0, 1.0]);
            let mut paint = layer(EffectKind::Paint, 1.0);
            paint.surface = source;
            Stack { base: Surface::with_default(PixelFormat::GRAY8, &[0.0]), layers: vec![paint, layer(EffectKind::Blur { radius: 2 }, 1.0)], active: 0 }
        };
        let at = |pixels: &[f32], x: usize, y: usize| pixels[y * 9 + x];

        let outside_source = compose_stack_region_with_shell_ids(&make_stack(0), 2, canvas, shell_ids).unwrap();
        assert_eq!(at(&outside_source, 1, 1), 0.0, "off-shell source pixels cannot bleed into shell 1");
        assert_eq!(at(&outside_source, 5, 1), 0.0, "off-shell source pixels cannot bleed into shell 2");

        let shell_source = compose_stack_region_with_shell_ids(&make_stack(3), 2, canvas, shell_ids).unwrap();
        assert!(at(&shell_source, 4, 1) > 0.0, "occupied shell values can blur outward to off-shell pixels");
        assert_eq!(at(&shell_source, 5, 1), 0.0, "shell 1 values cannot bleed into adjacent shell 2");
    }

    #[test]
    fn shell_aware_blur_matches_normal_gaussian_inside_one_unbounded_shell() {
        let mut source = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        source.write_region(Rect::new(2, 2, 3, 3), &[1.0, 1.0, 1.0, 1.0]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = source;
        let stack =
            Stack { base: Surface::with_default(PixelFormat::GRAY8, &[0.0]), layers: vec![paint, layer(EffectKind::Blur { radius: 1 }, 1.0)], active: 0 };
        let output = Rect::new(1, 1, 4, 4);
        let padded = Rect::new(0, 0, 5, 5);
        let ids = vec![1u32; 25];
        let shell_ids = ShellIdMap::new(padded, &ids).unwrap();
        let expected = compose_stack_region(&stack, 2, output).unwrap();
        let actual = compose_stack_region_with_shell_ids(&stack, 2, output, shell_ids).unwrap();
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!((actual - expected).abs() < 1e-6, "shell blur differs at output index {index}: {actual} vs {expected}");
        }
    }

    #[test]
    fn shell_aware_incremental_blur_parameter_change_matches_full_rebuild() {
        let canvas = Rect::new(0, 0, 16, 8);
        let mut ids = vec![0u32; 16 * 8];
        for y in 0..8usize {
            ids[y * 16..y * 16 + 8].fill(1);
            ids[y * 16 + 8..(y + 1) * 16].fill(2);
        }
        let shell_ids = ShellIdMap::new(canvas, &ids).unwrap();
        let mut source = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        source.write_region(Rect::new(7, 4, 8, 5), &[1.0, 1.0, 1.0, 1.0]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = source;
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut cached = LayerMask::hide_all();
        cached.surface = base.clone();
        cached.paint = Some(Stack { base, layers: vec![paint, layer(EffectKind::Blur { radius: 2 }, 1.0)], active: 0 });
        refresh_in_canvas_with_shell_ids(&mut cached, canvas, shell_ids);

        let old = cached.paint.clone();
        cached.paint.as_mut().unwrap().layers[1].kind = EffectKind::Blur { radius: 4 };
        refresh_changes_in_canvas_with_shell_ids(&mut cached, old.as_ref(), canvas, shell_ids);

        let mut full = cached.clone();
        refresh_in_canvas_with_shell_ids(&mut full, canvas, shell_ids);
        assert_eq!(cached.surface.read_region(canvas), full.surface.read_region(canvas), "property edits use the shell-aware full stack result");
        assert_eq!(cached.surface.sample_channel(8, 4, 0), 0.0, "changed radius still isolates the adjacent UV shell");
    }

    #[test]
    fn shell_aware_large_regions_split_into_bounded_blur_compositions() {
        let canvas = Rect::new(0, 0, 2100, 2000);
        let pixels = usize::try_from(canvas.width()).unwrap() * usize::try_from(canvas.height()).unwrap();
        let ids = vec![1u32; pixels];
        let shell_ids = ShellIdMap::new(canvas, &ids).unwrap();
        let stack = Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.0]),
            layers: vec![
                layer(EffectKind::Fill { value: 0.4, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false }, 1.0),
                layer(EffectKind::Blur { radius: 1 }, 1.0),
            ],
            active: 0,
        };
        let output = compose_stack_region_with_shell_ids(&stack, stack.layers.len(), canvas, shell_ids).unwrap();
        assert_eq!(output.len(), pixels);
        assert!(output.iter().all(|value| (*value - 0.4).abs() < 1e-6), "bounded subregions retain exact constant coverage");

        let mut mask = LayerMask::hide_all();
        mask.paint = Some(stack);
        assert!(refresh_in_canvas_with_shell_ids(&mut mask, canvas, shell_ids), "large refresh should succeed via bounded bands");
        for (x, y) in [(0, 0), (511, 31), (512, 32), (2099, 1999)] {
            assert!((mask.surface.sample_channel(x, y, 0) - 0.4).abs() < 0.01, "refresh missed ({x}, {y})");
        }
    }

    #[test]
    fn shell_aware_parallel_refresh_bands_match_full_composition_across_uv_seams() {
        let canvas = Rect::new(0, 0, 1024, 512);
        let width = canvas.width() as usize;
        let height = canvas.height() as usize;
        let mut ids = vec![0u32; width * height];
        for row in ids.chunks_exact_mut(width) {
            row[..width / 2 - 8].fill(1);
            row[width / 2 + 8..].fill(2);
        }
        let shell_ids = ShellIdMap::new(canvas, &ids).unwrap();
        let mut source = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        source.write_region(Rect::new(503, 255, 504, 256), &[1.0, 1.0, 1.0, 1.0]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = source;
        let stack = Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.0]),
            layers: vec![paint, layer(EffectKind::Blur { radius: 8 }, 1.0), layer(EffectKind::Blur { radius: 3 }, 0.75)],
            active: 0,
        };
        let expected = compose_stack_region_with_shell_ids(&stack, stack.layers.len(), canvas, shell_ids).unwrap();
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(stack);
        assert!(refresh_in_canvas_with_shell_ids(&mut mask, canvas, shell_ids));
        let actual = mask.surface.read_region(canvas);
        let mut expected_surface = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        expected_surface.write_region(canvas, &expected);
        assert_eq!(actual, expected_surface.read_region(canvas), "parallel bounded bands preserve quantized full-composition output");
        assert_eq!(mask.surface.sample_channel(520, 255, 0), 0.0, "shell 1's Blur never flows into shell 2");
        assert!(mask.surface.sample_channel(504, 255, 0) > 0.0, "shell values still bleed outward to off-shell pixels");
    }

    #[test]
    fn unsupported_shell_blur_refuses_without_mutating_cached_mask() {
        let canvas = Rect::new(0, 0, 1, 1);
        let shell_ids = ShellIdMap::new(canvas, &[1]).unwrap();
        let stack = Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.25]),
            layers: (0..21).map(|_| layer(EffectKind::Blur { radius: 64 }, 1.0)).collect(),
            active: 0,
        };
        let mut mask = LayerMask::hide_all();
        mask.surface.write_region(canvas, &[0.75]);
        mask.paint = Some(stack);
        let prior = mask.surface.clone();
        assert!(!refresh_in_canvas_with_shell_ids(&mut mask, canvas, shell_ids), "an unsplittable support halo must be reported");
        assert_eq!(mask.surface, prior, "failed shell recomposition must not write a default or partial result");
    }

    #[test]
    fn resampled_blur_uses_padded_support_across_preview_boundaries() {
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        let mut surface = Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]);
        surface.write_region(Rect::new(7, 8, 8, 9), &[1.0, 1.0, 1.0, 1.0]);
        paint.surface = surface;
        let stack = Stack { base, layers: vec![paint, layer(EffectKind::Blur { radius: 3 }, 1.0), layer(EffectKind::Blur { radius: 2 }, 1.0)], active: 0 };

        // Both requests use one source pixel per preview pixel. The smaller request
        // begins five pixels from the impulse; only the combined support of both
        // sequential Gaussian stages can carry it into the crop.
        let full = sample_stack_resampled(&stack, Rect::new(0, 0, 32, 16), [32, 16]).unwrap();
        let crop = sample_stack_resampled(&stack, Rect::new(12, 0, 20, 16), [8, 16]).unwrap();
        for y in 0..16 {
            for x in 0..8 {
                assert!((crop[y * 8 + x] - full[y * 32 + x + 12]).abs() < 1e-6, "preview seam at ({x}, {y})");
            }
        }
        assert!(crop[8 * 8] > 0.0, "padded stages preserve Blur energy entering through the preview boundary");
    }

    #[test]
    fn hidden_downstream_blur_does_not_disable_prefix_refresh() {
        let region = Rect::new(0, 0, 16, 16);
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.2]);
        let lower = layer(EffectKind::Fill { value: 0.4, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false }, 1.0);
        let mut active = layer(EffectKind::Paint, 1.0);
        active.surface = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        let mut blur = layer(EffectKind::Blur { radius: 8 }, 1.0);
        blur.visible = false;
        let mut mask = LayerMask::hide_all();
        mask.surface = base.clone();
        mask.paint = Some(Stack { base, layers: vec![lower, active, blur], active: 1 });
        let prefix = compose_prefix(mask.paint.as_ref().unwrap(), 1, region).unwrap();
        mask.paint.as_mut().unwrap().layers[1].surface.write_region(Rect::new(7, 7, 8, 8), &[1.0, 1.0, 1.0, 1.0]);

        assert!(refresh_from_prefix(&mut mask, region, 1, &prefix), "an invisible Blur is an identity suffix");
        let mut full = mask.clone();
        refresh_in_canvas(&mut full, region);
        assert_eq!(mask.surface.read_region(region), full.surface.read_region(region));
    }

    #[test]
    fn live_blur_damage_recomputes_neighbors_and_clips_at_canvas_edges() {
        let canvas = Rect::new(0, 0, 32, 24);
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.15]);
        let mut source = Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]);
        source.write_region(Rect::new(0, 9, 1, 10), &[1.0, 1.0, 1.0, 1.0]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = source;
        let blur = layer(EffectKind::Blur { radius: 4 }, 1.0);
        let mut cached = LayerMask::hide_all();
        cached.surface = base.clone();
        cached.paint = Some(Stack { base: base.clone(), layers: vec![paint, blur], active: 0 });
        refresh_in_canvas(&mut cached, canvas);
        assert!(cached.surface.sample_channel(1, 9, 0) > 0.2, "Blur spreads onto neighboring output pixels");

        let stack = cached.paint.as_mut().unwrap();
        stack.layers[0].surface.write_region(Rect::new(0, 9, 1, 10), &[0.0, 0.0, 0.0, 0.0]);
        refresh_damage_in_canvas(&mut cached, Rect::new(0, 9, 1, 10), canvas);

        let mut full = cached.clone();
        refresh_in_canvas(&mut full, canvas);
        assert_eq!(cached.surface.read_region(canvas), full.surface.read_region(canvas));
        assert!(cached.surface.sample_channel(1, 9, 0) < 0.16, "source damage refreshes the Blur halo during a live stroke");
        assert!(cached.surface.tiles().all(|(coord, _)| coord.tx >= 0 && coord.ty >= 0), "damage is clipped to the document canvas");
    }

    #[test]
    fn blur_damage_banded_refresh_matches_single_region_composition() {
        let canvas = Rect::new(0, 0, 1024, 768);
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.12]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        let blur = layer(EffectKind::Blur { radius: 12 }, 0.75);
        let mut cached = LayerMask::hide_all();
        cached.surface = base.clone();
        cached.paint = Some(Stack { base, layers: vec![paint, blur], active: 0 });
        refresh_in_canvas(&mut cached, canvas);

        let damage = Rect::new(0, 0, 768, 768);
        let bands = refresh_bands(damage, 512);
        assert!(bands.len() > 1, "regression must exercise multiple non-overlapping bands");
        cached.paint.as_mut().unwrap().layers[0].surface.write_region(Rect::new(254, 254, 258, 258), &vec![0.9; 4 * 4 * 4]);
        refresh_damage_in_canvas(&mut cached, damage, canvas);

        let stack = cached.paint.as_ref().unwrap();
        let full_pixels = compose_stack_region(stack, stack.layers.len(), canvas).unwrap();
        let mut full = Surface::with_default(cached.surface.format(), &cached.surface.default_pixel());
        full.write_region(canvas, &full_pixels);
        assert_eq!(cached.surface.read_region(canvas), full.read_region(canvas), "parallel band composition must match a single full-region Blur");
    }

    #[test]
    fn multi_tile_blur_damage_merges_only_complete_rectangles_and_matches_full_refresh() {
        use photocraft_geom::TileCoord;

        let canvas = Rect::new(0, 0, 768, 768);
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.12]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        let blur = layer(EffectKind::Blur { radius: 12 }, 0.8);
        let mut cached = LayerMask::hide_all();
        cached.surface = base.clone();
        cached.paint = Some(Stack { base, layers: vec![paint, blur], active: 0 });
        refresh_in_canvas(&mut cached, canvas);

        let old = cached.paint.clone();
        let damage = Rect::new(220, 220, 550, 550);
        let source = (0..damage.width() as usize * damage.height() as usize).flat_map(|_| [0.82, 0.1, 0.0, 1.0]).collect::<Vec<_>>();
        cached.paint.as_mut().unwrap().layers[0].surface.write_region(damage, &source);
        refresh_changes_in_canvas(&mut cached, old.as_ref(), canvas);

        let changed_tiles = (0..3).flat_map(|y| (0..3).map(move |x| TileCoord::new(x, y))).collect::<Vec<_>>();
        let merged = merge_adjacent_tiles(&changed_tiles);
        assert_eq!(merged.len(), 1, "nine adjacent output tiles need one Blur recomposition");
        assert_eq!(merged[0].0, canvas);
        let sparse = merge_adjacent_tiles(&[TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(3, 0)]);
        assert_eq!(sparse.len(), 2, "a gap must remain outside both recomposition regions");
        assert_eq!(sparse[0].0, Rect::new(0, 0, 512, 256));
        assert_eq!(sparse[1].0, Rect::new(768, 0, 1024, 256));

        let mut full = cached.clone();
        refresh_in_canvas(&mut full, canvas);
        assert_eq!(cached.surface.read_region(canvas), full.surface.read_region(canvas), "merged incremental Blur exactly matches a full rebuild");
    }

    #[test]
    fn blur_parameter_visibility_and_order_changes_match_a_full_refresh() {
        let canvas = Rect::new(0, 0, 512, 512);
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.1]);
        let mut source = Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]);
        source.write_region(Rect::new(248, 248, 264, 264), &vec![1.0; 16 * 16 * 4]);
        let mut paint = layer(EffectKind::Paint, 1.0);
        paint.surface = source;
        let blur = layer(EffectKind::Blur { radius: 4 }, 0.8);
        let fill = layer(EffectKind::Fill { value: 0.35, rotation: 0.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 0.6);
        let initial = Stack { base: base.clone(), layers: vec![paint, blur, fill], active: 1 };

        let assert_matches_full = |mut cached: LayerMask, edit: &dyn Fn(&mut Stack)| {
            let old = cached.paint.clone();
            if let Some(stack) = &mut cached.paint {
                edit(stack);
            }
            refresh_changes_in_canvas(&mut cached, old.as_ref(), canvas);
            let mut full = cached.clone();
            refresh_in_canvas(&mut full, canvas);
            assert_eq!(cached.surface.read_region(canvas), full.surface.read_region(canvas));
        };

        for scenario in 0..6 {
            let mut cached = LayerMask::hide_all();
            cached.surface = base.clone();
            cached.paint = Some(initial.clone());
            refresh_in_canvas(&mut cached, canvas);
            assert_matches_full(cached, &|stack| match scenario {
                0 => stack.layers[1].kind = EffectKind::Blur { radius: 9 },
                1 => stack.layers[1].opacity = 0.35,
                2 => stack.layers[1].visible = false,
                3 => stack.layers.swap(0, 1),
                4 => {
                    stack.layers.remove(1);
                }
                _ => stack.layers.insert(1, layer(EffectKind::Blur { radius: 7 }, 0.65)),
            });
        }
    }

    #[test]
    fn grayscale_blend_modes_use_layer_coverage_for_paint_and_fill() {
        let mut fill = layer(EffectKind::Fill { value: 0.5, rotation: 0.0, repeat_u: true, repeat_v: true, mirror_u: false, mirror_v: false }, 0.5);
        fill.blend = BlendMode::Multiply;
        let mut paint = layer(EffectKind::Paint, 0.5);
        paint.blend = BlendMode::Multiply;
        paint.surface.set_default_pixel(&[0.5, 0.5, 0.5, 1.0]);
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.5]);
        let stack = Stack { base, layers: vec![fill, paint], active: 1 };
        assert!((sample_stack_pixel(&stack, 0, 0) - 0.28125).abs() < 0.01);
    }

    #[test]
    fn compact_mask_blend_modes_have_scalar_coverage_semantics() {
        let below = 0.25;
        let source = 0.75;
        let coverage = 0.5;
        let cases = [
            (BlendMode::Normal, 0.5),
            (BlendMode::Darken, 0.25),
            (BlendMode::Multiply, 0.21875),
            (BlendMode::Lighten, 0.5),
            (BlendMode::Screen, 0.53125),
            (BlendMode::LinearDodge, 0.625),
            (BlendMode::Subtract, 0.125),
            (BlendMode::Difference, 0.375),
        ];
        assert_eq!(photocraft_doc::mask_paint::MASK_BLEND_MODES.len(), cases.len());
        for (mode, expected) in cases {
            assert!(photocraft_doc::mask_paint::is_mask_blend_mode(mode));
            assert!((blend_coverage(mode, below, source, coverage) - expected).abs() < 1e-6, "{mode:?}");
        }
        for rejected in [BlendMode::ColorBurn, BlendMode::Overlay, BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity] {
            assert!(!photocraft_doc::mask_paint::is_mask_blend_mode(rejected), "{rejected:?} is outside the mask blend set");
        }
    }
}
