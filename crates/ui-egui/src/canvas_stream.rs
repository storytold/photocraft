//! Viewport residency with bounded background work. A cached page never represents the
//! document's storage: zoom, device loss and eviction cannot change original pixels.
use crate::gpu_canvas::GpuCanvas;
use egui::{Color32, TextureHandle, TextureOptions};
use photocraft_doc::Document;
use photocraft_engine::display_color::CanvasDisplay;
use photocraft_geom::Rect;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

pub const PAGE_SIDE: u32 = 512;
const MAX_PAGES: usize = 512;
const MAX_JOBS: usize = 2;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
struct WorkerSlot;
impl Drop for WorkerSlot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageKey {
    pub doc: u64,
    pub snapshot: u64,
    pub revision: u64,
    pub preview: u64,
    pub display: u64,
    pub factor: u32,
    pub x: u32,
    pub y: u32,
}

impl PageKey {
    pub fn id(self) -> u64 {
        egui::Id::new(self).value()
    }
    pub fn core(self, doc: &Document) -> Rect {
        let side = PAGE_SIDE.saturating_mul(self.factor);
        let Some(x) = self.x.checked_mul(side).and_then(|x| i32::try_from(x).ok()) else { return Rect::EMPTY };
        let Some(y) = self.y.checked_mul(side).and_then(|y| i32::try_from(y).ok()) else { return Rect::EMPTY };
        if self.factor == 0 || self.factor > 1 << 16 {
            return Rect::EMPTY;
        }
        Rect::from_xywh(x, y, side, side).intersect(&doc.bounds())
    }
    pub fn source(self, doc: &Document) -> Rect {
        self.core(doc).inflate(self.factor as i32).intersect(&doc.bounds())
    }
}

pub fn factor(zoom: f32) -> u32 {
    if !zoom.is_finite() || zoom <= 0.0 {
        return 1;
    }
    let mut k = 1u32;
    while k < 1 << 16 && zoom * (k * 2) as f32 <= 1.0 {
        k *= 2;
    }
    k
}

pub fn pages(doc: &Document, visible: Rect, zoom: f32, revision: u64, preview: u64, display: u64) -> Vec<PageKey> {
    let visible = visible.intersect(&doc.bounds());
    if visible.is_empty() {
        return Vec::new();
    }
    let mut k = factor(zoom);
    loop {
        let side = PAGE_SIDE.saturating_mul(k);
        let (x0, x1, y0, y1) = (visible.x0 as u32 / side, (visible.x1 as u32).div_ceil(side), visible.y0 as u32 / side, (visible.y1 as u32).div_ceil(side));
        if u64::from(x1 - x0) * u64::from(y1 - y0) <= MAX_PAGES as u64 {
            let mut out = Vec::new();
            for y in y0..y1 {
                for x in x0..x1 {
                    out.push(PageKey { doc: doc.id.0, snapshot: doc as *const Document as usize as u64, revision, preview, display, factor: k, x, y });
                }
            }
            let (cx, cy) = (i64::from(x0 + x1), i64::from(y0 + y1));
            out.sort_by_key(|p| (i64::from(p.x) * 2 - cx).abs() + (i64::from(p.y) * 2 - cy).abs());
            return out;
        }
        if k >= 1 << 16 {
            return Vec::new();
        }
        k *= 2;
    }
}

struct Page {
    buffer: Option<photocraft_compose::Buffer>,
    rect: Rect,
    image: egui::ColorImage,
    texture: Option<TextureHandle>,
    gpu: bool,
    exact: bool,
    touched: u64,
}
impl Page {
    fn bytes(&self) -> u64 {
        self.image.pixels.len() as u64 * 4 + self.buffer.as_ref().map_or(0, |b| b.px.len() as u64 * 16)
    }
}
struct Completed {
    key: PageKey,
    exact: bool,
    result: Result<(photocraft_compose::Buffer, egui::ColorImage), String>,
}

