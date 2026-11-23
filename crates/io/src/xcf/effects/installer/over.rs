//! One fixed binary topology, materialized as ordinary native layers.
//! Both branches originate in the same admitted Raster owner; no graph evaluator,
//! arbitrary nested input, source text or executable join rule is retained.
use super::{Budgets, Plan, PreparedRaster, Refusal, Result, add, check, mul, prepare_plan};
use crate::xcf::effects::{Mapping, MaskRequirement};
use craft_xcf::effects::Owner;
use photocraft_color::BlendMode;
use photocraft_doc::{Affine, Document, Layer, LayerContent, SmartObject, SmartSource};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};
use std::sync::Arc;

pub(super) fn prepare(
    source_owner: Owner,
    owner: &Layer,
    bounds: Rect,
    document: &Document,
    stages: &[Mapping],
    budgets: &mut Budgets<'_>,
    ctl: &Interrupt<'_>,
) -> Result<PreparedRaster> {
    check(ctl)?;
    let [stage] = stages else { return Err(Refusal::Unsupported("binary graph must be the complete single owner stage")) };
    if !stage.over_original
        || !stage.envelope.visible
        || stage.envelope.opacity != 1.
        || stage.envelope.blend != BlendMode::Normal
        || stage.envelope.mask != MaskRequirement::None
    {
        return Err(Refusal::Unsupported("binary graph requires its verified full-strength envelope"));
    }
    let [filter] = stage.filters.as_slice() else { return Err(Refusal::Unsupported("one verified Invert upper branch")) };
    if filter.command != "image.adjustments.invert" || filter.params != serde_json::json!({}) {
        return Err(Refusal::Unsupported("binary graph requires ordinary native Invert"));
    }
    let LayerContent::Raster(source) = &owner.content else { return Err(Refusal::Unsupported("binary graph requires a raster owner")) };
    let width = usize::try_from(i64::from(bounds.x1) - i64::from(bounds.x0)).map_err(|_| Refusal::Limit)?;
    let height = usize::try_from(i64::from(bounds.y1) - i64::from(bounds.y0)).map_err(|_| Refusal::Limit)?;
    if width == 0 || height == 0 || width > craft_xcf::MAX_SIDE as usize || height > craft_xcf::MAX_SIDE as usize {
        return Err(Refusal::Unsupported("finite nonempty owner extent"));
    }
    let format = source.format();
    let local = Rect::from_xywh(0, 0, width as u32, height as u32);
    let tile_pixels = mul(TILE_SIZE as usize, TILE_SIZE as usize)?;
    let tile_bytes = add(mul(tile_pixels, format.bytes_per_pixel())?, 512)?;
    let local_tiles = usize::try_from(local.tile_count()).map_err(|_| Refusal::Limit)?;
    let local_bytes = mul(local_tiles, tile_bytes)?;
    let cache_bytes = mul(usize::try_from(bounds.tile_count()).map_err(|_| Refusal::Limit)?, tile_bytes)?;
    let profile_bytes = document.icc_profile.as_ref().map_or(0, |profile| profile.len());
    if profile_bytes > 4 << 20 {
        return Err(Refusal::Limit);
    }
    // The inner branch is the existing one-Raster bundle, never an arbitrary
    // nested source. The compound bundle adds one Raster and one cached Smart
    // layer referring to it: generated nesting depth is fixed at two.
    let branch_bound = add(add(mul(local_bytes, 2)?, mul(local_tiles, 32 * 1024)?)?, add(1 << 20, mul(profile_bytes, 2)?)?)?;
    let encoded_bound = mul(branch_bound, 4)?;
    let band_rows = (photocraft_compose::band_rows_for(width as u32, 1) as usize).min(height);
    // The ordinary compositor may hold source/backdrop temporaries in a band.
    // Prepay eight RGBA buffers; no full-image F32 composite is allocated.
    let scratch = mul(mul(mul(width, band_rows)?, 16)?, 8)?;
    let row_bytes = mul(mul(width, format.channels())?, 4)?;
    let extra = add(add(add(mul(local_bytes, 2)?, cache_bytes)?, encoded_bound)?, add(scratch, row_bytes)?)?;
    let retained_before = *budgets.retained;
    let mut upper_allowance = retained_before.min(crate::xcf::MAX_PIXEL_BYTES).checked_sub(extra).ok_or(Refusal::Limit)?;
    let pixels = mul(width, height)?;
    let work = add(mul(pixels, add(mul(format.channels(), 8)?, 128)?)?, mul(encoded_bound, 2)?)?;
    match budgets.work.checked_sub(work) {
        Some(left) => *budgets.work = left,
        None => {
            *budgets.work = 0;
            return Err(Refusal::Limit);
        }
    }
    // Bypass only this dispatch, not the existing scalar admission or renderer.
    // Its result remains private, and only our local storage allowance changes.
    let mut upper = prepare_plan(
        source_owner,
        owner,
        bounds,
        document,
        Plan::Plain(stages),
        &mut Budgets { retained: &mut upper_allowance, work: &mut *budgets.work },
        ctl,
    )?;
    let filtered = upper.smart.cache.take().ok_or(Refusal::Preparation("missing prepared upper branch cache"))?;
    let mut original_local = Surface::new(format);
    let mut filtered_local = Surface::new(format);
    let row_samples = mul(width, format.channels())?;
    let mut row = Vec::new();
    row.try_reserve_exact(row_samples).map_err(|_| Refusal::Limit)?;
    row.resize(row_samples, 0.);
    for y in 0..height {
        check(ctl)?;
        let source_y = bounds.y0.checked_add(y as i32).ok_or(Refusal::Limit)?;
        for (from, to) in [(source, &mut original_local), (&filtered, &mut filtered_local)] {
            for x in 0..width {
                let start = mul(x, format.channels())?;
                from.read_pixel(
                    bounds.x0.checked_add(x as i32).ok_or(Refusal::Limit)?,
                    source_y,
                    row.get_mut(start..add(start, format.channels())?).ok_or(Refusal::Limit)?,
                );
            }
            to.try_write_region(Rect::from_xywh(0, y as i32, width as u32, 1), &row).map_err(|_| Refusal::Limit)?;
        }
    }
    upper.smart.transform = Affine::IDENTITY;
    upper.smart.cache = Some(filtered_local);
    let mut contents = Document::new("Imported native source-over", Size::new(width as u32, height as u32), document.mode, document.depth);
    contents.resolution_dpi = document.resolution_dpi;
    contents.icc_profile = document.icc_profile.clone();
    contents.layers.push(Layer::new("Original lower branch", LayerContent::Raster(original_local)));
    contents.layers.push(Layer::new("Inverted upper branch", LayerContent::Smart(upper.smart)));
    let mut cache = Surface::new(format);
    photocraft_compose::try_render_bands(&contents, local, 1, |band| -> Result<()> {
        check(ctl)?;
        if band.px.len() != (band.rect.width() as usize).checked_mul(band.rect.height() as usize).ok_or(Refusal::Limit)? {
            return Err(Refusal::Preparation("incomplete native source-over band"));
        }
        for (offset, pixels) in band.px.chunks_exact(width).enumerate() {
            check(ctl)?;
            for (x, pixel) in pixels.iter().enumerate() {
                if pixel.iter().any(|value| !value.is_finite()) {
                    return Err(Refusal::Preparation("nonfinite native source-over output"));
                }
                let mut values = [0.; 4];
                let count = photocraft_raster::from_rgba_into(&format, *pixel, &mut values);
                if count != format.channels() {
                    return Err(Refusal::Preparation("native source-over channel conversion"));
                }
                let start = mul(x, format.channels())?;
                row.get_mut(start..add(start, count)?).ok_or(Refusal::Limit)?.copy_from_slice(values.get(..count).ok_or(Refusal::Limit)?);
            }
            let y = bounds.y0.checked_add(band.rect.y0).and_then(|y| y.checked_add(offset as i32)).ok_or(Refusal::Limit)?;
            cache.try_write_region(Rect::from_xywh(bounds.x0, y, width as u32, 1), &row).map_err(|_| Refusal::Limit)?;
        }
        Ok(())
    })
    .map_err(|error| match error {
        photocraft_compose::RenderBandsError::Sink(error) => error,
        photocraft_compose::RenderBandsError::Allocation(_) => Refusal::Limit,
    })?;
    check(ctl)?;
    let bytes =
        photocraft_format::save_to_bytes(&contents, &Default::default()).map_err(|_| Refusal::Preparation("native source-over bundle serialization"))?;
    check(ctl)?;
    if bytes.capacity() > encoded_bound {
        return Err(Refusal::Limit);
    }
    let retained = add(bytes.capacity(), cache_bytes)?;
    let remaining = retained_before.checked_sub(retained).ok_or(Refusal::Limit)?;
    let smart = SmartObject::new(
        SmartSource::Embedded { file_name: "xcf-source-over.pcraft".into(), bytes: Arc::new(bytes) },
        Affine::translate(f64::from(bounds.x0), f64::from(bounds.y0)),
        Some(cache),
    );
    *budgets.retained = remaining;
    Ok(PreparedRaster { source_owner, smart, notes: upper.notes })
}
