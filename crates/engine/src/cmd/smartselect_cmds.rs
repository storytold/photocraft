//! Smart selection commands: Quick Selection, Object Selection, Select Subject, Select and Mask
//! (Refine Edge) and Focus Area. The algorithms live in `photocraft_algo::segment` and
//! `photocraft_algo::matting`; these commands pick the pixels to analyse (active layer or
//! composite), run them, and store the result as the selection (or a mask / new layer).

use photocraft_algo::matting::{self, RefineParams};
use photocraft_algo::segment::{Sampler, SurfaceSampler, focus, grabcut, quick, subject};
use photocraft_algo::selection::{Region, SelectionMode, combine_region};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_plugins::adapter::{self, Image, Limits, OutputKind, Request};
use serde_json::{Value, json};

use crate::adapter_cmds::adapter_error;
use crate::commands::CommandSpec;
use crate::select_mask_edits;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_selection_or_mask(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or_else(|| "no document open".to_string())?;
    if d.doc.selection.is_some() {
        return Ok(());
    }
    if let Some(l) = d.active_layer.and_then(|id| d.doc.layer(id))
        && l.mask.is_some()
    {
        return Ok(());
    }
    Err("no selection or layer mask".into())
}

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn mode(p: &Value, default: &str) -> SelectionMode {
    let m = p.get("mode").and_then(Value::as_str).unwrap_or(default);
    SelectionMode::parse(if m == "new" { "replace" } else { m })
}
fn bad(cmd: &str, msg: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Composite of all visible layers.
struct CompositeSampler<'a>(&'a Document);

impl Sampler for CompositeSampler<'_> {
    fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
        photocraft_compose::render(self.0, r).px
    }
}

/// Runs `f` with a sampler over the active layer (or the composite with `all_layers`, or when the
/// active layer has no pixels).
fn with_sampler<R>(s: &Session, all_layers: bool, f: impl FnOnce(&dyn Sampler, &Document) -> R) -> Result<R> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    Ok(with_doc_sampler(&d.doc, d.active_layer, all_layers, f))
}

/// Runs `f` with a sampler over `layer`'s pixels in `doc` (or the composite with `all_layers`, or
/// when the layer has no pixels).
pub(crate) fn with_doc_sampler<R>(doc: &Document, layer: Option<LayerId>, all_layers: bool, f: impl FnOnce(&dyn Sampler, &Document) -> R) -> R {
    match (all_layers, layer.and_then(|id| doc.layer(id)).and_then(Layer::surface)) {
        (false, Some(surf)) => f(&SurfaceSampler(surf), doc),
        _ => f(&CompositeSampler(doc), doc),
    }
}

/// Stores `region` combined with the current selection by `m` as one history step.
/// Finding nothing never destroys work: the selection (and history) is left alone and the
/// caller gets `changed: false`, whatever the mode. (Select Subject and Object Selection run
/// heuristics that often find nothing on busy photos; clearing the selection then looks like
/// a dead button.)
fn apply(s: &mut Session, label: &str, region: Option<&Region>, m: SelectionMode) -> Result<Value> {
    if region.is_none() {
        // Nothing found: leave the selection (and history) alone.
        let selected = s.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some();
        return Ok(json!({ "selected": selected, "changed": false }));
    }
    let bbox = region.map(|r| r.bbox);
    let selected = s.edit(label, |doc, _| {
        doc.selection = combine_region(doc.selection.as_ref(), region, m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected, "changed": true, "bounds": bbox.map(|r| [r.x0, r.y0, r.width() as i32, r.height() as i32]) }))
}

fn points(p: &Value) -> Vec<(f32, f32)> {
    p.get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|q| Some((q.get(0)?.as_f64()? as f32, q.get(1)?.as_f64()? as f32))).collect())
        .unwrap_or_default()
}

/// Auto-Enhance for Quick Selection: a light edge refinement that flows the edge towards image
/// edges and removes blockiness.
const ENHANCE: RefineParams = RefineParams { radius: 4.0, smart_radius: true, smooth: 15.0, feather: 0.0, contrast: 25.0, shift_edge: 0.0 };

fn quick_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = points(p);
    if pts.is_empty() {
        return Err(bad("select.quick", "needs at least one point"));
    }
    let size = f(p, "size", 30.0).clamp(1.0, 5000.0);
    let m = match p.get("mode").and_then(Value::as_str).unwrap_or("add") {
        "subtract" => SelectionMode::Subtract,
        "replace" | "new" => SelectionMode::Replace,
        _ => SelectionMode::Add,
    };
    let enhance = b(p, "enhanceEdge", false);
    let region = with_sampler(s, b(p, "sampleAllLayers", false), |smp, doc| {
        let canvas = doc.bounds();
        let r = quick::quick_select(smp, canvas, &pts, size)?;
        if enhance { matting::refine_mask(smp, &matting::region_reader(&r), r.bbox, canvas, &ENHANCE) } else { Some(r) }
    })?;
    apply(s, "Quick Selection", region.as_ref(), m)
}

fn rect_param(p: &Value) -> Option<Rect> {
    let a = p.get("rect")?.as_array()?;
    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
    if v.len() < 4 {
        return None;
    }
    // Negative sizes (dragging up/left) are normalised.
    let (x0, x1) = (v[0].min(v[0] + v[2]), v[0].max(v[0] + v[2]));
    let (y0, y1) = (v[1].min(v[1] + v[3]), v[1].max(v[1] + v[3]));
    Some(Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32))
}

