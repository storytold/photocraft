use craft::imaging::ui::fill::{FillCommand, SamplingMode};
use egui::Color32;
use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::CMD;

/// What the plan (without the brush strokes) says: the window, the fill area and the sampling
/// option's area, over `rect`.
pub(super) struct Base {
    pub(super) window: Rect,
    pub(super) rect: Rect,
    pub(super) hole: Vec<bool>,
    pub(super) base: Vec<bool>,
    /// Whether each overlay proxy pixel has coverage in the sampled image; `pw * ph` long.
    pub(super) eligible_px: Vec<bool>,
}

impl Base {
    /// The index into `hole` and `base` of document point `(x, y)`, when it is inside `rect`.
    pub(super) fn index(&self, x: f64, y: f64) -> Option<usize> {
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        self.rect.contains(xi, yi).then(|| (yi - self.rect.y0) as usize * self.rect.width() as usize + (xi - self.rect.x0) as usize)
    }
}

/// The proxy the overlay planes live in: `pw * ph` pixels, each `k` document px wide, over `canvas`.
pub(super) struct Proxy {
    pub(super) pw: usize,
    pub(super) ph: usize,
    pub(super) k: usize,
    pub(super) canvas: Rect,
}

/// The preview's error when the sampling area has no usable pixels outside the fill area (the engine's
/// fill refuses that case with the same words).
pub(super) const NOTHING_TO_SAMPLE: &str = "nothing to sample from: the sampling area has no usable pixels outside the fill area";

/// `edit.contentAwareFill`'s parameters for `command` on `layer`: the command's own serialisation plus the layer.
pub(super) fn plan_params(command: &FillCommand, layer: LayerId) -> Value {
    let mut p = serde_json::to_value(command).unwrap_or_else(|_| json!({}));
    p["layer"] = json!(layer.0);
    p
}

/// The plan without strokes: the window, fill area and the option's area. Custom starts empty
/// (the window and fill area are those of Rectangular). `eligible_px` marks, at proxy resolution,
/// which pixels the image actually covers, so `describe()`'s sampled-pixel count leaves transparent
/// samples out the same way the engine leaves them out of the fill. The sampling overlay itself is
/// engine-driven (via `source()`), so `eligible_px` does not feed it.
pub(super) fn compute_base(doc: &Document, layer: LayerId, command: &FillCommand, proxy: &Proxy, sample_all: bool) -> Result<Base, String> {
    let mut p = plan_params(command, layer);
    if let Some(map) = p.as_object_mut() {
        map.remove("samplingStrokes");
    }
    let custom = command.sampling == SamplingMode::Custom;
    if custom {
        p["sampling"] = json!("rectangular");
    }
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan(doc, layer, &p, CMD).map_err(|e| e.to_string())?;
    let eligible_px = sampling_coverage(doc, layer, &plan, proxy, sample_all)?;
    let base = if custom { vec![false; plan.base.len()] } else { plan.base };
    Ok(Base { window: plan.window, rect: plan.rect, hole: plan.hole, base, eligible_px })
}

/// Coverage at proxy resolution: whether each proxy pixel's document point has an opaque sample.
/// Brush strokes can reach beyond the plan's window, so pixels outside `rect` are read from the
/// composite (Sample All Layers) or the layer itself.
fn sampling_coverage(
    doc: &Document,
    layer: LayerId,
    plan: &photocraft_engine::edit_menu_cmds::CafPlan,
    proxy: &Proxy,
    sample_all: bool,
) -> Result<Vec<bool>, String> {
    let Some(alpha) = plan.opts.alpha_channel else { return Ok(vec![true; proxy.pw * proxy.ph]) };
    let surface = doc.layer(layer).and_then(|l| l.surface()).ok_or("no pixel layer")?;
    let composite = (sample_all && plan.rect != proxy.canvas).then(|| photocraft_compose::render_reduced(doc, proxy.pw as u32, proxy.ph as u32));
    let n = plan.fmt.channels();
    let stride = plan.rect.width() as usize;
    let k = proxy.k as f64;
    let mut coverage = vec![false; proxy.pw * proxy.ph];
    for (i, eligible) in coverage.iter_mut().enumerate() {
        let (px, py) = (i % proxy.pw, i / proxy.pw);
        let (x, y) = (f64::from(proxy.canvas.x0) + (px as f64 + 0.5) * k, f64::from(proxy.canvas.y0) + (py as f64 + 0.5) * k);
        let (x, y) = (x.floor() as i32, y.floor() as i32);
        if !proxy.canvas.contains(x, y) {
            continue;
        }
        let value = if plan.rect.contains(x, y) {
            let sample = (y - plan.rect.y0) as usize * stride + (x - plan.rect.x0) as usize;
            plan.img[sample * n + alpha]
        } else if let Some(composite) = &composite {
            composite.get(px as i32, py as i32)[3]
        } else {
            let mut pixel = [0.0f32; 8];
            surface.read_pixel(x, y, &mut pixel);
            pixel[alpha]
        };
        *eligible = value > 0.0 && value.is_finite();
    }
    Ok(coverage)
}

