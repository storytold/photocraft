//! B06-11: native radial distortions, Wind, and initial component scaling.
//! Alpha replacement/threshold requires the actual owner's coverage, never opacity.

use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeMapping, StoredEffect, native_filter};
use photocraft_color::ColorMode;
use photocraft_doc::import_conversion::ConversionReason;
use serde_json::json;

type Result<T> = std::result::Result<T, MappingFailure>;
fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: ConversionReason::InvalidOptionalData, message: message.into() }
}
fn number(e: &StoredEffect<'_>, key: &str, default: f64, low: f64, high: f64) -> Result<f64> {
    let v = e.number(key).map_err(invalid)?.unwrap_or(default);
    if !v.is_finite() || !(low..=high).contains(&v) {
        return Err(invalid(format!("{} {key} is outside its finite source domain", e.operation)));
    }
    Ok(v)
}
fn integer(e: &StoredEffect<'_>, key: &str, default: u32, low: u32, high: u32) -> Result<u32> {
    let v = number(e, key, f64::from(default), f64::from(low), f64::from(high))?;
    if v.fract() != 0.0 {
        return Err(invalid(format!("{key} must be integral")));
    }
    Ok(v as u32)
}
fn choice(e: &StoredEffect<'_>, key: &str, default: usize, names: &[&str]) -> Result<usize> {
    match e.argument(key).map_err(invalid)? {
        None => Ok(default),
        Some(ArgumentValue::Enum(v)) => usize::try_from(*v).ok().filter(|v| *v < names.len()).ok_or_else(|| invalid(format!("unknown {key} enum"))),
        Some(ArgumentValue::String(v)) => names.iter().position(|n| n == v).ok_or_else(|| invalid(format!("unknown {key} nick"))),
        _ => Err(invalid(format!("{key} must be a stored enum/nick"))),
    }
}
fn allowed(e: &StoredEffect<'_>, names: &[&str]) -> Result<()> {
    for arg in &e.arguments {
        if !names.contains(&arg.name) {
            return Err(format!("unmapped {} argument {}", e.operation, arg.name).into());
        }
        e.argument(arg.name).map_err(invalid)?;
    }
    Ok(())
}

fn waves(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["x", "y", "amplitude", "period", "phi", "aspect", "sampler-type", "clamp"])?;
    let x = number(e, "x", 0.5, -f64::MAX, f64::MAX)?;
    let y = number(e, "y", 0.5, -f64::MAX, f64::MAX)?;
    let amplitude = number(e, "amplitude", 25.0, 0.0, 1000.0)?;
    let period = number(e, "period", 100.0, 0.1, 1000.0)?;
    let phase = number(e, "phi", 0.0, -1.0, 1.0)?;
    let aspect = number(e, "aspect", 1.0, 0.1, 10.0)?;
    choice(e, "sampler-type", 2, &["nearest", "linear", "cubic", "nohalo", "lohalo"])?;
    e.boolean("clamp").map_err(invalid)?;
    if amplitude == 0.0 {
        return Ok(NativeMapping { notes: vec!["Source wave amplitude is zero; preserve native pixels".into()], ..Default::default() });
    }
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err(invalid("Wave native owner bounds are empty"));
    }
    let clamp = e.boolean("clamp").map_err(invalid)?.unwrap_or(false);
    let width = f64::from(bounds.width());
    let height = f64::from(bounds.height());
    let center = (x * width, y * height);
    if !center.0.is_finite() || !center.1.is_finite() {
        return Err(invalid("Wave relative center overflows native coordinates"));
    }
    // Original standard radial sine field. Area-preserving ellipse axes and the
    // gradient's unit direction are native approximation policies, not source math.
    let sx = aspect.sqrt();
    let sy = 1.0 / sx;
    let mut mapping = super::residual_transform::coordinate_map(
        context,
        |px, py| {
            let u = f64::from(px) + 0.5;
            let v = f64::from(py) + 0.5;
            let dx = u - center.0;
            let dy = v - center.1;
            let radius = (dx * sx).hypot(dy * sy);
            let gx = dx * sx * sx;
            let gy = dy * sy * sy;
            let norm = gx.hypot(gy);
            let wave = amplitude * (std::f64::consts::TAU * (radius / period + phase)).sin();
            let (mut source_x, mut source_y) = if norm > 0.0 { (u + wave * gx / norm, v + wave * gy / norm) } else { (u, v) };
            if clamp {
                source_x = source_x.clamp(0.5, width - 0.5);
                source_y = source_y.clamp(0.5, height - 0.5);
            }
            (source_x, source_y)
        },
        "Source concentric waves become an independently authored standard radial sine coordinate resource feeding ordinary native Displace. Center uses relative native owner coordinates; phase uses complete turns; period and amplitude use pixels. Aspect uses area-preserving elliptic distance and its unit-gradient direction. This native geometry policy approximates source intent without retaining a source processor. Native U8 coordinate quantization, bounded displacement range, bilinear map/image interpolation and native depth/alpha rules apply; source nearest/cubic/nohalo/lohalo reconstruction is replaced by native bilinear sampling.",
    )?;
    for filter in &mut mapping.filters {
        filter.params["undefinedAreas"] = json!("repeat");
    }
    mapping.notes.push(format!(
        "Source clamp {clamp} selects an explicit native policy: true clamps authored source coordinates to the nearest owner pixel center; false keeps the full radial coordinates with native repeated-edge sampling. This replaces source abyss/reconstruction behavior rather than assuming the short source clamp description establishes identical boundary semantics. Stored center {x}/{y}, phase {phase}, aspect {aspect}, amplitude {amplitude}px and period {period}px all influence the editable native map"
    ));
    Ok(mapping)
}

