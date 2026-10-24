//! One-time bounded capture of ordinary native contents for static native resources.
//! No source operation names, graph syntax, external reads or document rules live here.
use super::coverage::{self, NativeFilterDomain};
use super::{IoError, Result, invalid};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, SmartObject, SmartSource};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

struct Admission {
    remaining: usize,
    source: usize,
    nodes: usize,
}
impl Admission {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(|| invalid("native snapshot exceeds its remaining capture allowance"))?;
        Ok(())
    }
}
fn check(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| IoError::Cancelled)
}

/// Return owner-local native pixels. Admission and the caller's budget publish
/// only after the entire capture succeeds; original encoded contents are immutable.
pub(super) fn capture(bytes: &[u8], target: &Document, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    check(ctl)?;
    let mut admission = Admission { remaining: (*budget).min(super::MAX_PIXEL_BYTES), source: super::MAX_PIXEL_BYTES, nodes: super::MAX_LAYERS };
    let (document, surface) = render_bytes(bytes, 0, &mut admission, ctl)?;
    if document.size != target.size {
        return Err(invalid("native snapshot current contents frame differs from its validated owner"));
    }
    let pixels = project_source(&surface, &document, target.pixel_format(), &mut admission.remaining, ctl)?;
    check(ctl)?;
    *budget = admission.remaining;
    Ok(pixels)
}
fn native_kind(bytes: &[u8], budget: usize, ctl: &Interrupt<'_>) -> Result<bool> {
    check(ctl)?;
    if bytes.get(..2) != Some(b"PK") {
        return Ok(false);
    }
    let index = bytes.len().checked_mul(4).ok_or_else(|| invalid("native snapshot archive inspection overflows"))?;
    if index > budget {
        return Err(invalid("native snapshot archive inspection exceeds remaining capture allowance"));
    }
    // ZipReader metadata allocation is admitted before signature inspection too.
    let native = photocraft_format::is_pcraft(bytes);
    check(ctl)?;
    Ok(native)
}

