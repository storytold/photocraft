//! Native bounds for one immutable transform capture. No scalar pixel buffers are extracted.
use super::{bad, native_area, source_budget, validate_area};
use crate::Result;
use photocraft_algo::transform::Homography;
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};
use serde_json::Value;
use std::sync::Arc;

pub struct AffinePreviewBounds {
    doc: Arc<Document>,
    layer: Option<LayerId>,
    colors: Option<crate::color_channels::ColorChannels>,
    background: [f32; 4],
    format: PixelFormat,
    source: Rect,
    original: Option<Rect>,
    exact: Option<Vec<Rect>>,
    retained_source_bytes: u64,
}

impl AffinePreviewBounds {
    pub fn new(doc: Arc<Document>, layer: Option<LayerId>, params: &Value, background: [f32; 4]) -> Result<Self> {
        let generation = photocraft_raster::spill::read_error_generation();
        let captured = Self::capture(doc.clone(), layer, params, background);
        super::check_capture(&doc, generation)?;
        captured
    }

    fn capture(doc: Arc<Document>, layer: Option<LayerId>, params: &Value, background: [f32; 4]) -> Result<Self> {
        let colors = crate::color_channels::explicit(params, crate::channel_cmds::color_count(&doc))?;
        let retained_source_bytes = colors.map_or(Ok(0), |colors| source_budget(&doc, layer, colors))?;
        let (source, format, original) = if let Some(colors) = colors {
            let area = native_area(&doc, layer)?;
            validate_area(area)?;
            let surface = doc.layer(layer.ok_or_else(|| bad("no active layer"))?).and_then(|l| l.surface()).ok_or_else(|| bad("missing raster"))?;
            if colors.indices().any(|k| k >= surface.format().mode.color_channels()) {
                return Err(bad("colour channel index is out of range"));
            }
            (area, PixelFormat::new(ColorMode::Grayscale, surface.format().sample, false), Some(area))
        } else {
            let surface = crate::transform_cmds::lone_target(&doc, layer, params)?.ok_or_else(|| bad("missing scalar target"))?;
            (photocraft_compose::bounds::content_bounds(surface), surface.format(), None)
        };
        let source = doc.selection.as_ref().map_or(source, Surface::content_bounds);
        let exact = (colors.is_none() || doc.selection.is_some()).then(|| vec![source]);
        Ok(Self { doc, layer, colors, background, format, source, original, exact, retained_source_bytes })
    }

    pub fn check(&mut self, h: &Homography) -> Result<()> {
        super::retry_capture(&self.doc)?;
        crate::transform_cmds::check_source_budget(self.retained_source_bytes)?;
        if let Some(selection) = &self.doc.selection {
            crate::transform_cmds::check_channel_geometry(self.source, selection.format(), h, None).map_err(|e| bad(e.to_string()))?;
        }
        if self.exact.is_none() {
            if crate::transform_cmds::check_channel_geometry(self.source, self.format, h, self.original).is_ok() {
                return Ok(());
            }
            // The enclosing native area can include default-valued samples the worker skips.
            // Calculate exact component bounds once, directly from encoded tiles, and reuse
            // them through every subsequent geometry change, including refused drags.
            let surface = self.doc.layer(self.layer.ok_or_else(|| bad("no active layer"))?).and_then(|l| l.surface()).ok_or_else(|| bad("missing raster"))?;
            let space = photocraft_compose::cmyk_space(&self.doc);
            let native = photocraft_color::convert::with_cmyk_space(space.as_ref(), || from_rgba(&surface.format(), self.background));
            let mut bounds = Vec::new();
            let generation = photocraft_raster::spill::read_error_generation();
            if let Some(colors) = self.colors {
                for k in colors.indices() {
                    let fill = native.get(k).copied().ok_or_else(|| bad("missing background component"))?;
                    let default = Surface::with_default(self.format, &[fill]);
                    bounds.push(component_bounds(surface, self.source, k, default.default_bytes())?);
                }
            }
            super::check_capture(&self.doc, generation)?;
            self.exact = Some(bounds);
        }
        for &source in self.exact.as_deref().ok_or_else(|| bad("missing native preview bounds"))? {
            crate::transform_cmds::check_channel_geometry(source, self.format, h, self.original).map_err(|e| bad(e.to_string()))?;
        }
        Ok(())
    }
}