fn whirl(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    use super::{NativeBranch, NativeStructure};
    use photocraft_doc::vector::PathOp;
    use photocraft_doc::{Path, Subpath, VectorMask};
    use photocraft_geom::Rect;
    allowed(e, &["whirl", "pinch", "radius"])?;
    let angle = number(e, "whirl", 90.0, -f64::MAX, f64::MAX)?;
    let pinch = number(e, "pinch", 0.0, -1.0, 1.0)?;
    let radius = number(e, "radius", 1.0, 0.0, 2.0)?;
    if radius == 0.0 || (angle == 0.0 && pinch == 0.0) {
        return Ok(NativeMapping {
            notes: vec!["Source whirl/pinch radius or both deformation amounts are zero; native input stays unchanged".into()],
            ..Default::default()
        });
    }
    let passes = if angle == 0.0 { 0.0 } else { (angle.abs() / 999.0).ceil() };
    let maximum = context.remaining_metadata_bytes.min(context.remaining_generation_bytes).saturating_sub(4096) / 512;
    if passes + usize::from(pinch != 0.0) as f64 > maximum.min(64) as f64 {
        return Err("Whirl exceeds the ordinary native64-filter chain or actual remaining parameter allowance".into());
    }
    context.interrupt.check().map_err(|e| e.to_string())?;
    let mut filters = Vec::new();
    filters.try_reserve_exact(passes as usize + usize::from(pinch != 0.0)).map_err(|_| "native whirl filter allocation unavailable")?;
    for _ in 0..passes as usize {
        context.interrupt.check().map_err(|e| e.to_string())?;
        filters.push(native_filter("filter.distort.twirl", json!({"angle":angle/passes})));
    }
    if pinch != 0.0 {
        filters.push(native_filter("filter.distort.pinch", json!({"amount":pinch*100.0})));
    }
    let note = "Source positive whirl/pinch directions become ordinary native Twirl then Pinch filters. Native elliptic support, radial falloff and repeated bilinear sampling approximate the combined source deformation; angle passes are limited by the existing native64-filter chain and actual parameter allowance rather than an arbitrary16-pass cap";
    if radius == 1.0 {
        return Ok(NativeMapping { filters, notes: vec![note.into()], ..Default::default() });
    }
    if context.remaining_metadata_bytes < 4096 || context.remaining_generation_bytes < 4096 {
        return Err("native whirl frame geometry exceeds remaining allowance".into());
    }
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err(invalid("native whirl owner bounds are empty"));
    }
    let width = (f64::from(bounds.width()) * radius).round().max(1.0);
    let height = (f64::from(bounds.height()) * radius).round().max(1.0);
    if width > f64::from(crate::xcf::MAX_SIDE) || height > f64::from(crate::xcf::MAX_SIDE) {
        return Err("native whirl frame exceeds native document dimensions".into());
    }
    let x = (f64::from(bounds.x0) + (f64::from(bounds.width()) - width) / 2.0).floor();
    let y = (f64::from(bounds.y0) + (f64::from(bounds.height()) - height) / 2.0).floor();
    if x < f64::from(i32::MIN + 1) || y < f64::from(i32::MIN + 1) || x + width > f64::from(i32::MAX) || y + height > f64::from(i32::MAX) {
        return Err(invalid("native whirl frame coordinates overflow"));
    }
    let frame = Rect::new(x as i32, y as i32, (x + width) as i32, (y + height) as i32);
    let ellipse = photocraft_vector::shapes::ellipse(x, y, width, height);
    let mut outside = Path::new(vec![Subpath::polygon(&[
        (f64::from(bounds.x0), f64::from(bounds.y0)),
        (f64::from(bounds.x1), f64::from(bounds.y0)),
        (f64::from(bounds.x1), f64::from(bounds.y1)),
        (f64::from(bounds.x0), f64::from(bounds.y1)),
    ])]);
    let mut hole = ellipse.clone();
    for subpath in &mut hole.subpaths {
        subpath.op = PathOp::Subtract;
    }
    outside.subpaths.extend(hole.subpaths);
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite {
            branches: vec![
                NativeBranch { vector_mask: Some(VectorMask::new(outside)), ..Default::default() },
                NativeBranch { filters, filter_frame: Some(frame), vector_mask: Some(VectorMask::new(ellipse)), ..Default::default() },
            ],
        }),
        notes: vec![format!(
            "{note}. Source radius {radius} selects a centered native parentcanvas {width} by {height} pixels: input is translated into it before filtering and ordinary native Smart Object placement restores the frame origin. Editable complementary ellipse masks keep original pixels outside the affected region and the framed deformation inside. Native integer frame snapping, ellipse aspect, antialias alpha, transparent frame padding, interpolation and owner clipping differ from source radius rules"
        )],
        ..Default::default()
    })
}

