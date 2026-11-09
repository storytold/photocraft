//! File › Open on a PDF: Photoshop's Import PDF dialog, as it lays it out. Select Pages or Images
//! with thumbnails (Thumbnail Size Small, Large or Fit Page; click, ⇧-click a range, ⌘/Ctrl-click to
//! toggle), Page Options (Name, Crop To, Anti-aliased), Image Size (Width and Height in a unit,
//! Constrain Proportions, Resolution, Mode, Bit Depth), Suppress Warnings and Open In Reverse
//! Order. OK opens every selected page (or image) as its own document through the ordinary open
//! path, or opens selected pages as raster layers in one document when that destination is chosen.
//!
//! The answers live in the dialog fields (`ui.dialog.set` drives them); the file's bytes and the
//! thumbnail textures live in [`PdfImport`] on the app. The import itself is
//! `photocraft_io::pdf_import`, which also holds what was measured on Photoshop.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_io::pdf_import_dialog::{self as pdf_import, CropTo, PdfImage, PdfInfo, PdfOpenOptions};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::widgets;

/// The field that marks a dialog as this module's.
const FIELD: &str = "__pdfImport";
/// The dialog's id in its fields: the held file is looked up by it, so several PDFs opened
/// together (Open, a drop, the command line) each keep their own.
const DIALOG: &str = "__pdfDialog";
/// The dialog's width in points (Photoshop's is about 690 px).
pub const WIDTH: f32 = 700.0;
/// Thumbnail Size choices (field value, label).
pub const THUMB_SIZES: &[(&str, &str)] = &[("small", "Small"), ("large", "Large"), ("fitPage", "Fit Page")];
/// Mode choices (field value, label), in Photoshop's order.
pub const MODES: &[(&str, &str)] = &[("gray", "Grayscale"), ("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
/// Bit Depth choices.
pub const DEPTHS: &[(u64, &str)] = &[(8, "8 bit"), (16, "16 bit")];
/// Resolution unit choices (field value, label).
pub const RES_UNITS: &[(&str, &str)] = &[("in", "Pixels/Inch"), ("cm", "Pixels/Centimeter")];
/// Longest side of a thumbnail, per Thumbnail Size (Fit Page follows the panel).
const SMALL: u32 = 64;
const LARGE: u32 = 128;
/// The thumbnail panel's size in points.
const PANEL: egui::Vec2 = vec2(296.0, 290.0);
/// Aggregate limit for one dialog confirmation; 128 MP bounds simultaneous page documents to
/// about 1 GiB even for 16-bit CMYK, before surface/history overhead.
const MAX_SELECTED_PIXELS: u64 = 128_000_000;
/// A single confirmation starts one open job per selected item.
const MAX_SELECTED_ITEMS: usize = 500;

/// A rendered thumbnail on the GPU.
struct Thumb {
    tex: egui::TextureHandle,
    size: (u32, u32),
}

enum PdfTask {
    Inspect(Option<String>),
    Images(Option<String>),
    Thumb { images: bool, index: usize, side: u32, size: String, password: Option<String> },
}

enum PdfResult {
    Inspect { password: Option<String>, result: Result<(PdfInfo, Vec<[f32; 4]>), String> },
    Images(Result<Vec<PdfImage>, String>),
    Thumb { images: bool, index: usize, size: String, result: Result<pdf_import::Thumbnail, String> },
}

/// Execute a PDF inspection or thumbnail task outside the UI-frame update.
fn run_pdf_task(bytes: &Arc<Vec<u8>>, task: PdfTask) -> PdfResult {
    match task {
        PdfTask::Inspect(password) => {
            PdfResult::Inspect { result: pdf_import::info_with_bounds(bytes, password.as_deref()).map_err(|e| e.to_string()), password }
        }
        PdfTask::Images(password) => PdfResult::Images(pdf_import::images(bytes, password.as_deref()).map_err(|e| e.to_string())),
        PdfTask::Thumb { images, index, side, size, password } => {
            let result = if images {
                pdf_import::image_thumbnail(bytes, index, side, password.as_deref())
            } else {
                pdf_import::thumbnail(bytes, index, side, password.as_deref())
            }
            .map_err(|e| e.to_string());
            PdfResult::Thumb { images, index, size, result }
        }
    }
}

/// One bounded worker per open PDF dialog. Hayro's parser and renderer are synchronous, so all
/// inspection and thumbnails run here instead of during an egui frame on native platforms.
struct PdfWorker {
    #[cfg(not(target_arch = "wasm32"))]
    tasks: Sender<PdfTask>,
    results: Receiver<PdfResult>,
    #[cfg(not(target_arch = "wasm32"))]
    cancelled: Arc<AtomicBool>,
    #[cfg(target_arch = "wasm32")]
    result_tx: Sender<PdfResult>,
    #[cfg(target_arch = "wasm32")]
    bytes: Arc<Vec<u8>>,
}

impl PdfWorker {
    /// Create a PDF inspection worker with its task and result channels.
    fn new(bytes: Arc<Vec<u8>>) -> Result<Self, String> {
        let (result_tx, result_rx) = mpsc::channel();
        #[cfg(target_arch = "wasm32")]
        {
            return Ok(Self { results: result_rx, result_tx, bytes });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (task_tx, task_rx) = mpsc::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancelled = Arc::clone(&cancelled);
            std::thread::Builder::new()
                .name("pdf-import".into())
                .spawn(move || {
                    while let Ok(task) = task_rx.recv() {
                        if worker_cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        let result = run_pdf_task(&bytes, task);
                        if worker_cancelled.load(Ordering::Relaxed) || result_tx.send(result).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|e| format!("couldn't start the PDF worker: {e}"))?;
            Ok(Self { tasks: task_tx, results: result_rx, cancelled })
        }
    }

    /// Queue a PDF inspection or thumbnail task for the active dialog.
    fn send(&self, task: PdfTask) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            self.result_tx.send(run_pdf_task(&self.bytes, task)).map_err(|_| "the PDF dialog closed".to_string())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.tasks.send(task).map_err(|_| "the PDF worker stopped unexpectedly".to_string())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for PdfWorker {
    /// Signal the PDF inspection worker to cancel when its dialog is removed.
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// The file behind the dialog: what the fields can't hold.
pub struct PdfImport {
    /// The dialog's id.
    pub dialog: u64,
    /// The file name (the Name field starts as it).
    pub file_name: String,
    /// Where it was read from (Open Recent), when known.
    pub path: Option<String>,
    pub bytes: Arc<Vec<u8>>,
    /// The pages, once the file could be read (a password may come first).
    pub info: Option<PdfInfo>,
    /// The raster images, listed when Images is first chosen.
    images: Option<Vec<PdfImage>>,
    /// The accepted user password of an encrypted file.
    pub password: Option<String>,
    thumbs: Vec<Option<Thumb>>,
    image_thumbs: Vec<Option<Thumb>>,
    worker: PdfWorker,
    loading: bool,
    images_pending: bool,
    images_failed: bool,
    thumb_pending: HashSet<(bool, usize, String)>,
    thumb_failed: HashSet<(bool, usize, String)>,
    /// The Thumbnail Size the textures were rendered for.
    thumb_key: String,
    /// Bounding boxes per page, computed when a page's size is first needed.
    bounds: Vec<Option<[f32; 4]>>,
    /// The item a ⇧-click range starts from.
    anchor: Option<usize>,
}

/// Whether `bytes` should open through this dialog. Signature checks are cheap; parsing is done
/// after the dialog has started, on its worker thread.
pub fn wants_dialog(name: &str, bytes: &[u8]) -> bool {
    photocraft_io::is_pdf_input(name, bytes)
}

/// Whether `fields` are this dialog's.
pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(FIELD)
}

/// The id of the dialog `fields` belong to.
fn dialog_of(f: &Map<String, Value>) -> Option<u64> {
    f.get(DIALOG).and_then(Value::as_u64)
}

/// The held file of the dialog `fields` belong to, taken out of the app (put it back when the
/// dialog stays up).
fn take(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Option<PdfImport> {
    let id = dialog_of(f)?;
    let i = app.pdf_import.iter().position(|st| st.dialog == id)?;
    Some(app.pdf_import.remove(i))
}

/// Opens `fields` as a new dialog and ties `st` to it: the held file follows the dialog.
fn show(app: &mut PhotocraftApp, mut st: PdfImport, mut fields: Map<String, Value>) -> u64 {
    fields.insert(DIALOG.into(), json!(0));
    let id = app.ui.open_dialog(DialogKind::Command, fields);
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert(DIALOG.into(), json!(id));
    }
    st.dialog = id;
    app.pdf_import.push(st);
    id
}

/// Lets the file held for dialog `id` go (the dialog was cancelled or closed).
pub fn forget(app: &mut PhotocraftApp, id: u64) {
    app.pdf_import.retain(|st| st.dialog != id);
}

/// Opens the dialog for the PDF `bytes` read from `path` (when known) as `name`. An encrypted
/// file asks for its password first. Returns the dialog id.
pub fn open(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: &[u8]) -> Result<u64, String> {
    if bytes.len() > pdf_import::MAX_PDF_BYTES {
        return Err(format!("the PDF is larger than the {} MiB import limit", pdf_import::MAX_PDF_BYTES / (1024 * 1024)));
    }
    let bytes = Arc::new(bytes.to_vec());
    let worker = PdfWorker::new(Arc::clone(&bytes))?;
    worker.send(PdfTask::Inspect(None))?;
    let mut f = Map::new();
    f.insert(FIELD.into(), json!(true));
    f.insert("__label".into(), json!(tl!("Import PDF")));
    f.insert("name".into(), json!(name));
    f.insert("__loading".into(), json!(true));
    defaults(&mut f, app);
    let st = PdfImport {
        dialog: 0,
        file_name: name.to_string(),
        path: path.map(str::to_string),
        bytes,
        info: None,
        images: None,
        password: None,
        thumbs: Vec::new(),
        image_thumbs: Vec::new(),
        worker,
        loading: true,
        images_pending: false,
        images_failed: false,
        thumb_pending: HashSet::new(),
        thumb_failed: HashSet::new(),
        thumb_key: String::new(),
        bounds: Vec::new(),
        anchor: None,
    };
    Ok(show(app, st, f))
}

/// Photoshop's defaults: the first page, Bounding Box, anti-aliased, 300 ppi, RGB, 8 bits, the
/// ruler unit for the size, proportions free, warnings shown, pages opened in reverse order.
fn defaults(f: &mut Map<String, Value>, app: &PhotocraftApp) {
    f.insert("select".into(), json!("pages"));
    f.insert("pages".into(), json!([1]));
    f.insert("images".into(), json!([]));
    f.insert("thumbnailSize".into(), json!("small"));
    f.insert("crop".into(), json!(CropTo::BoundingBox.id()));
    f.insert("antiAlias".into(), json!(true));
    // The ruler unit, when the size fields know it (percent falls back to pixels).
    let suffix = app.session.prefs().units_and_rulers.rulers.suffix().trim();
    let unit = crate::new_doc_ui::UNITS.iter().map(|u| u.0).find(|u| *u == suffix).unwrap_or("px");
    f.insert("unit".into(), json!(unit));
    f.insert("constrain".into(), json!(false));
    f.insert("resolution".into(), json!(pdf_import::DEFAULT_RESOLUTION));
    f.insert("resolutionUnit".into(), json!("in"));
    f.insert("mode".into(), json!("rgb"));
    f.insert("depth".into(), json!(8));
    f.insert("suppressWarnings".into(), json!(false));
    f.insert("reverseOrder".into(), json!(true));
    f.insert("pagesAsLayers".into(), json!(false));
}

/// Read a string-valued PDF dialog field with a fallback.
fn s<'a>(f: &'a Map<String, Value>, k: &str, d: &'a str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or(d)
}
/// Read a numeric PDF dialog field with a fallback.
fn num(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
/// Read a boolean PDF dialog field with a fallback.
fn flag(f: &Map<String, Value>, k: &str, d: bool) -> bool {
    f.get(k).and_then(Value::as_bool).unwrap_or(d)
}
/// The selected 1-based numbers under `k`, sorted and unique.
fn picks(f: &Map<String, Value>, k: &str) -> Vec<usize> {
    let mut v: Vec<usize> =
        f.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).filter(|n| *n >= 1).map(|n| n as usize).collect()).unwrap_or_default();
    v.sort_unstable();
    v.dedup();
    v
}

/// Estimate the total raster size of the currently selected pages or images.
fn selected_pixels(st: &PdfImport, f: &Map<String, Value>, selected: &[usize], images: bool) -> Option<u64> {
    let mut total = 0u64;
    if images {
        let list = st.images.as_ref()?;
        for n in selected {
            let image = list.get(n.checked_sub(1)?)?;
            total = total.checked_add(u64::from(image.width).checked_mul(u64::from(image.height))?)?;
        }
    } else {
        let info = st.info.as_ref()?;
        let crop = CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default();
        let resolution = num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION);
        let custom = f.get("pixelSize").and_then(Value::as_array).and_then(|a| match a.as_slice() {
            [w, h] => Some((w.as_u64()? as u32, h.as_u64()? as u32)),
            _ => None,
        });
        for n in selected {
            let index = n.checked_sub(1)?;
            let page = info.pages.get(index)?;
            let bounds = if crop == CropTo::BoundingBox { st.bounds.get(index).copied().flatten().unwrap_or(page.crop) } else { page.box_for(crop) };
            let size = custom.unwrap_or_else(|| pdf_import::pixel_size(page.displayed_size(bounds), resolution));
            total = total.checked_add(u64::from(size.0).checked_mul(u64::from(size.1))?)?;
        }
    }
    Some(total)
}

/// The options the fields describe for page or image `n`.
pub fn options(f: &Map<String, Value>, n: usize, images: bool, password: Option<&str>) -> PdfOpenOptions {
    let mode = match s(f, "mode", "rgb") {
        "gray" => photocraft_color::ColorMode::Grayscale,
        "cmyk" => photocraft_color::ColorMode::Cmyk,
        "lab" => photocraft_color::ColorMode::Lab,
        _ => photocraft_color::ColorMode::Rgb,
    };
    let depth = if f.get("depth").and_then(Value::as_u64) == Some(16) { photocraft_color::SampleType::U16 } else { photocraft_color::SampleType::U8 };
    let sized = f.get("pixelSize").and_then(Value::as_array).and_then(|a| match a.as_slice() {
        [w, h] => Some((w.as_u64()? as u32, h.as_u64()? as u32)),
        _ => None,
    });
    PdfOpenOptions {
        page: if images { 1 } else { n },
        image: images.then_some(n),
        crop: CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default(),
        resolution: num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION),
        mode,
        depth,
        anti_alias: flag(f, "antiAlias", true),
        name: Some(s(f, "name", "").to_string()).filter(|n| !n.trim().is_empty()),
        pixel_size: sized.filter(|_| !images),
        password: password.map(str::to_string),
        suppress_warnings: flag(f, "suppressWarnings", false),
        pages_as_layers: None,
        pages_as_artboards: None,
    }
}

/// The page box the fields crop to for page `index` (0-based), in points as displayed.
fn page_size_pt(st: &mut PdfImport, f: &Map<String, Value>, index: usize) -> Option<(f32, f32)> {
    let info = st.info.as_ref()?;
    let page = info.pages.get(index)?;
    let crop = CropTo::from_id(s(f, "crop", "boundingBox")).unwrap_or_default();
    let b = if crop == CropTo::BoundingBox { st.bounds.get(index).copied().flatten().unwrap_or(page.crop) } else { page.box_for(crop) };
    Some(page.displayed_size(b))
}

/// The pixel size the dialog shows: the typed one, else the first selected page's box at the
/// resolution.
fn pixel_size(st: &mut PdfImport, f: &Map<String, Value>) -> Option<(u32, u32)> {
    if let Some((w, h)) = f.get("pixelSize").and_then(Value::as_array).and_then(|a| match a.as_slice() {
        [w, h] => Some((w.as_u64()? as u32, h.as_u64()? as u32)),
        _ => None,
    }) {
        return Some((w.max(1), h.max(1)));
    }
    let first = picks(f, "pages").first().copied()?;
    let pt = page_size_pt(st, f, first.checked_sub(1)?)?;
    Some(pdf_import::pixel_size(pt, num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION)))
}