#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub resident_pages: usize,
    pub resident_bytes: u64,
    pub active_jobs: usize,
    pub uploads: u64,
    pub cancellations: u64,
    pub gpu_composites: u64,
    pub last_error: Option<String>,
}

pub struct Stream {
    cache: HashMap<PageKey, Page>,
    jobs: HashMap<PageKey, Arc<AtomicBool>>,
    tx: mpsc::SyncSender<Completed>,
    rx: mpsc::Receiver<Completed>,
    clock: u64,
    failed: HashSet<PageKey>,
    health_generation: u64,
    memory_generation: u64,
    pub stats: Stats,
}
impl Default for Stream {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(MAX_JOBS);
        Self {
            cache: HashMap::new(),
            jobs: HashMap::new(),
            tx,
            rx,
            clock: 0,
            failed: HashSet::new(),
            health_generation: 0,
            memory_generation: 0,
            stats: Stats::default(),
        }
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        for c in self.jobs.values() {
            c.store(true, Ordering::Relaxed);
        }
    }
}

fn compute(
    doc: &Document,
    key: PageKey,
    display: Option<&CanvasDisplay>,
    exact: bool,
    cancel: &AtomicBool,
) -> Result<(photocraft_compose::Buffer, egui::ColorImage), String> {
    let source = key.source(doc);
    let working = u64::from(source.width()) * u64::from(source.height().min(128)) * 16 + (16 << 20);
    let _working = photocraft_raster::memory::try_reserve(working)?;
    let before = photocraft_raster::spill::read_error_generation();
    let check = || cancel.load(Ordering::Relaxed);
    let i = photocraft_raster::Interrupt::cancel_only(&check);
    let buf = if exact {
        photocraft_compose::viewport::render_page(doc, key.source(doc), key.factor, i)?
    } else {
        photocraft_compose::viewport::preview_page(doc, key.source(doc), key.factor, i)?
    };
    i.check().map_err(|e| e.to_string())?;
    photocraft_raster::spill::check_since(before)?;
    let img = display.map_or_else(|| buf.to_rgba8(), |d| d.to_rgba8(&buf));
    let image = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    let encoded = display.map_or_else(|| buf.clone(), |d| d.texture_buffer(&buf).into_owned());
    Ok((encoded, image))
}