fn component_bounds(surface: &Surface, area: Rect, k: usize, default: &[u8]) -> Result<Rect> {
    let format = surface.format();
    let bytes = format.sample.bytes();
    let offset = k.checked_mul(bytes).ok_or_else(|| bad("colour offset overflow"))?;
    let base = surface.default_bytes().get(offset..offset + bytes).ok_or_else(|| bad("missing default sample"))?;
    let mut bounds = Rect::EMPTY;
    for tile in area.tiles() {
        let rect = tile.rect().intersect(&area);
        if bounds.contains_rect(&rect) {
            continue;
        }
        let Some(tile_data) = surface.tile(tile) else {
            if base != default {
                bounds = bounds.union(&rect);
            }
            continue;
        };
        let data = tile_data.try_bytes().map_err(crate::EngineError::Other)?;
        let origin = tile.rect();
        for y in rect.y0..rect.y1 {
            let mut x = rect.x0;
            while x < rect.x1 {
                if bounds.contains(x, y) {
                    x = bounds.x1.min(rect.x1);
                    continue;
                }
                let pixel = (y - origin.y0) as usize * photocraft_geom::TILE_SIZE as usize + (x - origin.x0) as usize;
                let start = pixel * format.bytes_per_pixel() + offset;
                let sample = data.get(start..start + bytes).ok_or_else(|| bad("missing native sample"))?;
                if sample != default {
                    bounds = bounds.union(&Rect::new(x, y, x + 1, y + 1));
                }
                x += 1;
            }
        }
    }
    Ok(bounds)
}

#[cfg(test)]
mod tests {
    use super::super::{plane, tests::session};
    use super::*;
    use photocraft_color::SampleType;
    use serde_json::json;

    #[test]
    fn encoded_component_bounds_match_the_extracted_native_plane() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for background in [[1.0; 4], [0.0, 0.0, 0.0, 1.0], [0.3, 0.6, 0.9, 1.0]] {
                    let mut s = session(mode, sample);
                    s.edit("sparse encoded bounds", |doc, id| {
                        let dst = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                        *dst = Surface::new(dst.format());
                        for (x, y) in [(-257, -2), (-1, 3), (258, 7)] {
                            let mut pixel = vec![0.5; dst.channels()];
                            if sample == SampleType::F32 {
                                pixel[0] = -0.0;
                                pixel[1] = 1.75;
                            }
                            dst.write_pixel(x, y, &pixel);
                        }
                        Ok(())
                    })
                    .unwrap();
                    let st = s.active().unwrap();
                    let src = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
                    let area = native_area(&st.doc, st.active_layer).unwrap();
                    for k in 0..mode.color_channels() {
                        let plane = plane(&st.doc, st.active_layer, &json!({"target":{"color":k}}), background).unwrap().unwrap();
                        assert_eq!(
                            component_bounds(src, area, k, plane.default_bytes()).unwrap(),
                            plane.content_bounds(),
                            "{mode:?}/{sample:?}/{background:?}/{k}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn exact_bounds_are_reused_for_different_refused_geometries() {
        let s = session(ColorMode::Rgb, SampleType::F32);
        let st = s.active().unwrap();
        let mut bounds = AffinePreviewBounds::new(st.doc.clone(), st.active_layer, &json!({"target":{"colors":[0,2]}}), s.tools.background).unwrap();
        let h = |dx| Homography([1.0, 0.0, dx, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(bounds.check(&h(2_000_000.0)).is_err());
        let cached = bounds.exact.as_ref().unwrap().as_ptr();
        assert!(bounds.check(&h(3_000_000.0)).is_err());
        assert_eq!(cached, bounds.exact.as_ref().unwrap().as_ptr());
        assert!(bounds.check(&h(2.0)).is_ok());
    }
}