fn point_param(p: &Value) -> Option<(f32, f32)> {
    let a = p.get("point")?.as_array()?;
    let (x, y) = (a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// The layer [`with_doc_sampler`] samples for `layer` / `all_layers`: `None` = the composite.
pub fn sampled_target(doc: &Document, layer: Option<LayerId>, all_layers: bool) -> Option<LayerId> {
    layer.filter(|id| !all_layers && doc.layer(*id).and_then(Layer::surface).is_some())
}

/// Object Finder (#4087): the object Object Selection would select at document point `(x, y)`,
/// sampling `layer` (or the composite). Pure, so the UI can run it on a worker thread while the
/// pointer hovers; `select.object` with `"point"` commits the same result.
pub fn find_object(doc: &Document, layer: Option<LayerId>, all_layers: bool, x: f32, y: f32) -> Option<Region> {
    with_doc_sampler(doc, layer, all_layers, |smp, doc| grabcut::object_at(smp, doc.bounds(), x, y, grabcut::OBJECT_AT_WORK_PX))
}

/// Most objects remembered by [`ObjectFinderCache`].
pub const OBJECT_FINDER_ENTRIES: usize = 12;

/// Object Finder results for one document's pixels (#4087): hovering over an object already
/// found, or clicking it, answers at once. Keyed like [`SubjectCache`] (valid while the pixels
/// are the same, the document held weakly) plus the sampled layer; a lookup hits any cached
/// object that covers the point, so one computation serves the whole object.
#[derive(Debug, Default)]
pub struct ObjectFinderCache {
    doc: std::sync::Weak<Document>,
    doc_id: Option<photocraft_doc::DocId>,
    target: Option<LayerId>,
    regions: Vec<std::sync::Arc<Region>>,
}

impl ObjectFinderCache {
    fn valid(&self, doc: &Document, target: Option<LayerId>) -> bool {
        self.doc_id == Some(doc.id) && self.target == target && self.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, doc))
    }

    /// A cached object of `doc` (sampling `target`) covering `(x, y)`.
    pub fn lookup(&self, doc: &Document, target: Option<LayerId>, x: f32, y: f32) -> Option<std::sync::Arc<Region>> {
        if !x.is_finite() || !y.is_finite() || !self.valid(doc, target) {
            return None;
        }
        let (px, py) = (x.floor() as i32, y.floor() as i32);
        self.regions.iter().find(|r| r.at(px, py) >= 0.5).cloned()
    }

    /// Remembers `region`, found in `doc` sampling `target`. A different document or pixels
    /// start over; the oldest object is dropped beyond [`OBJECT_FINDER_ENTRIES`].
    pub fn insert(&mut self, doc: &std::sync::Arc<Document>, target: Option<LayerId>, region: std::sync::Arc<Region>) {
        if !self.valid(doc, target) {
            self.regions.clear();
        }
        self.doc = std::sync::Arc::downgrade(doc);
        self.doc_id = Some(doc.id);
        self.target = target;
        if self.regions.len() >= OBJECT_FINDER_ENTRIES {
            self.regions.remove(0);
        }
        self.regions.push(region);
    }

    /// Number of remembered objects.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Forget the results if they were computed from the document `id` (it is closing).
    pub(crate) fn forget_doc(&mut self, id: photocraft_doc::DocId) {
        if self.doc_id == Some(id) {
            *self = Self::default();
        }
    }

    /// Follows `doc` past a selection-only edit (same pixels), so the next lookup still hits.
    fn follow(&mut self, doc: &std::sync::Arc<Document>) {
        if self.doc_id == Some(doc.id) && self.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, doc)) {
            self.doc = std::sync::Arc::downgrade(doc);
        }
    }
}

/// Object Selection samples only the active layer by default: when that layer has no pixels in
/// `area`, say so instead of quietly finding nothing (#5246; scripts often add an empty layer
/// above the photo first).
fn check_sampled_pixels(doc: &Document, layer: Option<LayerId>, all_layers: bool, area: Rect) -> Result<()> {
    match empty_sampled_area(doc, layer, all_layers, area) {
        Some(msg) => Err(bad("select.object", &format!("{msg}; set \"sampleAllLayers\": true or select a layer with pixels"))),
        None => Ok(()),
    }
}

/// Why sampling `layer` (unless `all_layers`) can find nothing in `area`: the layer is empty, or has no pixels there.
/// `None` = there is something to sample. The click path reports it as an error, the Object Finder hover as a hint.
pub fn empty_sampled_area(doc: &Document, layer: Option<LayerId>, all_layers: bool, area: Rect) -> Option<&'static str> {
    let surf = sampled_target(doc, layer, all_layers).and_then(|id| doc.layer(id)).and_then(Layer::surface)?;
    if !surf.content_bounds().intersect(&area).is_empty() {
        return None;
    }
    Some(if surf.content_bounds().is_empty() { "the active layer is empty" } else { "the active layer has no pixels in that area" })
}

/// `select.object` with `"point"`: the Object Finder's object under the point (cached).
fn object_at_point(s: &mut Session, p: &Value, (x, y): (f32, f32)) -> Result<Value> {
    let all_layers = b(p, "sampleAllLayers", false);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    check_sampled_pixels(&d.doc, d.active_layer, all_layers, Rect::from_xywh(x.floor() as i32, y.floor() as i32, 1, 1))?;
    let target = sampled_target(&d.doc, d.active_layer, all_layers);
    let region = match s.object_finder.lookup(&d.doc, target, x, y) {
        Some(r) => Some(r),
        None => {
            let doc = std::sync::Arc::clone(&d.doc);
            let found = find_object(&doc, d.active_layer, all_layers, x, y).map(std::sync::Arc::new);
            if let Some(r) = &found {
                s.object_finder.insert(&doc, target, std::sync::Arc::clone(r));
            }
            found
        }
    };
    let out = apply(s, "Object Selection", region.as_deref(), mode(p, "replace"))?;
    if let Some(d) = s.active() {
        let doc = std::sync::Arc::clone(&d.doc);
        s.object_finder.follow(&doc);
    }
    Ok(out)
}