/// The filled rectangle as an image (what Apply would write there), as a texture no wider than
/// `max_side`: past the device's limit it is box-averaged to fit (the rectangle it is drawn over
/// stays the plan's). The number is the document pixels each texel covers.
pub(super) fn compute_preview(
    doc: &Document,
    layer: LayerId,
    p: &Value,
    ctl: &photocraft_raster::Interrupt,
    max_side: usize,
) -> Result<(Rect, egui::ColorImage, u32), String> {
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan_with(doc, layer, p, CMD, ctl).map_err(|e| match e {
        photocraft_engine::EngineError::Cancelled => super::CANCELLED.to_string(),
        e => e.to_string(),
    })?;
    // Patches come only from the sampling area outside the fill area. With none (the Sampling
    // Brush erased it all, a Custom area not painted yet) the fill would come out black, so the
    // preview says so instead of showing that.
    if !plan.source.iter().any(|s| *s) {
        return Err(NOTHING_TO_SAMPLE.into());
    }
    let (w, h) = (plan.rect.width() as usize, plan.rect.height() as usize);
    let n = plan.fmt.channels();
    let filled =
        photocraft_algo::content_aware::fill_with(w, h, n, &plan.img, &plan.hole, &plan.source, &plan.opts, ctl).map_err(|_| super::CANCELLED.to_string())?;
    let own = plan.layer_pixels();
    // Pixel `i`: the fill over the fill area (a partly selected pixel by its coverage), the
    // layer's own pixels elsewhere.
    let pixel = |i: usize| -> [f32; 4] {
        let src = if plan.hole[i] { &filled[i * n..(i + 1) * n] } else { &own[i * n..(i + 1) * n] };
        let mut c = photocraft_raster::to_rgba(&plan.fmt, src);
        if plan.hole[i] {
            let k = plan.cover[i];
            let under = photocraft_raster::to_rgba(&plan.fmt, &own[i * n..(i + 1) * n]);
            for j in 0..4 {
                c[j] = under[j] + (c[j] - under[j]) * k;
            }
        }
        c
    };
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color = |c: [f32; 4]| Color32::from_rgba_unmultiplied(b(c[0]), b(c[1]), b(c[2]), b(c[3]));
    let f = crate::filter_preview_ui::texture_factor(w as u32, h as u32, max_side) as usize;
    if f == 1 {
        let px = (0..w * h).map(|i| color(pixel(i))).collect();
        return Ok((plan.rect, egui::ColorImage::new([w, h], px), 1));
    }
    let (tw, th) = (w.div_ceil(f), h.div_ceil(f));
    let mut px = Vec::with_capacity(tw * th);
    for ty in 0..th {
        for tx in 0..tw {
            // The block's premultiplied average.
            let (mut acc, mut count) = ([0.0f32; 4], 0.0f32);
            for y in ty * f..((ty + 1) * f).min(h) {
                for x in tx * f..((tx + 1) * f).min(w) {
                    let c = pixel(y * w + x);
                    for j in 0..3 {
                        acc[j] += c[j] * c[3];
                    }
                    acc[3] += c[3];
                    count += 1.0;
                }
            }
            let c = if acc[3] > 0.0 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / count] } else { [0.0; 4] };
            px.push(color(c));
        }
    }
    Ok((plan.rect, egui::ColorImage::new([tw, th], px), f as u32))
}
