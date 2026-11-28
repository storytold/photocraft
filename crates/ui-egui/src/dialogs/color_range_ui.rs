//! Select › Color Range… on the framework's `HostedColorRange`.
//!
//! The dialog, its sampling and its Selection / Image views are the framework's. This module is the
//! app side: it opens the dialog, maps its value to the `select.colorRange` parameters, builds the
//! preview pictures from a small proxy of the document, and runs the command at OK.
//!
//! The preview and OK run the same engine command, so the GUI, MCP and scripts select identically;
//! the preview compares a small proxy against samples from the original document; OK selects at
//! full resolution (one history step). Reduced previews approximate fine detail and effects.
//! Cancel never touches the document, so the previous selection stays as it was.

use photocraft_engine::color_cmds::ColorExt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::behavior::PixelSize;
use craft::imaging::selection::PreviewMode;
use craft::imaging::trace::{MaskRect, MaskSource};
use craft::imaging::ui::color_range::{ColorRangeSelect, ColorRangeValue, HostedColorRange};
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::primitives::Revision;
use egui::Color32;
use photocraft_doc::{DocId, Document};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;

/// A rendered preview picture the framework dialog shows as a raster: one fixed image per revision.
pub struct SharedPicture {
    image: egui::ColorImage,
    revision: Revision,
}

impl SharedPicture {
    /// A picture of `image`; `revision` changes whenever the pixels do.
    pub fn new(image: egui::ColorImage, revision: u64) -> Self {
        Self { image, revision: Revision::new(revision) }
    }
}

impl RasterSource for SharedPicture {
    fn size(&self) -> PixelSize {
        PixelSize::new(self.image.size[0] as u32, self.image.size[1] as u32)
    }

    fn revision(&self) -> Revision {
        self.revision
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        (since == self.revision).then(PixelRect::default)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut egui::ColorImage) {
        let [w, h] = self.image.size;
        let factor = factor.max(1) as i32;
        let columns = out.size[0].max(1);
        for (i, pixel) in out.pixels.iter_mut().enumerate() {
            let x = region.x0 + (i % columns) as i32 * factor;
            let y = region.y0 + (i / columns) as i32 * factor;
            *pixel = if x < 0 || y < 0 || x as usize >= w || y as usize >= h { Color32::TRANSPARENT } else { self.image.pixels[y as usize * w + x as usize] };
        }
    }
}

/// The engine command the dialog drives.
pub const COMMAND: &str = "select.colorRange";

/// Longest side of the preview thumbnail, in points.
const PREVIEW: u32 = 200;
/// Canvas overlays are view state; keep their allocation bounded on very large documents.
const CANVAS_PREVIEW: u32 = 1024;

/// The proxy's coverage as a mask the framework's `SelectionOverlay` draws and traces.
pub struct ProxyMask {
    size: [u32; 2],
    revision: u64,
    coverage: Arc<[f32]>,
}

impl MaskSource for ProxyMask {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn content_bounds(&self) -> MaskRect {
        MaskRect::of_size(self.size)
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        let [w, h] = self.size;
        for (i, value) in out.iter_mut().enumerate() {
            let x = x0 + i as i32 * step as i32;
            *value = if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 { 0.0 } else { self.coverage[y as usize * w as usize + x as usize] };
        }
    }
}

/// What the canvas shows over the document while the dialog is open: Selection Preview on the image.
pub struct SelectionPreview {
    /// The proxy's coverage.
    pub mask: Arc<ProxyMask>,
    /// The dialog's Selection Preview mode (never `None`: that shows nothing).
    pub mode: PreviewMode,
    /// Document pixels per proxy pixel.
    pub reduction: u32,
}

