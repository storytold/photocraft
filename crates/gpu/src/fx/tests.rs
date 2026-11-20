use super::*;
use photocraft_color::BlendMode;
use photocraft_doc::FxCommon;

/// Alpha-only coverage preserves the CPU oracle at every stored format, including sparse defaults.
#[test]
fn raster_shape_band_matches_reference_at_every_format() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    let modes = [
        ColorMode::Bitmap,
        ColorMode::Grayscale,
        ColorMode::Indexed,
        ColorMode::Rgb,
        ColorMode::Cmyk,
        ColorMode::Lab,
        ColorMode::Multichannel,
        ColorMode::Duotone,
    ];
    for mode in modes {
        for depth in SampleType::ALL {
            for alpha in [false, true] {
                let doc = Document::new("alpha", Size::new(30, 300), mode, depth);
                let format = PixelFormat::new(mode, depth, alpha);
                let mut defaults = vec![0.2; format.channels()];
                if alpha {
                    defaults[mode.color_channels()] = 0.35;
                }
                let mut surface = Surface::with_default(format, &defaults);
                for (y, a) in [(2, 0.0), (128, 0.7), (259, 1.0), (270, -0.2)] {
                    let mut pixel = defaults.clone();
                    if alpha {
                        pixel[mode.color_channels()] = a;
                    }
                    surface.fill_rect(Rect::new(-2, y, 20, y + 3), &pixel);
                }
                let mut layer = Layer::new("raster", LayerContent::Raster(surface));
                let region = Rect::new(-5, -7, 35, 305);
                assert_eq!(shape_band(&doc, &layer, region), photocraft_compose::layer_shape(&doc, &layer, region), "{format:?}");
                for kind in ["text", "smart"] {
                    let mut cached = cached_layer(kind, layer.surface().cloned());
                    assert_eq!(shape_band(&doc, &cached, region), photocraft_compose::layer_shape(&doc, &cached, region), "{kind} {format:?}");
                    cached.mask = Some(photocraft_doc::LayerMask::reveal_all());
                    assert_eq!(shape_band(&doc, &cached, region), photocraft_compose::layer_shape(&doc, &cached, region));
                }
                layer.mask = Some(photocraft_doc::LayerMask::reveal_all());
                assert_eq!(shape_band(&doc, &layer, region), photocraft_compose::layer_shape(&doc, &layer, region));
                layer.mask = None;
                layer.advanced.transparency_shapes = false;
                assert_eq!(shape_band(&doc, &layer, region), photocraft_compose::layer_shape(&doc, &layer, region));
            }
        }
    }
}

/// Vector-masked display surfaces must use reference coverage, even without a GPU adapter.
#[test]
fn vector_masked_shape_band_matches_reference() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Path, Subpath, VectorMask};
    use photocraft_geom::Size;
    for depth in SampleType::ALL {
        let doc = Document::new("vector", Size::new(30, 30), ColorMode::Rgb, depth);
        let region = doc.bounds();
        let mut raster = Layer::raster("vector", doc.pixel_format());
        raster.surface_mut().unwrap().fill_rect(region, &[0.3, 0.4, 0.5, 1.0]);
        for kind in ["raster", "text", "smart"] {
            let mut layer = if kind == "raster" { raster.clone() } else { cached_layer(kind, raster.surface().cloned()) };
            let unmasked = shape_band(&doc, &layer, region);
            let path = Path::new(vec![Subpath::polygon(&[(3.5, 2.0), (25.0, 6.3), (12.0, 28.7)])]);
            layer.vector_mask = Some(VectorMask::new(path));
            let expected = photocraft_compose::layer_shape(&doc, &layer, region);
            assert_ne!(expected, unmasked, "the mask must change coverage: {kind} {depth:?}");
            assert_eq!(shape_band(&doc, &layer, region), expected, "{kind} {depth:?}");
        }
    }
}