fn object_selection(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("rect").is_none()
        && let Some(pt) = point_param(p)
    {
        return object_at_point(s, p, pt);
    }
    let rect = rect_param(p).ok_or_else(|| bad("select.object", "needs \"rect\": [x, y, w, h] or \"point\": [x, y]"))?;
    let all_layers = b(p, "sampleAllLayers", false);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    check_sampled_pixels(&d.doc, d.active_layer, all_layers, rect)?;
    let region = with_sampler(s, all_layers, |smp, doc| grabcut::object_select(smp, doc.bounds(), rect, 160_000))?;
    apply(s, "Object Selection", region.as_ref(), mode(p, "replace"))
}

/// Select Subject's last result (#2865), so running it again on unchanged pixels answers at
/// once instead of recomputing superpixels, saliency and GrabCut (P29). Keyed by the pixels it
/// was computed from: valid while [`same_pixels`](crate::layer_multi_cmds::same_pixels) (a
/// tile-pointer comparison) holds against that document, so selection-only edits such as
/// Deselect keep it. One entry per session; the document is held weakly, so the cache never
/// keeps a closed or edited document's tiles alive (the session also drops it on pixel edits
/// and when its document closes). Its memory is the mask: one byte per pixel of its bounds.
/// Only classical results are cached: adapter output can depend on the selection seed, opaque
/// parameters/resources, device and registration identity, not merely the document's pixels.
#[derive(Debug, Default)]
pub struct SubjectCache {
    doc: std::sync::Weak<Document>,
    doc_id: Option<photocraft_doc::DocId>,
    /// The sampled layer; `None` = the composite.
    target: Option<LayerId>,
    /// Classical refinement changes the result even when the sampled pixels do not.
    enhance_edge: bool,
    region: Option<std::sync::Arc<Region>>,
}

impl SubjectCache {
    /// Forget the result if it was computed from the document `id` (it is closing).
    pub(crate) fn forget_doc(&mut self, id: photocraft_doc::DocId) {
        if self.doc_id == Some(id) {
            *self = Self::default();
        }
    }
}

const SUBJECT: &str = "select.subject";
const SUBJECT_OPERATION: &str = "select-subject";

struct SubjectParams {
    all_layers: bool,
    mode: SelectionMode,
    enhance_edge: bool,
    force_classical: bool,
    provider: Option<String>,
    device: Option<String>,
    resource: Option<String>,
    params: Value,
    allow_fallback: bool,
}

impl SubjectParams {
    fn parse(p: &Value) -> Result<Self> {
        let map = p.as_object().ok_or_else(|| bad(SUBJECT, "params must be an object"))?;
        for key in map.keys() {
            if !matches!(
                key.as_str(),
                "sampleAllLayers"
                    | "mode"
                    | "enhanceEdge"
                    | "forceClassical"
                    | "method"
                    | "provider"
                    | "device"
                    | "resource"
                    | "params"
                    | "allowFallback"
                    | "target"
                    | "coalesce"
            ) {
                let preview: String = key.chars().take(128).collect();
                return Err(bad(SUBJECT, &format!("unknown parameter `{preview}`; adapter parameters belong in `params`")));
            }
        }
        let boolean = |key: &str, default: bool| -> Result<bool> {
            map.get(key).map_or(Ok(default), |v| v.as_bool().ok_or_else(|| bad(SUBJECT, &format!("`{key}` must be a boolean"))))
        };
        let text = |key: &str, max: usize, id: bool| -> Result<Option<String>> {
            map.get(key)
                .map(|v| {
                    let value = v
                        .as_str()
                        .filter(|v| v.len() <= max && (!id || (!v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))))
                        .ok_or_else(|| {
                            bad(SUBJECT, &format!("`{key}` must be {}text of at most {max} bytes", if id { "a nonempty ASCII identifier: " } else { "" }))
                        })?;
                    Ok(value.to_string())
                })
                .transpose()
        };
        let mode = match map.get("mode").map(Value::as_str) {
            None | Some(Some("replace" | "new")) => SelectionMode::Replace,
            Some(Some("add")) => SelectionMode::Add,
            Some(Some("subtract")) => SelectionMode::Subtract,
            Some(Some("intersect")) => SelectionMode::Intersect,
            _ => return Err(bad(SUBJECT, "`mode` must be replace|new|add|subtract|intersect")),
        };
        let classical_method = match map.get("method").map(Value::as_str) {
            None | Some(Some("auto")) => false,
            Some(Some("classical")) => true,
            _ => return Err(bad(SUBJECT, "`method` must be auto|classical")),
        };
        if map.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
            return Err(bad(SUBJECT, "unsupported target: Select Subject analyses pixels, not a channel or mask"));
        }
        if map.get("coalesce").is_some_and(|v| v.as_str().is_none_or(|s| s.len() > 1024)) {
            return Err(bad(SUBJECT, "`coalesce` must be a string of at most 1024 bytes"));
        }
        let limits = Limits::default();
        let params = map.get("params");
        if let Some(value) = params {
            // Prove depth/size bounded before cloning any caller-built adapter JSON.
            limits.check_params(value).map_err(|e| bad(SUBJECT, &format!("adapter parameters: {e}")))?;
        }
        Ok(Self {
            all_layers: boolean("sampleAllLayers", true)?,
            mode,
            enhance_edge: boolean("enhanceEdge", false)?,
            force_classical: boolean("forceClassical", false)? || classical_method,
            provider: text("provider", 128, true)?,
            device: text("device", 128, true)?,
            resource: text("resource", limits.max_resource_bytes, false)?,
            params: params.cloned().unwrap_or_else(|| json!({})),
            allow_fallback: boolean("allowFallback", false)?,
        })
    }
}

