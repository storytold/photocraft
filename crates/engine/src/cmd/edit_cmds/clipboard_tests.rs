use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::LayerMask;
use std::sync::Arc;

fn masked(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Masked colour"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("fixture", |doc, _| {
        let l = doc.layer_mut(id).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 8, 30, 24), &[0.8, 0.2, 0.4, 1.0]);
        let mut mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, doc_depth(depth), false), &[1.0]);
        mask.fill_rect(Rect::new(6, 4, 20, 28), &[0.0]);
        mask.fill_rect(Rect::new(20, 4, 24, 28), &[0.5]);
        l.mask = Some(LayerMask { surface: mask, enabled: true, linked: false, density: 0.75, feather: 1.25 });
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn doc_depth(depth: u32) -> SampleType {
    match depth {
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => SampleType::U8,
    }
}

fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn pasted(s: &mut Session, params: Value) -> LayerId {
    LayerId(s.execute("edit.paste", params).unwrap()["layer"].as_u64().unwrap())
}

#[test]
fn copy_then_paste_keeps_an_editable_nonuniform_mask() {
    for depth in [8, 16, 32] {
        let (mut s, src) = masked(depth);
        let original = layer(&s, src).clone();
        s.execute("edit.copy", json!({})).unwrap();
        let id = pasted(&mut s, json!({}));
        assert!(layer(&s, id).mask == original.mask, "{depth}-bit editable mask and metadata");
        assert!(layer(&s, id).surface() == original.surface(), "independent colour data");
        assert!(original.surface().unwrap().tiles().count() > 0, "the colour sharing check is nonempty");
        assert_eq!(original.surface().unwrap().tiles().count(), layer(&s, id).surface().unwrap().tiles().count());
        assert!(original.mask.as_ref().unwrap().surface.tiles().count() > 0, "the mask sharing check is nonempty");
        assert_eq!(original.mask.as_ref().unwrap().surface.tiles().count(), layer(&s, id).mask.as_ref().unwrap().surface.tiles().count());
        for (a, b) in original.surface().unwrap().tiles().zip(layer(&s, id).surface().unwrap().tiles()) {
            assert_eq!(a.0, b.0);
            assert!(Arc::ptr_eq(a.1, b.1), "matching colour tiles remain COW-shared");
        }
        for (a, b) in original.mask.as_ref().unwrap().surface.tiles().zip(layer(&s, id).mask.as_ref().unwrap().surface.tiles()) {
            assert_eq!(a.0, b.0);
            assert!(Arc::ptr_eq(a.1, b.1), "matching mask tiles remain COW-shared");
        }
        assert_ne!(id, src);
        let area = Rect::new(10, 8, 30, 24);
        let a = photocraft_compose::render_layer(&original, area);
        let b = photocraft_compose::render_layer(layer(&s, id), area);
        assert_eq!(a.px, b.px, "the composited appearance matches");
        let steps = s.active().unwrap().history.past_len();
        s.undo();
        assert!(s.active().unwrap().doc.layer(id).is_none());
        s.redo();
        assert!(layer(&s, id).mask == original.mask);
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        s.edit("edit pasted mask and colour", |doc, _| {
            let l = doc.layer_mut(id).unwrap();
            l.mask.as_mut().unwrap().surface.write_pixel(12, 12, &[1.0]);
            l.surface_mut().unwrap().write_pixel(25, 12, &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        assert!(layer(&s, src).mask == original.mask, "editing the copy leaves the source mask alone");
        assert!(layer(&s, src).surface() == original.surface());
        let next = pasted(&mut s, json!({}));
        assert_ne!(next, id);
        assert!(layer(&s, next).mask == original.mask, "the clipboard is an independent snapshot");
        assert!(layer(&s, next).surface() == original.surface());
    }
}

#[test]
fn whole_masked_layer_has_masked_bitmap_and_untouched_hidden_colour() {
    for depth in [8, 16, 32] {
        let (mut s, src) = masked(depth);
        let colour = layer(&s, src).surface().unwrap().rgba(14, 12);
        assert!(colour[3] > 0.99);
        s.execute("edit.copy", json!({})).unwrap();
        let clip = s.clipboard.as_ref().unwrap();
        assert!(clip.layers.is_some(), "{depth}-bit: preserve an editable whole layer");
        let masked = clip.surface.rgba(14, 12);
        let visible = clip.surface.rgba(28, 12);
        assert!(masked[3] < 0.7, "{depth}-bit: the bitmap must apply the layer mask: {masked:?}");
        assert!(visible[3] > 0.9, "{depth}-bit: unmasked colour remains visible: {visible:?}");
        let original = clip.layers.as_ref().unwrap().layers[0].surface().unwrap().rgba(14, 12);
        assert_eq!(original, colour, "{depth}-bit: the editable copy keeps its hidden colour and alpha");
    }
}

#[test]
fn masked_layer_bitmap_includes_effects_outside_the_underlying_pixels() {
    use photocraft_doc::{BlendMode, Color, Effect, FxCommon, FxPaint, StrokeFx, StrokePosition};

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 512, "height": 384, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Small masked layer"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let raw = Rect::new(200, 150, 208, 158);
    s.edit("fixture", |doc, _| {
        let l = doc.layer_mut(id).unwrap();
        l.surface_mut().unwrap().fill_rect(raw, &[0.8, 0.2, 0.4, 1.0]);
        l.mask = Some(LayerMask::reveal_all());
        l.effects.enabled = true;
        l.effects.items.push(Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 6.0,
            position: StrokePosition::Outside,
            paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
            overprint: false,
        }));
        Ok(())
    })
    .unwrap();
    let doc = &s.active().unwrap().doc;
    let canvas = doc.bounds();
    let mut shown = scratch_doc(doc);
    shown.layers.push(doc.layer(id).unwrap().clone());
    shown.layers[0].visible = true;
    let reference = merged_surface(&shown);
    let mut outside_effect_is_visible = false;
    for y in raw.y0 - 12..raw.y1 + 12 {
        for x in raw.x0 - 12..raw.x1 + 12 {
            if !raw.contains(x, y) && reference.rgba(x, y)[3] > 0.01 {
                outside_effect_is_visible = true;
            }
        }
    }
    assert!(outside_effect_is_visible, "the reference fixture must actually render an exterior stroke");
    s.execute("edit.copy", json!({})).unwrap();
    let clip = s.clipboard.as_ref().unwrap();
    assert!(clip.layers.is_some(), "the masked layer remains editable");
    assert_eq!(clip.surface.read_region(canvas), reference.read_region(canvas), "bounded bitmap must equal the full-canvas compositor");
    assert_eq!(clip.bounds, raw.union(&reference.content_bounds()), "the bitmap bounds include the exterior effect");
}

#[test]
fn copying_a_group_keeps_visible_bounds_and_cross_document_placement() {
    let (mut s, child) = masked(8);
    s.edit("remove mask", |doc, _| {
        doc.layer_mut(child).unwrap().mask = None;
        Ok(())
    })
    .unwrap();
    let group = LayerId(s.execute("layer.groupLayers", json!({"name": "Group"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("add distant hidden child", |doc, _| {
        let mut hidden = Layer::raster("Hidden", PixelFormat::RGBA8);
        hidden.surface_mut().unwrap().fill_rect(Rect::new(50, 30, 60, 40), &[0.0, 1.0, 0.0, 1.0]);
        hidden.visible = false;
        doc.layer_mut(group).unwrap().children_mut().unwrap().push(hidden);
        Ok(())
    })
    .unwrap();
    crate::layer_multi_cmds::set_selection(&mut s, vec![group], Some(group), None).unwrap();
    let doc = &s.active().unwrap().doc;
    let canvas = doc.bounds();
    let mut shown = scratch_doc(doc);
    shown.layers.push(doc.layer(group).unwrap().clone());
    shown.layers[0].visible = true;
    let reference = lift(&merged_surface(&shown), None, canvas).unwrap();
    assert_eq!(reference.bounds, Rect::new(10, 8, 30, 24), "the hidden child must not affect the legacy bitmap bounds");
    s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(s.clipboard.as_ref().unwrap().bounds, reference.bounds);
    s.execute("file.new", json!({"width": 100, "height": 80, "background": "transparent"})).unwrap();
    let target = &s.active().unwrap().doc;
    let offset = paste_offset(&reference, target.id, target.bounds(), &json!({}), false);
    assert_eq!(offset, (30, 24));
    let result = s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(result["offset"], json!([offset.0, offset.1]), "the hidden child must not move the paste centre");
}

#[test]
fn failed_masked_bitmap_render_preserves_previous_clipboard_and_document() {
    let (mut s, _) = masked(8);
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 6, "height": 8})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let old_clip = s.clipboard.clone().unwrap();
    let old_doc = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let steps = s.active().unwrap().history.past_len();
    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.copy", json!({}));
    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for rendering the copied layer")), "{result:?}");
    let clip = s.clipboard.as_ref().unwrap();
    assert_eq!(clip.surface, old_clip.surface);
    assert_eq!(clip.bounds, old_clip.bounds);
    assert_eq!(clip.layers, old_clip.layers);
    assert_eq!(clip.resolution, old_clip.resolution);
    assert_eq!(clip.source, old_clip.source);
    let st = s.active().unwrap();
    assert_eq!(st.doc, old_doc);
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.past_len(), steps);
}

#[test]
fn masked_copy_estimate_counts_aligned_tiles_band_formats_and_effect_region() {
    let area = Rect::new(255, 255, 257, 257); // crosses both tile boundaries
    let rgba8 = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let rgba16 = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
    let gray8 = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
    let cmyk32 = PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true);
    let none = MaskedCopyFxMemory::default();
    let overlay = MaskedCopyFxMemory { retained: 4, build: 20 };
    let unsplit = MaskedCopyCmykSplitMemory::default();
    let pixels_in_four_tiles = 4 * 256 * 256;
    let rgba = masked_copy_estimate(area, area, rgba8, none, unsplit).unwrap();
    assert_eq!(rgba, pixels_in_four_tiles * 4 + 4 * 48);
    assert_eq!(masked_copy_estimate(area, area, rgba16, none, unsplit).unwrap(), pixels_in_four_tiles * 8 + 4 * 48);
    assert_eq!(masked_copy_estimate(area, area, gray8, none, unsplit).unwrap(), pixels_in_four_tiles * 2 + 4 * 48);
    assert_eq!(masked_copy_estimate(area, area, cmyk32, none, unsplit).unwrap(), pixels_in_four_tiles * 20 + 4 * 64);
    assert_eq!(masked_copy_estimate(area, area, rgba8, overlay, unsplit).unwrap(), rgba + 4 * 4);
    let raw = Rect::new(0, 0, 20_000, 20_000);
    assert_eq!(masked_copy_estimate(area, raw, rgba8, none, unsplit).unwrap(), rgba, "large raw bounds alone must not count as a render allocation");
    assert!(masked_copy_estimate(area, raw, rgba8, overlay, unsplit).unwrap() >= pixels_in_four_tiles * 4 + 20_000 * 20_000 * 20);
    assert!(check_masked_copy_budget(area, raw, rgba8, none, unsplit, 1).is_err(), "one tile-aligned RGBA8 output already exceeds this tiny budget");
    assert!(check_masked_copy_budget(Rect::new(0, 0, 8, 8), raw, rgba8, none, unsplit, 1).is_ok(), "a cheap render is not rejected just for large raw bounds");
    assert!(check_masked_copy_budget(Rect::new(0, 0, 8, 8), raw, rgba8, overlay, unsplit, 8192).is_err(), "an expensive full-region effect is rejected");
    let huge = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    assert!(masked_copy_estimate(huge, huge, cmyk32, none, unsplit).is_none(), "size overflow must be rejected");
}

#[test]
fn narrow_composite_surface_matches_lift_for_color_modes_and_depths() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(mode, depth, true);
            let mut doc = Document::new("fixture", photocraft_geom::Size::new(520, 32), mode, depth);
            let mut l = Layer::raster("narrow", fmt);
            let mut pixel = vec![0.25; fmt.channels()];
            *pixel.last_mut().unwrap() = 1.0;
            for x in [1, 256, 519] {
                l.surface_mut().unwrap().write_pixel(x, 2, &pixel);
            }
            l.mask = Some(LayerMask::reveal_all());
            doc.layers.push(l);
            let area = Rect::new(1, 2, 520, 3);
            let surface = photocraft_compose::composite_to_surface(&doc, area, fmt, None);
            assert!(surface.tile_count() > 1, "the fixture crosses tile boundaries");
            let lifted = lift(&surface, None, doc.bounds()).unwrap();
            assert_eq!(surface, lifted.surface, "the direct bitmap matches lift for {mode:?} {depth:?}");
            assert_eq!(surface.content_bounds().intersect(&doc.bounds()), lifted.bounds);
        }
    }
}

