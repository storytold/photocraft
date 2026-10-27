//! Trusted, session-local compatibility adapters (`plugin.adapter.*`).
//!
//! Trusted Rust callers can register adapters directly; native worker packages additionally
//! use the explicit-consent `adapter_packages` path. This module never interprets an opaque
//! `resource` or loads libraries itself, but a registered adapter can start a trusted program.
//! It does not change the sandboxed WebAssembly plug-in ABI.
//! The first engine bridge reads and writes the canvas rectangle only; off-canvas layer pixels
//! are retained unchanged. Inputs are native-mode f32 samples, not an RGB/u8 conversion.

use std::sync::Arc;

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_plugins::adapter::{self, Image, Limits, OutputKind, Request, Response};
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};

pub const LIST: &str = "plugin.adapter.list";
pub const RUN: &str = "plugin.adapter.run";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: RUN.into(), msg: msg.into() }
}

pub(crate) fn adapter_error(error: adapter::Error) -> EngineError {
    match error {
        adapter::Error::Canceled => EngineError::Cancelled,
        error => EngineError::Other(format!("adapter: {error}")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Pixels,
    Selection,
    LayerMask,
}

impl Destination {
    fn name(self) -> &'static str {
        match self {
            Self::Pixels => "pixels",
            Self::Selection => "selection",
            Self::LayerMask => "layerMask",
        }
    }

    fn accepts(self, kind: &OutputKind) -> bool {
        matches!((self, kind), (Self::Pixels, OutputKind::Image) | (Self::Selection | Self::LayerMask, OutputKind::Mask))
    }
}

struct Params {
    provider: String,
    operation: String,
    resource: Option<String>,
    params: Value,
    device: Option<String>,
    allow_fallback: bool,
    destination: Destination,
    layer: Option<LayerId>,
}

fn required_text(map: &Map<String, Value>, key: &str) -> Result<String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_string)
        .ok_or_else(|| bad(format!("`{key}` must be a nonempty string of at most 128 bytes")))
}

fn optional_text(map: &Map<String, Value>, key: &str, max_bytes: usize) -> Result<Option<String>> {
    map.get(key)
        .map(|v| {
            v.as_str().filter(|s| s.len() <= max_bytes).map(str::to_string).ok_or_else(|| bad(format!("`{key}` must be a string of at most {max_bytes} bytes")))
        })
        .transpose()
}

/// Preflight caller-built JSON before recursively cloning it into the worker snapshot. The
/// adapter core performs its own authoritative validation again before executing trusted code.
fn checked_params(value: &Value, limits: &Limits) -> Result<Value> {
    limits.check_params(value).map_err(|error| bad(format!("adapter parameters: {error}")))?;
    Ok(value.clone())
}

fn parse(p: &Value) -> Result<Params> {
    let map = p.as_object().ok_or_else(|| bad("params must be an object"))?;
    for key in map.keys() {
        if !matches!(key.as_str(), "provider" | "operation" | "resource" | "params" | "device" | "allowFallback" | "output" | "layer" | "target" | "coalesce") {
            let preview: String = key.chars().take(128).collect();
            let suffix = if preview.len() < key.len() { "…" } else { "" };
            return Err(bad(format!("unknown parameter `{preview}{suffix}`; adapter parameters belong in `params`")));
        }
    }
    if map.get("target").is_some_and(|target| target.as_str() != Some("pixels")) {
        return Err(bad("unsupported target: this adapter bridge only reads pixel layers (`target`: `pixels`)"));
    }
    if map.get("coalesce").is_some_and(|value| value.as_str().is_none_or(|s| s.len() > 1024)) {
        return Err(bad("`coalesce` must be a string of at most 1024 bytes"));
    }
    let destination = match required_text(map, "output")?.as_str() {
        "pixels" => Destination::Pixels,
        "selection" => Destination::Selection,
        "layerMask" => Destination::LayerMask,
        "depthLayer" => return Err(bad("unsupported output `depthLayer`: independent grayscale depth layers are not supported by this engine bridge yet")),
        _ => return Err(bad("`output` must be pixels|selection|layerMask|depthLayer (depthLayer is currently unsupported)")),
    };
    let limits = Limits::default();
    let params = match map.get("params") {
        None => Value::Object(Map::new()),
        Some(v) => checked_params(v, &limits)?,
    };
    let allow_fallback = match map.get("allowFallback") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("`allowFallback` must be a boolean"))?,
    };
    let layer = map.get("layer").map(|v| v.as_u64().map(LayerId).ok_or_else(|| bad("`layer` must be an unsigned layer id"))).transpose()?;
    Ok(Params {
        provider: required_text(map, "provider")?,
        operation: required_text(map, "operation")?,
        resource: optional_text(map, "resource", limits.max_resource_bytes)?,
        params,
        device: optional_text(map, "device", 128)?,
        allow_fallback,
        destination,
        layer,
    })
}

