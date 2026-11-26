//! Bounded native point adjustments over shared typed records. These are
//! disclosed native conversions, never a GEGL evaluator or implicit color profile.
use super::{Failure, Result, allowed, filter, number};
use craft_xcf::effects::{ArgumentValue, StoredEffect};
use photocraft_doc::{
    SmartFilter,
    adjust::{Adjustment, HueRange},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

pub(super) fn supports(op: &str) -> bool {
    matches!(
        op,
        "gegl:invert"
            | "gegl:invert-linear"
            | "gegl:invert-gamma"
            | "gimp:posterize"
            | "gegl:vibrance"
            | "gegl:saturation"
            | "gegl:channel-mixer"
            | "gegl:mono-mixer"
            | "gegl:sepia"
    )
}
pub(super) fn native(command: &str) -> bool {
    matches!(
        command,
        "image.adjustments.posterize"
            | "image.adjustments.vibrance"
            | "image.adjustments.hueSaturation"
            | "image.adjustments.channelMixer"
            | "image.adjustments.photoFilter"
    )
}
fn flag(e: &StoredEffect<'_>, key: &str, default: bool) -> Result<bool> {
    match e.arguments.iter().find(|arg| arg.name == key).map(|arg| &arg.value) {
        None => Ok(default),
        Some(ArgumentValue::Bool(v)) => Ok(*v),
        _ => Err(Failure::Invalid("native color flag needs a stored boolean")),
    }
}
fn choice(e: &StoredEffect<'_>, key: &str, default: i32) -> Result<i32> {
    match e.arguments.iter().find(|arg| arg.name == key).map(|arg| &arg.value) {
        None => Ok(default),
        Some(ArgumentValue::Enum(v)) => Ok(*v),
        _ => Err(Failure::Invalid("native color choice needs a stored enum")),
    }
}
pub(super) fn map(e: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    match e.operation {
        "gegl:invert" | "gegl:invert-linear" | "gegl:invert-gamma" => {
            allowed(e, &[], ctl)?;
            Ok((
                vec![filter("image.adjustments.invert", json!({}))],
                "Source inversion becomes ordinary native Invert. Native document encoding, signed/HDR range, clipping and depth rules supersede the source transfer choice; this is a native approximation.",
            ))
        }
        "gimp:posterize" => {
            allowed(e, &["levels"], ctl)?;
            let levels = number(e, "levels", 3., 2., 255.)?;
            if levels.fract() != 0. {
                return Err(Failure::Invalid("Posterize count must be integral"));
            }
            Ok((
                vec![filter("image.adjustments.posterize", json!({"levels":levels}))],
                "Source level count becomes native Posterize. Native channel, depth, HDR quantization and clipping apply; source 256-level and nonintegral counts remain unapplied.",
            ))
        }
        "gegl:vibrance" => {
            allowed(e, &["vibrance", "saturation"], ctl)?;
            let v = number(e, "vibrance", 0., -100., 100.)?;
            let scale = number(e, "saturation", 1., 0., 10.)?;
            let first = scale.min(2.);
            let mut filters = vec![filter("image.adjustments.vibrance", json!({"vibrance":v,"saturation":(first-1.)*100.}))];
            let mut remaining = scale / first.max(1.);
            if scale > 2. {
                for _ in 0..3 {
                    ctl.check().map_err(|_| Failure::Cancelled)?;
                    if remaining <= 1. {
                        break;
                    }
                    let factor = remaining.min(2.);
                    filters.push(filter("image.adjustments.vibrance", json!({"vibrance":0.,"saturation":(factor-1.)*100.})));
                    remaining /= factor;
                }
            }
            Ok((
                filters,
                "Source vibrance and saturation become at most four native Vibrance passes, with vibrance applied once. Native selective saturation, transfer and intermediate clipping/quantization are explicit approximations; opacity and coverage remain compound-stage controls.",
            ))
        }
        "gegl:saturation" => {
            allowed(e, &["scale", "colorspace"], ctl)?;
            let scale = number(e, "scale", 1., 0., 10.)?;
            if !matches!(choice(e, "colorspace", 0)?, 0..=2) {
                return Err(Failure::Unsupported("unrecognized source saturation color space"));
            }
            let percent = if scale <= 1. { (scale - 1.) * 100. } else { (1. - scale.recip()) * 100. };
            Ok((
                vec![filter("image.adjustments.hueSaturation", json!({"hue":0.,"saturation":percent,"lightness":0.,"colorize":false}))],
                "Source saturation gain becomes editable native master HSL saturation. The native positive/negative gain law, color model, transfer and clipping supersede source component-space behavior; this is a disclosed native approximation.",
            ))
        }
        "gegl:channel-mixer" | "gegl:mono-mixer" => mixer(e, ctl),
        "gegl:sepia" => {
            allowed(e, &["scale", "srgb"], ctl)?;
            let scale = number(e, "scale", 1., 0., 1.)?;
            flag(e, "srgb", true)?;
            Ok((
                vec![filter("image.adjustments.photoFilter", json!({"color":[172./255.,122./255.,51./255.],"density":scale*100.,"preserveLuminosity":true}))],
                "Source sepia strength becomes the ordinary native Photo Filter Sepia preset. Native luminosity, document transfer and depth rules intentionally replace the source sepia matrix and srgb flag; this is a native approximation.",
            ))
        }
        _ => Err(Failure::Unsupported("point color operation")),
    }
}
fn mixer(e: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    let mono = e.operation == "gegl:mono-mixer";
    let fields: &[&str] = if mono {
        &["preserve-luminosity", "red", "green", "blue"]
    } else {
        &["preserve-luminosity", "rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain"]
    };
    allowed(e, fields, ctl)?;
    if flag(e, "preserve-luminosity", false)? {
        return Err(Failure::Unsupported("source luminosity normalization has no verified mapping"));
    }
    let mut rows = [[0f64; 4]; 3];
    let keys = if mono {
        [["red", "green", "blue"]; 3]
    } else {
        [["rr-gain", "rg-gain", "rb-gain"], ["gr-gain", "gg-gain", "gb-gain"], ["br-gain", "bg-gain", "bb-gain"]]
    };
    for (row, (out, names)) in rows.iter_mut().zip(keys).enumerate() {
        for (col, name) in names.into_iter().enumerate() {
            out[col] = number(
                e,
                name,
                if mono {
                    0.333
                } else if row == col {
                    1.
                } else {
                    0.
                },
                -2.,
                2.,
            )? * 100.;
        }
    }
    let params = if mono { json!({"gray":rows[0],"monochrome":true}) } else { json!({"red":rows[0],"green":rows[1],"blue":rows[2],"monochrome":false}) };
    Ok((
        vec![filter("image.adjustments.channelMixer", params)],
        "Source coefficient fractions become native Channel Mixer percentages. Native whole-percent control rounding, F32 coefficient/constant quantization, channel interpretation and clipping are explicit approximations. Unverified preserve-luminosity normalization is left unapplied.",
    ))
}

/// The exact small native schemas produced above; no defaults are acquired from
/// arbitrary JSON. Engine round-trip tests compare the ordinary command parser.
pub(super) fn adjustment(f: &SmartFilter) -> Option<std::result::Result<Adjustment, super::installer::Refusal>> {
    if !native(&f.command) {
        return None;
    }
    use super::installer::Refusal;
    let parse = || {
        let bad = || Refusal::Unsupported("native point-color schema");
        let fields = f.params.as_object().ok_or_else(bad)?;
        let num = |key: &str, low: f64, high: f64| -> std::result::Result<f32, Refusal> {
            fields.get(key).and_then(Value::as_f64).filter(|v| v.is_finite() && (low..=high).contains(v)).map(|v| v as f32).ok_or_else(bad)
        };
        let boolean = |key: &str| fields.get(key).and_then(Value::as_bool).ok_or_else(bad);
        Ok(match f.command.as_str() {
            "image.adjustments.posterize" if fields.len() == 1 => {
                let levels = num("levels", 2., 255.)?;
                if levels.fract() != 0. {
                    return Err(bad());
                }
                Adjustment::Posterize { levels: levels as u32 }
            }
            "image.adjustments.vibrance" if fields.len() == 2 => {
                Adjustment::Vibrance { vibrance: num("vibrance", -100., 100.)?, saturation: num("saturation", -100., 100.)? }
            }
            "image.adjustments.hueSaturation" if fields.len() == 4 => Adjustment::HueSaturation {
                hue: num("hue", -180., 180.)?,
                saturation: num("saturation", -100., 100.)?,
                lightness: num("lightness", -100., 100.)?,
                colorize: boolean("colorize")?,
                ranges: HueRange::defaults(),
            },
            "image.adjustments.photoFilter" if fields.len() == 3 => {
                let colors = fields.get("color").and_then(Value::as_array).filter(|v| v.len() == 3).ok_or_else(bad)?;
                let mut color = [0.; 3];
                for (out, value) in color.iter_mut().zip(colors) {
                    *out = value.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(bad)? as f32;
                }
                Adjustment::PhotoFilter { color, density: num("density", 0., 100.)? / 100., preserve_luminosity: boolean("preserveLuminosity")? }
            }
            "image.adjustments.channelMixer" => {
                let mono = boolean("monochrome")?;
                let names = if mono { &["gray"][..] } else { &["red", "green", "blue"][..] };
                if fields.len() != names.len() + 1 {
                    return Err(bad());
                }
                let mut matrix = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
                for (name, row) in names.iter().zip(matrix.iter_mut()) {
                    let values = fields.get(*name).and_then(Value::as_array).filter(|v| v.len() == 4).ok_or_else(bad)?;
                    for (out, value) in row.iter_mut().zip(values) {
                        *out = (value.as_f64().filter(|v| v.is_finite() && (-200.0..=200.0).contains(v)).ok_or_else(bad)? as f32).round() / 100.;
                    }
                }
                Adjustment::ChannelMixer { matrix, monochrome: mono, cmyk: None }
            }
            _ => return Err(bad()),
        })
    };
    Some(parse())
}
