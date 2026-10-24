//! Finite actual-input native mask/compositing policies. W3C Compositing1 §9.1
//! supplies Porter–Duff intent; GIMP's public layer-mode manual supplies Split
//! and PassThrough intent. Output uses native masks/joins/adjustments exclusively.
use super::residual_spatial::{generation_bytes, invalid};
use super::{
    MappingContext, MappingFailure, NativeMapping, NativeMaskComponent, NativeStage, StoredEffect,
    graph_schema::{self, Kind},
    native_filter,
};
use photocraft_color::BlendMode;
use serde_json::json;

const SVG: &[(&str, &str)] = &[
    ("svg:clear", "gegl:clear"),
    ("svg:src-in", "gegl:src-in"),
    ("svg:dst-in", "gegl:dst-in"),
    ("svg:src-out", "gegl:src-out"),
    ("svg:dst-out", "gegl:dst-out"),
    ("svg:src-atop", "gegl:src-atop"),
    ("svg:dst-atop", "gegl:dst-atop"),
    ("svg:xor", "gegl:xor"),
];
const GIMP: &[&str] = &["gimp:anti-erase", "gimp:split", "gimp:pass-through"];
pub(super) fn operation(token: &str) -> Option<&'static str> {
    SVG.iter()
        .find(|(name, alias)| *name == token || *alias == token || name.strip_prefix("svg:") == Some(token))
        .map(|(name, _)| *name)
        .or_else(|| GIMP.iter().copied().find(|name| *name == token))
}
pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    if GIMP.contains(&op) {
        graph_schema::field(op, name)
    } else if SVG.iter().any(|(known, _)| *known == op) && name == "srgb" {
        Some(Kind::Boolean)
    } else {
        None
    }
}
#[derive(Debug)]
pub(super) struct Lowered {
    pub steps: Vec<NativeStage>,
    pub output: usize,
    pub notes: Vec<String>,
}
fn push(steps: &mut Vec<NativeStage>, next: usize, step: NativeStage) -> Result<usize, MappingFailure> {
    let output = next.checked_add(steps.len()).filter(|n| *n <= 64).ok_or("native mask construction exceeds64 global stages")?;
    if steps.len() >= 8 {
        return Err("native mask policy exceeds8 finite construction steps".into());
    }
    steps.push(step);
    Ok(output)
}
fn alpha(input: usize, value: f32) -> NativeStage {
    NativeStage::Apply {
        input,
        mapping: Box::new(NativeMapping { filters: vec![native_filter("filter.other.setAlpha", json!({"value":value}))], ..Default::default() }),
    }
}
fn mask(input: usize, mask: usize, invert: bool) -> NativeStage {
    NativeStage::Mask { input, mask, component: NativeMaskComponent::Alpha, invert, scale: 1. }
}
fn join(below: usize, above: usize, blend: BlendMode) -> NativeStage {
    NativeStage::Join { below, above, blend, opacity: 1. }
}
pub(super) fn lower(e: &StoredEffect<'_>, c: &MappingContext<'_>, main: usize, aux: usize, next: usize) -> Result<Lowered, MappingFailure> {
    let op = operation(e.operation).ok_or("unowned native mask policy")?;
    if main >= next || aux >= next || next == 0 || next > 64 {
        return Err(invalid("native mask policy requires actual earlier main/auxiliary inputs"));
    }
    generation_bytes(c, 32_768)?;
    if c.remaining_metadata_bytes < 32_768 {
        return Err(MappingFailure::resource("native mask policy exceeds remaining metadata allowance"));
    }
    for argument in &e.arguments {
        e.argument(argument.name).map_err(invalid)?;
        match field(op, argument.name) {
            Some(Kind::Boolean) => {
                e.boolean(argument.name).map_err(invalid)?;
            }
            Some(Kind::Choice(choices)) => {
                let value = e.enum_value(argument.name).map_err(invalid)?.ok_or_else(|| invalid("missing source mode value"))?;
                if !choices.iter().any(|(_, v)| *v == value) {
                    return Err(invalid("unknown declared source compositing enum"));
                }
            }
            Some(Kind::Number { min, max }) => {
                let value = e.number(argument.name).map_err(invalid)?.ok_or_else(|| invalid("missing source numeric value"))?;
                if !value.is_finite() || !(min..=max).contains(&value) {
                    return Err(invalid("source compositing value outside declared domain"));
                }
            }
            _ => return Err(format!("{op} property {} has no declared native mask policy", argument.name).into()),
        }
    }
    let opacity = if GIMP.contains(&op) { e.number("opacity").map_err(invalid)?.unwrap_or(1.) } else { 1. };
    if let Some(selector) = e.enum_value("layer-mode").map_err(invalid)? {
        let expected = match op {
            "gimp:split" => 60,
            "gimp:pass-through" => 61,
            _ => 28,
        };
        if selector != 28 && selector != expected {
            return Err("specialized source operation conflicts with inherited layer-mode selector; original data retained".into());
        }
    }
    let mut steps = Vec::new();
    steps.try_reserve_exact(8).map_err(|_| MappingFailure::resource("native mask construction reservation unavailable"))?;
    let source = if opacity < 1. {
        let values: Vec<f64> = (0..256).map(|n| f64::from(n) / 255. * opacity).collect();
        push(
            &mut steps,
            next,
            NativeStage::Apply {
                input: aux,
                mapping: Box::new(NativeMapping { filters: vec![native_filter("filter.other.alphaCurve", json!({"values":values}))], ..Default::default() }),
            },
        )?
    } else {
        aux
    };
    let output = match op {
        "svg:clear" => push(&mut steps, next, alpha(main, 0.))?,
        "svg:src-in" => push(&mut steps, next, mask(source, main, false))?,
        "svg:dst-in" => push(&mut steps, next, mask(main, source, false))?,
        "svg:src-out" => push(&mut steps, next, mask(source, main, true))?,
        "svg:dst-out" => push(&mut steps, next, mask(main, source, true))?,
        "svg:src-atop" | "svg:dst-atop" | "svg:xor" => {
            let (below, above) = if op == "svg:dst-atop" {
                (mask(source, main, true), mask(main, source, false))
            } else {
                (mask(main, source, true), mask(source, main, op == "svg:xor"))
            };
            let below = push(&mut steps, next, below)?;
            let above = push(&mut steps, next, above)?;
            push(&mut steps, next, join(below, above, BlendMode::Normal))?
        }
        "gimp:anti-erase" => {
            let union = push(&mut steps, next, join(main, source, BlendMode::Normal))?;
            let opaque = push(&mut steps, next, alpha(main, 1.))?;
            push(&mut steps, next, mask(opaque, union, false))?
        }
        "gimp:split" => {
            let lower = push(&mut steps, next, NativeStage::Channel { input: main, component: NativeMaskComponent::Alpha })?;
            let upper = push(&mut steps, next, NativeStage::Channel { input: source, component: NativeMaskComponent::Alpha })?;
            let difference = push(&mut steps, next, join(lower, upper, BlendMode::Subtract))?;
            let opaque = push(&mut steps, next, alpha(main, 1.))?;
            push(&mut steps, next, NativeStage::Mask { input: opaque, mask: difference, component: NativeMaskComponent::Composite, invert: false, scale: 1. })?
        }
        "gimp:pass-through" => push(&mut steps, next, join(main, source, BlendMode::Normal))?,
        _ => return Err("unowned native mask construction".into()),
    };
    c.interrupt.check().map_err(|_| MappingFailure::resource("native mask construction cancelled"))?;
    Ok(Lowered {
        steps,
        output,
        notes: vec![format!(
            "{op} becomes ordinary native editable contents, static pixel masks and native layer compositing. Masks capture actual prepared native input alpha once; they are editable native masks, not live source dependencies. In/Out preserve masked branch intent; Atop/Xor recombine masked branches with native Normal, so fractional overlap follows PhotoCraft alpha rules rather than exact source Porter–Duff sums. AntiErase uses native Normal alpha union with native SetAlpha target colors; hidden transparent color recovery follows native storage. Split uses native alpha-channel rasters and ordinary native Subtract, then a native mask. PassThrough uses an isolated native Normal join of the actual images; outside-group interactions cannot be recovered from flattened branches. Source opacity attenuates the actual auxiliary once. Source srgb/blend/composite-space selectors remain history while native document ICC, depth, alpha and compositing rules apply."
        )],
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue, NativeStructure};
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent, LayerMask, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::{Interrupt, Surface};
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            resolution_dpi: 72.,
            linear: false,
            owner_bounds: Rect::new(0, 0, 2, 1),
            native_filter_bounds: Rect::new(0, 0, 2, 1),
            target_profile: None,
            remaining_generation_bytes: usize::MAX,
            remaining_metadata_bytes: usize::MAX,
            interrupt: Interrupt::NONE,
        }
    }
    #[test]
    fn exact_names_controls_actual_inputs_and_global_references_are_bounded() {
        for (name, alias) in SVG {
            assert_eq!(operation(name), Some(*name));
            assert_eq!(operation(alias), Some(*name));
        }
        assert_eq!(operation("gimp:future"), None);
        for (name, _) in SVG {
            let value = lower(&StoredEffect::test(name, vec![]), &context(), 1, 2, 3).unwrap();
            assert!(!value.steps.is_empty());
            assert!(value.output >= 3 && value.output < 11);
        }
        assert!(lower(&StoredEffect::test("svg:src-in", vec![]), &context(), 3, 2, 3).is_err());
        let atop = lower(&StoredEffect::test("svg:dst-atop", vec![]), &context(), 1, 2, 3).unwrap();
        assert!(matches!(atop.steps[0], NativeStage::Mask { input: 2, mask: 1, invert: true, .. }));
        assert!(matches!(atop.steps[1], NativeStage::Mask { input: 1, mask: 2, invert: false, .. }));
        assert!(matches!(atop.steps[2], NativeStage::Join { below: 3, above: 4, .. }));
        let bad = StoredEffect::test("gimp:split", vec![Argument { name: "opacity", value: ArgumentValue::Literal(1.1) }]);
        assert!(lower(&bad, &context(), 0, 1, 2).is_err());
        let bad = StoredEffect::test("svg:xor", vec![Argument { name: "future", value: ArgumentValue::Int(1) }]);
        assert!(lower(&bad, &context(), 0, 1, 2).is_err());
        assert!(lower(&StoredEffect::test("gimp:split", vec![]), &context(), 0, 1, 63).is_err());
        let mut c = context();
        c.remaining_metadata_bytes = 0;
        assert!(lower(&StoredEffect::test("svg:src-in", vec![]), &c, 0, 1, 2).is_err());
    }
    #[test]
    fn opacity_attenuates_the_actual_auxiliary_once_before_static_masks() {
        let source = StoredEffect::test("gimp:anti-erase", vec![Argument { name: "opacity", value: ArgumentValue::Literal(0.5) }]);
        let result = lower(&source, &context(), 4, 7, 8).unwrap();
        let NativeStage::Apply { input: 7, mapping } = &result.steps[0] else { panic!("actual auxiliary attenuation") };
        assert_eq!(mapping.filters[0].command, "filter.other.alphaCurve");
        assert_eq!(mapping.filters[0].params["values"][255], 0.5);
        assert!(matches!(result.steps[1], NativeStage::Join { below: 4, above: 8, opacity: 1., .. }));
        assert!(matches!(result.steps[3], NativeStage::Mask { input: 10, mask: 9, component: NativeMaskComponent::Alpha, invert: false, scale: 1. }));
    }
    #[test]
    fn prepared_native_masks_match_independent_native_counterparts_and_reopen() {
        use crate::xcf::native_effects::{NativeEnvelope, NativeOwner};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (da, sa) in [(0., 0.), (0., 1.), (1., 0.), (1., 1.), (0.5, 0.25)] {
                for op in ["svg:src-in", "svg:dst-out", "svg:xor", "gimp:split", "gimp:anti-erase"] {
                    let mut doc = Document::new("Native source", Size::new(2, 1), ColorMode::Rgb, depth);
                    let mut d = Surface::new(doc.pixel_format());
                    d.fill_rect(doc.bounds(), &[1., 0., 0., da]);
                    let mut s = Surface::new(doc.pixel_format());
                    s.fill_rect(doc.bounds(), &[0., 0., 1., sa]);
                    let original = Layer::new("Actual main", LayerContent::Raster(d.clone()));
                    let mut budget = crate::xcf::MAX_PIXEL_BYTES;
                    let mut owner = NativeOwner::new(&doc, &original, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
                    let mut steps = vec![NativeStage::Apply {
                        input: 0,
                        mapping: Box::new(NativeMapping {
                            structure: Some(NativeStructure::Replace {
                                layer: Box::new(Layer::new("Actual auxiliary", LayerContent::Raster(s.clone()))),
                                patterns: vec![],
                            }),
                            ..Default::default()
                        }),
                    }];
                    let lowered = lower(&StoredEffect::test(op, vec![]), &context(), 0, 1, 2).unwrap();
                    steps.extend(lowered.steps);
                    owner
                        .push(
                            NativeMapping { structure: Some(NativeStructure::Stages { steps, output: lowered.output }), ..Default::default() },
                            NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                            &mut budget,
                            &Interrupt::NONE,
                        )
                        .unwrap();
                    let actual = owner.snapshot(&mut budget, &Interrupt::NONE).unwrap();
                    // Independent ordinary native document with explicit pixel masks.
                    let a = d.pixel(0, 0)[3];
                    let b = s.pixel(0, 0)[3];
                    let masked = |name: &str, pixels: Surface, value: f32| {
                        let mut layer = Layer::new(name, LayerContent::Raster(pixels));
                        layer.mask = Some(LayerMask {
                            surface: Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[value]),
                            ..LayerMask::reveal_all()
                        });
                        layer
                    };
                    let mut expected = Document::new("Independent native masks", Size::new(2, 1), ColorMode::Rgb, depth);
                    match op {
                        "svg:src-in" => expected.layers.push(masked("Source masked by destination", s.clone(), a)),
                        "svg:dst-out" => expected.layers.push(masked("Destination inverse-source mask", d.clone(), 1. - b)),
                        "svg:xor" => {
                            expected.layers.push(masked("Destination outside source", d.clone(), 1. - b));
                            expected.layers.push(masked("Source outside destination", s.clone(), 1. - a));
                        }
                        _ => {
                            let coverage = if op == "gimp:split" { (a - b).max(0.) } else { 1. - (1. - a) * (1. - b) };
                            // Independently author a normal native Smart Object,
                            // Set Alpha filter and static mask. Preparing a fully
                            // transparent embedded image may discard hidden RGB;
                            // compare the real native policy, not invented red.
                            let mut source = Document::new("Independent native main", Size::new(2, 1), ColorMode::Rgb, depth);
                            source.layers.push(Layer::new("Original red samples", LayerContent::Raster(d.clone())));
                            let bytes = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
                            let mut smart = SmartObject::new(
                                SmartSource::Embedded { file_name: "independent-main.pcraft".into(), bytes: std::sync::Arc::new(bytes) },
                                photocraft_geom::Affine::IDENTITY,
                                None,
                            );
                            smart.smart_filters.push(SmartFilter {
                                command: "filter.other.setAlpha".into(),
                                params: json!({"value":1.}),
                                blend: BlendMode::Normal,
                                opacity: 1.,
                                visible: true,
                            });
                            let mut target = Layer::new("Native Smart target with scalar mask", LayerContent::Smart(smart));
                            target.mask = Some(LayerMask {
                                surface: Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage]),
                                ..LayerMask::reveal_all()
                            });
                            expected.layers.push(target);
                        }
                    }
                    let expected_bytes = photocraft_format::save_to_bytes(&expected, &Default::default()).unwrap();
                    let expected_pixels = crate::xcf::native_snapshot::capture(&expected_bytes, &expected, &mut budget, &Interrupt::NONE).unwrap();
                    if op == "gimp:anti-erase" && sa == 1. {
                        let pixel = expected_pixels.pixel(0, 0);
                        assert!(pixel[3] > 0.99);
                        if da > 0. {
                            assert!(pixel[0] > 0.99);
                        } else {
                            assert_eq!(&pixel[..3], &[0., 0., 0.]);
                        }
                    }
                    for x in 0..2 {
                        let expected = expected_pixels.pixel(x, 0);
                        let pixel = actual.pixel(x, 0);
                        for (value, expected) in pixel.iter().zip(&expected) {
                            assert!((*value - expected).abs() < 0.015, "{op} depth{depth:?} alpha({da},{sa}) actual{pixel:?} expected{expected:?}");
                        }
                    }
                    doc.layers.push(Layer::new("Native result", owner.content()));
                    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                    drop(owner);
                    drop(original);
                    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                    let after = crate::xcf::native_snapshot::capture(&bytes, &reopened, &mut budget, &Interrupt::NONE).unwrap();
                    for x in 0..2 {
                        assert_eq!(actual.pixel(x, 0), after.pixel(x, 0));
                    }
                }
            }
        }
    }
}