/// Validate adapter metadata before the generic dispatcher clones caller-built parameters.
/// Other commands retain their existing admission path.
pub(crate) fn preflight(id: &str, params: &Value) -> Result<()> {
    crate::smartselect_cmds::preflight(id, params)?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::adapter_packages::preflight(id, params)?;
    if id == RUN {
        parse(params)?;
    } else if id == LIST && params.as_object().is_none_or(|map| !map.is_empty()) {
        return Err(EngineError::BadParams { cmd: LIST.into(), msg: "this command takes an empty object".into() });
    }
    Ok(())
}

fn has_document(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn pixel_target(s: &Session) -> Result<()> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.floating.is_some() {
        return Err(bad("unsupported floating selection: use select.drop or edit.undo before running an adapter"));
    }
    if st.channel_view.target != crate::channel_cmds::ChannelTarget::Composite
        || st.doc.quick_mask.is_some()
        || s.tools.mask_targeted
        || crate::mask_view_cmds::current(st).is_some()
    {
        return Err(bad("unsupported target: select the composite pixel layer, not a color channel, mask, alpha channel or Quick Mask"));
    }
    Ok(())
}

fn color_mode(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Bitmap => "bitmap",
        ColorMode::Grayscale => "gray",
        ColorMode::Indexed => "indexed",
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
        ColorMode::Lab => "lab",
        ColorMode::Multichannel => "multichannel",
        ColorMode::Duotone => "duotone",
    }
}

/// Check the canvas before constructing its Rect or reading any dense sample buffer.
fn checked_canvas(doc: &Document, format: PixelFormat, limits: &Limits) -> Result<Rect> {
    let (width, height) = (doc.size.width, doc.size.height);
    let x1 = i32::try_from(width).map_err(|_| bad("canvas width exceeds adapter coordinate limits"))?;
    let y1 = i32::try_from(height).map_err(|_| bad("canvas height exceeds adapter coordinate limits"))?;
    let channels = u8::try_from(format.channels()).map_err(|_| bad("pixel channel count exceeds adapter limits"))?;
    let mut samples = limits.check_image_size(width, height, channels).map_err(adapter_error)?;
    if doc.selection.is_some() {
        samples = samples.checked_add(limits.check_image_size(width, height, 1).map_err(adapter_error)?).ok_or_else(|| bad("canvas sample count overflow"))?;
    }
    if samples > limits.max_samples {
        return Err(bad("canvas and selection exceed adapter sample limits"));
    }
    samples.checked_mul(std::mem::size_of::<f32>()).filter(|bytes| *bytes <= isize::MAX as usize).ok_or_else(|| bad("canvas byte count overflow"))?;
    if doc.icc_profile.as_ref().is_some_and(|p| p.len() > limits.max_icc_bytes.min(adapter::MAX_ICC_BYTES)) {
        return Err(bad("document ICC profile exceeds adapter limits"));
    }
    if let Some(selection) = &doc.selection {
        let sf = selection.format();
        if sf.mode != ColorMode::Grayscale || sf.alpha || selection.channels() != 1 {
            return Err(bad("unsupported selection format: expected a single grayscale coverage plane"));
        }
    }
    Ok(Rect::new(0, 0, x1, y1))
}

fn check_write(doc: &Document, layer: LayerId, destination: Destination) -> Result<()> {
    if destination == Destination::Pixels || destination == Destination::LayerMask {
        // Background conversion is a separate, explicit command; never silently unlock it.
        if destination == Destination::LayerMask && doc.is_background(layer) {
            return Err(bad("unsupported layerMask destination on a Background: convert it to a normal layer first"));
        }
        crate::commands::check_pixels_unlocked(doc, layer)?;
    }
    Ok(())
}

