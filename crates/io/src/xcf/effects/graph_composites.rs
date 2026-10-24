//! Explicit binary graph adapters to ordinary native layer blending.
//! Property facts: pinned GIMP gimpoperationlayermode declarations and GEGL
//! generated compositor registrations. No source blend implementation is used.
use super::{
    MappingContext, MappingFailure, NativeMapping, StoredEffect,
    graph_schema::{self, Kind},
    native_filter,
};
use photocraft_color::BlendMode;
use photocraft_doc::import_conversion::ConversionReason;

pub(super) struct BinaryMapping {
    pub blend: BlendMode,
    pub opacity: f32,
    pub swap: bool,
    pub auxiliary: Option<NativeMapping>,
    /// Select an actual input directly: false is main, true is auxiliary.
    pub select: Option<bool>,
    pub knockout: bool,
    pub notes: Vec<String>,
}

const GIMP: &[&str] = &[
    "gimp:dissolve",
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
];

// Exact generated registrations, not an inferred svg:/gegl: prefix substitution.
const SVG: &[(&str, &str, BlendMode, bool)] = &[
    ("svg:src-over", "gegl:over", BlendMode::Normal, false),
    ("svg:dst-over", "gegl:dst-over", BlendMode::Normal, true),
    ("svg:color-burn", "gegl:color-burn", BlendMode::ColorBurn, false),
    ("svg:color-dodge", "gegl:color-dodge", BlendMode::ColorDodge, false),
    ("svg:darken", "gegl:darken", BlendMode::Darken, false),
    ("svg:difference", "gegl:difference", BlendMode::Difference, false),
    ("svg:exclusion", "gegl:exclusion", BlendMode::Exclusion, false),
    ("svg:hard-light", "gegl:hard-light", BlendMode::HardLight, false),
    ("svg:lighten", "gegl:lighten", BlendMode::Lighten, false),
    ("svg:overlay", "gegl:overlay", BlendMode::Overlay, false),
    ("svg:screen", "gegl:screen", BlendMode::Screen, false),
    ("svg:plus", "gegl:plus", BlendMode::LinearDodge, false),
];

pub(super) fn operation(token: &str) -> Option<&'static str> {
    if matches!(token, "gegl:mix" | "mix") {
        return Some("gegl:mix");
    }
    if let Some(op) = super::graph_math::operation(token) {
        return Some(op);
    }
    match token {
        "svg:src" | "gegl:src" | "src" => return Some("svg:src"),
        "svg:dst" | "gegl:dst" | "dst" => return Some("svg:dst"),
        _ => {}
    }
    if let Some(op) = GIMP.iter().find(|op| **op == token) {
        return Some(op);
    }
    if token == "over" {
        return Some("svg:src-over");
    }
    if token == "gegl:soft-light" || token == "soft-light" {
        return Some("gegl:soft-light");
    }
    SVG.iter().find(|(op, alias, _, _)| *op == token || *alias == token || alias.strip_prefix("gegl:") == Some(token)).map(|(op, ..)| *op)
}

pub(super) fn field(operation: &str, name: &str) -> Option<Kind> {
    if operation == "gegl:mix" {
        return (name == "ratio").then_some(Kind::Number { min: -f64::MAX, max: f64::MAX });
    }
    if super::graph_math::operation(operation).is_some() {
        return super::graph_math::field(operation, name);
    }
    if GIMP.contains(&operation) {
        graph_schema::field(if operation == "gimp:dissolve" { "gimp:layer-mode" } else { operation }, name)
    } else if name == "srgb" && self::operation(operation).is_some() {
        Some(Kind::Boolean)
    } else {
        None
    }
}

fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: ConversionReason::InvalidOptionalData, message: message.into() }
}