/// "20.8M" for a size in bytes, as Photoshop's Image Size line (1 M = 1024 × 1024).
pub fn size_label(w: u32, h: u32, mode: &str, depth: u64) -> String {
    let channels = match mode {
        "gray" => 1.0,
        "cmyk" => 4.0,
        _ => 3.0,
    };
    let bytes = f64::from(w) * f64::from(h) * channels * if depth == 16 { 2.0 } else { 1.0 };
    // Three significant digits, as Photoshop prints it (20.8M, 3.20M, 512K).
    let three = |v: f64| {
        if v >= 100.0 {
            format!("{v:.0}")
        } else if v >= 10.0 {
            format!("{v:.1}")
        } else {
            format!("{v:.2}")
        }
    };
    if bytes >= 1024.0 * 1024.0 { format!("{}M", three(bytes / (1024.0 * 1024.0))) } else { format!("{}K", three(bytes / 1024.0)) }
}

/// The thumbnail's longest side in pixels for a Thumbnail Size.
fn thumb_side(size: &str) -> u32 {
    match size {
        "large" => LARGE,
        "fitPage" => (PANEL.x - 40.0) as u32,
        _ => SMALL,
    }
}

/// Apply worker results on the UI thread, including terminal failures so a broken preview cannot
/// request repaint forever.
fn poll_worker(st: &mut PdfImport, ctx: &egui::Context, f: &mut Map<String, Value>) {
    loop {
        match st.worker.results.try_recv() {
            Ok(PdfResult::Inspect { password, result }) => {
                st.loading = false;
                f.remove("__loading");
                match result {
                    Ok((info, bounds)) => {
                        let n = info.pages.len();
                        st.info = Some(info);
                        st.bounds = bounds.into_iter().map(Some).collect();
                        st.password = password;
                        st.thumbs = (0..n).map(|_| None).collect();
                        f.remove("__needsPassword");
                        f.remove("password");
                        f.remove("__error");
                    }
                    Err(e) if e.contains(pdf_import::PASSWORD_REQUIRED) => {
                        st.info = None;
                        f.insert("__needsPassword".into(), json!(true));
                        f.entry("password".to_string()).or_insert_with(|| json!(""));
                    }
                    Err(e) if e.contains(pdf_import::WRONG_PASSWORD) => {
                        st.info = None;
                        f.insert("__needsPassword".into(), json!(true));
                        f.insert("__error".into(), json!("That isn't the PDF's password."));
                    }
                    Err(e) => {
                        f.remove("__needsPassword");
                        f.insert("__error".into(), json!(e));
                    }
                }
            }
            Ok(PdfResult::Images(result)) => {
                st.images_pending = false;
                match result {
                    Ok(images) => {
                        st.image_thumbs = (0..images.len()).map(|_| None).collect();
                        st.images = Some(images);
                        st.images_failed = false;
                    }
                    Err(e) => {
                        st.images_failed = true;
                        f.insert("__imageError".into(), json!(e));
                    }
                }
            }
            Ok(PdfResult::Thumb { images, index, size, result }) => {
                st.thumb_pending.remove(&(images, index, size.clone()));
                if size != st.thumb_key {
                    continue;
                }
                match result {
                    Ok(t) => {
                        let thumb = Thumb {
                            tex: ctx.load_texture(
                                format!("pdf-{}-thumb-{index}", if images { "image" } else { "page" }),
                                egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba),
                                egui::TextureOptions::LINEAR,
                            ),
                            size: (t.width, t.height),
                        };
                        let slots = if images { &mut st.image_thumbs } else { &mut st.thumbs };
                        if let Some(slot) = slots.get_mut(index.saturating_sub(1)) {
                            *slot = Some(thumb);
                        }
                    }
                    Err(_) => {
                        st.thumb_failed.insert((images, index, size));
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                if st.loading {
                    st.loading = false;
                    f.remove("__loading");
                    f.insert("__error".into(), json!("the PDF worker stopped unexpectedly"));
                }
                break;
            }
        }
    }
}

/// Requests missing thumbnails a few at a time; decoding and rendering stay on the worker.
fn render_thumbs(st: &mut PdfImport, ctx: &egui::Context, f: &mut Map<String, Value>, images: bool, size: &str) {
    if st.thumb_key != size {
        st.thumbs.iter_mut().for_each(|t| *t = None);
        st.image_thumbs.iter_mut().for_each(|t| *t = None);
        st.thumb_failed.clear();
        st.thumb_key = size.to_string();
    }
    let side = thumb_side(size);
    let mut budget = 3;
    if images {
        if st.images.is_none() && !st.images_pending && !st.images_failed {
            match st.worker.send(PdfTask::Images(st.password.clone())) {
                Ok(()) => st.images_pending = true,
                Err(e) => {
                    st.images_failed = true;
                    f.insert("__imageError".into(), json!(e));
                }
            }
        }
        for i in 0..st.image_thumbs.len() {
            let key = (true, i + 1, size.to_string());
            if st.image_thumbs[i].is_some() || st.thumb_pending.contains(&key) || st.thumb_failed.contains(&key) || budget == 0 {
                continue;
            }
            budget -= 1;
            match st.worker.send(PdfTask::Thumb { images: true, index: i + 1, side, size: size.to_string(), password: st.password.clone() }) {
                Ok(()) => {
                    st.thumb_pending.insert(key);
                }
                Err(_) => {
                    st.thumb_failed.insert(key);
                }
            }
        }
        if st.images_pending || st.thumb_pending.iter().any(|(kind, _, key)| *kind && key == size) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        return;
    }
    for i in 0..st.thumbs.len() {
        let key = (false, i + 1, size.to_string());
        if st.thumbs[i].is_some() || st.thumb_pending.contains(&key) || st.thumb_failed.contains(&key) || budget == 0 {
            continue;
        }
        budget -= 1;
        match st.worker.send(PdfTask::Thumb { images: false, index: i + 1, side, size: size.to_string(), password: st.password.clone() }) {
            Ok(()) => {
                st.thumb_pending.insert(key);
            }
            Err(_) => {
                st.thumb_failed.insert(key);
            }
        }
    }
    if st.thumb_pending.iter().any(|(kind, _, key)| !*kind && key == size) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// The dialog body. Returns nothing: OK and Cancel are the frame's.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(mut st) = take(app, f) else {
        ui.label(egui::RichText::new(tl!("The PDF is no longer available.")).color(t.danger));
        return;
    };
    poll_worker(&mut st, ui.ctx(), f);
    if st.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tl!("Reading PDF…"));
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        app.pdf_import.push(st);
        return;
    }
    if flag(f, "__needsPassword", false) {
        password_body(ui, f, &t);
        app.pdf_import.push(st);
        return;
    }
    if st.info.is_none() {
        ui.label(egui::RichText::new(f.get("__error").and_then(Value::as_str).unwrap_or("The PDF couldn't be read.")).color(t.danger));
        app.pdf_import.push(st);
        return;
    }
    let images = s(f, "select", "pages") == "images";
    let size = s(f, "thumbnailSize", "small").to_string();
    render_thumbs(&mut st, ui.ctx(), f, images, &size);
    // Keep the standard dialog buttons reachable on short windows, as other import dialogs do.
    let room = (ui.ctx().content_rect().height() - 170.0).max(180.0);
    egui::ScrollArea::vertical().id_salt("pdf-import-body").max_height(room).show(ui, |ui| {
        // Reserve a stable body height even while asynchronous image thumbnails
        // load. Otherwise the auto-centered dialog jumps under the cursor,
        // making a pending Pages/Images radio click target the wrong position.
        ui.set_min_height(room);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(PANEL.x);
                left_column(ui, f, &mut st, &t, images, &size);
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(WIDTH - PANEL.x - 24.0);
                right_column(ui, f, &mut st, &t, images);
            });
        })
    });
    app.pdf_import.push(st);
}