struct Snapshot {
    doc: Arc<Document>,
    layer: LayerId,
    format: PixelFormat,
    area: Rect,
    destination: Destination,
}

struct Completed {
    surface: Surface,
    execution: adapter::Execution,
    ctx: JobCtx,
}

struct Context<'a>(&'a JobCtx);

impl adapter::Context for Context<'_> {
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }

    fn progress(&self, fraction: f32) {
        self.0.progress(fraction, "");
    }
}

fn request(snapshot: &Snapshot, p: Params) -> Result<Request> {
    let doc = &snapshot.doc;
    let surface = doc
        .layer(snapshot.layer)
        .and_then(|l| match &l.content {
            LayerContent::Raster(s) => Some(s),
            _ => None,
        })
        .ok_or_else(|| bad("unsupported input: expected a raster pixel layer"))?;
    let image = Image {
        x: snapshot.area.x0,
        y: snapshot.area.y0,
        width: snapshot.area.width(),
        height: snapshot.area.height(),
        channels: u8::try_from(snapshot.format.channels()).map_err(|_| bad("unsupported channel count"))?,
        color_mode: color_mode(snapshot.format.mode).into(),
        source_depth: snapshot.format.sample.bits() as u8,
        alpha: snapshot.format.alpha,
        icc_profile: match &doc.icc_profile {
            Some(profile) => {
                let mut bytes = crate::allocation::capacity(profile.len(), "adapter ICC profile")?;
                bytes.extend_from_slice(profile);
                bytes
            }
            None => Vec::new(),
        },
        data: surface.try_read_region(snapshot.area)?,
    };
    let selection = doc
        .selection
        .as_ref()
        .map(|surface| -> Result<Image> {
            Ok(Image {
                x: snapshot.area.x0,
                y: snapshot.area.y0,
                width: snapshot.area.width(),
                height: snapshot.area.height(),
                channels: 1,
                color_mode: "gray".into(),
                source_depth: 32,
                alpha: false,
                icc_profile: Vec::new(),
                data: surface.try_read_region(snapshot.area)?,
            })
        })
        .transpose()?;
    Ok(Request { operation: p.operation, resource: p.resource, params: p.params, device: p.device, allow_fallback: p.allow_fallback, image, selection })
}

