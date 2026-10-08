//! Bounded, cancellable viewport pages. Reduction is globally aligned, so independently
//! rendered pages have exactly the same pixel footprints at their shared edges.
use crate::{Buffer, render, render_bands};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

/// A quick, explicitly provisional reduction. Only the requested source rows are read; the
/// exact composite below replaces it after visible pages have first become available.
pub fn preview_page(doc: &Document, source: Rect, factor: u32, interrupt: Interrupt<'_>) -> Result<Buffer, String> {
    if factor <= 1 || !crate::proxy::proxy_faithful(doc) {
        return render_page(doc, source, factor, interrupt);
    }
    let k = i32::try_from(factor).ok().filter(|k| *k > 0 && *k <= 1 << 20).ok_or("invalid viewport reduction")?;
    let source = source.intersect(&doc.bounds());
    if source.is_empty() {
        return Ok(Buffer::transparent(Rect::EMPTY));
    }
    let rect = Rect::new(
        source.x0 / k,
        source.y0 / k,
        source.x1.div_euclid(k) + i32::from(source.x1.rem_euclid(k) != 0),
        source.y1.div_euclid(k) + i32::from(source.y1.rem_euclid(k) != 0),
    );
    if rect.width() > 1024 || rect.height() > 1024 {
        return Err("viewport page is too large".into());
    }
    fn shrink(s: &Surface, r: Rect, k: i32, i: Interrupt<'_>) -> Result<Surface, String> {
        let mut out = Surface::with_default(s.format(), &s.default_pixel());
        let bpp = s.format().bytes_per_pixel();
        for y in r.y0..r.y1 {
            i.check().map_err(|e| e.to_string())?;
            let sy = y.saturating_mul(k).saturating_add(k / 2);
            let row = s.to_interleaved(Rect::new(r.x0.saturating_mul(k), sy, r.x1.saturating_mul(k), sy.saturating_add(1)));
            let mut reduced = Vec::with_capacity(r.width() as usize * bpp);
            for x in 0..r.width() as usize {
                let offset = (x * k as usize + k as usize / 2) * bpp;
                reduced.extend_from_slice(row.get(offset..offset + bpp).ok_or("invalid viewport sample")?);
            }
            out.write_interleaved(Rect::new(r.x0, y, r.x1, y + 1), &reduced);
        }
        Ok(out)
    }
    fn layer(l: &mut photocraft_doc::Layer, r: Rect, k: i32, i: Interrupt<'_>, depth: usize) -> Result<(), String> {
        if depth > 256 {
            return Err("viewport layer nesting limit exceeded".into());
        }
        if let Some(m) = &mut l.mask {
            m.surface = shrink(&m.surface, r, k, i)?;
        }
        if let Some(c) = &mut l.fill_cache {
            c.surface = shrink(&c.surface, r, k, i)?;
        }
        use photocraft_doc::LayerContent;
        match &mut l.content {
            LayerContent::Raster(s) => *s = shrink(s, r, k, i)?,
            LayerContent::Group(g) => {
                if let Some(a) = &mut g.artboard {
                    a.rect = Rect::new(a.rect.x0.div_euclid(k), a.rect.y0.div_euclid(k), a.rect.x1.div_euclid(k), a.rect.y1.div_euclid(k));
                }
                for c in &mut g.children {
                    layer(c, r, k, i, depth + 1)?;
                }
            }
            LayerContent::Text(t) => {
                if let Some(s) = &mut t.cache {
                    *s = shrink(s, r, k, i)?;
                }
            }
            LayerContent::Shape(t) => {
                if let Some(s) = &mut t.cache {
                    *s = shrink(s, r, k, i)?;
                }
            }
            LayerContent::Smart(t) => {
                if let Some(s) = &mut t.cache {
                    *s = shrink(s, r, k, i)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut p = doc.clone();
    p.size = photocraft_doc::Size::new(doc.size.width.div_ceil(factor), doc.size.height.div_ceil(factor));
    for l in &mut p.layers {
        layer(l, rect, k, interrupt, 0)?;
    }
    for c in &mut p.channels {
        c.surface = shrink(&c.surface, rect, k, interrupt)?;
    }
    interrupt.check().map_err(|e| e.to_string())?;
    Ok(render(&p, rect))
}

pub fn render_page(doc: &Document, source: Rect, factor: u32, interrupt: Interrupt<'_>) -> Result<Buffer, String> {
    interrupt.check().map_err(|e| e.to_string())?;
    let source = source.intersect(&doc.bounds());
    if source.is_empty() {
        return Ok(Buffer::transparent(Rect::EMPTY));
    }
    let k = i32::try_from(factor).ok().filter(|k| *k > 0 && *k <= 1 << 20).ok_or("invalid viewport reduction")?;
    let out = Rect::new(
        source.x0.div_euclid(k),
        source.y0.div_euclid(k),
        source.x1.div_euclid(k) + i32::from(source.x1.rem_euclid(k) != 0),
        source.y1.div_euclid(k) + i32::from(source.y1.rem_euclid(k) != 0),
    );
    if out.width() > 1024 || out.height() > 1024 {
        return Err("viewport page is too large".into());
    }
    if k == 1 {
        return Ok(render(doc, source));
    }
    let mut result = Buffer::transparent(out);
    render_bands(doc, source, 128, |band| -> Result<(), String> {
        interrupt.check().map_err(|e| e.to_string())?;
        for (i, p) in band.px.iter().enumerate() {
            let x = band.rect.x0 + (i % band.rect.width() as usize) as i32;
            let y = band.rect.y0 + (i / band.rect.width() as usize) as i32;
            let offset = (y.div_euclid(k) - out.y0) as usize * out.width() as usize + (x.div_euclid(k) - out.x0) as usize;
            let q = result.px.get_mut(offset).ok_or("invalid viewport accumulator")?;
            for (a, b) in q.iter_mut().take(3).zip(p.iter()) {
                *a += *b * p.get(3).copied().unwrap_or(0.0);
            }
            if let Some(a) = q.get_mut(3) {
                *a += p.get(3).copied().unwrap_or(0.0);
            }
        }
        Ok(())
    })?;
    for (i, p) in result.px.iter_mut().enumerate() {
        let x = out.x0 + (i % out.width() as usize) as i32;
        let y = out.y0 + (i / out.width() as usize) as i32;
        let footprint = Rect::from_xywh(x.saturating_mul(k), y.saturating_mul(k), factor, factor).intersect(&source);
        let alpha = p.get(3).copied().unwrap_or(0.0);
        if alpha > 0.0 {
            for c in p.iter_mut().take(3) {
                *c /= alpha;
            }
            if let Some(a) = p.get_mut(3) {
                *a /= (u64::from(footprint.width()) * u64::from(footprint.height())).max(1) as f32;
            }
        }
    }
    interrupt.check().map_err(|e| e.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::Size;
    #[test]
    fn high_zoom_is_original_resolution_at_every_depth() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let r = Rect::new(65001, 22003, 65513, 22515);
            let mut d = Document::new("large", Size::new(86400, 43200), ColorMode::Rgb, depth);
            let mut s = photocraft_raster::Surface::new(d.pixel_format());
            s.fill_rect(r, &[1.0; 4]);
            d.layers.push(photocraft_doc::Layer::new("region", photocraft_doc::LayerContent::Raster(s)));
            let p = render_page(&d, r, 1, Interrupt::NONE).unwrap();
            assert_eq!(p.rect, r);
            assert_eq!(p.px, render(&d, r).px);
        }
    }
    #[test]
    fn independent_pages_match_whole_reduction_and_odd_edges() {
        let d = Document::with_background("d", Size::new(53, 31), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let whole = render_page(&d, d.bounds(), 4, Interrupt::NONE).unwrap();
        for r in [Rect::new(0, 0, 28, 31), Rect::new(28, 0, 53, 31)] {
            let p = render_page(&d, r, 4, Interrupt::NONE).unwrap();
            for y in p.rect.y0..p.rect.y1 {
                for x in p.rect.x0..p.rect.x1 {
                    assert_eq!(
                        p.px[((y - p.rect.y0) * p.rect.width() as i32 + x - p.rect.x0) as usize],
                        whole.px[(y * whole.rect.width() as i32 + x) as usize]
                    );
                }
            }
        }
    }
    #[test]
    fn rejects_invalid_sizes_and_honors_cancellation() {
        let d = Document::new("d", Size::new(86400, 43200), ColorMode::Rgb, SampleType::U8);
        assert!(render_page(&d, d.bounds(), 1, Interrupt::NONE).is_err());
        assert!(render_page(&d, d.bounds(), 0, Interrupt::NONE).is_err());
        assert!(render_page(&d, Rect::new(0, 0, 512, 512), 1, Interrupt::cancel_only(&|| true)).is_err());
    }
}
