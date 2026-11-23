//! Compute RAW Develop adjustments from the same proxy used by the interactive preview.

use photocraft_algo::camera_raw::{
    CameraRaw, DevelopInputs,
    auto::{Image, measure_shadow_noise},
    develop,
};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use super::{CommandSpec, layer_param, lens_cmds};
use crate::{EngineError, Result, Session};

pub use photocraft_algo::camera_raw::auto::{Panel, suggest, suggest_full};

const ID: &str = "filter.cameraRaw.autoSettings";
const PROXY_SIDE: usize = 900;

/// RAW Develop works over the canvas plus the layer's pixels outside it.
pub struct Proxy {
    pub pixels: Vec<[f32; 4]>,
    pub width: usize,
    pub height: usize,
    pub full_width: usize,
    pub full_height: usize,
    pub block: usize,
    pub area: Rect,
}

impl Proxy {
    pub fn new(surface: &Surface, canvas: Rect) -> std::result::Result<Self, String> {
        let area = canvas.union(&surface.content_bounds());
        let width = area.x1.checked_sub(area.x0).filter(|n| (1..=1_048_576).contains(n));
        let height = area.y1.checked_sub(area.y0).filter(|n| (1..=1_048_576).contains(n));
        let (Some(w), Some(h)) = (width, height) else { return Err("RAW Develop preview bounds are too large".into()) };
        let (w, h) = (w as usize, h as usize);
        if w.checked_mul(h).is_none_or(|pixels| pixels > 512_000_000) {
            return Err("RAW Develop preview bounds are too large".into());
        }
        let block = w.max(h).div_ceil(PROXY_SIDE).max(1);
        let (width, height) = (w.div_ceil(block), h.div_ceil(block));
        let pixels = box_average(surface, area, block);
        Ok(Self { pixels, width, height, full_width: w, full_height: h, block, area })
    }

    /// Raster selection or smart-filter mask at the centre of each proxy block.
    pub fn sample_coverage(&self, value: impl Fn(i32, i32) -> f32) -> Vec<f32> {
        let k = self.block;
        (0..self.height)
            .flat_map(|y| {
                (0..self.width).map(move |x| {
                    (self.area.x0 + (x * k + k / 2).min(self.full_width - 1) as i32, self.area.y0 + (y * k + k / 2).min(self.full_height - 1) as i32)
                })
            })
            .map(|(x, y)| value(x, y).clamp(0.0, 1.0))
            .collect()
    }

    /// Sample native shadow pixels through the same selection used by Auto.
    pub fn shadow_noise(&self, surface: &Surface, gamma: f32, coverage: Option<&[f32]>) -> f32 {
        measure_shadow_noise(surface, self.area, gamma, |x, y| {
            coverage.is_none_or(|mask| {
                let col = (x - self.area.x0) as usize / self.block;
                let row = (y - self.area.y0) as usize / self.block;
                mask.get(row * self.width + col).is_some_and(|&a| a >= 0.5)
            })
        })
    }
}

/// Box averaging reads each source row once. Repeat the last source row when a block
/// runs off the bottom, without rereading it for very tall blocks on one-row images.
fn box_average(surf: &Surface, area: Rect, k: usize) -> Vec<[f32; 4]> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let mut proxy = vec![[0.0f32; 4]; pw * ph];
    let mut row = vec![[0.0f32; 4]; w];
    let mut acc = vec![[0.0f64; 5]; pw];
    for py in 0..ph {
        // f64 avoids overflow when summing large, finite HDR values.
        acc.fill([0.0; 5]);
        for sy in py * k..((py + 1) * k).min(h) {
            let weight = if sy == h - 1 { ((py + 1) * k - sy) as f64 } else { 1.0 };
            let y = area.y0 + sy as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            for (x, q) in row.iter().enumerate() {
                if let Some(a) = acc.get_mut(x / k) {
                    for c in 0..4 {
                        a[c] += f64::from(q[c]) * weight;
                    }
                    a[4] += weight;
                }
            }
        }
        for (pixel, a) in proxy.iter_mut().skip(py * pw).zip(&acc) {
            *pixel = [0, 1, 2, 3].map(|c| (a[c] / a[4]) as f32);
        }
    }
    proxy
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}

fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let empty = serde_json::Map::new();
    let fields = if params.is_null() { &empty } else { params.as_object().ok_or_else(|| invalid("expected an object"))? };
    if let Some(key) = fields.keys().find(|k| !matches!(k.as_str(), "layer" | "panel" | "settings")) {
        return Err(invalid(format!("unknown parameter {key}")));
    }
    let panels = match fields.get("panel") {
        None => Panel::ALL.to_vec(),
        Some(value) if value.as_str() == Some("all") => Panel::ALL.to_vec(),
        Some(value) => vec![value.as_str().and_then(Panel::from_id).ok_or_else(|| invalid("panel must be light, whiteBalance, color, effects or all"))?],
    };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let id = layer_param(session, params)?;
    let layer = state.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let surface = layer.surface().filter(|_| matches!(layer.content, photocraft_doc::LayerContent::Raster(_) | photocraft_doc::LayerContent::Smart(_)));
    let surface = surface.ok_or_else(|| invalid("RAW Develop needs a pixel layer or smart object"))?;
    let settings = match fields.get("settings") {
        Some(value) => lens_cmds::raw_params(ID, value)?,
        None => CameraRaw::default(),
    };
    settings.validate().map_err(invalid)?;
    let encoding_gamma = lens_cmds::encoding_gamma(&state.doc);
    let proxy = Proxy::new(surface, state.doc.bounds()).map_err(EngineError::Other)?;
    let coverage = state.doc.selection.as_ref().map(|mask| proxy.sample_coverage(|x, y| mask.sample_channel(x, y, 0)));
    let image = Image::new(&proxy.pixels, proxy.width, proxy.height, coverage.as_deref())
        .ok_or_else(|| invalid("invalid RAW Develop preview"))?
        .with_encoding_gamma(encoding_gamma)
        .with_shadow_noise(proxy.shadow_noise(surface, encoding_gamma, coverage.as_deref()));
    let result = if panels.len() == Panel::ALL.len() {
        let float = surface.format().sample == photocraft_color::SampleType::F32;
        suggest_full(&image, &panels, &settings, float, |pixels, w, h, parameters| {
            let inputs = DevelopInputs { pixel_scale: w as f32 / proxy.full_width as f32, encoding_gamma, ..DevelopInputs::default() };
            develop(pixels, w, h, parameters, &inputs, float);
        })
    } else {
        suggest(&image, &panels, &settings)
    };
    Ok(json!({"layer": id.0, "settings": result}))
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Suggest RAW Develop Settings",
        menu: &[],
        shortcut: None,
        params: r#"{"panel":"all|light|whiteBalance|color|effects","layer":id?,"settings":{}?}"#,
        enabled: lens_cmds::raw_enabled,
        run,
        journal: false,
    }]
}

#[cfg(test)]
mod box_average_tests;
#[cfg(test)]
mod tests;
