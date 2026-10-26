//! Native destination mapping. Provider buffers and display previews stay bounded.
use super::*;
use photocraft_algo::resample::{Resample, resize_surface_in_canvas};

pub(super) fn check_destination(roi: Rect, format: PixelFormat, mask: bool) -> Result<usize> {
    if roi.is_empty() || roi.x0 < 0 || roi.y0 < 0 {
        return Err(error("Invalid AI destination coordinates"));
    }
    let columns = u64::from(roi.width()).div_ceil(256).checked_add(1);
    let rows = u64::from(roi.height()).div_ceil(256).checked_add(1);
    let bytes = columns
        .and_then(|x| x.checked_mul(rows?))
        .and_then(|n| n.checked_mul(256 * 256))
        .and_then(|n| n.checked_mul((format.bytes_per_pixel() + usize::from(mask) * 4) as u64))
        .and_then(|n| usize::try_from(n).ok());
    bytes.filter(|n| *n <= CACHE_BYTES).ok_or_else(|| error("AI destination exceeds the native result memory budget; select a smaller area"))
}

pub fn working_size(target: ImageSize, longest: u32) -> Result<ImageSize> {
    if target.width == 0 || target.height == 0 || longest == 0 {
        return Err(error("AI dimensions must be positive"));
    }
    let edge = target.width.max(target.height);
    let scale = (f64::from(longest) / f64::from(edge)).min(1.0);
    let size =
        ImageSize { width: (f64::from(target.width) * scale).round().max(1.0) as u32, height: (f64::from(target.height) * scale).round().max(1.0) as u32 };
    size.pixels().map_err(error)?;
    Ok(size)
}

fn resize(surface: &Surface, from: ImageSize, to: ImageSize, filter: Resample) -> Result<Surface> {
    if from.width == 0 || from.height == 0 || to.width == 0 || to.height == 0 {
        return Err(error("Invalid AI resampling dimensions"));
    }
    let run = || {
        resize_surface_in_canvas(
            surface,
            f64::from(to.width) / f64::from(from.width),
            f64::from(to.height) / f64::from(from.height),
            filter,
            Rect::from_xywh(0, 0, from.width, from.height),
        )
    };
    // Surface resampling groups bands by the Rayon pool size. Bound its scratch space
    // independently of the number of cores on the host (particularly for an 8K result).
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::ThreadPoolBuilder::new().num_threads(2).build().map_err(error).map(|pool| pool.install(run))
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(run())
    }
}

pub(super) fn reduce_surface(surface: &Surface, roi: Rect, to: ImageSize) -> Result<Surface> {
    let local = surface.translated(-roi.x0, -roi.y0, roi);
    if to == (ImageSize { width: roi.width(), height: roi.height() }) {
        return Ok(local);
    }
    resize(&local, ImageSize { width: roi.width(), height: roi.height() }, to, Resample::Lanczos)
}

pub(super) fn transport_mask(surface: &Surface, roi: Rect, to: ImageSize) -> Result<Mask> {
    let reduced = reduce_surface(surface, roi, to)?;
    let values = reduced.read_region(Rect::from_xywh(0, 0, to.width, to.height));
    Mask::new(to, values.into_iter().map(|v| v.clamp(0.0, 1.0)).collect()).map_err(error)
}

pub(super) fn native_selection(surface: &Surface, roi: Rect) -> Result<Surface> {
    if surface.default_pixel().iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(error("AI selection coverage must be finite and between zero and one"));
    }
    for (tile, _) in surface.tiles() {
        let rect = tile.rect().intersect(&roi);
        if !rect.is_empty() && surface.read_region(rect).iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(error("AI selection coverage must be finite and between zero and one"));
        }
    }
    Ok(surface.translated(0, 0, roi))
}

