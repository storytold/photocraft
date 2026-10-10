//! Edit › Generative Fill (#41): fill the selection from a prompt through the image provider the
//! app configured ([`Session::set_image_provider`]). With no provider the command is disabled and
//! nothing is ever sent anywhere.
//!
//! The selection's bounds plus some surrounding context are rendered (all visible layers, in
//! sRGB) and sent with a mask of the selection, once per variation (three by default, as
//! Photoshop makes). The results land in a new group masked by the selection, one layer per
//! variation with the first showing, so nothing under it changes and one undo removes it.
//! `layer.generative.selectVariation` flips between them and `layer.generative.more` asks for
//! more. The network calls run as one cancellable background job.
//!
//! Pixel interchange follows #730's approach (composite through the document's profile to sRGB,
//! and back into the document's mode and depth on the way in).

use std::sync::Arc;

use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerId, LayerMask};
use photocraft_genai::{EditRequest, ImageProvider, ImageSize};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const FILL: &str = "edit.generativeFill";
pub const SELECT_VARIATION: &str = "layer.generative.selectVariation";
pub const MORE: &str = "layer.generative.more";
const LABEL: &str = "Generative Fill";
/// Longest side sent to the provider; larger areas are scaled down for the request and the
/// result is scaled back up.
const UPLOAD_SIDE: u32 = 2048;
const DEFAULT_VARIATIONS: usize = 3;
const MAX_VARIATIONS: u64 = 4;
/// Largest result accepted, per side.
const RESULT_SIDE: u32 = 8192;

/// A configured provider, shared with the background job that calls it.
pub type SharedProvider = Arc<dyn ImageProvider>;

impl Session {
    /// Install (or with `None`, remove) the generative image provider. The app builds it from the
    /// user's settings and key; headless and test sessions have none unless they set one.
    pub fn set_image_provider(&mut self, provider: Option<SharedProvider>) {
        self.genai = provider;
    }

    pub fn image_provider(&self) -> Option<&SharedProvider> {
        self.genai.as_ref()
    }
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: FILL.into(), msg: msg.into() }
}

fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

fn fill_enabled(s: &Session) -> std::result::Result<(), String> {
    let p = s.image_provider().ok_or("set up a generative AI provider first (Generative AI Settings)")?;
    if !p.capabilities().edit {
        return Err(format!("{} can't fill selections", p.info().name));
    }
    let st = s.active().ok_or("no document open")?;
    let sel = st.doc.selection.as_ref().ok_or("no selection")?;
    if sel.content_bounds().intersect(&st.doc.bounds()).is_empty() {
        return Err("the selection is empty".into());
    }
    Ok(())
}