#[test]
fn cmyk_split_estimate_counts_two_standins_batches_and_effect_map_sets() {
    use photocraft_doc::{BlendMode, Color, Effect, FxCommon};

    let area = Rect::new(0, 0, 6000, 4000);
    let fx_region = area.inflate(2);
    let effect = MaskedCopyFxMemory { retained: 4, build: 20 };
    let dense_tiles = 24 * 16; // 6000x4000 rounded to 256x256 tiles
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Cmyk, depth, true);
        let mut doc = Document::new("fixture", photocraft_geom::Size::new(6000, 4000), ColorMode::Cmyk, depth);
        let mut l = Layer::raster("cmyk", fmt);
        l.surface_mut().unwrap().write_pixel(1, 1, &[0.1, 0.2, 0.3, 0.4, 1.0]);
        l.surface_mut().unwrap().write_pixel(5999, 3999, &[0.1, 0.2, 0.3, 0.4, 1.0]);
        l.surface_mut().unwrap().write_pixel(-5, -5, &[0.1, 0.2, 0.3, 0.4, 1.0]);
        l.mask = Some(LayerMask::reveal_all());
        l.effects.enabled = true;
        l.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), color: Color::rgb(0.0, 0.0, 1.0) });
        assert_eq!(l.surface().unwrap().tile_count(), 3, "the sparse source includes a tile outside the canvas");
        doc.layers.push(l);
        assert_eq!(masked_copy_uses_cmyk_split(&doc), depth != SampleType::F32, "only 8/16-bit use the split compositor");
        let split = masked_copy_cmyk_split_memory(doc.layers[0].surface().unwrap().tile_count(), fmt).unwrap();
        let rgb = PixelFormat::new(ColorMode::Rgb, depth, true);
        assert_eq!(split.retained, 2 * 3 * (256 * 256 * rgb.bytes_per_pixel() as u64 + 256), "both stand-ins include the off-canvas tile");
        assert_eq!(split.building, 3 * 256 * 256 * (fmt.channels() + rgb.channels()) as u64 * 4);
        let unsplit = MaskedCopyCmykSplitMemory::default();
        let no_extra_tiles = MaskedCopyCmykSplitMemory { retained: 0, building: 0, enabled: true };
        let without = masked_copy_estimate(area, fx_region, fmt, effect, unsplit).unwrap();
        let with_second_map = masked_copy_estimate(area, fx_region, fmt, effect, no_extra_tiles).unwrap();
        assert!(with_second_map >= without + u64::from(fx_region.width()) * u64::from(fx_region.height()) * 4);
        let actual_path = if depth == SampleType::F32 { unsplit } else { masked_copy_cmyk_split_memory(dense_tiles, fmt).unwrap() };
        assert!(check_masked_copy_budget(area, fx_region, fmt, effect, actual_path, 8192).is_ok(), "native 24 MP CMYK {depth:?} overlay");
        doc.layers[0].effects.items.push(Effect::PatternOverlay {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            name: "fixture".into(),
            id: "fixture".into(),
            scale: 1.0,
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        });
        assert!(!masked_copy_uses_cmyk_split(&doc), "pattern overlays use the generic compositor instead of split CMYK");
    }
    #[cfg(target_pointer_width = "64")]
    assert!(masked_copy_cmyk_split_memory(usize::MAX, PixelFormat::new(ColorMode::Cmyk, SampleType::U16, true)).is_none());
}

