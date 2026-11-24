use super::*;
use photocraft_color::BlendMode;
use photocraft_doc::FxCommon;

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

/// #14933: the Size 1 Inner Shadow program convolves the ¼–½–¼ taps the CPU's `shadow_map`
/// applies, instead of dropping the identity width-1 blur, so both compositors build the same map.
/// A Size 6 inner shadow still takes the tent of its own size, untouched.
#[test]
fn size_one_inner_shadow_convolves_the_cpus_kernel() {
    use crate::plan::Kernel;
    let light = GlobalLight::default();
    let Effect::DropShadow(mut s) = Effect::default_drop_shadow() else { unreachable!() };
    s.distance = 0.0;
    s.spread = 0.0;
    let mut blurs = |size: f32| {
        s.size = size;
        program(&Effect::InnerShadow(s.clone()), &light, false)
            .stages
            .into_iter()
            .filter(|st| st.kernel == Kernel::MBlur)
            .map(|st| (st.radius, st.lut.map(|l| (*l).clone()).unwrap_or_default()))
            .collect::<Vec<_>>()
    };
    let one = blurs(1.0);
    assert_eq!(one.len(), 2, "two separable passes over the Size 1 inner shadow map");
    let (r1, w1) = photocraft_compose::effects::size_one_kernel();
    for (radius, lut) in &one {
        assert_eq!(*radius, r1, "the Size 1 kernel radius");
        assert_eq!(lut.as_slice(), w1.as_slice(), "the taps the CPU map applies");
    }
    let (r6, w6) = photocraft_compose::effects::tent_kernel(6.0);
    let six = blurs(6.0);
    assert_eq!(six.len(), 2, "a Size 6 inner shadow still blurs twice");
    for (radius, lut) in &six {
        assert_eq!(*radius, r6);
        assert_eq!(lut.as_slice(), w6.as_slice(), "Size 6 stays on the tent");
    }
}

/// #14934: the GPU's Inner Shadow program takes the same sub-pixel source offset as the CPU's
/// `shadow_map` - without a device, by asking what stages the effect builds. The measured gate
/// (Anti-aliased on, Gaussian contour, the inner branch) is what adds the stage; a Linear contour
/// or a Drop Shadow keeps the program, and the program key, exactly as it was.
#[test]
fn anti_aliased_inner_shadow_shifts_the_map_stage() {
    use photocraft_doc::{Contour, FxCommon, Shadow};
    let pts: [(f32, f32); 11] = [
        (0.0, 0.0),
        (0.117, 0.023),
        (0.172, 0.055),
        (0.234, 0.117),
        (0.305, 0.234),
        (0.383, 0.414),
        (0.523, 0.688),
        (0.617, 0.812),
        (0.703, 0.891),
        (0.805, 0.953),
        (1.0, 1.0),
    ];
    let gaussian = Contour::Custom {
        name: "Gaussian".into(),
        points: pts.iter().map(|(input, output)| photocraft_doc::adjust::CurvePoint { input: *input, output: *output }).collect(),
        corners: (0..pts.len()).map(|i| i > 0 && i < pts.len() - 1).collect(),
    };
    let shadow = |anti_aliased: bool, contour: Contour| Shadow {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 0.0,
        use_global_light: false,
        distance: 0.0,
        spread: 0.0,
        size: 10.0,
        contour,
        anti_alias: anti_aliased,
        noise: 0.0,
        knocks_out: false,
    };
    fn shift_stages(p: &MapProgram) -> Vec<&Stage> {
        p.stages.iter().filter(|s| s.kernel == Kernel::MShift).collect()
    }
    let light = GlobalLight::default();
    let aa = photocraft_compose::effects::aa_shift(true, &gaussian, 10.0, 0.0);
    assert_eq!(aa, -1.0 / 3.0, "the CPU offset the GPU has to copy at the measured Size 10, Choke 0");
    // The inner branch always takes one shift stage: it is where `1 - alpha` happens. At Distance 0
    // without Anti-aliased that stage is the identity, and that is what must change.
    let off = program(&Effect::InnerShadow(shadow(false, gaussian.clone())), &light, false);
    assert_eq!(shift_stages(&off).len(), 1, "the inner branch keeps its one (identity) shift stage");
    assert_eq!(shift_stages(&off)[0].p0, [0.0, 0.0, 1.0, 1.0], "p0 = (dx, dy, outside, invert) at Distance 0, Anti-aliased off");
    let on = program(&Effect::InnerShadow(shadow(true, gaussian.clone())), &light, false);
    assert_eq!(shift_stages(&on).len(), 1, "the Anti-aliased inner shadow still builds one shift stage, not two");
    assert_eq!(shift_stages(&on)[0].p0, [aa, aa, 1.0, 1.0], "the shift stage must carry the CPU's sub-pixel source offset");
    assert!(
        shift_stages(&on)[0].radius >= 1,
        "a sub-pixel shift reads the tap beyond its base, so the stage radius must cover it: {}",
        shift_stages(&on)[0].radius
    );
    assert_ne!(on.key, off.key, "the flag has to invalidate the cached program");
    // Unmeasured cases keep today's stage: the contour gate is the CPU's.
    let linear = program(&Effect::InnerShadow(shadow(true, Contour::Linear)), &light, false);
    assert_eq!(shift_stages(&linear)[0].p0, [0.0, 0.0, 1.0, 1.0], "a Linear inner shadow still ignores Anti-aliased (#14934 leaves that contour unmeasured)");
    // A Drop Shadow (its own item is #14881) keeps its whole-pixel program either way.
    let drop_on = program(&Effect::DropShadow(shadow(true, gaussian.clone())), &light, false);
    let drop_off = program(&Effect::DropShadow(shadow(false, gaussian)), &light, false);
    assert_eq!(shift_stages(&drop_on).len(), 0, "a Drop Shadow at Distance 0 takes no shift stage");
    assert_eq!(drop_on.key, drop_off.key, "the Drop Shadow still ignores Anti-aliased (#14881 is its own item)");
}

