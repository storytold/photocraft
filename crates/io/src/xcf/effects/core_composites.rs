//! Frozen B07 core operations translated only to ordinary native data.
//! Binary operations require their actual auxiliary pixels; the owner is never fabricated as aux.
use super::{MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, NativeStructure, StoredEffect, native_filter};
use photocraft_color::BlendMode;
use photocraft_doc::import_conversion::ConversionReason;
use serde_json::json;
type Result<T> = std::result::Result<T, MappingFailure>;

fn allowed(effect: &StoredEffect<'_>, fields: &[&str]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for argument in &effect.arguments {
        if !fields.contains(&argument.name) || !seen.insert(argument.name) {
            return Err(MappingFailure {
                reason: ConversionReason::InvalidOptionalData,
                message: format!("Unknown or duplicate {} field {}", effect.operation, argument.name),
            });
        }
    }
    Ok(())
}
fn number(effect: &StoredEffect<'_>, name: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let value = effect.number(name)?.unwrap_or(default);
    if !value.is_finite() || !(lo..=hi).contains(&value) {
        return Err(MappingFailure { reason: ConversionReason::InvalidOptionalData, message: format!("{name} is outside finite source bounds") });
    }
    Ok(value)
}
fn integer(effect: &StoredEffect<'_>, name: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let value = number(effect, name, default, lo, hi)?;
    if value.fract() != 0.0 {
        return Err(format!("{name} requires an integer pixel count").into());
    }
    Ok(value)
}
fn morphology_passes(radius: f64, grow: bool) -> Vec<photocraft_doc::SmartFilter> {
    let mut remaining = radius;
    let mut filters = Vec::new();
    // Frozen source bound2342 -> at most5 native passes, each within native radius<=500.
    for _ in 0..5 {
        if remaining <= 0.0 {
            break;
        }
        let step = remaining.min(500.0);
        filters.push(native_filter(if grow { "filter.other.maximum" } else { "filter.other.minimum" }, json!({"radius":step,"preserve":"roundness"})));
        remaining -= step;
    }
    filters
}
fn morphology(effect: &StoredEffect<'_>, border: bool, grow: bool) -> Result<NativeMapping> {
    let fields: &[&str] = if border {
        &["radius-x", "radius-y", "feather", "edge-lock"]
    } else if grow {
        &["radius-x", "radius-y"]
    } else {
        &["radius-x", "radius-y", "edge-lock"]
    };
    allowed(effect, fields)?;
    let x = integer(effect, "radius-x", 1.0, 1.0, 2342.0)?;
    let y = integer(effect, "radius-y", 1.0, 1.0, 2342.0)?;
    let edge = effect.boolean("edge-lock")?.unwrap_or(false);
    let feather = effect.boolean("feather")?.unwrap_or(false);
    let radius = (x * y).sqrt();
    let mut mapping = NativeMapping {
        notes: vec![format!(
            "Source mask morphology radii({x},{y}) pixels become ordinary native per-channel RGBA morphology, including alpha. Equal axes preserve radius; unequal axes use an explicit area-equivalent circular radius sqrt(rx*ry), not directional/source-kernel equivalence. Radius>500 is a bounded sum of native roundness passes; discrete-kernel differences apply. Source edge-lock={edge} becomes native repeated layer-edge sampling. Source coverage-plane/channel semantics and native RGBA compositing differ; original source controls remain inert history."
        )],
        ..Default::default()
    };
    if border {
        let mut dilate = morphology_passes(radius, true);
        let mut erode = morphology_passes(radius, false);
        if feather {
            let mut variance = radius * radius;
            for _ in 0..6 {
                if variance <= 0.0 {
                    break;
                }
                let sigma = variance.sqrt().min(1000.0);
                let filter = native_filter("filter.blur.gaussianBlur", json!({"radius":sigma}));
                dilate.push(filter.clone());
                erode.push(filter);
                variance = (variance - sigma * sigma).max(0.0);
            }
            mapping.notes.push("Source feather becomes ordinary native Gaussian smoothing of both morphology branches at the derived border radius, factored by Gaussian variance into bounded native passes. Linear smoothing commutes with the gray morphological difference; native alpha/interpolation differ.".into());
        }
        mapping.structure = Some(NativeStructure::Composite {
            branches: vec![
                NativeBranch { filters: dilate, ..Default::default() },
                NativeBranch { filters: erode, blend: BlendMode::Difference, ..Default::default() },
            ],
        });
        mapping.notes.push("Source border becomes native dilation/erosion branches combined by native Difference (morphological gradient). This preserves a meaningful editable border structure; native alpha compositing applies, not coverage subtraction.".into());
    } else {
        mapping.filters = morphology_passes(radius, grow);
    }
    Ok(mapping)
}

