use super::rendering::{refresh_or_fail, unapplied_filters};
use super::source::{SourceImage, cache_put, can_cache, source_bytes, source_cache_key};
use super::{
    Affine, Arc, BlendMode, Document, EngineError, Homography, Layer, LayerContent, LayerId, PLACED_KEYS, PixelFormat, Rect, Result, Session, Size,
    SmartContentsId, SmartObject, SmartSource, Surface, Value, encode_source, json, layer_param, other, source_box, translate_surface,
};
use photocraft_algo::transform::Interp;

/// Rounds float noise off a composed transform (scale 0.1 then 10 should be exactly 1), so
/// repeated transforms land back on exact, lossless placements.
pub(crate) fn snap_affine(a: Affine) -> Affine {
    Affine { m: a.m.map(|v| if (v - v.round()).abs() < 1e-9 { v.round() } else { v }) }
}

/// Where the smart object's source pixels land in the document: its projective map (Distort,
/// Perspective) or its affine transform.
pub fn placement(sm: &SmartObject) -> Homography {
    sm.perspective.map(Homography).unwrap_or_else(|| affine_homography(&sm.transform))
}

pub(crate) fn affine_homography(a: &Affine) -> Homography {
    let [a, b, c, d, e, f] = a.m;
    Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])
}

/// Sets where the source lands. An affine map is stored exactly in `transform` (no
/// `perspective`); a projective one in `perspective`, with `transform` its affine approximation at
/// the source origin (for code that only needs scale and position).
pub(crate) fn set_placement(sm: &mut SmartObject, h: Homography) {
    let m = h.0;
    let n = if m[8].abs() > 1e-12 { m.map(|v| v / m[8]) } else { m };
    if n[6].abs() < 1e-12 && n[7].abs() < 1e-12 {
        sm.transform = snap_affine(Affine { m: [n[0], n[3], n[1], n[4], n[2], n[5]] });
        sm.perspective = None;
        return;
    }
    let h = Homography(n);
    let (o, x, y) = (h.apply(0.0, 0.0), h.apply(1.0, 0.0), h.apply(0.0, 1.0));
    sm.transform = Affine { m: [x.0 - o.0, x.1 - o.1, y.0 - o.0, y.1 - o.1, o.0, o.1] };
    sm.perspective = Some(n);
}

/// Composes `a` (document → document) onto the smart object's placement.
pub(crate) fn transform_placement(sm: &mut SmartObject, a: &Affine) {
    match sm.perspective {
        None => sm.transform = snap_affine(a.mul(&sm.transform)),
        Some(_) => set_placement(sm, affine_homography(a).mul(&placement(sm))),
    }
}

/// Moves a smart object by whole pixels without re-rendering.
pub(crate) fn shift_smart(sm: &mut SmartObject, dx: i32, dy: i32) {
    transform_placement(sm, &Affine::translate(dx as f64, dy as f64));
    if let Some(c) = &mut sm.cache {
        *c = crate::layer_multi_cmds::shift_surface(c, dx, dy);
    }
    if let Some(m) = &mut sm.filter_mask {
        m.surface = crate::layer_multi_cmds::shift_surface(&m.surface, dx, dy);
    }
}