/// The area sent: the selection's bounds with a margin of context around them, grown toward the
/// provider's output shape so the result isn't stretched, inside the canvas.
pub(crate) fn context_area(selected: Rect, canvas: Rect, aspect: impl Fn(u32, u32) -> (u32, u32)) -> Rect {
    let side = selected.width().max(selected.height());
    // Half the selection's size on each side: enough of the scene to read what runs into it.
    let margin = (side / 2).clamp(64, 1024) as i32;
    let r = Rect::new(selected.x0 - margin, selected.y0 - margin, selected.x1.saturating_add(margin), selected.y1.saturating_add(margin)).intersect(&canvas);
    let (a, b) = aspect(r.width(), r.height());
    if a == 0 || b == 0 || r.is_empty() {
        return r;
    }
    let (w, h) = (u64::from(r.width()), u64::from(r.height()));
    let (want_w, want_h) =
        if w * u64::from(b) < h * u64::from(a) { ((h * u64::from(a)).div_ceil(u64::from(b)), h) } else { (w, (w * u64::from(b)).div_ceil(u64::from(a))) };
    let grow = |lo: i32, hi: i32, want: u64, min: i32, max: i32| -> (i32, i32) {
        let want = want.min((max - min) as u64) as i32;
        let extra = want - (hi - lo);
        let lo = (lo - extra / 2).max(min);
        let hi = (lo + want).min(max);
        (hi - want, hi)
    };
    let (x0, x1) = grow(r.x0, r.x1, want_w, canvas.x0, canvas.x1);
    let (y0, y1) = grow(r.y0, r.y1, want_h, canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// The visible image over `area`, in 8-bit sRGB.
fn srgb_composite(doc: &Document, area: Rect) -> Result<Surface> {
    let rgb = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut surface = Surface::new(rgb);
    let mut data = Vec::new();
    photocraft_compose::render_bands(doc, area, 0, |band| -> Result<()> {
        data.clear();
        data.resize(band.px.len() * 4, 0.0);
        for (p, out) in band.px.iter().zip(data.as_chunks_mut::<4>().0.iter_mut()) {
            photocraft_raster::from_rgba_into(&rgb, *p, out);
        }
        surface.write_region(band.rect, &data);
        Ok(())
    })?;
    let t = Transform::new(&crate::color_cmds::composite_profile(doc), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).map_err(other)?;
    Ok(crate::color_cmds::convert_surface(&surface, ColorMode::Rgb, rgb, &t))
}

/// sRGB pixels into the document's mode, depth and profile.
fn to_document(doc: &Document, s: &Surface) -> Result<Surface> {
    let fmt = PixelFormat::new(doc.mode, doc.depth, true);
    let t = Transform::new(Builtin::Srgb.profile(), &crate::color_cmds::document_profile(doc), Intent::RelativeColorimetric, true).map_err(other)?;
    let converted = crate::color_cmds::convert_surface(s, ColorMode::Rgb, fmt, &t);
    Ok(if converted.format() == fmt { converted } else { converted.convert(fmt) })
}

/// `area` of `s` scaled to `w`×`h`, at the origin.
fn scaled(s: &Surface, area: Rect, w: u32, h: u32) -> Surface {
    let mut local = Surface::new(s.format());
    local.write_region(Rect::from_xywh(0, 0, area.width(), area.height()), &s.read_region(area));
    if (w, h) == (area.width(), area.height()) {
        return local;
    }
    let sx = f64::from(w) / f64::from(area.width());
    let sy = f64::from(h) / f64::from(area.height());
    photocraft_algo::resample::resize_surface(&local, sx, sy, photocraft_algo::resample::Resample::Bicubic)
}

/// Largest mean difference (0..1) around the selection still treated as colour drift.
const MAX_DRIFT: f64 = 0.25;
/// Grey the selected part of the image is cleared to before it is sent.
const HOLE: f32 = 128.0;

fn encode_rgba(px: Vec<u8>, w: u32, h: u32) -> Result<Vec<u8>> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
}

/// The image and mask sent, both `w`×`h`: `image` (sRGB, already scaled) with the selection
/// cleared to flat grey, and an opaque black-and-white PNG of the selection (white = fill). The
/// provider never sees what is being replaced, as Photoshop's fill doesn't, so it can't paint it
/// back: an empty prompt removes the selected object and continues what lies around it.
fn request_images(image: &Surface, sel: &Surface, area: Rect, w: u32, h: u32) -> Result<(Vec<u8>, Vec<u8>, Reference)> {
    let r = Rect::from_xywh(0, 0, w, h);
    let rgba = image.convert(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true)).to_interleaved(r);
    let coverage = scaled(sel, area, w, h).read_region(r);
    let mut cleared = Vec::with_capacity(rgba.len());
    let mut mask = Vec::with_capacity(rgba.len());
    let mut original = Vec::with_capacity(coverage.len() * 3);
    for (px, &v) in rgba.as_chunks::<4>().0.iter().zip(&coverage) {
        let m = v.clamp(0.0, 1.0);
        let a = f32::from(px[3]) / 255.0;
        // Composite over the hole grey too, so transparent areas read as cleared, not black.
        for &c in &px[..3] {
            let c = f32::from(c) * a + HOLE * (1.0 - a);
            original.push(c / 255.0);
            cleared.push((c * (1.0 - m) + HOLE * m).round() as u8);
        }
        cleared.push(255);
        let g = (m * 255.0).round() as u8;
        mask.extend_from_slice(&[g, g, g, 255]);
    }
    let reference = Reference { original, coverage, w, h };
    Ok((encode_rgba(cleared, w, h)?, encode_rgba(mask, w, h)?, reference))
}