/// Fill opacity, Blend If and clipping change compositing, not the content shape used by effects.
/// A translated surface also keeps its stored alpha outside the document canvas.
#[test]
fn direct_alpha_shape_matches_reference_with_layer_blending_and_overhang() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{BlendRange, Document};
    use photocraft_geom::Size;

    for depth in SampleType::ALL {
        let doc = Document::new("overhang", Size::new(30, 30), ColorMode::Rgb, depth);
        let mut raster = Layer::raster("offset", doc.pixel_format());
        raster.surface_mut().unwrap().fill_rect(Rect::new(2, 3, 24, 25), &[0.2, 0.4, 0.6, 0.7]);
        let surface = raster.surface().unwrap();
        let moved = surface.translated(-8, 7, surface.content_bounds());
        *raster.surface_mut().unwrap() = moved;
        let region = Rect::new(-10, -4, 38, 40);

        for kind in ["raster", "text", "smart"] {
            let base = if kind == "raster" { raster.clone() } else { cached_layer(kind, raster.surface().cloned()) };
            let expected = photocraft_compose::layer_shape(&doc, &base, region);
            assert!(expected.iter().any(|&a| a > 0.0), "{kind} {depth:?}: no content");
            let width = region.width() as usize;
            let outside_canvas = (12 - region.y0) as usize * width + (-4 - region.x0) as usize;
            assert!(expected[outside_canvas] > 0.0, "{kind} {depth:?}: no overhang");
            assert_eq!(shape_band(&doc, &base, region), expected, "{kind} {depth:?}: translated");

            let mut changed = base.clone();
            let check = |layer: &Layer, setting: &str| {
                assert_eq!(photocraft_compose::layer_shape(&doc, layer, region), expected, "{kind} {depth:?}: {setting} changed the oracle");
                assert_eq!(shape_band(&doc, layer, region), expected, "{kind} {depth:?}: {setting}");
            };
            changed.fill_opacity = 0.25;
            check(&changed, "fill opacity");
            changed.blend_if.set(0, [BlendRange { black: [200, 200], white: [255, 255] }, BlendRange::FULL]);
            check(&changed, "Blend If");
            changed.clipped = true;
            check(&changed, "clipping");
        }
    }
}

#[test]
fn banded_shape_comparison_matches_full_shape_and_detects_edits() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let doc = Document::new("compare", Size::new(300, 400), ColorMode::Rgb, depth);
        let mut layer = Layer::raster("layer", doc.pixel_format());
        layer.surface_mut().unwrap().fill_rect(Rect::new(10, 30, 250, 370), &[0.3, 0.4, 0.5, 0.7]);
        let region = Rect::new(-10, -20, 290, 390);
        assert!(shape_matches(&doc, &layer, Rect::EMPTY, &[]));
        let cached = shape(&doc, &layer, region);
        assert!(shape_matches(&doc, &layer, region, &cached));
        assert!(!shape_matches(&doc, &layer, region, &cached[..cached.len() - 1]));
        layer.surface_mut().unwrap().fill_rect(Rect::new(100, 250, 101, 251), &[0.0, 0.0, 0.0, 0.0]);
        assert!(!shape_matches(&doc, &layer, region, &cached));
    }
}

/// Floating alpha is copied exactly, including non-finite values and signed zero.
#[test]
fn raster_shape_band_keeps_float_alpha_bits() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    let doc = Document::new("float alpha", Size::new(4, 1), ColorMode::Rgb, SampleType::F32);
    let mut layer = Layer::raster("alpha", PixelFormat::RGBA32F);
    for (x, alpha) in [f32::from_bits(0x7fc0_0001), f32::NEG_INFINITY, f32::INFINITY, -0.0].into_iter().enumerate() {
        layer.surface_mut().unwrap().fill_rect(Rect::new(x as i32, 0, x as i32 + 1, 1), &[0.2, 0.3, 0.4, alpha]);
    }
    let r = doc.bounds();
    let actual = shape_band(&doc, &layer, r);
    let expected = photocraft_compose::layer_shape(&doc, &layer, r);
    assert_eq!(actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
    let cached = shape(&doc, &layer, r);
    assert!(shape_matches(&doc, &layer, r, &cached), "unchanged NaN alpha must reuse the cached shape");
    layer.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[0.2, 0.3, 0.4, f32::from_bits(0x7fc0_0002)]);
    assert!(!shape_matches(&doc, &layer, r, &cached), "a changed NaN payload must invalidate the cache");
}

