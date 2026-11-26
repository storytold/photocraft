//! Existing native pixel paths, with private scene admission and no source evaluator.
use super::{Adjustment, Buffer, Document, Interrupt, Rect, Refusal, Result, SmartFilter, Surface, Transfer, add, adjust, check, mul, native_adjustment};
use photocraft_algo::FilterParams;
pub(super) fn params(f: &SmartFilter) -> Option<Result<FilterParams>> {
    if !matches!(
        f.command.as_str(),
        "filter.blur.gaussianBlur" | "filter.blur.boxBlur" | "filter.blur.motionBlur" | "filter.sharpen.unsharpMask" | "filter.other.highPass"
    ) {
        return None;
    }
    let parse = || {
        let p = f.params.as_object().ok_or(Refusal::Unsupported("native spatial object"))?;
        let n = |key: &str, low: f64, high: f64| {
            p.get(key)
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite() && (low..=high).contains(v))
                .map(|v| v as f32)
                .ok_or(Refusal::Unsupported("native spatial scalar"))
        };
        Ok(match f.command.as_str() {
            "filter.blur.gaussianBlur" if p.len() == 1 => FilterParams::GaussianBlur { radius: n("radius", 0.1, 1000.)? },
            "filter.blur.boxBlur" if p.len() == 1 => FilterParams::BoxBlur { radius: n("radius", 1., 2000.)? },
            "filter.blur.motionBlur" if p.len() == 2 => FilterParams::MotionBlur { angle: n("angle", -360., 360.)?, distance: n("distance", 1., 2000.)? },
            "filter.sharpen.unsharpMask" if p.len() == 3 => {
                FilterParams::UnsharpMask { radius: n("radius", 0.1, 1000.)?, amount: n("amount", 1., 500.)?, threshold: n("threshold", 0., 255.)? }
            }
            "filter.other.highPass" if p.len() == 1 => FilterParams::HighPass { radius: n("radius", 0.1, 1000.)? },
            _ => return Err(Refusal::Unsupported("native spatial keys")),
        })
    };
    Some(parse())
}
pub(super) struct Admission {
    pub temporary: usize,
    pub work: usize,
    pub retained: usize,
}
fn storage(area: Rect, format: &photocraft_color::PixelFormat) -> Result<usize> {
    let bytes =
        photocraft_raster::tile_storage_bytes(area, format).checked_add(area.tile_count().checked_mul(512).ok_or(Refusal::Limit)?).ok_or(Refusal::Limit)?;
    usize::try_from(bytes).map_err(|_| Refusal::Limit)
}
fn float_bytes(area: Rect, n: usize) -> Result<usize> {
    mul(mul(area.width() as usize, area.height() as usize)?, mul(n, 4)?)
}
/// Price the native estimator's conservative transient workspace plus the source
/// and output component visits before any kernel. This is allocation/byte-work
/// admission, not a claim to count every native floating-point instruction.
pub(super) fn admission(
    filters: &[SmartFilter],
    operations: &[Option<FilterParams>],
    owner: Rect,
    document: &Document,
    format: &photocraft_color::PixelFormat,
    ctl: &Interrupt<'_>,
) -> Result<Admission> {
    if filters.len() != operations.len() {
        return Err(Refusal::Unsupported("incomplete native stage plan"));
    }
    let canvas = document.bounds();
    let domain = canvas.union(&owner);
    let mut current = owner;
    let mut peak = 0;
    let mut work = 0;
    let mut retained = storage(current, format)?;
    for (f, p) in filters.iter().zip(operations) {
        check(ctl)?;
        if !f.visible {
            continue;
        }
        if let Some(p) = p {
            let area = photocraft_algo::output_area(p, current, canvas, None).intersect(&domain);
            let workspace = usize::try_from(photocraft_algo::working_bytes(p, area, canvas, format.channels())).map_err(|_| Refusal::Limit)?;
            let out = storage(current.union(&area), format)?;
            peak = peak.max(add(add(mul(retained, 2)?, mul(out, 2)?)?, workspace)?);
            work = add(work, add(add(float_bytes(current, format.channels())?, float_bytes(area, format.channels())?)?, workspace)?)?;
            current = current.union(&area);
            retained = out;
        } else {
            let row = mul(current.width() as usize, mul(format.channels() + 4, 4)?)?;
            peak = peak.max(add(mul(retained, 2)?, row)?);
            work = add(work, mul(float_bytes(current, format.channels())?, 4)?)?;
        }
        if peak > crate::xcf::MAX_PIXEL_BYTES {
            return Err(Refusal::Limit);
        }
    }
    Ok(Admission { temporary: peak, work, retained })
}
pub(super) fn render(
    source: &Surface,
    filters: &[SmartFilter],
    operations: &[Option<FilterParams>],
    owner: Rect,
    document: &Document,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    if filters.len() != operations.len() {
        return Err(Refusal::Unsupported("incomplete native stage plan"));
    }
    let canvas = document.bounds();
    let domain = canvas.union(&owner);
    let mut current = source.clone();
    for (f, p) in filters.iter().zip(operations) {
        check(ctl)?;
        if !f.visible {
            continue;
        }
        current = if let Some(p) = p {
            let area = photocraft_algo::output_area(p, current.content_bounds(), canvas, None).intersect(&domain);
            photocraft_algo::try_apply_in_with(&current, p, area, canvas, None, domain, ctl)
                .map_err(|e| if e.is_cancelled() { Refusal::Cancelled } else { Refusal::Preparation("native spatial kernel") })?
        } else {
            point(&current, &native_adjustment(f)?, document, ctl)?
        };
        validate(&current, ctl)?;
    }
    check(ctl)?;
    Ok(current)
}
fn point(source: &Surface, adjustment: &Adjustment, document: &Document, ctl: &Interrupt<'_>) -> Result<Surface> {
    let area = source.content_bounds();
    let format = source.format();
    let width = area.width() as usize;
    let mut out = source.clone();
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(width).map_err(|_| Refusal::Limit)?;
    rgba.resize(width, [0.; 4]);
    let mut buffer = Buffer { rect: Rect::from_xywh(0, 0, area.width(), 1), px: rgba };
    for y in area.y0..area.y1 {
        check(ctl)?;
        let row_area = Rect::from_xywh(area.x0, y, area.width(), 1);
        let mut samples = source.try_read_region(row_area).map_err(|_| Refusal::Preparation("native point source"))?;
        for (p, v) in buffer.px.iter_mut().zip(samples.chunks_exact(format.channels())) {
            *p = photocraft_raster::to_rgba(&format, v);
        }
        adjust::apply_depth(adjustment, &mut buffer, Transfer::for_document(document.mode, document.depth), Some(document.depth));
        for (v, p) in samples.chunks_exact_mut(format.channels()).zip(&buffer.px) {
            if p.iter().any(|v| !v.is_finite()) {
                return Err(Refusal::Preparation("nonfinite native spatial-stage output"));
            }
            photocraft_raster::from_rgba_into(&format, *p, v);
        }
        out.try_write_region(row_area, &samples).map_err(|_| Refusal::Preparation("native point result"))?;
    }
    check(ctl)?;
    Ok(out)
}

// Native floating kernels may overflow even with finite source samples. Never
// publish that optional result; keep the paid attempt and original owner pixels.
fn validate(surface: &Surface, ctl: &Interrupt<'_>) -> Result<()> {
    let area = surface.content_bounds();
    for y in area.y0..area.y1 {
        check(ctl)?;
        let row = surface.try_read_region(Rect::from_xywh(area.x0, y, area.width(), 1)).map_err(|_| Refusal::Preparation("native spatial output read"))?;
        if row.iter().any(|v| !v.is_finite()) {
            return Err(Refusal::Preparation("nonfinite native spatial output"));
        }
    }
    check(ctl)
}