impl Stream {
    /// Carry unaffected derived pages across a single local edit. Jobs still in flight retain
    /// their old identity and are cancelled by update; no stale worker result is rebased.
    pub fn rebase_damage(&mut self, doc: &Document, target: PageKey, damage: Rect, gpu: Option<&GpuCanvas>) {
        if target.preview != 0 {
            return;
        }
        let keys: Vec<_> = self
            .cache
            .keys()
            .copied()
            .filter(|k| {
                k.doc == target.doc
                    && k.revision.checked_add(1) == Some(target.revision)
                    && k.preview == 0
                    && k.display == target.display
                    && k.source(doc).intersect(&damage).is_empty()
            })
            .collect();
        for key in keys {
            let new = PageKey { snapshot: target.snapshot, revision: target.revision, ..key };
            if let Some(mut p) = self.cache.remove(&key) {
                if p.gpu {
                    p.gpu = gpu.is_some_and(|g| g.rename_page(key.doc, key.id(), new.id()));
                }
                self.cache.insert(new, p);
            }
        }
    }
    pub fn gpu_keys(&self) -> Vec<u64> {
        self.cache.keys().map(|k| k.id()).collect()
    }
    pub fn texture(&self, key: &PageKey) -> Option<egui::TextureId> {
        self.cache.get(key)?.texture.as_ref().map(TextureHandle::id)
    }
    pub fn reset_gpu(&mut self) {
        for p in self.cache.values_mut() {
            p.gpu = false;
        }
    }
    pub fn update(&mut self, ctx: &egui::Context, doc: Arc<Document>, wanted: &[PageKey], display: Option<Arc<CanvasDisplay>>, options: StreamOptions<'_>) {
        let StreamOptions { limit, gpu, refine } = options;
        self.clock = self.clock.wrapping_add(1);
        let generation = photocraft_raster::spill::read_error_generation();
        let memory_generation = photocraft_raster::memory::configuration_generation();
        if generation != self.health_generation || memory_generation != self.memory_generation {
            self.failed.clear();
            self.health_generation = generation;
            self.memory_generation = memory_generation;
        }
        let desired: HashSet<PageKey> = wanted.iter().copied().collect();
        // Keep cancelled workers in the job count until they exit, so rapid pans cannot spawn
        // unbounded threads or retain unbounded document snapshots.
        for (key, cancel) in &self.jobs {
            if !desired.contains(key) && !cancel.swap(true, Ordering::Relaxed) {
                self.stats.cancellations += 1;
            }
        }
        while let Ok(c) = self.rx.try_recv() {
            let cancelled = self.jobs.remove(&c.key).is_some_and(|c| c.load(Ordering::Relaxed));
            if cancelled || !desired.contains(&c.key) {
                continue;
            }
            match c.result {
                Ok((buffer, image)) => {
                    let rect = buffer.rect;
                    self.cache.insert(c.key, Page { buffer: Some(buffer), rect, image, texture: None, gpu: false, exact: c.exact, touched: self.clock });
                    self.stats.last_error = None;
                }
                Err(e) => {
                    self.failed.insert(c.key);
                    self.stats.last_error = Some(e);
                }
            }
        }
        if let Some(g) = gpu {
            g.touch_pages(doc.id.0, &wanted.iter().map(|k| k.id()).collect::<Vec<_>>());
            self.cache.retain(|k, p| !(desired.contains(k) && p.gpu && !g.has_page(k.doc, k.id())));
        }
        // Revision changes invalidate derived pages; master tiles and History remain untouched.
        self.cache.retain(|k, _| {
            k.doc != doc.id.0
                || wanted.first().is_some_and(|w| (k.snapshot, k.revision, k.preview, k.display) == (w.snapshot, w.revision, w.preview, w.display))
        });
        for key in wanted {
            if let Some(p) = self.cache.get_mut(key) {
                p.touched = self.clock;
            }
        }
        let mut bytes: u64 = self.cache.values().map(Page::bytes).sum();
        while bytes > limit || self.cache.len() > MAX_PAGES {
            let cold = self
                .cache
                .iter()
                .filter(|(k, _)| !desired.contains(k))
                .min_by_key(|(_, p)| p.touched)
                .map(|(k, _)| *k)
                .or_else(|| self.cache.iter().min_by_key(|(_, p)| p.touched).map(|(k, _)| *k));
            let Some(key) = cold else { break };
            if let Some(p) = self.cache.remove(&key) {
                bytes = bytes.saturating_sub(p.bytes());
            }
        }
        // First make every visible page available, then refine provisional reduced pages.
        let previews_ready = wanted.iter().all(|k| self.cache.contains_key(k));
        for key in wanted {
            if self.jobs.len() >= MAX_JOBS {
                break;
            }
            if self.jobs.contains_key(key) || self.failed.contains(key) {
                continue;
            }
            let exact = key.factor == 1 || !photocraft_compose::proxy::proxy_faithful(&doc) || (refine && previews_ready);
            if self.cache.get(key).is_some_and(|p| !exact || p.exact) {
                continue;
            }
            if bytes.saturating_add(514 * 514 * 20) > limit {
                break;
            }
            self.spawn(ctx, doc.clone(), *key, display.clone(), exact);
        }
        self.stats.resident_pages = self.cache.len();
        self.stats.resident_bytes = bytes;
        self.stats.active_jobs = self.jobs.len();
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn spawn(&mut self, ctx: &egui::Context, doc: Arc<Document>, key: PageKey, display: Option<Arc<CanvasDisplay>>, exact: bool) {
        if ACTIVE.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |n| (n < MAX_JOBS).then_some(n + 1)).is_err() {
            return;
        }
        let slot = WorkerSlot;
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(key, cancel.clone());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let result = std::thread::Builder::new().name("pc-viewport-page".into()).spawn(move || {
                let _slot = slot;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(&doc, key, display.as_deref(), exact, &cancel)))
                    .unwrap_or_else(|_| Err("viewport rendering failed; original document preserved".into()));
                let _ = tx.try_send(Completed { key, exact, result });
                ctx.request_repaint();
            });
            if let Err(e) = result {
                self.jobs.remove(&key);
                self.stats.last_error = Some(e.to_string());
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _slot = slot;
            let result = compute(&doc, key, display.as_deref(), exact, &cancel);
            let _ = tx.try_send(Completed { key, exact, result });
            ctx.request_repaint();
        }
    }

    /// Upload at most two pages per frame; all disk reads and colour transforms ran on workers.
    pub fn upload(&mut self, ctx: &egui::Context, doc: &Document, wanted: &[PageKey], gpu: Option<&GpuCanvas>, limit: u64, encode_srgb: bool) {
        let mut n = 0;
        for key in wanted {
            let Some(page) = self.cache.get_mut(key) else { continue };
            if let Some(g) = gpu {
                if !page.gpu && n < 2 {
                    page.gpu = page.buffer.as_ref().is_some_and(|b| {
                        g.upload_page(
                            crate::gpu_canvas::PageUpload {
                                key: doc.id.0,
                                page: key.id(),
                                size: [doc.size.width, doc.size.height],
                                factor: key.factor,
                                depth: doc.depth,
                                limit,
                                core: key.core(doc),
                            },
                            b,
                        )
                    });
                    if page.gpu {
                        if key.factor == 1 && g.composite_resident(doc, key.source(doc), encode_srgb).is_ok() {
                            self.stats.gpu_composites += 1;
                        }
                        self.stats.uploads += 1;
                        n += 1;
                        page.buffer = None;
                    }
                }
            } else if page.texture.is_none() && n < 2 {
                page.texture = Some(ctx.load_texture(format!("page-{}", key.id()), page.image.clone(), TextureOptions::LINEAR));
                self.stats.uploads += 1;
                n += 1;
                page.buffer = None;
            }
        }
        if n > 0 {
            ctx.request_repaint();
        }
        self.stats.resident_bytes = self.cache.values().map(Page::bytes).sum();
    }

    pub fn paint_cpu(&self, painter: &egui::Painter, xf: &crate::canvas::ViewXform, doc: &Document, wanted: &[PageKey]) {
        for key in wanted {
            let Some(p) = self.cache.get(key) else { continue };
            let Some(t) = &p.texture else { continue };
            let core = key.core(doc);
            let full = Rect::from_xywh(p.rect.x0 * key.factor as i32, p.rect.y0 * key.factor as i32, p.rect.width() * key.factor, p.rect.height() * key.factor);
            let uv = egui::Rect::from_min_max(
                egui::pos2((core.x0 - full.x0) as f32 / full.width() as f32, (core.y0 - full.y0) as f32 / full.height() as f32),
                egui::pos2((core.x1 - full.x0) as f32 / full.width() as f32, (core.y1 - full.y0) as f32 / full.height() as f32),
            );
            let uv = if xf.flip { egui::Rect::from_min_max(egui::pos2(uv.max.x, uv.min.y), egui::pos2(uv.min.x, uv.max.y)) } else { uv };
            painter.image(t.id(), xf.doc_rect(core), uv, Color32::WHITE);
        }
    }
}