/// Filled-shape identity survives integer moves, including fractional anchors and handles.
#[test]
fn filled_shape_key_preserves_translation_but_detects_deformation() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath};
    let mut layer = Layer::new(
        "shape",
        LayerContent::Shape(ShapeLayer {
            path: Path::new(vec![Subpath::polygon(&[(20.0, 20.0), (70.0, 20.0), (70.0, 60.0), (20.0, 60.0)])]),
            fill: Some(Fill::Solid(photocraft_color::Color::WHITE)),
            ..Default::default()
        }),
    );
    let canvas = Rect::new(0, 0, 200, 200);
    let before = shape_key(&layer, canvas);
    let LayerContent::Shape(shape) = &mut layer.content else { unreachable!() };
    shape.path = shape.path.transform(&photocraft_geom::Affine::translate(13.0, 7.0));
    assert_eq!(before, shape_key(&layer, canvas));
    let LayerContent::Shape(shape) = &mut layer.content else { unreachable!() };
    shape.path.subpaths[0].knots[1].anchor.x += 3.0;
    assert_ne!(before, shape_key(&layer, canvas));

    let LayerContent::Shape(shape) = &mut layer.content else { unreachable!() };
    shape.path = Path::new(vec![Subpath::polygon(&[(0.1, 0.2), (0.3, 0.2), (0.3, 1.7), (0.1, 1.7)])]);
    shape.path.subpaths[0].knots[1].out_ctrl = photocraft_geom::Point::new(0.45, 0.65);
    let original = shape.path.clone();
    let before = shape_key(&layer, canvas);
    for (dx, dy) in [(1.0, 1.0), (-13.0, 7.0), (100_000.0, -100_000.0)] {
        let LayerContent::Shape(shape) = &mut layer.content else { unreachable!() };
        shape.path = original.transform(&photocraft_geom::Affine::translate(dx, dy));
        assert_eq!(before, shape_key(&layer, canvas), "fractional path translated by {dx}, {dy}");
    }
    let LayerContent::Shape(shape) = &mut layer.content else { unreachable!() };
    shape.path.subpaths[0].knots[1].out_ctrl.x += 0.01;
    assert_ne!(before, shape_key(&layer, canvas), "a handle deformation invalidates the key");
}

#[test]
fn programs_key_on_geometry_not_colour() {
    let light = GlobalLight::default();
    let Effect::DropShadow(mut s) = Effect::default_drop_shadow() else { unreachable!() };
    let a = program(&Effect::DropShadow(s.clone()), &light, false);
    s.color = photocraft_color::Color::rgb(1.0, 0.0, 0.0);
    s.common = FxCommon::new(BlendMode::Screen, 0.3);
    let b = program(&Effect::DropShadow(s.clone()), &light, false);
    assert_eq!(a.key, b.key);
    s.size += 1.0;
    assert_ne!(a.key, program(&Effect::DropShadow(s), &light, false).key);
}

/// A styled group's effect shape follows its own vector mask and its children's Blend If and
/// Advanced Blending, so the cached shape must be rebuilt when any of them changes.
#[test]
fn group_key_follows_vector_mask_blend_if_and_advanced() {
    use photocraft_doc::{BlendRange, Knockout, Path, PixelFormat, Subpath, VectorMask};
    let light = GlobalLight::default();
    let group = Layer::group("g", vec![Layer::raster("child", PixelFormat::RGBA8)]);
    let base = group_key(&group, &light, &mut |_| {});

    let mask = |x: f64| VectorMask::new(Path::new(vec![Subpath::polygon(&[(x, 0.0), (x + 10.0, 0.0), (x + 10.0, 10.0)])]));
    let mut masked = group.clone();
    masked.vector_mask = Some(mask(0.0));
    let with_mask = group_key(&masked, &light, &mut |_| {});
    assert_ne!(base, with_mask, "adding the group's vector mask");
    masked.vector_mask = Some(mask(5.0));
    assert_ne!(with_mask, group_key(&masked, &light, &mut |_| {}), "moving the group's vector mask");

    fn child_mut(g: &mut Layer) -> &mut Layer {
        let LayerContent::Group(gr) = &mut g.content else { unreachable!() };
        &mut gr.children[0]
    }
    let mut blend_if = group.clone();
    child_mut(&mut blend_if).blend_if.ranges = vec![[BlendRange { black: [128, 128], white: [255, 255] }, BlendRange::FULL]];
    assert_ne!(base, group_key(&blend_if, &light, &mut |_| {}), "a child's Blend If");

    let mut advanced = group.clone();
    child_mut(&mut advanced).advanced.knockout = Knockout::Deep;
    assert_ne!(base, group_key(&advanced, &light, &mut |_| {}), "a child's Advanced Blending");
}