#[test]
fn disabled_effects_do_not_consume_masked_copy_effect_budget() {
    use photocraft_doc::{BlendMode, Color, Effect, FxCommon, FxPaint, StrokeFx, StrokePosition};

    let (mut s, id) = masked(8);
    s.edit("disabled effects", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        layer.effects.enabled = true;
        let mut effect = Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 6.0,
            position: StrokePosition::Outside,
            paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
            overprint: false,
        });
        effect.set_enabled(false);
        layer.effects.items = vec![effect; 100];
        Ok(())
    })
    .unwrap();
    s.edit_prefs(|p| p.performance.memory_usage_mb = 1);
    s.execute("edit.copy", json!({})).expect("disabled effects do not consume the copy estimate");
    assert!(s.clipboard.as_ref().unwrap().layers.is_some());
}

#[test]
fn masked_copy_budget_allows_typical_rgb8_photos_on_default_web_budget() {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let none = MaskedCopyFxMemory::default();
    let unsplit = MaskedCopyCmykSplitMemory::default();
    for (w, h) in [(6000, 4000), (6000, 6000), (9000, 4000)] {
        let area = Rect::new(0, 0, w, h);
        assert!(check_masked_copy_budget(area, area, fmt, none, unsplit, 1536).is_ok(), "{w}x{h}");
    }
    let area = Rect::new(0, 0, 6000, 6000);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        assert!(check_masked_copy_budget(area, area, fmt, none, unsplit, 8192).is_ok(), "native 36 MP {sample:?}");
    }
    let very_wide = Rect::new(0, 0, 300_000, 256);
    assert!(check_masked_copy_budget(very_wide, very_wide, fmt, none, unsplit, 1536).is_err());
}

