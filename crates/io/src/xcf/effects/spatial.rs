//! Bounded spatial translations using ordinary native kernels. Source edge,
//! alpha, anisotropy and intermediate differences are disclosed, never evaluated.
use super::{Failure, Result, allowed, filter, number};
use craft_xcf::effects::{ArgumentValue, StoredEffect};
use photocraft_doc::SmartFilter;
use photocraft_raster::Interrupt;
use serde_json::json;
pub(super) fn supports(op: &str) -> bool {
    matches!(op, "gegl:gaussian-blur" | "gegl:box-blur" | "gegl:motion-blur-linear" | "gegl:unsharp-mask" | "gegl:high-pass")
}
fn flag(e: &StoredEffect<'_>, key: &str) -> Result<()> {
    match e.arguments.iter().find(|a| a.name == key).map(|a| &a.value) {
        None | Some(ArgumentValue::Bool(_)) => Ok(()),
        _ => Err(Failure::Invalid("spatial flag needs a stored boolean")),
    }
}
fn choice(e: &StoredEffect<'_>, key: &str, max: i32) -> Result<()> {
    match e.arguments.iter().find(|a| a.name == key).map(|a| &a.value) {
        None => Ok(()),
        Some(ArgumentValue::Enum(v)) if (0..=max).contains(v) => Ok(()),
        _ => Err(Failure::Invalid("unrecognized spatial choice")),
    }
}
pub(super) fn map(e: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    match e.operation {
        "gegl:gaussian-blur" => {
            allowed(e, &["std-dev-x", "std-dev-y", "filter", "abyss-policy", "clip-extent"], ctl)?;
            let x = number(e, "std-dev-x", 1.5, 0., 1500.)?;
            let y = number(e, "std-dev-y", 1.5, 0., 1500.)?;
            choice(e, "filter", 2)?;
            choice(e, "abyss-policy", 3)?;
            flag(e, "clip-extent")?;
            let sigma = ((x * x + y * y) * 0.5).sqrt();
            if sigma < 0.1 {
                return Err(Failure::Unsupported("Gaussian below native editable minimum remains unapplied"));
            }
            let passes = ((sigma / 1000.).powi(2).ceil() as usize).max(1);
            let radius = sigma / (passes as f64).sqrt();
            let mut filters = Vec::new();
            for _ in 0..passes {
                ctl.check().map_err(|_| Failure::Cancelled)?;
                filters.push(filter("filter.blur.gaussianBlur", json!({"radius":radius})));
            }
            Ok((
                filters,
                "Source X/Y Gaussian variances become an isotropic native RMS sigma, split into at most three native passes inside editable limits. Native kernel, alpha weighting, edge extension, output extent and depth rounding supersede source anisotropy/filter/abyss/clip choices; this is a native approximation.",
            ))
        }
        "gegl:box-blur" => {
            allowed(e, &["radius"], ctl)?;
            let radius = number(e, "radius", 4., 0., 1000.)?;
            if radius.fract() != 0. {
                return Err(Failure::Invalid("box radius must be integral"));
            }
            if radius < 1. {
                return Err(Failure::Unsupported("zero box radius has no editable native control"));
            }
            Ok((
                vec![filter("filter.blur.boxBlur", json!({"radius":radius}))],
                "Source integer neighborhood radius becomes native Box Blur half-width. Native alpha weighting, repeated-edge behavior and output extent apply as a disclosed native approximation.",
            ))
        }
        "gegl:motion-blur-linear" => {
            allowed(e, &["length", "angle"], ctl)?;
            let distance = number(e, "length", 10., 0., 1000.)?;
            let angle = number(e, "angle", 0., -180., 180.)?;
            if distance < 1. {
                return Err(Failure::Unsupported("motion length below native editable minimum"));
            }
            Ok((
                vec![filter("filter.blur.motionBlur", json!({"distance":distance,"angle":-angle}))],
                "Clockwise source image-space angle becomes negative native mathematical angle, using the preserved independently observed impulse convention. Native centered line sampling, alpha, edges and output bounds are explicit approximations.",
            ))
        }
        "gegl:unsharp-mask" => {
            allowed(e, &["std-dev", "scale", "threshold"], ctl)?;
            let radius = number(e, "std-dev", 3., 0., 1500.)?;
            let scale = number(e, "scale", 0.5, 0., 300.)?;
            let threshold = number(e, "threshold", 0., 0., 1.)?;
            if !(0.1..=1000.).contains(&radius) || scale < 0.01 {
                return Err(Failure::Unsupported("unsharp parameters require an unproved native structure"));
            }
            let passes = (scale.ln_1p() / 6f64.ln()).ceil().max(1.) as usize;
            let amount = if passes == 1 { scale * 100. } else { (scale.ln_1p() / passes as f64).exp_m1() * 100. };
            if !(1.0..=500.).contains(&amount) {
                return Err(Failure::Range("native unsharp amount"));
            }
            let mut filters = Vec::new();
            for _ in 0..passes {
                ctl.check().map_err(|_| Failure::Cancelled)?;
                filters.push(filter("filter.sharpen.unsharpMask", json!({"radius":radius,"amount":amount,"threshold":threshold*255.})));
            }
            Ok((
                filters,
                "Source sigma, sharpening scale and normalized threshold become bounded native Unsharp Mask sigma, percent and 8-bit-level controls. Large gain uses at most four native passes; native intermediate clipping, threshold law, edges and depth arithmetic are explicit approximations.",
            ))
        }
        "gegl:high-pass" => {
            allowed(e, &["std-dev", "contrast"], ctl)?;
            let radius = number(e, "std-dev", 4., 0., 10000.)?;
            let contrast = number(e, "contrast", 1., 0., 5.)?;
            if !(0.1..=1000.).contains(&radius) || contrast != 1. {
                return Err(Failure::Unsupported("High Pass requires an unproved native branch or contrast construction"));
            }
            Ok((
                vec![filter("filter.other.highPass", json!({"radius":radius}))],
                "Source unit-contrast sigma becomes ordinary native High Pass. Native neutral pivot, Gaussian alpha weighting, edges, output extent and HDR/depth rules apply; broader radii or nonunit contrast remain unapplied.",
            ))
        }
        _ => Err(Failure::Unsupported("spatial operation")),
    }
}