#[test]
fn windows_grow_by_reach_and_temps_are_reused() {
    let light = GlobalLight::default();
    let Effect::DropShadow(mut s) = Effect::default_drop_shadow() else { unreachable!() };
    s.spread = 0.5;
    let p = program(&Effect::DropShadow(s), &light, false);
    let region = Rect::new(-100, -100, 1000, 1000);
    let d = Rect::new(0, 0, 10, 10);
    let fr = |_| 5;
    let w = p.windows(d, region, &fr);
    let last = *w.last().unwrap();
    assert!(last.contains_rect(&d.inflate(7)), "{last:?}");
    // Earlier stages need at least as much as later ones read.
    assert!(w[0].width() >= last.width());
    let (t, n) = p.temps();
    assert!(n <= 3, "{n} temporaries for {} stages", p.stages.len());
    assert!(t.last().unwrap().is_none());
}

/// A distance field computed in bands from grown windows equals the whole-region field
/// wherever it is within its reach (the property incremental updates rely on).
#[test]
fn banded_fields_match_whole_region_within_reach() {
    let region = Rect::new(-20, -10, 300, 290);
    let (w, h) = (region.width() as usize, region.height() as usize);
    let mut shape = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 - 150.0, y as f32 - 140.0);
            let r = (fx * fx + fy * fy).sqrt() + 8.0 * (fy * 0.07).sin() * (fx * 0.05).cos();
            shape[y * w + x] = (90.0 - r).clamp(0.0, 1.0) * if (x / 13 + y / 17) % 5 == 0 { 0.6 } else { 1.0 };
        }
    }
    for kind in [
        FieldKind::Outside,
        FieldKind::Inside,
        FieldKind::BevelOutside,
        FieldKind::BevelInside,
        FieldKind::ChokeInside,
        FieldKind::StrokeOutside,
        FieldKind::StrokeInside,
        FieldKind::StrokeOutsideExact,
        FieldKind::StrokeOutsideVector,
        FieldKind::StrokeOutsideVectorChamfer,
    ] {
        let reach = 9;
        let whole = photocraft_compose::effects::distance_field(kind, shape.clone(), w, h, reach as f32);
        let banded = field(kind, reach, &shape, region, region);
        let part = Rect::new(40, 30, 170, 120);
        let partial = field(kind, reach, &shape, region, part);
        for (i, (a, b)) in whole.iter().zip(&banded).enumerate() {
            if a.min(*b) < reach as f32 {
                assert!((a - b).abs() < 1e-5, "{kind:?} at {i}: {a} vs {b}");
            }
        }
        for (i, b) in partial.iter().enumerate() {
            let (x, y) = (part.x0 + (i % part.width() as usize) as i32, part.y0 + (i / part.width() as usize) as i32);
            let a = whole[(y - region.y0) as usize * w + (x - region.x0) as usize];
            if a.min(*b) < reach as f32 {
                assert!((a - b).abs() < 1e-5, "{kind:?} at ({x},{y}): {a} vs {b}");
            }
        }
    }
}
#[test]
fn windowed_bevel_fields_match_whole_region_on_anti_aliased_edges() {
    // The bevel fields read the edge pixels around the foot of the perpendicular and take the
    // direction of each from its neighbours, so the halo of a part of the region has to cover all of
    // that. A wide bar with clean anti-aliased edges (almost along the 128 row bands the other
    // fields use, across them and at an angle) puts sharp edge pixels where bands would meet and
    // inside the partial window. The whole region is one window, so it is exactly the CPU's field.
    let region = Rect::new(-20, -10, 300, 290);
    let (w, h) = (region.width() as usize, region.height() as usize);
    let part = Rect::new(40, 30, 170, 120);
    let mut too_small_noticed = false;
    for deg in [1.5f32, 5.0, 30.0, 88.5] {
        let (sn, cs) = deg.to_radians().sin_cos();
        let inside = |x: f32, y: f32| {
            let (u, v) = ((x - 150.0) * cs + (y - 128.3) * sn, -(x - 150.0) * sn + (y - 128.3) * cs);
            u.abs() <= 140.0 && v.abs() <= 60.0
        };
        let mut shape = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let hits = (0..64).filter(|k| inside(x as f32 + ((k % 8) as f32 + 0.5) / 8.0, y as f32 + ((k / 8) as f32 + 0.5) / 8.0)).count();
                shape[y * w + x] = hits as f32 / 64.0;
            }
        }
        for reach in [9, 24] {
            for kind in [FieldKind::BevelInside, FieldKind::BevelOutside] {
                let whole = photocraft_compose::effects::distance_field(kind, shape.clone(), w, h, reach as f32);
                let differs = |part_field: &[f32], out: Rect| {
                    let ow = out.width() as usize;
                    part_field
                        .iter()
                        .enumerate()
                        .filter(|&(i, b)| {
                            let (x, y) = (out.x0 + (i % ow) as i32, out.y0 + (i / ow) as i32);
                            let a = whole[(y - region.y0) as usize * w + (x - region.x0) as usize];
                            a.min(*b) < reach as f32 && (a - b).abs() > 1e-5
                        })
                        .count()
                };
                let all = field(kind, reach, &shape, region, region);
                assert!(
                    all.iter().zip(&whole).all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{kind:?}, {deg} deg, reach {reach}: the whole region is not the CPU's field"
                );
                assert_eq!(
                    differs(&field(kind, reach, &shape, region, part), part),
                    0,
                    "{kind:?}, {deg} deg, reach {reach}: partial window differs from whole"
                );
                // The test can see a halo that is too small: two rows past the part is not enough.
                too_small_noticed |= differs(&field_banded(kind, reach, &shape, region, part, 2), part) > 0;
            }
        }
    }
    assert!(too_small_noticed, "a 2 row halo went unnoticed, so this test guards nothing");
}

