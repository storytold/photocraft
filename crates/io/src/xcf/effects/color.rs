//! Frozen B03: O013, O016, O023, O026, O053, O071, O099, O100,
//! O101, O125, O139, O140, O144 and O160. Native commands and math only.

use photocraft_color::ColorMode;
use serde_json::{Value, json};

use super::{ArgumentValue, ConfigExpr, MappingContext, MappingFailure, MappingResult, NativeMapping, StoredEffect, native_filter};

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    let fields: &[&str] = match effect.operation {
        "gegl:c2g" => &["radius", "samples", "iterations", "enhance-shadows"],
        "gegl:channel-mixer" => &["preserve-luminosity", "rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain"],
        "gegl:color-temperature" => &["original-temperature", "intended-temperature"],
        "gegl:component-extract" => &["component", "invert", "linear"],
        "gegl:hue-chroma" => &["hue", "chroma", "lightness"],
        "gegl:mono-mixer" => &["preserve-luminosity", "red", "green", "blue"],
        "gegl:saturation" => &["scale", "colorspace"],
        "gegl:sepia" => &["scale", "srgb"],
        "gegl:shadows-highlights" => &["shadows", "highlights", "whitepoint", "radius", "compress", "shadows-ccorrect", "highlights-ccorrect"],
        "gegl:vibrance" => &["vibrance", "saturation"],
        "gimp:color-balance" | "gimp:hue-saturation" => &["config"],
        "gimp:colorize" => &["color", "hue", "saturation", "lightness"],
        "gimp:desaturate" => &["mode"],
        _ => return None,
    };
    let mut seen = std::collections::HashSet::new();
    for argument in &effect.arguments {
        if !seen.insert(argument.name) {
            return Some(Err(format!("Duplicate source color-effect argument {}", argument.name).into()));
        }
        if !fields.contains(&argument.name) {
            return Some(Err(format!("Unmapped source color-effect argument {}", argument.name).into()));
        }
    }
    Some(match effect.operation {
        "gegl:c2g" => c2g(effect, context),
        "gegl:channel-mixer" => channel_mixer(effect, context),
        "gegl:color-temperature" => temperature(effect, context),
        "gegl:component-extract" => component(effect, context),
        "gegl:hue-chroma" => hue_chroma(effect, context),
        "gegl:mono-mixer" => mono_mixer(effect, context),
        "gegl:saturation" => saturation(effect),
        "gegl:sepia" => sepia(effect),
        "gegl:shadows-highlights" => shadows_highlights(effect),
        "gegl:vibrance" => vibrance(effect),
        "gimp:color-balance" => color_balance(effect),
        "gimp:colorize" => colorize(effect),
        "gimp:desaturate" => desaturate(effect, context),
        "gimp:hue-saturation" => hue_saturation(effect),
        _ => return None,
    })
}

fn one(command: &str, params: Value, note: impl Into<String>) -> NativeMapping {
    NativeMapping { filters: vec![native_filter(command, params)], notes: vec![note.into()], ..Default::default() }
}

fn number(effect: &StoredEffect<'_>, key: &str, default: f64, low: f64, high: f64) -> Result<f64, MappingFailure> {
    let value = effect.number(key)?.unwrap_or(default);
    if !value.is_finite() || !(low..=high).contains(&value) {
        return Err(format!("{key} must be finite in {low}..{high}; source configuration retained unapplied").into());
    }
    Ok(value)
}

fn boolean(effect: &StoredEffect<'_>, key: &str, default: bool) -> Result<bool, MappingFailure> {
    Ok(effect.boolean(key)?.unwrap_or(default))
}

fn channel_mixer(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    let mut rows = [[0.0; 4]; 3];
    for ((row, names), diagonal) in
        rows.iter_mut().zip([["rr-gain", "rg-gain", "rb-gain"], ["gr-gain", "gg-gain", "gb-gain"], ["br-gain", "bg-gain", "bb-gain"]]).zip(0usize..)
    {
        for (column, name) in names.iter().enumerate() {
            if let Some(slot) = row.get_mut(column) {
                *slot = number(effect, name, if column == diagonal { 1.0 } else { 0.0 }, -2.0, 2.0)?;
            }
        }
    }
    let preserve = boolean(effect, "preserve-luminosity", false)?;
    if preserve {
        for row in &mut rows {
            normalize_mixer_row(row);
        }
    }
    if rows.iter().flatten().any(|v| v.abs() > 2.0) {
        return mixer_lookup(rows, false, context);
    }
    let [red, green, blue] = rows.map(|row| row.map(|v| v * 100.0));
    Ok(one(
        "image.adjustments.channelMixer",
        json!({"red":red,"green":green,"blue":blue,"monochrome":false}),
        if preserve {
            "Preserve-luminosity becomes standard neutral-preserving row normalization (signed sum; zero-sum rows use L1 normalization). This is an explicit native approximation, not source normalization equivalence; native document math apply."
        } else {
            "Source coefficient fractions become native Channel Mixer percentages; native document color interpretation and clipping apply."
        },
    ))
}

fn mono_row(row: [f64; 4], note: impl Into<String>) -> NativeMapping {
    one("image.adjustments.channelMixer", json!({"monochrome":true,"gray":row}), note)
}

fn mono_mixer(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    let mut row = [number(effect, "red", 0.333, -5.0, 5.0)?, number(effect, "green", 0.333, -5.0, 5.0)?, number(effect, "blue", 0.333, -5.0, 5.0)?, 0.0];
    let preserve = boolean(effect, "preserve-luminosity", false)?;
    if preserve {
        normalize_mixer_row(&mut row);
    }
    if row.iter().any(|v| v.abs() > 2.0) {
        return mixer_lookup([row; 3], true, context);
    }
    Ok(mono_row(
        row.map(|v| v * 100.0),
        if preserve {
            "Preserve-luminosity becomes standard neutral-preserving coefficient normalization (signed sum; zero-sum uses L1). This declared native approximation does not claim source normalization equivalence."
        } else {
            "Weighted source monochrome coefficients become native Channel Mixer percentages with zero constant; native document color rules apply."
        },
    ))
}

fn normalize_mixer_row(row: &mut [f64; 4]) {
    let sum: f64 = row.iter().take(3).sum();
    let divisor = if sum.abs() > 1e-8 { sum } else { row.iter().take(3).map(|v| v.abs()).sum() };
    if divisor != 0.0 {
        for value in row.iter_mut().take(3) {
            *value /= divisor;
        }
    }
}

fn mixer_lookup(rows: [[f64; 4]; 3], monochrome: bool, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    native_lookup(
        context,
        "Native extended channel mixer",
        vec![format!(
            "Source coefficient domain exceeds editable native Channel Mixer bounds; a bounded native Color Lookup stores the standard matrix instead (monochrome={monochrome}). Preserve normalization, when requested, is standard neutral-preserving signed-sum/L1 normalization, not source algorithm equivalence. Native CMS, interpolation and gamut/HDR bounding apply."
        )],
        |rgb| rows.map(|row| (row[0] * f64::from(rgb[0]) + row[1] * f64::from(rgb[1]) + row[2] * f64::from(rgb[2]) + row[3]) as f32),
    )
}

fn component(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    let component = effect.enum_value("component")?.unwrap_or(0);
    let invert = boolean(effect, "invert", false)?;
    let linear = boolean(effect, "linear", false)?;
    let mut mapping = match component {
        0..=2 => {
            let mut row = [0.0; 4];
            let index = usize::try_from(component).map_err(|_| MappingFailure::from("invalid RGB component"))?;
            if let Some(slot) = row.get_mut(index) {
                *slot = if invert { -100.0 } else { 100.0 };
            }
            row[3] = if invert { 100.0 } else { 0.0 };
            mono_row(row, "Native monochrome Channel Mixer extracts the declared RGB component, with inversion as an explicit matrix constant.")
        }
        3..=7 => {
            let (model, index) = match component {
                3 => ("hsb", 0),
                4 => ("hsb", 1),
                5 => ("hsb", 2),
                6 => ("hsl", 1),
                _ => ("hsl", 2),
            };
            component_chain(model, index, invert, context)?
        }
        8..=19 => native_component_lookup(component, invert, context)?,
        20 => {
            let mut filters = Vec::new();
            if invert {
                let values: Vec<f64> = (0..256).map(|i| 1. - f64::from(i) / 255.).collect();
                filters.push(native_filter("filter.other.alphaCurve", json!({"values":values})));
            }
            filters.push(native_filter("filter.other.alphaToColor", json!({})));
            NativeMapping{filters,notes:vec!["Source alpha extraction becomes ordinary native Alpha to Color: clamped coverage occupies the document model's neutral stored channels and output is opaque. Inversion, when requested, is an ordinary native alpha transfer before extraction, preserving neutral output in every model. Native depth/color encoding and opaque coverage policy apply; no RGB-only LUT or baked source alpha snapshot is substituted.".into()],..Default::default()}
        }
        _ => return Err("Unknown source extraction component".into()),
    };
    if linear != context.linear {
        mapping.notes.push(
            "Source component linear/nonlinear interpretation differs from native document interpretation; native component math remains authoritative.".into(),
        );
    }
    Ok(mapping)
}