fn prepare_result(snapshot: &Snapshot, request: &Request, response: Response, ctx: &JobCtx) -> Result<Completed> {
    if !snapshot.destination.accepts(&response.output.kind) {
        return Err(bad("adapter output kind does not match the requested destination"));
    }
    let mut image = response.output.image;
    if (image.x, image.y, image.width, image.height) != (request.image.x, request.image.y, request.image.width, request.image.height) {
        return Err(bad("unsupported adapter output geometry: this bridge requires the unchanged canvas rectangle"));
    }
    let surface = if snapshot.destination == Destination::Pixels {
        let before = &request.image;
        if image.channels != before.channels
            || image.color_mode != before.color_mode
            || image.source_depth != before.source_depth
            || image.alpha != before.alpha
            || image.icc_profile != before.icc_profile
            || image.data.len() != before.data.len()
        {
            return Err(bad("adapter image output must preserve the input layout, mode, depth and ICC profile"));
        }
        let keep_alpha = snapshot.doc.effective_locks(snapshot.layer).transparency || snapshot.doc.is_background(snapshot.layer);
        let channels = usize::from(image.channels);
        if channels == 0 {
            return Err(bad("adapter returned zero image channels"));
        }
        for (i, (out, old)) in image.data.chunks_exact_mut(channels).zip(before.data.chunks_exact(channels)).enumerate() {
            if i % 4096 == 0 {
                ctx.check()?;
            }
            let coverage = match &request.selection {
                Some(selection) => *selection.data.get(i).ok_or_else(|| bad("selection sample count mismatch"))?,
                None => 1.0,
            };
            if !coverage.is_finite() || !(0.0..=1.0).contains(&coverage) {
                return Err(bad("selection coverage must be finite and in 0..1"));
            }
            let coverage64 = f64::from(coverage);
            let alpha_weights = if image.alpha && !keep_alpha && coverage > 0.0 && coverage < 1.0 {
                // Selection mixes associated colour contributions, not straight colours. In
                // particular, hidden RGB in a transparent candidate must not darken an edge.
                let old_alpha = f64::from(*old.last().ok_or_else(|| bad("missing input alpha sample"))?);
                let new_alpha = f64::from(*out.last().ok_or_else(|| bad("missing output alpha sample"))?);
                let old_weight = (1.0 - coverage64) * old_alpha;
                let new_weight = coverage64 * new_alpha;
                Some((old_weight, new_weight, old_weight + new_weight))
            } else {
                None
            };
            for (channel, (out, old)) in out.iter_mut().zip(old).enumerate() {
                if coverage == 0.0 || (keep_alpha && image.alpha && channel == channels - 1) {
                    *out = *old;
                } else if coverage < 1.0 {
                    // f64 intermediates also keep HDR and tiny-alpha products representable.
                    *out = if let Some((old_weight, new_weight, alpha)) = alpha_weights {
                        if channel == channels - 1 {
                            alpha as f32
                        } else if alpha > 0.0 {
                            ((old_weight * f64::from(*old) + new_weight * f64::from(*out)) / alpha) as f32
                        } else {
                            0.0
                        }
                    } else {
                        ((1.0 - coverage64) * f64::from(*old) + coverage64 * f64::from(*out)) as f32
                    };
                }
                if !out.is_finite() {
                    return Err(bad("adapter produced a non-finite blended sample"));
                }
            }
        }
        let mut surface = snapshot.doc.layer(snapshot.layer).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(snapshot.layer))?.clone();
        surface.try_write_region(snapshot.area, &image.data)?;
        surface
    } else {
        if image.channels != 1 || !matches!(image.color_mode.as_str(), "gray" | "grayscale") || image.alpha || !image.icc_profile.is_empty() {
            return Err(bad("mask output requires a single grayscale channel without alpha or an ICC profile"));
        }
        // Selection output is authoritative: do NOT multiply it by the old selection again.
        let depth = if snapshot.destination == Destination::Selection { SampleType::F32 } else { snapshot.doc.depth };
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        surface.try_write_region(snapshot.area, &image.data)?;
        surface
    };
    ctx.check()?;
    Ok(Completed { surface, execution: response.execution, ctx: ctx.clone() })
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let p = parse(p)?;
    pixel_target(s)?;
    let registered = s.adapters.get(&p.provider).ok_or_else(|| bad(format!("adapter `{}` is not registered (see plugin.adapter.list)", p.provider)))?;
    let descriptor = registered.descriptor();
    let operation = descriptor.operations.iter().find(|o| o.id == p.operation).ok_or_else(|| bad("adapter does not advertise this operation"))?;
    if !p.destination.accepts(&operation.output_kind) {
        return Err(bad("operation output kind is incompatible with `output` (images: pixels; masks: selection|layerMask; depthLayer: unsupported)"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let layer = p.layer.or(st.active_layer).ok_or_else(|| bad("no active pixel layer; give `layer`"))?;
    let l = st.doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    let LayerContent::Raster(surface) = &l.content else {
        return Err(bad("unsupported input: adapters require a raster pixel layer, not a smart object, group or generated layer"));
    };
    if !matches!(st.doc.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab)
        || surface.format().mode != st.doc.mode
        || surface.format().sample != st.doc.depth
    {
        return Err(bad(
            "unsupported input mode: this bridge requires matching native RGB, Gray, CMYK or Lab document/layer formats and depth; expanded Indexed/Bitmap/Duotone/Multichannel are not supported",
        ));
    }
    if !operation.color_modes.iter().any(|mode| mode == color_mode(surface.format().mode)) {
        return Err(bad("adapter operation does not advertise this native color mode"));
    }
    if let Some(device) = &p.device {
        let device = descriptor.devices.iter().find(|d| d.id == *device).ok_or_else(|| bad("adapter does not advertise the requested device"))?;
        if !device.operations.contains(&p.operation) {
            return Err(bad("requested device does not advertise this operation"));
        }
    }
    let limits = Limits::default();
    let area = checked_canvas(&st.doc, surface.format(), &limits)?;
    check_write(&st.doc, layer, p.destination)?;
    let snapshot = Arc::new(Snapshot { doc: st.doc.clone(), layer, format: surface.format(), area, destination: p.destination });
    let worker_snapshot = snapshot.clone();
    let provider = p.provider.clone();
    let operation = p.operation.clone();
    let label = format!("Adapter: {}", descriptor.name);
    let edit_label = label.clone();
    crate::jobs::run(
        s,
        &label,
        true,
        move |ctx| {
            ctx.check()?;
            let request = request(&worker_snapshot, p)?;
            ctx.check()?;
            let response = registered.run(&request, &Context(ctx), &limits);
            // Cancellation wins even when an adapter reports a generic error or ignores the flag.
            ctx.check()?;
            let response = response.map_err(adapter_error)?;
            prepare_result(&worker_snapshot, &request, response, ctx)
        },
        move |s, completed| {
            completed.ctx.check()?;
            let st = s.active().ok_or(EngineError::NoDocument)?;
            // This also protects inline use and captures document id, geometry, format, profile,
            // layer identities and selection. Jobs additionally enforce their revision snapshot.
            if !Arc::ptr_eq(&st.doc, &snapshot.doc) {
                return Err(bad("document changed while the adapter ran; result discarded"));
            }
            check_write(&st.doc, snapshot.layer, snapshot.destination)?;
            s.edit(&edit_label, |doc, _| {
                completed.ctx.check()?;
                match snapshot.destination {
                    Destination::Pixels => {
                        let l = doc.layer_mut(snapshot.layer).ok_or(EngineError::NoLayer(snapshot.layer))?;
                        let LayerContent::Raster(surface) = &mut l.content else { return Err(bad("target is no longer a pixel layer")) };
                        *surface = completed.surface;
                    }
                    Destination::Selection => doc.selection = Some(completed.surface),
                    Destination::LayerMask => {
                        let l = doc.layer_mut(snapshot.layer).ok_or(EngineError::NoLayer(snapshot.layer))?;
                        let existing = l.mask.take().unwrap_or_else(LayerMask::reveal_all);
                        l.mask = Some(LayerMask { surface: completed.surface, ..existing });
                    }
                }
                Ok(())
            })?;
            Ok(
                json!({"provider": provider, "operation": operation, "output": snapshot.destination.name(), "layer": snapshot.layer.0, "execution": completed.execution}),
            )
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: LIST,
            label: "List Compatibility Adapters",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: crate::commands::always,
            run: |s, p| {
                if p.as_object().is_none_or(|m| !m.is_empty()) {
                    return Err(EngineError::BadParams { cmd: LIST.into(), msg: "this command takes an empty object".into() });
                }
                Ok(json!({"apiVersion": adapter::API_VERSION, "installed": s.adapters.list()}))
            },
            journal: false,
        },
        CommandSpec {
            id: RUN,
            label: "Run Compatibility Adapter",
            menu: &[],
            shortcut: None,
            params: r##"{"provider":text,"operation":text,"resource":text?,"params":object={},"device":text?,"allowFallback":bool=false,"output":"pixels|selection|layerMask|depthLayer","layer":id?,"target":"pixels"?} (output required; depthLayer unsupported; canvas rectangle only; registration is trusted Rust API only)"##,
            enabled: has_document,
            run,
            journal: true,
        },
    ]
}

impl Session {
    /// Register a trusted, in-process adapter for this session only.
    ///
    /// Explicit-consent native package installation is a separate local engine path; it is not
    /// an arbitrary DLL loader and remains denied by default capability-scoped automation.
    /// Registration neither runs the adapter nor promises devices not in its descriptor.
    pub fn register_adapter(&mut self, adapter: Arc<dyn adapter::Adapter>) -> Result<()> {
        self.adapters.register(adapter).map_err(adapter_error)
    }

    /// Remove a trusted registration, preventing new runs. Already-started jobs retain their
    /// immutable adapter snapshot until completion; cancel those explicitly with the job API.
    /// Removing an installed package is a separate explicit package-management operation.
    /// There is no arbitrary native-library loader command.
    pub fn unregister_adapter(&mut self, id: &str) -> bool {
        self.adapters.remove(id)
    }
}

#[cfg(test)]
#[path = "adapter_cmds/tests.rs"]
mod tests;