/// Open the dialog with Photoshop's defaults (the value's `Default`). `Err` without a document: the
/// dialog's sample points are clamped to the document's size.
pub fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let label = crate::dialogs::filter_dialog::title_of(photocraft_engine::commands::find(COMMAND).map_or(tl!("Color Range…"), |c| c.label));
    let idx = app.session.active_index().ok_or("no document")?;
    let size = app.session.active().map(|st| [st.doc.size.width, st.doc.size.height]).ok_or("no document")?;
    app.color_range = None;
    let dialog = HostedColorRange::new(label, ColorRangeValue::default(), size).canvas(CanvasHost::primary(app, idx).canvas_id());
    Ok(app.framework.dialogs.open(dialog))
}

/// The engine's id of the value's Select entry (`"sampledColors"`, `"reds"`, …): the value's own serialization.
fn select_id(select: &ColorRangeSelect) -> String {
    serde_json::to_value(select).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "sampledColors".into())
}

/// The `select.colorRange` params for the dialog's value: only what the Select mode uses.
pub fn params(value: &ColorRangeValue) -> Value {
    let select = select_id(&value.select);
    let c = value.controls();
    let mut p = Map::new();
    p.insert("select".into(), json!(select));
    p.insert("invert".into(), json!(value.invert));
    if c.sampling {
        p.insert("fuzziness".into(), json!(value.fuzziness));
        let (add, sub) = (&value.samples.add, &value.samples.subtract);
        let picked = !add.is_empty() || !sub.is_empty();
        if !add.is_empty() {
            p.insert("points".into(), json!(add));
        }
        if !sub.is_empty() {
            p.insert("subtractPoints".into(), json!(sub));
            if let Some(order) = value.samples.order() {
                p.insert("order".into(), json!(order));
            }
        }
        // Localized clusters need a picked position; until then it is the plain colour range.
        if c.range && picked {
            p.insert("localized".into(), json!(true));
            p.insert("range".into(), json!(value.range));
        }
    } else if matches!(value.select, ColorRangeSelect::SkinTones) {
        p.insert("fuzziness".into(), json!(value.fuzziness));
    } else if c.tonal {
        p.insert("fuzziness".into(), json!(value.tone_fuzziness));
        let range = match value.select {
            ColorRangeSelect::Shadows => json!(value.shadows_level),
            ColorRangeSelect::Highlights => json!(value.highlights_level),
            _ => {
                let (lo, hi) = (value.midtones_low, value.midtones_high);
                json!([lo.min(hi), lo.max(hi)])
            }
        };
        p.insert("tonalRange".into(), range);
    }
    Value::Object(p)
}

/// What the canvas shows of the open Color Range dialog's Selection Preview, for the document area's version
/// (`region_versions::canvas`): the preview mode and the fields, which change with no command.
pub(crate) fn canvas_key(app: &PhotocraftApp) -> Option<(PreviewMode, String)> {
    let dialog = app.framework.dialogs.get::<HostedColorRange>()?;
    Some((dialog.preview_mode(), params(dialog.value()).to_string()))
}

/// Cached preview: the document proxy (per revision) and the pictures drawn from it.
pub struct Preview {
    doc: DocId,
    revision: u64,
    /// Proxy scale: proxy pixel = `k` document pixels.
    k: u32,
    proxy: Arc<Document>,
    /// The proxy's thumbnail, the dialog's Image view.
    image: Option<Arc<SharedPicture>>,
    /// Why the thumbnail couldn't be made.
    image_error: Option<String>,
    /// Hash of the params the mask picture shows.
    mask_key: u64,
    mask: Option<Arc<SharedPicture>>,
    /// Why the preview couldn't be drawn (shown in the dialog and logged, never silent: #145).
    error: Option<String>,
    canvas_proxy: Option<(u32, Arc<Document>)>,
    canvas_mask: Option<Arc<ProxyMask>>,
}