fn render_bytes(bytes: &[u8], depth: usize, admission: &mut Admission, ctl: &Interrupt<'_>) -> Result<(Document, Surface)> {
    check(ctl)?;
    if depth > 64 {
        return Err(invalid("native snapshot exceeds 64 embedded native documents"));
    }
    admission.source = admission.source.checked_sub(bytes.len()).ok_or_else(|| invalid("native snapshot repeated embedded source bytes exceed limit"))?;
    // The native measured loader counts decoded tiles/blobs, not manifest
    // object allocations. Admit ZIP indexes and bounded JSON representation
    // separately before each load, including fan-out copies of earlier inputs.
    admission.charge(bytes.len().checked_mul(4).ok_or_else(|| invalid("native snapshot archive admission overflows"))?)?;
    let manifest_limit = super::MAX_METADATA_BYTES.min(admission.remaining / 64);
    let manifest_bytes = {
        let zip = photocraft_format::zip::ZipReader::new(bytes).map_err(|e| invalid(format!("native snapshot archive: {e}")))?;
        if zip.find("manifest.json").is_none() {
            return Err(invalid("uncached native snapshot contents require an embedded native document"));
        }
        let manifest = zip.read_by_name("manifest.json", manifest_limit).map_err(|e| invalid(format!("native snapshot manifest admission: {e}")))?;
        manifest.len()
    };
    admission.charge(manifest_bytes.checked_mul(64).ok_or_else(|| invalid("native snapshot metadata admission overflows"))?)?;
    check(ctl)?;
    let options = photocraft_format::LoadOptions {
        max_manifest_bytes: manifest_bytes,
        max_blob_bytes: admission.remaining,
        max_total_bytes: admission.remaining as u64,
        max_canvas_side: super::MAX_SIDE,
        max_canvas_pixels: (admission.remaining / 32) as u64,
        preserve_ids: false,
    };
    let (mut document, measured) =
        photocraft_format::load_from_bytes_measured(bytes, &options).map_err(|e| invalid(format!("native snapshot embedded contents: {e}")))?;
    admission.charge(usize::try_from(measured).map_err(|_| invalid("native snapshot decoded size exceeds platform limits"))?)?;
    check(ctl)?;
    // Layer data is prepared privately while the actual native document's frame,
    // profile, resources and typography remain available as immutable context.
    let mut layers = std::mem::take(&mut document.layers);
    for layer in &mut layers {
        prepare_layer(layer, &document, depth, 0, admission, ctl)?;
    }
    document.layers = layers;
    let format = document.pixel_format();
    let surface = coverage::render_prepared_document(&document, format, &mut admission.remaining, ctl)?;
    finite(&surface, document.bounds(), ctl)?;
    Ok((document, surface))
}
fn prepare_layer(layer: &mut Layer, document: &Document, depth: usize, groups: usize, admission: &mut Admission, ctl: &Interrupt<'_>) -> Result<()> {
    check(ctl)?;
    if groups > super::MAX_DEPTH {
        return Err(invalid("native snapshot exceeds native group depth"));
    }
    if !layer.visible {
        return Ok(());
    }
    match &mut layer.content {
        LayerContent::Group(group) => {
            admission.nodes = admission.nodes.checked_sub(1).ok_or_else(|| invalid("native snapshot layer count exceeds limit"))?;
            for child in &mut group.children {
                prepare_layer(child, document, depth, groups + 1, admission, ctl)?;
            }
            admit_styles(layer, document, &mut admission.remaining)?;
        }
        LayerContent::Smart(smart) => {
            admission.nodes = admission.nodes.checked_sub(1).ok_or_else(|| invalid("native snapshot layer count exceeds limit"))?;
            let native_bytes = match &smart.source {
                SmartSource::Embedded { bytes, .. } if native_kind(bytes, admission.remaining, ctl)? => Some(bytes.clone()),
                _ if smart.cache.is_some() => None,
                SmartSource::Linked { .. } => {
                    return Err(invalid("uncached linked native contents require explicit resource resolution; snapshot performs no external read"));
                }
                _ => return Err(invalid("uncached snapshot contents require an embedded native document; nonnative source decoding is not authorized")),
            };
            if let Some(bytes) = native_bytes {
                // Actual native model wins over a historical/stale cache.
                let (source, pixels) = render_bytes(&bytes, depth + 1, admission, ctl)?;
                let pixels = project_source(&pixels, &source, document.pixel_format(), &mut admission.remaining, ctl)?;
                let placed = place(&pixels, source.bounds(), smart, &mut admission.remaining, ctl)?;
                smart.cache = Some(replay_smart(&placed, smart, document, &mut admission.remaining, ctl)?);
            }
            admit_styles(layer, document, &mut admission.remaining)?;
        }
        _ => coverage::prepare_direct_layer(layer, document, &mut admission.remaining, groups, &mut admission.nodes, ctl)?,
    }
    check(ctl)
}
fn admit_styles(layer: &Layer, document: &Document, budget: &mut usize) -> Result<()> {
    if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) || !layer.fill_opacity.is_finite() || !(0.0..=1.0).contains(&layer.fill_opacity) {
        return Err(invalid("native snapshot layer opacity is invalid"));
    }
    let margin = photocraft_compose::effects::margin(layer).max(0);
    let b = document.bounds();
    let area = Rect::new(
        b.x0.checked_sub(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
        b.y0.checked_sub(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
        b.x1.checked_add(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
        b.y1.checked_add(margin).ok_or_else(|| invalid("native snapshot style extent overflows"))?,
    );
    for _ in &layer.effects.items {
        coverage::reserve_native_frame(area, document.pixel_format(), budget)?;
    }
    Ok(())
}
fn place(surface: &Surface, bounds: Rect, smart: &SmartObject, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    if smart.stack_mode.is_some() {
        return Err(invalid("native snapshot stack statistics need bounded native frame preparation; no source substitution is allowed"));
    }
    if smart.warp.as_ref().is_some_and(|warp| !warp.is_identity()) {
        return Err(invalid("native snapshot mesh warp needs bounded native mesh preparation; no source substitution is allowed"));
    }
    let [a, b, c, d, e, f] = smart.transform.m;
    if smart.perspective.is_none()
        && [a, b, c, d, e, f].iter().all(|v| v.is_finite())
        && (a - 1.).abs() < 1e-9
        && b.abs() < 1e-9
        && c.abs() < 1e-9
        && (d - 1.).abs() < 1e-9
        && (e - e.round()).abs() < 1e-9
        && (f - f.round()).abs() < 1e-9
    {
        if e.abs() > f64::from(i32::MAX) || f.abs() > f64::from(i32::MAX) {
            return Err(invalid("native snapshot integer placement exceeds coordinates"));
        }
        let (dx, dy) = (e.round() as i32, f.round() as i32);
        for (value, shift) in [(bounds.x0, dx), (bounds.x1, dx), (bounds.y0, dy), (bounds.y1, dy)] {
            value.checked_add(shift).ok_or_else(|| invalid("native snapshot translated frame overflows"))?;
        }
        coverage::reserve_native_frame(bounds.translate(dx, dy), surface.format(), budget)?;
        check(ctl)?;
        return Ok(photocraft_algo::resample::translate_surface(surface, dx, dy));
    }
    let matrix = photocraft_algo::transform::Homography(smart.perspective.unwrap_or([a, c, e, b, d, f, 0., 0., 1.]));
    coverage::native_projective_warp_full(surface, bounds, &matrix, budget, ctl)
}
fn replay_smart(placed: &Surface, smart: &SmartObject, document: &Document, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    if !smart.filters_enabled || !smart.smart_filters.iter().any(|f| f.visible) {
        return Ok(placed.clone());
    }
    if smart.smart_filters.len() > 64 {
        return Err(invalid("native snapshot Smart filter stack exceeds limit"));
    }
    let canvas = document.bounds();
    let object = placed.content_bounds();
    let extent = canvas.union(&object);
    let mut current = placed.clone();
    for filter in smart.smart_filters.iter().filter(|f| f.visible) {
        check(ctl)?;
        let reference = if matches!(
            filter.command.as_str(),
            "filter.render.clouds" | "filter.render.differenceClouds" | "filter.render.fibers" | "filter.render.lensFlare"
        ) && !object.is_empty()
        {
            object
        } else {
            canvas
        };
        let output = coverage::raw_filter(&current, filter, NativeFilterDomain { reference_bounds: reference, extent, document }, budget, ctl)?;
        current = if filter.blend == BlendMode::Normal && filter.opacity >= 1. {
            output
        } else {
            let mix_area = current.content_bounds().union(&output.content_bounds());
            coverage::reserve_native_frame(mix_area, current.format(), budget)?;
            photocraft_compose::smart_mix::mix(&current, &output, mix_area, filter.blend, filter.opacity, ctl).map_err(|e| {
                if e == photocraft_compose::smart_mix::Error::Cancelled { IoError::Cancelled } else { invalid(format!("native snapshot Smart strength: {e}")) }
            })?
        };
    }
    if let Some(mask) = smart.filter_mask.as_ref().filter(|m| m.enabled) {
        current = filter_mask(placed, &current, mask, budget, ctl)?;
    }
    Ok(current)
}
fn filter_mask(base: &Surface, filtered: &Surface, mask: &LayerMask, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    let area = base.content_bounds().union(&filtered.content_bounds());
    let fmt = base.format();
    coverage::reserve_native_frame(area, fmt, budget)?;
    // Local base/filter/result staging each owns a padded native tile, even
    // for a one-pixel mask. Logical frame scratch alone does not price these.
    if !area.is_empty() {
        let scratch = (photocraft_geom::TILE_SIZE as usize)
            .checked_mul(photocraft_geom::TILE_SIZE as usize)
            .and_then(|n| n.checked_mul(fmt.bytes_per_pixel()))
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| invalid("native filter-mask staging allocation overflows"))?;
        *budget = budget.checked_sub(scratch).ok_or_else(|| invalid("native filter-mask padded staging exceeds capture allowance"))?;
    }
    let mut output = filtered.clone();
    // Native filter-mask mixing is pixel-local. Invoke the existing policy on
    // bounded tiles, retaining its exact color/depth/alpha arithmetic and clamp.
    for tile in area.tiles() {
        check(ctl)?;
        let r = tile.rect().intersect(&area);
        let mut b = Surface::new(fmt);
        let mut f = Surface::new(fmt);
        b.try_write_region(r, &base.try_read_region(r).map_err(|e| invalid(format!("native mask base read: {e}")))?)
            .map_err(|e| invalid(format!("native mask base staging: {e}")))?;
        f.try_write_region(r, &filtered.try_read_region(r).map_err(|e| invalid(format!("native mask output read: {e}")))?)
            .map_err(|e| invalid(format!("native mask output staging: {e}")))?;
        let mixed = photocraft_compose::filter_mask::mix(&b, &f, mask);
        check(ctl)?;
        output
            .try_write_region(r, &mixed.try_read_region(r).map_err(|e| invalid(format!("native mask result read: {e}")))?)
            .map_err(|e| invalid(format!("native mask result write: {e}")))?;
    }
    Ok(output)
}
fn finite(surface: &Surface, bounds: Rect, ctl: &Interrupt<'_>) -> Result<()> {
    for y in bounds.y0..bounds.y1 {
        check(ctl)?;
        let row = surface.try_read_region(Rect::new(bounds.x0, y, bounds.x1, y + 1)).map_err(|e| invalid(format!("native snapshot sample read: {e}")))?;
        if row.iter().any(|v| !v.is_finite()) {
            return Err(invalid("native snapshot produced nonfinite pixels"));
        }
    }
    Ok(())
}

/// Mirror ordinary Smart source projection, including its established default
/// Gray working profile and integer-to-F32 transfer policy. No new parent-ICC rule.
fn project_source(surface: &Surface, source: &Document, target: PixelFormat, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    check(ctl)?;
    let bounds = source.bounds();
    let gray = target.mode == ColorMode::Grayscale && source.pixel_format().mode != ColorMode::Grayscale;
    let linear = target.sample == SampleType::F32 && source.depth != SampleType::F32;
    let mut current = surface.clone();
    if gray {
        use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
        let space = match source.mode {
            ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => ColorSpace::Gray,
            ColorMode::Cmyk => ColorSpace::Cmyk,
            ColorMode::Lab => ColorSpace::Lab,
            _ => ColorSpace::Rgb,
        };
        let profile = source.icc_profile.as_ref().and_then(|b| Profile::parse(b).ok()).filter(|p| p.color_space == space);
        let fallback = photocraft_cms::builtin::default_for(space).unwrap_or(Builtin::Srgb.profile());
        let transform = Transform::new(profile.as_ref().unwrap_or(fallback), Builtin::DotGain20Compat.profile(), Intent::RelativeColorimetric, true)
            .map_err(|e| invalid(format!("native Smart Gray projection: {e}")))?;
        let fmt = PixelFormat::new(ColorMode::Grayscale, source.depth, true);
        coverage::reserve_native_frame(bounds, fmt, budget)?;
        current = cms_gray(&current, fmt, &transform, bounds, ctl)?;
    }
    if linear {
        let old = current.format();
        coverage::reserve_native_frame(bounds, PixelFormat { sample: SampleType::F32, ..old }, budget)?;
        current = current.try_convert(PixelFormat { sample: SampleType::F32, ..old }).map_err(|e| invalid(format!("native Smart depth projection: {e}")))?;
        if matches!(current.format().mode, ColorMode::Rgb | ColorMode::Grayscale) {
            let transfer = photocraft_compose::adjust::Transfer::for_document(current.format().mode, source.depth);
            let fmt = current.format();
            let colors = fmt.mode.color_channels();
            for y in bounds.y0..bounds.y1 {
                check(ctl)?;
                let r = Rect::new(bounds.x0, y, bounds.x1, y + 1);
                let mut row = current.try_read_region(r).map_err(|e| invalid(format!("native linear projection read: {e}")))?;
                for pixel in row.chunks_exact_mut(fmt.channels()) {
                    for value in pixel.iter_mut().take(colors) {
                        *value = transfer.decode(*value);
                    }
                }
                current.try_write_region(r, &row).map_err(|e| invalid(format!("native linear projection write: {e}")))?;
            }
        }
    }
    if current.format() != target {
        coverage::reserve_native_frame(bounds, target, budget)?;
        current = current.try_convert(target).map_err(|e| invalid(format!("native Smart model projection: {e}")))?;
    }
    finite(&current, bounds, ctl)?;
    Ok(current)
}
fn cms_gray(source: &Surface, target: PixelFormat, transform: &photocraft_cms::Transform, bounds: Rect, ctl: &Interrupt<'_>) -> Result<Surface> {
    let mut output = Surface::new(target);
    let from = source.format();
    for y in bounds.y0..bounds.y1 {
        check(ctl)?;
        let r = Rect::new(bounds.x0, y, bounds.x1, y + 1);
        let values = source.try_read_region(r).map_err(|e| invalid(format!("native CMS projection read: {e}")))?;
        let count = (bounds.width() as usize).checked_mul(target.channels()).ok_or_else(|| invalid("native CMS projection row overflows"))?;
        let mut converted = Vec::new();
        converted.try_reserve_exact(count).map_err(|_| invalid("native CMS projection row unavailable"))?;
        converted.resize(count, 0.);
        match from.sample {
            SampleType::F32 => transform.convert_f32(&values, from.channels(), &mut converted, target.channels(), true),
            SampleType::U8 => {
                for (pixel, out) in values.chunks_exact(from.channels()).zip(converted.chunks_exact_mut(target.channels())) {
                    let mut src = [0u8; 5];
                    for (sample, value) in src.iter_mut().zip(pixel) {
                        *sample = (value.clamp(0., 1.) * 255. + 0.5) as u8;
                    }
                    let mut dst = [0u8; 2];
                    let input = src.get(..from.channels()).ok_or_else(|| invalid("native CMS source channel count exceeds supported color layout"))?;
                    transform.convert_u8(input, from.channels(), &mut dst, target.channels(), true);
                    for (sample, value) in out.iter_mut().zip(dst) {
                        *sample = f32::from(value) / 255.;
                    }
                }
            }
            SampleType::U16 => {
                for (pixel, out) in values.chunks_exact(from.channels()).zip(converted.chunks_exact_mut(target.channels())) {
                    let mut src = [0u16; 5];
                    for (sample, value) in src.iter_mut().zip(pixel) {
                        *sample = (value.clamp(0., 1.) * 65535. + 0.5) as u16;
                    }
                    let mut dst = [0u16; 2];
                    let input = src.get(..from.channels()).ok_or_else(|| invalid("native CMS source channel count exceeds supported color layout"))?;
                    transform.convert_u16(input, from.channels(), &mut dst, target.channels(), true);
                    for (sample, value) in out.iter_mut().zip(dst) {
                        *sample = f32::from(value) / 65535.;
                    }
                }
            }
        }
        output.try_write_region(r, &converted).map_err(|e| invalid(format!("native CMS projection write: {e}")))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::Color;
    use photocraft_doc::{Affine, Fill, Path, ShapeLayer, SmartFilter, Subpath};
    use photocraft_geom::Size;
    use std::sync::Arc;
    fn raster(mode: ColorMode, depth: SampleType) -> Document {
        let mut doc = Document::new("Native snapshot input", Size::new(3, 2), mode, depth);
        let mut layer = Layer::raster("Native actual pixels", doc.pixel_format());
        let values = if mode == ColorMode::Rgb { vec![0.1, 0.3, 0.6, 0.5] } else { vec![0.3, 0.5] };
        layer.surface_mut().unwrap().fill_rect(doc.bounds(), &values);
        doc.layers.push(layer);
        doc
    }
    fn bytes(doc: &Document) -> Vec<u8> {
        photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()
    }
    fn assert_pixels(a: &Surface, b: &Surface, bounds: Rect) {
        assert_eq!(a.format(), b.format());
        assert_eq!(a.read_region(bounds), b.read_region(bounds));
    }
    #[test]
    fn snapshot_replays_native_smart_strength_and_filter_mask_without_doubling_alpha() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let source = raster(mode, depth);
                let mut doc = source.clone();
                doc.layers.clear();
                let filter = SmartFilter {
                    command: "image.adjustments.invert".into(),
                    params: serde_json::json!({}),
                    visible: true,
                    opacity: 0.4,
                    blend: BlendMode::Multiply,
                };
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.5]);
                let mut smart =
                    SmartObject::new(SmartSource::Embedded { file_name: "actual.pcraft".into(), bytes: Arc::new(bytes(&source)) }, Affine::IDENTITY, None);
                smart.smart_filters = vec![filter.clone()];
                smart.filter_mask = Some(mask.clone());
                let mut stale = Surface::new(source.pixel_format());
                let stale_values = if mode == ColorMode::Rgb { vec![0.9, 0.9, 0.9, 1.] } else { vec![0.9, 1.] };
                stale.fill_rect(source.bounds(), &stale_values);
                smart.cache = Some(stale); // current embedded data must override this historical cache

                doc.layers.push(Layer::new("Actual native editable Smart", LayerContent::Smart(smart)));
                let mut allowance = super::super::MAX_PIXEL_BYTES;
                let got = capture(&bytes(&doc), &doc, &mut allowance, &Interrupt::NONE).unwrap();
                let original = source.layers[0].surface().unwrap();
                let mut ordinary = photocraft_compose::render(&source, source.bounds());
                photocraft_compose::adjust::apply_in(
                    &photocraft_doc::Adjustment::Invert,
                    &mut ordinary,
                    photocraft_compose::adjust::Transfer::for_document(mode, depth),
                    if depth == SampleType::F32 { Some(depth) } else { None },
                    mode,
                );
                let mut inverted = Surface::new(doc.pixel_format());
                let samples = ordinary.px.iter().flat_map(|pixel| photocraft_raster::from_rgba(&doc.pixel_format(), *pixel)).collect::<Vec<_>>();
                inverted.write_region(doc.bounds(), &samples);
                let mixed = photocraft_compose::smart_mix::mix(original, &inverted, doc.bounds(), filter.blend, filter.opacity, &Interrupt::NONE).unwrap();
                let masked = photocraft_compose::filter_mask::mix(original, &mixed, &mask);
                assert_pixels(&got, &masked, doc.bounds());
                assert!(
                    (got.sample_channel(0, 0, got.channels() - 1) - original.sample_channel(0, 0, original.channels() - 1)).abs() < 0.005,
                    "native filter strength must preserve coverage"
                );
            }
        }
    }
    #[test]
    fn snapshot_uses_native_projection_depth_profile_and_retains_prepared_geometry() {
        let source = raster(ColorMode::Rgb, SampleType::U8);
        let mut doc = source.clone();
        doc.depth = SampleType::F32;
        doc.layers.clear();
        doc.layers.push(Layer::new(
            "Native depth projection",
            LayerContent::Smart(SmartObject::new(
                SmartSource::Embedded { file_name: "integer.pcraft".into(), bytes: Arc::new(bytes(&source)) },
                Affine::IDENTITY,
                None,
            )),
        ));
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let got = capture(&bytes(&doc), &doc, &mut allowance, &Interrupt::NONE).unwrap();
        let transfer = photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, SampleType::U8);
        assert!((got.sample_channel(0, 0, 0) - transfer.decode(source.layers[0].surface().unwrap().sample_channel(0, 0, 0))).abs() < 1e-6);
        assert!((got.sample_channel(0, 0, 3) - source.layers[0].surface().unwrap().sample_channel(0, 0, 3)).abs() < 1e-6);
        let mut gray = source.clone();
        gray.mode = ColorMode::Grayscale;
        gray.icc_profile = Some(photocraft_cms::Builtin::SGray.profile().to_bytes());
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let got = capture(&bytes(&source), &gray, &mut allowance, &Interrupt::NONE).unwrap();
        let transform = photocraft_cms::Transform::new(
            photocraft_cms::Builtin::Srgb.profile(),
            photocraft_cms::Builtin::DotGain20Compat.profile(),
            photocraft_cms::Intent::RelativeColorimetric,
            true,
        )
        .unwrap();
        let pixel = source.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
        let input = pixel.iter().map(|v| (v * 255. + 0.5) as u8).collect::<Vec<_>>();
        let mut output = [0u8; 2];
        transform.convert_u8(&input, 4, &mut output, 2, true);
        assert!(
            (got.sample_channel(0, 0, 0) - f32::from(output[0]) / 255.).abs() < 1e-6,
            "ordinary Smart Gray projection uses native default working profile, not a new assigned-parent rule"
        );
        let mut shape = Document::new("Editable native vector", Size::new(12, 12), ColorMode::Rgb, SampleType::U16);
        shape.layers.push(Layer::new(
            "Native vector",
            LayerContent::Shape(ShapeLayer {
                path: Path::new(vec![Subpath::polygon(&[(1., 1.), (10., 1.), (10., 10.), (1., 10.)])]),
                fill: Some(Fill::Solid(Color::rgb(0.2, 0.5, 0.8))),
                ..Default::default()
            }),
        ));
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let got = capture(&bytes(&shape), &shape, &mut allowance, &Interrupt::NONE).unwrap();
        assert!(got.sample_channel(5, 5, 3) > 0.99);
        assert_eq!(shape.layers[0].name, "Native vector");
    }
    #[test]
    fn snapshot_never_resolves_external_locators_and_failure_budget_is_atomic() {
        let source = raster(ColorMode::Rgb, SampleType::F32);
        let original = bytes(&source);
        let mut linked = source.clone();
        linked.layers = vec![Layer::new(
            "No external permission",
            LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: "C:/not-authorized/native-snapshot.pcraft".into() }, Affine::IDENTITY, None)),
        )];
        let mut allowance = super::super::MAX_PIXEL_BYTES;
        let before = allowance;
        assert!(capture(&bytes(&linked), &linked, &mut allowance, &Interrupt::NONE).is_err());
        assert_eq!(allowance, before);
        let LayerContent::Smart(smart) = &mut linked.layers[0].content else {
            panic!("Smart");
        };
        smart.cache = source.layers[0].surface().cloned();
        let captured = capture(&bytes(&linked), &linked, &mut allowance, &Interrupt::NONE).unwrap();
        assert_pixels(&captured, source.layers[0].surface().unwrap(), source.bounds());
        let mut zero = 0;
        assert!(capture(&original, &source, &mut zero, &Interrupt::NONE).is_err());
        assert_eq!(zero, 0);
        let mut allowance = before;
        let cancel = || true;
        assert!(matches!(capture(&original, &source, &mut allowance, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(allowance, before);
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 5;
        assert!(matches!(capture(&original, &source, &mut allowance, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(allowance, before);
        assert_eq!(bytes(&source), original);
    }
    #[test]
    fn tiled_filter_mask_matches_native_density_hdr_and_tile_boundary_policy() {
        let bounds = Rect::new(255, 0, 258, 1);
        for alpha in [false, true] {
            let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, alpha);
            let mut base = Surface::new(format);
            let mut filtered = Surface::new(format);
            let before = if alpha { vec![3., -2., 1., 0.4] } else { vec![3., -2., 1.] };
            let after = if alpha { vec![1.5, 2., -1., 0.7] } else { vec![1.5, 2., -1.] };
            base.fill_rect(bounds, &before);
            filtered.fill_rect(bounds, &after);
            let mask = LayerMask { surface: Surface::with_default(PixelFormat::GRAY8, &[0.2]), density: 0.35, feather: 3., ..LayerMask::reveal_all() };
            let expected = photocraft_compose::filter_mask::mix(&base, &filtered, &mask);
            let mut budget = super::super::MAX_PIXEL_BYTES;
            let got = filter_mask(&base, &filtered, &mask, &mut budget, &Interrupt::NONE).unwrap();
            assert_pixels(&got, &expected, bounds);
            if alpha {
                assert!((0.0..=1.0).contains(&got.sample_channel(255, 0, 0)));
            } else {
                assert!(got.sample_channel(255, 0, 0) > 1., "ordinary alpha-absent native mask policy retains HDR color");
            }
            let mut tiny = 0;
            assert!(filter_mask(&base, &filtered, &mask, &mut tiny, &Interrupt::NONE).is_err());
            assert_eq!(tiny, 0);
        }
    }
}