/// What a result is matched against: the area as it was before the hole was cleared (RGB, 0..1)
/// and the selection's coverage, both at the request size.
pub(crate) struct Reference {
    original: Vec<f32>,
    coverage: Vec<f32>,
    w: u32,
    h: u32,
}

/// Box blur of a `w`×`h` plane with radius `r`, edges clamped.
fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize) {
    let pass = |line: &mut [f32], tmp: &mut Vec<f32>| {
        let n = line.len();
        tmp.clear();
        tmp.extend_from_slice(line);
        let mut sum: f32 = (0..=r.min(n - 1)).map(|i| tmp[i]).sum();
        let mut count = r.min(n - 1) + 1;
        for i in 0..n {
            line[i] = sum / count as f32;
            if i + r + 1 < n {
                sum += tmp[i + r + 1];
                count += 1;
            }
            if i >= r {
                sum -= tmp[i - r];
                count -= 1;
            }
        }
    };
    let mut tmp = Vec::new();
    let mut col = vec![0.0; h];
    for row in v.chunks_mut(w) {
        pass(row, &mut tmp);
    }
    for x in 0..w {
        for y in 0..h {
            col[y] = v[y * w + x];
        }
        pass(&mut col, &mut tmp);
        for y in 0..h {
            v[y * w + x] = col[y];
        }
    }
}

/// Take out the colour drift a model adds when it redraws the whole image, so the fill meets the
/// photo around it without a seam. Everything outside the selection should have come back
/// unchanged; comparing it with the original measures the drift. First a per-channel gain and
/// offset fitted over all of it, then what is left near the edge, spread smoothly inward
/// (normalized convolution), as Photoshop's fills blend into their surroundings.
/// `px` is RGBA (0..1) at the reference size.
fn match_surroundings(px: &mut [f32], r: &Reference) {
    let (w, h) = (r.w as usize, r.h as usize);
    let outside: Vec<f32> = r.coverage.iter().map(|c| 1.0 - c.clamp(0.0, 1.0)).collect();
    let weight: f64 = outside.iter().map(|&o| f64::from(o)).sum();
    if weight < 16.0 || px.len() != w * h * 4 {
        return;
    }
    // Drift is a few percent. A result that doesn't resemble the original around the selection
    // (reframed, restyled) is left alone rather than forced onto it.
    let diff: f64 = (0..w * h).map(|i| f64::from(outside[i]) * (0..3).map(|c| f64::from((px[i * 4 + c] - r.original[i * 3 + c]).abs())).sum::<f64>()).sum();
    if diff / (weight * 3.0) > MAX_DRIFT {
        return;
    }
    let radius = (w.max(h) / 16).max(2);
    for c in 0..3 {
        let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for i in 0..w * h {
            let (o, x, y) = (f64::from(outside[i]), f64::from(px[i * 4 + c]), f64::from(r.original[i * 3 + c]));
            sx += o * x;
            sy += o * y;
            sxx += o * x * x;
            sxy += o * x * y;
        }
        let (mx, my) = (sx / weight, sy / weight);
        let var = sxx / weight - mx * mx;
        let gain = if var > 1e-5 { ((sxy / weight - mx * my) / var).clamp(0.5, 2.0) } else { 1.0 };
        let (gain, offset) = (gain as f32, (my - gain * mx) as f32);
        let mut num = vec![0.0f32; w * h];
        for i in 0..w * h {
            let v = &mut px[i * 4 + c];
            *v = *v * gain + offset;
            num[i] = outside[i] * (r.original[i * 3 + c] - *v);
        }
        let mut den = outside.clone();
        for _ in 0..3 {
            box_blur(&mut num, w, h, radius);
            box_blur(&mut den, w, h, radius);
        }
        for i in 0..w * h {
            let v = &mut px[i * 4 + c];
            if r.coverage[i] > 0.0 && den[i] > 1e-4 {
                *v += num[i] / den[i];
            }
            *v = v.clamp(0.0, 1.0);
        }
    }
}