/// Draw the password prompt for an encrypted PDF.
fn password_body(ui: &mut egui::Ui, f: &mut Map<String, Value>, t: &Tokens) {
    ui.label(egui::RichText::new(tl!("The PDF is protected by a password.")).color(t.text));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Password:")).color(t.text_dim));
        let mut pw = s(f, "password", "").to_string();
        let id = ui.scope_id().with("pdf-password");
        let r = widgets::text_field(ui, id, &mut pw, "", 220.0, false);
        if r.changed() {
            f.insert("password".into(), json!(pw));
        }
    });
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(e)).color(t.danger));
    }
}

/// Draw the PDF pages or images list and its thumbnail selections.
fn left_column(ui: &mut egui::Ui, f: &mut Map<String, Value>, st: &mut PdfImport, t: &Tokens, images: bool, size: &str) {
    ui.label(egui::RichText::new(tl!("Select:")).color(t.text_dim));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        if radio(ui, !images, "Pages").clicked() {
            f.insert("select".into(), json!("pages"));
        }
        if radio(ui, images, "Images").clicked() {
            f.insert("select".into(), json!("images"));
        }
    });
    ui.add_space(4.0);
    // The thumbnail panel.
    let (panel, _) = ui.allocate_exact_size(PANEL, Sense::hover());
    ui.painter().rect_filled(panel, t.radius_sm, t.field);
    ui.painter().rect_stroke(panel, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    let key = if images { "images" } else { "pages" };
    let count = if images { st.images.as_ref().map_or(0, Vec::len) } else { st.info.as_ref().map_or(0, |i| i.pages.len()) };
    let mut selected = picks(f, key);
    let side = thumb_side(size) as f32;
    let cell = vec2(side + 16.0, side + 30.0);
    let per_row = ((panel.width() - 8.0) / cell.x).floor().max(1.0) as usize;
    let rows = count.div_ceil(per_row.max(1));
    let inner = panel.shrink(4.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(inner);
    egui::ScrollArea::vertical().id_salt(("pdf-thumbs", key)).max_height(inner.height()).auto_shrink([false, false]).show(&mut child, |ui| {
        let mods = ui.input(|i| i.modifiers);
        let mut clicked = None;
        let (grid, _) = ui.allocate_exact_size(vec2(inner.width(), (rows as f32 * cell.y).max(1.0)), Sense::hover());
        for i in 0..count {
            let (row, col) = (i / per_row.max(1), i % per_row.max(1));
            let r = Rect::from_min_size(grid.min + vec2(col as f32 * cell.x, row as f32 * cell.y), cell);
            if !ui.is_rect_visible(r) {
                continue;
            }
            let thumb = if images { st.image_thumbs.get(i) } else { st.thumbs.get(i) }.and_then(Option::as_ref);
            let on = selected.contains(&(i + 1));
            let paper = Rect::from_center_size(pos2(r.center().x, r.top() + 8.0 + side / 2.0), vec2(side, side));
            let (pw, ph) = thumb.map_or((side, side), |th| {
                let k = (side / th.size.0.max(1) as f32).min(side / th.size.1.max(1) as f32);
                (th.size.0 as f32 * k, th.size.1 as f32 * k)
            });
            let img = Rect::from_center_size(paper.center(), vec2(pw, ph));
            ui.painter().rect_filled(img, 0.0, Color32::WHITE);
            if let Some(th) = thumb {
                ui.painter().image(th.tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if on {
                ui.painter().rect_stroke(img.expand(2.0), 0.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
            }
            let label = (i + 1).to_string();
            ui.painter().text(pos2(r.center().x, r.bottom() - 11.0), egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(11.0), t.text);
            if on {
                let y = r.bottom() - 3.0;
                ui.painter().line_segment([pos2(img.left(), y), pos2(img.right(), y)], Stroke::new(2.0, t.accent));
            }
            let resp = ui.interact(r, ui.scope_id().with(("pdf-thumb", key, i)), Sense::click());
            if resp.clicked() {
                clicked = Some(i);
            }
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, format!("{} {label}", if images { "Image" } else { "Page" }))
            });
        }
        if let Some(i) = clicked {
            let n = i + 1;
            if mods.shift
                && let Some(a) = st.anchor
            {
                let (lo, hi) = (a.min(i), a.max(i));
                for k in lo..=hi {
                    if !selected.contains(&(k + 1)) {
                        selected.push(k + 1);
                    }
                }
            } else if mods.command {
                if let Some(p) = selected.iter().position(|v| *v == n) {
                    selected.remove(p);
                } else {
                    selected.push(n);
                }
                st.anchor = Some(i);
            } else {
                selected = vec![n];
                st.anchor = Some(i);
            }
            selected.sort_unstable();
            f.insert(key.into(), json!(selected));
            f.remove("pixelSize");
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, tl!("Select All"), 100.0).clicked() {
            selected = (1..=count).collect();
            f.insert(key.into(), json!(selected));
            st.anchor = None;
            f.remove("pixelSize");
        }
        if widgets::secondary_button(ui, tl!("None"), 70.0).clicked() {
            selected.clear();
            f.insert(key.into(), json!(selected));
            st.anchor = None;
            f.remove("pixelSize");
        }
    });
    ui.label(egui::RichText::new(tl!("Ctrl/Cmd-click: add or remove. Shift-click: select a range.")).color(t.text_dim).size(11.0));
    let status = if images && count == 0 {
        tl!("Select one or more images").to_string()
    } else if images {
        crate::i18n::fmt(tl!("{n} of {total} image(s) selected"), &[("n", &selected.len().to_string()), ("total", &count.to_string())])
    } else {
        crate::i18n::fmt(tl!("{n} of {total} page(s) selected"), &[("n", &selected.len().to_string()), ("total", &count.to_string())])
    };
    ui.label(egui::RichText::new(status).color(t.text_dim));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Thumbnail Size:")).color(t.text_dim));
        let mut cur = size.to_string();
        if widgets::dropdown(
            ui,
            "pdf-thumb-size",
            tl!("Thumbnail Size:"),
            &mut cur,
            &THUMB_SIZES.iter().map(|(v, l)| (v.to_string(), *l)).collect::<Vec<_>>(),
            110.0,
        ) {
            f.insert("thumbnailSize".into(), json!(cur));
        }
    });
}