#[test]
fn common_24mp_effects_use_type_specific_memory_estimates() {
    use photocraft_doc::{BlendMode, Color, Effect, FxCommon, FxPaint, StrokeFx, StrokePosition};

    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let unsplit = MaskedCopyCmykSplitMemory::default();
    let area = Rect::new(0, 0, 6000, 4000);
    let mut layer = Layer::raster("effects", fmt);
    layer.effects.enabled = true;
    let overlay = Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), color: Color::rgb(0.0, 0.0, 1.0) };
    layer.effects.items.push(overlay.clone());
    let one = masked_copy_effect_memory(&layer).unwrap();
    assert_eq!((one.retained, one.build), (4, 20), "an overlay only needs the shared shape map");
    let fx_region = area.inflate(photocraft_compose::effects::margin(&layer));
    assert!(check_masked_copy_budget(area, fx_region, fmt, one, unsplit, 8192).is_ok(), "24 MP overlay on native");
    assert!(check_masked_copy_budget(area, fx_region, fmt, one, unsplit, 1536).is_ok(), "24 MP overlay on web");
    layer.effects.items.push(overlay);
    let two = masked_copy_effect_memory(&layer).unwrap();
    assert_eq!((two.retained, two.build), (one.retained, one.build), "overlays share their shape map");
    assert!(check_masked_copy_budget(area, fx_region, fmt, two, unsplit, 1536).is_ok(), "24 MP with multiple overlays on web");
    layer.effects.items.push(Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: 6.0,
        position: StrokePosition::Outside,
        paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
        overprint: false,
    }));
    let stroke = masked_copy_effect_memory(&layer).unwrap();
    assert_eq!((stroke.retained, stroke.build), (16, 48));
    let fx_region = area.inflate(photocraft_compose::effects::margin(&layer));
    assert!(check_masked_copy_budget(area, fx_region, fmt, stroke, unsplit, 8192).is_ok(), "24 MP stroke and overlays on native");
    assert!(check_masked_copy_budget(area, fx_region, fmt, stroke, unsplit, 1536).is_err(), "complex full-area stroke needs more memory than web budget");
}