fn component_chain(model: &str, index: usize, invert: bool, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    let mut row = [0.0; 4];
    if context.mode == ColorMode::Grayscale {
        // Gray has hue/saturation zero and value/lightness equal to its RGB display value.
        if index == 2 {
            row[0] = if invert { -100.0 } else { 100.0 };
        }
        row[3] = if invert { 100.0 } else { 0.0 };
        return Ok(mono_row(row, "Native gray component extraction; hue/saturation become zero and value/lightness preserve native gray."));
    }
    if context.mode != ColorMode::Rgb {
        return Err("HSB/HSL component chain currently requires native RGB; other native model conversion remains unimplemented".into());
    }
    if let Some(slot) = row.get_mut(index) {
        *slot = if invert { -100.0 } else { 100.0 };
    }
    row[3] = if invert { 100.0 } else { 0.0 };
    let mut mapping = mono_row(
        row,
        "Native HSB/HSL conversion followed by explicit component extraction; PhotoCraft's bounded component conversion and native color math apply.",
    );
    mapping.filters.insert(0, native_filter("filter.other.hsbHsl", json!({"inputMode":"rgb","rowOrder":model})));
    Ok(mapping)
}

fn native_component_lookup(component: i32, invert: bool, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    native_lookup(
        context,
        "Native color component extraction",
        vec![format!(
            "Source component {component} becomes a native color-component lookup: PhotoCraft's existing CMS CMYK and D50 Lab, standard polar LCh, or full-range BT.601 YCbCr. Native Lab uses L/100 and (a,b+128)/255; LCh chroma uses the native a/b channel diagonal 128*sqrt(2), and hue/360. These explicit native encodings, profile conversion, 17-cube interpolation, gamut/HDR bounds and retained native alpha are approximations, not source extraction equivalence."
        )],
        |rgb| {
            use photocraft_color::convert;
            let value = match component {
                8..=11 => convert::rgb_to_cmyk(rgb).get(usize::try_from(component - 8).unwrap_or(0)).copied().unwrap_or(0.0),
                // Public BT.601 normalized color-difference equations, §2.5.2:
                // https://www.itu.int/rec/r-rec-bt.601/
                12 => convert::rgb_to_gray(rgb),
                13 => (rgb[2] - convert::rgb_to_gray(rgb)) / 1.772 + 0.5,
                14 => (rgb[0] - convert::rgb_to_gray(rgb)) / 1.402 + 0.5,
                _ => {
                    let [l, a, b] = convert::srgb_to_lab(rgb);
                    match component {
                        15 => l / 100.0,
                        16 => (a + 128.0) / 255.0,
                        17 => (b + 128.0) / 255.0,
                        18 => a.hypot(b) / (128.0 * 2.0f32.sqrt()),
                        19 => b.atan2(a).to_degrees().rem_euclid(360.0) / 360.0,
                        _ => 0.0,
                    }
                }
            };
            [if invert { 1.0 - value } else { value }; 3]
        },
    )
}

fn native_saturation_percent(scale: f64) -> f64 {
    // Existing native Hue/Saturation divides positive saturation by 1-s.
    // This maps a requested multiplicative gain without the +100% singularity.
    if scale > 1.0 { (1.0 - 1.0 / scale) * 100.0 } else { (scale - 1.0) * 100.0 }
}

fn saturation(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    let scale = number(effect, "scale", 1.0, 0.0, 10.0)?;
    let model = effect.enum_value("colorspace")?.unwrap_or(0);
    if !(0..=2).contains(&model) {
        return Err("Unknown source saturation color model".into());
    }
    Ok(one(
        "image.adjustments.hueSaturation",
        json!({"hue":0,"saturation":native_saturation_percent(scale),"lightness":0,"colorize":false}),
        format!(
            "Source scale 0..10 becomes native HSL gain: reductions use 100*(scale-1), increases use 100*(1-1/scale), because positive native saturation divides by 1-s. Source color model {model} becomes native HSL with an explicit component-model approximation; native float-depth quantization and clipping apply."
        ),
    ))
}

fn vibrance(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    let value = number(effect, "vibrance", 0.0, -100.0, 100.0)?;
    let scale = number(effect, "saturation", 1.0, 0.0, 10.0)?;
    let first = scale.min(2.0);
    let mut mapping = one(
        "image.adjustments.vibrance",
        json!({"vibrance":value,"saturation":(first-1.0)*100.0}),
        "Source vibrance percentage and neutral-relative saturation become native Vibrance controls; native selective-saturation/color rules are an algorithm approximation. Source saturation above2 is factored into at most four native passes with gain<=2, applying vibrance once. Native affine saturation preserves its gray axis between passes; intermediate clipping and transfer rounding remain explicit native differences.",
    );
    if scale > 2.0 {
        let mut remaining = scale / 2.0;
        // Source maximum10 needs at most three additional factors, never input-sized work.
        for _ in 0..3 {
            if remaining <= 1.0 {
                break;
            }
            let factor = remaining.min(2.0);
            mapping.filters.push(native_filter("image.adjustments.vibrance", json!({"vibrance":0,"saturation":(factor-1.0)*100.0})));
            remaining /= factor;
        }
    }
    Ok(mapping)
}

fn sepia(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    let scale = number(effect, "scale", 1.0, 0.0, 1.0)?;
    let srgb = boolean(effect, "srgb", true)?;
    // Existing native Photo Filter sepia preset (adjust_params::PHOTO_FILTERS), not GEGL's matrix.
    Ok(one(
        "image.adjustments.photoFilter",
        json!({"color":[172.0/255.0,122.0/255.0,51.0/255.0],"density":scale*100.0,"preserveLuminosity":true}),
        format!(
            "Source sepia strength becomes editable native Photo Filter Sepia density. This deliberately uses PhotoCraft's existing sepia preset and luminosity rule, not source matrix equivalence; source srgb={srgb} interpretation is retained as history."
        ),
    ))
}

fn shadows_highlights(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    // Independent stock GIMP probes establish directions, not response equivalence.
    let shadows = number(effect, "shadows", 0.0, -100.0, 100.0)?;
    let highlights = number(effect, "highlights", 0.0, -100.0, 100.0)?;
    let radius = number(effect, "radius", 100.0, 0.1, 1500.0)?;
    let compress = number(effect, "compress", 50.0, 0.0, 100.0)?;
    let whitepoint = number(effect, "whitepoint", 0.0, -10.0, 10.0)?;
    let shadow_color = number(effect, "shadows-ccorrect", 100.0, 0.0, 100.0)?;
    let highlight_color = number(effect, "highlights-ccorrect", 50.0, 0.0, 100.0)?;
    let remaining = 1.0 - compress / 100.0;
    let tone = 100.0 - compress;
    let parameters = |shadow: f64, highlight: f64, color: f64| {
        json!({
            "shadowAmount":shadow*remaining,"highlightAmount":highlight*remaining,
            "shadowRadius":radius,"highlightRadius":radius,"shadowTone":tone,"highlightTone":tone,
            "color":color,"midtone":0,"blackClip":0,"whiteClip":0
        })
    };
    let mut mapping = one(
        "image.adjustments.shadowsHighlights",
        parameters(shadows.max(0.0), 0.0, shadow_color),
        "Source tonal controls become explicit ordinary native operations: separate Shadows/Highlights passes preserve separate color-correction percentages and the same pixel-radius unit. Compression becomes complementary native tonal width and remaining strength (100-compress); this monotonic native approximation is identity at compress100, not source response equivalence. Native Color uses the same source percentage domain and observed increasing-chroma direction, with native color math authoritative.",
    );
    mapping.filters.push(native_filter("image.adjustments.shadowsHighlights", parameters(0.0, (-highlights).max(0.0), highlight_color)));
    if shadows < 0.0 || highlights > 0.0 {
        mapping.filters.push(native_filter(
            "image.adjustments.curves",
            json!({"points":[
                [0.0,0.0],[63.75,63.75*(1.0+shadows.min(0.0)*remaining/100.0)],
                [127.5,127.5],[191.25,191.25+63.75*highlights.max(0.0)*remaining/100.0],[255.0,255.0]
            ]}),
        ));
        mapping.notes.push("Opposite-sign controls use an ordinary native Curves approximation, preserving black/mid/white with quarter-tone offsets proportional to source strength and complementary compression. Native spline interpolation and local-vs-global differences apply.".into());
    }
    if whitepoint != 0.0 {
        mapping.filters.push(native_filter("image.adjustments.exposure", json!({"exposure":(1.0+whitepoint/100.0).log2(),"offset":0,"gamma":1})));
        mapping.notes.push("Source white-point percentage becomes a native Exposure gain 1+whitepoint/100 expressed in EV log2(gain). Its observed brightening/darkening direction is preserved; this standard native gain is an explicit approximation, not a calibrated source white-point response.".into());
    }
    Ok(mapping)
}

