//! The internal layer clipboard is a COW snapshot, never an OS bitmap or a serialized file.
//! A separate composited surface supplies native bitmap interoperability.

use super::*;
use photocraft_color::ColorMode;

fn layered(mode: ColorMode) -> bool {
    matches!(mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab)
}

/// Bound trees before calling recursive document, colour and geometry helpers.
fn check_tree(layers: &[Layer], depth: usize, count: &mut usize) -> Result<()> {
    if depth > 64 {
        return Err(EngineError::Other("Could not copy: layer groups are nested too deeply".into()));
    }
    for l in layers {
        *count = count.saturating_add(1);
        if *count > 10_000 {
            return Err(EngineError::Other("Could not copy: too many layers".into()));
        }
        if let Some(children) = l.children() {
            check_tree(children, depth + 1, count)?;
        }
    }
    Ok(())
}

/// Only transferred data is capped; a large destination or unrelated source layers are valid.
fn check_data(layers: &[Layer], bytes: &mut u64) -> Result<()> {
    for l in layers {
        for surface in l.surface().into_iter().chain(l.mask.as_ref().map(|m| &m.surface)).chain(l.fill_cache.as_ref().map(|c| &c.surface)) {
            for (coord, tile) in surface.tiles() {
                *bytes = bytes.saturating_add(tile.bytes().len() as u64);
                if *bytes > 512 * 1024 * 1024 {
                    return Err(EngineError::Other("Clipboard layer data exceeds the 512 MiB transfer limit".into()));
                }
                if [coord.tx, coord.ty].iter().any(|c| i64::from(*c).abs() > 39_000) {
                    return Err(EngineError::Other("Clipboard layer pixels are outside the supported coordinate range".into()));
                }
            }
            check_area(surface.content_bounds())?;
        }
        if let Some(children) = l.children() {
            check_data(children, bytes)?;
        }
    }
    Ok(())
}

pub(super) fn capture(s: &Session) -> Result<Clip> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let src = &st.doc;
    if !layered(src.mode) {
        return Err(EngineError::Other(format!("Layer clipboard does not support {:?}; copy a pixel selection instead", src.mode)));
    }
    check_tree(&src.layers, 0, &mut 0)?;
    let ids = crate::layer_multi_cmds::top_level(src, &st.selected_layers());
    if ids.is_empty() {
        return Err(EngineError::Other("no layers selected".into()));
    }
    let mut doc = Document::new("Clipboard", src.size, src.mode, src.depth);
    doc.icc_profile = src.icc_profile.clone();
    doc.resolution_dpi = src.resolution_dpi;
    doc.global_light = src.global_light;
    doc.patterns = src.patterns.clone();
    for id in ids {
        // Keep ids in the snapshot. Paste assigns fresh ids every time, including group children.
        doc.layers.push(src.layer(id).ok_or(EngineError::NoLayer(id))?.clone());
    }
    check_data(&doc.layers, &mut 0)?;
    let bounds = doc.layers.iter().filter_map(crate::layer_multi_cmds::layer_bounds).fold(Rect::EMPTY, |a, b| a.union(&b));
    let bounds = if bounds.is_empty() { src.bounds() } else { bounds };
    check_area(bounds)?;
    let fmt = PixelFormat::new(ColorMode::Rgb, src.depth, true);
    let mut surface = Surface::new(fmt);
    // Render only the copied extent in bounded bands; no canvas-sized intermediate buffer.
    photocraft_compose::render_bands(&doc, bounds, 0, |band| -> Result<()> {
        let mut data = Vec::new();
        data.try_reserve_exact(band.px.len().saturating_mul(4)).map_err(|_| EngineError::Other("Not enough memory for the clipboard preview".into()))?;
        for p in band.px {
            data.extend_from_slice(&p);
        }
        surface.write_region(band.rect, &data);
        Ok(())
    })?;
    surface.prune();
    Ok(Clip { surface, bounds, profile: Some(crate::color_cmds::composite_profile(&doc)), layers: Some(doc) })
}

/// A transfer moves the complete layer, including unlinked masks. Link flags govern later
/// editing gestures, not the relative placement of the newly pasted layer and its masks.
fn shift_unlinked(layers: &mut [Layer], dx: i32, dy: i32) {
    let a = photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy));
    for l in layers {
        if let Some(m) = l.mask.as_mut().filter(|m| !m.linked) {
            m.surface = shifted(&m.surface, dx, dy);
        }
        if let Some(m) = l.vector_mask.as_mut().filter(|m| !m.linked) {
            m.path = m.path.transform(&a);
        }
        if let Some(children) = l.children_mut() {
            shift_unlinked(children, dx, dy);
        }
    }
}

pub(super) fn prepare(s: &Session, source: &Document, dest: &Document, dx: i32, dy: i32) -> Result<Document> {
    if source.layers.is_empty() {
        return Err(EngineError::Other("The layer clipboard is empty; copy again before pasting".into()));
    }
    check_tree(&source.layers, 0, &mut 0)?;
    check_tree(&dest.layers, 0, &mut 0)?;
    check_data(&source.layers, &mut 0)?;
    if !layered(dest.mode) {
        return Err(EngineError::Other(format!("Layer clipboard does not support a {:?} destination", dest.mode)));
    }
    let mut copies = source.clone();
    copies.layers = source.layers.iter().map(Layer::duplicate).collect();
    // Clipboard copies are independent of link groups in either open document.
    crate::image_cmds::for_each_layer(&mut copies.layers, &mut |l| l.link_group = None);
    for l in &mut copies.layers {
        if crate::extra_cmds::is_background(l) {
            l.name = dest.next_layer_name("Layer");
            l.locks = Default::default();
        }
    }
    if (copies.mode, &copies.icc_profile) != (dest.mode, &dest.icc_profile) {
        crate::color_cmds::convert_document(&mut copies, &crate::color_cmds::document_profile(dest), s.color.settings.intent(), s.color.settings.bpc)?;
    }
    // Keep matching tiles shared. Surface::convert rewrites every sample even when the
    // format is unchanged, which made a same-document 24 MP paste take about two seconds.
    crate::image_cmds::for_each_surface(&mut copies.layers, true, &mut |surface, _| {
        if surface.format().sample != dest.depth {
            *surface = surface.convert(surface.format().with_sample(dest.depth));
        }
    });
    copies.depth = dest.depth;
    if dx != 0 || dy != 0 {
        shift_unlinked(&mut copies.layers, dx, dy);
        let snapshot = copies.clone();
        for l in &mut copies.layers {
            crate::commands::translate_layer(&snapshot, l, dx, dy);
            crate::vector_cmds::translate_vectors(&snapshot, l, f64::from(dx), f64::from(dy));
        }
    }
    Ok(copies)
}

pub(super) fn paste(s: &mut Session, source: &Document, dx: i32, dy: i32) -> Result<Value> {
    let dest = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let copies = prepare(s, source, dest, dx, dy)?;
    let ids = s.edit("Paste", |doc, active| {
        for pattern in copies.patterns {
            if !doc.patterns.iter().any(|p| p.id == pattern.id) {
                doc.patterns.push(pattern);
            }
        }
        let mut ids = Vec::new();
        for l in copies.layers {
            let id = doc.insert_above(*active, l);
            *active = Some(id);
            ids.push(id);
        }
        doc.selection = None;
        Ok(ids)
    })?;
    let last = ids.last().copied();
    crate::layer_multi_cmds::reselect(s, ids.clone(), last);
    Ok(json!({"layer": last.map(|id| id.0), "layers": ids.iter().map(|id| id.0).collect::<Vec<_>>(), "offset": [dx, dy]}))
}