pub struct StreamOptions<'a> {
    pub limit: u64,
    pub gpu: Option<&'a GpuCanvas>,
    pub refine: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    #[test]
    fn page_count_depends_on_view_not_document_size() {
        let d = Document::new("Earth", photocraft_doc::Size::new(86400, 43200), ColorMode::Rgb, SampleType::U8);
        let p = pages(&d, Rect::new(64000, 22000, 66048, 23080), 1.0, 1, 0, 0);
        assert!(p.len() <= 20);
        assert!(p.iter().all(|p| p.factor == 1));
        assert!(pages(&d, d.bounds(), 0.01, 1, 0, 0).len() <= MAX_PAGES);
        assert_eq!(factor(f32::NAN), 1);
    }
    #[test]
    fn neighboring_pages_have_matching_border_footprints() {
        let d = Document::new("d", photocraft_doc::Size::new(5000, 3000), ColorMode::Rgb, SampleType::U8);
        let p = pages(&d, d.bounds(), 0.5, 1, 0, 0);
        assert!(p.iter().all(|p| p.source(&d).width() <= 514 * p.factor));
        assert!(p.iter().all(|p| p.source(&d).x0 % p.factor as i32 == 0));
    }

    #[test]
    fn local_damage_reuses_only_unaffected_pages() {
        let d = Document::new("d", photocraft_doc::Size::new(2048, 512), ColorMode::Rgb, SampleType::U8);
        let old = pages(&d, d.bounds(), 1.0, 1, 0, 0);
        let mut s = Stream::default();
        for key in &old {
            s.cache.insert(
                *key,
                Page {
                    buffer: None,
                    rect: key.core(&d),
                    image: egui::ColorImage::filled([1, 1], Color32::RED),
                    texture: None,
                    gpu: false,
                    exact: true,
                    touched: 0,
                },
            );
        }
        let target = PageKey { revision: 2, snapshot: 123, ..old[0] };
        let damage = Rect::new(10, 10, 20, 20);
        s.rebase_damage(&d, target, damage, None);
        assert_eq!(s.cache.keys().filter(|k| k.revision == 2).count(), 3);
        assert!(s.cache.keys().filter(|k| k.revision == 2).all(|k| k.source(&d).intersect(&damage).is_empty()));
        assert_eq!(PageKey { x: u32::MAX, ..target }.core(&d), Rect::EMPTY);
    }

    #[test]
    fn asynchronous_cpu_pages_preserve_source_pixels_and_discard_obsolete_work() {
        let mut d = Document::new("large", photocraft_doc::Size::new(86400, 43200), ColorMode::Rgb, SampleType::U8);
        let mut surface = photocraft_raster::Surface::new(d.pixel_format());
        for x in 65000..65080 {
            surface.fill_rect(Rect::new(x, 22000, x + 1, 22080), &[if x % 2 == 0 { 1.0 } else { 0.0 }, 0.5, 0.25, 1.0]);
        }
        d.layers.push(photocraft_doc::Layer::new("detail", photocraft_doc::LayerContent::Raster(surface)));
        let doc = Arc::new(d);
        let ctx = egui::Context::default();
        let mut s = Stream::default();
        let old = pages(&doc, Rect::new(10, 10, 20, 20), 1.0, 0, 0, 0);
        let wanted = pages(&doc, Rect::new(65000, 22000, 65080, 22080), 2.0, 1, 0, 0);
        let limit = 12 << 20;
        s.update(&ctx, doc.clone(), &old, None, StreamOptions { limit, gpu: None, refine: true });
        for _ in 0..1000 {
            s.update(&ctx, doc.clone(), &wanted, None, StreamOptions { limit, gpu: None, refine: true });
            s.upload(&ctx, &doc, &wanted, None, 0, false);
            assert!(s.stats.active_jobs <= MAX_JOBS);
            assert!(s.stats.resident_bytes <= limit);
            if wanted.iter().all(|k| s.texture(k).is_some()) && s.jobs.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(s.stats.cancellations > 0);
        assert!(s.stats.last_error.is_none(), "{:?}", s.stats.last_error);
        assert!(wanted.iter().all(|k| s.texture(k).is_some()));
        assert!(s.cache.keys().all(|k| k.revision == 1 && k.factor == 1));
        for key in wanted {
            let p = &s.cache[&key];
            let reference = photocraft_compose::render(&doc, p.rect).to_rgba8();
            assert_eq!(p.image, egui::ColorImage::from_rgba_unmultiplied([reference.width as usize, reference.height as usize], &reference.pixels));
            assert!(p.buffer.is_none(), "upload must release the float buffer");
        }
    }
}