fn wind(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["style", "direction", "edge", "threshold", "strength", "seed"])?;
    let style = choice(e, "style", 0, &["wind", "blast"])?;
    let direction = choice(e, "direction", 0, &["left", "right", "top", "bottom"])?;
    let edge = choice(e, "edge", 1, &["both", "leading", "trailing"])?;
    let threshold = integer(e, "threshold", 10, 0, 50)?;
    let strength = integer(e, "strength", 10, 1, 100)?;
    let seed = match e.argument("seed").map_err(invalid)? {
        None => 0,
        Some(ArgumentValue::UInt(v)) => *v,
        _ => return Err(invalid("Wind seed must be an unsigned stored integer")),
    };
    // Stock source probe: left/right/top/bottom name the streak destination.
    let from_right = matches!(direction, 0 | 3);
    let gain = f64::from(strength) / 10.0;
    let passes = gain.ceil() as usize;
    if passes
        .checked_mul(512)
        .and_then(|bytes| bytes.checked_add(4096))
        .is_none_or(|bytes| bytes > context.remaining_metadata_bytes.min(context.remaining_generation_bytes))
    {
        return Err("Native Wind passes exceed actual remaining parameter allowance".into());
    }
    let mut filters = Vec::new();
    filters.try_reserve_exact(passes).map_err(|_| "Native Wind pass allocation unavailable")?;
    for pass in 0..passes {
        context.interrupt.check().map_err(|error| error.to_string())?;
        let mut filter = native_filter(
            "filter.stylize.wind",
            json!({"method":if style==0 {"wind"} else {"blast"},"direction":if from_right {"fromRight"} else {"fromLeft"},"seed":seed}),
        );
        filter.opacity = (gain - pass as f64).min(1.0) as f32;
        filters.push(filter);
    }
    Ok(NativeMapping {
        quarter_turns: if direction >= 2 { 1 } else { 0 },
        filters,
        notes: vec![format!(
            "Source Wind/Blast and actual stored seed become native Wind. Source directions name destination: left→fromRight/right→fromLeft; vertical directions use a native quarter-turn sandwich with matching destination. Source strength {strength} selects an explicit native magnitude policy relative to source default10: strength/10 native passes, with the fractional final pass mixed through ordinary native filter opacity (at most10 passes). This increases native streak reach/intensity, not identical source length or random samples. Native streak lengths, absolute edge detector and threshold apply; source polarity edge{edge} and threshold{threshold} are superseded by ordinary native fixed absolute edge detection because native Wind has no directional sign/gradient threshold controls. An invert sandwich cannot restore polarity selection for an absolute detector"
        )],
        ..Default::default()
    })
}