pub(super) fn native_lookup(
    context: &MappingContext,
    title: &str,
    notes: Vec<String>,
    operation: impl Fn([f32; 3]) -> [f32; 3],
) -> Result<NativeMapping, MappingFailure> {
    use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
    let gray = context.mode == ColorMode::Grayscale;
    if !matches!(context.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Native color lookup currently requires RGB or Gray document interpretation".into());
    }
    let fallback = if gray {
        if context.linear { photocraft_cms::builtin::linear_gray().clone() } else { Builtin::SGray.profile().clone() }
    } else if context.linear {
        Builtin::LinearSrgb.profile().clone()
    } else {
        Builtin::Srgb.profile().clone()
    };
    let profile = match context.target_profile.as_deref() {
        Some(bytes) => Profile::parse(bytes).map_err(|error| format!("Native lookup target profile: {error}"))?,
        None => fallback,
    };
    if profile.color_space != if gray { ColorSpace::Gray } else { ColorSpace::Rgb } {
        return Err("Native lookup target profile/model mismatch".into());
    }
    let to_rgb =
        Transform::new(&profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).map_err(|error| format!("Native lookup input CMS: {error}"))?;
    let from_rgb =
        Transform::new(Builtin::Srgb.profile(), &profile, Intent::RelativeColorimetric, false).map_err(|error| format!("Native lookup output CMS: {error}"))?;
    let channels = if gray { 1 } else { 3 };
    super::resources::lookup(17, title, notes, |grid| {
        let samples = grid.map(|v| v as f32);
        let mut rgb = [0.0f32; 3];
        to_rgb.convert_f32(&samples[..channels], channels, &mut rgb, 3, false);
        let adjusted = operation(rgb);
        let mut native = [0.0f32; 3];
        from_rgb.convert_f32(&adjusted, 3, &mut native[..channels], channels, false);
        if gray { [f64::from(native[0]); 3] } else { native.map(f64::from) }
    })
    .map_err(Into::into)
}

fn hue_chroma(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    let hue = number(effect, "hue", 0.0, -180.0, 180.0)? as f32;
    let chroma = number(effect, "chroma", 0.0, -100.0, 100.0)? as f32;
    let lightness = number(effect, "lightness", 0.0, -100.0, 100.0)? as f32;
    native_lookup(context,"Native CIE LCh conversion",vec![
        "Source hue degrees, absolute C* delta and absolute L* delta become a bounded 17-cube native Color Lookup. The table uses only PhotoCraft's existing native CMS/Lab conversion and standard polar geometry; no source renderer or source image is sampled.".into(),
        "Assigned native target profile/model is converted to native sRGB/Lab and back. Native tetrahedral interpolation, bounded 0..1 lookup/HDR behavior and native Lab gamut clipping are explicit approximations; original source controls remain historical data, not runtime rules.".into(),
    ],|rgb|{
        let [l,a,b]=photocraft_color::convert::srgb_to_lab(rgb);
        let c=a.hypot(b);let angle=b.atan2(a)+hue.to_radians();let c=(c+chroma).max(0.0);
        photocraft_color::convert::lab_to_srgb([l+lightness,c*angle.cos(),c*angle.sin()])
    })
}

/// Independent numerical Planck spectrum, using the analytic CIE1931 observer in
/// Wyman/Sloan/Shirley (2013), Eq4/Table1: https://jcgt.org/published/0002/02/01/.
/// Only published mathematical coefficients are used, not the paper's sample code.
/// h,c,k are exact SI constants: https://physics.nist.gov/cuu/Constants/.
fn temperature_white(kelvin: f64) -> Result<[f64; 3], MappingFailure> {
    if !kelvin.is_finite() || !(1000.0..=12000.0).contains(&kelvin) {
        return Err("Color temperature is outside 1000..12000 kelvin".into());
    }
    const X: &[[f64; 4]] = &[[0.362, 442.0, 0.0624, 0.0374], [1.056, 599.8, 0.0264, 0.0323], [-0.065, 501.1, 0.0490, 0.0382]];
    const Y: &[[f64; 4]] = &[[0.821, 568.8, 0.0213, 0.0247], [0.286, 530.9, 0.0613, 0.0322]];
    const Z: &[[f64; 4]] = &[[1.217, 437.0, 0.0845, 0.0278], [0.681, 459.0, 0.0385, 0.0725]];
    let mut xyz = [0.0; 3];
    // Fixed 5nm quadrature; common radiance constants and wavelength step cancel at Y=1.
    for nm in (360..=830).step_by(5) {
        let wavelength = f64::from(nm);
        let micrometers = wavelength * 0.001;
        let c2 = 6.626_070_15e-34 * 299_792_458.0 / 1.380_649e-23;
        let radiance = micrometers.powi(-5) / (c2 / (wavelength * 1e-9 * kelvin)).exp_m1();
        for (component, lobes) in xyz.iter_mut().zip([X, Y, Z]) {
            let matching: f64 = lobes
                .iter()
                .map(|&[amplitude, center, left, right]| {
                    let distance = (wavelength - center) * if wavelength < center { left } else { right };
                    amplitude * (-0.5 * distance * distance).exp()
                })
                .sum();
            *component += radiance * matching;
        }
    }
    let y = xyz[1];
    if !y.is_finite() || y <= 0.0 || xyz.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err("Native spectral white-point approximation produced invalid XYZ".into());
    }
    Ok(xyz.map(|v| v / y))
}

fn temperature(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    use photocraft_cms::math;
    let original = number(effect, "original-temperature", 6500.0, 1000.0, 12000.0)?;
    let intended = number(effect, "intended-temperature", 6500.0, 1000.0, 12000.0)?;
    if original == intended {
        return Ok(one(
            "image.adjustments.channelMixer",
            json!({"red":[100,0,0,0],"green":[0,100,0,0],"blue":[0,0,100,0],"monochrome":false}),
            "Equal original/intended white points are the identity operation; no native lookup interpolation or HDR clipping is added.",
        ));
    }
    let from = temperature_white(original)?;
    let to = temperature_white(intended)?;
    let d50 = photocraft_color::convert::D50.map(f64::from);
    let d65 = math::xy_to_xyz(math::D65_XY);
    let adaptation = math::mul(&math::bradford(d65, d50), &math::mul(&math::bradford(to, from), &math::bradford(d50, d65)));
    if adaptation.iter().flatten().any(|v| !v.is_finite()) {
        return Err("Native white-point adaptation is nonfinite".into());
    }
    native_lookup(context,"Native temperature adaptation",vec![
        format!("Source original={original}K and intended={intended}K become native white-balance Bradford adaptation from intended white to original white in a bounded editable 17-cube Color Lookup. It uses published Planck/CIE mathematics and existing PhotoCraft CMS matrices, never a source renderer or source pixel samples."),
        "The analytic observer fit, fixed 5nm spectral quadrature, Bradford model, native assigned-profile roundtrip, tetrahedral interpolation and native bounded gamut/HDR domain are explicit algorithm approximations; source RGB-gain equivalence is not claimed.".into(),
    ],|rgb|{
        let linear=rgb.map(photocraft_color::convert::srgb_to_linear);
        let xyz=photocraft_color::convert::mat3_mul(&photocraft_color::convert::SRGB_TO_XYZ_D50,linear).map(f64::from);
        let adjusted=math::apply(&adaptation,xyz).map(|v|v as f32);
        photocraft_color::convert::mat3_mul(&photocraft_color::convert::XYZ_D50_TO_SRGB,adjusted).map(photocraft_color::convert::linear_to_srgb)
    })
}

fn c2g(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    use super::{NativeBranch, NativeStructure};
    let radius = number(effect, "radius", 300., 2., 6000.)?;
    let samples = number(effect, "samples", 4., 1., 1000.)?;
    let iterations = number(effect, "iterations", 10., 1., 1000.)?;
    if [radius, samples, iterations].iter().any(|value| value.fract() != 0.) {
        return Err("C2G radius/samples/iterations must be integral source controls".into());
    }
    let shadows = boolean(effect, "enhance-shadows", false)?;
    let sigma = radius / 3.;
    let gray = native_filter("image.adjustments.blackWhite", json!({"reds":40,"yellows":60,"greens":40,"cyans":60,"blues":20,"magentas":80}));
    let mut mapping = if sigma <= 1000. {
        super::spatial::gaussian_workspace(context, sigma, 6)?;
        NativeMapping {
            filters: vec![gray, native_filter("filter.sharpen.unsharpMask", json!({"amount":50.,"radius":sigma,"threshold":0.}))],
            ..Default::default()
        }
    } else {
        let count = (sigma / 1000.).powi(2).ceil() as usize;
        let pass_sigma = sigma / (count as f64).sqrt();
        super::spatial::gaussian_workspace(context, pass_sigma, 6)?;
        let bytes = (count + 16) * 2048 + 8192;
        if bytes > context.remaining_metadata_bytes || bytes > context.remaining_generation_bytes {
            return Err(MappingFailure::resource("Native C2G contrast branches exceed remaining metadata/work allowance"));
        }
        let mut blur = vec![gray.clone()];
        for _ in 0..count {
            context.interrupt.check().map_err(|_| MappingFailure::resource("Native C2G contrast generation cancelled"))?;
            blur.push(native_filter("filter.blur.gaussianBlur", json!({"radius":pass_sigma})));
        }
        blur.push(native_filter("image.adjustments.exposure", json!({"exposure":-1.,"offset":0.,"gamma":1.})));
        NativeMapping {
            structure: Some(NativeStructure::Composite {
                branches: vec![
                    NativeBranch {
                        filters: vec![gray, native_filter("image.adjustments.exposure", json!({"exposure":1.5_f64.log2(),"offset":0.,"gamma":1.}))],
                        ..Default::default()
                    },
                    NativeBranch { filters: blur, blend: photocraft_color::BlendMode::Subtract, clipped: true, ..Default::default() },
                ],
            }),
            ..Default::default()
        }
    };
    if shadows {
        let defaults = photocraft_algo::tone::ShadowsHighlights::default();
        mapping.filters.push(native_filter("image.adjustments.shadowsHighlights",json!({"shadowAmount":defaults.shadow_amount,"shadowTone":defaults.shadow_tone,"shadowRadius":sigma,"highlightAmount":0.,"color":0.,"midtone":0.,"blackClip":0.,"whiteClip":0.})));
    }
    mapping.notes.push(format!("Source color-to-gray intent becomes ordinary native Black & White plus local contrast at native Unsharp default 50 percent. The source pixel radius defines a 3-sigma Gaussian neighborhood; radii above 3000 use variance-split native Gaussian/Exposure/clipped-Subtract branches rather than a silently capped dialog. Native hue weights, Gaussian boundaries, gain transfer, negative/HDR clipping and native alpha govern edits. enhance-shadows={shadows} selects native Shadows/Highlights default shadow amount/tone with the same native neighborhood and neutral color/clipping controls. Source samples={samples},iterations={iterations} are superseded stochastic sampling-quality provenance: deterministic native convolution has no Monte Carlo quality controls, so they intentionally do not change native pixels and are not missing mappings. Source stochastic extrema/color-distance statistics are approximated, not executed"));
    Ok(mapping)
}