/// Draw page options, output dimensions, and document-layout controls.
fn right_column(ui: &mut egui::Ui, f: &mut Map<String, Value>, st: &mut PdfImport, t: &Tokens, images: bool) {
    group(ui, t, tl!("Open as"), |ui| {
        let as_layers = !images && flag(f, "pagesAsLayers", false);
        let as_artboards = !images && flag(f, "pagesAsArtboards", false);
        if radio(ui, !as_layers && !as_artboards, "Separate tabs").clicked() && !images {
            f.insert("pagesAsLayers".into(), json!(false));
            f.insert("pagesAsArtboards".into(), json!(false));
        }
        if images {
            ui.label(egui::RichText::new(tl!("One selected image per tab.")).color(t.text_dim).size(11.0));
        } else {
            if radio(ui, as_layers, "Layers in one document").clicked() {
                f.insert("pagesAsLayers".into(), json!(true));
                f.insert("pagesAsArtboards".into(), json!(false));
            }
            if radio(ui, as_artboards, "PDF pages as artboards (editable)").clicked() {
                f.insert("pagesAsLayers".into(), json!(false));
                f.insert("pagesAsArtboards".into(), json!(true));
                // A PDF page is a *sheet*, not a tight crop around its ink.
                // The legacy Bounding Box default removes blank margins and can
                // make the page appear cropped at the bottom and sides.
                if s(f, "crop", "boundingBox") == "boundingBox" {
                    f.insert("crop".into(), json!(CropTo::MediaBox.id()));
                }
                // A size typed for a different mode/page must not distort all pages.
                f.remove("pixelSize");
            }
            let hint = if as_artboards {
                "One artboard per PDF page in one tab, for page-by-page editing."
            } else if as_layers {
                tl!("One selected page per layer, in one tab.")
            } else {
                tl!("One selected page per tab.")
            };
            ui.label(egui::RichText::new(hint).color(t.text_dim).size(11.0));
        }
    });
    ui.add_space(6.0);
    let label_w = 76.0;
    let field = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(label_w, 22.0), Sense::hover());
            ui.painter().text(pos2(r.right() - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, tl!(label), egui::FontId::proportional(12.0), t.text_dim);
            add(ui);
        });
    };
    ui.add_enabled_ui(!images, |ui| {
        group(ui, t, tl!("Page Options"), |ui| {
            field(ui, "Name:", &mut |ui| {
                let mut name = s(f, "name", "").to_string();
                let r = widgets::text_field(ui, ui.scope_id().with("pdf-name"), &mut name, "", 190.0, false).accessible_name(tl!("Name:"));
                if r.changed() {
                    f.insert("name".into(), json!(name));
                }
            });
            field(ui, "Crop To:", &mut |ui| {
                let mut cur = s(f, "crop", "boundingBox").to_string();
                let opts: Vec<(String, &str)> = CropTo::ALL.iter().map(|c| (c.id().to_string(), c.label())).collect();
                if widgets::dropdown(ui, "pdf-crop", tl!("Crop To:"), &mut cur, &opts, 120.0) {
                    f.insert("crop".into(), json!(cur));
                    f.remove("pixelSize");
                }
            });
            let mut aa = flag(f, "antiAlias", true);
            if widgets::checkbox(ui, &mut aa, tl!("Anti-aliased")).changed() {
                f.insert("antiAlias".into(), json!(aa));
            }
        });
        ui.add_space(6.0);
        let (w, h) = pixel_size(st, f).unwrap_or((0, 0));
        let mode = s(f, "mode", "rgb").to_string();
        let depth = f.get("depth").and_then(Value::as_u64).unwrap_or(8);
        let title =
            if w == 0 { tl!("Image Size: --").to_string() } else { crate::i18n::fmt(tl!("Image Size: {size}"), &[("size", &size_label(w, h, &mode, depth))]) };
        group(ui, t, &title, |ui| {
            let unit = s(f, "unit", "px").to_string();
            let ppi = num(f, "resolution", pdf_import::DEFAULT_RESOLUTION).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION);
            let constrain = flag(f, "constrain", false);
            let units: Vec<(String, &str)> = crate::new_doc_ui::UNITS.iter().map(|(v, l, _)| (v.to_string(), *l)).collect();
            let decimals = if unit == "px" { 0 } else { 2 };
            // Multiple PDF pages may have different dimensions: a single manual
            // width/height would stretch or truncate all of them. Only DPI is shared.
            ui.add_enabled_ui(!flag(f, "pagesAsArtboards", false), |ui| {
                for (key, label, cur, other) in [("width", "Width:", w, h), ("height", "Height:", h, w)] {
                    field(ui, label, &mut |ui| {
                        let mut v = crate::new_doc_ui::to_unit(cur as f32, &unit, ppi);
                        let r = widgets::value_field_prec(ui, &mut v, 0.0..=300_000.0, "", 70.0, decimals).accessible_name(tl!(label));
                        if r.changed() && !r.dragged() {
                            let px = crate::new_doc_ui::from_unit(v, &unit, ppi).round().clamp(1.0, 300_000.0) as u32;
                            let (nw, nh) = if constrain && cur > 0 {
                                let o = (f64::from(px) * f64::from(other) / f64::from(cur)).round().clamp(1.0, 300_000.0) as u32;
                                if key == "width" { (px, o) } else { (o, px) }
                            } else if key == "width" {
                                (px, other.max(1))
                            } else {
                                (other.max(1), px)
                            };
                            f.insert("pixelSize".into(), json!([nw, nh]));
                        }
                        ui.add_space(6.0);
                        let mut u = unit.clone();
                        let unit_name = if key == "width" { tl!("Width Unit:") } else { tl!("Height Unit:") };
                        if widgets::dropdown(ui, &format!("pdf-unit-{key}"), unit_name, &mut u, &units, 110.0) {
                            f.insert("unit".into(), json!(u));
                        }
                    });
                }
                let mut c = constrain;
                if widgets::checkbox(ui, &mut c, tl!("Constrain Proportions")).changed() {
                    f.insert("constrain".into(), json!(c));
                }
            });
            if flag(f, "pagesAsArtboards", false) {
                ui.small(tl!("Each PDF page keeps its full dimensions; change Resolution to change image quality."));
            }
            field(ui, "Resolution:", &mut |ui| {
                let res_unit = s(f, "resolutionUnit", "in").to_string();
                let mut v = if res_unit == "cm" { ppi / 2.54 } else { ppi };
                let r = widgets::value_field_prec(ui, &mut v, 0.1..=pdf_import::MAX_RESOLUTION, "", 70.0, if res_unit == "cm" { 2 } else { 0 })
                    .accessible_name(tl!("Resolution:"));
                if r.changed() && !r.dragged() {
                    let new = (if res_unit == "cm" { v * 2.54 } else { v }).clamp(pdf_import::MIN_RESOLUTION, pdf_import::MAX_RESOLUTION);
                    // A typed size is a physical size: it keeps its inches, so its pixels follow.
                    if let Some([pw, ph]) = f
                        .get("pixelSize")
                        .and_then(Value::as_array)
                        .map(|a| [a.first().and_then(Value::as_u64).unwrap_or(1), a.get(1).and_then(Value::as_u64).unwrap_or(1)])
                    {
                        let k = f64::from(new) / f64::from(ppi.max(0.001));
                        f.insert(
                            "pixelSize".into(),
                            json!([(pw as f64 * k).round().clamp(1.0, 300_000.0) as u64, (ph as f64 * k).round().clamp(1.0, 300_000.0) as u64]),
                        );
                    }
                    f.insert("resolution".into(), json!(new));
                }
                ui.add_space(6.0);
                let mut ru = res_unit.clone();
                let opts: Vec<(String, &str)> = RES_UNITS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                if widgets::dropdown(ui, "pdf-res-unit", tl!("Resolution Unit:"), &mut ru, &opts, 130.0) {
                    f.insert("resolutionUnit".into(), json!(ru));
                }
            });
            field(ui, "Mode:", &mut |ui| {
                let mut m = mode.clone();
                let opts: Vec<(String, &str)> = MODES.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                if widgets::dropdown(ui, "pdf-mode", tl!("Mode:"), &mut m, &opts, 130.0) {
                    f.insert("mode".into(), json!(m));
                }
            });
            field(ui, "Bit Depth:", &mut |ui| {
                let mut d = depth;
                if widgets::dropdown(ui, "pdf-depth", tl!("Bit Depth:"), &mut d, DEPTHS, 70.0) {
                    f.insert("depth".into(), json!(d));
                }
            });
        });
    });
    ui.add_space(8.0);
    let mut sw = flag(f, "suppressWarnings", false);
    if widgets::checkbox(ui, &mut sw, tl!("Suppress Warnings")).changed() {
        f.insert("suppressWarnings".into(), json!(sw));
    }
    let pages_as_layers = !images && flag(f, "pagesAsLayers", false);
    let pages_as_artboards = !images && flag(f, "pagesAsArtboards", false);
    ui.add_enabled_ui(!images && !pages_as_layers && !pages_as_artboards, |ui| {
        let mut rev = flag(f, "reverseOrder", true);
        if widgets::checkbox(ui, &mut rev, tl!("Open In Reverse Order")).changed() {
            f.insert("reverseOrder".into(), json!(rev));
        }
    });
    if let Some(e) = f.get("__error").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(e)).color(t.danger));
    }
}