#[test]
fn oversized_sparse_masked_copy_keeps_clipboard_document_and_history() {
    let (mut s, _) = masked(8);
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 3000, "height": 2000, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Two distant pixels"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("sparse fixture", |doc, _| {
        let l = doc.layer_mut(id).unwrap();
        let surface = l.surface_mut().unwrap();
        surface.write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
        surface.write_pixel(2999, 1999, &[0.0, 0.0, 1.0, 1.0]);
        l.mask = Some(LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();
    s.edit_prefs(|p| p.performance.memory_usage_mb = 1);
    let old_clip = s.clipboard.clone().unwrap();
    let old_doc = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let steps = s.active().unwrap().history.past_len();
    let result = s.execute("edit.copy", json!({}));
    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("copy memory limit")), "{result:?}");
    let clip = s.clipboard.as_ref().unwrap();
    assert_eq!(clip.surface, old_clip.surface);
    assert_eq!(clip.bounds, old_clip.bounds);
    assert_eq!(clip.layers, old_clip.layers);
    assert_eq!(clip.resolution, old_clip.resolution);
    assert_eq!(clip.source, old_clip.source);
    let st = s.active().unwrap();
    assert_eq!(st.doc, old_doc);
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.past_len(), steps);
}

#[test]
fn source_edits_after_copy_do_not_change_the_clipboard() {
    let (mut s, src) = masked(32);
    let original = layer(&s, src).clone();
    s.execute("edit.copy", json!({})).unwrap();
    s.edit("edit source", |doc, _| {
        let l = doc.layer_mut(src).unwrap();
        l.mask.as_mut().unwrap().surface.write_pixel(12, 12, &[1.0]);
        l.surface_mut().unwrap().write_pixel(25, 12, &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let id = pasted(&mut s, json!({}));
    assert!(layer(&s, id).mask == original.mask);
    assert!(layer(&s, id).surface() == original.surface());
}

#[test]
fn disabled_masks_inverted_samples_and_link_flags_survive_whole_layer_paste() {
    for enabled in [true, false] {
        for linked in [true, false] {
            let (mut s, src) = masked(32);
            s.edit("inverted fixture", |doc, _| {
                let m = doc.layer_mut(src).unwrap().mask.as_mut().unwrap();
                m.enabled = enabled;
                m.linked = linked;
                m.surface = Surface::with_default(m.surface.format(), &[0.0]);
                m.surface.fill_rect(Rect::new(10, 8, 20, 24), &[1.0]);
                Ok(())
            })
            .unwrap();
            let original = layer(&s, src).mask.as_ref().unwrap().clone();
            s.execute("edit.copy", json!({})).unwrap();
            let id = pasted(&mut s, json!({"center": [40, 36]}));
            let m = layer(&s, id).mask.as_ref().unwrap();
            assert_eq!((m.enabled, m.linked, m.density, m.feather), (original.enabled, original.linked, original.density, original.feather));
            assert_eq!(m.surface.sample_channel(32, 32, 0), 1.0);
            assert_eq!(m.surface.sample_channel(45, 32, 0), 0.0);
            assert_eq!(m.surface.default_pixel(), vec![0.0]);
        }
    }
}

#[test]
fn cross_document_nested_groups_translate_linked_and_unlinked_masks_once() {
    use photocraft_doc::vector::{Path, Subpath, VectorMask};

    fn assert_masks_shifted_once(original: &Layer, pasted: &Layer, dx: i32, dy: i32) {
        let mut expected_mask = original.mask.clone().unwrap();
        expected_mask.surface = shifted(&expected_mask.surface, dx, dy);
        assert_eq!(pasted.mask.as_ref(), Some(&expected_mask), "raster mask for {}", original.name);
        let mut expected_vector = original.vector_mask.clone().unwrap();
        expected_vector.path = expected_vector.path.transform(&photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy)));
        assert_eq!(pasted.vector_mask.as_ref(), Some(&expected_vector), "vector mask for {}", original.name);
        match (original.children(), pasted.children()) {
            (Some(before), Some(after)) => {
                assert_eq!(before.len(), after.len());
                for (before, after) in before.iter().zip(after) {
                    assert_masks_shifted_once(before, after, dx, dy);
                }
            }
            (None, None) => {}
            _ => panic!("layer hierarchy changed"),
        }
    }

    let (mut s, child) = masked(8);
    let inner = LayerId(s.execute("layer.groupLayers", json!({"name": "Inner"})).unwrap()["layer"].as_u64().unwrap());
    s.select_layer(inner).unwrap();
    let outer = LayerId(s.execute("layer.groupLayers", json!({"name": "Outer"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("mixed linked masks", |doc, _| {
        for (id, raster_linked, vector_linked) in [(outer, true, false), (inner, false, true), (child, true, false)] {
            let layer = doc.layer_mut(id).unwrap();
            let mut surface = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
            surface.fill_rect(Rect::new(11, 9, 17, 15), &[0.0]);
            layer.mask = Some(LayerMask { surface, enabled: false, linked: raster_linked, density: 0.7, feather: 0.0 });
            let mut vector = VectorMask::new(Path::new(vec![Subpath::polygon(&[(12.0, 10.0), (18.0, 10.0), (18.0, 16.0), (12.0, 16.0)])]));
            vector.enabled = false;
            vector.linked = vector_linked;
            layer.vector_mask = Some(vector);
        }
        Ok(())
    })
    .unwrap();
    s.select_layer(outer).unwrap();
    let original = layer(&s, outer).clone();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 100, "height": 80, "background": "transparent"})).unwrap();
    let result = s.execute("edit.paste", json!({})).unwrap();
    let id = LayerId(result["layer"].as_u64().unwrap());
    let dx = i32::try_from(result["offset"][0].as_i64().unwrap()).unwrap();
    let dy = i32::try_from(result["offset"][1].as_i64().unwrap()).unwrap();
    assert_ne!(dx, 0);
    assert_ne!(dy, 0);
    assert_masks_shifted_once(&original, layer(&s, id), dx, dy);
    assert_eq!(s.documents()[0].doc.layer(outer), Some(&original));
}

#[test]
fn cross_document_paste_moves_both_masks_even_when_unlinked_and_converts_colour() {
    use photocraft_doc::vector::{Path, Subpath, VectorMask};
    for source_depth in [8, 16, 32] {
        for (mode, dest_depth) in [("grayscale", 16), ("cmyk", 32), ("lab", 8), ("rgb", 32)] {
            let (mut s, src) = masked(source_depth);
            s.edit("vector fixture", |doc, _| {
                doc.layer_mut(src).unwrap().vector_mask = Some(VectorMask {
                    path: Path::new(vec![Subpath::polygon(&[(11.0, 9.0), (29.0, 9.0), (29.0, 23.0), (11.0, 23.0)])]),
                    enabled: false,
                    linked: false,
                    density: 0.6,
                    feather: 2.0,
                });
                Ok(())
            })
            .unwrap();
            let original = layer(&s, src).clone();
            s.execute("edit.copy", json!({})).unwrap();
            s.execute("file.new", json!({"width": 100, "height": 80, "depth": dest_depth})).unwrap();
            if mode != "rgb" {
                s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
            }
            let id = pasted(&mut s, json!({"center": [50, 40]}));
            // Source colour extent is (10,8)-(30,24), so centre (20,16) moves by (30,24).
            let l = layer(&s, id);
            assert_eq!(l.surface().unwrap().content_bounds(), Rect::new(40, 32, 60, 48));
            let m = l.mask.as_ref().unwrap();
            let old = original.mask.as_ref().unwrap();
            assert_eq!((m.enabled, m.linked, m.density, m.feather), (old.enabled, old.linked, old.density, old.feather));
            assert!((m.surface.sample_channel(51, 36, 0) - old.surface.sample_channel(21, 12, 0)).abs() < 0.005);
            assert_eq!(m.surface.sample_channel(42, 36, 0), 0.0);
            assert_eq!(m.surface.default_pixel(), vec![1.0]);
            assert_eq!(m.surface.format().sample, doc_depth(dest_depth));
            let vm = l.vector_mask.as_ref().unwrap();
            let old = original.vector_mask.as_ref().unwrap();
            assert_eq!(vm.path, old.path.transform(&photocraft_geom::Affine::translate(30.0, 24.0)));
            assert_eq!((vm.enabled, vm.linked, vm.density, vm.feather), (old.enabled, old.linked, old.density, old.feather));
            assert_eq!(l.surface().unwrap().format().sample, doc_depth(dest_depth));
            let pixel = l.surface().unwrap().pixel(55, 36);
            assert_eq!(pixel.last().copied(), Some(1.0));
            // Validate the converted colour against the CMS transform, independent of paste.
            let source_doc = &s.documents()[0].doc;
            let dest_doc = &s.active().unwrap().doc;
            let t = photocraft_cms::Transform::new(
                &crate::color_cmds::document_profile(source_doc),
                &crate::color_cmds::document_profile(dest_doc),
                s.color.settings.intent(),
                s.color.settings.bpc,
            )
            .unwrap();
            let source_pixel = original.surface().unwrap().pixel(25, 12);
            let mut expected = vec![0.0; dest_doc.mode.color_channels()];
            t.eval(&source_pixel[..3], &mut expected);
            for (actual, expected) in pixel.iter().zip(expected) {
                assert!((actual - expected).abs() < 0.015, "{source_depth} -> {mode}/{dest_depth}: {pixel:?}");
            }
            assert!(s.documents()[0].doc.layer(src).unwrap().mask == original.mask);
            s.undo();
            assert!(s.active().unwrap().doc.layer(id).is_none());
            s.redo();
            assert_eq!(layer(&s, id).mask.as_ref().unwrap().surface.sample_channel(42, 36, 0), 0.0);
        }
    }
}

#[test]
fn same_mode_profile_conversion_changes_colour_but_keeps_mask_coverage() {
    use photocraft_cms::{Builtin, Transform};
    let (mut s, src) = masked(16);
    s.edit("source profile", |doc, _| {
        doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
        Ok(())
    })
    .unwrap();
    let original = layer(&s, src).clone();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 16})).unwrap();
    s.edit("destination profile", |doc, _| {
        doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        Ok(())
    })
    .unwrap();
    let result = s.execute("edit.paste", json!({})).unwrap();
    let id = LayerId(result["layer"].as_u64().unwrap());
    let dx = i32::try_from(result["offset"][0].as_i64().unwrap()).unwrap();
    let dy = i32::try_from(result["offset"][1].as_i64().unwrap()).unwrap();
    assert_eq!((dx, dy), (12, 8), "cross-document paste centres the copied bounds");
    let t = Transform::new(Builtin::AdobeRgbCompat.profile(), Builtin::DisplayP3.profile(), s.color.settings.intent(), s.color.settings.bpc).unwrap();
    let source_pixel = original.surface().unwrap().pixel(25, 12);
    let mut expected = [0.0; 3];
    t.eval(&source_pixel[..3], &mut expected);
    let actual = layer(&s, id).surface().unwrap().pixel(25 + dx, 12 + dy);
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 0.001, "converted pixel {actual:?}, expected {expected:?}");
    }
    assert_eq!(actual[3], source_pixel[3]);
    let mut expected_mask = original.mask.clone().unwrap();
    expected_mask.surface = shifted(&expected_mask.surface, dx, dy);
    assert_eq!(layer(&s, id).mask.as_ref(), Some(&expected_mask));
    assert_ne!(actual, source_pixel);
    assert_eq!(s.documents()[0].doc.layer(src), Some(&original));
}