/// Called by the dispatcher's adapter preflight before it clones caller-built JSON.
pub(crate) fn preflight(id: &str, params: &Value) -> Result<()> {
    if id == SUBJECT && !params.is_null() {
        SubjectParams::parse(params)?;
    }
    Ok(())
}

struct SubjectContext<'a>(&'a crate::jobs::JobCtx);

impl adapter::Context for SubjectContext<'_> {
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }

    fn progress(&self, fraction: f32) {
        self.0.progress(fraction, "");
    }
}

/// Resolve on every invocation, before consulting the classical-only cache. Registry mutations,
/// provider/device/resource/params and the selection seed can all change adapter output, so no
/// adapter result is cached until a complete input/registration identity is available.
fn subject_adapter(s: &Session, p: &SubjectParams) -> Result<Option<std::sync::Arc<adapter::RegisteredAdapter>>> {
    if p.force_classical {
        return Ok(None);
    }
    let registered = match &p.provider {
        Some(id) => Some(s.adapters.get(id).ok_or_else(|| bad(SUBJECT, &format!("adapter `{id}` is not registered")))?),
        None => {
            // Automatic routing must not let an alphabetically earlier, incompatible or
            // unprepared package shadow another usable provider. Consult cached readiness;
            // selecting a subject must never implicitly probe workers or prepare models.
            let input_mode = s
                .active()
                .and_then(|st| subject_format(&st.doc, st.active_layer, p.all_layers).ok())
                .and_then(|format| subject_color_mode(format.mode).ok());
            s.adapters
                .list()
                .iter()
                .find(|d| {
                    #[cfg(not(target_arch = "wasm32"))]
                    if !s.adapter_packages.ready_for_automatic_run(&d.id) {
                        return false;
                    }
                    let op_match = d.operations.iter().any(|op| {
                        op.id == SUBJECT_OPERATION
                            && op.output_kind == OutputKind::Mask
                            && input_mode.as_ref().is_some_and(|mode| op.color_modes.iter().any(|m| m == mode))
                    });
                    let device_match = p.device.as_ref().is_none_or(|id| {
                        d.devices.iter().any(|dev| dev.id == *id && dev.operations.iter().any(|op| op == SUBJECT_OPERATION))
                    });
                    op_match && device_match
                })
                .and_then(|descriptor| s.adapters.get(&descriptor.id))
        }
    };
    let Some(registered) = registered else {
        if p.device.is_some() || p.resource.is_some() || p.allow_fallback || p.params.as_object().is_some_and(|m| !m.is_empty()) {
            return Err(bad(SUBJECT, "adapter options were supplied but no select-subject adapter is registered"));
        }
        return Ok(None);
    };
    let descriptor = registered.descriptor();
    let operation =
        descriptor.operations.iter().find(|op| op.id == SUBJECT_OPERATION).ok_or_else(|| bad(SUBJECT, "adapter does not advertise `select-subject`"))?;
    if operation.output_kind != OutputKind::Mask {
        return Err(bad(SUBJECT, "`select-subject` operation must produce a mask"));
    }
    if let Some(id) = &p.device {
        let device = descriptor.devices.iter().find(|d| d.id == *id).ok_or_else(|| bad(SUBJECT, "adapter does not advertise the requested device"))?;
        if !device.operations.iter().any(|o| o == SUBJECT_OPERATION) {
            return Err(bad(SUBJECT, "requested device does not advertise `select-subject`"));
        }
    }
    Ok(Some(registered))
}

fn subject_color_mode(mode: ColorMode) -> Result<&'static str> {
    match mode {
        ColorMode::Rgb => Ok("rgb"),
        ColorMode::Grayscale => Ok("gray"),
        ColorMode::Cmyk => Ok("cmyk"),
        ColorMode::Lab => Ok("lab"),
        _ => Err(bad(SUBJECT, "adapter subject selection supports native RGB, Gray, CMYK or Lab, not expanded palette/duotone/multichannel samples")),
    }
}

fn subject_format(doc: &Document, layer: Option<LayerId>, all_layers: bool) -> Result<PixelFormat> {
    if let Some(surface) = sampled_target(doc, layer, all_layers).and_then(|id| doc.layer(id)).and_then(Layer::surface) {
        let format = surface.format();
        subject_color_mode(format.mode)?;
        if format.mode != doc.mode || format.sample != doc.depth {
            return Err(bad(SUBJECT, "adapter input requires matching native document/layer color mode and depth"));
        }
        return Ok(format);
    }
    // The compositor emits RGB, not native CMYK/Lab/Gray. Never attach the original non-RGB
    // ICC profile to that RGB buffer or silently sample a different layer instead.
    if doc.mode != ColorMode::Rgb {
        return Err(bad(SUBJECT, "non-RGB adapter composites are unsupported; use sampleAllLayers:false on a native pixel layer or forceClassical:true"));
    }
    Ok(PixelFormat::new(ColorMode::Rgb, doc.depth, true))
}

