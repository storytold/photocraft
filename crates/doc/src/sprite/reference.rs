//! Precise reference cels retain their original samples; the layer shows a bounded nearest
//! neighbor projection. Reference layers are locked so painting cannot overwrite this cache.

use super::{SpriteCel, pixels};
use crate::{Rect, Surface};

pub(super) fn projection(cel: &SpriteCel) -> Result<Surface, String> {
    let Some([x, y, w, h]) = cel.precise else { return Ok(cel.pixels.clone()) };
    if w <= 0 || h <= 0 {
        return Err("precise reference dimensions must be positive".into());
    }
    let rect = bounds([x, y, w, h]);
    pixels::checked_rect(rect, cel.pixels.format().bytes_per_pixel())?;
    let channels = cel.pixels.format().channels();
    let mut out = Surface::new(cel.pixels.format());
    for row in rect.y0..rect.y1 {
        let mut values = Vec::with_capacity(rect.width() as usize * channels);
        for col in rect.x0..rect.x1 {
            let (sx, sy) = ((f64::from(col) + 0.5) * 65536.0 - f64::from(x), (f64::from(row) + 0.5) * 65536.0 - f64::from(y));
            if sx < 0.0 || sy < 0.0 || sx >= f64::from(w) || sy >= f64::from(h) {
                values.extend(std::iter::repeat_n(0.0, channels));
                continue;
            }
            let source_x = i64::from(cel.rect.x0) + (sx / f64::from(w) * f64::from(cel.rect.width())).floor() as i64;
            let source_y = i64::from(cel.rect.y0) + (sy / f64::from(h) * f64::from(cel.rect.height())).floor() as i64;
            let source_x = i32::try_from(source_x).map_err(|_| "reference cel coordinate overflow")?;
            let source_y = i32::try_from(source_y).map_err(|_| "reference cel coordinate overflow")?;
            values.extend(cel.pixels.pixel(source_x, source_y));
        }
        out.try_write_region(Rect::from_xywh(rect.x0, row, rect.width(), 1), &values).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub(super) fn bounds([x, y, w, h]: [i32; 4]) -> Rect {
    let left = i64::from(x).div_euclid(65536);
    let top = i64::from(y).div_euclid(65536);
    let right = (i64::from(x) + i64::from(w) + 65535).div_euclid(65536);
    let bottom = (i64::from(y) + i64::from(h) + 65535).div_euclid(65536);
    Rect::new(left as i32, top as i32, right as i32, bottom as i32)
}