#[test]
fn selection_copy_copy_merged_and_cut_keep_bitmap_semantics() {
    let (mut s, src) = masked(16);
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 6, "height": 8})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().layers.is_none());
    let id = pasted(&mut s, json!({}));
    assert!(layer(&s, id).mask.is_none());
    assert_eq!(layer(&s, id).surface().unwrap().content_bounds(), Rect::new(10, 8, 16, 16));
    assert_eq!(layer(&s, id).surface().unwrap().pixel(12, 12), layer(&s, src).surface().unwrap().pixel(12, 12));
    s.select_layer(src).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 6, "height": 8})).unwrap();
    s.execute("edit.copyMerged", json!({})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().layers.is_none());
    s.execute("edit.cut", json!({})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().layers.is_none());
    s.undo();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("edit.cut", json!({})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().layers.is_none(), "Cut still takes pixels without a selection");
}

#[test]
fn vector_only_and_fully_hidden_masks_stay_editable() {
    use photocraft_doc::vector::{Path, Subpath, VectorMask};
    for vector_only in [false, true] {
        let (mut s, src) = masked(8);
        s.edit("hidden fixture", |doc, _| {
            let l = doc.layer_mut(src).unwrap();
            if vector_only {
                l.mask = None;
                l.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(100.0, 100.0), (110.0, 100.0), (110.0, 110.0), (100.0, 110.0)])])));
            } else {
                l.mask = Some(LayerMask { surface: Surface::with_default(PixelFormat::GRAY8, &[0.0]), ..LayerMask::reveal_all() });
            }
            Ok(())
        })
        .unwrap();
        let original = layer(&s, src).clone();
        s.execute("edit.copy", json!({})).unwrap();
        let clip = s.clipboard.as_ref().unwrap();
        assert_eq!(clip.bounds, Rect::new(10, 8, 30, 24), "hidden colour still determines layer placement");
        assert_eq!(clip.surface.rgba(25, 12)[3], 0.0, "external bitmap respects the mask");
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        assert_ne!(id, src);
        assert!(layer(&s, id).mask == original.mask);
        assert_eq!(layer(&s, id).vector_mask, original.vector_mask);
        assert!(layer(&s, id).surface() == original.surface(), "hidden colour is preserved");
    }
}