/// Bound every dense input allocation before reading pixels or constructing the canvas Rect.
fn subject_canvas(doc: &Document, format: PixelFormat, limits: &Limits) -> Result<Rect> {
    let (width, height) = (doc.size.width, doc.size.height);
    let x1 = i32::try_from(width).map_err(|_| bad(SUBJECT, "canvas width exceeds adapter coordinate limits"))?;
    let y1 = i32::try_from(height).map_err(|_| bad(SUBJECT, "canvas height exceeds adapter coordinate limits"))?;
    let channels = u8::try_from(format.channels()).map_err(|_| bad(SUBJECT, "channel count exceeds adapter limits"))?;
    let mut samples = limits.check_image_size(width, height, channels).map_err(adapter_error)?;
    if let Some(selection) = &doc.selection {
        if selection.format().mode != ColorMode::Grayscale || selection.format().alpha || selection.channels() != 1 {
            return Err(bad(SUBJECT, "selection context must be a single grayscale coverage plane"));
        }
        samples = samples
            .checked_add(limits.check_image_size(width, height, 1).map_err(adapter_error)?)
            .ok_or_else(|| bad(SUBJECT, "canvas sample count overflow"))?;
    }
    if samples > limits.max_samples {
        return Err(bad(SUBJECT, "canvas and selection exceed adapter sample limits"));
    }
    if doc.icc_profile.as_ref().is_some_and(|p| p.len() > limits.max_icc_bytes.min(adapter::MAX_ICC_BYTES)) {
        return Err(bad(SUBJECT, "document ICC profile exceeds adapter limits"));
    }
    Ok(Rect::new(0, 0, x1, y1))
}

fn subject_request(doc: &Document, layer: Option<LayerId>, format: PixelFormat, bounds: Rect, p: SubjectParams, ctx: &crate::jobs::JobCtx) -> Result<Request> {
    ctx.check()?;
    let channels = u8::try_from(format.channels()).map_err(|_| bad(SUBJECT, "unsupported channel count"))?;
    let data = if let Some(surface) = sampled_target(doc, layer, p.all_layers).and_then(|id| doc.layer(id)).and_then(Layer::surface) {
        surface.try_read_region(bounds)?
    } else {
        let count = Limits::default().check_image_size(bounds.width(), bounds.height(), channels).map_err(adapter_error)?;
        let mut data = crate::allocation::capacity(count, "subject adapter composite")?;
        // One band of scratch, not an additional full-canvas RGBA allocation. Cancellation is
        // observed between bands as well as inside the adapter.
        photocraft_compose::render_bands(doc, bounds, 256, |band| -> Result<()> {
            ctx.check()?;
            for px in band.px {
                data.extend_from_slice(&px);
            }
            Ok(())
        })?;
        data
    };
    ctx.check()?;
    let mut profile = crate::allocation::capacity(doc.icc_profile.as_ref().map_or(0, |p| p.len()), "subject adapter ICC profile")?;
    if let Some(bytes) = &doc.icc_profile {
        profile.extend_from_slice(bytes);
    }
    let image = Image {
        x: bounds.x0,
        y: bounds.y0,
        width: bounds.width(),
        height: bounds.height(),
        channels,
        color_mode: subject_color_mode(format.mode)?.into(),
        source_depth: format.sample.bits() as u8,
        alpha: format.alpha,
        icc_profile: profile,
        data,
    };
    let selection = doc
        .selection
        .as_ref()
        .map(|surface| -> Result<Image> {
            Ok(Image {
                x: bounds.x0,
                y: bounds.y0,
                width: bounds.width(),
                height: bounds.height(),
                channels: 1,
                color_mode: "gray".into(),
                source_depth: 32,
                alpha: false,
                icc_profile: Vec::new(),
                data: surface.try_read_region(bounds)?,
            })
        })
        .transpose()?;
    ctx.check()?;
    Ok(Request {
        operation: SUBJECT_OPERATION.into(),
        resource: p.resource,
        params: p.params,
        device: p.device,
        allow_fallback: p.allow_fallback,
        image,
        selection,
    })
}

fn enhance_subject(doc: &Document, layer: Option<LayerId>, all_layers: bool, region: Option<Region>, enhance: bool) -> Option<Region> {
    if enhance {
        with_doc_sampler(doc, layer, all_layers, |sampler, _| {
            region.and_then(|r| matting::refine_mask(sampler, &matting::region_reader(&r), r.bbox, doc.bounds(), &ENHANCE))
        })
    } else {
        region
    }
}