/// Decode a provider's PNG or JPEG into 8-bit sRGB, with its size.
fn decode(bytes: &[u8]) -> Result<(Surface, Rect)> {
    let opts = codecs::DecodeOptions {
        limits: codecs::Limits { max_width: RESULT_SIDE, max_height: RESULT_SIDE, max_pixels: 40_000_000, max_alloc: 512 * 1024 * 1024 },
        keep_orientation: false,
    };
    let format = codecs::detect(bytes)
        .filter(|f| matches!(f, Format::Png | Format::Jpeg | Format::WebP))
        .ok_or_else(|| other("the image service returned an unreadable image"))?;
    let img = codecs::decode_as_with(format, bytes, &opts).map_err(|e| other(format!("the image service returned an unreadable image ({e})")))?;
    let img = img.convert(ChannelLayout::Rgba, CodecSample::U8);
    let r = Rect::from_xywh(0, 0, img.width(), img.height());
    if r.is_empty() {
        return Err(other("the image service returned an empty image"));
    }
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.write_interleaved(r, img.data());
    Ok((s, r))
}

/// The new layer's name: Photoshop names a generative layer after its prompt.
fn layer_name(prompt: &str) -> String {
    let p = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if p.is_empty() {
        return LABEL.into();
    }
    match p.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", &p[..i]),
        None => p,
    }
}

/// What a generative layer was made from (the image and mask sent, the prompt and where the
/// result goes), so Generate More can ask the provider again without re-rendering a document that
/// now holds the earlier results. Kept in the session only: a reopened file's generative layer is
/// an ordinary group.
pub(crate) struct Source {
    prompt: String,
    area: Rect,
    request: EditRequest,
    reference: Reference,
}

fn variations_param(p: &Value, key: &str) -> Result<usize> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(DEFAULT_VARIATIONS),
        Some(v) => match v.as_u64() {
            Some(n @ 1..=MAX_VARIATIONS) => Ok(n as usize),
            _ => Err(bad(format!("`{key}` must be a whole number from 1 to {MAX_VARIATIONS}"))),
        },
    }
}