#[test]
fn multiple_selected_layers_keep_order_masks_and_one_history_step() {
    let (mut s, first) = masked(8);
    s.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
    let second = s.active().unwrap().active_layer.unwrap();
    s.edit("second pixels", |doc, _| {
        doc.layer_mut(second).unwrap().surface_mut().unwrap().fill_rect(Rect::new(32, 8, 40, 24), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    crate::layer_multi_cmds::set_selection(&mut s, vec![second, first], Some(second), None).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let copied = &s.clipboard.as_ref().unwrap().layers.as_ref().unwrap().layers;
    assert_eq!(copied.iter().map(|l| l.id).collect::<Vec<_>>(), vec![first, second], "stack order, not click order");
    let steps = s.active().unwrap().history.past_len();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let doc = &s.active().unwrap().doc;
    let ids: Vec<_> = doc.layers.iter().filter(|l| l.id != first && l.id != second && l.name != "Background").map(|l| l.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(doc.layer(ids[0]).unwrap().mask == doc.layer(first).unwrap().mask);
    assert_eq!(doc.layer(ids[1]).unwrap().name, "Second");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    s.undo();
    assert!(ids.iter().all(|id| s.active().unwrap().doc.layer(*id).is_none()));
    s.redo();
    assert!(ids.iter().all(|id| s.active().unwrap().doc.layer(*id).is_some()));
}

#[test]
fn selected_group_and_child_are_copied_only_once() {
    let (mut s, child) = masked(8);
    s.execute("layer.groupLayers", json!({"name": "Masked group"})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    crate::layer_multi_cmds::set_selection(&mut s, vec![group, child], Some(child), None).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(s.clipboard.as_ref().unwrap().layers.as_ref().unwrap().layers.len(), 1);
    let id = pasted(&mut s, json!({}));
    let children = layer(&s, id).children().unwrap();
    assert_ne!(id, group);
    assert_eq!(children.len(), 1);
    assert_ne!(children[0].id, child);
    assert!(children[0].mask == layer(&s, child).mask);
}

#[test]
fn paste_into_and_outside_use_the_bitmap_of_a_whole_masked_layer() {
    let (mut s, _) = masked(8);
    s.execute("edit.copy", json!({})).unwrap();
    let clipboard = s.clipboard.as_ref().unwrap().clone();
    assert!(clipboard.layers.is_some());
    for command in ["edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"] {
        s.execute("select.rect", json!({"x": 24, "y": 8, "width": 6, "height": 16})).unwrap();
        s.execute(command, json!({"center": [20, 16]})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        assert!(layer(&s, id).vector_mask.is_none());
        assert!(layer(&s, id).surface() == Some(&clipboard.surface), "Paste Into/Outside use the flattened bitmap");
        assert!(layer(&s, id).mask.is_some(), "destination selection supplies the new mask");
        assert!(s.clipboard.as_ref().unwrap().layers.is_some(), "the original editable clipboard remains available");
        s.undo();
    }
}

#[test]
fn paste_into_and_outside_flatten_multiple_layers_without_discarding_the_clipboard() {
    for command in ["edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"] {
        let (mut s, first) = masked(8);
        s.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
        let second = s.active().unwrap().active_layer.unwrap();
        s.edit("second pixels", |doc, _| {
            doc.layer_mut(second).unwrap().surface_mut().unwrap().fill_rect(Rect::new(32, 8, 40, 24), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        crate::layer_multi_cmds::set_selection(&mut s, vec![first, second], Some(second), None).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let clipboard = s.clipboard.as_ref().unwrap().clone();
        assert_eq!(clipboard.layers.as_ref().unwrap().layers.len(), 2);
        s.execute("select.rect", json!({"x": 24, "y": 8, "width": 6, "height": 16})).unwrap();
        let before = s.active().unwrap().doc.layers.len();
        let steps = s.active().unwrap().history.past_len();
        s.execute(command, json!({"center": [25, 16]})).unwrap();
        let st = s.active().unwrap();
        let pasted = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(st.doc.layers.len(), before + 1, "{command}: paste exactly one flattened layer");
        assert_eq!(st.history.past_len(), steps + 1, "{command}: one undo step");
        assert_eq!(pasted.surface(), Some(&clipboard.surface), "{command}: use the visible bitmap");
        assert!(pasted.mask.is_some(), "{command}: destination selection supplies the mask");
        assert_eq!(s.clipboard.as_ref().unwrap().layers.as_ref().unwrap().layers.len(), 2, "{command}: retain the editable source");
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.layers.len(), before, "{command}: one undo removes the paste");
    }
}