fn scalar(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["n-components", "factor"])?;
    let components = integer(e, "n-components", 2, 1, 16)?;
    let factor = number(e, "factor", 1.0, f64::from(f32::MIN_POSITIVE), f64::from(f32::MAX))?;
    if factor == 1.0 {
        return Ok(NativeMapping { notes: vec!["Source scalar factor is1; all native color and alpha samples stay unchanged".into()], ..Default::default() });
    }
    if !matches!(context.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Scalar component conversion requires a native RGB/Gray channel interpretation".into());
    }
    let color_channels = if context.mode == ColorMode::Rgb { 3 } else { 1 };
    let applied = components.min(color_channels);
    let mut filters = Vec::new();
    if context.mode == ColorMode::Rgb && factor <= 2.0 {
        let rows: [Vec<f64>; 3] = std::array::from_fn(|i| {
            let mut row = vec![0.0; 4];
            if let Some(slot) = row.get_mut(i) {
                *slot = 100.0 * if (i as u32) < applied { factor } else { 1.0 };
            }
            row
        });
        let [red, green, blue] = rows;
        filters.push(native_filter("image.adjustments.channelMixer", json!({"red":red,"green":green,"blue":blue,"monochrome":false})));
    } else {
        let outputs = (0..256).map(|i| (f64::from(i) * factor).clamp(0.0, 255.0)).collect::<Vec<_>>();
        let mut params = json!({"pencil":true});
        if context.mode == ColorMode::Grayscale {
            params["points"] = json!(outputs);
        } else {
            for name in ["red", "green", "blue"].into_iter().take(applied as usize) {
                params[name] = json!(outputs);
            }
        }
        filters.push(native_filter("image.adjustments.curves", params));
    }
    Ok(NativeMapping {
        filters,
        notes: vec![format!(
            "Source scalar factor{factor} becomes initial native color-component scaling on{applied} channels through diagonal Channel Mixer where coefficients fit, otherwise a bounded native256-sample pencil Curve. This is a native approximation of component intent; generic source component layout is not asserted identical to native RGB/Gray. Native document encoding, interpolation and gamut/HDR clipping apply. Requested components{components} beyond native color channels{color_channels} remain retained; generic data beyond the native color channels has no inferred alpha identity: native alpha remains unchanged, never replaced by layer opacity. Independent stock generic n-components4 conversion is not RGBA (a factor0.5 probe produced gray0.125 with alpha1 from RGB0.25/0.5/0.75), so component count cannot authorize an AlphaCurve"
        )],
        ..Default::default()
    })
}