/// Ask for `count` results of one request. The calls run side by side (each is a slow network
/// round trip); any that fail are dropped as long as one succeeds.
fn request_all(ctx: &crate::jobs::JobCtx, provider: &dyn ImageProvider, request: &EditRequest, count: usize) -> Result<Vec<Vec<u8>>> {
    let name = provider.info().name;
    let msg = if count == 1 { format!("Waiting for {name}") } else { format!("Waiting for {name} ({count} variations)") };
    ctx.progress(0.15, &msg);
    #[cfg(not(target_arch = "wasm32"))]
    let results: Vec<_> = if count == 1 {
        vec![provider.edit(request)]
    } else {
        std::thread::scope(|scope| {
            let calls: Vec<_> = (0..count).map(|_| scope.spawn(|| provider.edit(request))).collect();
            calls.into_iter().map(|c| c.join().unwrap_or(Err(photocraft_genai::GenError::BadResponse("the request failed")))).collect()
        })
    };
    #[cfg(target_arch = "wasm32")]
    let results: Vec<_> = (0..count).map(|_| provider.edit(request)).collect();
    ctx.check()?;
    let mut first_error = None;
    let mut images = Vec::new();
    for r in results {
        match r {
            Ok(img) => images.push(img.bytes),
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    match (images.is_empty(), first_error) {
        (true, Some(e)) => Err(other(e)),
        (true, None) => Err(other("the image service returned nothing")),
        _ => Ok(images),
    }
}

/// A provider's image, colour-matched to the original around the selection, scaled into `area`
/// in document coordinates.
fn place(bytes: &[u8], area: Rect, reference: &Reference) -> Result<Surface> {
    let (got, full) = decode(bytes)?;
    let r = Rect::from_xywh(0, 0, reference.w, reference.h);
    let mut at_request = scaled(&got, full, reference.w, reference.h);
    let mut px = at_request.read_region(r);
    match_surroundings(&mut px, reference);
    at_request.write_region(r, &px);
    let fitted = scaled(&at_request, r, area.width(), area.height());
    let mut placed = Surface::new(fitted.format());
    placed.write_region(area, &fitted.read_region(Rect::from_xywh(0, 0, area.width(), area.height())));
    Ok(placed)
}

fn variation_layer(doc: &Document, pixels: &Surface, n: usize, visible: bool) -> Result<Layer> {
    let converted = to_document(doc, pixels)?;
    let mut layer = Layer::raster(format!("Variation {n}"), converted.format());
    *layer.surface_mut().ok_or_else(|| other("could not create the layer"))? = converted;
    layer.visible = visible;
    Ok(layer)
}

fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.trim().to_owned(),
        Some(_) => return Err(bad("`prompt` must be text")),
    };
    photocraft_genai::check_prompt(&prompt, false).map_err(|e| bad(e.to_string()))?;
    let count = variations_param(p, "variations")?;
    fill_enabled(s).map_err(bad)?;
    let provider = s.image_provider().cloned().ok_or_else(|| bad("no generative AI provider"))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let sel = doc.selection.clone().ok_or_else(|| bad("no selection"))?;
    let canvas = doc.bounds();
    let area = context_area(sel.content_bounds().intersect(&canvas), canvas, |w, h| provider.edit_aspect(w, h));
    let k = (f64::from(UPLOAD_SIDE) / f64::from(area.width().max(area.height()))).min(1.0);
    let (w, h) = (((f64::from(area.width()) * k).round() as u32).max(1), ((f64::from(area.height()) * k).round() as u32).max(1));
    let name = layer_name(&prompt);
    crate::jobs::run(
        s,
        LABEL,
        true,
        move |ctx| {
            ctx.progress(0.05, "Preparing image");
            let (image, mask, reference) = request_images(&scaled(&srgb_composite(&doc, area)?, area, w, h), &sel, area, w, h)?;
            ctx.check()?;
            let request = EditRequest { image, mask, prompt, size: ImageSize::for_long_side(w.max(h)) };
            let results = request_all(ctx, provider.as_ref(), &request, count)?;
            ctx.progress(0.9, "Placing the results");
            let placed = results.iter().map(|b| place(b, area, &reference)).collect::<Result<Vec<_>>>()?;
            let source = Source { prompt: request.prompt.clone(), area, request, reference };
            Ok((placed, sel, source))
        },
        move |s, (placed, sel, source)| {
            let (group, ids) = s.edit(LABEL, |doc, active| {
                // Photoshop's generative layer holds its variations; here that is a group masked by
                // the selection with one layer per variation, the first one showing.
                let children = placed.iter().enumerate().map(|(i, px)| variation_layer(doc, px, i + 1, i == 0)).collect::<Result<Vec<_>>>()?;
                let ids: Vec<u64> = children.iter().map(|l| l.id.0).collect();
                let mut group = Layer::group(name, children);
                group.mask = Some(LayerMask { surface: sel, ..LayerMask::reveal_all() });
                let id = doc.insert_above(*active, group);
                crate::layer_multi_cmds::check_group_depth(doc, LABEL)?;
                *active = Some(id);
                Ok((id, ids))
            })?;
            s.genai_sources.insert(group, Arc::new(source));
            Ok(json!({"layer": group.0, "variations": ids, "bounds": [area.x0, area.y0, area.width(), area.height()]}))
        },
    )
}

/// The generative layer `p["layer"]` names, or the active one: the group itself or one of its
/// variations.
fn generative_target(s: &Session, p: &Value) -> Option<LayerId> {
    let st = s.active()?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => LayerId(id),
        None => st.active_layer?,
    };
    let is_generative = |id: LayerId| s.genai_sources.contains_key(&id) && st.doc.layer(id).is_some_and(Layer::is_group);
    if is_generative(id) {
        return Some(id);
    }
    let path = st.doc.path_of(id)?;
    let parent = st.doc.layer_at(path.get(..path.len().checked_sub(1)?)?)?.id;
    is_generative(parent).then_some(parent)
}

impl Session {
    /// The active generative layer (a Generative Fill group, or the active variation's group), for
    /// the Properties panel's variations.
    pub fn generative_layer(&self) -> Option<LayerId> {
        generative_target(self, &Value::Null)
    }

    /// The prompt a generative layer was made with.
    pub fn generative_prompt(&self, id: LayerId) -> Option<&str> {
        self.genai_sources.get(&id).map(|s| s.prompt.as_str())
    }
}

fn select_enabled(s: &Session) -> std::result::Result<(), String> {
    generative_target(s, &Value::Null).map(|_| ()).ok_or_else(|| "select a generative layer first".into())
}