fn query_key(app: &PhotocraftApp, params: &Value) -> u64 {
    let mut key = std::collections::hash_map::DefaultHasher::new();
    params.to_string().hash(&mut key);
    let foreground = app.session.tools.foreground;
    foreground.map(f32::to_bits).hash(&mut key);
    // This runs several times per frame, including mere pointer motion. Debug-formatting a
    // ProofView serializes the whole ICC profile/LUT; proof() also clones the default profile.
    // Borrow the state and use the profile's memoized content hash instead.
    let proof = app.session.active().and_then(|st| app.session.color().proof_ref(st.doc.id));
    proof.is_some().hash(&mut key);
    if let Some(proof) = proof {
        let setup = &proof.setup;
        (setup.profile.content_hash(), setup.intent, setup.bpc, setup.simulate_paper, setup.kind).hash(&mut key);
        (proof.enabled, proof.gamut_warning, proof.gamut_threshold.to_bits()).hash(&mut key);
    }
    key.finish().max(1)
}

/// Keep the accelerated preview paths in sync with the engine's Color Range replacement.
/// The full-resolution engine handles other combine modes itself.
fn scope_preview(proxy: &Document, params: &Value, mask: &mut [f32]) {
    if matches!(params.get("mode").and_then(Value::as_str), Some("add" | "subtract" | "intersect")) {
        return;
    }
    if let Some(selection) = proxy.selection.as_ref() {
        let prior = photocraft_algo::selection::mask_from_surface(Some(selection), proxy.bounds());
        for (value, bound) in mask.iter_mut().zip(prior) {
            *value = value.min(bound);
        }
    }
}