pub(super) fn mapped_surface(format: PixelFormat, profile: &Profile, roi: Rect, image: &RgbaImage) -> Result<Surface> {
    if image.size() == (ImageSize { width: roi.width(), height: roi.height() }) {
        return to_surface(format, profile, roi, image.samples(), Builtin::Srgb.profile());
    }
    let local_rect = Rect::from_xywh(0, 0, image.size().width, image.size().height);
    let local = to_surface(format, profile, local_rect, image.samples(), Builtin::Srgb.profile())?;
    let native = resize(&local, image.size(), ImageSize { width: roi.width(), height: roi.height() }, Resample::Bicubic)?;
    Ok(native.translated(roi.x0, roi.y0, Rect::from_xywh(0, 0, roi.width(), roi.height())))
}

pub(super) fn mapped_mask(mask: &Mask, roi: Rect) -> Result<Surface> {
    let mut local = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    local.write_region(Rect::from_xywh(0, 0, mask.size().width, mask.size().height), mask.values());
    let native = resize(&local, mask.size(), ImageSize { width: roi.width(), height: roi.height() }, Resample::Bilinear)?;
    Ok(native.translated(roi.x0, roi.y0, Rect::from_xywh(0, 0, roi.width(), roi.height())))
}

/// x4 detail restoration with a 64-source-pixel halo per patch. Only the central
/// native pixels are retained, so model padding remains outside the patch seam.
pub(super) fn detail_surface(
    format: PixelFormat,
    profile: &Profile,
    roi: Rect,
    image: &RgbaImage,
    upscale: &mut dyn FnMut(&RgbaImage) -> Result<RgbaImage>,
) -> Result<Surface> {
    let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
    source.write_region(Rect::from_xywh(0, 0, image.size().width, image.size().height), image.samples());
    detail_native(format, profile, roi, DetailSource { surface: &source, size: image.size(), profile: Builtin::Srgb.profile() }, upscale)
}

pub(super) struct DetailSource<'a> {
    pub surface: &'a Surface,
    pub size: ImageSize,
    pub profile: &'a Profile,
}

pub(super) fn detail_native(
    format: PixelFormat,
    profile: &Profile,
    roi: Rect,
    source: DetailSource<'_>,
    upscale: &mut dyn FnMut(&RgbaImage) -> Result<RgbaImage>,
) -> Result<Surface> {
    check_destination(roi, format, false)?;
    const PATCH: i32 = 2048;
    const HALO: i32 = 64;
    let pre_size = ImageSize { width: roi.width().div_ceil(4), height: roi.height().div_ceil(4) };
    let pre = resize(source.surface, source.size, pre_size, Resample::Lanczos)?;
    let source_bounds = Rect::from_xywh(0, 0, source.size.width, source.size.height);
    let mut alpha = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    for tile in source_bounds.tiles() {
        let rect = tile.rect().intersect(&source_bounds);
        let values: Vec<f32> = source
            .surface
            .read_region(rect)
            .chunks_exact(source.surface.channels())
            .map(|pixel| if source.surface.format().alpha { pixel.last().copied().unwrap_or(0.0) } else { 1.0 })
            .collect();
        alpha.write_region(rect, &values);
    }
    let alpha = resize(&alpha, source.size, ImageSize { width: roi.width(), height: roi.height() }, Resample::Bilinear)?;
    let to_srgb = transform::cached(source.profile, Builtin::Srgb.profile(), Default::default()).map_err(error)?;
    let pre_bounds = Rect::from_xywh(0, 0, pre_size.width, pre_size.height);
    let mut native = Surface::new(format);
    let mut y = 0i32;
    while y < roi.height() as i32 {
        let mut x = 0i32;
        while x < roi.width() as i32 {
            let core = Rect::new(x, y, x.saturating_add(PATCH).min(roi.width() as i32), y.saturating_add(PATCH).min(roi.height() as i32));
            let read = Rect::new(
                core.x0 / 4 - HALO,
                core.y0 / 4 - HALO,
                core.x1.div_euclid(4) + i32::from(core.x1 % 4 != 0) + HALO,
                core.y1.div_euclid(4) + i32::from(core.y1 % 4 != 0) + HALO,
            )
            .intersect(&pre_bounds);
            let raw = pre.read_region(read);
            let mut samples = vec![
                0.0;
                (read.width() as usize)
                    .checked_mul(read.height() as usize)
                    .and_then(|n| n.checked_mul(4))
                    .ok_or_else(|| error("Upscale patch allocation overflow"))?
            ];
            to_srgb.convert_f32(&raw, pre.channels(), &mut samples, 4, true);
            for value in &mut samples {
                *value = value.clamp(0.0, 1.0);
            }
            let patch = RgbaImage::new(ImageSize { width: read.width(), height: read.height() }, samples).map_err(error)?;
            let restored = upscale(&patch)?;
            let expected = ImageSize {
                width: read.width().checked_mul(4).ok_or_else(|| error("AI upscale size overflow"))?,
                height: read.height().checked_mul(4).ok_or_else(|| error("AI upscale size overflow"))?,
            };
            if restored.size() != expected {
                return Err(error("AI upscaler returned unexpected patch dimensions"));
            }
            let origin = (read.x0 * 4, read.y0 * 4);
            let converted = to_surface(format, profile, Rect::from_xywh(0, 0, expected.width, expected.height), restored.samples(), Builtin::Srgb.profile())?;
            for tile in core.tiles() {
                let rect = tile.rect().intersect(&core);
                let local = rect.translate(-origin.0, -origin.1);
                let mut pixels = converted.read_region(local);
                if format.alpha {
                    for (pixel, coverage) in pixels.chunks_exact_mut(format.channels()).zip(alpha.read_region(rect)) {
                        if let Some(value) = pixel.last_mut() {
                            *value = coverage.clamp(0.0, 1.0);
                        }
                    }
                }
                native.write_region(rect.translate(roi.x0, roi.y0), &pixels);
            }
            x = core.x1;
        }
        y = y.saturating_add(PATCH);
    }
    Ok(native)
}

