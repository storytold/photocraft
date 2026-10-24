//! Exact graph resource boundaries, not a source object deserializer/evaluator.
//! Facts: pinned GEGL0.4.72 contrast-curve property declarations, gegl-op.h
//! property_curve default, GeglCurve public API, and piecewise-blend pad schema.
//! Actual stock2.99.18 saved/reopened probes are recorded under
//! plan/003-gimp-2.99.18-research/graph-resource-probe (local evidence).
use super::graph_schema::Kind;
use super::residual_spatial::invalid;
use super::{MappingContext, MappingFailure, MappingResult, NativeMapping, NativeMaskComponent, NativeStage, StoredEffect, native_filter};
use photocraft_color::{BlendMode, ColorMode};
use serde_json::json;

const NAMES: &[&str] = &["gegl:contrast-curve", "gegl:piecewise-blend"];
const LEVEL_PADS: &[&str] =
    &["aux1", "aux2", "aux3", "aux4", "aux5", "aux6", "aux7", "aux8", "aux9", "aux10", "aux11", "aux12", "aux13", "aux14", "aux15", "aux16"];

pub(super) fn operation(token: &str) -> Option<&'static str> {
    NAMES.iter().copied().find(|name| *name == token || name.strip_prefix("gegl:") == Some(token))
}

/// Actual declared image pads. These are graph edges, never scalar properties.
pub(super) fn level_pad(name: &str) -> Option<usize> {
    LEVEL_PADS.iter().position(|pad| *pad == name).map(|index| index + 1)
}

pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    match (op, name) {
        ("gegl:contrast-curve", "sampling-points") => Some(Kind::Integer { min: 0, max: 65_536 }),
        ("gegl:contrast-curve", "curve") => Some(Kind::Unavailable(
            "actual GeglCurve bounds/control points have no admitted graph-chain literal serializer; XML curve-point data is a separate source format",
        )),
        ("gegl:piecewise-blend", "levels") => Some(Kind::Integer { min: 0, max: 16 }),
        ("gegl:piecewise-blend", "gamma") => Some(Kind::Number { min: 0., max: f64::MAX }),
        ("gegl:piecewise-blend", "linear-mask") => Some(Kind::Boolean),
        _ => None,
    }
}

fn integer(e: &StoredEffect<'_>, name: &str, default: i32, maximum: i32) -> Result<i32, MappingFailure> {
    let value = e.number(name).map_err(invalid)?.unwrap_or(f64::from(default));
    if !value.is_finite() || value.fract() != 0. || !(0. ..=f64::from(maximum)).contains(&value) {
        return Err(invalid(format!("{} {name} is outside its declared integer domain", e.operation)));
    }
    Ok(value as i32)
}

fn map_known(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> Result<NativeMapping, MappingFailure> {
    c.interrupt.check().map_err(|error| error.to_string())?;
    for argument in &e.arguments {
        e.argument(argument.name).map_err(invalid)?;
        if field(e.operation, argument.name).is_none() {
            return Err(invalid(format!(
                "{} has no scalar property {}; named level-image pads require actual structured graph edges",
                e.operation, argument.name
            )));
        }
    }
    match e.operation {
        "gegl:contrast-curve" => {
            let sampling = integer(e, "sampling-points", 0, 65_536)?;
            if e.argument("curve").map_err(invalid)?.is_some() {
                return Err(MappingFailure::resource(
                    "A nondefault Contrast Curve requires the actual GeglCurve y bounds and control points. They are not represented by a scalar/string object name here; no curve is invented from the owner image.",
                ));
            }
            let mut filters = Vec::new();
            if c.mode == ColorMode::Rgb {
                filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]})));
            } else if c.mode != ColorMode::Grayscale {
                return Err("Default contrast curve native luminance policy requires RGB/Gray contents".into());
            }
            Ok(NativeMapping {
                filters,
                notes: vec![format!(
                    "Absent curve uses the declared GeglCurve default identity transfer, not an invented image or custom curve. Source YA grayscale intent becomes ordinary native monochrome Channel Mixer with native luminance weights (Gray remains Gray). Source sampling-points={sampling} is superseded by the deterministic native transfer. Native profile, depth, HDR and alpha rules apply; source Babl grayscale/outer filter compositing is not emulated."
                )],
                ..Default::default()
            })
        }
        "gegl:piecewise-blend" => {
            let levels = integer(e, "levels", 0, 16)?;
            let gamma = e.number("gamma").map_err(invalid)?.unwrap_or(1.);
            if !gamma.is_finite() || gamma < 0. {
                return Err(invalid("piecewise-blend gamma is outside its finite nonnegative source domain"));
            }
            let linear = e.boolean("linear-mask").map_err(invalid)?.unwrap_or(true);
            if levels == 0 {
                return Ok(NativeMapping {
                    filters: vec![native_filter("filter.other.setAlpha", json!({"value":0.}))],
                    notes: vec![format!(
                        "Stock saved/reopened zero-level Piecewise Blend has zero visible contribution (partial-alpha outer Normal preview stays byte-identical to its original owner, unlike an input identity). This becomes ordinary native Set Alpha0, preserving native hidden colors/HDR rather than executing a source blend. Source gamma={gamma} and linear-mask={linear} are inactive without levels; native document alpha/compositing rules apply."
                    )],
                    ..Default::default()
                });
            }
            Err(MappingFailure::resource(format!(
                "Piecewise Blend declares {levels} actual level images on aux1..aux{levels}. This unary mapping received no admitted level-image bindings; images must come from the real serialized graph edges, never the owner or a default image. Named-pad syntax is portable and independently stock-saved/reopened, so available edges require the structured native adapter rather than a blanket unsupported classification."
            )))
        }
        _ => Err(invalid("unknown exact resource operation")),
    }
}

pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> MappingResult {
    operation(e.operation)?;
    Some(map_known(e, c))
}

/// Lower only the actual supplied image edges. Indices already name earlier
/// native results; `next_global` is the first new result index, not a node id.
pub(super) fn piecewise(
    e: &StoredEffect<'_>,
    c: &MappingContext<'_>,
    main: usize,
    pads: &[(usize, usize)],
    next_global: usize,
) -> Result<(Vec<NativeStage>, usize, Vec<String>), MappingFailure> {
    if e.operation != "gegl:piecewise-blend" || next_global == 0 || next_global > 65 || main >= next_global || pads.len() > 16 {
        return Err(invalid("Piecewise native lowering requires its exact operation, bounded earlier main input and at most16 named image pads"));
    }
    c.interrupt.check().map_err(|error| error.to_string())?;
    for argument in &e.arguments {
        e.argument(argument.name).map_err(invalid)?;
        if field(e.operation, argument.name).is_none() {
            return Err(invalid("Piecewise image edges are not scalar arguments"));
        }
    }
    let levels = integer(e, "levels", 0, 16)? as usize;
    let gamma = e.number("gamma").map_err(invalid)?.unwrap_or(1.);
    if !gamma.is_finite() || gamma < 0. {
        return Err(invalid("Piecewise gamma must be finite and nonnegative"));
    }
    let linear = e.boolean("linear-mask").map_err(invalid)?.unwrap_or(true);
    let mut images = [None; 16];
    for &(pad, input) in pads {
        let slot = pad.checked_sub(1).and_then(|index| images.get_mut(index)).ok_or_else(|| invalid("Unknown Piecewise level-image pad"))?;
        if input >= next_global || slot.replace(input).is_some() {
            return Err(invalid("Piecewise level-image binding is duplicated or does not reference an earlier native result"));
        }
    }
    for image in images.iter().take(levels) {
        if image.is_none() {
            return Err(MappingFailure::resource("Piecewise positive levels require every actual aux1..auxN image; no owner/default image is substituted"));
        }
    }
    let unused = pads.iter().filter(|(pad, _)| *pad > levels).count();
    if levels == 0 {
        if next_global > 64 {
            return Err(MappingFailure::resource("Piecewise zero coverage exceeds the64 native stage allowance"));
        }
        let mapping = map_known(e, c)?;
        return Ok((
            vec![NativeStage::Apply { input: main, mapping: Box::new(mapping) }],
            next_global,
            vec![format!("Zero active Piecewise levels leave {unused} declared image edges inactive; no substitute image is constructed.")],
        ));
    }
    let mut output = images.first().copied().flatten().ok_or_else(|| MappingFailure::resource("Missing actual first Piecewise image"))?;
    let note = format!(
        "Piecewise{levels} uses only actual connected aux1..aux{levels} native images. One level selects that image directly. Multiple levels use native composite-channel capture (arithmetic color-channel mean), editable256-point pencil Curves cumulative membership ramps and static native pixel masks, then native Normal joins. Source gamma={gamma} becomes bounded native Exposure gamma{} in0.01..9.99; native clipping/evaluation applies. Source linear-mask={linear} is superseded by the actual native document/channel/profile policy, not a retained Babl transfer switch. Native partial-alpha layering differs from source weighted interpolation; no source kernel remains. {unused} higher declared pads are inactive.",
        gamma.clamp(0.01, 9.99)
    );
    if levels == 1 {
        return Ok((
            Vec::new(),
            output,
            vec![format!(
                "Piecewise one level selects only actual aux1 directly. Main mask, gamma={gamma} and linear-mask={linear} do not select between images; {unused} higher declared image pads are inactive. No owner/default image is substituted; ordinary native alpha/profile/compositing apply."
            )],
        ));
    }
    let native_gamma = gamma.clamp(0.01, 9.99);
    let gamma_stage = usize::from(native_gamma != 1.);
    let count = 1 + gamma_stage + 3 * (levels - 1);
    if next_global.checked_sub(1).and_then(|used| used.checked_add(count)).is_none_or(|total| total > 64) {
        return Err(MappingFailure::resource("Piecewise native channel/curve/mask/join plan exceeds the shared64 stage allowance"));
    }
    let table_entries = (levels - 1) * 256;
    let metadata = table_entries * 24 + 4096;
    let scratch = table_entries * (2 * std::mem::size_of::<serde_json::Value>() + 16) + count * std::mem::size_of::<NativeStage>() + 4096;
    if metadata > c.remaining_metadata_bytes || scratch > c.remaining_generation_bytes {
        return Err(MappingFailure::resource("Piecewise native curve-table construction exceeds remaining metadata/working allowance"));
    }
    let mut steps = Vec::new();
    steps.try_reserve_exact(count).map_err(|_| MappingFailure::resource("Piecewise native stage reservation unavailable"))?;
    steps.push(NativeStage::Channel { input: main, component: NativeMaskComponent::Composite });
    let mut channel = next_global;
    if gamma_stage != 0 {
        steps.push(NativeStage::Apply {
            input: channel,
            mapping: Box::new(NativeMapping {
                filters: vec![native_filter("image.adjustments.exposure", json!({"exposure":0.,"offset":0.,"gamma":native_gamma}))],
                ..Default::default()
            }),
        });
        channel = next_global + steps.len() - 1;
    }
    for level in 1..levels {
        c.interrupt.check().map_err(|error| error.to_string())?;
        let mut table = Vec::new();
        table.try_reserve_exact(256).map_err(|_| MappingFailure::resource("Piecewise native curve-table reservation unavailable"))?;
        for sample in 0..256 {
            // Original native authoring policy: opaque Normal layers select
            // neighboring levels by cumulative ramps; no source sampler used.
            let weight = (f64::from(sample) / 255. * (levels - 1) as f64 - (level - 1) as f64).clamp(0., 1.);
            table.push(weight * 255.);
        }
        steps.push(NativeStage::Apply {
            input: channel,
            mapping: Box::new(NativeMapping {
                filters: vec![native_filter("image.adjustments.curves", json!({"pencil":true,"points":table}))],
                ..Default::default()
            }),
        });
        let mask = next_global + steps.len() - 1;
        let image = images.get(level).copied().flatten().ok_or_else(|| MappingFailure::resource("Missing actual Piecewise level image"))?;
        steps.push(NativeStage::Mask { input: image, mask, component: NativeMaskComponent::Composite, invert: false, scale: 1. });
        let above = next_global + steps.len() - 1;
        steps.push(NativeStage::Join { below: output, above, blend: BlendMode::Normal, opacity: 1. });
        output = next_global + steps.len() - 1;
    }
    Ok((steps, output, vec![note]))
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue};
    use super::*;
    use photocraft_doc::import_conversion::ConversionReason;
    use photocraft_geom::Rect;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            resolution_dpi: 72.,
            linear: false,
            owner_bounds: Rect::new(0, 0, 8, 6),
            native_filter_bounds: Rect::new(0, 0, 8, 6),
            target_profile: None,
            remaining_generation_bytes: usize::MAX,
            remaining_metadata_bytes: usize::MAX,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn n(name: &'static str, value: f64) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Literal(value) }
    }
    #[test]
    fn exact_resource_schema_distinguishes_scalar_fields_and_actual_level_pads() {
        assert_eq!(operation("contrast-curve"), Some("gegl:contrast-curve"));
        assert_eq!(operation("piecewise-blend"), Some("gegl:piecewise-blend"));
        assert_eq!(operation("gimp:piecewise-blend"), None);
        for i in 1..=16 {
            assert_eq!(level_pad(&format!("aux{i}")), Some(i));
        }
        for name in ["aux", "aux0", "aux17", "aux01", "input"] {
            assert_eq!(level_pad(name), None);
        }
        assert_eq!(field("gegl:piecewise-blend", "aux1"), None);
        assert!(matches!(field("gegl:contrast-curve", "curve"), Some(Kind::Unavailable(_))));
    }
    #[test]
    fn declared_default_curve_is_native_gray_while_actual_custom_curve_is_named_missing_resource() {
        let mapped = map(&StoredEffect::test("gegl:contrast-curve", vec![n("sampling-points", 65536.)]), &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].command, "image.adjustments.channelMixer");
        assert_eq!(mapped.filters[0].params, json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]}));
        let custom = StoredEffect::test("gegl:contrast-curve", vec![Argument { name: "curve", value: ArgumentValue::String("missing-object") }]);
        assert_eq!(map(&custom, &context()).unwrap().unwrap_err().reason, ConversionReason::ResourceUnavailable);
        let mut gray = context();
        gray.mode = ColorMode::Grayscale;
        assert!(map(&StoredEffect::test("gegl:contrast-curve", vec![]), &gray).unwrap().unwrap().filters.is_empty());
        for value in [-1., 65537., 0.5, f64::NAN] {
            assert!(map(&StoredEffect::test("gegl:contrast-curve", vec![n("sampling-points", value)]), &context()).unwrap().is_err());
        }
    }
    #[test]
    fn zero_levels_is_native_zero_coverage_and_positive_levels_need_real_images() {
        let mapped = map(&StoredEffect::test("gegl:piecewise-blend", vec![]), &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].command, "filter.other.setAlpha");
        assert_eq!(mapped.filters[0].params, json!({"value":0.}));
        for levels in [1., 16.] {
            let err = map(&StoredEffect::test("gegl:piecewise-blend", vec![n("levels", levels)]), &context()).unwrap().unwrap_err();
            assert_eq!(err.reason, ConversionReason::ResourceUnavailable);
            assert!(err.message.contains("portable"));
        }
        for args in [vec![n("levels", 17.)], vec![n("levels", 0.5)], vec![n("gamma", -1.)], vec![n("gamma", f64::INFINITY)], vec![n("linear-mask", 1.)]] {
            assert!(map(&StoredEffect::test("gegl:piecewise-blend", args), &context()).unwrap().is_err());
        }
    }
    #[test]
    fn actual_piecewise_images_have_bounded_backward_native_stages() {
        let e = StoredEffect::test("gegl:piecewise-blend", vec![n("levels", 1.)]);
        let (steps, output, notes) = piecewise(&e, &context(), 0, &[(1, 2)], 3).unwrap();
        assert!(steps.is_empty());
        assert_eq!(output, 2);
        assert!(notes[0].contains("actual aux1"));
        let e = StoredEffect::test("gegl:piecewise-blend", vec![n("levels", 16.), n("gamma", 2.5)]);
        let pads = (1..=16).map(|i| (i, i)).collect::<Vec<_>>();
        let (steps, output, _) = piecewise(&e, &context(), 0, &pads, 18).unwrap();
        assert_eq!(steps.len(), 47);
        assert_eq!(output, 64);
        assert!(matches!(steps[0], NativeStage::Channel { input: 0, component: NativeMaskComponent::Composite }));
        let NativeStage::Apply { mapping, .. } = &steps[1] else { panic!("native gamma") };
        assert_eq!(mapping.filters[0].params, json!({"exposure":0.,"offset":0.,"gamma":2.5}));
        let mut full = (0..17).map(|_| NativeStage::Apply { input: 0, mapping: Box::default() }).collect::<Vec<_>>();
        full.extend(steps);
        super::super::validate_stages(&full, output).unwrap();
        assert_eq!(piecewise(&e, &context(), 0, &pads, 19).unwrap_err().reason, ConversionReason::ResourceUnavailable);
    }
    #[test]
    fn piecewise_missing_images_differ_from_malformed_edges_and_budget_rejection() {
        let e = StoredEffect::test("gegl:piecewise-blend", vec![n("levels", 2.)]);
        assert_eq!(piecewise(&e, &context(), 0, &[(1, 1)], 3).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        for pads in [vec![(0, 1), (2, 2)], vec![(17, 1), (2, 2)], vec![(1, 1), (1, 2)], vec![(1, 1), (2, 3)]] {
            assert_eq!(piecewise(&e, &context(), 0, &pads, 3).unwrap_err().reason, ConversionReason::InvalidOptionalData);
        }
        let mut c = context();
        c.remaining_generation_bytes = 1;
        assert_eq!(piecewise(&e, &c, 0, &[(1, 1), (2, 2)], 3).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        c = context();
        c.remaining_metadata_bytes = 1;
        assert_eq!(piecewise(&e, &c, 0, &[(1, 1), (2, 2)], 3).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        let cancelled = || true;
        c = context();
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        assert!(piecewise(&e, &c, 0, &[(1, 1), (2, 2)], 3).is_err());
    }
    #[test]
    fn actual_native_curve_mask_and_normal_join_match_independent_counterpart_and_reopen() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::adjust::ToneSpace;
        use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerMask};
        use photocraft_geom::Size;
        use photocraft_raster::Surface;
        let e = StoredEffect::test("gegl:piecewise-blend", vec![n("levels", 2.)]);
        let (steps, output, _) = piecewise(&e, &context(), 0, &[(1, 1), (2, 2)], 3).unwrap();
        assert_eq!(output, 6);
        assert_eq!(steps.len(), 4);
        let NativeStage::Apply { input: 3, mapping } = &steps[1] else { panic!("native mask curve") };
        let values = mapping.filters[0].params["points"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32 / 255.).collect::<Vec<_>>();
        assert_eq!(values.len(), 256);
        assert!(matches!(steps[2], NativeStage::Mask { input: 2, mask: 4, component: NativeMaskComponent::Composite, invert: false, scale: 1. }));
        assert!(matches!(steps[3], NativeStage::Join { below: 1, above: 5, blend: BlendMode::Normal, opacity: 1. }));
        let area = Rect::new(0, 0, 5, 1);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut channel = Surface::new(fmt);
            for x in 0..5 {
                let v = x as f32 / 4.;
                channel.write_pixel(x, 0, &[v, v, v, 1.]);
            }
            let mut mask_doc = Document::new("Native captured channel", Size::new(5, 1), ColorMode::Rgb, depth);
            mask_doc.layers.push(Layer::new("Captured composite", LayerContent::Raster(channel.clone())));
            mask_doc.layers.push(Layer::new(
                "Native membership curve",
                LayerContent::Adjustment(Adjustment::Curves {
                    master: photocraft_doc::adjust::curve_map(&values),
                    per_channel: std::array::from_fn(|_| Vec::new()),
                    space: ToneSpace::Rgb,
                    black: Vec::new(),
                    pencil: true,
                }),
            ));
            let mask_pixels = photocraft_compose::render(&mask_doc, area);
            let mut actual_mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            let mut expected_mask = actual_mask.clone();
            for x in 0..5 {
                actual_mask.write_pixel(x, 0, &[mask_pixels.get(x, 0)[0]]);
                // An ordinary native adjustment commits integer results at
                // document precision: U16 adjustments use 32768 steps, while
                // raster storage uses 65535. The independent identity-ramp
                // counterpart must include that boundary before mask capture.
                let value = channel.pixel(x, 0)[0];
                let expected = match depth {
                    SampleType::U8 => (value * 255. + 0.5).floor() / 255.,
                    SampleType::U16 => (value * 32768. + 0.5).floor() / 32768.,
                    SampleType::F32 => value,
                };
                expected_mask.write_pixel(x, 0, &[expected]);
            }
            let build = |mask| {
                let mut lower = Surface::new(fmt);
                lower.fill_rect(area, &[0., 0., 1., 0.75]);
                let mut upper = Surface::new(fmt);
                upper.fill_rect(area, &[1., 0., 0., 0.5]);
                let mut doc = Document::new("Native level join", Size::new(5, 1), ColorMode::Rgb, depth);
                doc.layers.push(Layer::new("Actual level1", LayerContent::Raster(lower)));
                let mut top = Layer::new("Actual level2", LayerContent::Raster(upper));
                top.mask = Some(LayerMask { surface: mask, ..LayerMask::reveal_all() });
                doc.layers.push(top);
                doc
            };
            let actual = build(actual_mask);
            let expected = build(expected_mask);
            let before = photocraft_compose::render(&actual, area).px;
            let counterpart = photocraft_compose::render(&expected, area).px;
            if depth == SampleType::F32 {
                // Native Curves samples its pencil ramp into a 4096-entry
                // float LUT, then interpolates that LUT during evaluation.
                // The independently direct mask has no such interpolation:
                // an identity ramp can therefore differ by a float ULP
                // before native Normal unpremultiplication. Bound that
                // arithmetic difference tightly; integer commits stay exact.
                for (a, b) in before.iter().flatten().zip(counterpart.iter().flatten()) {
                    assert!(a.is_finite() && b.is_finite() && *a >= 0. && *b >= 0.);
                    assert!(a.to_bits().abs_diff(b.to_bits()) <= 2, "native float LUT counterpart: {a} vs {b}");
                }
            } else {
                assert_eq!(before, counterpart);
            }
            assert!(before[2][3] > 0.75 && before[2][3] < 1.);
            let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(before, photocraft_compose::render(&reopened, area).px);
        }
    }
}
