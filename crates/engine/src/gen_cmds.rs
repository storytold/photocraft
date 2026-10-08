//! Optional OpenAI image commands. The worker owns a snapshot and applies one undo step.
use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_gen::{ImageProvider, OpenAiProvider};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 16_000_000;
const MARGIN: i32 = 64;

#[derive(Clone, Copy)]
enum Kind {
    Generate,
    Fill,
    Expand,
    Vary,
}

#[derive(Clone, Copy)]
enum VariationSource {
    Composite,
    Layer,
}

fn variation_source(p: &Value, cmd: &str) -> Result<VariationSource> {
    match p.get("source") {
        None => Ok(VariationSource::Composite),
        Some(Value::String(s)) if s == "composite" => Ok(VariationSource::Composite),
        Some(Value::String(s)) if s == "layer" => Ok(VariationSource::Layer),
        _ => Err(bad(cmd, "`source` must be `composite` or `layer`")),
    }
}

fn key_enabled(s: &Session) -> std::result::Result<(), String> {
    if s.active().is_none() {
        return Err("no document open".into());
    }
    if std::env::var("OPENAI_API_KEY").ok().is_none_or(|v| v.trim().is_empty()) {
        return Err("Generative AI is off: set OPENAI_API_KEY".into());
    }
    Ok(())
}

