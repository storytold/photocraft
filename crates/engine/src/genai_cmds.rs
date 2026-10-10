//! Edit › Generative Fill (#41): fill the selection from a prompt through the image provider the
//! app configured ([`Session::set_image_provider`]). With no provider the command is disabled and
//! nothing is ever sent anywhere.
//!
//! The selection's bounds plus some surrounding context are rendered (all visible layers, in
//! sRGB) and sent with a mask of the selection. The result lands as a new layer masked by the
//! selection, as Photoshop's generative layer is, so nothing under it changes and one undo
//! removes it. The network call runs as a cancellable background job.
//!
//! Pixel interchange follows #730's approach (composite through the document's profile to sRGB,
//! and back into the document's mode and depth on the way in).

use std::sync::Arc;

use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerMask};
use photocraft_genai::{EditRequest, ImageProvider, ImageSize};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const FILL: &str = "edit.generativeFill";
const LABEL: &str = "Generative Fill";
/// Longest side sent to the provider; larger areas are scaled down for the request and the
/// result is scaled back up.
const UPLOAD_SIDE: u32 = 2048;
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

fn encode_png(s: &Surface, w: u32, h: u32) -> Result<Vec<u8>> {
    let r = Rect::from_xywh(0, 0, w, h);
    let rgba = s.convert(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_interleaved(r)).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
}

/// The selection over `area`, scaled to `w`×`h`, as an opaque black-and-white PNG (white = fill).
fn mask_png(sel: &Surface, area: Rect, w: u32, h: u32) -> Result<Vec<u8>> {
    let gray = scaled(sel, area, w, h);
    let values = gray.read_region(Rect::from_xywh(0, 0, w, h));
    let mut px = Vec::with_capacity(values.len() * 4);
    for v in values {
        let g = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        px.extend_from_slice(&[g, g, g, 255]);
    }
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
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

fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.trim().to_owned(),
        Some(_) => return Err(bad("`prompt` must be text")),
    };
    photocraft_genai::check_prompt(&prompt, false).map_err(|e| bad(e.to_string()))?;
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
            let image = encode_png(&scaled(&srgb_composite(&doc, area)?, area, w, h), w, h)?;
            let mask = mask_png(&sel, area, w, h)?;
            ctx.check()?;
            ctx.progress(0.15, &format!("Waiting for {}", provider.info().name));
            let request = EditRequest { image, mask, prompt, size: ImageSize::for_long_side(w.max(h)) };
            let result = provider.edit(&request).map_err(other)?;
            ctx.check()?;
            ctx.progress(0.9, "Placing the result");
            let (got, full) = decode(&result.bytes)?;
            let fitted = scaled(&got, full, area.width(), area.height());
            let mut placed = Surface::new(fitted.format());
            placed.write_region(area, &fitted.read_region(Rect::from_xywh(0, 0, area.width(), area.height())));
            Ok((placed, sel))
        },
        move |s, (pixels, sel)| {
            let id = s.edit(LABEL, |doc, active| {
                let converted = to_document(doc, &pixels)?;
                let mut layer = Layer::raster(name, converted.format());
                *layer.surface_mut().ok_or_else(|| other("could not create the layer"))? = converted;
                layer.mask = Some(LayerMask { surface: sel, ..LayerMask::reveal_all() });
                let id = doc.insert_above(*active, layer);
                *active = Some(id);
                Ok(id)
            })?;
            Ok(json!({"layer": id.0, "bounds": [area.x0, area.y0, area.width(), area.height()]}))
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: FILL,
        label: "Generative Fill…",
        menu: &["Edit"],
        shortcut: None,
        params: r#"{"prompt":str?} fills the selection on a new masked layer through the configured provider; an empty prompt continues the surrounding image through it. Sends the selected area and its surroundings to that provider."#,
        enabled: fill_enabled,
        run: fill,
        journal: true,
    }]
}

#[cfg(test)]
#[path = "genai_cmds_tests.rs"]
mod tests;
