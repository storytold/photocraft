use super::batch::tile_grid;
use super::consts::{FX_BUDGET, PAGE, STRIDE};
use super::textures::{Cell, TexKind};
use super::{DeviceHealth, Document, FieldKind, HashMap, Rect, Role, TILE_SIZE, Unsupported, fx, plan};

/// The side of the square pages a compositor stores layer surfaces in on a device whose largest
/// texture side is `max_dim`: the largest power-of-two multiple of the tile size within it and
/// [`PAGE`] ([`Compositor::page_size`](super::Compositor::page_size)).
pub fn page_side(max_dim: u32) -> u32 {
    let tile = TILE_SIZE as u32;
    let limit = max_dim.max(tile).min(PAGE.max(tile));
    let mut page = tile;
    while page.saturating_mul(2) <= limit {
        page *= 2;
    }
    page
}

/// Each distinct layer and mask surface `doc`'s plan samples: its tiles within the canvas tile
/// grid, and the bytes per pixel of its page texture.
fn plan_surfaces(doc: &Document) -> Vec<(Rect, u64)> {
    let Ok(plan) = plan(doc) else { return Vec::new() };
    let grid = tile_grid(doc.bounds());
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for pass in &plan.passes {
        let sources = [pass.tex.as_ref().map(|t| (t.layer, t.role, t.surface.get())), pass.mask.as_ref().map(|m| (m.layer, Role::Mask, m.surface.get()))];
        for (layer, role, s) in sources.into_iter().flatten() {
            if seen.insert((layer, role)) {
                out.push((s.tile_bounds().intersect(&grid), TexKind::for_surface(role, s.format()).bytes_per_pixel() as u64));
            }
        }
    }
    out
}

/// Bytes of the layer pages a render of `region` of `doc` needs resident with pages of side
/// `page` (each layer and mask surface's tiles within the region's page cells, in their texture
/// format): [`Compositor::working_set`](super::Compositor::working_set), priced before any
/// compositor exists, so a memory budget can be sized for a document's first refresh.
pub fn layer_working_set(doc: &Document, region: Rect, page: u32) -> u64 {
    let page = page.max(1);
    let p = i32::try_from(page).unwrap_or(i32::MAX);
    let cells = grid_rects(region.intersect(&doc.bounds()), page)
        .into_iter()
        .map(|r| cell_rect((r.x0.div_euclid(p), r.y0.div_euclid(p)), page))
        .fold(Rect::EMPTY, |a, c| if a.is_empty() { c } else { a.union(&c) });
    plan_surfaces(doc).into_iter().fold(0u64, |total, (area, bpp)| {
        let a = area.intersect(&cells);
        total.saturating_add((a.width() as u64).saturating_mul(a.height() as u64).saturating_mul(bpp))
    })
}

/// At most the bytes of layer pages one page cell of `doc` needs resident with pages of side
/// `page`: each layer and mask surface's tiles, up to a whole page each. A refresh draws cell by
/// cell, keeping the cell it draws resident, so a budget of a few cells keeps edits in the view
/// uploading only the tiles they change, even where the whole document's pages don't fit.
pub fn cell_working_set(doc: &Document, page: u32) -> u64 {
    let page_px = u64::from(page.max(1)).saturating_mul(u64::from(page.max(1)));
    plan_surfaces(doc).into_iter().fold(0u64, |total, (area, bpp)| {
        let px = (area.width() as u64).saturating_mul(area.height() as u64).min(page_px);
        total.saturating_add(px.saturating_mul(bpp))
    })
}

/// A memory budget ([`Compositor::set_memory_budget`](super::Compositor::set_memory_budget))
/// whose layer-page share holds `pages` bytes, with no more on top than the caches take: a
/// quarter of the budget, at most [`FX_BUDGET`].
pub fn budget_for_pages(pages: u64) -> u64 {
    let quarter = pages.div_ceil(3);
    pages.saturating_add(quarter.min(FX_BUDGET as u64))
}

/// Document pixels of page cell `c`.
pub(crate) fn cell_rect(c: Cell, page: u32) -> Rect {
    let p = page as i32;
    Rect::new(c.0.saturating_mul(p), c.1.saturating_mul(p), c.0.saturating_add(1).saturating_mul(p), c.1.saturating_add(1).saturating_mul(p))
}

/// `r` cut along the grid of `step`-sized squares (row-major).
pub(crate) fn grid_rects(r: Rect, step: u32) -> Vec<Rect> {
    let s = step.max(1) as i32;
    let mut out = Vec::new();
    let mut y = r.y0.div_euclid(s) * s;
    while y < r.y1 {
        let mut x = r.x0.div_euclid(s) * s;
        while x < r.x1 {
            out.push(Rect::new(x, y, x.saturating_add(s), y.saturating_add(s)).intersect(&r));
            x = x.saturating_add(s);
        }
        y = y.saturating_add(s);
    }
    out
}

