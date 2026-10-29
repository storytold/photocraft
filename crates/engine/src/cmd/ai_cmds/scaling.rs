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