fn composer(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["layer-mode", "opacity", "blend-space", "composite-space", "composite-mode"])?;
    number(effect, "opacity", 1.0, 0.0, 1.0)?;
    for name in ["layer-mode", "blend-space", "composite-space", "composite-mode"] {
        effect.enum_value(name)?;
    }
    Err(MappingFailure::resource(format!(
        "{} is a two-input composer. Its scalar configuration contains no serialized auxiliary image/coverage pixels, so no complete native blend pair can be constructed. Actual native BlendMode/coverage data could represent the operation only with the real second input; owner pixels are not substituted as auxiliary data.",
        effect.operation
    )))
}
fn crop(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["x", "y", "width", "height"])?;
    integer(effect, "x", 0.0, f64::from(i32::MIN), f64::from(i32::MAX))?;
    integer(effect, "y", 0.0, f64::from(i32::MIN), f64::from(i32::MAX))?;
    integer(effect, "width", 0.0, 0.0, f64::from(i32::MAX))?;
    integer(effect, "height", 0.0, 0.0, f64::from(i32::MAX))?;
    Err(MappingFailure::resource(
        "Compose-crop requires its actual auxiliary composed pixels/coverage; scalar crop bounds alone cannot establish a unary owner crop. No source coordinate convention or missing backdrop is invented.",
    ))
}
fn profile_transform(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["rendering-intent", "black-point-compensation"])?;
    let intent = effect.enum_value("rendering-intent")?.unwrap_or(1);
    if !(0..=3).contains(&intent) {
        return Err("Unknown source ICC rendering intent".into());
    }
    effect.boolean("black-point-compensation")?;
    Err(MappingFailure::resource(
        "Stock effect serialization omits source/destination GimpColorProfile objects and Babl format pointers. Rendering intent/BPC alone cannot identify the two actual native ICC endpoints; document ICC is not silently substituted as both profiles.",
    ))
}
fn mask_components(effect: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(effect, &["mask", "alpha"])?;
    // Public GimpComponentMask enum assigns R/G/B/A bits1/2/4/8 (core-enums.h).
    let mask = integer(effect, "mask", 15.0, 0.0, 15.0)? as u32;
    let alpha = number(effect, "alpha", 0.0, 0.0, 1.0)?;
    if mask == 0 {
        return Ok(NativeMapping {
            notes: vec!["Empty source component mask selects no replacements; native input is unchanged.".into()],
            ..Default::default()
        });
    }
    if mask == 8 {
        return Ok(NativeMapping { filters:vec![native_filter("filter.other.setAlpha",json!({"value":alpha}))], notes:vec!["Source absolute alpha replacement becomes ordinary native Set Alpha, preserving native color samples while assigning coverage independently of the old alpha. Native document compositing and editing rules apply.".into()], ..Default::default() });
    }
    if mask & 7 != 0 {
        return Err(MappingFailure::resource(
            "Replacing source RGB component bits requires the actual auxiliary component pixels, which the scalar effect descriptor does not serialize. Existing owner channels are not invented as the missing auxiliary input.",
        ));
    }
    Err("Invalid source component replacement mask".into())
}
fn semi_flatten(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(effect, &["color"])?;
    let color = if effect.argument("color")?.is_some() {
        super::resources::native_color(effect, "color", context).map_err(MappingFailure::from)?
    } else {
        // Independently observed stockGIMP default is opaque white; no implicit source image.
        photocraft_color::Color::WHITE.in_mode(context.mode)
    };
    Ok(NativeMapping{structure:Some(NativeStructure::Composite{branches:vec![
        NativeBranch{fill:Some(photocraft_doc::Fill::Solid(color)),..Default::default()},
        NativeBranch::default()
    ]}),notes:vec!["Source semi-flatten fill color becomes an editable native solid fill beneath unchanged native input. Native source-over fills fractional coverage, while entirely transparent source pixels also reveal the native underlay; that zero-alpha difference is explicit. Native document color/compositing rules apply; source rendering is not emulated.".into()],..Default::default()})
}

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    Some(match effect.operation {
        "gimp:profile-transform"=>profile_transform(effect),
        "gimp:mask-components"=>mask_components(effect),
        "gimp:semi-flatten"=>semi_flatten(effect,context),
        "gimp:border"=>morphology(effect,true,false),
        "gimp:grow"=>morphology(effect,false,true),
        "gimp:shrink"=>morphology(effect,false,false),
        "gimp:compose-crop"=>crop(effect),
        "gimp:buffer-source-validate"=>allowed(effect,&[]).and_then(|()|Err(MappingFailure::resource("Stock saver does not serialize the declared GEGL buffer object. A missing source buffer cannot be reconstructed from scalar configuration; no owner or external file is substituted."))),
        "gimp:flood"=>allowed(effect,&[]).map(|()|NativeMapping {filters:vec![native_filter("filter.other.fillHoles",json!({"connectivity":4,"channel":"each"}))],notes:vec!["Source hole filling becomes ordinary native Fill Holes using boundary-connected four-neighbor grayscale reconstruction independently on each native channel, including alpha. The native topology/filter is an explicit approximation to source coverage-plane processing; native document color/compositing rules apply.".into()],..Default::default()}),
        "gimp:addition-legacy"|"gimp:anti-erase"|"gimp:behind"|"gimp:burn-legacy"|"gimp:darken-only-legacy"|
        "gimp:difference-legacy"|"gimp:divide-legacy"|"gimp:dodge-legacy"|"gimp:erase"|"gimp:grain-extract-legacy"|
        "gimp:grain-merge-legacy"|"gimp:hardlight-legacy"|"gimp:hsl-color-legacy"|"gimp:hsv-hue-legacy"|
        "gimp:hsv-saturation-legacy"|"gimp:hsv-value-legacy"|"gimp:layer-mode"|"gimp:lighten-only-legacy"|
        "gimp:merge"|"gimp:multiply-legacy"|"gimp:normal"|"gimp:overwrite"|"gimp:pass-through"|"gimp:replace"|
        "gimp:screen-legacy"|"gimp:softlight-legacy"|"gimp:split"|"gimp:subtract-legacy"=>composer(effect),
        _=>return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue, ColorLiteral};
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Fill, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;
    const AUXILIARY: &[&str] = &[
        "gimp:addition-legacy",
        "gimp:anti-erase",
        "gimp:behind",
        "gimp:burn-legacy",
        "gimp:darken-only-legacy",
        "gimp:difference-legacy",
        "gimp:divide-legacy",
        "gimp:dodge-legacy",
        "gimp:erase",
        "gimp:grain-extract-legacy",
        "gimp:grain-merge-legacy",
        "gimp:hardlight-legacy",
        "gimp:hsl-color-legacy",
        "gimp:hsv-hue-legacy",
        "gimp:hsv-saturation-legacy",
        "gimp:hsv-value-legacy",
        "gimp:layer-mode",
        "gimp:lighten-only-legacy",
        "gimp:merge",
        "gimp:multiply-legacy",
        "gimp:normal",
        "gimp:overwrite",
        "gimp:pass-through",
        "gimp:replace",
        "gimp:screen-legacy",
        "gimp:softlight-legacy",
        "gimp:split",
        "gimp:subtract-legacy",
        "gimp:compose-crop",
        "gimp:buffer-source-validate",
        "gimp:profile-transform",
    ];
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(-2, 3, 3, 8),
            native_filter_bounds: Rect::new(0, 0, 5, 5),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn run(operation: &'static str, args: Vec<Argument<'_>>) -> Result<NativeMapping> {
        map(&StoredEffect::test(operation, args), &context()).expect("owned exact identity")
    }
    fn int(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Int(value) }
    }
    fn flag(name: &'static str, value: bool) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Bool(value) }
    }

    #[test]
    fn all_frozen_binary_rows_require_actual_second_input_or_profile_endpoints() {
        assert_eq!(AUXILIARY.len(), 31);
        for operation in AUXILIARY {
            let failure = run(operation, vec![]).expect_err("no auxiliary source pixels");
            assert_eq!(failure.reason, ConversionReason::ResourceUnavailable, "{operation}");
            assert!(!failure.message.is_empty());
        }
        assert!(map(&StoredEffect::test("gegl:exposure", vec![]), &context()).is_none());
    }
    #[test]
    fn morphology_source_domain_uses_bounded_native_passes_without_clamping() {
        for operation in ["gimp:grow", "gimp:shrink"] {
            let m = run(operation, vec![int("radius-x", 2342), int("radius-y", 2342)]).expect("full source domain");
            assert_eq!(m.filters.len(), 5);
            let total: f64 = m.filters.iter().map(|f| f.params["radius"].as_f64().expect("pixel radius")).sum();
            assert_eq!(total, 2342.0);
            assert!(m.filters.iter().all(|f| f.params["radius"].as_f64().is_some_and(|v| v <= 500.0)));
        }
        let anisotropic = run("gimp:grow", vec![int("radius-x", 2), int("radius-y", 8)]).expect("declared native circular approximation");
        assert_eq!(anisotropic.filters[0].params["radius"], 4.0);
        assert!(anisotropic.notes[0].contains("area-equivalent"));
        let border = run("gimp:border", vec![int("radius-x", 2342), int("radius-y", 2342), flag("feather", true)]).expect("bounded border");
        assert!(border.filters.is_empty());
        let Some(NativeStructure::Composite { branches }) = border.structure else { panic!("native branches") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[1].blend, BlendMode::Difference);
        assert!(branches.iter().all(|branch| branch.filters.len() == 11));
    }
    #[test]
    fn native_roundness_morphology_changes_rgba_coverage_at_all_depths() {
        let area = Rect::new(0, 0, 5, 5);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            surface.write_pixel(2, 2, &[1.0, 1.0, 1.0, 1.0]);
            let grown = photocraft_algo::apply_in(
                &surface,
                &photocraft_algo::FilterParams::Maximum { radius: 1.0, preserve: photocraft_algo::Preserve::Roundness },
                area,
                area,
                None,
                area,
            );
            for (x, y) in [(2, 2), (1, 2), (3, 2), (2, 1), (2, 3)] {
                assert_eq!(grown.pixel(x, y), vec![1.0; 4]);
            }
            assert_eq!(grown.pixel(1, 1), vec![0.0; 4]);
            let shrunk = photocraft_algo::apply_in(
                &grown,
                &photocraft_algo::FilterParams::Minimum { radius: 1.0, preserve: photocraft_algo::Preserve::Roundness },
                area,
                area,
                None,
                area,
            );
            assert_eq!(shrunk.pixel(2, 2), vec![1.0; 4]);
            assert_eq!(shrunk.pixel(1, 2), vec![0.0; 4]);
        }
    }
    #[test]
    fn semi_flatten_native_underlay_renders_edits_and_reopens_as_native_fill() {
        let literal = ColorLiteral { encoding: "R'G'B'A u8", pixels: &[0, 0, 255, 255], profile: &[] };
        let m = run("gimp:semi-flatten", vec![Argument { name: "color", value: ArgumentValue::Color(Some(literal)) }]).expect("native underlay");
        assert!(m.notes[0].contains("entirely transparent"));
        let Some(NativeStructure::Composite { mut branches }) = m.structure else { panic!("native composite") };
        let fill = branches.remove(0).fill.expect("real native fill, not duplicate owner");
        assert!(branches[0].fill.is_none());
        let mut doc = Document::new("Native underlay", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
        doc.layers.push(Layer::new("Editable native fill", LayerContent::Fill(fill)));
        let mut owner = Layer::raster("Editable native source", doc.pixel_format());
        owner.surface_mut().expect("raster").write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.5]);
        doc.layers.push(owner);
        let output = photocraft_compose::flatten(&doc);
        assert!((output.px[0][0] - 0.5).abs() < 1e-4 && (output.px[0][2] - 0.5).abs() < 1e-4);
        assert_eq!(output.px[0][3], 1.0);
        assert!(output.px[1][2] > 0.999);
        doc.layers[0].content = LayerContent::Fill(Fill::Solid(photocraft_color::Color::rgb(0.0, 1.0, 0.0)));
        let edited = photocraft_compose::flatten(&doc);
        assert!(edited.px[0][1] > 0.49 && edited.px[0][2] < 1e-5);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).expect("native save");
        let reopened = photocraft_format::load_from_bytes(&saved).expect("native reopen");
        assert_eq!(photocraft_compose::flatten(&reopened).px, edited.px);
    }
    #[test]
    fn alpha_only_zero_and_empty_component_masks_have_concrete_native_counterparts() {
        assert!(run("gimp:mask-components", vec![int("mask", 0)]).expect("no selected components").structure.is_none());
        for alpha in [0.0, 0.25, 1.0] {
            let m = run("gimp:mask-components", vec![int("mask", 8), Argument { name: "alpha", value: ArgumentValue::Float(alpha) }])
                .expect("absolute native alpha");
            assert_eq!(m.filters[0].command, "filter.other.setAlpha");
            assert_eq!(m.filters[0].params["value"], alpha);
            assert!(m.structure.is_none());
        }
        let m = run("gimp:flood", vec![]).expect("ordinary native topology");
        assert_eq!(m.filters[0].command, "filter.other.fillHoles");
        assert_eq!(m.filters[0].params, json!({"connectivity":4,"channel":"each"}));
        assert!(m.notes[0].contains("approximation"));
    }
    #[test]
    fn hostile_core_inputs_are_rejected_without_allocating_source_sizes() {
        for (operation, args) in [
            ("gimp:grow", vec![int("radius-x", 2343)]),
            ("gimp:border", vec![int("radius-y", 0)]),
            ("gimp:grow", vec![int("radius-x", 1), int("radius-x", 2)]),
            ("gimp:compose-crop", vec![int("width", -1)]),
            ("gimp:normal", vec![Argument { name: "opacity", value: ArgumentValue::Float(f32::NAN) }]),
            ("gimp:mask-components", vec![int("mask", 16)]),
        ] {
            assert!(run(operation, args).is_err());
        }
    }

    fn stored_resource(operation: &str) -> Vec<u8> {
        use crate::xcf::tests::{W, hierarchy};
        let mut w = W::new(26);
        for value in [1, 1, 0, 150] {
            w.u32(value);
        }
        w.prop(17, &[0]);
        w.end();
        let layer = w.slot();
        w.ptr(0);
        w.ptr(0);
        w.ptr(0);
        let at = w.out.len();
        w.patch(layer, at);
        for value in [1, 1, 1] {
            w.u32(value);
        }
        w.string("Usable source");
        w.end();
        let pixels = w.slot();
        w.ptr(0);
        let effect = w.slot();
        w.ptr(0);
        let h = hierarchy(&mut w, 1, 1, 4, &[20, 40, 60, 255], crate::xcf::Compression::None);
        w.patch(pixels, h);
        let at = w.out.len();
        w.patch(effect, at);
        w.string("Inert missing resource");
        w.string("untrusted-icon");
        w.string(operation);
        w.string("0:0");
        w.prop(9001, &[3, 5, 7]);
        w.end();
        w.ptr(0);
        w.out
    }
    #[test]
    fn missing_auxiliary_import_retains_raw_data_and_usable_pixels_through_native_reopen() {
        use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
        for operation in ["gimp:addition-legacy", "gimp:compose-crop", "gimp:buffer-source-validate", "gimp:profile-transform"] {
            let bytes = stored_resource(operation);
            let imported = crate::import("core.xcf", &bytes).expect("usable XCF input");
            assert_eq!(
                imported.document.layers[0].surface().expect("unmodified native source").pixel(0, 0),
                vec![20.0 / 255.0, 40.0 / 255.0, 60.0 / 255.0, 1.0]
            );
            let Some(ImportConversion::Report(report)) = &imported.document.metadata.import_conversion else { panic!("current report") };
            assert!(report.entries.iter().any(|entry| entry.reason == ConversionReason::ResourceUnavailable
                && entry.disposition == ConversionDisposition::RetainedUnapplied
                && entry.source_description.contains(operation)));
            assert!(report.attachments.iter().any(|attachment| attachment.bytes.windows(operation.len()).any(|window| window == operation.as_bytes())));
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).expect("native save");
            let reopened = photocraft_format::load_from_bytes(&saved).expect("native reopen");
            assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        }
    }
}