/// Both bevel fields from one window (sharp edges found once) are bit for bit the fields
/// computed one at a time, over the whole region and over a part of it.
#[test]
fn paired_bevel_fields_equal_the_fields_one_at_a_time() {
    let region = Rect::new(-20, -10, 300, 290);
    let (w, h) = (region.width() as usize, region.height() as usize);
    let mut shape = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 - 150.0, y as f32 - 140.0);
            let r = (fx * fx + fy * fy).sqrt() + 8.0 * (fy * 0.07).sin() * (fx * 0.05).cos();
            shape[y * w + x] = (90.0 - r).clamp(0.0, 1.0) * if (x / 13 + y / 17) % 5 == 0 { 0.6 } else { 1.0 };
        }
    }
    let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    for reach in [3, 9, 40] {
        for out in [region, Rect::new(40, 30, 170, 120), Rect::new(-20, 200, 300, 290)] {
            let [inside, outside] = bevel_fields(reach, &shape, region, out);
            assert_eq!(bits(&inside), bits(&field(FieldKind::BevelInside, reach, &shape, region, out)), "inside, reach {reach}, {out:?}");
            assert_eq!(bits(&outside), bits(&field(FieldKind::BevelOutside, reach, &shape, region, out)), "outside, reach {reach}, {out:?}");
            // The banded path (wasm) pairs them per band the same way.
            let halo = field_radius(FieldKind::BevelInside, reach) + 1;
            let banded =
                fields_in_windows(false, &shape, region, out, halo, |s, w, h| photocraft_compose::effects::bevel_distance_fields(s, w, h, reach as f32));
            for (k, kind) in [FieldKind::BevelInside, FieldKind::BevelOutside].into_iter().enumerate() {
                let [one] =
                    fields_in_windows(false, &shape, region, out, halo, |s, w, h| [photocraft_compose::effects::distance_field(kind, s, w, h, reach as f32)]);
                assert_eq!(bits(&banded[k]), bits(&one), "banded {kind:?}, reach {reach}, {out:?}");
            }
        }
    }
}