fn desaturate(effect: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping, MappingFailure> {
    match effect.enum_value("mode")?.unwrap_or(3) {
        0 => component_chain("hsl", 2, false, context),
        4 => component_chain("hsb", 2, false, context),
        2 => Ok(mono_row(
            [100.0 / 3.0, 100.0 / 3.0, 100.0 / 3.0, 0.0],
            "Source arithmetic-average desaturation becomes native equal-weight monochrome Channel Mixer.",
        )),
        1 | 3 => Ok(mono_row(
            [21.26, 71.52, 7.22, 0.0],
            "Source luma/luminance desaturation becomes native Rec.709-weighted monochrome Channel Mixer. Native document encoding/profile rules apply; linear luminance and assigned-profile weighting differences are explicit approximations.",
        )),
        _ => Err("Unknown source desaturation mode".into()),
    }
}

fn scalar<'a, 'b>(tail: &'b [ConfigExpr<'a>], name: &str) -> Result<&'b ConfigExpr<'a>, MappingFailure> {
    match tail {
        [value] => Ok(value),
        _ => Err(format!("{name} config field must contain exactly one value").into()),
    }
}
fn config_number(value: &ConfigExpr<'_>, name: &str, low: f64, high: f64) -> Result<f64, MappingFailure> {
    value.as_number().filter(|n| n.is_finite() && (low..=high).contains(n)).ok_or_else(|| format!("{name} configuration value outside {low}..{high}").into())
}
fn config_bool(value: &ConfigExpr<'_>) -> Result<bool, MappingFailure> {
    match value.as_symbol() {
        Some("yes" | "true") => Ok(true),
        Some("no" | "false") => Ok(false),
        _ => Err("Invalid source boolean configuration value".into()),
    }
}
fn base_config(name: &str) -> bool {
    matches!(
        name,
        "time" | "gimp-clipping" | "gimp-region" | "gimp-mode" | "gimp-opacity" | "gimp-blend-space" | "gimp-composite-space" | "gimp-composite-mode"
    )
}

fn color_balance(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    let config = effect.config("config")?.ok_or_else(|| MappingFailure::from("Missing color-balance config"))?;
    let mut tones = [[0.0; 3]; 3];
    let mut selected = 1usize;
    let mut preserve = true;
    for form in &config.forms {
        let list = form.as_list().ok_or_else(|| MappingFailure::from("Invalid color-balance config form"))?;
        let name = form.head().ok_or_else(|| MappingFailure::from("Unnamed color-balance field"))?;
        if base_config(name) {
            continue;
        }
        let value = scalar(list.get(1..).unwrap_or(&[]), name)?;
        match name {
            "range" => {
                selected = match value.as_symbol() {
                    Some("shadows") => 0,
                    Some("midtones") => 1,
                    Some("highlights") => 2,
                    _ => return Err("Unknown color-balance tonal range".into()),
                }
            }
            "preserve-luminosity" => preserve = config_bool(value)?,
            "cyan-red" | "magenta-green" | "yellow-blue" => {
                let component = match name {
                    "cyan-red" => 0,
                    "magenta-green" => 1,
                    _ => 2,
                };
                if let Some(slot) = tones.get_mut(selected).and_then(|row| row.get_mut(component)) {
                    *slot = config_number(value, name, -1.0, 1.0)? * 100.0;
                }
            }
            _ => return Err(format!("Unmapped source color-balance config field {name}").into()),
        }
    }
    let [shadows, midtones, highlights] = tones;
    Ok(one(
        "image.adjustments.colorBalance",
        json!({"shadows":shadows,"midtones":midtones,"highlights":highlights,"preserveLuminosity":preserve}),
        "Repeated source tonal ranges become native Color Balance percentage arrays; native tonal weighting and preserve-luminosity math remain authoritative.",
    ))
}

fn hue_saturation(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    let config = effect.config("config")?.ok_or_else(|| MappingFailure::from("Missing hue/saturation config"))?;
    let names = ["master", "reds", "yellows", "greens", "cyans", "blues", "magentas"];
    let mut values = [[0.0; 3]; 7];
    let mut selected = 0usize;
    let mut overlap = 0.0;
    for form in &config.forms {
        let list = form.as_list().ok_or_else(|| MappingFailure::from("Invalid hue/saturation config form"))?;
        let name = form.head().ok_or_else(|| MappingFailure::from("Unnamed hue/saturation field"))?;
        if base_config(name) {
            continue;
        }
        let value = scalar(list.get(1..).unwrap_or(&[]), name)?;
        match name {
            "range" => {
                selected = match value.as_symbol() {
                    Some("all") => 0,
                    Some("red") => 1,
                    Some("yellow") => 2,
                    Some("green") => 3,
                    Some("cyan") => 4,
                    Some("blue") => 5,
                    Some("magenta") => 6,
                    _ => return Err("Unknown hue/saturation source range".into()),
                }
            }
            "overlap" => overlap = config_number(value, name, 0.0, 1.0)?,
            "hue" | "saturation" | "lightness" => {
                let c = match name {
                    "hue" => 0,
                    "saturation" => 1,
                    _ => 2,
                };
                if let Some(slot) = values.get_mut(selected).and_then(|row| row.get_mut(c)) {
                    *slot = config_number(value, name, -1.0, 1.0)? * if c == 0 { 180.0 } else { 100.0 };
                }
            }
            _ => return Err(format!("Unmapped source hue/saturation config field {name}").into()),
        }
    }
    let mut params = json!({"hue":values[0][0],"saturation":values[0][1],"lightness":values[0][2],"colorize":false});
    // Source overlap is a fraction of a 60-degree sector. Translate its
    // transition extents into the existing native four-slider hue ranges;
    // negative red bounds deliberately preserve the circular wrap.
    let ramp = overlap * 30.0;
    for (index, (name, row)) in names.iter().zip(values.iter()).skip(1).enumerate() {
        let center = index as f64 * 60.0;
        let bounds = [center - 30.0 - ramp, center - 30.0 + ramp, center + 30.0 - ramp, center + 30.0 + ramp];
        params[*name] = json!({"hue":row[0],"saturation":row[1],"lightness":row[2],"range":bounds});
    }
    Ok(one(
        "image.adjustments.hueSaturation",
        params,
        format!(
            "Source master and six hue sectors become native degree/percentage controls. Source overlap={overlap} becomes editable native circular hue-range limits and linear transition widths. Native hue/saturation/lightness composition and color-model rules remain authoritative; source rendering is approximated."
        ),
    ))
}