/// Layer ▸ Smart Objects ▸ Reset Transform, and the same item of the Layers panel menu (#11662):
/// the placement goes back to 100 % of the current contents' own size with rotation, skew, Distort,
/// Perspective **and Warp** cleared, centred where the object sits. Measured in the reference
/// application 2026-11-06 (see the #11675 discussion): a smart object warped with Arc 50, rotated
/// and scaled came back with no warp at all, at its contents' own size, unrotated, and centred on
/// the average of its four corners. The smart filters stay.
///
/// The step needs the contents re-rendered from their source: a stack whose filters can't all be
/// applied, or a source that can't be read, refuses it whole rather than recording the old (still
/// warped) pixels under a placement that no longer describes them.
pub(super) fn reset_transform(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let LayerContent::Smart(sm) = &st.doc.layer(id).ok_or(EngineError::NoLayer(id))?.content else {
        return Err(other("the layer is not a smart object"));
    };
    // Checked before anything changes, so a filter PhotoCraft doesn't have (kept verbatim from a
    // PSD) leaves the document as it was, as `layer.smartObjects.updateModifiedContent` insists on
    // a re-render too.
    let stuck = unapplied_filters(sm);
    if let Some(name) = stuck.first() {
        return Err(other(format!("the {name} smart filter is not supported, so the smart object's transform can't be reset")));
    }
    // The contents' own box in source pixels: 100 % maps it one-to-one onto the document.
    let [x0, y0, x1, y1] = source_box(&st.doc, sm).ok_or_else(|| other("the smart object's contents are unavailable"))?;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let old = placement(sm);
    // The object stays where it is: the average of the placement's four corners, which is its
    // centre for an affine placement and the quad's centre once Distort or Perspective bent it.
    let (mut sx, mut sy) = (0.0, 0.0);
    for (u, v) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
        let (px, py) = old.apply(u, v);
        sx += px;
        sy += py;
    }
    let (dx, dy) = (sx / 4.0 - cx, sy / 4.0 - cy);
    let to = Affine { m: [1.0, 0.0, 0.0, 1.0, dx, dy] };
    let new = affine_homography(&to);
    // Everything the accumulated transform moved the pixels by, so the linked masks and the Smart
    // Filter mask follow the placement back as they do under Free Transform.
    let rel = new.mul(&old.inverse().ok_or_else(|| other("the smart object has a degenerate transform"))?);
    if ![dx, dy, cx, cy].iter().all(|v| v.is_finite()) || !rel.0.iter().all(|v| v.is_finite()) {
        return Err(other("the smart object's placement is not a usable transform"));
    }
    s.edit("Reset Transform", |doc, _| {
        // The Warp is part of what the reference takes back, so the re-render below starts from the
        // unwarped contents. How far a painted Smart Filter mask should follow a cleared warp is
        // unmeasured: here it follows the placement, as it does under Free Transform.
        if let Some(LayerContent::Smart(sm)) = doc.layer_mut(id).map(|l| &mut l.content) {
            sm.warp = None;
        }
        crate::transform_cmds::transform_by_map(doc, id, &rel, None, Interp::Bicubic)?;
        // Required, not best-effort: `transform_by_map` re-renders too, but falls back to the old
        // pixels when that isn't possible. `Session::edit` runs this on a copy, so an `Err` here
        // records no step and leaves the document exactly as it was. The state is unchanged since
        // that render, so the guarded refresh answers from the stack cache rather than placing the
        // contents a second time.
        refresh_or_fail(doc, id)?;
        // `new ∘ old⁻¹ ∘ old` only lands on the reset map to within float noise, and could leave a
        // Distort placement behind as a nearly affine one: store the exact map.
        if let Some(LayerContent::Smart(sm)) = doc.layer_mut(id).map(|l| &mut l.content) {
            set_placement(sm, new);
        }
        Ok(json!({"layer": id.0, "transform": to.m}))
    })
}

/// Forget PSD placed-layer data that no longer describes the smart object (its source changed):
/// PSD export then writes fresh placed-layer blocks and embeds the new source.
pub fn detach_psd(l: &mut Layer) {
    if let LayerContent::Smart(sm) = &mut l.content {
        sm.psd_raw = None;
    }
    l.psd_blocks.retain(|(k, _)| !PLACED_KEYS.contains(&k));
}

// ---------- conversion ----------

/// Moves a layer (any kind) by whole pixels without re-rendering anything.
pub(super) fn shift_layer(l: &mut Layer, dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    let a = Affine::translate(dx as f64, dy as f64);
    let mv = |s: &Surface| translate_surface(s, dx, dy);
    if let Some(m) = &mut l.mask {
        m.surface = mv(&m.surface);
    }
    if let Some(vm) = &mut l.vector_mask {
        vm.path = vm.path.transform(&a);
    }
    if let Some(fc) = &mut l.fill_cache {
        fc.surface = mv(&fc.surface);
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = mv(s),
        LayerContent::Text(t) => {
            t.transform = a.mul(&t.transform);
            if let Some(c) = &mut t.cache {
                *c = mv(c);
            }
        }
        LayerContent::Shape(sh) => {
            crate::vector_cmds::transform_shape(sh, &a);
            if let Some(c) = &mut sh.cache {
                *c = mv(c);
            }
        }
        LayerContent::Smart(sm) => shift_smart(sm, dx, dy),
        LayerContent::Group(g) => g.children.iter_mut().for_each(|c| shift_layer(c, dx, dy)),
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
    }
}

