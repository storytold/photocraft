//! Named stock graph relatives translated to native tonal controls.
//! Facts are property/registration declarations in pinned GEGL 0.4.72
//! common/{grey,posterize,threshold,wavelet-blur}.c, not processing code.
use super::{MappingContext, MappingFailure, MappingResult, NativeMapping, StoredEffect, graph_schema::Kind, native_filter};
use photocraft_color::ColorMode;
use photocraft_doc::import_conversion::ConversionReason;
use serde_json::json;

pub(super) fn operation(token: &str) -> Option<&'static str> {
    match token {
        "gray" | "grey" | "gegl:gray" | "gegl:grey" => Some("gegl:gray"),
        "posterize" | "gegl:posterize" => Some("gegl:posterize"),
        "threshold" | "gegl:threshold" => Some("gegl:threshold"),
        "wavelet-blur" | "gegl:wavelet-blur" => Some("gegl:wavelet-blur"),
        _ => None,
    }
}
pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    match (op, name) {
        ("gegl:posterize", "levels") => Some(Kind::Integer { min: 1, max: 64 }),
        ("gegl:threshold", "value" | "high") => Some(Kind::Number { min: -200., max: 200. }),
        ("gegl:wavelet-blur", "radius") => Some(Kind::Number { min: 0., max: 1500. }),
        _ => None,
    }
}
fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: ConversionReason::InvalidOptionalData, message: message.into() }
}
fn number(e: &StoredEffect<'_>, name: &str, default: f64) -> Result<f64, MappingFailure> {
    let n = e.number(name)?.unwrap_or(default);
    let (low, high, integer) = match field(e.operation, name) {
        Some(Kind::Number { min, max }) => (min, max, false),
        Some(Kind::Integer { min, max }) => (min as f64, max as f64, true),
        _ => return Err(invalid("Missing explicit stock tone field domain")),
    };
    if !n.is_finite() || !(low..=high).contains(&n) || (integer && n.fract() != 0.) {
        return Err(invalid(format!("Stock tone field {name} is outside its finite declared domain")));
    }
    Ok(n)
}
pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> MappingResult {
    operation(e.operation)?;
    Some((|| {
        c.interrupt.check().map_err(|e| e.to_string())?;
        let mut seen = std::collections::HashSet::new();
        for arg in &e.arguments {
            if field(e.operation, arg.name).is_none() || !seen.insert(arg.name) {
                return Err(invalid(format!("Unknown or duplicate stock tone field {}:{}", e.operation, arg.name)));
            }
        }
        let mut mapping = NativeMapping::default();
        match e.operation {
            "gegl:gray" => {
                if c.mode == ColorMode::Rgb {
                    mapping.filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]})));
                } else if c.mode != ColorMode::Grayscale {
                    return Err("Native gray conversion requires RGB/Gray contents".into());
                }
                mapping.notes.push("Stock Gray becomes ordinary native monochrome Channel Mixer with native luminance weights; native document profile/transfer and channel rules apply, rather than source Babl grayscale emulation. Gray input remains native Gray.".into());
            }
            "gegl:posterize" => {
                let levels = number(e, "levels", 8.)?;
                mapping.filters.push(native_filter("image.adjustments.posterize", json!({"levels":levels.max(2.)})));
                mapping.notes.push(format!("Stock {levels} quantization levels become editable native Posterize. Source level1 uses native minimum2 explicitly; native rounding, depth, alpha and HDR behavior apply."));
            }
            "gegl:threshold" => {
                let low = number(e, "value", 0.5)?;
                let high = number(e, "high", 1.)?;
                if c.mode == ColorMode::Rgb {
                    mapping.filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]})));
                } else if c.mode != ColorMode::Grayscale {
                    return Err("Native threshold conversion requires RGB/Gray contents".into());
                }
                let points = (0..256)
                    .map(|i| {
                        let x = f64::from(i) / 255.;
                        if x >= low && x <= high { 255 } else { 0 }
                    })
                    .collect::<Vec<_>>();
                mapping.filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":points})));
                mapping.notes.push(format!("Stock threshold interval [{low},{high}] becomes native luminance extraction and editable native pencil Curves. Inclusive bounds, crossed empty interval and out-of-range controls are explicit; native 0..1 curve input clipping, edge interpolation, profile/depth and alpha rules apply. A supplied auxiliary threshold image requires a separate actual-input adapter; it is never replaced by this scalar control."));
            }
            "gegl:wavelet-blur" => {
                let radius = number(e, "radius", 1.)?;
                let mut variance = radius * radius;
                for _ in 0..3 {
                    if variance <= 0. {
                        break;
                    }
                    let sigma = variance.sqrt().min(1000.);
                    mapping.filters.push(native_filter("filter.blur.gaussianBlur", json!({"radius":sigma})));
                    variance = (variance - sigma * sigma).max(0.);
                }
                mapping.notes.push(format!("Stock wavelet radius {radius} pixels becomes ordinary native Gaussian smoothing at the same nominal native radius, factored into bounded native passes by variance above1000. Native Gaussian kernel, edge, alpha and document color behavior are an explicit approximation; no wavelet source kernel is reproduced."));
            }
            _ => return Err("Stock tone identity has no native adapter".into()),
        }
        Ok(mapping)
    })())
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue};
    use super::*;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
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
    #[test]
    fn declared_tone_controls_become_editable_native_commands() {
        let c = context();
        let gray = map(&StoredEffect::test("gegl:gray", vec![]), &c).unwrap().unwrap();
        assert_eq!(gray.filters[0].params["gray"], json!([21.26, 71.52, 7.22, 0.]));
        let posterize = map(&StoredEffect::test("gegl:posterize", vec![Argument { name: "levels", value: ArgumentValue::Int(1) }]), &c).unwrap().unwrap();
        assert_eq!(posterize.filters[0].params, json!({"levels":2.}));
        let threshold = map(
            &StoredEffect::test(
                "gegl:threshold",
                vec![Argument { name: "value", value: ArgumentValue::Literal(0.25) }, Argument { name: "high", value: ArgumentValue::Literal(0.75) }],
            ),
            &c,
        )
        .unwrap()
        .unwrap();
        let points = threshold.filters[1].params["points"].as_array().unwrap();
        assert_eq!(points.len(), 256);
        assert_eq!(points[63], 0);
        assert_eq!(points[64], 255);
        assert_eq!(points[191], 255);
        assert_eq!(points[192], 0);
        let blur = map(&StoredEffect::test("gegl:wavelet-blur", vec![Argument { name: "radius", value: ArgumentValue::Literal(1500.) }]), &c).unwrap().unwrap();
        let sum: f64 = blur.filters.iter().map(|f| f.params["radius"].as_f64().unwrap().powi(2)).sum();
        assert_eq!(blur.filters.len(), 3);
        assert!((sum - 2250000.).abs() < 1e-6);
        assert_eq!(operation("gegl:grey"), Some("gegl:gray"));
        assert_eq!(operation("gimp:posterize"), None);
    }
    #[test]
    fn invalid_or_unowned_fields_do_not_silently_discard_controls() {
        for (op, name, value) in [("gegl:posterize", "levels", 1.5), ("gegl:threshold", "value", 201.), ("gegl:wavelet-blur", "radius", f64::NAN)] {
            let failure = map(&StoredEffect::test(op, vec![Argument { name, value: ArgumentValue::Literal(value) }]), &context()).unwrap().unwrap_err();
            assert_eq!(failure.reason, ConversionReason::InvalidOptionalData);
        }
        assert!(map(&StoredEffect::test("gegl:gray", vec![Argument { name: "unknown", value: ArgumentValue::Int(1) }]), &context()).unwrap().is_err());
    }
}