fn select_variation(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: &str| EngineError::BadParams { cmd: SELECT_VARIATION.into(), msg: msg.into() };
    let group = generative_target(s, p).ok_or_else(|| bad("select a generative layer first"))?;
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("`index` must be a variation number from 0"))? as usize;
    let children = s.active().and_then(|st| st.doc.layer(group)).and_then(Layer::children).map_or(0, <[Layer]>::len);
    if index >= children {
        return Err(bad(&format!("this generative layer has {children} variations")));
    }
    s.edit("Generative Variation", |doc, _| {
        let layers = doc.layer_mut(group).and_then(Layer::children_mut).ok_or_else(|| other("the generative layer is gone"))?;
        for (i, l) in layers.iter_mut().enumerate() {
            l.visible = i == index;
        }
        Ok(())
    })?;
    Ok(json!({"layer": group.0, "index": index}))
}

fn more_enabled(s: &Session) -> std::result::Result<(), String> {
    let p = s.image_provider().ok_or("set up a generative AI provider first (Generative AI Settings)")?;
    if !p.capabilities().edit {
        return Err(format!("{} can't fill selections", p.info().name));
    }
    select_enabled(s)
}

fn more(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: String| EngineError::BadParams { cmd: MORE.into(), msg };
    let count = variations_param(p, "count").map_err(|_| bad(format!("`count` must be a whole number from 1 to {MAX_VARIATIONS}")))?;
    more_enabled(s).map_err(bad)?;
    let group = generative_target(s, p).ok_or_else(|| bad("select a generative layer first".into()))?;
    let provider = s.image_provider().cloned().ok_or_else(|| bad("no generative AI provider".into()))?;
    let source = s.genai_sources.get(&group).cloned().ok_or_else(|| bad("select a generative layer first".into()))?;
    crate::jobs::run(
        s,
        LABEL,
        true,
        move |ctx| {
            let results = request_all(ctx, provider.as_ref(), &source.request, count)?;
            ctx.progress(0.9, "Placing the results");
            results.iter().map(|b| place(b, source.area, &source.reference)).collect::<Result<Vec<_>>>()
        },
        move |s, placed| {
            let ids = s.edit("Generate More", |doc, _| {
                let have = doc.layer(group).and_then(Layer::children).map(<[Layer]>::len).ok_or_else(|| other("the generative layer is gone"))?;
                let new = placed.iter().enumerate().map(|(i, px)| variation_layer(doc, px, have + i + 1, i == 0)).collect::<Result<Vec<_>>>()?;
                let ids: Vec<u64> = new.iter().map(|l| l.id.0).collect();
                let layers = doc.layer_mut(group).and_then(Layer::children_mut).ok_or_else(|| other("the generative layer is gone"))?;
                // The first new variation shows, as when the layer was made.
                for l in layers.iter_mut() {
                    l.visible = false;
                }
                layers.extend(new);
                Ok(ids)
            })?;
            Ok(json!({"layer": group.0, "variations": ids}))
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FILL,
            label: "Generative Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r#"{"prompt":str?,"variations":1..4=3} fills the selection through the configured provider: a new group masked by the selection holds one layer per variation, the first showing. An empty prompt continues the surrounding image through it. Sends the selected area and its surroundings to that provider, once per variation."#,
            enabled: fill_enabled,
            run: fill,
            journal: true,
        },
        CommandSpec {
            id: SELECT_VARIATION,
            label: "Generative Variation",
            menu: &[],
            shortcut: None,
            params: r#"{"index":uint,"layer":id?} shows variation `index` (0 = the first) of the active generative layer and hides the others."#,
            enabled: select_enabled,
            run: select_variation,
            journal: true,
        },
        CommandSpec {
            id: MORE,
            label: "Generate More",
            menu: &[],
            shortcut: None,
            params: r#"{"count":1..4=3,"layer":id?} asks the provider for more variations of the active generative layer, with the same image, mask and prompt, and adds them to it. Sends that area to the provider again."#,
            enabled: more_enabled,
            run: more,
            journal: true,
        },
    ]
}

#[cfg(test)]
#[path = "genai_cmds_tests.rs"]
mod tests;