pub(super) fn map(e: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    Some(match e.operation {
        "gegl:waves" => waves(e, context),
        "gegl:whirl-pinch" => whirl(e, context),
        "gegl:wind" => wind(e, context),
        "gimp:scalar-multiply" => scalar(e, context),
        "gimp:set-alpha" | "gimp:threshold-alpha" => {
            if let Err(error) = allowed(e, &["value"]) {
                return Some(Err(error));
            }
            let value = match number(e, "value", if e.operation == "gimp:set-alpha" { 1.0 } else { 0.5 }, 0.0, 1.0) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            let command = if e.operation == "gimp:set-alpha" { "filter.other.setAlpha" } else { "filter.other.thresholdAlpha" };
            Ok(NativeMapping{filters:vec![native_filter(command,json!({"value":value}))],notes:vec![if e.operation=="gimp:set-alpha"{
                "Source unary constant alpha replacement becomes ordinary native Set Alpha; no auxiliary source is required or invented. Native color samples, depth, and document editing/compositing semantics are preserved."
            }else{
                "Source unary alpha threshold becomes ordinary native Threshold Alpha with explicit strict alpha>value rule (equality becomes0). Native color samples and document editing/compositing semantics are preserved; source scalar-layout differences remain an explicit native approximation."
            }.into()],..Default::default()})
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::Argument;
    use super::*;
    use photocraft_geom::Rect;
    fn ctx() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(-20, 5, 180, 105),
            native_filter_bounds: Rect::new(0, 0, 200, 100),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn arg(name: &'static str, value: ArgumentValue<'static>) -> Argument<'static> {
        Argument { name, value }
    }
    fn run(op: &'static str, args: Vec<Argument<'static>>) -> Result<NativeMapping> {
        map(&StoredEffect::test(op, args), &ctx()).unwrap()
    }
    #[test]
    fn wind_destination_and_axis_are_native_and_seed_survives() {
        for (direction, native, turns) in [(0, "fromRight", 0), (1, "fromLeft", 0), (2, "fromLeft", 1), (3, "fromRight", 1)] {
            let a = run("gegl:wind", vec![arg("direction", ArgumentValue::Enum(direction)), arg("seed", ArgumentValue::UInt(u32::MAX))]).unwrap();
            assert_eq!(a.filters[0].params["direction"], native);
            assert_eq!(a.quarter_turns, turns);
            assert_eq!(a.filters[0].params["seed"], u32::MAX);
        }
    }
    #[test]
    fn wind_magnitude_is_native_repetition_and_fractional_opacity_with_budgets() {
        let small = run("gegl:wind", vec![arg("strength", ArgumentValue::Int(1))]).unwrap();
        assert_eq!(small.filters.len(), 1);
        assert_eq!(small.filters[0].opacity, 0.1);
        let large = run("gegl:wind", vec![arg("strength", ArgumentValue::Int(25))]).unwrap();
        assert_eq!(large.filters.iter().map(|filter| filter.opacity).collect::<Vec<_>>(), vec![1.0, 1.0, 0.5]);
        assert_eq!(run("gegl:wind", vec![arg("strength", ArgumentValue::Int(100))]).unwrap().filters.len(), 10);
        assert!(wind(&StoredEffect::test("gegl:wind", vec![]), &MappingContext { remaining_metadata_bytes: 1, ..ctx() }).is_err());
    }
    #[test]
    fn wind_strength_matches_independent_native_stages_and_reopens_as_editable_filters() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Affine, Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 32, 2);
        let mapping = run("gegl:wind", vec![arg("strength", ArgumentValue::Int(25)), arg("seed", ArgumentValue::UInt(17))]).unwrap();
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut original = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            original.write_region(bounds, &(0..64).flat_map(|i| [if i % 32 >= 16 { 0.9 } else { 0.1 }, 0.25, 0.75, 1.0]).collect::<Vec<_>>());
            let apply = |input: &Surface, seed, opacity| {
                let changed = photocraft_algo::apply_in_with(
                    input,
                    &photocraft_algo::FilterParams::Wind { method: photocraft_algo::WindMethod::Wind, from_right: true, seed },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap();
                let mut document = Document::new("Native Wind mix", Size::new(32, 2), ColorMode::Rgb, depth);
                document.layers.push(Layer::new("Input", LayerContent::Raster(input.clone())));
                let mut top = Layer::new("Wind", LayerContent::Raster(changed));
                top.opacity = opacity;
                document.layers.push(top);
                let pixels = photocraft_compose::render(&document, bounds).px.into_iter().flatten().collect::<Vec<_>>();
                let mut result = Surface::new(input.format());
                result.write_region(bounds, &pixels);
                result
            };
            let mut actual = original.clone();
            for filter in &mapping.filters {
                actual = apply(&actual, filter.params["seed"].as_u64().unwrap() as u32, filter.opacity);
            }
            let expected = apply(&apply(&apply(&original, 17, 1.0), 17, 1.0), 17, 0.5);
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            let mut document = Document::new("Native Wind input", Size::new(32, 2), ColorMode::Rgb, depth);
            document.layers.push(Layer::new("Pixels", LayerContent::Raster(original)));
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let mut smart =
                SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
            smart.smart_filters = mapping.filters.clone();
            document.layers = vec![Layer::new("Editable native Wind", LayerContent::Smart(smart))];
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native Wind Smart Object") };
            assert_eq!(restored.smart_filters, mapping.filters);
        }
    }
    #[test]
    fn radial_waves_keep_center_phase_aspect_and_boundary_as_native_map_data() {
        let a = run("gegl:waves", vec![arg("amplitude", ArgumentValue::Float(2.0)), arg("period", ArgumentValue::Float(25.0))]).unwrap();
        assert_eq!(a.filters[0].command, "filter.distort.displace");
        assert_eq!(a.filters[0].params["undefinedAreas"], "repeat");
        let map = crate::smart_map::displace_embedded_map(&a.filters[0].params).unwrap();
        assert_eq!(map.rect, Rect::new(0, 0, 200, 100));
        for args in [
            vec![arg("x", ArgumentValue::Float(0.2))],
            vec![arg("phi", ArgumentValue::Float(0.25))],
            vec![arg("aspect", ArgumentValue::Float(2.0))],
            vec![arg("clamp", ArgumentValue::Bool(true))],
        ] {
            let changed = run("gegl:waves", args).unwrap();
            assert_ne!(changed.filters[0].params["__displacePsd"], run("gegl:waves", vec![]).unwrap().filters[0].params["__displacePsd"]);
        }
        assert!(run("gegl:waves", vec![arg("amplitude", ArgumentValue::Float(0.0))]).unwrap().filters.is_empty());
        let tiny = MappingContext { remaining_metadata_bytes: 1, ..ctx() };
        assert!(waves(&StoredEffect::test("gegl:waves", vec![]), &tiny).is_err());
    }
    #[test]
    fn wave_native_sine_counterpart_renders_and_survives_native_edit_reopen() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Affine, Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 4, 1);
        let context = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..ctx() };
        let mapping = waves(
            &StoredEffect::test(
                "gegl:waves",
                vec![arg("x", ArgumentValue::Float(0.0)), arg("amplitude", ArgumentValue::Float(1.0)), arg("period", ArgumentValue::Float(4.0))],
            ),
            &context,
        )
        .unwrap();
        let filter = mapping.filters[0].clone();
        let embedded = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        // Independent samples of sin(pi/4), sin(3pi/4), sin(5pi/4), sin(7pi/4).
        let independent = std::sync::Arc::new(photocraft_algo::Image {
            rect: bounds,
            ch: 3,
            data: [1.0, 1.0, 0.0, 0.0].into_iter().flat_map(|x| [x, 128.0 / 255.0, 0.5]).collect(),
        });
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.write_region(bounds, &[0.0, 0.2, 1.5, 0.25, 0.3, 0.4, 1.5, 0.5, 0.6, 0.6, 1.5, 0.75, 1.0, 0.8, 1.5, 1.0]);
        let apply = |horizontal, map| {
            photocraft_algo::apply_in_with(
                &source,
                &photocraft_algo::FilterParams::Displace {
                    horizontal,
                    vertical: 0.0,
                    stretch: true,
                    undefined: photocraft_algo::UndefinedAreas::Repeat,
                    map: Some(map),
                },
                bounds,
                bounds,
                None,
                bounds,
                &Interrupt::NONE,
            )
            .unwrap()
            .read_region(bounds)
        };
        let horizontal = filter.params["horizontal"].as_f64().unwrap() as f32;
        assert_eq!(apply(horizontal, embedded.clone()), apply((0.5_f64.sqrt() * 100.0 / 128.0) as f32, independent));
        let mut input = Document::new("Native wave input", Size::new(4, 1), ColorMode::Rgb, SampleType::F32);
        input.layers.push(Layer::new("Pixels", LayerContent::Raster(source.clone())));
        let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
        smart.smart_filters = mapping.filters;
        let mut doc = Document::new("Native wave", Size::new(4, 1), ColorMode::Rgb, SampleType::F32);
        doc.layers.push(Layer::new("Editable Displace", LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native Smart Object") };
        let map = crate::smart_map::displace_embedded_map(&restored.smart_filters[0].params).unwrap();
        assert_eq!(apply(horizontal, map), apply(horizontal, embedded));
        assert_eq!(apply(0.0, crate::smart_map::displace_embedded_map(&filter.params).unwrap()), source.read_region(bounds));
        let cancelled = || true;
        assert!(waves(&StoredEffect::test("gegl:waves", vec![]), &MappingContext { interrupt: Interrupt::cancel_only(&cancelled), ..context }).is_err());
    }
    #[test]
    fn whirl_pinch_are_ordered_and_large_angles_bounded() {
        let a = run("gegl:whirl-pinch", vec![arg("whirl", ArgumentValue::Float(1500.0)), arg("pinch", ArgumentValue::Float(0.5))]).unwrap();
        assert_eq!(a.filters.len(), 3);
        assert_eq!(a.filters[0].params["angle"], 750.0);
        assert_eq!(a.filters[2].params["amount"], 50.0);
        assert!(run("gegl:whirl-pinch", vec![arg("whirl", ArgumentValue::Float(1e30))]).is_err());
        let larger = run("gegl:whirl-pinch", vec![arg("whirl", ArgumentValue::Float(20000.0))]).unwrap();
        assert!(larger.filters.len() > 16 && larger.filters.len() <= 64);
    }
    #[test]
    fn whirl_radius_is_an_editable_native_parent_frame_with_complementary_masks() {
        use super::super::NativeStructure;
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 12, 8);
        let mut c = ctx();
        c.native_filter_bounds = bounds;
        c.owner_bounds = bounds;
        let effect = StoredEffect::test("gegl:whirl-pinch", vec![arg("whirl", ArgumentValue::Float(90.0)), arg("radius", ArgumentValue::Float(0.5))]);
        let mapped = map(&effect, &c).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("native parent frame") };
        let frame = branches[1].filter_frame.unwrap();
        assert_eq!(frame, Rect::new(3, 2, 9, 6));
        assert_eq!(branches[1].filters[0].params["angle"], 90.0);
        let outside = photocraft_vector::vector_mask_values(branches[0].vector_mask.as_ref().unwrap(), bounds);
        let inside = photocraft_vector::vector_mask_values(branches[1].vector_mask.as_ref().unwrap(), bounds);
        assert_eq!((outside[0], inside[0]), (1.0, 0.0));
        assert_eq!((outside[4 * 12 + 6], inside[4 * 12 + 6]), (0.0, 1.0));
        for (a, b) in outside.iter().zip(&inside) {
            assert!((a + b - 1.0).abs() < 1e-5);
        }
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let mut source = Surface::new(format);
        source.write_region(bounds, &(0..8).flat_map(|y| (0..12).flat_map(move |x| [x as f32 / 12.0, y as f32 / 8.0, 0.25, 0.6])).collect::<Vec<_>>());
        let local = Rect::new(0, 0, 6, 4);
        let mut input = Surface::new(format);
        input.write_region(local, &source.read_region(frame));
        let fp = photocraft_algo::FilterParams::Twirl { angle: 90.0 };
        let framed = photocraft_algo::apply_in_with(&input, &fp, local, local, None, local, &Interrupt::NONE).unwrap();
        let independent = photocraft_algo::apply_in_with(&source, &fp, frame, frame, None, bounds, &Interrupt::NONE).unwrap();
        for y in frame.y0..frame.y1 {
            for x in frame.x0..frame.x1 {
                for (local, global) in framed.pixel(x - frame.x0, y - frame.y0).iter().zip(independent.pixel(x, y)) {
                    assert!((*local - global).abs() < 1e-6);
                }
            }
        }
        let mut restored = Surface::new(format);
        restored.write_region(frame, &framed.read_region(local));
        let mut document = Document::new("Native radius frame", Size::new(12, 8), ColorMode::Rgb, SampleType::F32);
        for (index, branch) in branches.into_iter().enumerate() {
            let mut layer = Layer::new("Native ellipse region", LayerContent::Raster(if index == 0 { source.clone() } else { restored.clone() }));
            layer.vector_mask = branch.vector_mask;
            document.layers.push(layer);
        }
        let rendered = photocraft_compose::render(&document, bounds);
        assert_eq!(rendered.px[0].as_slice(), source.pixel(0, 0).as_slice());
        assert!((rendered.px[4 * 12 + 6][3] - 0.6).abs() < 1e-6);
        assert_ne!(rendered.px[3 * 12 + 7].as_slice(), source.pixel(7, 3).as_slice());
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, bounds).px, rendered.px);
        c.remaining_metadata_bytes = 0;
        assert!(map(&effect, &c).unwrap().is_err());
        let cancel = || true;
        c = ctx();
        c.interrupt = Interrupt::cancel_only(&cancel);
        assert!(map(&effect, &c).unwrap().is_err());
    }
    #[test]
    fn scalar_preserves_channel_scope_and_reports_unapplied_alpha() {
        let a = run("gimp:scalar-multiply", vec![arg("n-components", ArgumentValue::Int(2)), arg("factor", ArgumentValue::Float(0.5))]).unwrap();
        assert_eq!(a.filters[0].params["red"], json!([50.0, 0.0, 0.0, 0.0]));
        assert_eq!(a.filters[0].params["blue"], json!([0.0, 0.0, 100.0, 0.0]));
        let a = run("gimp:scalar-multiply", vec![arg("n-components", ArgumentValue::Int(4)), arg("factor", ArgumentValue::Float(3.0))]).unwrap();
        assert_eq!(a.filters[0].command, "image.adjustments.curves");
        assert!(a.notes[0].contains("component count cannot authorize an AlphaCurve"));
        assert!(a.filters.iter().all(|filter| !filter.command.contains("alpha")));
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                run("gimp:scalar-multiply", vec![arg("factor", ArgumentValue::Float(value))]).unwrap_err().reason,
                ConversionReason::InvalidOptionalData
            );
        }
    }
    #[test]
    fn generic_scalar_component_count_never_relabels_native_alpha() {
        use photocraft_color::SampleType;
        use photocraft_doc::Adjustment;
        let mapping = run("gimp:scalar-multiply", vec![arg("n-components", ArgumentValue::Int(4)), arg("factor", ArgumentValue::Float(0.5))]).unwrap();
        assert_eq!(mapping.filters.len(), 1);
        let params = &mapping.filters[0].params;
        let matrix = std::array::from_fn(|row| std::array::from_fn(|column| params[["red", "green", "blue"][row]][column].as_f64().unwrap() as f32 / 100.0));
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let original: Vec<_> = (0..5).map(|i| [0.25, 0.5, 0.75, i as f32 / 4.0]).collect();
            let mut actual = photocraft_compose::Buffer { rect: Rect::new(0, 0, 5, 1), px: original.clone() };
            let mut expected = actual.clone();
            let transfer = photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth);
            photocraft_compose::adjust::apply_in(&Adjustment::ChannelMixer { matrix, monochrome: false }, &mut actual, transfer, Some(depth), ColorMode::Rgb);
            photocraft_compose::adjust::apply_in(
                &Adjustment::ChannelMixer { matrix: [[0.5, 0., 0., 0.], [0., 0.5, 0., 0.], [0., 0., 0.5, 0.]], monochrome: false },
                &mut expected,
                transfer,
                Some(depth),
                ColorMode::Rgb,
            );
            assert_eq!(actual.px, expected.px);
            assert!(actual.px.iter().zip(original).all(|(actual, original)| actual[3] == original[3]));
        }
        assert!(mapping.notes[0].contains("generic n-components4 conversion is not RGBA"));
    }
    #[test]
    fn alpha_replacement_is_not_misrepresented_as_layer_opacity() {
        for op in ["gimp:set-alpha", "gimp:threshold-alpha"] {
            assert!(run(op, vec![]).unwrap().structure.is_none());
            assert_eq!(run(op, vec![arg("value", ArgumentValue::Float(1.1))]).unwrap_err().reason, ConversionReason::InvalidOptionalData);
        }
        assert!(map(&StoredEffect::test("not:owned", vec![]), &ctx()).is_none());
    }
    #[test]
    fn declared_aux_pad_is_not_mistaken_for_a_missing_required_source_resource() {
        for (source, native) in [("gimp:set-alpha", "filter.other.setAlpha"), ("gimp:threshold-alpha", "filter.other.thresholdAlpha")] {
            for value in [0.0, 0.25, 0.5, 1.0] {
                let mapping = run(source, vec![arg("value", ArgumentValue::Float(value))]).unwrap();
                assert_eq!(mapping.filters[0].command, native);
                assert_eq!(mapping.filters[0].params["value"], value);
                assert_eq!(mapping.filters[0].opacity, 1.);
            }
        }
        assert!(run("gimp:threshold-alpha", vec![]).unwrap().notes[0].contains("strict alpha>value"));
    }
}