/// Union of the bounds that visible `layers` can paint onto `canvas` or beyond it, including
/// each visible layer's bounded compositor effect reach. Hidden layers and layers clipped to a
/// hidden base contribute nothing.
fn conversion_region(layers: &[Layer], canvas: Rect) -> Rect {
    let mut out = Rect::EMPTY;
    let mut base_visible = true;
    for layer in layers {
        if !layer.clipped {
            base_visible = layer.visible;
        }
        if !layer.visible || !base_visible {
            continue;
        }
        out = out.union(&layer_conversion_bounds(layer, canvas));
    }
    out
}

/// Conservative rendered bounds for one visible `layer` (and its visible descendants): unclipped
/// pixel content plus the compositor's bounded effect region.
fn layer_conversion_bounds(layer: &Layer, canvas: Rect) -> Rect {
    let mut bounds = match &layer.content {
        LayerContent::Group(group) => {
            let own = if layer.frame.is_some() || group.artboard.is_some() || group.frame_stroke.is_some() {
                photocraft_compose::layer_bounds(layer, canvas)
            } else {
                Rect::EMPTY
            };
            own.union(&conversion_region(&group.children, canvas))
        }
        LayerContent::Fill(_) => canvas.union(&layer.fill_cache.as_ref().map_or(Rect::EMPTY, |fc| fc.surface.content_bounds())),
        LayerContent::Adjustment(_) => Rect::EMPTY,
        _ => {
            let mut own = layer.surface().map_or(Rect::EMPTY, Surface::content_bounds);
            own = own.union(&photocraft_compose::layer_bounds(layer, canvas));
            if !photocraft_compose::transparent_outside(layer) {
                own = own.union(&canvas);
            }
            own
        }
    };
    if photocraft_compose::effects::has_effects(layer) {
        let margin = photocraft_compose::effects::margin(layer);
        bounds = bounds.union(&photocraft_compose::whole_effect_region(layer, canvas, margin));
    }
    bounds
}

fn any_layer(l: &Layer, f: &dyn Fn(&Layer) -> bool) -> bool {
    f(l) || l.children().is_some_and(|c| c.iter().any(|c| any_layer(c, f)))
}

/// Preflight check for Smart Object conversion working memory and renderer limits.
/// Returns the estimated peak working bytes (tiled output surface, one RGBA composite band,
/// and retained compositor effect maps) to reserve before rendering begins.
fn check_conversion_budget(region: Rect, canvas: Rect, fmt: PixelFormat, layers: &[Layer]) -> Result<u64> {
    let (width, height) = (region.width(), region.height());
    let pixels = region.size().area();
    if pixels > photocraft_compose::MAX_RENDER_PIXELS {
        return Err(other(format!(
            "Smart Object conversion at {width} x {height} px exceeds the renderer's {} pixel limit. Reduce the canvas or effect extents",
            photocraft_compose::MAX_RENDER_PIXELS
        )));
    }
    let band_rows = photocraft_compose::band_rows_for(width, 0);
    let band_height = u64::from(u32::try_from(band_rows).unwrap_or(u32::MAX)).min(u64::from(height));
    let band_pixels = u64::from(width).saturating_mul(band_height);
    if band_pixels > photocraft_compose::MAX_RENDER_PIXELS {
        return Err(other(format!(
            "Smart Object conversion at {width} x {height} px exceeds the renderer's {} pixel band limit. Reduce the canvas or effect extents",
            photocraft_compose::MAX_RENDER_PIXELS
        )));
    }
    let band_bytes = band_pixels.saturating_mul(std::mem::size_of::<[f32; 4]>() as u64);
    let surface_bytes = photocraft_raster::tile_storage_bytes(region, &fmt);
    let effect_bytes = photocraft_compose::estimated_effect_map_bytes(layers, canvas, region, band_rows)
        .ok_or_else(|| other("Smart Object conversion effect maps exceed the renderer's working-memory limit. Reduce the canvas or effects"))?;
    let total = surface_bytes.saturating_add(band_bytes).saturating_add(effect_bytes);
    if total > crate::allocation::MAX_DENSE_BYTES {
        return Err(other(format!(
            "Smart Object conversion at {width} x {height} px needs about {} MiB of working memory, exceeding the {} MiB working-memory limit. Convert fewer layers or reduce the canvas or effect extents",
            total.div_ceil(1 << 20),
            crate::allocation::MAX_DENSE_BYTES >> 20
        )));
    }
    Ok(total)
}