fn bad(cmd: &str, msg: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

fn prompt<'a>(p: &'a Value, cmd: &str, required: bool) -> Result<&'a str> {
    if p.get("prompt").is_some_and(|v| !v.is_string()) {
        return Err(bad(cmd, "`prompt` must be text"));
    }
    let v = p.get("prompt").and_then(Value::as_str).unwrap_or(if required { "" } else { "Create a faithful visual variation of this image." });
    if v.trim().is_empty() || v.len() > 32_000 {
        return Err(bad(cmd, "`prompt` must be nonempty text (at most 32000 bytes)"));
    }
    Ok(v)
}
fn size<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    if p.get("size").is_some_and(|v| !v.is_string()) {
        return Err(bad(cmd, "`size` must be text"));
    }
    let v = p.get("size").and_then(Value::as_str).unwrap_or("1024x1024");
    if !matches!(v, "1024x1024" | "1024x1536" | "1536x1024" | "auto") {
        return Err(bad(cmd, "`size` must be 1024x1024, 1024x1536, 1536x1024, or auto"));
    }
    Ok(v)
}
fn limit(r: Rect, cmd: &str) -> Result<()> {
    if r.is_empty() || r.width() > MAX_SIDE || r.height() > MAX_SIDE || u64::from(r.width()) * u64::from(r.height()) > MAX_PIXELS {
        return Err(bad(cmd, "image too large for generative AI (maximum 4096 px per side and 16 MP)"));
    }
    Ok(())
}
fn coded_size(w: u32, h: u32) -> &'static str {
    if w > h.saturating_mul(5) / 4 {
        "1536x1024"
    } else if h > w.saturating_mul(5) / 4 {
        "1024x1536"
    } else {
        "1024x1024"
    }
}
fn fit(s: &Surface, r: Rect) -> (Surface, Rect) {
    let k = (1024.0 / f64::from(r.width())).min(1024.0 / f64::from(r.height())).min(1.0);
    let w = (f64::from(r.width()) * k).round().max(1.0) as u32;
    let h = (f64::from(r.height()) * k).round().max(1.0) as u32;
    let mut src = Surface::new(s.format());
    src.write_region(Rect::from_xywh(0, 0, r.width(), r.height()), &s.read_region(r));
    if k >= 1.0 {
        return (src, Rect::from_xywh(0, 0, r.width(), r.height()));
    }
    let out = photocraft_algo::resample::resize_surface(
        &src,
        f64::from(w) / f64::from(r.width()),
        f64::from(h) / f64::from(r.height()),
        photocraft_algo::resample::Resample::Bicubic,
    );
    (out, Rect::from_xywh(0, 0, w, h))
}
fn png(s: &Surface, r: Rect) -> Result<Vec<u8>> {
    let img = Image::from_u8(r.width(), r.height(), ChannelLayout::Rgba, s.to_interleaved(r)).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
}
fn decode_png(bytes: &[u8]) -> Result<(Surface, Rect)> {
    let opts = codecs::DecodeOptions {
        limits: codecs::Limits { max_width: MAX_SIDE, max_height: MAX_SIDE, max_pixels: MAX_PIXELS, max_alloc: 128 * 1024 * 1024 },
        keep_orientation: false,
    };
    let img = codecs::decode_as_with(Format::Png, bytes, &opts).map_err(other)?.convert(ChannelLayout::Rgba, CodecSample::U8);
    let r = Rect::from_xywh(0, 0, img.width(), img.height());
    limit(r, "image.ai")?;
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.write_interleaved(r, img.data());
    Ok((s, r))
}
fn rgb_for_api(doc: &Document, area: Rect) -> Result<Surface> {
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
    let src = crate::color_cmds::composite_profile(doc);
    let t = Transform::new(&src, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).map_err(other)?;
    Ok(crate::color_cmds::convert_surface(&surface, ColorMode::Rgb, rgb, &t))
}
fn from_api(doc: &Document, s: &Surface) -> Result<Surface> {
    let fmt = PixelFormat::new(doc.mode, doc.depth, true);
    let target = crate::color_cmds::document_profile(doc);
    let t = Transform::new(Builtin::Srgb.profile(), &target, Intent::RelativeColorimetric, true).map_err(other)?;
    let converted = crate::color_cmds::convert_surface(s, ColorMode::Rgb, fmt, &t);
    Ok(if converted.format() == fmt { converted } else { converted.convert(fmt) })
}
fn mask_png(r: Rect, mut alpha: impl FnMut(i32, i32) -> f32) -> Result<Vec<u8>> {
    let count = (r.width() as usize).checked_mul(r.height() as usize).and_then(|n| n.checked_mul(4)).ok_or_else(|| bad("image.ai", "mask too large"))?;
    let mut pixels = vec![255u8; count];
    for (i, px) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = r.x0 + (i % r.width() as usize) as i32;
        let y = r.y0 + (i / r.width() as usize) as i32;
        px[3] = ((1.0 - alpha(x, y).clamp(0.0, 1.0)) * 255.0).round() as u8;
    }
    let img = Image::from_u8(r.width(), r.height(), ChannelLayout::Rgba, pixels).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
}
// The placement mask reaches into the source by half the requested expansion, capped at 64 px.
// Use distance to the nearest edge so both axes meet smoothly at corners.
fn expand_blend_alpha(x: i32, y: i32, old: Rect, band: u32) -> f32 {
    if !old.contains(x, y) {
        return 1.0;
    }
    let distance = (x - old.x0).min(old.x1 - 1 - x).min(y - old.y0).min(old.y1 - 1 - y) as f32;
    let t = (1.0 - distance / band as f32).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn match_expand_colors(generated: &mut Surface, original: &Surface, area: Rect, old: Rect, band: u32) {
    let mut pixels = generated.read_region(area);
    let mut original_sum = [0.0f64; 3];
    let mut generated_sum = [0.0f64; 3];
    let mut original_squares = [0.0f64; 3];
    let mut generated_squares = [0.0f64; 3];
    let mut count = 0.0f64;
    for y in old.y0..old.y1 {
        for x in old.x0..old.x1 {
            if expand_blend_alpha(x, y, old, band) <= 0.0 {
                continue;
            }
            let source = original.rgba(x, y);
            let index = (y as usize * area.width() as usize + x as usize) * 4;
            let Some(result) = pixels.get(index..index + 4) else { continue };
            if source[3] < 0.5 || result[3] < 0.5 {
                continue;
            }
            count += 1.0;
            for channel in 0..3 {
                let a = f64::from(source[channel]);
                let b = f64::from(result[channel]);
                original_sum[channel] += a;
                generated_sum[channel] += b;
                original_squares[channel] += a * a;
                generated_squares[channel] += b * b;
            }
        }
    }
    if count == 0.0 {
        return;
    }
    let mut mean = [0.0; 3];
    let mut target = [0.0; 3];
    let mut gain = [1.0; 3];
    for channel in 0..3 {
        mean[channel] = generated_sum[channel] / count;
        target[channel] = original_sum[channel] / count;
        let source_std = (original_squares[channel] / count - target[channel] * target[channel]).max(0.0).sqrt();
        let result_std = (generated_squares[channel] / count - mean[channel] * mean[channel]).max(0.0).sqrt();
        if result_std > 1e-6 {
            gain[channel] = source_std / result_std;
        }
    }
    for pixel in pixels.as_chunks_mut::<4>().0 {
        for channel in 0..3 {
            pixel[channel] = ((f64::from(pixel[channel]) - mean[channel]) * gain[channel] + target[channel]).clamp(0.0, 1.0) as f32;
        }
    }
    generated.write_region(area, &pixels);
}
fn place(s: &mut Session, label: &str, rgb: Surface, area: Rect, mask: Option<Surface>, expand: Option<u32>) -> Result<Value> {
    let id = s.edit(label, |doc, active| {
        if let Some(n) = expand {
            crate::image_cmds::translate_doc(doc, n as i32, n as i32);
            doc.size = Size::new(area.width(), area.height());
            crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
        }
        let fmt = PixelFormat::new(doc.mode, doc.depth, true);
        let mut converted = from_api(doc, &rgb)?;
        let mut pixels = converted.read_region(area);
        if let Some(m) = &mask {
            let channels = fmt.channels();
            for (i, px) in pixels.chunks_exact_mut(channels).enumerate() {
                let x = area.x0 + (i % area.width() as usize) as i32;
                let y = area.y0 + (i / area.width() as usize) as i32;
                if let Some(a) = px.last_mut() {
                    *a *= m.sample_channel(x, y, 0).clamp(0.0, 1.0);
                }
            }
            converted = Surface::new(fmt);
            converted.write_region(area, &pixels);
        }
        let mut layer = Layer::raster(doc.next_layer_name(label), fmt);
        *layer.surface_mut().ok_or_else(|| EngineError::Other("could not create layer".into()))? = converted;
        let id = doc.insert_above(*active, layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer":id.0,"bounds":[area.x0,area.y0,area.width(),area.height()]}))
}
fn run(s: &mut Session, p: &Value, kind: Kind, cmd: &'static str, label: &'static str) -> Result<Value> {
    // Validate before taking a snapshot or starting a network job.
    if p.get("model").is_some_and(|v| !v.is_string()) {
        return Err(bad(cmd, "`model` must be text"));
    }
    let provider = OpenAiProvider::from_env(p.get("model").and_then(Value::as_str)).map_err(other)?;
    run_with_provider(s, p, kind, cmd, label, provider)
}

fn run_with_provider(s: &mut Session, p: &Value, kind: Kind, cmd: &'static str, label: &'static str, provider: impl ImageProvider + 'static) -> Result<Value> {
    let text = prompt(p, cmd, !matches!(kind, Kind::Vary))?.to_owned();
    let requested = size(p, cmd)?.to_owned();
    let source = if matches!(kind, Kind::Vary) { Some(variation_source(p, cmd)?) } else { None };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let bounds = doc.bounds();
    let (area, selected, expand) = match kind {
        Kind::Generate => {
            let (w, h) = match requested.as_str() {
                "1024x1536" => (1024, 1536),
                "1536x1024" => (1536, 1024),
                _ => (1024, 1024),
            };
            (Rect::from_xywh(0, 0, w, h), None, None)
        }
        Kind::Fill => {
            let sel = doc.selection.clone().ok_or_else(|| bad(cmd, "requires an active selection"))?;
            let b = sel.content_bounds().intersect(&bounds);
            if b.is_empty() {
                return Err(bad(cmd, "selection is empty"));
            }
            let x0 = (b.x0 - MARGIN).max(0);
            let y0 = (b.y0 - MARGIN).max(0);
            let x1 = b.x1.saturating_add(MARGIN).min(bounds.x1);
            let y1 = b.y1.saturating_add(MARGIN).min(bounds.y1);
            (Rect::new(x0, y0, x1, y1), Some(sel), None)
        }
        Kind::Expand => {
            let n = p
                .get("pixels")
                .and_then(Value::as_u64)
                .filter(|&n| (1..=1024).contains(&n))
                .ok_or_else(|| bad(cmd, "`pixels` must be an integer from 1 to 1024"))? as u32;
            let w = doc.size.width.checked_add(n * 2).ok_or_else(|| bad(cmd, "width out of range"))?;
            let h = doc.size.height.checked_add(n * 2).ok_or_else(|| bad(cmd, "height out of range"))?;
            (Rect::from_xywh(0, 0, w, h), None, Some(n))
        }
        Kind::Vary => {
            if matches!(source, Some(VariationSource::Layer)) {
                let id = st.active_layer.ok_or_else(|| bad(cmd, "requires an active layer"))?;
                let layer = doc.layer(id).and_then(Layer::surface).ok_or_else(|| bad(cmd, "active layer must be a raster layer"))?;
                if layer.content_bounds().intersect(&bounds).is_empty() {
                    return Err(bad(cmd, "active layer is empty"));
                }
            }
            (bounds, None, None)
        }
    };
    limit(area, cmd)?;
    let original = st.active_layer;
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.check()?;
            ctx.progress(0.05, "Preparing image");
            let (input, edit_mask, target_mask, expansion_source) = match kind {
                Kind::Generate => (None, None, None, None),
                Kind::Fill => {
                    let composite = rgb_for_api(&doc, area)?;
                    let (small, r) = fit(&composite, area);
                    let sel = selected.as_ref().ok_or_else(|| bad(cmd, "selection is missing"))?;
                    let sx = f64::from(area.width()) / f64::from(r.width());
                    let sy = f64::from(area.height()) / f64::from(r.height());
                    let mask = mask_png(r, |x, y| sel.sample_channel(area.x0 + (f64::from(x) * sx) as i32, area.y0 + (f64::from(y) * sy) as i32, 0))?;
                    (Some(png(&small, r)?), Some(mask), selected.clone(), None)
                }
                Kind::Expand => {
                    let n = expand.ok_or_else(|| bad(cmd, "expansion margin is missing"))?;
                    let band = n.div_ceil(2).min(64);
                    let composite = rgb_for_api(&doc, doc.bounds())?;
                    let shifted = photocraft_algo::resample::translate_surface(&composite, n as i32, n as i32);
                    let (small, r) = fit(&shifted, area);
                    let sx = f64::from(area.width()) / f64::from(r.width());
                    let sy = f64::from(area.height()) / f64::from(r.height());
                    let old = Rect::from_xywh(n as i32, n as i32, doc.size.width, doc.size.height);
                    let mask =
                        mask_png(r, |x, y| if old.contains(area.x0 + (f64::from(x) * sx) as i32, area.y0 + (f64::from(y) * sy) as i32) { 0.0 } else { 1.0 })?;
                    let mut keep = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
                    let mut values = Vec::with_capacity(area.width() as usize * area.height() as usize);
                    for y in 0..area.height() as i32 {
                        for x in 0..area.width() as i32 {
                            values.push(expand_blend_alpha(x, y, old, band));
                        }
                    }
                    keep.write_region(area, &values);
                    (Some(png(&small, r)?), Some(mask), Some(keep), Some(shifted))
                }
                Kind::Vary => {
                    let rgb = match source {
                        Some(VariationSource::Composite) => rgb_for_api(&doc, area)?,
                        Some(VariationSource::Layer) => {
                            let id = original.ok_or_else(|| bad(cmd, "active layer is missing"))?;
                            let layer = doc.layer(id).and_then(Layer::surface).ok_or_else(|| bad(cmd, "active layer must be a raster layer"))?;
                            let source = layer.convert(PixelFormat::new(doc.mode, doc.depth, true));
                            let t = Transform::new(&crate::color_cmds::document_profile(&doc), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true)
                                .map_err(other)?;
                            crate::color_cmds::convert_surface(&source, doc.mode, PixelFormat::new(ColorMode::Rgb, SampleType::U8, true), &t)
                                .convert(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true))
                        }
                        None => return Err(bad(cmd, "variation source is missing")),
                    };
                    let (small, r) = fit(&rgb, area);
                    (Some(png(&small, r)?), None, None, None)
                }
            };
            ctx.check()?;
            ctx.progress(0.2, "Waiting for OpenAI");
            let result = match kind {
                Kind::Generate => provider.generate(&text, &requested),
                Kind::Fill | Kind::Expand => provider.edit(
                    input.as_deref().unwrap_or_default(),
                    edit_mask.as_deref().unwrap_or_default(),
                    &text,
                    coded_size(area.width(), area.height()),
                ),
                Kind::Vary => provider.variations(input.as_deref().unwrap_or_default(), &text, coded_size(area.width(), area.height())),
            }
            .map_err(other)?;
            ctx.check()?;
            ctx.progress(0.8, "Decoding image");
            let (decoded, got) = decode_png(&result)?;
            let output_size = if matches!(kind, Kind::Generate) { requested.as_str() } else { coded_size(area.width(), area.height()) };
            let expected = match output_size {
                "1024x1024" => Some((1024, 1024)),
                "1024x1536" => Some((1024, 1536)),
                "1536x1024" => Some((1536, 1024)),
                _ => None,
            };
            if expected.is_some_and(|(w, h)| got.width() != w || got.height() != h) {
                return Err(other("image service returned a different image size than requested"));
            }
            let sx = f64::from(area.width()) / f64::from(got.width());
            let sy = f64::from(area.height()) / f64::from(got.height());
            let scaled = photocraft_algo::resample::resize_surface(&decoded, sx, sy, photocraft_algo::resample::Resample::Bicubic);
            let mut positioned = Surface::new(scaled.format());
            let local = Rect::from_xywh(0, 0, area.width(), area.height());
            positioned.write_region(area, &scaled.read_region(local));
            if let (Some(n), Some(original)) = (expand, expansion_source.as_ref()) {
                let old = Rect::from_xywh(n as i32, n as i32, doc.size.width, doc.size.height);
                match_expand_colors(&mut positioned, original, area, old, n.div_ceil(2).min(64));
            }
            ctx.check()?;
            Ok((positioned, target_mask))
        },
        move |s, (image, mask)| place(s, label, image, area, mask, expand),
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "image.ai.generate",
            label: "Generate Image…",
            menu: &["Image"],
            shortcut: None,
            params: r##"{"prompt":str,"size":"1024x1024|1024x1536|1536x1024|auto"?,"model":"gpt-image-1"?}"##,
            enabled: key_enabled,
            run: |s, p| run(s, p, Kind::Generate, "image.ai.generate", "Generated Image"),
            journal: true,
        },
        CommandSpec {
            id: "edit.ai.generativeFill",
            label: "Generative Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r##"{"prompt":str,"model":"gpt-image-1"?} requires a selection"##,
            enabled: key_enabled,
            run: |s, p| run(s, p, Kind::Fill, "edit.ai.generativeFill", "Generative Fill"),
            journal: true,
        },
        CommandSpec {
            id: "image.ai.expandCanvas",
            label: "Expand Canvas…",
            menu: &["Image"],
            shortcut: None,
            params: r##"{"pixels":1..1024,"prompt":str,"model":"gpt-image-1"?}"##,
            enabled: key_enabled,
            run: |s, p| run(s, p, Kind::Expand, "image.ai.expandCanvas", "Expand Canvas with AI"),
            journal: true,
        },
        CommandSpec {
            id: "layer.ai.variations",
            label: "Variations…",
            menu: &["Layer"],
            shortcut: None,
            params: r##"{"prompt":str?,"source":"composite|layer"="composite","model":"gpt-image-1"?} composite uses the visible image; layer uses the active raster layer"##,
            enabled: key_enabled,
            run: |s, p| run(s, p, Kind::Vary, "layer.ai.variations", "Generated Variation"),
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_gen::{GenError, HttpTransport};
    use std::sync::{Arc, Condvar, Mutex};

    type Request = (String, String, Vec<u8>);
    type Requests = Arc<Mutex<Vec<Request>>>;
    type Gate = Arc<(Mutex<(bool, bool)>, Condvar)>;

    #[derive(Clone)]
    struct MockTransport {
        requests: Requests,
        reply: Arc<Vec<u8>>,
        gate: Option<Gate>,
    }
    impl HttpTransport for MockTransport {
        fn post(&self, path: &str, _: &str, content_type: &str, body: &[u8]) -> photocraft_gen::Result<Vec<u8>> {
            self.requests.lock().map_err(|_| GenError::Service("test lock"))?.push((path.into(), content_type.into(), body.to_vec()));
            if let Some(gate) = &self.gate {
                let (lock, wake) = &**gate;
                let mut state = lock.lock().map_err(|_| GenError::Service("test lock"))?;
                state.0 = true;
                wake.notify_all();
                while !state.1 {
                    state = wake.wait(state).map_err(|_| GenError::Service("test lock"))?;
                }
            }
            Ok((*self.reply).clone())
        }
    }
    fn provider(reply: Vec<u8>) -> (OpenAiProvider<MockTransport>, Requests) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport { requests: requests.clone(), reply: Arc::new(reply), gate: None };
        (OpenAiProvider::new("test-key".into(), "gpt-image-1", transport).unwrap(), requests)
    }
    fn response(w: u32, h: u32) -> Vec<u8> {
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        for px in pixels.as_chunks_mut::<4>().0.iter_mut() {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        let image = Image::from_u8(w, h, ChannelLayout::Rgba, pixels).unwrap();
        let png = codecs::encode(&image, Format::Png, &Default::default()).unwrap();
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in png.chunks(3) {
            let a = chunk[0];
            let b = chunk.get(1).copied().unwrap_or(0);
            let c = chunk.get(2).copied().unwrap_or(0);
            encoded.push(ALPHABET[(a >> 2) as usize] as char);
            encoded.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            encoded.push(if chunk.len() > 1 { ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize] as char } else { '=' });
            encoded.push(if chunk.len() > 2 { ALPHABET[(c & 63) as usize] as char } else { '=' });
        }
        serde_json::to_vec(&json!({"data":[{"b64_json":encoded}]})).unwrap()
    }
    fn session(w: u32, h: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":w,"height":h})).unwrap();
        s
    }
    fn depth_session(mode: ColorMode, depth: SampleType) -> Session {
        let mut s = Session::new();
        let mode_name = if mode == ColorMode::Grayscale { "gray" } else { "rgb" };
        let depth_number = match depth {
            SampleType::U8 => 8,
            SampleType::U16 => 16,
            SampleType::F32 => 32,
        };
        s.execute("file.new", json!({"width":24,"height":24,"mode":mode_name,"depth":depth_number,"background":"transparent"})).unwrap();
        let state = s.active_mut().unwrap();
        let mut doc = (*state.doc).clone();
        let fmt = doc.pixel_format();
        let high_precision = if depth == SampleType::F32 { 1.75 } else { 0.123_456 };
        let pixel = if mode == ColorMode::Grayscale { vec![high_precision, 1.0] } else { vec![high_precision, 0.567_89, 0.812_34, 1.0] };
        let mut pixels = Vec::new();
        for _ in 0..24 * 24 {
            pixels.extend_from_slice(&pixel);
        }
        let mut surface = Surface::new(fmt);
        surface.write_region(doc.bounds(), &pixels);
        let id = doc.top_layer().unwrap();
        *doc.layer_mut(id).unwrap().surface_mut().unwrap() = surface;
        state.doc = Arc::new(doc);
        s
    }
    fn request_image(requests: &Requests) -> Image {
        let requests = requests.lock().unwrap();
        let body = &requests[0].2;
        let marker = b"name=\"image\"";
        let field = body.windows(marker.len()).position(|w| w == marker).unwrap();
        let start = body[field..].windows(4).position(|w| w == b"\r\n\r\n").unwrap() + field + 4;
        let end = body[start..].windows(4).position(|w| w == b"\r\n--").unwrap() + start;
        codecs::decode_as(Format::Png, &body[start..end]).unwrap()
    }
    fn call(s: &mut Session, kind: Kind, p: Value, provider: OpenAiProvider<MockTransport>) -> Result<Value> {
        let (id, label) = match kind {
            Kind::Generate => ("image.ai.generate", "Generated Image"),
            Kind::Fill => ("edit.ai.generativeFill", "Generative Fill"),
            Kind::Expand => ("image.ai.expandCanvas", "Expand Canvas with AI"),
            Kind::Vary => ("layer.ai.variations", "Generated Variation"),
        };
        run_with_provider(s, &p, kind, id, label, provider)
    }
    #[test]
    fn invalid_params_fail_before_network() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":16,"height":16})).unwrap();
        assert!(prompt(&json!({"prompt":[]}), "image.ai.generate", true).is_err());
        assert!(size(&json!({"size":12}), "image.ai.generate").is_err());
        assert!(limit(Rect::from_xywh(0, 0, MAX_SIDE + 1, 1), "image.ai.generate").is_err());
        for id in ["image.ai.generate", "edit.ai.generativeFill", "image.ai.expandCanvas", "layer.ai.variations"] {
            assert!(s.execute(id, json!({"prompt":[],"pixels":-1})).is_err());
        }
    }

    #[test]
    fn interchange_pixels_keep_document_depth_and_mask_alpha() {
        let r = Rect::from_xywh(0, 0, 2, 1);
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        src.write_region(r, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let doc = Document::new("x", Size::new(2, 1), ColorMode::Rgb, depth);
            let out = from_api(&doc, &src).unwrap();
            assert_eq!(out.format().sample, depth);
            assert!(out.sample_channel(0, 0, 0) > 0.99);
        }
        let mask = mask_png(r, |x, _| if x == 0 { 1.0 } else { 0.0 }).unwrap();
        let decoded = codecs::decode_as(Format::Png, &mask).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
        assert_eq!(decoded.data()[3], 0);
        assert_eq!(decoded.data()[7], 255);
    }

    #[test]
    fn generated_layer_preserves_source_and_one_undo_restores_document() {
        let mut s = session(96, 80);
        let before = (*s.active().unwrap().doc).clone();
        let depth = s.active().unwrap().history.past_len();
        let (p, requests) = provider(response(1024, 1024));
        let result = call(&mut s, Kind::Generate, json!({"prompt":"red square"}), p).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.walk().len(), before.walk().len() + 1);
        assert_eq!(st.history.past_len(), depth + 1);
        assert_eq!(st.doc.layer(before.top_layer().unwrap()), before.layer(before.top_layer().unwrap()));
        assert!(st.doc.layer(photocraft_doc::LayerId(result["layer"].as_u64().unwrap())).is_some());
        assert_eq!(requests.lock().unwrap()[0].0, "/generations");
        let saved = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
        assert!(!saved.windows(b"test-key".len()).any(|w| w == b"test-key"));
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, before);
    }

    #[test]
    fn fill_clamps_margin_and_sends_transparent_selected_mask() {
        let mut s = session(96, 80);
        s.execute("select.rect", json!({"x":0,"y":0,"width":4,"height":4,"antiAlias":false})).unwrap();
        let before = (*s.active().unwrap().doc).clone();
        let depth = s.active().unwrap().history.past_len();
        let (p, requests) = provider(response(1024, 1024));
        let result = call(&mut s, Kind::Fill, json!({"prompt":"fill"}), p).unwrap();
        assert_eq!(result["bounds"], json!([0, 0, 68, 68]));
        let st = s.active().unwrap();
        assert_eq!(st.history.past_len(), depth + 1);
        assert_eq!(st.doc.layer(before.top_layer().unwrap()), before.layer(before.top_layer().unwrap()));
        let layer = st.doc.layer(photocraft_doc::LayerId(result["layer"].as_u64().unwrap())).unwrap();
        let surf = layer.surface().unwrap();
        assert!(surf.rgba(1, 1)[3] > 0.99);
        assert_eq!(surf.rgba(10, 10)[3], 0.0);
        assert_eq!(surf.rgba(80, 70)[3], 0.0);
        let req = requests.lock().unwrap();
        assert_eq!(req[0].0, "/edits");
        let body = &req[0].2;
        let start = body.windows(b"name=\"mask\"".len()).position(|w| w == b"name=\"mask\"").unwrap();
        let png_start = body[start..].windows(4).position(|w| w == b"\r\n\r\n").unwrap() + start + 4;
        let mask = codecs::decode_as(Format::Png, &body[png_start..]).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
        assert_eq!(mask.data()[3], 0);
        assert_eq!(mask.data()[(10 * 68 + 10) * 4 + 3], 255);
        drop(req);
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, before);
    }

    #[test]
    fn expansion_moves_source_and_undo_restores_canvas() {
        let mut s = session(8, 8);
        s.execute("edit.fill", json!({"color":"#00ff00"})).unwrap();
        let before = (*s.active().unwrap().doc).clone();
        let old_id = before.top_layer().unwrap();
        let depth = s.active().unwrap().history.past_len();
        let (p, _) = provider(response(1024, 1024));
        call(&mut s, Kind::Expand, json!({"prompt":"extend","pixels":2}), p).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.size, Size::new(12, 12));
        assert_eq!(st.history.past_len(), depth + 1);
        assert_eq!(st.doc.walk().len(), before.walk().len() + 1);
        let old = st.doc.layer(old_id).unwrap().surface().unwrap();
        assert_eq!(old.rgba(2, 2), before.layer(old_id).unwrap().surface().unwrap().rgba(0, 0));
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, before);
    }

    #[test]
    fn expansion_matches_color_across_original_edge_and_preserves_center() {
        let mut s = session(24, 24);
        s.execute("edit.fill", json!({"color":"#406080"})).unwrap();
        let before = (*s.active().unwrap().doc).clone();
        let source = rgb_for_api(&before, before.bounds()).unwrap();
        let (p, requests) = provider(response(1024, 1024)); // The mock returns solid red.
        let result = call(&mut s, Kind::Expand, json!({"prompt":"extend","pixels":8}), p).unwrap();
        let layer = s.active().unwrap().doc.layer(photocraft_doc::LayerId(result["layer"].as_u64().unwrap())).unwrap();
        let alpha = layer.surface().unwrap();
        assert!(alpha.rgba(9, 20)[3] > 0.1);
        assert!(alpha.rgba(9, 20)[3] < 1.0);
        assert_eq!(alpha.rgba(20, 20)[3], 0.0);
        let rendered = rgb_for_api(&s.active().unwrap().doc, Rect::from_xywh(0, 0, 40, 40)).unwrap();
        let y = 20;
        for x in 7..13 {
            let left = rendered.rgba(x, y);
            let right = rendered.rgba(x + 1, y);
            for channel in 0..3 {
                assert!((left[channel] - right[channel]).abs() < 0.08, "color seam at x={x}, channel={channel}");
            }
        }
        assert_eq!(rendered.rgba(20, 20), source.rgba(12, 12));
        assert_eq!(rendered.rgba(8, 20), source.rgba(0, 12));
        let req = requests.lock().unwrap();
        let body = &req[0].2;
        let start = body.windows(b"name=\"mask\"".len()).position(|w| w == b"name=\"mask\"").unwrap();
        let png_start = body[start..].windows(4).position(|w| w == b"\r\n\r\n").unwrap() + start + 4;
        let api_mask = codecs::decode_as(Format::Png, &body[png_start..]).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
        assert_eq!(api_mask.data()[3], 0); // New border is transparent to the API.
        assert_eq!(api_mask.data()[(20 * 40 + 20) * 4 + 3], 255); // Original stays opaque.
        drop(req);
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, before);
    }

    #[test]
    fn expansion_blend_band_is_monotone_and_bounded_at_corners() {
        let old = Rect::from_xywh(1024, 1024, 256, 256);
        for pixels in [1u32, 2, 8, 128, 1024] {
            let band = pixels.div_ceil(2).min(64);
            assert_eq!(
                band,
                match pixels {
                    1 | 2 => 1,
                    8 => 4,
                    _ => 64,
                }
            );
            let mut previous = 1.0;
            for distance in 0..=band + 1 {
                let alpha = expand_blend_alpha(old.x0 + distance as i32, old.y0 + 128, old, band);
                assert!((0.0..=1.0).contains(&alpha));
                assert!(alpha <= previous);
                previous = alpha;
            }
            assert_eq!(expand_blend_alpha(old.x0 - 1, old.y0, old, band), 1.0);
            assert_eq!(expand_blend_alpha(old.x0, old.y0, old, band), 1.0);
            assert_eq!(expand_blend_alpha(old.x0 + band as i32, old.y0 + band as i32, old, band), 0.0);
            for y in old.y0 - 1..=old.y0 + band as i32 {
                for x in old.x0 - 1..=old.x0 + band as i32 {
                    assert!((0.0..=1.0).contains(&expand_blend_alpha(x, y, old, band)));
                }
            }
        }
    }

    #[test]
    fn expansion_accepts_minimum_and_maximum_pixel_margins() {
        for pixels in [1u32, 2, 1024] {
            let mut s = session(4, 4);
            s.execute("edit.fill", json!({"color":"#406080"})).unwrap();
            let before = (*s.active().unwrap().doc).clone();
            let (p, _) = provider(response(1024, 1024));
            call(&mut s, Kind::Expand, json!({"prompt":"extend","pixels":pixels}), p).unwrap();
            assert_eq!(s.active().unwrap().doc.size, Size::new(4 + pixels * 2, 4 + pixels * 2));
            assert!(s.undo());
            assert_eq!(*s.active().unwrap().doc, before);
        }
    }

    #[test]
    fn invalid_preconditions_and_wrong_image_size_leave_no_history() {
        let mut s = session(32, 24);
        let before = (*s.active().unwrap().doc).clone();
        let depth = s.active().unwrap().history.past_len();
        for (kind, params) in
            [(Kind::Generate, json!({"prompt":" "})), (Kind::Fill, json!({"prompt":"fill"})), (Kind::Expand, json!({"prompt":"extend","pixels":0}))]
        {
            let (p, requests) = provider(response(1024, 1024));
            assert!(call(&mut s, kind, params, p).is_err());
            assert!(requests.lock().unwrap().is_empty());
        }
        s.active_mut().unwrap().active_layer = None;
        let (p, _) = provider(response(1024, 1024));
        assert!(call(&mut s, Kind::Vary, json!({"source":"layer"}), p).is_err());
        s.active_mut().unwrap().active_layer = before.top_layer();
        let (p, _) = provider(response(4, 4));
        assert!(call(&mut s, Kind::Generate, json!({"prompt":"red"}), p).unwrap_err().to_string().contains("different image size"));
        for reply in [b"{".to_vec(), br#"{"data":[{"b64_json":"AQID"}]}"#.to_vec()] {
            let (p, _) = provider(reply);
            assert!(call(&mut s, Kind::Generate, json!({"prompt":"red"}), p).is_err());
        }
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), depth);
    }

    #[test]
    fn fill_rejects_empty_selection_and_clamps_oversized_selection() {
        let mut s = session(32, 24);
        let mut doc = (*s.active().unwrap().doc).clone();
        doc.selection = Some(Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)));
        s.active_mut().unwrap().doc = Arc::new(doc);
        let (p, requests) = provider(response(1024, 1024));
        assert!(call(&mut s, Kind::Fill, json!({"prompt":"fill"}), p).is_err());
        assert!(requests.lock().unwrap().is_empty());
        s.execute("select.rect", json!({"x":-10,"y":-10,"width":100,"height":100,"antiAlias":false})).unwrap();
        let (p, _) = provider(response(1536, 1024));
        let result = call(&mut s, Kind::Fill, json!({"prompt":"fill"}), p).unwrap();
        assert_eq!(result["bounds"], json!([0, 0, 32, 24]));
    }

    #[test]
    fn variations_need_nonempty_active_raster_and_undo_cleanly() {
        let mut s = session(32, 24);
        s.execute("layer.new.layer", json!({})).unwrap();
        let (p, requests) = provider(response(1024, 1024));
        assert!(call(&mut s, Kind::Vary, json!({"source":"layer"}), p).is_err());
        assert!(requests.lock().unwrap().is_empty());
        s.execute("edit.fill", json!({"color":"#008000"})).unwrap();
        let before = (*s.active().unwrap().doc).clone();
        let old_id = s.active().unwrap().active_layer.unwrap();
        let depth = s.active().unwrap().history.past_len();
        let (p, requests) = provider(response(1536, 1024));
        call(&mut s, Kind::Vary, json!({"source":"layer"}), p).unwrap();
        assert_eq!(requests.lock().unwrap()[0].0, "/edits");
        assert_eq!(s.active().unwrap().history.past_len(), depth + 1);
        assert_eq!(s.active().unwrap().doc.layer(old_id), before.layer(old_id));
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, before);
    }

    #[test]
    fn all_image_commands_preserve_depth_and_untouched_pixels() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for (kind, vary_source) in
                    [(Kind::Generate, "composite"), (Kind::Fill, "composite"), (Kind::Expand, "composite"), (Kind::Vary, "composite"), (Kind::Vary, "layer")]
                {
                    let mut s = depth_session(mode, depth);
                    if matches!(kind, Kind::Fill) {
                        s.execute("select.rect", json!({"x":12,"y":12,"width":2,"height":2,"antiAlias":false})).unwrap();
                    }
                    let before = (*s.active().unwrap().doc).clone();
                    let old_id = before.top_layer().unwrap();
                    let old = before.layer(old_id).unwrap().surface().unwrap();
                    let old_pixel = old.pixel(12, 12);
                    if depth == SampleType::F32 {
                        assert_eq!(old_pixel[0], 1.75);
                    }
                    let old_bytes = old.to_interleaved(before.bounds());
                    let history = s.active().unwrap().history.past_len();
                    let (p, requests) = provider(response(1024, 1024));
                    let params = match kind {
                        Kind::Generate => json!({"prompt":"generate"}),
                        Kind::Fill => json!({"prompt":"fill"}),
                        Kind::Expand => json!({"prompt":"expand","pixels":8}),
                        Kind::Vary => json!({"source":vary_source}),
                    };
                    let result = call(&mut s, kind, params, p).unwrap();
                    let state = s.active().unwrap();
                    let layer = state.doc.layer(photocraft_doc::LayerId(result["layer"].as_u64().unwrap())).unwrap().surface().unwrap();
                    assert_eq!(layer.format(), PixelFormat::new(mode, depth, true), "{mode:?} {depth:?}");
                    let source = state.doc.layer(old_id).unwrap().surface().unwrap();
                    if matches!(kind, Kind::Expand) {
                        assert_eq!(source.pixel(20, 20), old_pixel);
                        assert_eq!(layer.rgba(20, 20)[3], 0.0);
                    } else {
                        assert_eq!(source.to_interleaved(before.bounds()), old_bytes);
                    }
                    if matches!(kind, Kind::Fill) {
                        assert_eq!(layer.rgba(0, 0)[3], 0.0);
                        assert_eq!(layer.rgba(20, 20)[3], 0.0);
                        assert!(layer.rgba(12, 12)[3] > 0.0);
                    }
                    if !matches!(kind, Kind::Generate) {
                        let image = request_image(&requests);
                        assert_eq!(image.layout(), ChannelLayout::Rgba);
                        assert_eq!(image.sample_type(), CodecSample::U8);
                        assert!(image.width() > 0 && image.height() > 0);
                    }
                    assert_eq!(state.history.past_len(), history + 1);
                    assert!(s.undo());
                    assert_eq!(*s.active().unwrap().doc, before);
                }
            }
        }
    }

    #[test]
    fn variations_use_visible_composite_by_default_or_active_layer_only() {
        let mut s = depth_session(ColorMode::Rgb, SampleType::U8);
        s.execute("layer.new.layer", json!({})).unwrap();
        let state = s.active_mut().unwrap();
        let mut doc = (*state.doc).clone();
        let active = state.active_layer.unwrap();
        let top = doc.layer_mut(active).unwrap().surface_mut().unwrap();
        top.write_region(Rect::from_xywh(12, 12, 2, 2), &[0.0, 1.0, 0.0, 1.0].repeat(4));
        state.doc = Arc::new(doc);
        let before = (*s.active().unwrap().doc).clone();
        for (params, lower_visible) in [(json!({}), true), (json!({"source":"layer"}), false)] {
            let (p, requests) = provider(response(1024, 1024));
            let result = call(&mut s, Kind::Vary, params, p).unwrap();
            assert_eq!(result["bounds"], json!([0, 0, 24, 24]));
            let layer_id = photocraft_doc::LayerId(result["layer"].as_u64().unwrap());
            assert_eq!(s.active().unwrap().doc.layer(layer_id).unwrap().surface().unwrap().content_bounds(), before.bounds());
            let image = request_image(&requests);
            let outside = &image.data()[..4];
            assert_eq!(outside[3] > 0, lower_visible);
            let inside = (12 * 24 + 12) * 4;
            assert!(image.data()[inside + 1] > image.data()[inside]);
            assert!(s.undo());
            assert_eq!(*s.active().unwrap().doc, before);
        }
        s.active_mut().unwrap().active_layer = None;
        let (p, _) = provider(response(1024, 1024));
        call(&mut s, Kind::Vary, json!({}), p).unwrap();
        assert!(s.undo());
        let (p, requests) = provider(response(1024, 1024));
        assert!(call(&mut s, Kind::Vary, json!({"source":"invalid"}), p).is_err());
        assert!(requests.lock().unwrap().is_empty());
        let (p, requests) = provider(response(1024, 1024));
        assert!(call(&mut s, Kind::Vary, json!({"source":3}), p).is_err());
        assert!(requests.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_provider_job_leaves_document_and_undo_untouched() {
        let mut s = session(16, 16);
        let before = (*s.active().unwrap().doc).clone();
        let depth = s.active().unwrap().history.past_len();
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let transport = MockTransport { requests: Arc::new(Mutex::new(Vec::new())), reply: Arc::new(response(1024, 1024)), gate: Some(gate.clone()) };
        let provider = OpenAiProvider::new("test-key".into(), "gpt-image-1", transport).unwrap();
        let started = s
            .start_job(
                "image.ai.generate",
                json!({"prompt":"red"}),
                "Generated Image",
                true,
                move |_| provider.generate("red", "1024x1024").map_err(other),
                |s, _| {
                    place(s, "Generated Image", Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true)), Rect::from_xywh(0, 0, 1, 1), None, None)
                },
            )
            .unwrap();
        let crate::jobs::Started::Job(id) = started else { panic!("expected background job") };
        let (lock, wake) = &*gate;
        let state = lock.lock().unwrap();
        let mut state = wake.wait_while(state, |state| !state.0).unwrap();
        assert!(s.cancel_job(id));
        state.1 = true;
        wake.notify_all();
        drop(state);
        s.join_cancelled_jobs();
        s.poll_jobs();
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), depth);
    }
}