/// The selection mask `params` would make, on the proxy: the engine command run on a scratch
/// session (Out Of Gamut uses the document's own proof setup).
fn proxy_mask(app: &PhotocraftApp, proxy: &Document, k: u32, params: &Value) -> Result<Vec<f32>, String> {
    let area = proxy.bounds();
    if params.get("select").and_then(Value::as_str).unwrap_or("sampledColors") == "sampledColors" {
        // Pick on the original document, not the thumbnail's nearest retained pixel. The
        // prepared Lab query is the same one the command uses at full resolution.
        let query = photocraft_engine::selection_cmds::ColorRangeSamples::new(&app.session, params).map_err(|e| e.to_string())?;
        // Like the command (and OK): `sampleAllLayers: false` judges the active layer's own
        // pixels, otherwise the composite.
        let all_layers = params.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(true);
        let layer = app.session.active().and_then(|d| d.active_layer).and_then(|id| proxy.layer(id)).and_then(|l| l.surface());
        let px = match (all_layers, layer) {
            (false, Some(surf)) => {
                let mut px = vec![[0.0f32; 4]; area.width() as usize * area.height() as usize];
                surf.read_rgba_into(area, &mut px);
                px
            }
            _ => photocraft_compose::render(proxy, area).into_px(),
        };
        let invert = params.get("invert").and_then(Value::as_bool).unwrap_or(false);
        let mut mask: Vec<f32> = query
            .coverage(&px, area.width() as usize, k as f32, [0.0, 0.0])
            .into_iter()
            .map(|v| {
                let v = if invert { 1.0 - v } else { v };
                // Match the command's GRAY8 mask rounding, before display interpolation.
                (v.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0
            })
            .collect();
        scope_preview(proxy, params, &mut mask);
        return Ok(mask);
    }
    if params.get("select").and_then(Value::as_str) == Some("outOfGamut") {
        let pv = app.session.color().proof(proxy.id);
        let (m, _) = photocraft_engine::color_cmds::gamut_mask(proxy, &pv.setup, pv.gamut_threshold).map_err(|e| e.to_string())?;
        let invert = params.get("invert").and_then(Value::as_bool).unwrap_or(false);
        let mut mask: Vec<f32> = m.iter().map(|v| f32::from(*v) / 255.0).map(|v| if invert { 1.0 - v } else { v }).collect();
        scope_preview(proxy, params, &mut mask);
        return Ok(mask);
    }
    let mut p = params.clone();
    // Eyedropper points are in document pixels; the proxy is 1/k the size.
    let (w, h) = (f64::from(area.width().max(1)), f64::from(area.height().max(1)));
    if let Some(o) = p.as_object_mut() {
        for key in ["points", "subtractPoints"] {
            if let Some(Value::Array(a)) = o.get_mut(key) {
                for v in a.iter_mut() {
                    if let Some([x, y]) = v.as_array().map(Vec::as_slice).and_then(|s| match s {
                        [x, y] => Some([x.as_f64()?, y.as_f64()?]),
                        _ => None,
                    }) {
                        *v = json!([(x / f64::from(k)).floor().clamp(0.0, w - 1.0), (y / f64::from(k)).floor().clamp(0.0, h - 1.0)]);
                    }
                }
            }
        }
    }
    let mut s = photocraft_engine::Session::new();
    s.tools = app.session.tools.clone();
    // Preserve the proxy's pre-existing selection so preview and OK use the same scope.
    s.add_document(proxy.clone(), None);
    if let Some(id) = app.session.active().and_then(|d| d.active_layer) {
        let _ = s.select_layer(id);
    }
    s.execute(COMMAND, p).map_err(|e| e.to_string())?;
    let d = s.active().ok_or("no preview document")?;
    Ok(photocraft_algo::selection::mask_from_surface(d.doc.selection.as_ref(), area))
}

/// Record why the preview failed (logged once per new reason), or clear it. A thumbnail failure stays shown.
fn report(app: &mut PhotocraftApp, error: Option<String>) {
    let Some(p) = app.color_range.as_mut() else { return };
    let error = error.or_else(|| p.image_error.clone());
    if p.error != error {
        if let Some(e) = &error {
            log::warn!("Color Range preview: {e}");
        }
        p.error = error;
    }
}

/// The proxy's thumbnail as the dialog's Image view shows it.
fn thumbnail(app: &PhotocraftApp, proxy: &Document) -> Result<Arc<SharedPicture>, String> {
    let (w, h) = (proxy.size.width as usize, proxy.size.height as usize);
    let buf = photocraft_compose::thumbnail_buffer(proxy, proxy.size.width.max(proxy.size.height));
    let thumb = app.session.color().canvas_display(proxy).map_err(|e| e.to_string())?.to_rgba8(&buf);
    let size = [thumb.width as usize, thumb.height as usize];
    if size != [w, h] || thumb.pixels.len() != w * h * 4 {
        return Err(format!("the image thumbnail is {}×{}, expected {w}×{h}", thumb.width, thumb.height));
    }
    Ok(Arc::new(SharedPicture::new(egui::ColorImage::from_rgba_unmultiplied(size, &thumb.pixels), 1)))
}

/// Makes the cached proxy current for the active document and its revision. `None` without a document.
fn ensure_proxy(app: &mut PhotocraftApp) -> Option<()> {
    let (doc_id, revision, doc) = {
        let st = app.session.active()?;
        (st.doc.id, st.revision, st.doc.clone())
    };
    if matches!(&app.color_range, Some(p) if p.doc == doc_id && p.revision == revision) {
        return Some(());
    }
    let side = doc.size.width.max(doc.size.height).max(1);
    let k = side.div_ceil(PREVIEW).max(1);
    let proxy = Arc::new(crate::proxy::proxy_document(&doc, k));
    let (image, image_error) = match thumbnail(app, &proxy) {
        Ok(picture) => (Some(picture), None),
        Err(e) => {
            log::warn!("Color Range preview: {e}");
            (None, Some(e))
        }
    };
    app.color_range = Some(Preview {
        doc: doc_id,
        revision,
        k,
        proxy,
        image,
        error: image_error.clone(),
        image_error,
        mask_key: 0,
        mask: None,
        canvas_proxy: None,
        canvas_mask: None,
    });
    Some(())
}

/// Rebuilds the dialog's mask picture when `params` differ from the cached ones and hands the pictures to the dialog.
fn refresh(app: &mut PhotocraftApp, params: &Value) {
    if ensure_proxy(app).is_none() {
        return;
    }
    let key = query_key(app, params);
    let Some((proxy, k)) = app.color_range.as_ref().filter(|p| p.mask_key != key || p.mask.is_none()).map(|p| (p.proxy.clone(), p.k)) else { return };
    let (w, h) = (proxy.size.width as usize, proxy.size.height as usize);
    let mask = match proxy_mask(app, &proxy, k, params) {
        Ok(m) => {
            report(app, None);
            m
        }
        Err(e) => {
            report(app, Some(e));
            vec![0.0; w * h]
        }
    };
    if mask.len() != w * h {
        report(app, Some(format!("the selection preview has {} pixels, expected {}", mask.len(), w * h)));
        return;
    }
    let px: Vec<Color32> = mask.iter().map(|v| Color32::from_gray((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect();
    let picture = Arc::new(SharedPicture::new(egui::ColorImage::new([w, h], px), key));
    let Some(p) = app.color_range.as_mut() else { return };
    p.mask = Some(picture);
    p.mask_key = key;
    if let Some(dialog) = app.framework.dialogs.get_mut::<HostedColorRange>() {
        dialog.set_preview(p.mask.clone().map(|m| m as _), p.image.clone().map(|i| i as _), p.k, p.error.clone().map(Into::into));
    }
}

/// The dialog's turn each pass (a `TAKERS` entry): runs OK, drops the cache on Cancel, and while the dialog is open
/// keeps its preview pictures current. Rebuilds nothing while the params, document and proof setup are unchanged.
pub fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    match app.framework.dialogs.take::<HostedColorRange>() {
        Some(DialogResult::Ok(value)) => {
            app.color_range = None;
            if let Err(e) = app.run(COMMAND, params(&value)) {
                log::warn!("Color Range: {e}");
            }
            return;
        }
        Some(DialogResult::Cancel) => {
            app.color_range = None;
            return;
        }
        Some(_) | None => {}
    }
    let Some(dialog) = app.framework.dialogs.get::<HostedColorRange>() else {
        app.color_range = None;
        return;
    };
    let params = params(dialog.value());
    refresh(app, &params);
}

/// The Selection Preview the canvas shows over the active document, `None` while the dialog is closed or its
/// Selection Preview mode is None. The canvas proxy's mask is rebuilt only when the params, document or proof setup
/// change.
pub fn selection_preview(app: &mut PhotocraftApp, _ctx: &egui::Context) -> Option<SelectionPreview> {
    let dialog = app.framework.dialogs.get::<HostedColorRange>()?;
    let mode = dialog.preview_mode();
    if matches!(mode, PreviewMode::None) {
        return None;
    }
    let params = params(dialog.value());
    ensure_proxy(app)?;
    if app.color_range.as_ref()?.canvas_proxy.is_none() {
        let original = app.session.active()?.doc.clone();
        let k = original.size.width.max(original.size.height).max(1).div_ceil(CANVAS_PREVIEW);
        let proxy = Arc::new(crate::proxy::proxy_document(&original, k));
        app.color_range.as_mut()?.canvas_proxy = Some((k, proxy));
    }
    let key = query_key(app, &params);
    let (k, proxy) = app.color_range.as_ref()?.canvas_proxy.clone()?;
    if app.color_range.as_ref()?.canvas_mask.as_ref().is_none_or(|m| m.revision != key) {
        let coverage = match proxy_mask(app, &proxy, k, &params) {
            Ok(mask) => mask,
            Err(e) => {
                report(app, Some(e));
                return None;
            }
        };
        let size = [proxy.size.width, proxy.size.height];
        if coverage.len() != size[0] as usize * size[1] as usize {
            report(app, Some(format!("the canvas preview has {} pixels, expected {}×{}", coverage.len(), size[0], size[1])));
            return None;
        }
        app.color_range.as_mut()?.canvas_mask = Some(Arc::new(ProxyMask { size, revision: key, coverage: coverage.into() }));
    }
    Some(SelectionPreview { mask: app.color_range.as_ref()?.canvas_mask.clone()?, mode, reduction: k })
}

#[cfg(test)]
mod tests;
