use super::rendering::{refresh_or_fail, unapplied_filters};
use super::source::{SourceImage, cache_key, cache_put, source_bytes};
use super::{
    Affine, Arc, BlendMode, Document, EngineError, Homography, Layer, LayerContent, LayerId, PLACED_KEYS, PixelFormat, Rect, Result, Session, Size,
    SmartContentsId, SmartObject, SmartSource, Surface, Value, encode_source, json, layer_param, other, source_box, translate_surface,
};
use photocraft_algo::transform::Interp;
use photocraft_geom::TILE_SIZE;

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
pub(crate) fn detach_psd(l: &mut Layer) {
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

fn subtree_bounds(l: &Layer, canvas: Rect) -> Rect {
    let has_effects = photocraft_compose::effects::has_effects(l);
    let bounds = match &l.content {
        LayerContent::Group(g) => {
            // Even without children, a group with Transparency Shapes Layer off can draw
            // effects from its mask.
            let own = if has_effects && !l.advanced.transparency_shapes { photocraft_compose::effect_source_bounds(l, canvas) } else { Rect::EMPTY };
            g.children.iter().map(|c| subtree_bounds(c, canvas)).fold(own, |a, b| a.union(&b))
        }
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => canvas,
        _ => {
            let bounds = photocraft_compose::effect_source_bounds(l, canvas);
            // A shapeless knockout takes its shape from the canvas but still draws pixels beyond it.
            if photocraft_compose::advanced::shapeless(l) { bounds.union(&l.surface().map_or(Rect::EMPTY, Surface::content_bounds)) } else { bounds }
        }
    };
    // Effects become embedded content too. Grow each subtree by the compositor's bounded
    // reach, including a group's effects after those of its children.
    if !bounds.is_empty() && has_effects { bounds.inflate(photocraft_compose::effects::margin(l)) } else { bounds }
}

fn any_layer(l: &Layer, f: &dyn Fn(&Layer) -> bool) -> bool {
    f(l) || l.children().is_some_and(|c| c.iter().any(|c| any_layer(c, f)))
}

// Bound the extra working memory for off-canvas coverage independently of the existing canvas.
const MAX_CONVERSION_EXPANSION_BYTES: u64 = if cfg!(target_pointer_width = "64") { 1 << 30 } else { 256 << 20 };

fn conversion_work_bytes(region: Rect, fmt: PixelFormat) -> u64 {
    let pixels = region.size().area();
    let pixel_bytes = fmt.bytes_per_pixel() as u64;
    // Writing a region allocates a whole tile for each intersection, even at unaligned edges.
    let surface = region.tile_count().saturating_mul((TILE_SIZE as u64).pow(2)).saturating_mul(pixel_bytes);
    let composite = pixels.saturating_mul(std::mem::size_of::<[f32; 4]>() as u64);
    // Encode directly into the cropped tiles. Once the composite is freed, seeding the source
    // cache can hold a second tiled surface if the source origin is off the tile grid.
    composite.saturating_add(surface).max(surface.saturating_mul(2))
}

fn check_conversion_budget(region: Rect, canvas: Rect, fmt: PixelFormat, layers: &[Layer]) -> Result<u64> {
    let (width, height) = (region.width(), region.height());
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > photocraft_compose::MAX_RENDER_PIXELS {
        return Err(other(format!(
            "Smart Object conversion at {width} x {height} px exceeds the renderer's {} pixel limit. Reduce the canvas or effect extents",
            photocraft_compose::MAX_RENDER_PIXELS
        )));
    }
    // Conversion already rendered the full canvas. Keep that allowance and cap the expansion.
    let limit = conversion_work_bytes(canvas, fmt).saturating_add(MAX_CONVERSION_EXPANSION_BYTES);
    let bytes = conversion_work_bytes(region, fmt);
    if bytes > limit {
        return Err(other(format!(
            "Smart Object conversion at {width} x {height} px needs about {} MiB for its dense buffers, exceeding the {} MiB working-memory limit. Convert fewer layers or reduce the canvas or effect extents",
            bytes.div_ceil(1 << 20),
            limit >> 20
        )));
    }
    let effect_bytes = photocraft_compose::estimated_effect_map_bytes(layers, canvas, region)
        .ok_or_else(|| other("Smart Object conversion effect maps exceed the renderer's working-memory limit. Reduce the canvas or effects"))?;
    let total = bytes.saturating_add(effect_bytes);
    if cfg!(target_pointer_width = "32") && total > crate::image_cmds::MAX_RESAMPLE_BYTES {
        return Err(other(format!(
            "Smart Object conversion at {width} x {height} px needs about {} MiB including effect maps, exceeding the {} MiB working-memory limit. Reduce the canvas or effects",
            total.div_ceil(1 << 20),
            crate::image_cmds::MAX_RESAMPLE_BYTES >> 20
        )));
    }
    Ok(total)
}

/// Write only the final cache rectangle while preserving the rendered pixel format and colour space.
fn conversion_cache(buf: &photocraft_compose::Buffer, bounds: Rect, fmt: PixelFormat) -> Result<Surface> {
    let mut cache = Surface::new(fmt);
    let bpp = fmt.bytes_per_pixel();
    let cmyk = photocraft_color::convert::active_cmyk_space();
    cache
        .try_write_rows_with(bounds, |y, x0, row| {
            photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                let mut samples = [0.0; 5];
                for (offset, dst) in row.chunks_exact_mut(bpp).enumerate() {
                    let x = x0.saturating_add(offset as i32);
                    photocraft_raster::from_rgba_into(&fmt, buf.get(x, y), &mut samples);
                    photocraft_raster::encode_pixel(&fmt, &samples, dst);
                }
            });
        })
        .map_err(|e| other(e.to_string()))?;
    cache.prune();
    Ok(cache)
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
    let region = layers.iter().map(|l| subtree_bounds(l, canvas)).fold(canvas, |a, b| a.union(&b));
    let fmt = doc.pixel_format();
    let work = check_conversion_budget(region, canvas, fmt, &layers)?;
    #[cfg(target_arch = "wasm32")]
    crate::memory_gate::ensure("convert to a Smart Object", work)?;
    #[cfg(not(target_arch = "wasm32"))]
    let _working = photocraft_raster::memory::try_reserve_operation(work).map_err(other)?;
    sub.layers = layers;

    let buf = photocraft_compose::render(&sub, region);
    let w = region.width() as usize;
    let mut b = Rect::EMPTY;
    for (i, p) in buf.px.iter().enumerate() {
        if p[3] > 0.0 {
            let (x, y) = (region.x0 + (i % w) as i32, region.y0 + (i / w) as i32);
            b = b.union(&Rect::new(x, y, x + 1, y + 1));
        }
    }
    if b.is_empty() {
        return Err(other("the layer is empty, so there is nothing to convert"));
    }
    let cache = conversion_cache(&buf, b, fmt)?;
    drop(buf.into_px());

    sub.layers.iter_mut().for_each(|l| shift_layer(l, -b.x0, -b.y0));
    sub.size = Size::new(b.width(), b.height());
    let bytes = encode_source(&sub)?;
    // Seed the decode cache with the composite we already have.
    cache_put(cache_key(&bytes, fmt), SourceImage { surface: Arc::new(translate_surface(&cache, -b.x0, -b.y0)), bounds: sub.bounds() });

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
    fn direct_cache_matches_the_existing_conversion_and_crop() {
        use photocraft_color::{ColorMode, SampleType};

        let region = Rect::new(-3, -2, 260, 4);
        let bounds = Rect::new(-1, -1, 258, 3);
        let mut buf = photocraft_compose::Buffer::filled(region, [0.75, 0.25, 0.5, 0.0]);
        buf.px[region.width() as usize + 12] = [0.9, 0.3, 0.1, 1.0];
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
            for sample in SampleType::ALL {
                let format = PixelFormat::new(mode, sample, true);
                let surface = super::super::source::buffer_to_surface(&buf, format);
                let mut expected = photocraft_algo::resample::crop_surface(&surface, bounds);
                expected.prune();
                assert_eq!(conversion_cache(&buf, bounds, format).unwrap(), expected, "{mode:?} {sample:?}");
            }
        }
    }

    #[test]
    fn conversion_budget_scales_with_sample_depth_without_allocating_pixels() {
        let width = if cfg!(target_pointer_width = "64") { 34_000 } else { 8_500 };
        let region = Rect::new(0, 0, width, 1_000);
        assert!(check_conversion_budget(region, Rect::EMPTY, PixelFormat::RGBA8, &[]).is_ok(), "8-bit effect extents fit the budget");
        assert!(check_conversion_budget(region, Rect::EMPTY, PixelFormat::RGBA16, &[]).is_ok(), "16-bit effect extents fit the budget");
        assert!(check_conversion_budget(region, Rect::EMPTY, PixelFormat::RGBA32F, &[]).is_err(), "the same extent needs more storage at 32 bits");
        assert!(check_conversion_budget(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), Rect::EMPTY, PixelFormat::RGBA32F, &[]).is_err());
    }

    #[test]
    fn a_thin_region_can_exceed_the_budget_in_whole_tiles() {
        let region = Rect::new(-1, 0, 1, 1_000_000);
        assert!(check_conversion_budget(region, Rect::EMPTY, PixelFormat::RGBA8, &[]).is_err());
    }

    #[test]
    fn a_full_canvas_shadow_fits_the_32_bit_expansion_allowance() {
        let canvas = Rect::new(0, 0, 6_000, 6_000);
        let region = canvas.inflate(402);
        let bytes = conversion_work_bytes(region, PixelFormat::RGBA8);
        assert!(bytes - conversion_work_bytes(canvas, PixelFormat::RGBA8) < 256 << 20);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_large_rgb32_canvas_can_use_the_desktop_memory_budget() {
        let canvas = Rect::new(0, 0, 12_000, 12_000);
        assert!(check_conversion_budget(canvas, canvas, PixelFormat::RGBA32F, &[]).unwrap() < 8 << 30);
    }
}