fn select_subject_adapter(s: &mut Session, p: SubjectParams, registered: std::sync::Arc<adapter::RegisteredAdapter>) -> Result<Value> {
    // Automatic subject routing must not bypass the same capability gate as an explicit
    // plugin.adapter.run (especially for installed adapters that launch native workers).
    crate::jobs::authorize_render_command(
        crate::adapter_cmds::RUN,
        &json!({
            "provider": registered.descriptor().id, "operation": SUBJECT_OPERATION, "output": "selection",
            "device": p.device, "resource": p.resource, "params": p.params, "allowFallback": p.allow_fallback,
        }),
    )?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = std::sync::Arc::clone(&st.doc);
    let layer = st.active_layer;
    let format = subject_format(&doc, layer, p.all_layers)?;
    let color_mode = subject_color_mode(format.mode)?;
    if !registered.descriptor().operations.iter().any(|op| op.id == SUBJECT_OPERATION && op.color_modes.iter().any(|mode| mode == color_mode)) {
        return Err(bad(SUBJECT, &format!("adapter does not advertise native color mode `{color_mode}` for `select-subject`")));
    }
    let limits = Limits::default();
    let bounds = subject_canvas(&doc, format, &limits)?;
    let worker_doc = std::sync::Arc::clone(&doc);
    let (all_layers, enhance, mode) = (p.all_layers, p.enhance_edge, p.mode);
    let provider = registered.descriptor().id.clone();
    crate::jobs::run(
        s,
        "Select Subject",
        true,
        move |ctx| {
            let request = subject_request(&worker_doc, layer, format, bounds, p, ctx)?;
            let response = registered.run(&request, &SubjectContext(ctx), &limits);
            // Cancellation wins even if the adapter ignores the flag or returns a generic error.
            ctx.check()?;
            let response = response.map_err(adapter_error)?;
            // RegisteredAdapter has validated kind, exact geometry, scalar gray layout and range.
            let mut mask = crate::allocation::capacity(response.output.image.data.len(), "subject adapter mask")?;
            for (index, sample) in response.output.image.data.iter().enumerate() {
                if index % 4096 == 0 {
                    ctx.check()?;
                }
                mask.push((sample * 255.0).round() as u8);
            }
            let region = photocraft_algo::segment::trim_region(Region { bbox: bounds, mask });
            ctx.check()?;
            let region = enhance_subject(&worker_doc, layer, all_layers, region, enhance);
            ctx.check()?;
            Ok((region, response.execution, ctx.clone()))
        },
        move |s, (region, execution, ctx)| {
            ctx.check()?;
            if !std::sync::Arc::ptr_eq(&s.active().ok_or(EngineError::NoDocument)?.doc, &doc) {
                return Err(bad(SUBJECT, "document changed while the adapter ran; result discarded"));
            }
            let mut out = apply(s, "Select Subject", region.as_ref(), mode)?;
            out["provider"] = json!(provider);
            out["execution"] = json!(execution);
            Ok(out)
        },
    )
}

fn select_subject(s: &mut Session, p: &Value) -> Result<Value> {
    let p = SubjectParams::parse(p)?;
    if s.active().ok_or(EngineError::NoDocument)?.floating.is_some() {
        return Err(bad(SUBJECT, "unsupported floating selection: use select.drop or edit.undo before Select Subject"));
    }
    if let Some(registered) = subject_adapter(s, &p)? {
        return select_subject_adapter(s, p, registered);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let target = sampled_target(&d.doc, d.active_layer, p.all_layers);
    let c = &s.subject_cache;
    let hit = c.doc_id == Some(d.doc.id)
        && c.target == target
        && c.enhance_edge == p.enhance_edge
        && c.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, &d.doc));
    let region = if hit {
        c.region.clone()
    } else {
        let doc = std::sync::Arc::clone(&d.doc);
        let region = with_doc_sampler(&doc, d.active_layer, p.all_layers, |smp, doc| subject::select_subject(smp, doc.bounds()));
        let region = enhance_subject(&doc, d.active_layer, p.all_layers, region, p.enhance_edge).map(std::sync::Arc::new);
        s.subject_cache =
            SubjectCache { doc: std::sync::Arc::downgrade(&doc), doc_id: Some(doc.id), target, enhance_edge: p.enhance_edge, region: region.clone() };
        region
    };
    let out = apply(s, "Select Subject", region.as_deref(), p.mode)?;
    // Follow the document past selection-only edits so a short history does not evict a hot
    // classical result. Adapter invocations never read or populate this cache.
    if let Some(d) = s.active()
        && s.subject_cache.doc_id == Some(d.doc.id)
        && s.subject_cache.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, &d.doc))
    {
        s.subject_cache.doc = std::sync::Arc::downgrade(&d.doc);
    }
    Ok(out)
}

fn focus_area(s: &mut Session, p: &Value) -> Result<Value> {
    let (range, noise) = (f(p, "range", 0.5).clamp(0.0, 1.0), f(p, "noise", 0.0).clamp(0.0, 1.0));
    let region = with_sampler(s, b(p, "sampleAllLayers", true), |smp, doc| focus::focus_area(smp, doc.bounds(), range, noise))?;
    apply(s, "Focus Area", region.as_ref(), mode(p, "replace"))
}

fn refine_params(p: &Value) -> RefineParams {
    RefineParams {
        radius: f(p, "radius", 0.0).clamp(0.0, 250.0),
        smart_radius: b(p, "smartRadius", false),
        smooth: f(p, "smooth", 0.0).clamp(0.0, 100.0),
        feather: f(p, "feather", 0.0).clamp(0.0, 1000.0),
        contrast: f(p, "contrast", 0.0).clamp(0.0, 100.0),
        shift_edge: f(p, "shiftEdge", 0.0).clamp(-100.0, 100.0),
    }
}

/// Most Refine Edge Brush strokes, and points across them, one Select and Mask takes.
pub const MAX_STROKES: usize = 4096;
pub const MAX_STROKE_POINTS: usize = 100_000;