/// #3646: a stroke's outer edge is the fitted 5 × 5 chamfer while it is narrow and the exact
/// Euclidean transform once it is wide — the chamfer's octagon facets a wide stroke. The GPU
/// builds it from the field its width picks (`stroke_outside_exact`), while the spread of drop
/// shadows and softer outer glows keeps the chamfer field (fitted to Photoshop, #224) — so
/// routing a wide stroke through the chamfer, or a shadow through the exact field, fails here.
#[test]
fn a_stroke_uses_the_chamfer_narrow_and_the_exact_field_wide() {
    use photocraft_color::{BlendMode, Color};
    use photocraft_doc::{FxCommon, FxPaint, StrokeFx, StrokePosition};
    let light = GlobalLight::default();
    let stroke = |size, position| {
        Effect::Stroke(StrokeFx { common: FxCommon::new(BlendMode::Normal, 1.0), size, position, paint: FxPaint::Color(Color::BLACK), overprint: false })
    };
    // Narrow (< STROKE_CHAMFER_MAX): the chamfer, exactly as before #3646.
    let narrow = program(&stroke(6.0, StrokePosition::Outside), &light, false);
    assert!(narrow.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutside), "a narrow raster outside stroke keeps the chamfer: {:?}", narrow.fields);
    assert!(!narrow.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutsideExact), "…not the exact field: {:?}", narrow.fields);
    // Wide: the exact transform.
    let wide = program(&stroke(40.0, StrokePosition::Outside), &light, false);
    assert!(
        wide.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutsideExact),
        "a wide raster outside stroke needs the exact outer field: {:?}",
        wide.fields
    );
    assert!(!wide.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutside), "…not the chamfer field: {:?}", wide.fields);
    // A shape layer's outline stroke follows the same width rule; the inside edge keeps the chamfer.
    let vector_narrow = program(&stroke(6.0, StrokePosition::Outside), &light, true);
    assert!(vector_narrow.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutsideVectorChamfer), "{:?}", vector_narrow.fields);
    let vector_wide = program(&stroke(40.0, StrokePosition::Outside), &light, true);
    assert!(vector_wide.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutsideVector), "{:?}", vector_wide.fields);
    let inner = program(&stroke(40.0, StrokePosition::Inside), &light, false);
    assert!(inner.fields.iter().any(|(k, _)| *k == FieldKind::StrokeInside), "{:?}", inner.fields);
    // The shadow / softer-glow spread is unchanged: still the chamfer.
    let Effect::DropShadow(mut s) = Effect::default_drop_shadow() else { unreachable!() };
    s.spread = 0.5;
    let shadow = program(&Effect::DropShadow(s), &light, false);
    assert!(shadow.fields.iter().any(|(k, _)| *k == FieldKind::StrokeOutside), "the shadow spread keeps the chamfer: {:?}", shadow.fields);
}

/// Cached shapes and group keys reserve tile addresses without keeping pixels (#4530):
/// damage still tells a replaced tile from an unchanged one, and a dropped tile's pixels
/// are freed although the entry remembers it.
#[test]
fn shape_tiles_and_group_pins_hold_addresses_not_pixels() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut s = Surface::new(fmt);
    // A ramp, so no two tiles share their pixels.
    for x in 0..600 {
        let v = x as f32 / 600.0;
        s.fill_rect(Rect::new(x, 0, x + 1, 300), &[v, 0.2, 0.9, 1.0]);
    }
    let first = s.tiles().find(|(c, _)| c.tx == 0 && c.ty == 0).map(|(_, t)| t.clone()).unwrap();
    assert_eq!(Arc::strong_count(&first), 2);
    let old = snapshot(&s);
    assert_eq!(Arc::strong_count(&first), 2, "the snapshot adds no strong reference");
    assert!(damage(&old, &s).is_empty());
    s.fill_rect(Rect::new(0, 0, 4, 4), &[0.0, 0.0, 0.0, 1.0]);
    let origin = s.tiles().map(|(c, _)| *c).find(|c| c.tx == 0 && c.ty == 0).unwrap();
    assert_eq!(damage(&old, &s), origin.rect(), "only the edited tile");
    drop(first);
    assert!(old.values().any(|w| w.strong_count() == 0), "the replaced tile's pixels are freed");

    let child = Layer::new("c", LayerContent::Raster(s));
    let n = child.surface().map_or(0, |s| s.tile_count());
    let group = Layer::group("g", vec![child]);
    let mut pins = Vec::new();
    let key = group_key(&group, &GlobalLight::default(), &mut |t| pins.push(Arc::downgrade(t)));
    assert_eq!(key, group_key(&group, &GlobalLight::default(), &mut |_| {}), "pinning doesn't change the key");
    assert_eq!(pins.len(), n);
    drop(group);
    assert!(pins.iter().all(|w| w.strong_count() == 0), "a group's pins keep none of its pixels");
}