/// A titled box, as Photoshop groups the page options and the image size.
fn group(ui: &mut egui::Ui, t: &Tokens, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).color(t.text_dim).size(12.0));
    egui::Frame::new().stroke(Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        content(ui);
    });
}

/// Draw a selectable radio-style option in the import dialog.
fn radio(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    // One native radio widget covers the indicator and its label, with an explicit
    // accessible name and one hit target. The old split circle/label could leave
    // a mode switch unregistered after the asynchronous images pane was shown.
    ui.add(egui::RadioButton::new(on, tl!(label)))
}

/// OK: with a password pending, try it (the dialog stays up with the pages on success, with the
/// error otherwise); else open every selected page or image, or stack the selected pages together.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let Some(mut st) = take(app, f) else { return Err("no PDF is waiting to open".into()) };
    let mut fields = f.clone();
    if st.loading {
        fields.insert("__loading".into(), json!(true));
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    if flag(f, "__needsPassword", false) {
        let pw = s(f, "password", "").to_string();
        st.worker.send(PdfTask::Inspect(Some(pw.clone())))?;
        st.password = Some(pw);
        st.loading = true;
        fields.insert("__loading".into(), json!(true));
        fields.remove("__error");
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    if st.info.is_none() {
        fields.insert("__error".into(), json!("The PDF couldn't be read."));
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    let images = s(f, "select", "pages") == "images";
    let key = if images { "images" } else { "pages" };
    let mut selected = picks(f, key);
    if selected.is_empty() {
        let mut fields = f.clone();
        fields.insert("__error".into(), json!(if images { "Select one or more images." } else { "Select one or more pages." }));
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    if selected.len() > MAX_SELECTED_ITEMS {
        fields.insert("__error".into(), json!("You can open at most 500 pages or images at a time. Select fewer items."));
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    if selected_pixels(&st, f, &selected, images).is_none_or(|pixels| pixels > MAX_SELECTED_PIXELS) {
        fields.insert("__error".into(), json!("The selected pages exceed the 128 MP import limit. Select fewer pages or reduce their size."));
        let id = show(app, st, fields);
        return Ok(json!({"dialog": id, "pending": true}));
    }
    let pages_as_layers = !images && flag(f, "pagesAsLayers", false);
    let pages_as_artboards = !images && flag(f, "pagesAsArtboards", false);
    if !images && !pages_as_layers && !pages_as_artboards && flag(f, "reverseOrder", true) {
        selected.reverse();
    }
    let name = Some(s(f, "name", "").trim()).filter(|n| !n.is_empty()).map_or(st.file_name.clone(), str::to_string);
    if pages_as_layers || pages_as_artboards {
        let first_page = selected.first().copied().ok_or_else(|| "Select one or more pages.".to_string())?;
        let mut pdf = options(f, first_page, false, st.password.as_deref());
        if pages_as_artboards {
            pdf.pages_as_artboards = Some(selected);
        } else {
            pdf.pages_as_layers = Some(selected);
        }
        let opts = photocraft_io::ImportOptions { pdf: Some(pdf), ..photocraft_engine::jobs::import_options(app.session.prefs()) };
        return match app.open_bytes_with(&name, &st.bytes, st.path.as_deref(), opts) {
            Ok(_) => Ok(json!({"opened": 1, "failed": 0, "layers": picks(f, "pages").len()})),
            Err(e) => {
                app.open_failed(&st.file_name, &e);
                Ok(json!({"opened": 0, "failed": 1}))
            }
        };
    }
    let mut opened = 0;
    let mut failed = Vec::new();
    let batch = crate::jobs_ui::begin_batch(app);
    for (seq, n) in selected.into_iter().enumerate() {
        let opts = photocraft_io::ImportOptions {
            pdf: Some(options(f, n, images, st.password.as_deref())),
            ..photocraft_engine::jobs::import_options(app.session.prefs())
        };
        let pending = app.jobs.opens.len();
        let order = crate::jobs_ui::OpenOrder { batch, seq };
        match app.open_bytes_with(&name, &st.bytes, st.path.as_deref(), opts) {
            Ok(_) => {
                opened += 1;
                match app.jobs.opens.get_mut(pending) {
                    Some(tab) => tab.order = Some(order),
                    None => crate::jobs_ui::opened_in_order(&mut app.jobs, order),
                }
            }
            Err(e) => failed.push(e),
        }
    }
    crate::jobs_ui::end_batch(app);
    if let Some(e) = failed.first() {
        app.open_failed(&st.file_name, e);
    }
    Ok(json!({"opened": opened, "failed": failed.len()}))
}

#[cfg(test)]
mod tests;