/// Scans `buf` for the tight bounding rectangle of pixels with alpha > 0.
fn alpha_bounds(buf: &photocraft_compose::Buffer) -> Rect {
    let w = buf.rect.width() as usize;
    if w == 0 || buf.rect.is_empty() {
        return Rect::EMPTY;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (row_idx, row) in buf.px.chunks_exact(w).enumerate() {
        let Some(first) = row.iter().position(|p| p[3] > 0.0) else { continue };
        let last = row.iter().rposition(|p| p[3] > 0.0).unwrap_or(first);
        let y = buf.rect.y0 + row_idx as i32;
        x0 = x0.min(buf.rect.x0 + first as i32);
        x1 = x1.max(buf.rect.x0 + last as i32 + 1);
        y0 = y0.min(y);
        y1 = y1.max(y + 1);
    }
    if x0 < x1 && y0 < y1 { Rect::new(x0, y0, x1, y1) } else { Rect::EMPTY }
}

/// Encodes `bounds` of `buf` directly into `cache` using `cmyk` when converting CMYK pixels.
/// Fully transparent pixels write the surface's default pixel so CMYK/Lab interior tiles prune.
fn write_band_to_cache(
    cache: &mut Surface,
    buf: &photocraft_compose::Buffer,
    bounds: Rect,
    cmyk: Option<&Arc<photocraft_color::convert::CmykSpace>>,
) -> Result<()> {
    let fmt = cache.format();
    let bpp = fmt.bytes_per_pixel();
    let mut default_encoded = vec![0u8; bpp];
    photocraft_raster::encode_pixel(&fmt, &cache.default_pixel(), &mut default_encoded);
    cache
        .try_write_rows_with(bounds, |y, x0, row| {
            photocraft_color::convert::with_cmyk_space(cmyk, || {
                let mut samples = [0.0; 5];
                for (offset, dst) in row.chunks_exact_mut(bpp).enumerate() {
                    let p = buf.get(x0.saturating_add(offset as i32), y);
                    if fmt.alpha && (p[3] <= 0.0 || p[3].is_nan()) {
                        dst.copy_from_slice(&default_encoded);
                    } else {
                        photocraft_raster::from_rgba_into(&fmt, p, &mut samples);
                        photocraft_raster::encode_pixel(&fmt, &samples, dst);
                    }
                }
            });
        })
        .map_err(|e| other(e.to_string()))
}

/// Builds the smart-object layer replacing `l` in `doc`: the layer (with its mask, effects and
/// fill opacity) becomes a nested document cropped to its rendered bounds; visibility, opacity,
/// blend mode, clipping and label stay on the smart layer. The cache is the exact composite, so
/// the document looks the same before and after.
pub fn layer_to_smart(doc: &Document, l: &Layer) -> Result<Layer> {
    let background = doc.is_background(l.id);
    let mut inner = l.clone();
    inner.visible = true;
    inner.opacity = 1.0;
    inner.clipped = false;
    inner.locks = Default::default();
    if !(inner.is_group() && inner.blend == BlendMode::PassThrough) {
        inner.blend = BlendMode::Normal;
    }
    if background {
        inner.name = "Layer 0".into();
    }
    let mut out = layers_to_smart(doc, &l.name, vec![inner])?;
    if background {
        out.name = "Layer 0".into();
    }
    out.visible = l.visible;
    out.opacity = l.opacity;
    out.blend = if l.blend == BlendMode::PassThrough { BlendMode::Normal } else { l.blend };
    out.clipped = l.clipped;
    out.label = l.label;
    if !background {
        out.locks = l.locks;
    }
    Ok(out)
}

/// Builds a smart-object layer named `name` whose nested document holds `layers` (bottom to top)
/// at its top level, as Photoshop does when several layers are converted: no wrapper group. The
/// nested document is cropped to the rendered bounds and the cache is the exact composite (the
/// same as the layers inside a pass-through group). The smart layer is visible, opaque and Normal.
pub fn layers_to_smart(doc: &Document, name: &str, layers: Vec<Layer>) -> Result<Layer> {
    let canvas = doc.bounds();
    let mut sub = Document::new(format!("{name}.pcraft"), doc.size, doc.mode, doc.depth);
    sub.resolution_dpi = doc.resolution_dpi;
    sub.icc_profile = doc.icc_profile.clone();
    sub.global_light = doc.global_light;
    // Pattern fills and pattern layer effects reference document-level patterns by ID.
    // Preserve those resources in the embedded document before rendering or saving it.
    sub.patterns = doc.patterns.clone();
    if layers.iter().any(|l| any_layer(l, &|x| matches!(x.content, LayerContent::Smart(_)))) {
        // Nested PSD placed layers find their embedded files here.
        sub.metadata.psd_global_blocks = doc.metadata.psd_global_blocks.clone();
    }
    let region = conversion_region(&layers, canvas);
    if region.is_empty() {
        return Err(other("the layer is empty, so there is nothing to convert"));
    }
    let fmt = doc.pixel_format();
    let work = check_conversion_budget(region, canvas, fmt, &layers)?;
    #[cfg(target_arch = "wasm32")]
    crate::memory_gate::ensure("convert to a Smart Object", work)?;
    #[cfg(not(target_arch = "wasm32"))]
    let _working = photocraft_raster::memory::try_reserve_operation(work).map_err(other)?;
    sub.layers = layers.into();

    let cmyk = photocraft_compose::cmyk_space(&sub);
    let mut cache = Surface::new(fmt);
    let mut b = Rect::EMPTY;
    photocraft_compose::try_render_bands(&sub, region, 0, |band| -> Result<()> {
        let band_bounds = alpha_bounds(&band);
        if !band_bounds.is_empty() {
            b = b.union(&band_bounds);
            write_band_to_cache(&mut cache, &band, band_bounds, cmyk.as_ref())?;
        }
        Ok(())
    })
    .map_err(|err| match err {
        photocraft_compose::RenderBandsError::Allocation(e) => other(format!("not enough memory to convert to a Smart Object: {e}")),
        photocraft_compose::RenderBandsError::Sink(e) => e,
    })?;
    if b.is_empty() {
        return Err(other("the layer is empty, so there is nothing to convert"));
    }
    cache.prune();

    sub.layers.iter_mut().for_each(|l| shift_layer(l, -b.x0, -b.y0));
    sub.size = Size::new(b.width(), b.height());
    let bytes = encode_source(&sub)?;
    // The encoded native document is non-SVG; match the classified render/bounds path.
    if can_cache(&cache) {
        cache_put(
            source_cache_key(&sub.name, &bytes, fmt, Some(false)),
            SourceImage { surface: Arc::new(translate_surface(&cache, -b.x0, -b.y0)), bounds: sub.bounds() },
        );
    }

    let source = SmartSource::Embedded { file_name: sub.name.clone(), bytes: Arc::new(bytes) };
    Ok(Layer::new(name, LayerContent::Smart(SmartObject::new(source, Affine::translate(b.x0 as f64, b.y0 as f64), Some(cache)))))
}

pub(super) fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = match crate::commands::explicit_layer(s, p)? {
        Some(id) => vec![id],
        None => crate::layer_multi_cmds::selected(s),
    };
    let new_id = s.edit("Convert to Smart Object", |doc, active| {
        let ids = crate::layer_multi_cmds::top_level(doc, &ids);
        let top = *ids.last().ok_or_else(|| other("no layer selected"))?;
        let l = doc.layer(top).ok_or(EngineError::NoLayer(top))?;
        let so = if ids.len() == 1 {
            layer_to_smart(doc, l)?
        } else {
            let children = ids.iter().map(|id| doc.layer(*id).cloned().ok_or(EngineError::NoLayer(*id))).collect::<Result<Vec<_>>>()?;
            layers_to_smart(doc, &l.name, children)?
        };
        let new_id = so.id;
        // As with Group Layers, place the result in the top selected root's parent. Insert
        // before removal so paths shifting in other parents cannot discard unselected content.
        doc.insert_above(Some(top), so);
        for id in &ids {
            doc.remove(*id).ok_or(EngineError::NoLayer(*id))?;
        }
        if let [only] = ids[..] {
            retarget_layer_refs(doc, only, new_id);
        }
        *active = Some(new_id);
        Ok(new_id)
    })?;
    crate::layer_multi_cmds::reselect(s, vec![new_id], Some(new_id));
    Ok(json!({"layer": new_id.0}))
}