/// Number of rectangles `grid_rects` would produce, without allocating them during preflight.
pub(crate) fn grid_rect_count(r: Rect, step: u32) -> Result<usize, Unsupported> {
    if r.is_empty() {
        return Ok(0);
    }
    let s = i64::from(step.max(1));
    let axis = |lo: i32, hi: i32| usize::try_from((i64::from(hi) - 1).div_euclid(s) - i64::from(lo).div_euclid(s) + 1).ok();
    axis(r.x0, r.x1).and_then(|x| axis(r.y0, r.y1).and_then(|y| x.checked_mul(y))).ok_or_else(|| Unsupported("GPU compositor chunk count is too large".into()))
}

/// Host length and GPU capacity for the chunk/pass uniform records, including their u32
/// dynamic binding offsets. Decline unsupported stacks before an invalid buffer faults the device.
pub(crate) fn uniform_buffer_len(chunks: usize, passes: usize, limit: u64) -> Result<usize, Unsupported> {
    let too_large = || Unsupported("GPU compositor uniform buffer is too large".into());
    let records = chunks.checked_add(passes).and_then(|n| u64::try_from(n).ok()).ok_or_else(too_large)?;
    let bytes = records.checked_mul(STRIDE).ok_or_else(too_large)?;
    if bytes > limit {
        return Err(Unsupported(format!("GPU compositor uniform buffer requires {bytes} bytes; device limit is {limit}")));
    }
    if bytes.saturating_sub(STRIDE) > u64::from(u32::MAX) {
        return Err(Unsupported("GPU compositor uniform offsets exceed u32".into()));
    }
    usize::try_from(bytes).map_err(|_| too_large())
}

/// The reach up to which each distance field must be exact (the largest any program asks for).
pub(crate) fn field_reaches(progs: &[fx::MapProgram]) -> HashMap<FieldKind, i32> {
    let mut want: HashMap<FieldKind, i32> = HashMap::new();
    for (k, r) in progs.iter().flat_map(|p| p.fields.iter()) {
        let w = want.entry(*k).or_insert(0);
        *w = (*w).max(*r);
    }
    want
}

/// Why a refresh whose layer pages don't fit GPU memory is drawn on the CPU.
pub const OVER_BUDGET: &str = "layers exceed the GPU memory budget; full refresh on the CPU";

/// Which of `pages` (LRU clock, key, bytes) to drop, least recently used first, so `incoming`
/// more bytes fit `budget` on top of the `total` held now. Callers leave out pages that must stay.
pub(crate) fn lru_victims<K>(mut pages: Vec<(u64, K, u64)>, total: u64, incoming: u64, budget: u64) -> Vec<K> {
    pages.sort_unstable_by_key(|p| p.0);
    let mut total = total;
    let mut out = Vec::new();
    for (_, k, bytes) in pages {
        if total.saturating_add(incoming) <= budget {
            break;
        }
        total = total.saturating_sub(bytes);
        out.push(k);
    }
    out
}

/// Runs `f` inside GPU error scopes and returns the first error raised.
/// Native wgpu reports validation/internal/OOM errors synchronously; where they only arrive
/// asynchronously (WebGPU) none is reported.
pub(crate) fn first_error<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> (T, Option<wgpu::Error>) {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let out = f();
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut error = None;
    // Scopes pop in reverse order.
    for pop in [memory.pop(), internal.pop(), validation.pop()] {
        if let std::task::Poll::Ready(Some(e)) = std::pin::pin!(pop).poll(&mut cx) {
            error.get_or_insert(e);
        }
    }
    (out, error)
}

/// Runs `f` inside the error scopes of [`first_error`], turning a reported error into
/// [`Unsupported`] instead of letting it escape to wgpu's panic handler.
///
/// Every native caller needs this, watched or not: a validation error (writing into a destroyed
/// or too-small buffer, say) is raised synchronously and never reaches the health callbacks, so a
/// watched compositor still panicked on it. WebGPU reports its errors asynchronously, long after
/// the scope has popped, so wasm relies on the health callbacks alone.
///
/// A scope also swallows the error that [`DeviceHealth`]'s uncaptured-error handler would have
/// recorded, so a reported error is handed back to `health`: a watched device must stop being
/// used and let the caller fall back to the CPU compositor.
pub(crate) fn scoped_work<T>(device: &wgpu::Device, health: Option<DeviceHealth>, f: impl FnOnce() -> Result<T, Unsupported>) -> Result<T, Unsupported> {
    // WebGPU errors arrive asynchronously; its callers still use the health callbacks.
    if cfg!(target_arch = "wasm32") {
        return f();
    }
    let (result, error) = first_error(device, f);
    match error {
        Some(error) => {
            if let Some(health) = health.as_ref() {
                health.caught(device, &error);
            }
            Err(Unsupported(format!("GPU work failed: {error}")))
        }
        None => result,
    }
}