/// The Refine Edge Brush strokes of `refineStrokes`:
/// `[{"points":[[x,y],…],"size":px,"mode":"add|subtract"}]`, in painting order.
fn refine_strokes(p: &Value) -> Result<Vec<matting::EdgeStroke>> {
    const CMD: &str = "select.refineEdge";
    let Some(v) = p.get("refineStrokes").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| bad(CMD, "refineStrokes must be an array of strokes"))?;
    if list.len() > MAX_STROKES {
        return Err(bad(CMD, &format!("refineStrokes holds more than {MAX_STROKES} strokes")));
    }
    let mut total = 0usize;
    let mut strokes = Vec::with_capacity(list.len());
    for stroke in list {
        let o = stroke.as_object().ok_or_else(|| bad(CMD, "each refineStrokes entry must be {\"points\":[[x,y],…],\"size\":px,\"mode\":\"add|subtract\"}"))?;
        let pts = o.get("points").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| bad(CMD, "each stroke needs a non-empty points array"))?;
        total = total.saturating_add(pts.len());
        if total > MAX_STROKE_POINTS {
            return Err(bad(CMD, &format!("refineStrokes holds more than {MAX_STROKE_POINTS} points")));
        }
        let points = pts
            .iter()
            .map(|pt| {
                let xy = pt.as_array().filter(|a| a.len() == 2)?;
                let (x, y) = (xy.first()?.as_f64()?, xy.get(1)?.as_f64()?);
                let ok = |v: f64| v.is_finite() && v.abs() <= 1e7;
                (ok(x) && ok(y)).then_some((x as f32, y as f32))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| bad(CMD, "stroke points must be [x,y] pairs of finite numbers"))?;
        let range = matting::BRUSH_SIZE_RANGE;
        let size = match o.get("size") {
            None => 30.0,
            Some(v) => v
                .as_f64()
                .filter(|s| s.is_finite() && (f64::from(*range.start())..=f64::from(*range.end())).contains(s))
                .ok_or_else(|| bad(CMD, &format!("stroke size must be {}..{} px", range.start(), range.end())))? as f32,
        };
        let add = match o.get("mode").map(|m| m.as_str()) {
            None | Some(Some("add")) => true,
            Some(Some("subtract")) => false,
            Some(_) => return Err(bad(CMD, "stroke mode must be add|subtract")),
        };
        strokes.push(matting::EdgeStroke { points, size, add });
    }
    if matting::swept_area(&strokes) > matting::MAX_SWEPT_AREA {
        return Err(bad(CMD, "refineStrokes paint too large an area"));
    }
    Ok(strokes)
}

/// Select and Mask: applies the workspace's tool `edits` to the selection (or layer mask), refines
/// its edge, then outputs to the selection, a layer mask on the active layer, a new layer or a new
/// document (each with or without a mask).
/// Decontaminating colours needs new pixels, so it upgrades `selection` / `layerMask` output to
/// `newLayerWithMask` (as Photoshop does). New-layer outputs hide the source layer; new-document
/// outputs leave the source document untouched.
fn refine_edge(s: &mut Session, p: &Value) -> Result<Value> {
    let params = refine_params(p);
    let strokes = refine_strokes(p)?;
    let decontaminate = b(p, "decontaminate", false);
    let amount = f(p, "amount", 100.0).clamp(0.0, 100.0);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let active = d.active_layer.filter(|id| d.doc.layer(*id).is_some());
    let default_output =
        if d.doc.selection.is_none() && active.and_then(|id| d.doc.layer(id)).and_then(|l| l.mask.as_ref()).is_some() { "layerMask" } else { "selection" };
    let mut output = p.get("output").and_then(Value::as_str).unwrap_or(default_output).to_string();
    if !matches!(output.as_str(), "selection" | "layerMask" | "newLayer" | "newLayerWithMask" | "newDocument" | "newDocumentWithMask") {
        return Err(bad("select.refineEdge", "output must be selection|layerMask|newLayer|newLayerWithMask|newDocument|newDocumentWithMask"));
    }
    if decontaminate && matches!(output.as_str(), "selection" | "layerMask") {
        output = "newLayerWithMask".into();
    }
    let edits = select_mask_edits::parse(p)?;
    // Before anything is painted: a thin stroke across a huge canvas paints few pixels but many tiles.
    select_mask_edits::check_size(&edits, d.doc.bounds())?;
    select_mask_edits::check_strokes(&strokes, d.doc.bounds())?;
    let region = with_sampler(s, b(p, "sampleAllLayers", false), |smp, doc| {
        let mask_surf = active.and_then(|id| doc.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| &m.surface);
        let target_mask = p.get("target").and_then(Value::as_str) == Some("mask");
        let start = if target_mask { mask_surf.or(doc.selection.as_ref()) } else { doc.selection.as_ref().or(mask_surf) };
        // The workspace's tool edits change the selection before its edge is refined.
        let edited = if edits.is_empty() { None } else { select_mask_edits::apply(doc, active, start.cloned(), &edits) };
        let Some(sel) = (if edits.is_empty() { start } else { edited.as_ref() }) else { return Ok(None) };
        let brush = matting::EdgeBrush::new(&strokes, doc.bounds());
        // The refined result is dense over the selection's bounds: refuse what would not fit
        // rather than allocate it (a thin edit across a huge canvas; the #8783 review).
        let content = sel.content_bounds();
        let area = matting::refine_area(content, doc.bounds(), &params, brush.as_ref());
        if u64::from(area.width()) * u64::from(area.height()) > matting::MAX_REFINE_AREA {
            return Err(bad(
                "select.refineEdge",
                &format!("the selection is too large to refine ({} × {} px; at most {} px in all)", area.width(), area.height(), matting::MAX_REFINE_AREA),
            ));
        }
        Ok(matting::refine_mask_brushed(smp, &matting::surface_reader(sel), content, doc.bounds(), &params, brush.as_ref()))
    })??;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let active = d.active_layer.filter(|id| d.doc.layer(*id).is_some());
    match output.as_str() {
        "selection" => {
            let selected = s.edit("Select and Mask", |doc, _| {
                doc.selection = region.as_ref().map(matting::region_surface);
                Ok(doc.selection.is_some())
            })?;
            Ok(json!({ "output": output, "selected": selected }))
        }
        "layerMask" => {
            let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            s.edit("Select and Mask", |doc, _| {
                let surface =
                    region.as_ref().map(matting::region_surface).unwrap_or_else(|| photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8));
                crate::extra_cmds::background_to_layer_for_mask(doc, id);
                let existing = doc.layer(id).and_then(|l| l.mask.as_ref()).cloned().unwrap_or_else(LayerMask::reveal_all);
                doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(LayerMask { surface, ..existing });
                doc.selection = None;
                Ok(())
            })?;
            Ok(json!({ "output": output, "layer": id.0, "selected": false }))
        }
        _ => {
            let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let src_layer = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let src = src_layer.surface().ok_or_else(|| {
                EngineError::Other(format!("the active layer is {} {} layer without pixels", src_layer.content.article(), src_layer.content.kind_name()))
            })?;
            let name = d.doc.copy_name(&src_layer.name);
            let empty = Region { bbox: Rect::EMPTY, mask: Vec::new() };
            let reg = region.as_ref().unwrap_or(&empty);
            let pixels = if decontaminate { matting::decontaminate(src, reg, params.radius, amount) } else { src.clone() };
            let with_mask = matches!(output.as_str(), "newLayerWithMask" | "newDocumentWithMask");
            let (content, mask) = if with_mask {
                (pixels, Some(LayerMask { surface: matting::region_surface(reg), ..LayerMask::reveal_all() }))
            } else {
                (matting::masked_copy(&pixels, reg), None)
            };
            if output.starts_with("newDocument") {
                // Same canvas, colour mode, depth and profile, so the pixels need no conversion.
                let mut doc = Document::new("Untitled", d.doc.size, d.doc.mode, d.doc.depth);
                doc.resolution_dpi = d.doc.resolution_dpi;
                doc.icc_profile = d.doc.icc_profile.clone();
                doc.color_table = d.doc.color_table.clone();
                doc.duotone = d.doc.duotone.clone();
                let mut layer = Layer::new(src_layer.name.clone(), LayerContent::Raster(content));
                layer.mask = mask;
                let layer_id = layer.id;
                doc.layers.push(layer);
                let i = s.add_document(doc, None);
                return Ok(json!({ "output": output, "document": i, "layer": layer_id.0, "selected": false }));
            }
            let new_id = s.edit("Select and Mask", |doc, act| {
                let mut layer = Layer::new(name, LayerContent::Raster(content));
                layer.mask = mask;
                let nid = doc.insert_above(Some(id), layer);
                if let Some(l) = doc.layer_mut(id) {
                    l.visible = false;
                }
                doc.selection = None;
                *act = Some(nid);
                Ok(nid)
            })?;
            Ok(json!({ "output": output, "layer": new_id.0, "selected": false }))
        }
    }
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