pub(super) fn multiply_masks(mut mask: Surface, selection: &Surface, roi: Rect) -> Surface {
    for tile in roi.tiles() {
        let rect = tile.rect().intersect(&roi);
        let mut values = mask.read_region(rect);
        let native = selection.read_region(rect);
        for (value, selected) in values.iter_mut().zip(native) {
            *value = value.clamp(0.0, 1.0) * selected;
        }
        mask.write_region(rect, &values);
    }
    mask
}

pub(super) fn native_composite(doc: &Document, roi: Rect) -> Result<Surface> {
    let mut native = Surface::new(doc.pixel_format());
    let profile = document_profile(doc)?;
    let from = crate::color_cmds::composite_profile(doc);
    let band_height = (2 * 1024 * 1024u32).checked_div(roi.width()).unwrap_or(1).clamp(1, 256);
    photocraft_compose::render_bands(doc, roi, band_height as i32, |band| -> Result<()> {
        let pixels: Vec<f32> = band.px.into_iter().flatten().collect();
        let converted = to_surface(doc.pixel_format(), &profile, band.rect, &pixels, &from)?;
        native.write_region(band.rect, &converted.read_region(band.rect));
        Ok(())
    })?;
    Ok(native)
}

pub(super) fn preview(doc: &Document, roi: Rect) -> Result<PreviewImage> {
    let size = working_size(ImageSize { width: roi.width(), height: roi.height() }, 1024)?;
    let pixels: Vec<f32> = photocraft_compose::render_reduced_rect(doc, roi, size.width, size.height).px.into_iter().flatten().collect();
    let mut converted = vec![0.0; pixels.len()];
    let t = transform::cached(&crate::color_cmds::composite_profile(doc), Builtin::Srgb.profile(), Default::default()).map_err(error)?;
    t.convert_f32(&pixels, 4, &mut converted, 4, true);
    if converted.iter().any(|v| !v.is_finite()) {
        return Err(error("AI preview has non-finite pixels"));
    }
    Ok(PreviewImage { size, rgba: converted.into_iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect() })
}