#[test]
fn bands_end_at_the_coordinate_ceiling() {
    let r = Rect::new(0, i32::MAX - 100, 8, i32::MAX);
    let bands = super::bands(r, 64);
    assert_eq!(bands.len(), 2, "{bands:?}");
    assert_eq!((bands[0].y0, bands[0].y1, bands[1].y1), (i32::MAX - 100, i32::MAX - 36, i32::MAX));
    assert_eq!(super::bands(Rect::new(0, 0, 8, 10), 0).len(), 10, "a zero step still makes progress");
}

/// Cached display pixels used by text and Smart Objects; no font or embedded-file decoding.
fn cached_layer(name: &str, cache: Option<Surface>) -> Layer {
    use photocraft_doc::{SmartObject, SmartSource, TextLayer};
    match name {
        "text" => Layer::new(name, LayerContent::Text(TextLayer { cache, ..Default::default() })),
        "smart" => Layer::new(
            name,
            LayerContent::Smart(SmartObject::new(
                SmartSource::Embedded { file_name: "synthetic.png".into(), bytes: Arc::new(Vec::new()) },
                photocraft_geom::Affine::IDENTITY,
                cache,
            )),
        ),
        _ => Layer::new(name, LayerContent::Raster(cache.unwrap())),
    }
}

/// Manual component benchmark: exact coverage-cache verification, excluding GPU work and presentation.
#[test]
#[ignore = "manual 20 MP effect-coverage benchmark; run with --release --ignored --nocapture"]
fn cached_coverage_benchmark() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    let doc = Document::new("coverage bench", Size::new(5472, 3648), ColorMode::Rgb, SampleType::U8);
    let mut surface = Surface::new(doc.pixel_format());
    surface.fill_rect(doc.bounds(), &[0.2, 0.3, 0.4, 0.7]);
    let region = Rect::new(-30, -30, 5502, 3678);
    for kind in ["raster", "text", "smart"] {
        let layer = cached_layer(kind, Some(surface.clone()));
        let cached = shape(&doc, &layer, region);
        assert!(shape_matches(&doc, &layer, region, &cached));
        let mut samples = Vec::new();
        for _ in 0..11 {
            let t = std::time::Instant::now();
            assert!(std::hint::black_box(shape_matches(&doc, &layer, region, &cached)));
            samples.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("COVERAGE {kind} {samples:?}");
    }
}

/// Missing and replaced display caches must never reuse stale effect coverage.
#[test]
fn cached_coverage_tracks_missing_and_replaced_surfaces() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    let doc = Document::new("caches", Size::new(12, 10), ColorMode::Rgb, SampleType::U8);
    for kind in ["text", "smart"] {
        let absent = cached_layer(kind, None);
        assert_eq!(shape_band(&doc, &absent, doc.bounds()), photocraft_compose::layer_shape(&doc, &absent, doc.bounds()));
        let first = cached_layer(kind, Some(Surface::with_default(PixelFormat::RGBA8, &[0.3, 0.4, 0.5, 0.2])));
        let old = shape(&doc, &first, doc.bounds());
        let second = cached_layer(kind, Some(Surface::with_default(PixelFormat::RGBA8, &[0.3, 0.4, 0.5, 0.8])));
        assert!(!shape_matches(&doc, &second, doc.bounds(), &old));
        assert_eq!(shape_band(&doc, &second, doc.bounds()), photocraft_compose::layer_shape(&doc, &second, doc.bounds()));
    }
}

/// Recycled coverage must be reset when a following layer has opaque defaults or no alpha.
#[test]
fn reused_coverage_bands_preserve_changing_formats_and_defaults() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    let doc = Document::new("reuse", Size::new(512, 128), ColorMode::Rgb, SampleType::U8);
    for kind in ["raster", "text", "smart"] {
        for depth in SampleType::ALL {
            for alpha in [true, false, true] {
                let fmt = PixelFormat::new(ColorMode::Rgb, depth, alpha);
                let mut surface = Surface::with_default(fmt, &[0.3, 0.4, 0.5, 0.7]);
                surface.fill_rect(Rect::new(4, 8, 50, 70), &[0.1, 0.2, 0.3, 0.0]);
                let layer = cached_layer(kind, Some(surface));
                let mut out = shape_band(&doc, &layer, doc.bounds());
                assert_eq!(out, photocraft_compose::layer_shape(&doc, &layer, doc.bounds()), "{kind} {fmt:?}");
                out.fill(f32::NAN);
                photocraft_raster::scratch::give(out);
            }
        }
    }
}