/// Smart selection command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.quick",
            "Quick Selection",
            [],
            r##"{"points":[[x,y],…],"size":px=30,"mode":"add|subtract|replace"="add","sampleAllLayers":bool=false,"enhanceEdge":bool=false}"##,
            has_doc,
            quick_selection
        ),
        spec!(
            "select.object",
            "Object Selection",
            [],
            r##"{"rect":[x,y,w,h] | "point":[x,y],"mode":"replace|add|subtract|intersect"="replace","sampleAllLayers":bool=false} (point: the Object Finder's object under the point, as hovering previews it)"##,
            has_doc,
            object_selection
        ),
        spec!(
            "select.subject",
            "Subject",
            ["Select"],
            r##"{"sampleAllLayers":bool=true,"mode":"replace|new|add|subtract|intersect"="replace","provider":text?,"device":text?,"resource":text?,"params":object={},"allowFallback":bool=false,"enhanceEdge":bool=false,"forceClassical":bool=false,"method":"auto|classical"="auto","target":"pixels"?} (trusted registered adapter first, otherwise classical; adapter results are not cached; non-RGB adapter inputs require sampleAllLayers:false on a native pixel layer)"##,
            has_doc,
            select_subject
        ),
        spec!(
            "select.refineEdge",
            "Refine Edge",
            [],
            r##"{"radius":px=0,"smartRadius":bool=false,"smooth":0..100=0,"feather":px=0,"contrast":0..100=0,"shiftEdge":-100..100=0,"decontaminate":bool=false,"amount":0..100=100,"output":"selection|layerMask|newLayer|newLayerWithMask|newDocument|newDocumentWithMask"="selection","sampleAllLayers":bool=false,"refineStrokes":[{"points":[[x,y],…],"size":px=30,"mode":"add|subtract"="add"},…]=[],"edits":[{"tool":"quick","points":[[x,y],…],"size":px=30,"mode":"add|subtract"="add","sampleAllLayers":bool=false} | {"tool":"brush","points":[[x,y],…],"size":px=30,"hardness":0..100=100,"mode":"add|subtract"="add"} | {"tool":"lasso","points":[[x,y],…≥3],"mode":"add|subtract"="add"} | {"tool":"object","rect":[x,y,w,h] | "point":[x,y],"mode":"add|subtract"="add","sampleAllLayers":bool=false} | {"tool":"subject","mode":"replace|add|subtract"="replace","sampleAllLayers":bool=true} | {"tool":"clear"} | {"tool":"invert"},…]=[]} (refineStrokes: the Refine Edge Brush, in painting order: add refines the painted area from the image, subtract keeps the original edge there; edits: the workspace's Quick Selection, Brush, Lasso, Object Selection, Select Subject, Clear Selection and Invert, applied in order to the selection before its edge is refined)"##,
            has_selection_or_mask,
            refine_edge
        ),
        spec!(
            "select.focusArea",
            "Focus Area…",
            ["Select"],
            r##"{"range":0..1=0.5,"noise":0..1=0,"sampleAllLayers":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
            has_doc,
            focus_area
        ),
    ]
}

#[cfg(test)]
mod tests;