fn colorize(effect: &StoredEffect<'_>) -> Result<NativeMapping, MappingFailure> {
    let complete = ["hue", "saturation", "lightness"].iter().all(|key| effect.arguments.iter().any(|arg| arg.name == *key));
    if let Some(color) = effect.argument("color")? {
        match color {
            ArgumentValue::Color(Some(_)) | ArgumentValue::LiteralColor { .. } if !complete => {
                let mut rgba = super::resources::color(effect, "color")?;
                if rgba.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&rgba[3]) {
                    return Err("Color-only Colorize needs a finite native sRGB literal with normalized alpha".into());
                }
                // Independent stock save/reopen canonicalizes this setter's
                // alpha to one without changing its H/S/L controls. It is color
                // provenance, not an instruction to replace owner coverage.
                rgba[3] = 1.0;
                // Reuse PhotoCraft's actual HSL conversion instead of a second color algorithm.
                let area = photocraft_geom::Rect::new(0, 0, 1, 1);
                let mut pixel = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, photocraft_color::SampleType::F32, true));
                pixel.write_region(area, &rgba);
                let converted = photocraft_algo::apply(
                    &pixel,
                    &photocraft_algo::FilterParams::HsbHsl { input: photocraft_algo::HsbModel::Rgb, output: photocraft_algo::HsbModel::Hsl },
                    area,
                    area,
                    None,
                );
                let values = converted.read_region(area);
                let (h, s, l) = match values.as_slice() {
                    [h, s, l, _] => (*h, *s, *l),
                    _ => return Err("Native literal HSL conversion returned an invalid channel count".into()),
                };
                return Ok(one(
                    "image.adjustments.hueSaturation",
                    json!({"hue":f64::from(h)*360.0,"saturation":f64::from(s)*100.0,"lightness":(f64::from(l)*2.0-1.0)*100.0,"colorize":true}),
                    "Source Colorize color alias is interpreted through its own profile and native CMS/HSL conversion, then translated into one editable native Colorize adjustment. Native luminance/colorization rules and the existing bounded HSL RGB domain apply, including out-of-gamut literal clipping. Source literal alpha is retained as provenance but does not replace owner coverage: independent stock save/reopen canonicalizes alpha0/.25/1 to the same H/S/L controls and opaque color alias. Color and scalar aliases are never applied twice.",
                ));
            }
            ArgumentValue::Color(_) | ArgumentValue::LiteralColor { .. } => {}
            _ => return Err("Colorize color alias has an invalid stored argument type".into()),
        }
    }
    let hue = number(effect, "hue", 0.5, 0.0, 1.0)?;
    let saturation = number(effect, "saturation", 0.5, 0.0, 1.0)?;
    let lightness = number(effect, "lightness", 0.0, -1.0, 1.0)?;
    Ok(one(
        "image.adjustments.hueSaturation",
        json!({"hue":hue*360.0,"saturation":saturation*100.0,"lightness":lightness*100.0,"colorize":true}),
        "Source saved scalar aliases become native absolute hue degrees, saturation/lightness percentages and Colorize mode. Stock GIMP save/reopen canonicalizes its color setter into these scalar aliases; the redundant typed-color alias is retained and not applied twice. Native luminance/colorization behavior applies.",
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ColorLiteral, parse_config};
    use super::*;
    use photocraft_color::{BlendMode, PixelFormat, SampleType};
    use photocraft_doc::{Adjustment, Document, Layer, LayerContent, SmartObject, SmartSource};
    use photocraft_geom::{Affine, Rect, Size};
    use photocraft_raster::Surface;
    use std::sync::Arc;

    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(-3, 2, 4, 7),
            native_filter_bounds: Rect::new(0, 0, 7, 5),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn float(name: &'static str, value: f32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Float(value) }
    }
    fn mapped(operation: &'static str, args: Vec<Argument<'_>>) -> NativeMapping {
        map(&StoredEffect::test(operation, args), &context()).expect("owned B03 identity").expect("native conversion")
    }

    #[test]
    fn native_mixer_counterparts_preserve_asymmetric_rows_and_mono_units() {
        let m = mapped("gegl:channel-mixer", vec![float("rr-gain", 0.75), float("rg-gain", 0.25), float("bb-gain", -0.5)]);
        assert_eq!(m.filters[0].command, "image.adjustments.channelMixer");
        assert_eq!(m.filters[0].params, json!({"red":[75.0,25.0,0.0,0.0],"green":[0.0,100.0,0.0,0.0],"blue":[0.0,0.0,-50.0,0.0],"monochrome":false}));
        let m = mapped("gegl:mono-mixer", vec![float("red", 0.25), float("green", 0.5), float("blue", 0.25)]);
        assert_eq!(m.filters[0].params, json!({"monochrome":true,"gray":[25.0,50.0,25.0,0.0]}));
        let expected = Adjustment::ChannelMixer { matrix: [[0.25, 0.5, 0.25, 0.0], [0.0; 4], [0.0; 4]], monochrome: true };
        let mut buffer = photocraft_compose::Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.2, 0.6, 0.8, 0.375]] };
        photocraft_compose::adjust::apply(&expected, &mut buffer);
        assert!(buffer.px[0][..3].iter().all(|v| (*v - 0.55).abs() < 1e-6));
        assert_eq!(buffer.px[0][3], 0.375);
    }

    #[test]
    fn native_saturation_and_vibrance_use_neutral_relative_scales_without_clamping() {
        let m = mapped("gegl:saturation", vec![float("scale", 1.25)]);
        assert_eq!(m.filters[0].params, json!({"hue":0,"saturation":native_saturation_percent(1.25),"lightness":0,"colorize":false}));
        let m = mapped("gegl:vibrance", vec![float("vibrance", -20.0), float("saturation", 1.5)]);
        assert_eq!(m.filters[0].params, json!({"vibrance":-20.0,"saturation":50.0}));
        for value in [10.1, f32::NAN, f32::INFINITY] {
            let effect = StoredEffect::test("gegl:saturation", vec![float("scale", value)]);
            assert!(map(&effect, &context()).expect("owned").is_err());
        }
    }

    #[test]
    fn repeated_core_ranges_become_independent_native_controls() {
        let mut budget = 1 << 20;
        let config = parse_config(
            "(time 0) (range shadows) (cyan-red 0.25) (magenta-green -0.5) (range highlights) (yellow-blue 0.75) (preserve-luminosity no)",
            &mut budget,
        )
        .expect("source config");
        let m = mapped("gimp:color-balance", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        assert_eq!(m.filters[0].params, json!({"shadows":[25.0,-50.0,0.0],"midtones":[0.0,0.0,0.0],"highlights":[0.0,0.0,75.0],"preserveLuminosity":false}));
        let config = parse_config("(range all) (hue 0.25) (saturation -0.5) (range blue) (hue -0.5) (lightness 0.75) (overlap 0.2)", &mut budget)
            .expect("source config");
        let m = mapped("gimp:hue-saturation", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
        assert_eq!(m.filters[0].params["hue"], 45.0);
        assert_eq!(m.filters[0].params["saturation"], -50.0);
        assert_eq!(m.filters[0].params["blues"], json!({"hue":-90.0,"saturation":0.0,"lightness":75.0,"range":[204.0,216.0,264.0,276.0]}));
        assert_eq!(m.filters[0].params["reds"], json!({"hue":0.0,"saturation":0.0,"lightness":0.0,"range":[-36.0,-24.0,24.0,36.0]}));
        assert!(m.notes[0].contains("overlap=0.2"));
    }

    #[test]
    fn core_hue_overlap_becomes_native_circular_limits_and_persists() {
        use photocraft_doc::adjust::HueRange;
        for overlap in [0.0, 0.5, 1.0] {
            let mut budget = 1 << 20;
            let raw = format!("(range red) (hue 0.25) (overlap {overlap})");
            let config = parse_config(&raw, &mut budget).expect("config");
            let mapped = mapped("gimp:hue-saturation", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]);
            let p = &mapped.filters[0].params;
            let width = overlap as f32 * 30.0;
            let mut ranges = HueRange::defaults();
            for (index, range) in ranges.iter_mut().enumerate() {
                let c = index as f32 * 60.0;
                range.bounds = [c - 30.0 - width, c - 30.0 + width, c + 30.0 - width, c + 30.0 + width];
                let key = ["reds", "yellows", "greens", "cyans", "blues", "magentas"][index];
                assert_eq!(p[key]["range"], json!(range.bounds));
            }
            ranges[0].hue = 45.0;
            // Independent native slider behavior, including the red wrap and
            // the midpoint between adjacent sectors (source degrees / 60).
            assert_eq!(ranges[0].weight(0.0), 1.0);
            assert_eq!(ranges[0].weight(359.0), ranges[0].weight(-1.0));
            if overlap > 0.0 {
                assert!((ranges[0].weight(30.0) - 0.5).abs() < 1e-6);
                assert!((ranges[1].weight(30.0) - 0.5).abs() < 1e-6);
            }
            let independent = Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges };
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut doc = Document::new("native hue ranges", Size::new(3, 1), ColorMode::Rgb, depth);
                let mut layer = Layer::raster("native pixels", PixelFormat::new(ColorMode::Rgb, depth, true));
                let LayerContent::Raster(surface) = &mut layer.content else { panic!("raster") };
                surface.write_region(Rect::new(0, 0, 3, 1), &[1., 0., 0., 1., 1., 0.5, 0., 0.5, 1., 0., 0.1, 0.75]);
                doc.layers.push(layer);
                let mut adjustment = Layer::raster("editable hue", PixelFormat::new(ColorMode::Rgb, depth, true));
                adjustment.content = LayerContent::Adjustment(independent.clone());
                doc.layers.push(adjustment);
                let source = photocraft_format::save_to_bytes(&doc, &Default::default()).expect("native Smart source");
                let mut object =
                    SmartObject::new(SmartSource::Embedded { file_name: "native-hue-source.pcraft".into(), bytes: Arc::new(source) }, Affine::IDENTITY, None);
                object.smart_filters = mapped.filters.clone();
                let mut editable = Layer::raster("imported native filter data", PixelFormat::new(ColorMode::Rgb, depth, true));
                editable.content = LayerContent::Smart(object);
                doc.layers.push(editable);
                let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).expect("save native");
                let reopened = photocraft_format::load_from_bytes(&bytes).expect("reopen native");
                let LayerContent::Smart(object) = &reopened.layers[2].content else { panic!("persisted native filter") };
                assert_eq!(object.smart_filters[0].params, *p);
                let LayerContent::Adjustment(actual) = &reopened.layers[1].content else { panic!("native adjustment") };
                assert_eq!(actual, &independent);
                let mut expected =
                    photocraft_compose::Buffer { rect: Rect::new(0, 0, 3, 1), px: vec![[1., 0., 0., 1.], [1., 0.5, 0., 0.5], [1., 0., 0.1, 0.75]] };
                let mut actual_pixels = expected.clone();
                photocraft_compose::adjust::apply(&independent, &mut expected);
                photocraft_compose::adjust::apply(actual, &mut actual_pixels);
                assert_eq!(expected.px, actual_pixels.px);
            }
        }
        for bad in ["-0.1", "1.1"] {
            let mut budget = 1 << 20;
            let raw = format!("(overlap {bad})");
            let config = parse_config(&raw, &mut budget).expect("source config");
            assert!(
                map(&StoredEffect::test("gimp:hue-saturation", vec![Argument { name: "config", value: ArgumentValue::Config(config) }]), &context())
                    .expect("owned")
                    .is_err()
            );
        }
        assert!(parse_config("(overlap nan)", &mut (1 << 20)).is_err());
    }

    #[test]
    fn hsv_value_uses_existing_native_component_chain_and_keeps_alpha() {
        let m = mapped("gegl:component-extract", vec![Argument { name: "component", value: ArgumentValue::Enum(5) }]);
        assert_eq!(m.filters.len(), 2);
        assert_eq!(m.filters[0].command, "filter.other.hsbHsl");
        assert_eq!(m.filters[0].params, json!({"inputMode":"rgb","rowOrder":"hsb"}));
        assert_eq!(m.filters[1].params, json!({"monochrome":true,"gray":[0.0,0.0,100.0,0.0]}));
        let area = Rect::new(0, 0, 1, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.write_region(area, &[0.2, 0.6, 0.8, 0.375]);
        let converted = photocraft_algo::apply(
            &source,
            &photocraft_algo::FilterParams::HsbHsl { input: photocraft_algo::HsbModel::Rgb, output: photocraft_algo::HsbModel::Hsb },
            area,
            area,
            None,
        );
        let pixels = converted.read_region(area);
        let mut buffer = photocraft_compose::Buffer { rect: area, px: vec![[pixels[0], pixels[1], pixels[2], pixels[3]]] };
        photocraft_compose::adjust::apply(&Adjustment::ChannelMixer { matrix: [[0.0, 0.0, 1.0, 0.0], [0.0; 4], [0.0; 4]], monochrome: true }, &mut buffer);
        assert!(buffer.px[0][..3].iter().all(|v| (*v - 0.8).abs() < 1e-6));
        assert_eq!(buffer.px[0][3], 0.375);
    }

    #[test]
    fn colorize_uses_saved_scalar_aliases_once_and_persists_ordinary_native_filters() {
        let color = ColorLiteral { encoding: "RGBA float", pixels: &[0; 16], profile: &[] };
        let m = mapped(
            "gimp:colorize",
            vec![float("hue", 0.25), float("saturation", 0.5), float("lightness", -0.25), Argument { name: "color", value: ArgumentValue::Color(Some(color)) }],
        );
        assert_eq!(m.filters.len(), 1);
        assert_eq!(m.filters[0].params, json!({"hue":90.0,"saturation":50.0,"lightness":-25.0,"colorize":true}));
        assert!(m.notes[0].contains("not applied twice"));
        let mut source = Document::new("native contents", Size::new(2, 1), ColorMode::Rgb, SampleType::U16);
        source.layers.push(Layer::raster("Original native pixels", PixelFormat::new(ColorMode::Rgb, SampleType::U16, true)));
        let bytes = photocraft_format::save_to_bytes(&source, &Default::default()).expect("source native document");
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, None);
        smart.smart_filters = m.filters;
        let mut native = Document::new("native counterpart", Size::new(2, 1), ColorMode::Rgb, SampleType::U16);
        let mut layer = Layer::raster("Editable native object", PixelFormat::new(ColorMode::Rgb, SampleType::U16, true));
        layer.content = LayerContent::Smart(smart);
        native.layers.push(layer);
        let bytes = photocraft_format::save_to_bytes(&native, &Default::default()).expect("native save");
        let reopened = photocraft_format::load_from_bytes(&bytes).expect("native reopen");
        let LayerContent::Smart(object) = &reopened.layers[0].content else {
            panic!("native smart object");
        };
        assert_eq!(object.smart_filters[0].command, "image.adjustments.hueSaturation");
        assert_eq!(object.smart_filters[0].params, json!({"hue":90.0,"saturation":50.0,"lightness":-25.0,"colorize":true}));
        assert_eq!(object.smart_filters[0].blend, BlendMode::Normal);
        assert_eq!(object.smart_filters[0].opacity, 1.0);
    }

    #[test]
    fn unknown_and_unproved_color_configurations_are_not_silently_substituted() {
        let cases = [
            StoredEffect::test("gegl:mono-mixer", vec![float("red", 5.1)]),
            StoredEffect::test("gegl:hue-chroma", vec![float("chroma", 101.0)]),
            StoredEffect::test("gegl:color-temperature", vec![float("intended-temperature", 999.0)]),
            StoredEffect::test("gegl:component-extract", vec![Argument { name: "component", value: ArgumentValue::Enum(999) }]),
            StoredEffect::test("gegl:saturation", vec![float("invented-source-field", 1.0)]),
            StoredEffect::test("gegl:saturation", vec![float("scale", 1.0), float("scale", 1.5)]),
        ];
        for effect in cases {
            let failure = map(&effect, &context()).expect("owned identity").expect_err("unapplied");
            assert!(!failure.message.is_empty());
        }
        assert!(map(&StoredEffect::test("gegl:exposure", vec![]), &context()).is_none());
    }

    #[test]
    fn colorize_literal_alpha_is_not_owner_coverage_and_reopens_as_native_data() {
        use photocraft_doc::adjust::HueRange;
        for alpha in [0, 64, 255] {
            let data = [0, 255, 0, alpha];
            let literal = ColorLiteral { encoding: "R'G'B'A u8", pixels: &data, profile: &[] };
            let m = mapped("gimp:colorize", vec![Argument { name: "color", value: ArgumentValue::Color(Some(literal)) }]);
            let p = &m.filters[0].params;
            assert!((p["hue"].as_f64().unwrap() - 120.).abs() < 0.01);
            assert_eq!(p["saturation"], 100.);
            assert_eq!(p["lightness"], 0.);
            assert!(m.notes[0].contains("does not replace owner coverage"));
            let decoded = Adjustment::HueSaturation {
                hue: p["hue"].as_f64().unwrap() as f32,
                saturation: p["saturation"].as_f64().unwrap() as f32,
                lightness: p["lightness"].as_f64().unwrap() as f32,
                colorize: true,
                ranges: HueRange::defaults(),
            };
            let independent = Adjustment::HueSaturation { hue: 120., saturation: 100., lightness: 0., colorize: true, ranges: HueRange::defaults() };
            for depth in SampleType::ALL {
                let bounds = Rect::new(0, 0, 3, 1);
                let pixels = vec![[0.2, 0.3, 0.4, 0.], [0.4, 0.5, 0.6, 0.375], [0.7, 0.6, 0.5, 1.]];
                let mut expected = photocraft_compose::Buffer { rect: bounds, px: pixels.clone() };
                let mut actual = expected.clone();
                for (adjustment, buffer) in [(&independent, &mut expected), (&decoded, &mut actual)] {
                    photocraft_compose::adjust::apply_in(
                        adjustment,
                        buffer,
                        photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth),
                        Some(depth),
                        ColorMode::Rgb,
                    );
                }
                for (got, want) in actual.px.iter().zip(&expected.px) {
                    for (a, b) in got.iter().zip(want) {
                        assert!((a - b).abs() < 1e-6);
                    }
                }
                assert_eq!(actual.px.iter().map(|p| p[3]).collect::<Vec<_>>(), vec![0., 0.375, 1.]);
                let mut doc = Document::new("Native literal Colorize", Size::new(3, 1), ColorMode::Rgb, depth);
                let mut source = Surface::new(doc.pixel_format());
                source.write_region(bounds, &pixels.into_iter().flatten().collect::<Vec<_>>());
                doc.layers.push(Layer::new("Original", LayerContent::Raster(source)));
                let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                let mut object = SmartObject::new(SmartSource::Embedded { file_name: "native.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, None);
                object.smart_filters = m.filters.clone();
                doc.layers.push(Layer::new("Editable Colorize", LayerContent::Smart(object)));
                let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                let LayerContent::Smart(object) = &reopened.layers[1].content else { panic!("native Colorize") };
                assert_eq!(object.smart_filters, m.filters);
            }
        }
    }

    #[test]
    fn color_only_alias_uses_native_literal_hsl_and_shadow_direction_is_explicit() {
        let literal = ColorLiteral { encoding: "R'G'B'A u8", pixels: &[0, 255, 0, 255], profile: &[] };
        let m = mapped("gimp:colorize", vec![Argument { name: "color", value: ArgumentValue::Color(Some(literal)) }]);
        let params = &m.filters[0].params;
        assert!((params["hue"].as_f64().expect("hue") - 120.0).abs() < 0.01);
        assert!((params["saturation"].as_f64().expect("sat") - 100.0).abs() < 0.01);
        assert!(params["lightness"].as_f64().expect("lightness").abs() < 0.01);
        let m = mapped("gegl:shadows-highlights", vec![float("shadows", 40.0), float("highlights", -30.0), float("radius", 12.0)]);
        assert_eq!(m.filters[0].command, "image.adjustments.shadowsHighlights");
        assert_eq!(m.filters.len(), 2);
        assert_eq!(m.filters[0].params["shadowAmount"], 20.0);
        assert_eq!(m.filters[0].params["color"], 100.0);
        assert_eq!(m.filters[1].params["highlightAmount"], 15.0);
        assert_eq!(m.filters[1].params["color"], 50.0);
        assert_eq!(m.filters[0].params["shadowRadius"], 12.0);
        assert!(m.notes[0].contains("separate color-correction percentages"));
        for (name, value) in [("shadows", -101.0), ("highlights", 101.0)] {
            assert!(map(&StoredEffect::test("gegl:shadows-highlights", vec![float(name, value)]), &context()).expect("owned").is_err());
        }
    }

    #[test]
    fn cie_lightness_units_generate_native_lookup_data_with_independent_black_endpoint() {
        let effect = StoredEffect::test("gegl:hue-chroma", vec![float("lightness", 20.0)]);
        let m = map(&effect, &context()).expect("owned").expect("native table");
        assert_eq!(m.filters[0].command, "image.adjustments.colorLookup");
        assert!(m.filters[0].params.get("file").is_none());
        let table = photocraft_cms::lutfile::parse("native.cube", m.filters[0].params["data"].as_str().expect("embedded cube").as_bytes())
            .expect("ordinary native LUT");
        assert_eq!(table.size, 17);
        // Independent standard Lab black +20 L*: neutral Y=((20+16)/116)^3,
        // encoded with the normal sRGB transfer. This expectation calls no mapper/CMS helper.
        let y = (36.0f64 / 116.0).powi(3);
        let encoded = 1.055 * y.powf(1.0 / 2.4) - 0.055;
        for value in table.data.iter().take(3) {
            assert!((f64::from(*value) - encoded).abs() < 0.0003, "{value} vs {encoded}");
        }
        assert!(table.data.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
    }

    #[test]
    fn cie_lookup_honors_linear_and_assigned_native_profile_models() {
        let effect = StoredEffect::test("gegl:hue-chroma", vec![float("lightness", 20.0), float("chroma", 15.0), float("hue", 30.0)]);
        let base = map(&effect, &context()).expect("owned").expect("sRGB");
        let mut p3 = context();
        p3.target_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
        let wide = map(&effect, &p3).expect("owned").expect("native assigned P3");
        assert_ne!(base.filters[0].params["data"], wide.filters[0].params["data"]);
        let mut invalid = context();
        invalid.target_profile = Some(photocraft_cms::Builtin::SGray.profile().to_bytes());
        assert!(map(&effect, &invalid).expect("owned").is_err());
        let lift = StoredEffect::test("gegl:hue-chroma", vec![float("lightness", 20.0)]);
        let mut linear = context();
        linear.linear = true;
        linear.target_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        let m = map(&lift, &linear).expect("owned").expect("linear target");
        let table = photocraft_cms::lutfile::parse("native.cube", m.filters[0].params["data"].as_str().expect("cube").as_bytes()).expect("native LUT");
        let expected = (36.0f64 / 116.0).powi(3);
        for value in table.data.iter().take(3) {
            assert!((f64::from(*value) - expected).abs() < 0.0002);
        }
        linear.mode = ColorMode::Grayscale;
        linear.target_profile = Some(photocraft_cms::builtin::linear_gray().to_bytes());
        assert!(map(&lift, &linear).expect("owned").is_ok());
    }

    #[test]
    fn spectral_white_points_cover_source_domain_and_match_independent_illuminant_a() {
        for kelvin in [1000.0, 2856.0, 6500.0, 12000.0] {
            let white = temperature_white(kelvin).expect("bounded physical white");
            assert_eq!(white[1], 1.0);
            assert!(white.iter().all(|value| value.is_finite() && *value > 0.0));
        }
        let a = temperature_white(2856.0).expect("illuminant A approximation");
        let sum: f64 = a.iter().sum();
        // Independent CIE illuminant A reference, NIST SP545: x=.4476, y=.4075.
        assert!((a[0] / sum - 0.4476).abs() < 0.003);
        assert!((a[1] / sum - 0.4075).abs() < 0.003);
        for invalid in [0.0, 999.0, 12001.0, f64::NAN, f64::INFINITY] {
            assert!(temperature_white(invalid).is_err());
        }
    }

    #[test]
    fn temperature_native_lookup_obeys_observed_correction_direction_and_neutral_identity() {
        let cool = mapped("gegl:color-temperature", vec![float("original-temperature", 6500.0), float("intended-temperature", 2856.0)]);
        let warm = mapped("gegl:color-temperature", vec![float("original-temperature", 2856.0), float("intended-temperature", 6500.0)]);
        let parse = |m: &NativeMapping| {
            photocraft_cms::lutfile::parse("native.cube", m.filters[0].params["data"].as_str().expect("native cube").as_bytes()).expect("native table")
        };
        let cool = parse(&cool);
        let warm = parse(&warm);
        let neutral = ((8 * 17 + 8) * 17 + 8) * 3;
        assert!(cool.data[neutral + 2] > cool.data[neutral], "intended lower temperature corrects toward blue");
        assert!(warm.data[neutral] > warm.data[neutral + 2], "original lower temperature corrects toward red");
        for edge in [1000.0, 12000.0] {
            let mapping = mapped("gegl:color-temperature", vec![float("intended-temperature", edge)]);
            assert!(parse(&mapping).data.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
        }
        let neutral = mapped("gegl:color-temperature", vec![float("original-temperature", 2856.0), float("intended-temperature", 2856.0)]);
        assert_eq!(neutral.filters[0].command, "image.adjustments.channelMixer");
        assert!(neutral.filters[0].params.get("data").is_none(), "identity must not add lookup clipping to HDR");
    }
    #[test]
    fn native_saturation_gain_covers_full_source_domain_and_matches_native_response() {
        let input = [0.4, 0.5, 0.6]; // HSL saturation .2, lightness .5.
        for scale in [0.0, 0.5, 1.0, 1.25, 2.0, 5.0, 10.0] {
            let m = mapped("gegl:saturation", vec![float("scale", scale as f32)]);
            let percent = m.filters[0].params["saturation"].as_f64().expect("native percentage");
            assert!((-100.0..100.0).contains(&percent));
            let result = photocraft_compose::adjust::hue_saturation(input, 0.0, percent as f32 / 100.0, 0.0, false);
            let saturation = (result[2] - result[0]) / (1.0 - (result[2] + result[0] - 1.0).abs());
            assert!((f64::from(saturation) - (0.2f64 * scale).min(1.0)).abs() < 1e-5);
        }
    }

    #[test]
    fn mixer_normalization_preserves_neutrals_and_extended_domain_is_native_lookup() {
        let preserve = Argument { name: "preserve-luminosity", value: ArgumentValue::Bool(true) };
        let m = mapped("gegl:mono-mixer", vec![float("red", 1.0), float("green", 2.0), float("blue", 1.0), preserve]);
        assert_eq!(m.filters[0].params["gray"], json!([25.0, 50.0, 25.0, 0.0]));
        let m = mapped(
            "gegl:channel-mixer",
            vec![float("rr-gain", 1.0), float("rg-gain", -1.0), Argument { name: "preserve-luminosity", value: ArgumentValue::Bool(true) }],
        );
        assert_eq!(m.filters[0].params["red"], json!([50.0, -50.0, 0.0, 0.0]));
        let m = mapped("gegl:mono-mixer", vec![float("red", 5.0), float("green", -5.0), float("blue", 0.0)]);
        assert_eq!(m.filters[0].command, "image.adjustments.colorLookup");
        let table = photocraft_cms::lutfile::parse("native.cube", m.filters[0].params["data"].as_str().expect("cube").as_bytes()).expect("table");
        assert_eq!(&table.data[..3], &[0.0, 0.0, 0.0]);
        assert!(table.data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(m.notes[0].contains("standard matrix"));
    }

    #[test]
    fn native_components_cover_color_models_and_extract_alpha_as_ordinary_native_data() {
        for component in 8..=19 {
            let m = mapped("gegl:component-extract", vec![Argument { name: "component", value: ArgumentValue::Enum(component) }]);
            let table = photocraft_cms::lutfile::parse("native.cube", m.filters[0].params["data"].as_str().expect("cube").as_bytes()).expect("native table");
            assert!(table.data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
            if component == 13 || component == 14 {
                assert!(table.data[..3].iter().all(|v| (*v - 0.5).abs() < 1e-4));
            }
            if component == 15 {
                assert!(table.data[..3].iter().all(|v| v.abs() < 1e-4));
            }
            if component == 16 || component == 17 {
                assert!(table.data[..3].iter().all(|v| (*v - 128.0 / 255.0).abs() < 1e-4));
            }
            let inverted = mapped(
                "gegl:component-extract",
                vec![Argument { name: "component", value: ArgumentValue::Enum(component) }, Argument { name: "invert", value: ArgumentValue::Bool(true) }],
            );
            assert_ne!(m.filters[0].params["data"], inverted.filters[0].params["data"]);
        }
        for invert in [false, true] {
            let effect = StoredEffect::test(
                "gegl:component-extract",
                vec![Argument { name: "component", value: ArgumentValue::Enum(20) }, Argument { name: "invert", value: ArgumentValue::Bool(invert) }],
            );
            let m = map(&effect, &context()).expect("owned").expect("native alpha extraction");
            assert_eq!(m.filters.last().unwrap().command, "filter.other.alphaToColor");
            assert_eq!(m.filters.len(), if invert { 2 } else { 1 });
            if invert {
                let values = m.filters[0].params["values"].as_array().unwrap();
                assert_eq!(m.filters[0].command, "filter.other.alphaCurve");
                assert_eq!(values.len(), 256);
                assert_eq!(values[0], 1.);
                assert_eq!(values[255], 0.);
            }
            assert!(m.notes[0].contains("output is opaque"));
        }
        for argument in [ArgumentValue::Enum(-1), ArgumentValue::Enum(21), ArgumentValue::Enum(999), ArgumentValue::String("alpha")] {
            let effect = StoredEffect::test("gegl:component-extract", vec![Argument { name: "component", value: argument }]);
            assert!(map(&effect, &context()).expect("owned").is_err());
        }
    }

    #[test]
    fn signed_tonal_controls_become_separate_native_curves_with_anchored_midpoint() {
        let m = mapped("gegl:shadows-highlights", vec![float("shadows", -40.0), float("highlights", 60.0)]);
        assert_eq!(m.filters.len(), 3);
        assert_eq!(m.filters[0].params["shadowAmount"], 0.0);
        assert_eq!(m.filters[1].params["highlightAmount"], 0.0);
        assert_eq!(m.filters[2].command, "image.adjustments.curves");
        let points = m.filters[2].params["points"].as_array().expect("native points");
        assert_eq!(points[0], json!([0.0, 0.0]));
        assert_eq!(points[2], json!([127.5, 127.5]));
        assert_eq!(points[4], json!([255.0, 255.0]));
        assert!(points[1][1].as_f64().expect("shadow") < 63.75);
        assert!(points[3][1].as_f64().expect("highlight") > 191.25);
        assert!(m.notes.iter().any(|note| note.contains("Curves approximation")));
    }
    #[test]
    fn remaining_tonal_controls_translate_to_directional_native_parameters() {
        let compressed = mapped("gegl:shadows-highlights", vec![float("shadows", 90.0), float("highlights", -90.0), float("compress", 100.0)]);
        assert!(compressed.filters.iter().all(|filter| filter.params["shadowAmount"] == 0.0 && filter.params["highlightAmount"] == 0.0));
        let expanded = mapped(
            "gegl:shadows-highlights",
            vec![
                float("shadows", 90.0),
                float("highlights", -90.0),
                float("compress", 0.0),
                float("shadows-ccorrect", 25.0),
                float("highlights-ccorrect", 75.0),
            ],
        );
        assert_eq!(expanded.filters[0].params["shadowAmount"], 90.0);
        assert_eq!(expanded.filters[0].params["shadowTone"], 100.0);
        assert_eq!(expanded.filters[0].params["color"], 25.0);
        assert_eq!(expanded.filters[1].params["color"], 75.0);
        for (whitepoint, expected_gain) in [(-10.0, 0.9f64), (10.0, 1.1)] {
            let m = mapped("gegl:shadows-highlights", vec![float("whitepoint", whitepoint)]);
            let exposure = m.filters.last().expect("native exposure");
            assert_eq!(exposure.command, "image.adjustments.exposure");
            let stops = exposure.params["exposure"].as_f64().expect("EV");
            assert!((stops.exp2() - expected_gain).abs() < 1e-12);
        }
    }
    #[test]
    fn extended_vibrance_gain_uses_bounded_native_passes_with_one_vibrance_stage() {
        for scale in [2.1f32, 4.0, 10.0] {
            let m = mapped("gegl:vibrance", vec![float("vibrance", 30.0), float("saturation", scale)]);
            assert!((2..=4).contains(&m.filters.len()));
            assert_eq!(m.filters[0].params["vibrance"], 30.0);
            assert!(m.filters.iter().skip(1).all(|filter| filter.params["vibrance"] == 0));
            let product: f64 = m.filters.iter().map(|filter| 1.0 + filter.params["saturation"].as_f64().expect("native percentage") / 100.0).product();
            assert!((product - f64::from(scale)).abs() < 1e-6);
        }
        let m = mapped("gegl:vibrance", vec![float("saturation", 10.0)]);
        let input = [0.48, 0.5, 0.52];
        let gray = 0.288 * input[0] + 0.712 * input[1];
        let expected = input.map(|v| gray + 10.0 * (v - gray));
        let mut actual = input;
        for filter in m.filters {
            actual = photocraft_compose::adjust::vibrance_px(
                actual,
                0.0,
                filter.params["saturation"].as_f64().expect("native saturation") as f32 / 100.0,
                photocraft_compose::adjust::Transfer::Gamma(1.0),
                1.0,
            );
        }
        assert!(actual.iter().zip(expected).all(|(a, b)| (*a - b).abs() < 1e-5));
    }
    #[test]
    fn c2g_native_radius_shadow_and_superseded_quality_are_explicit_without_alpha_loss() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::adjust::Adjustment;
        use photocraft_geom::Rect;
        use photocraft_raster::{Interrupt, Surface};
        let source = StoredEffect::test("gegl:c2g", vec![float("radius", 6.)]);
        let mapped = map(&source, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[1].command, "filter.sharpen.unsharpMask");
        assert_eq!(mapped.filters[1].params["radius"], 2.);
        let quality =
            map(&StoredEffect::test("gegl:c2g", vec![float("radius", 6.), float("samples", 1000.), float("iterations", 1.)]), &context()).unwrap().unwrap();
        assert_eq!(mapped.filters, quality.filters, "Monte Carlo quality must not fabricate native contrast changes");
        assert!(quality.notes[0].contains("superseded stochastic sampling-quality"));
        let enhanced =
            map(&StoredEffect::test("gegl:c2g", vec![float("radius", 6.), Argument { name: "enhance-shadows", value: ArgumentValue::Bool(true) }]), &context())
                .unwrap()
                .unwrap();
        assert_eq!(enhanced.filters[2].command, "image.adjustments.shadowsHighlights");
        assert_eq!(enhanced.filters[2].params["shadowAmount"], 35.);
        assert_eq!(enhanced.filters[2].params["shadowRadius"], 2.);
        let bounds = Rect::new(0, 0, 7, 5);
        let mut buffer =
            photocraft_compose::Buffer { rect: bounds, px: (0..35).map(|i| [i as f32 / 40., (34 - i) as f32 / 40., 0.25, [0.25, 0.5, 0.75][i % 3]]).collect() };
        let alpha: Vec<_> = buffer.px.iter().map(|p| p[3]).collect();
        photocraft_compose::adjust::apply_depth(
            &Adjustment::BlackWhite { weights: [40., 60., 40., 60., 20., 80.], tint: None },
            &mut buffer,
            photocraft_compose::adjust::Transfer::Gamma(1.),
            Some(SampleType::F32),
        );
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        surface.write_region(bounds, &buffer.px.iter().flatten().copied().collect::<Vec<_>>());
        let sharpened = photocraft_algo::apply_in_with(
            &surface,
            &photocraft_algo::FilterParams::UnsharpMask { amount: 50., radius: 2., threshold: 0. },
            bounds,
            bounds,
            None,
            bounds,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_ne!(sharpened.read_region(bounds), surface.read_region(bounds));
        for (pixel, expected) in sharpened.read_region(bounds).as_chunks::<4>().0.iter().zip(alpha) {
            assert_eq!(pixel[3], expected);
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
        }
    }
    #[test]
    fn c2g_large_neighborhood_native_clipped_branches_preserve_alpha_and_native_reopen() {
        use super::super::NativeStructure;
        use photocraft_color::{BlendMode, PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::{Rect, Size};
        use photocraft_raster::{Interrupt, Surface};
        let converted = map(&StoredEffect::test("gegl:c2g", vec![float("radius", 6000.)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native broad C2G") };
        assert_eq!(branches[1].blend, BlendMode::Subtract);
        assert!(branches[1].clipped);
        assert_eq!(branches[1].filters.iter().filter(|f| f.command == "filter.blur.gaussianBlur").count(), 4);
        let bounds = Rect::new(0, 0, 7, 5);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..35)
            .flat_map(|i| {
                let value = 0.1 + i as f32 / 100.;
                [value, value, value, 0.5]
            })
            .collect();
        source.write_region(bounds, &pixels);
        let mut doc = Document::new("Native broad C2G", Size::new(7, 5), ColorMode::Rgb, SampleType::F32);
        doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        for (index, branch) in branches.iter().enumerate() {
            let mut surface = source.clone();
            if index == 1 {
                for _ in 0..4 {
                    surface = photocraft_algo::apply_in_with(
                        &surface,
                        &photocraft_algo::FilterParams::GaussianBlur { radius: 1000. },
                        bounds,
                        bounds,
                        None,
                        bounds,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                }
            }
            let gain = if index == 0 { 1.5 } else { 0.5 };
            let samples: Vec<_> = surface.read_region(bounds).as_chunks::<4>().0.iter().flat_map(|p| [p[0] * gain, p[1] * gain, p[2] * gain, p[3]]).collect();
            surface.write_region(bounds, &samples);
            let mut layer = Layer::new("Native broad contrast branch", LayerContent::Raster(surface));
            layer.clipped = branch.clipped;
            layer.blend = branch.blend;
            doc.layers.push(layer);
        }
        let before = photocraft_compose::render(&doc, bounds);
        assert!(before.px.iter().all(|p| p[3] == 0.5));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, bounds).px, before.px);
        let tiny = MappingContext { remaining_generation_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:c2g", vec![]), &tiny).unwrap().is_err());
        assert!(map(&StoredEffect::test("gegl:c2g", vec![float("samples", 1.5)]), &context()).unwrap().is_err());
    }
}
