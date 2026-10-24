//! Declared stock unary arithmetic translated once into native tonal controls.
//! GEGL0.4.72 generated registration descriptions define the scalar expressions;
//! no source processing implementation or auxiliary evaluator is used here.
use super::graph_schema::Kind;
use super::residual_spatial::{generation_bytes, invalid};
use super::{MappingContext, MappingFailure, MappingResult, NativeMapping, StoredEffect, native_filter};
use serde_json::json;

const NAMES: &[&str] = &["gegl:add", "gegl:subtract", "gegl:multiply", "gegl:divide", "gegl:gamma"];
const MAX_PASSES: usize = 32;
pub(super) fn operation(token: &str) -> Option<&'static str> {
    NAMES.iter().copied().find(|name| *name == token || name.strip_prefix("gegl:") == Some(token))
}
pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    (NAMES.contains(&op) && name == "value").then_some(Kind::Number { min: -f64::MAX, max: f64::MAX })
}
fn mixer(gain: f64, offset: f64) -> photocraft_doc::SmartFilter {
    native_filter(
        "image.adjustments.channelMixer",
        json!({"red":[gain*100.,0.,0.,offset*100.],"green":[0.,gain*100.,0.,offset*100.],"blue":[0.,0.,gain*100.,offset*100.],"monochrome":false}),
    )
}
fn affine(c: &MappingContext<'_>, gain: f64, offset: f64) -> photocraft_doc::SmartFilter {
    let mut kernel = [0.; 25];
    if let Some(center) = kernel.get_mut(12) {
        *center = gain;
    }
    let mut channels = vec![true; c.mode.color_channels() + 1];
    if let Some(alpha) = channels.last_mut() {
        *alpha = false;
    }
    native_filter("filter.other.convolution", json!({"kernel":kernel,"scale":1.,"offset":offset,"channels":channels,"alphaWeighted":false,"edge":"repeat"}))
}
fn map_known(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> Result<NativeMapping, MappingFailure> {
    generation_bytes(c, 65_536)?;
    if c.remaining_metadata_bytes < 65_536 {
        return Err(MappingFailure::resource("bounded native arithmetic parameters exceed remaining metadata allowance"));
    }
    for arg in &e.arguments {
        e.argument(arg.name).map_err(invalid)?;
        if arg.name != "value" {
            return Err(
                format!("{} scalar adapter does not accept {}; actual auxiliary input requires its own binary native adapter", e.operation, arg.name).into()
            );
        }
    }
    let default = if matches!(e.operation, "gegl:add" | "gegl:subtract") { 0. } else { 1. };
    let value = e.number("value").map_err(invalid)?.unwrap_or(default);
    if !value.is_finite() {
        return Err(invalid("stock arithmetic value must be finite"));
    }
    let mut filters = Vec::new();
    let mut notes = Vec::new();
    filters.try_reserve_exact(MAX_PASSES).map_err(|_| MappingFailure::resource("native arithmetic filter reservation unavailable"))?;
    match e.operation {
        "gegl:add" | "gegl:subtract" => {
            let mut remaining = if e.operation == "gegl:subtract" { -value } else { value };
            if remaining.abs() > 64. || (remaining != 0. && remaining as f32 == 0.) {
                filters.push(affine(c, 1., remaining));
                notes.push("Scalar offset outside bounded native F32 Channel Mixer constants becomes one ordinary center-only native F64 Convolution gain1/bias. Color channels are selected and native owner alpha is excluded; native per-depth precision/clipping applies without discarding or silently rounding the source control at import.".into());
            } else {
                for _ in 0..MAX_PASSES {
                    if remaining == 0. {
                        break;
                    }
                    let step = remaining.clamp(-2., 2.);
                    filters.push(mixer(1., step));
                    remaining -= step;
                }
            }
        }
        "gegl:multiply" | "gegl:divide" => {
            let mut remaining = if e.operation == "gegl:divide" { if value == 0. { 0. } else { value.recip() } } else { value };
            if !remaining.is_finite() {
                return Err("scalar reciprocal exceeds finite native F64 Convolution coefficients; original source value retained".into());
            }
            if remaining.abs() > 4_294_967_296. || (remaining != 0. && remaining as f32 == 0.) {
                filters.push(affine(c, remaining, 0.));
                notes.push("Scalar gain outside bounded native F32 Channel Mixer factors becomes one ordinary center-only native F64 Convolution coefficient with zero bias. Native color-channel arithmetic, depth clipping/precision and unchanged owner alpha apply; no32-pass or F32-constant cutoff is treated as intrinsic.".into());
            } else {
                for _ in 0..MAX_PASSES {
                    let step = remaining.clamp(-2., 2.);
                    filters.push(mixer(step, 0.));
                    if step == remaining {
                        break;
                    }
                    remaining /= step;
                }
            }
        }
        "gegl:gamma" => {
            let gamma = value.recip();
            if value > 0. && gamma.is_finite() && (0.01..=9.99).contains(&gamma) {
                filters.push(native_filter("image.adjustments.exposure", json!({"exposure":0.,"offset":0.,"gamma":gamma})));
            } else {
                if !matches!(c.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
                    return Err("normalized scalar gamma Curves requires native RGB/Gray composite channel semantics".into());
                }
                // One-time authoring of ordinary native curve data from the
                // publicly declared scalar expression, never a replay callback.
                let mut points = Vec::new();
                points.try_reserve_exact(256).map_err(|_| MappingFailure::resource("native gamma curve reservation unavailable"))?;
                for sample in 0..256 {
                    c.interrupt.check().map_err(|_| MappingFailure::resource("native gamma curve conversion cancelled"))?;
                    let x = f64::from(sample) / 255.;
                    // Negative exponents exceed white throughout(0,1]. Their
                    // zero-input pole uses explicit finite native white policy.
                    let y = if value <= 0. { 1. } else { x.powf(value) };
                    points.push(y.clamp(0., 1.) * 255.);
                }
                filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":points})));
                notes.push(format!("Scalar exponent {value} is outside ordinary native Exposure gamma controls and becomes editable native256-sample pencil Curves. Positive powers author a normalized0..1 transfer; native endpoint extrapolation in32-bit RGB, normalized limits in other modes/depths, piecewise-linear interpolation and depth quantization apply. Signed/HDR power behavior is explicitly replaced by native document tone rules. Zero exponent uses native constant white; negative exponents clip the positive normalized domain to white and their zero-input pole uses a finite native white endpoint, with no infinite sample computed. Original source scalar remains historical data."));
            }
        }
        _ => return Err("unowned stock arithmetic".into()),
    }
    c.interrupt.check().map_err(|_| MappingFailure::resource("native arithmetic conversion cancelled"))?;
    let working_rgb = c.mode == photocraft_color::ColorMode::Grayscale && filters.iter().any(|filter| filter.command == "image.adjustments.channelMixer");
    notes.push("Unary stock scalar arithmetic becomes bounded ordinary native Channel Mixer, Exposure or pencil Curves controls. Native document encoding, signed/HDR policy, alpha preservation, F32 parameter precision and integer intermediate clipping/quantization remain authoritative. Gray Channel Mixer uses ordinary native RGB working contents and native final Gray conversion. Only the scalar input form is translated; no absent auxiliary input is invented.".into());
    Ok(NativeMapping { filters, working_rgb, notes, ..Default::default() })
}
pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> MappingResult {
    operation(e.operation)?;
    Some(map_known(e, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcf::effects::{Argument, ArgumentValue};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
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
    fn source(op: &'static str, v: f64) -> StoredEffect<'static> {
        StoredEffect::test(op, vec![Argument { name: "value", value: ArgumentValue::Literal(v) }])
    }
    #[test]
    fn defaults_domains_and_exact_native_parameters_are_independent() {
        assert_eq!(operation("multiply"), Some("gegl:multiply"));
        assert_eq!(operation("math:arbitrary"), None);
        assert!(map(&StoredEffect::test("gegl:add", vec![]), &context()).unwrap().unwrap().filters.is_empty());
        let add = map(&source("gegl:add", 2.5), &context()).unwrap().unwrap();
        assert_eq!(add.filters.len(), 2);
        assert_eq!(add.filters[0].params["red"], json!([100., 0., 0., 200.]));
        assert_eq!(add.filters[1].params["red"], json!([100., 0., 0., 50.]));
        let mul = map(&source("gegl:multiply", -6.), &context()).unwrap().unwrap();
        assert_eq!(mul.filters.len(), 3);
        assert_eq!(mul.filters[0].params["red"], json!([-200., 0., 0., 0.]));
        assert_eq!(mul.filters[2].params["red"], json!([150., 0., 0., 0.]));
        assert_eq!(map(&source("gegl:gamma", 2.), &context()).unwrap().unwrap().filters[0].params, json!({"exposure":0.,"offset":0.,"gamma":0.5}));
        assert_eq!(map(&source("gegl:gamma", 0.), &context()).unwrap().unwrap().filters[0].command, "image.adjustments.curves");
        assert_eq!(map(&source("gegl:add", 65.), &context()).unwrap().unwrap().filters[0].command, "filter.other.convolution");
        assert_eq!(map(&source("gegl:multiply", f64::MAX), &context()).unwrap().unwrap().filters[0].command, "filter.other.convolution");
        let divide = map(&source("gegl:divide", 0.), &context()).unwrap().unwrap();
        assert_eq!(divide.filters[0].params["red"], json!([0., 0., 0., 0.]));
        let mut c = context();
        c.remaining_metadata_bytes = 65_535;
        assert!(map(&source("gegl:multiply", 1.), &c).unwrap().is_err());
        let cancel = || true;
        let progress = |_| {};
        let c = MappingContext { interrupt: Interrupt::new(&cancel, &progress), ..context() };
        assert!(map(&source("gegl:multiply", 1.), &c).unwrap().is_err());
        let actual_aux = StoredEffect::test("gegl:add", vec![Argument { name: "aux", value: ArgumentValue::String("not a scalar") }]);
        assert!(map(&actual_aux, &context()).unwrap().unwrap_err().message.contains("binary native adapter"));
        let mut gray = context();
        gray.mode = ColorMode::Grayscale;
        assert!(map(&source("gegl:multiply", 0.5), &gray).unwrap().unwrap().working_rgb);
        assert!(!map(&source("gegl:gamma", 2.), &gray).unwrap().unwrap().working_rgb);
    }
    #[test]
    fn native_channel_mixer_counterpart_preserves_alpha_and_native_reopen_each_depth() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mapped = map(&source("gegl:multiply", 0.5), &context()).unwrap().unwrap();
            let p = &mapped.filters[0].params;
            let mut rows = [[0.; 4]; 3];
            for (row, name) in rows.iter_mut().zip(["red", "green", "blue"]) {
                for (slot, value) in row.iter_mut().zip(p[name].as_array().unwrap()) {
                    *slot = value.as_f64().unwrap() as f32 / 100.;
                }
            }
            let actual = Adjustment::ChannelMixer { matrix: rows, monochrome: false };
            let expected = Adjustment::ChannelMixer { matrix: [[0.5, 0., 0., 0.], [0., 0.5, 0., 0.], [0., 0., 0.5, 0.]], monochrome: false };
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            surface.fill_rect(Rect::new(0, 0, 2, 1), &[0.6, 0.4, 0.2, 0.5]);
            let build = |adjustment| {
                let mut doc = Document::new("Native arithmetic", Size::new(2, 1), ColorMode::Rgb, depth);
                doc.layers.push(Layer::new("Original", LayerContent::Raster(surface.clone())));
                doc.layers.push(Layer::new("Editable adjustment", LayerContent::Adjustment(adjustment)));
                doc
            };
            let doc = build(actual);
            let expected = build(expected);
            let before = photocraft_compose::render(&doc, doc.bounds()).px;
            assert_eq!(before, photocraft_compose::render(&expected, expected.bounds()).px);
            assert!((before[0][3] - 0.5).abs() < 0.01);
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(before, photocraft_compose::render(&reopened, reopened.bounds()).px);
        }
    }
    #[test]
    fn gamma_outside_exposure_uses_independent_native_root_curve_with_bounded_endpoints() {
        use photocraft_doc::adjust::{CurvePoint, ToneSpace};
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let mut c = context();
            c.mode = mode;
            for exponent in [0.0625, 0., -2., f64::MAX] {
                let mapped = map(&source("gegl:gamma", exponent), &c).unwrap().unwrap();
                assert!(!mapped.working_rgb);
                let filter = &mapped.filters[0];
                assert_eq!(filter.command, "image.adjustments.curves");
                assert_eq!(filter.params["pencil"], true);
                assert!(mapped.notes.iter().any(|note| note.contains("Signed/HDR") && note.contains("zero-input pole")));
                let outputs = filter.params["points"].as_array().unwrap();
                assert_eq!(outputs.len(), 256);
                assert!(outputs.iter().all(|value| value.as_f64().is_some_and(|v| v.is_finite() && (0.0..=255.0).contains(&v))));
                let points: Vec<CurvePoint> =
                    outputs.iter().enumerate().map(|(i, value)| CurvePoint { input: i as f32 / 255., output: value.as_f64().unwrap() as f32 / 255. }).collect();
                if exponent == 0.0625 {
                    // Independently specified sixteen-th-root native curve,
                    // constructed with four square roots rather than importer pow.
                    let expected: Vec<CurvePoint> = (0..256)
                        .map(|i| {
                            let x = i as f32 / 255.;
                            CurvePoint { input: x, output: x.sqrt().sqrt().sqrt().sqrt() }
                        })
                        .collect();
                    for (a, b) in points.iter().zip(&expected) {
                        assert!((a.output - b.output).abs() < 1e-6);
                    }
                    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                        let curve = |master| Adjustment::Curves {
                            master,
                            per_channel: std::array::from_fn(|_| vec![CurvePoint { input: 0., output: 0. }, CurvePoint { input: 1., output: 1. }]),
                            space: ToneSpace::Rgb,
                            black: vec![],
                            pencil: true,
                        };
                        let build = |adjustment| {
                            let mut doc = Document::new("Native root gamma", Size::new(2, 1), mode, depth);
                            let mut pixels = Surface::new(doc.pixel_format());
                            let normal = if mode == ColorMode::Rgb { vec![0.25, 0.5, 0.75, 0.6] } else { vec![0.25, 0.6] };
                            pixels.write_pixel(0, 0, &normal);
                            let hdr = if mode == ColorMode::Rgb { vec![-0.25, 2., 0.5, 0.6] } else { vec![-0.25, 0.6] };
                            pixels.write_pixel(1, 0, &hdr);
                            doc.layers.push(Layer::new("Native original", LayerContent::Raster(pixels)));
                            doc.layers.push(Layer::new("Editable pencil gamma", LayerContent::Adjustment(adjustment)));
                            doc
                        };
                        let actual = build(curve(points.clone()));
                        let independent = build(curve(expected.clone()));
                        let rendered = photocraft_compose::render(&actual, actual.bounds()).px;
                        let expected = photocraft_compose::render(&independent, independent.bounds()).px;
                        for (actual, expected) in rendered.iter().zip(&expected) {
                            for (a, b) in actual.iter().zip(expected) {
                                assert!((a - b).abs() < 0.00002, "native gamma {mode:?}/{depth:?}: {a}!={b}");
                            }
                        }
                        assert!(rendered[0][0] > 0.8 && (rendered[0][3] - 0.6).abs() < 0.01);
                        let saved = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
                        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
                        assert_eq!(rendered, photocraft_compose::render(&reopened, reopened.bounds()).px);
                    }
                } else if exponent <= 0. {
                    assert!(points.iter().all(|p| p.output == 1.));
                }
            }
            let mut low = context();
            low.mode = mode;
            low.remaining_generation_bytes = 100;
            assert!(map(&source("gegl:gamma", 0.0625), &low).unwrap().is_err());
            let cancel = || true;
            let progress = |_| {};
            let canceled = MappingContext { interrupt: Interrupt::new(&cancel, &progress), ..context() };
            assert!(map(&source("gegl:gamma", 0.0625), &canceled).unwrap().is_err());
        }
    }
    #[test]
    fn affine_extremes_use_native_f64_convolution_with_alpha_excluded_and_native_reopen() {
        use photocraft_algo::convolution::{self, Edge, Params};
        use photocraft_doc::{SmartObject, SmartSource};
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let mut c = context();
            c.mode = mode;
            for (op, value, gain, bias) in [
                ("gegl:add", 65., 1., 65.),
                ("gegl:subtract", 65., 1., -65.),
                ("gegl:add", 1e-60, 1., 1e-60),
                ("gegl:multiply", 1e12, 1e12, 0.),
                ("gegl:multiply", -1e12, -1e12, 0.),
                ("gegl:multiply", 1e-60, 1e-60, 0.),
            ] {
                let mapped = map(&source(op, value), &c).unwrap().unwrap();
                let filter = &mapped.filters[0];
                assert_eq!(mapped.filters.len(), 1);
                assert_eq!(filter.command, "filter.other.convolution");
                assert!(!mapped.working_rgb);
                assert!(mapped.notes.iter().any(|n| n.contains("F64 Convolution")));
                let p = &filter.params;
                let parsed = Params {
                    kernel: p["kernel"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect(),
                    scale: p["scale"].as_f64().unwrap(),
                    offset: p["offset"].as_f64().unwrap(),
                    channels: p["channels"].as_array().unwrap().iter().map(|v| v.as_bool().unwrap()).collect(),
                    alpha_weighted: p["alphaWeighted"].as_bool().unwrap(),
                    edge: Edge::Repeat,
                };
                let mut kernel = vec![0.; 25];
                kernel[12] = gain;
                let mut channels = vec![true; mode.color_channels()];
                channels.push(false);
                let expected = Params { kernel, scale: 1., offset: bias, channels, alpha_weighted: false, edge: Edge::Repeat };
                assert_eq!(parsed, expected);
                for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                    let frame = Rect::new(0, 0, 3, 1);
                    let mut pixels = Surface::new(PixelFormat::new(mode, depth, true));
                    for x in 0..3 {
                        let v = match x {
                            0 => -0.25,
                            1 => 0.,
                            _ => 2.,
                        };
                        let samples = if mode == ColorMode::Rgb { vec![v, v * 0.5, 1. - v, 0.37] } else { vec![v, 0.37] };
                        pixels.write_pixel(x, 0, &samples);
                    }
                    let actual = convolution::apply_with(&pixels, &parsed, frame, frame, None, &Interrupt::NONE).unwrap();
                    let independent = convolution::apply_with(&pixels, &expected, frame, frame, None, &Interrupt::NONE).unwrap();
                    assert_eq!(actual.read_region(frame), independent.read_region(frame));
                    for x in 0..3 {
                        assert_eq!(actual.pixel(x, 0).last(), pixels.pixel(x, 0).last());
                        assert!(actual.pixel(x, 0).iter().all(|v| v.is_finite()));
                    }
                    let mut contents = Document::new("Independent native convolution input", Size::new(3, 1), mode, depth);
                    contents.layers.push(Layer::new("Original signed HDR samples", LayerContent::Raster(pixels)));
                    let embedded = photocraft_format::save_to_bytes(&contents, &Default::default()).unwrap();
                    let mut smart = SmartObject::new(
                        SmartSource::Embedded { file_name: "native-input.pcraft".into(), bytes: std::sync::Arc::new(embedded) },
                        photocraft_geom::Affine::IDENTITY,
                        None,
                    );
                    smart.smart_filters.push(filter.clone());
                    let mut doc = Document::new("Native affine scalar", Size::new(3, 1), mode, depth);
                    doc.layers.push(Layer::new("Editable native convolution", LayerContent::Smart(smart)));
                    let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
                    let LayerContent::Smart(stored) = &reopened.layers[0].content else { panic!("native Smart Filter") };
                    assert_eq!(stored.smart_filters[0].params, filter.params);
                    let mut budget = crate::xcf::MAX_PIXEL_BYTES;
                    let first = crate::xcf::native_snapshot::capture(&saved, &doc, &mut budget, &Interrupt::NONE).unwrap();
                    let second = crate::xcf::native_snapshot::capture(&saved, &reopened, &mut budget, &Interrupt::NONE).unwrap();
                    assert_eq!(first.read_region(frame), second.read_region(frame));
                }
            }
            let mut limited = context();
            limited.mode = mode;
            limited.remaining_metadata_bytes = 0;
            assert!(map(&source("gegl:add", 65.), &limited).unwrap().is_err());
            let cancel = || true;
            let progress = |_| {};
            let canceled = MappingContext { mode, interrupt: Interrupt::new(&cancel, &progress), ..context() };
            assert!(map(&source("gegl:multiply", 1e12), &canceled).unwrap().is_err());
        }
        assert!(map(&source("gegl:divide", f64::MIN_POSITIVE / 1024.), &context()).unwrap().is_err());
    }
}