fn retarget_layer_refs(doc: &mut Document, from: LayerId, to: LayerId) {
    let Some((fill_opacity, effects)) = doc.layer(to).map(|l| (l.fill_opacity, l.effects.clone())) else { return };
    for comp in doc.layer_comps.iter_mut().chain(doc.last_document_state.as_mut()) {
        for state in comp.states.iter_mut().filter(|state| state.layer == from) {
            state.layer = to;
            if let Some(appearance) = &mut state.appearance {
                appearance.fill_opacity = fill_opacity;
                appearance.effects = effects.clone();
            }
        }
    }
    for def in doc.variables.defs.iter_mut().filter(|def| def.layer == from) {
        def.layer = to;
    }
    for slice in doc.slices.list.iter_mut().filter(|slice| slice.layer == Some(from)) {
        slice.layer = Some(to);
    }
}

// ---------- contents ----------

pub(super) fn via_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("New Smart Object via Copy", |doc, active| {
        let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let mut copy = src.duplicate();
        copy.name = doc.copy_name(&src.name);
        if let LayerContent::Smart(sm) = &mut copy.content {
            // Independent contents: resolve to embedded bytes and drop the shared PSD uuid.
            sm.contents_id = SmartContentsId::fresh();
            if let Some((file_name, bytes)) = source_bytes(&doc.metadata, &sm.source) {
                sm.source = SmartSource::Embedded { file_name, bytes };
            }
        } else {
            return Err(other("the layer is not a smart object"));
        }
        detach_psd(&mut copy);
        let new_id = doc.insert_above(Some(id), copy);
        *active = Some(new_id);
        Ok(json!({"layer": new_id.0}))
    })
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn direct_band_writer_matches_conversion_and_prunes_transparent_pixels() {
        use photocraft_color::{ColorMode, SampleType};

        let region = Rect::new(-3, -2, 260, 4);
        let bounds = Rect::new(-1, -1, 258, 3);
        let mut buf = photocraft_compose::Buffer::filled(region, [0.0; 4]);
        let opaque_x = region.x0 + 12;
        let opaque_y = region.y0 + 1;
        buf.px[region.width() as usize + 12] = [0.9, 0.3, 0.1, 1.0];
        assert_eq!(alpha_bounds(&buf), Rect::new(opaque_x, opaque_y, opaque_x + 1, opaque_y + 1));

        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
            for sample in SampleType::ALL {
                let format = PixelFormat::new(mode, sample, true);
                let mut cache = Surface::new(format);
                write_band_to_cache(&mut cache, &buf, bounds, None).unwrap();
                cache.prune();
                let surface = super::super::source::buffer_to_surface(&buf, format);
                let expected = photocraft_algo::resample::crop_surface(&surface, bounds);
                // Opaque pixel matches bit-for-bit across every colour mode and depth.
                assert_eq!(cache.pixel(opaque_x, opaque_y), expected.pixel(opaque_x, opaque_y), "{mode:?} {sample:?}");
                // Transparent interior pixels keep the default pixel so CMYK/Lab tiles stay sparse.
                assert_eq!(cache.pixel(bounds.x0, bounds.y0), cache.default_pixel(), "{mode:?} {sample:?}");
                assert_eq!(cache.tile_count(), 1, "only the tile containing the opaque pixel stays allocated: {mode:?} {sample:?}");
            }
        }
    }

    #[test]
    fn direct_band_writer_honors_document_cmyk_profile() {
        let params =
            craft_cms::synth::CmykParams { description: "Test Uncoated".into(), tvi: [0.26, 0.26, 0.26, 0.3], grid_a2b: 5, grid_b2a: 9, ..Default::default() };
        let profile = craft_cms::synth::cmyk_profile(&params).unwrap().to_bytes();
        let space = photocraft_color::convert::CmykSpace::for_profile(Some(&profile)).unwrap();
        let rect = Rect::new(0, 0, 2, 2);
        let buf = photocraft_compose::Buffer::filled(rect, [0.8, 0.35, 0.2, 1.0]);
        let mut with_profile = Surface::new(PixelFormat::CMYKA8);
        let mut with_default = Surface::new(PixelFormat::CMYKA8);
        write_band_to_cache(&mut with_profile, &buf, rect, Some(&space)).unwrap();
        write_band_to_cache(&mut with_default, &buf, rect, None).unwrap();
        assert_ne!(with_profile.pixel(0, 0), with_default.pixel(0, 0), "custom CMYK ICC profile must affect conversion encoding");
        let expected = photocraft_color::convert::with_cmyk_space(Some(&space), || {
            let mut ref_surface = Surface::new(PixelFormat::CMYKA8);
            ref_surface.fill_rect(rect, &photocraft_raster::from_rgba(&PixelFormat::CMYKA8, [0.8, 0.35, 0.2, 1.0]));
            ref_surface.pixel(0, 0)
        });
        assert_eq!(with_profile.pixel(0, 0), expected);
    }

    #[test]
    fn conversion_budget_scales_with_sample_depth_without_allocating_pixels() {
        // 572M pixels fits within MAX_RENDER_PIXELS (1 Gi px), but at 32-bit float (16 B/px)
        // tile storage (~8.57 GiB + band) exceeds the 8 GiB 64-bit dense working limit.
        let (width, height) = if cfg!(target_pointer_width = "64") { (26_000, 22_000) } else { (14_000, 10_000) };
        let region = Rect::new(0, 0, width, height);
        assert!(check_conversion_budget(region, region, PixelFormat::RGBA8, &[]).is_ok(), "8-bit extent fits the budget");
        assert!(check_conversion_budget(region, region, PixelFormat::RGBA16, &[]).is_ok(), "16-bit extent fits the budget");
        assert!(check_conversion_budget(region, region, PixelFormat::RGBA32F, &[]).is_err(), "the same extent exceeds the budget at 32 bits");
        assert!(check_conversion_budget(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), Rect::EMPTY, PixelFormat::RGBA32F, &[]).is_err());
    }

    #[test]
    fn a_thin_region_can_exceed_the_budget_in_whole_tiles() {
        let height = if cfg!(target_pointer_width = "64") { 4_200_000 } else { 1_100_000 };
        let region = Rect::new(-1, 0, 1, height);
        assert!(region.size().area() < photocraft_compose::MAX_RENDER_PIXELS);
        assert!(check_conversion_budget(region, Rect::EMPTY, PixelFormat::RGBA8, &[]).is_err());
    }

    #[test]
    fn a_full_canvas_shadow_fits_comfortably_within_the_32_bit_budget() {
        let canvas = Rect::new(0, 0, 6_000, 6_000);
        let region = canvas.inflate(402);
        let work = check_conversion_budget(region, canvas, PixelFormat::RGBA8, &[]).unwrap();
        // Banded conversion holds one ~139 MiB RGBA band plus the ~182 MiB RGBA8 surface (~321 MiB total).
        assert!(work < 512 << 20, "banded 36 MP RGB8 conversion should stay under 512 MiB without effects, got {work}");
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_large_rgb32_canvas_fits_the_desktop_memory_budget() {
        let canvas = Rect::new(0, 0, 12_000, 12_000);
        // Banded conversion needs ~2.16 GiB tiled surface + ~147 MiB RGBA band (~2.30 GiB), well below 8 GiB.
        assert!(check_conversion_budget(canvas, canvas, PixelFormat::RGBA32F, &[]).unwrap() < 3 << 30);
    }
}
