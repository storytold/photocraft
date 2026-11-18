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
}

impl Base {
    /// The index into `hole` and `base` of document point `(x, y)`, when it is inside `rect`.
    pub(super) fn index(&self, x: f64, y: f64) -> Option<usize> {
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        self.rect.contains(xi, yi).then(|| (yi - self.rect.y0) as usize * self.rect.width() as usize + (xi - self.rect.x0) as usize)
    }
}

/// The preview's error when the sampling area has no pixels outside the fill area (the engine's
/// fill refuses that case with the same words).
pub(super) const NOTHING_TO_SAMPLE: &str = "nothing to sample from: the sampling area has no pixels outside the fill area";

/// `edit.contentAwareFill`'s parameters for `command` on `layer`: the command's own serialisation plus the layer.
pub(super) fn plan_params(command: &FillCommand, layer: LayerId) -> Value {
    let mut p = serde_json::to_value(command).unwrap_or_else(|_| json!({}));
    p["layer"] = json!(layer.0);
    p
}

/// The plan without strokes: the window, fill area and the option's area. Custom starts empty
/// (the window and fill area are those of Rectangular).
pub(super) fn compute_base(doc: &Document, layer: LayerId, command: &FillCommand) -> Result<Base, String> {
    let mut p = plan_params(command, layer);
    if let Some(map) = p.as_object_mut() {
        map.remove("samplingStrokes");
    }
    let custom = command.sampling == SamplingMode::Custom;
    if custom {
        p["sampling"] = json!("rectangular");
    }
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan(doc, layer, &p, CMD).map_err(|e| e.to_string())?;
    let base = if custom { vec![false; plan.base.len()] } else { plan.base };
    Ok(Base { window: plan.window, rect: plan.rect, hole: plan.hole, base })
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