/// #14934 review: the GPU's gate is the CPU's rule, including its two degenerate neighbours. At
/// Size 1, and at Size 2 with Choke 50 %, the blur the map gets has width 1, which is the
/// identity, so a sub-pixel source offset would be the entire map; at any Choke that dilates the
/// CPU recomputes its distance field on the offset map while this builder shifts a whole-pixel
/// field of the unshifted shape. In every one of those the program must be the Anti-aliased-off
/// program, stage for stage and key for key. Away from the degeneracies the offset follows the
/// inputs, not the measured Size 10.
#[test]
fn anti_aliased_inner_shadow_stage_follows_the_measured_geometry() {
    use photocraft_doc::{Contour, FxCommon, Shadow};
    let pts: [(f32, f32); 11] = [
        (0.0, 0.0),
        (0.117, 0.023),
        (0.172, 0.055),
        (0.234, 0.117),
        (0.305, 0.234),
        (0.383, 0.414),
        (0.523, 0.688),
        (0.617, 0.812),
        (0.703, 0.891),
        (0.805, 0.953),
        (1.0, 1.0),
    ];
    let gaussian = || Contour::Custom {
        name: "Gaussian".into(),
        points: pts.iter().map(|(input, output)| photocraft_doc::adjust::CurvePoint { input: *input, output: *output }).collect(),
        corners: (0..pts.len()).map(|i| i > 0 && i < pts.len() - 1).collect(),
    };
    let shadow = |size: f32, spread: f32, distance: f32, anti_aliased: bool| Shadow {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 0.0,
        use_global_light: false,
        distance,
        spread,
        size,
        contour: gaussian(),
        anti_alias: anti_aliased,
        noise: 0.0,
        knocks_out: false,
    };
    // The inner branch always builds its source shift stage (it is where `1 - alpha` happens); a
    // Choke that dilates at a Distance other than 0 adds a second one for the distance field.
    let shift_key = |e: &Effect| -> (Vec<[f32; 4]>, u64) {
        let p = program(e, &GlobalLight::default(), false);
        (p.stages.iter().filter(|s| s.kernel == Kernel::MShift).map(|s| s.p0).collect(), p.key)
    };
    let source_shift = |e: &Effect| -> [f32; 4] {
        let (stages, _) = shift_key(e);
        assert!(!stages.is_empty(), "the inner branch builds its source shift stage for {e:?}");
        stages[0]
    };
    for (size, spread, what) in [
        (1.0, 0.0, "Size 1 (blur width 1: the identity)"),
        (2.0, 0.5, "Size 2 Choke 50 % (blur width 1)"),
        (10.0, 0.5, "Size 10 Choke 50 % (the CPU dilates an offset map, the GPU a whole-pixel field)"),
        (10.0, 0.05, "Size 10 Choke 5 % (dilates 1 px too)"),
    ] {
        let on = shift_key(&Effect::InnerShadow(shadow(size, spread, 0.0, true)));
        let off = shift_key(&Effect::InnerShadow(shadow(size, spread, 0.0, false)));
        assert_eq!(on, off, "the {what} inner shadow program moved with Anti-aliased: on {:?}, off {:?}", on.0, off.0);
    }
    // Away from the degeneracies the rule is in the inputs: Size 1.5 and Size 20 take the offset
    // exactly as the measured Size 10 does, and so does a Choke that dilates nothing.
    let aa = -1.0f32 / 3.0;
    for size in [1.5, 2.0, 3.0, 20.0] {
        let p0 = source_shift(&Effect::InnerShadow(shadow(size, 0.0, 0.0, true)));
        assert_eq!(p0, [aa, aa, 1.0, 1.0], "Size {size} at Choke 0 must carry the CPU's sub-pixel offset");
    }
    let thin = shift_key(&Effect::InnerShadow(shadow(10.0, 0.04, 0.0, true)));
    assert_eq!(thin.0, vec![[aa, aa, 1.0, 1.0]], "Choke 4 % dilates nothing (round(0.4) = 0), so the map shape is the measured one");
    let thin_off = shift_key(&Effect::InnerShadow(shadow(10.0, 0.04, 0.0, false)));
    assert_ne!(thin.1, thin_off.1, "and the flag still invalidates that program's key");
    // The gate reads the Size and Choke, not the measured Distance 0: at Distance 5 (angle 0, so a
    // whole-pixel light offset of -5, 0) the Choke 0 program carries that offset plus the third of
    // a pixel, exactly as `shadow_map` shifts by `dx + aa`; the Choke 50 % program at the same
    // Distance stays whole-pixel, gated like every other dilating Choke, and keeps its second
    // (distance field) shift stage exactly as it does with the flag off.
    let far_on = source_shift(&Effect::InnerShadow(shadow(10.0, 0.0, 5.0, true)));
    assert_eq!(far_on, [-5.0 + aa, 0.0 + aa, 1.0, 1.0], "Size 10 Choke 0 at Distance 5 must carry the light offset plus the sub-pixel offset");
    let far_off = source_shift(&Effect::InnerShadow(shadow(10.0, 0.0, 5.0, false)));
    assert_eq!(far_off, [-5.0, 0.0, 1.0, 1.0], "with the flag off the shift stays the whole-pixel light offset");
    let far_on_key = shift_key(&Effect::InnerShadow(shadow(10.0, 0.0, 5.0, true))).1;
    assert_ne!(far_on_key, shift_key(&Effect::InnerShadow(shadow(10.0, 0.0, 5.0, false))).1, "and the flag invalidates the Distance 5 program key");
    let far_choke_on = shift_key(&Effect::InnerShadow(shadow(10.0, 0.5, 5.0, true)));
    let far_choke_off = shift_key(&Effect::InnerShadow(shadow(10.0, 0.5, 5.0, false)));
    assert_eq!(far_choke_on.0.len(), 2, "Choke 50 % at Distance 5 shifts the source and the distance field");
    assert_eq!(far_choke_on, far_choke_off, "the Choke 50 % program at Distance 5 stayed whole-pixel: on {:?}, off {:?}", far_choke_on.0, far_choke_off.0);
}