fn native_mode(value: i32) -> Result<(BlendMode, bool), MappingFailure> {
    use BlendMode as B;
    let mode = match value {
        0 | 28 | 58 | 59 => (B::Normal, false), 1 => (B::Dissolve, false), 2 | 29 => (B::Normal, true),
        3 | 30 => (B::Multiply, false), 4 | 31 => (B::Screen, false), 5 | 19 | 45 => (B::SoftLight, false),
        6 | 32 => (B::Difference, false), 7 | 33 => (B::LinearDodge, false), 8 | 34 => (B::Subtract, false),
        9 | 35 => (B::Darken, false), 10 | 36 => (B::Lighten, false), 11 | 24 | 37 => (B::Hue, false),
        12 | 25 | 38 => (B::Saturation, false), 13 | 26 | 39 => (B::Color, false), 14 | 27 | 40 | 56 => (B::Luminosity, false),
        15 | 41 => (B::Divide, false), 16 | 42 => (B::ColorDodge, false), 17 | 43 => (B::ColorBurn, false),
        18 | 44 => (B::HardLight, false), 20 | 21 | 46 | 47 => (B::LinearLight, false), 23 => (B::Overlay, false), 48 => (B::VividLight, false),
        49 => (B::PinLight, false), 50 => (B::LinearLight, false), 51 => (B::HardMix, false),
        52 => (B::Exclusion, false), 53 => (B::LinearBurn, false), 54 => (B::DarkerColor, false), 55 => (B::LighterColor, false),
        _ => return Err(format!("Actual auxiliary input is present, but source layer mode {value} needs a separate native coverage/knockout mapping; substituting Normal would discard its meaning").into()),
    };
    Ok(mode)
}

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<BinaryMapping, MappingFailure> {
    let op = operation(effect.operation).ok_or("Binary native adapter does not own this source identity")?;
    let mut seen = std::collections::HashSet::new();
    for arg in &effect.arguments {
        if field(op, arg.name).is_none() || !seen.insert(arg.name) {
            return Err(invalid(format!("Unknown or duplicate binary graph field {op}:{}", arg.name)));
        }
    }
    if op == "gegl:mix" {
        let ratio = effect.number("ratio")?.unwrap_or(0.5);
        if !ratio.is_finite() {
            return Err(invalid("Stock mixing ratio must be finite"));
        }
        if !(0.0..=1.0).contains(&ratio) {
            return Err("Stock extrapolating mix ratio is outside native normalized layer opacity; original controls remain retained without clamping".into());
        }
        return Ok(BinaryMapping {
            blend: BlendMode::Normal,
            opacity: ratio as f32,
            swap: false,
            auxiliary: None,
            select: if ratio == 0.0 {
                Some(false)
            } else if ratio == 1.0 {
                Some(true)
            } else {
                None
            },
            knockout: false,
            notes: vec![format!(
                "Stock mixing ratio {ratio} becomes ordinary native upper-layer opacity. Endpoints select the actual input exactly. Between endpoints native source-over alpha, document profile and depth apply; straight-component source interpolation is explicitly approximated by editable native layer compositing."
            )],
        });
    }
    if matches!(op, "svg:src" | "svg:dst") {
        effect.boolean("srgb")?;
        return Ok(BinaryMapping {
            blend: BlendMode::Normal,
            opacity: 1.0,
            swap: false,
            auxiliary: None,
            select: Some(op == "svg:src"),
            knockout: false,
            notes: vec![format!(
                "{op} selects its actual native input without a synthetic backdrop, duplicated owner or source evaluator. Native profile/depth rules apply."
            )],
        });
    }
    let (blend, swap, opacity) = if super::graph_math::operation(op).is_some() {
        if effect.number("value")?.is_some_and(|value| !value.is_finite()) {
            return Err(invalid("Inactive binary scalar control must still be finite"));
        }
        let blend = match op {
            "gegl:add" => BlendMode::LinearDodge,
            "gegl:subtract" => BlendMode::Subtract,
            "gegl:multiply" => BlendMode::Multiply,
            "gegl:divide" => BlendMode::Divide,
            _ => return Err("Actual auxiliary exponent image is available, but ordinary native layer blending has no per-pixel variable exponent control; source gamma graph remains unapplied".into()),
        };
        (blend, false, 1.0)
    } else if GIMP.contains(&op) {
        let opacity = effect.number("opacity")?.unwrap_or(1.0);
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(invalid("Binary graph opacity must be finite 0..1"));
        }
        for name in ["blend-space", "composite-space", "composite-mode", "layer-mode"] {
            if let Some(value) = effect.enum_value(name)? {
                let Some(Kind::Choice(domain)) = field(op, name) else {
                    return Err(invalid("Missing frozen binary enum domain"));
                };
                if !domain.iter().any(|(_, known)| *known == value) {
                    return Err(invalid(format!("Invalid binary {name} enum")));
                }
            }
        }
        let fixed = match op {
            "gimp:dissolve" => 1,
            "gimp:normal" => 28, "gimp:behind" => 29, "gimp:multiply-legacy" => 3, "gimp:screen-legacy" => 4,
            "gimp:softlight-legacy" => 19, "gimp:difference-legacy" => 6, "gimp:addition-legacy" => 7,
            "gimp:subtract-legacy" => 8, "gimp:darken-only-legacy" => 9, "gimp:lighten-only-legacy" => 10,
            "gimp:hsv-hue-legacy" => 11, "gimp:hsv-saturation-legacy" => 12, "gimp:hsl-color-legacy" => 13,
            "gimp:hsv-value-legacy" => 14, "gimp:divide-legacy" => 15, "gimp:dodge-legacy" => 16,
            "gimp:burn-legacy" => 17, "gimp:hardlight-legacy" => 18,
            "gimp:grain-extract-legacy" => 20, "gimp:grain-merge-legacy" => 21,
            "gimp:erase" | "gimp:merge" | "gimp:replace" | "gimp:overwrite" => 28,
            "gimp:layer-mode" => effect.enum_value("layer-mode")?.unwrap_or(28),
            _ => return Err(format!("Actual auxiliary input is available for {op}; its coverage replacement/erase/grain/pass-through behavior still requires a native structure adapter, rather than a missing-resource fallback").into()),
        };
        // Specialized classes have their own processing identity. A conflicting
        // inherited selector needs source evidence, rather than overriding it.
        if op != "gimp:layer-mode" && effect.enum_value("layer-mode")?.is_some_and(|value| value != fixed && value != 28) {
            return Err("Conflicting specialized composer and inherited layer-mode selector need an explicit native conversion policy".into());
        }
        let (blend, swap) = native_mode(fixed)?;
        (blend, swap, opacity as f32)
    } else {
        effect.boolean("srgb")?;
        let (blend, swap) = if op == "gegl:soft-light" {
            (BlendMode::SoftLight, false)
        } else {
            SVG.iter().find(|(known, ..)| *known == op).map(|(_, _, blend, swap)| (*blend, *swap)).ok_or("Missing explicit native compositor")?
        };
        (blend, swap, 1.0)
    };
    // Behind attenuation belongs to the auxiliary image even after input swap.
    // Ordinary native Alpha Curve scales that real image before it becomes below.
    // Public GIMP compositing documentation describes the grain midpoint
    // intent. Translate it to editable Photoshop Linear Light and Channel Mixer.
    let grain = match op {
        "gimp:grain-extract-legacy" => Some((-0.5, 0.75)),
        "gimp:grain-merge-legacy" => Some((0.5, 0.25)),
        "gimp:layer-mode" => match effect.enum_value("layer-mode")? {
            Some(20 | 46) => Some((-0.5, 0.75)),
            Some(21 | 47) => Some((0.5, 0.25)),
            _ => None,
        },
        _ => None,
    };
    let auxiliary = if let Some((gain, offset)) = grain {
        Some(NativeMapping {
            working_rgb: context.mode == photocraft_color::ColorMode::Grayscale,
            filters: vec![native_filter("image.adjustments.channelMixer", serde_json::json!({
                "red":[gain*100.,0.,0.,offset*100.], "green":[0.,gain*100.,0.,offset*100.],
                "blue":[0.,0.,gain*100.,offset*100.], "monochrome":false
            }))],
            notes: vec!["Grain intent becomes native Channel Mixer followed by native Linear Light. Native midpoint, RGB/Gray conversion, alpha, intermediate precision/clipping and document profile behavior apply; this is an explicit editable native approximation.".into()],
            ..Default::default()
        })
    } else if swap && opacity != 1.0 {
        let values = (0..256).map(|index| f64::from(index) / 255.0 * f64::from(opacity)).collect::<Vec<_>>();
        Some(NativeMapping { filters: vec![native_filter("filter.other.alphaCurve", serde_json::json!({"values": values}))], ..Default::default() })
    } else {
        None
    };
    Ok(BinaryMapping {
        blend,
        swap,
        opacity: if swap { 1.0 } else { opacity },
        auxiliary,
        select: if matches!(op, "gimp:replace" | "gimp:overwrite") && opacity == 1.0 { Some(true) } else { None },
        knockout: op == "gimp:erase" || (op == "gimp:layer-mode" && effect.enum_value("layer-mode")? == Some(58)),
        notes: vec![format!(
            "Actual main/auxiliary images become native {blend:?} layers with native layer opacity. Source blend/composite spaces, legacy color formula, coverage policy and HDR/clipping follow ordinary PhotoCraft rules; source controls remain inert conversion history. An arithmetic scalar control is inactive when the actual auxiliary image is supplied. Behind swaps actual inputs and attenuates the actual auxiliary coverage before the swap. Erase uses native Shallow Knockout with hidden fill; Merge uses native Normal, and full-opacity Replace/Overwrite selects the actual auxiliary input. Partial replacement uses native layer opacity and its alpha policy. This is a native approximation of {op}, not source compositing emulation."
        )],
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue};
    use super::*;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: photocraft_color::ColorMode::Rgb,
            linear: false,
            resolution_dpi: 72.,
            owner_bounds: photocraft_geom::Rect::new(0, 0, 8, 6),
            native_filter_bounds: photocraft_geom::Rect::new(0, 0, 8, 6),
            target_profile: None,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn run(effect: &StoredEffect<'_>) -> Result<BinaryMapping, MappingFailure> {
        super::map(effect, &context())
    }
    #[test]
    fn exact_stock_aliases_do_not_confuse_arithmetic_multiply_with_blending() {
        assert_eq!(operation("screen"), Some("svg:screen"));
        assert_eq!(operation("gegl:screen"), Some("svg:screen"));
        assert_eq!(operation("gegl:multiply"), Some("gegl:multiply"));
        assert_eq!(operation("svg:multiply"), None);
        assert_eq!(operation("gimp:multiply-legacy"), Some("gimp:multiply-legacy"));
        assert_eq!(operation("multiply-legacy"), None);
        assert_eq!(operation("src-over"), None);
        let mix = |ratio| run(&StoredEffect::test("gegl:mix", vec![Argument { name: "ratio", value: ArgumentValue::Literal(ratio) }]));
        assert_eq!(mix(0.).unwrap().select, Some(false));
        assert_eq!(mix(1.).unwrap().select, Some(true));
        assert_eq!(mix(0.25).unwrap().opacity, 0.25);
        assert!(mix(-0.25).is_err());
        for (op, mode) in [
            ("gegl:add", BlendMode::LinearDodge),
            ("gegl:subtract", BlendMode::Subtract),
            ("gegl:multiply", BlendMode::Multiply),
            ("gegl:divide", BlendMode::Divide),
        ] {
            assert_eq!(run(&StoredEffect::test(op, vec![])).unwrap().blend, mode);
        }
        assert_eq!(run(&StoredEffect::test("gimp:dissolve", vec![])).unwrap().blend, BlendMode::Dissolve);
        for (op, gain, offset) in [("gimp:grain-extract-legacy", -50., 75.), ("gimp:grain-merge-legacy", 50., 25.)] {
            let mapped = run(&StoredEffect::test(op, vec![])).unwrap();
            assert_eq!(mapped.blend, BlendMode::LinearLight);
            let auxiliary = mapped.auxiliary.unwrap();
            assert!(!auxiliary.working_rgb);
            let gray = MappingContext { mode: photocraft_color::ColorMode::Grayscale, ..context() };
            assert!(super::map(&StoredEffect::test(op, vec![]), &gray).unwrap().auxiliary.unwrap().working_rgb);
            assert_eq!(auxiliary.filters[0].params["red"], serde_json::json!([gain, 0., 0., offset]));
        }
        for (op, alias, mode, swap) in SVG {
            for name in [*op, *alias] {
                let mapping = run(&StoredEffect::test(name, vec![])).unwrap();
                assert_eq!((mapping.blend, mapping.swap, mapping.opacity), (*mode, *swap, 1.0));
            }
        }
    }
    #[test]
    fn behind_scales_actual_auxiliary_before_swap_and_invalid_controls_fail() {
        let args = || vec![Argument { name: "opacity", value: ArgumentValue::Literal(0.4) }];
        let mapping = run(&StoredEffect::test("gimp:behind", args())).unwrap();
        assert!(mapping.swap);
        assert_eq!(mapping.opacity, 1.0);
        let auxiliary = mapping.auxiliary.unwrap();
        assert_eq!(auxiliary.filters[0].command, "filter.other.alphaCurve");
        let last = auxiliary.filters[0].params["values"][255].as_f64().unwrap();
        assert!((last - 0.4).abs() < 1e-7);
        let normal = run(&StoredEffect::test("gimp:normal", args())).unwrap();
        assert_eq!(normal.opacity, 0.4);
        assert!(normal.auxiliary.is_none());
        for value in [f64::NAN, f64::INFINITY, -1., 2.] {
            let failure = run(&StoredEffect::test("gimp:normal", vec![Argument { name: "opacity", value: ArgumentValue::Literal(value) }])).err().unwrap();
            assert_eq!(failure.reason, ConversionReason::InvalidOptionalData);
        }
        assert!(run(&StoredEffect::test("gimp:erase", vec![])).unwrap().knockout);
        let failure = run(&StoredEffect::test("gimp:anti-erase", vec![])).err().unwrap();
        assert_eq!(failure.reason, ConversionReason::MissingMapping);
        assert!(failure.message.contains("Actual auxiliary"));
        assert_eq!(run(&StoredEffect::test("gegl:src", vec![])).unwrap().select, Some(true));
        assert_eq!(run(&StoredEffect::test("svg:dst", vec![])).unwrap().select, Some(false));
    }
}
